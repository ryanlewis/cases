package web

import (
	"bytes"
	"embed"
	"encoding/json"
	"errors"
	"fmt"
	"html/template"
	"io/fs"
	"net/http"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/extension"

	"github.com/ryanlewis/cases/internal/store"
)

//go:embed templates/*.html
var templateFS embed.FS

//go:embed static
var embedded embed.FS

// staticFS is served under /static/. htmx.min.js is htmx 2.0.10, taken from
// the npm tarball htmx.org-2.0.10.tgz (integrity sha512-kdeJe7ZVwaS6QMz/ebBIVtZdpwen6L0OQ5GOhPV9MKBb196TCZeZu4yA7ZIQsaLKv7EpXz+So7KSXNuHXhj7Cw==).
// Its SHA-256 is htmxSHA256; a test checks the embedded file against it.
var staticFS, _ = fs.Sub(embedded, "static")

const htmxSHA256 = "71ea67185bfa8c98c39d31717c6fce5d852370fcdfd129db4543774d3145c0de"

// markdown renders bodies. The default renderer omits raw HTML and blanks
// dangerous link targets, so a body cannot inject markup. The GFM table
// extension is added so status/comparison tables in bodies render as tables.
var markdown = goldmark.New(goldmark.WithExtensions(extension.Table))

func renderMarkdown(src string) template.HTML {
	var buf bytes.Buffer
	if err := markdown.Convert([]byte(src), &buf); err != nil {
		return template.HTML(template.HTMLEscapeString(src))
	}
	return template.HTML(buf.String()) //nolint:gosec // goldmark output with raw HTML disabled
}

var funcs = template.FuncMap{
	"markdown":   renderMarkdown,
	"pathEscape": url.PathEscape,
	"stamp": func(t time.Time) string {
		if t.IsZero() {
			return ""
		}
		return t.UTC().Format("2006-01-02 15:04 UTC")
	},
	// checked reports whether a re-rendered form had value selected for name.
	"checked": func(form url.Values, name, value string) bool {
		return form != nil && form.Get(name) == value
	},
	"formValue": func(form url.Values, name string) string {
		if form == nil {
			return ""
		}
		return form.Get(name)
	},
	"inc": func(i int) int { return i + 1 },
}

var pages = map[string]*template.Template{}

func init() {
	for _, page := range []string{"inbox", "case", "done"} {
		pages[page] = template.Must(template.New("layout.html").Funcs(funcs).ParseFS(templateFS, "templates/layout.html", "templates/"+page+".html"))
	}
}

// page is what every full page carries for the layout.
type page struct {
	Title    string
	Blocking int
}

// countBlocking counts open blocking cases: the ones a worker is idle on.
func countBlocking(cases []*store.Case) int {
	n := 0
	for _, c := range cases {
		if c.State == store.StateOpen && c.Urgency == store.UrgencyBlocking {
			n++
		}
	}
	return n
}

type card struct {
	*store.Case
	Age     string
	Excerpt []string
}

type inboxData struct {
	page
	Cards []card
}

func (s *Server) inboxData() (inboxData, error) {
	cases, err := s.cases()
	if err != nil {
		return inboxData{}, err
	}
	var shown []*store.Case
	for _, c := range cases {
		if c.State == store.StateOpen || c.State == store.StateParked {
			shown = append(shown, c)
		}
	}
	store.SortInbox(shown)
	now := time.Now()
	d := inboxData{page: page{Title: "Inbox", Blocking: countBlocking(cases)}}
	for _, c := range shown {
		d.Cards = append(d.Cards, card{Case: c, Age: Age(c.OpenedAt, now), Excerpt: excerpt(c.Body, 3)})
	}
	return d, nil
}

// excerpt returns the first n non-empty lines of a body, as plain text.
func excerpt(body string, n int) []string {
	var lines []string
	for line := range strings.SplitSeq(body, "\n") {
		if line = strings.TrimSpace(line); line == "" {
			continue
		}
		if r := []rune(line); len(r) > 160 {
			line = string(r[:160]) + "…"
		}
		if lines = append(lines, line); len(lines) == n {
			break
		}
	}
	return lines
}

func (s *Server) inbox(w http.ResponseWriter, r *http.Request) {
	d, err := s.inboxData()
	if err != nil {
		s.fail(w, err)
		return
	}
	s.render(w, http.StatusOK, "inbox", "layout.html", d)
}

func (s *Server) inboxFragment(w http.ResponseWriter, r *http.Request) {
	d, err := s.inboxData()
	if err != nil {
		s.fail(w, err)
		return
	}
	s.render(w, http.StatusOK, "inbox", "inbox-fragment", d)
}

type doneCard struct {
	*store.Case
	Age string
}

func (s *Server) done(w http.ResponseWriter, r *http.Request) {
	cases, err := s.cases()
	if err != nil {
		s.fail(w, err)
		return
	}
	d := struct {
		page
		Cards []doneCard
	}{page: page{Title: "Done", Blocking: countBlocking(cases)}}
	var shown []*store.Case
	for _, c := range cases {
		switch c.State {
		case store.StateAnswered, store.StatePickedUp, store.StateClosed, store.StateWithdrawn:
			shown = append(shown, c)
		}
	}
	slices.SortStableFunc(shown, func(a, b *store.Case) int { return b.UpdatedAt.Compare(a.UpdatedAt) })
	now := time.Now()
	for _, c := range shown {
		d.Cards = append(d.Cards, doneCard{Case: c, Age: Age(c.UpdatedAt, now)})
	}
	s.render(w, http.StatusOK, "done", "layout.html", d)
}

type threadEntry struct {
	store.Event
	Lines    []string
	Markdown template.HTML
}

type caseData struct {
	page
	Case   *store.Case
	Thread []threadEntry
	Error  string
	Form   url.Values
}

func thread(c *store.Case) []threadEntry {
	var out []threadEntry
	for _, ev := range c.Events {
		e := threadEntry{Event: ev}
		switch ev.Type {
		case store.EventOpen:
			// The body is shown at the top of the page.
		case store.EventNote:
			var n store.NoteRecord
			_ = json.Unmarshal(ev.Data, &n)
			e.Markdown = renderMarkdown(n.Body)
		case store.EventClose:
			var cl store.CloseRecord
			_ = json.Unmarshal(ev.Data, &cl)
			e.Markdown = renderMarkdown(cl.Outcome)
			e.Lines = cl.Links
		default:
			e.Lines = c.Describe(ev)
		}
		out = append(out, e)
	}
	return out
}

func (s *Server) casePage(w http.ResponseWriter, r *http.Request) {
	c, cases, err := s.find(r.PathValue("id"))
	if err != nil {
		s.fail(w, err)
		return
	}
	if c == nil {
		http.NotFound(w, r)
		return
	}
	s.renderCase(w, http.StatusOK, c, cases, "", nil)
}

func (s *Server) renderCase(w http.ResponseWriter, status int, c *store.Case, cases []*store.Case, msg string, form url.Values) {
	d := caseData{
		page:   page{Title: c.Title, Blocking: countBlocking(cases)},
		Case:   c,
		Thread: thread(c),
		Error:  msg,
		Form:   form,
	}
	s.render(w, status, "case", "layout.html", d)
}

// threadFragment is polled by the case page. When the case has moved to a
// state other than the one the page was rendered in, the form on the page is
// stale, so it sends htmx to the case page with a fresh GET. A reload would
// be wrong: the page may be the re-rendered result of a failed POST, and
// reloading it would post the form again.
func (s *Server) threadFragment(w http.ResponseWriter, r *http.Request) {
	c, _, err := s.find(r.PathValue("id"))
	if err != nil {
		s.fail(w, err)
		return
	}
	if c == nil {
		http.NotFound(w, r)
		return
	}
	if string(c.State) != r.URL.Query().Get("state") {
		w.Header().Set("HX-Redirect", "/cases/"+url.PathEscape(c.ID))
		w.WriteHeader(http.StatusNoContent)
		return
	}
	s.render(w, http.StatusOK, "case", "thread", caseData{Case: c, Thread: thread(c)})
}

// loadForPost resolves the case a form posts to, straight from disk.
func (s *Server) loadForPost(w http.ResponseWriter, r *http.Request) (string, *store.Case, bool) {
	dir, err := store.CaseDir(s.root, r.PathValue("id"))
	if err != nil {
		http.NotFound(w, r)
		return "", nil, false
	}
	c, err := store.Load(dir)
	if errors.Is(err, fs.ErrNotExist) {
		http.NotFound(w, r)
		return "", nil, false
	}
	if err != nil {
		s.fail(w, err)
		return "", nil, false
	}
	r.Body = http.MaxBytesReader(w, r.Body, maxForm)
	if err := r.ParseForm(); err != nil {
		http.Error(w, "bad form: "+err.Error(), http.StatusBadRequest)
		return "", nil, false
	}
	return dir, c, true
}

func (s *Server) answer(w http.ResponseWriter, r *http.Request) {
	dir, c, ok := s.loadForPost(w, r)
	if !ok {
		return
	}
	rec, park, err := answerFromForm(c, r.PostForm)
	if err == nil {
		if park {
			_, err = store.Park(dir, store.ParkRecord{Note: rec.Note})
		} else {
			_, err = store.Answer(dir, rec)
		}
	}
	if err != nil {
		cases, _ := s.cases()
		s.renderCase(w, http.StatusUnprocessableEntity, c, cases, err.Error(), r.PostForm)
		return
	}
	http.Redirect(w, r, "/cases/"+url.PathEscape(c.ID), http.StatusSeeOther)
}

func (s *Server) resume(w http.ResponseWriter, r *http.Request) {
	dir, c, ok := s.loadForPost(w, r)
	if !ok {
		return
	}
	if _, err := store.Resume(dir, store.AuthorHuman, store.ResumeRecord{}); err != nil {
		cases, _ := s.cases()
		s.renderCase(w, http.StatusUnprocessableEntity, c, cases, err.Error(), nil)
		return
	}
	http.Redirect(w, r, "/cases/"+url.PathEscape(c.ID), http.StatusSeeOther)
}

// answerFromForm turns the posted response form into an answer. park is true
// when a stuck case is being parked, which is a park event rather than an
// answer. The store validates the result against the kind; this only reports
// a response that was not chosen at all, in words that fit the form.
func answerFromForm(c *store.Case, f url.Values) (rec store.AnswerRecord, park bool, err error) {
	rec.Note = strings.TrimSpace(f.Get("note"))
	switch c.Kind {
	case store.KindDecision:
		switch choice := f.Get("choice"); choice {
		case "":
			return rec, false, errors.New("choose an option")
		case "other":
			rec.Other = true
		default:
			n, err := strconv.Atoi(choice)
			if err != nil {
				return rec, false, fmt.Errorf("choice %q is not an option", choice)
			}
			rec.Choice = n
		}
	case store.KindApproval:
		for _, row := range c.Rows {
			verdict := f.Get("verdict." + row.ID)
			if verdict == "" {
				return rec, false, fmt.Errorf("choose approve, hold or reject for %q", row.Label)
			}
			rec.Rows = append(rec.Rows, store.RowAnswer{ID: row.ID, Verdict: verdict, Note: strings.TrimSpace(f.Get("note." + row.ID))})
		}
	case store.KindSignoff:
		if rec.Signoff = f.Get("signoff"); rec.Signoff == "" {
			return rec, false, errors.New("choose accept or request changes")
		}
	case store.KindStuck:
		choice := f.Get("stuck")
		if choice == "" && strings.TrimSpace(f.Get("text")) != "" {
			// Guidance typed without ticking its button still means guidance.
			choice = "text"
		}
		switch choice {
		case "text":
			if rec.Text = strings.TrimSpace(f.Get("text")); rec.Text == "" {
				return rec, false, errors.New("write the guidance, or choose park or drop")
			}
		case "park":
			return rec, true, nil
		case "drop":
			rec.Drop = true
		default:
			return rec, false, errors.New("choose guidance, park or drop")
		}
	case store.KindFYI:
		rec.Ack = f.Get("ack") != ""
	}
	return rec, false, nil
}

// render executes a named template into a buffer first, so a template error
// becomes a 500 rather than half a page.
func (s *Server) render(w http.ResponseWriter, status int, set, name string, data any) {
	var buf bytes.Buffer
	if err := pages[set].ExecuteTemplate(&buf, name, data); err != nil {
		s.fail(w, err)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.WriteHeader(status)
	_, _ = buf.WriteTo(w)
}

func (s *Server) fail(w http.ResponseWriter, err error) {
	s.logf("error: %v", err)
	http.Error(w, "internal error", http.StatusInternalServerError)
}
