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
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/extension"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"

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
var markdown = goldmark.New(
	goldmark.WithExtensions(extension.Table),
	goldmark.WithParserOptions(parser.WithASTTransformers(util.Prioritized(newTab{}, 100))),
)

// newTab makes links that leave the inbox open in a new tab. The attributes
// are set on the parsed tree here, never taken from the body: goldmark's
// attribute syntax is not enabled.
type newTab struct{}

func (newTab) Transform(doc *ast.Document, reader text.Reader, _ parser.Context) {
	_ = ast.Walk(doc, func(n ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering {
			return ast.WalkContinue, nil
		}
		var dest []byte
		switch l := n.(type) {
		case *ast.Link:
			dest = l.Destination
		case *ast.AutoLink:
			dest = l.URL(reader.Source())
		default:
			return ast.WalkContinue, nil
		}
		if leavesInbox(string(dest)) {
			n.SetAttributeString("target", "_blank")
			n.SetAttributeString("rel", "noopener noreferrer")
		}
		return ast.WalkContinue, nil
	})
}

// leavesInbox reports whether a link destination points off the inbox: it has
// a scheme or is protocol-relative. It does not use url.Parse, which rejects
// destinations such as "https://example.com/100%" that browsers still follow.
func leavesInbox(dest string) bool {
	if strings.HasPrefix(dest, "//") {
		return true
	}
	for i, r := range dest {
		switch {
		case r == ':':
			return i > 0
		case 'a' <= r && r <= 'z', 'A' <= r && r <= 'Z':
		case i > 0 && ('0' <= r && r <= '9' || r == '+' || r == '-' || r == '.'):
		default:
			return false
		}
	}
	return false
}

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
	pages["case"] = template.Must(template.New("layout.html").Funcs(funcs).ParseFS(templateFS, "templates/layout.html", "templates/inbox.html", "templates/case.html"))
	pages["done"] = template.Must(template.New("layout.html").Funcs(funcs).ParseFS(templateFS, "templates/layout.html", "templates/done.html"))
}

// page is what every full page carries for the layout.
type page struct {
	Title    string
	Blocking int
	Nav      string // the header link to mark as current
}

// countBlocking counts open blocking cases: the ones an agent is idle on.
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

// inboxData is the list column. Selected is the id of the case beside it,
// and the title is that case's title, so the polled list keeps both.
type inboxData struct {
	page
	Cards    []card
	Selected string
	// Empty is set on / when it was rendered with no case to show, so the
	// polled list reloads / once a case arrives.
	Empty bool
}

// inboxCases returns the open and parked cases, in inbox order.
func inboxCases(cases []*store.Case) []*store.Case {
	var shown []*store.Case
	for _, c := range cases {
		if c.State == store.StateOpen || c.State == store.StateParked {
			shown = append(shown, c)
		}
	}
	store.SortInbox(shown)
	return shown
}

func newInbox(cases []*store.Case, selected string) inboxData {
	d := inboxData{page: page{Title: "inbox", Blocking: countBlocking(cases), Nav: "inbox"}, Selected: selected}
	for _, c := range cases {
		if c.ID == selected {
			d.Title = c.Title
			break
		}
	}
	now := time.Now()
	for _, c := range inboxCases(cases) {
		d.Cards = append(d.Cards, card{Case: c, Age: Age(c.OpenedAt, now), Excerpt: excerpt(c.Body, 3)})
	}
	return d
}

// nextCase is where a successful answer, park or resume of the case with id
// goes: the case after it in the inbox, or the inbox when it was the last.
// cases must be read before the write, while the case still has its place.
func nextCase(cases []*store.Case, id string) string {
	shown := inboxCases(cases)
	for i, c := range shown {
		if c.ID == id && i+1 < len(shown) {
			return "/cases/" + url.PathEscape(shown[i+1].ID)
		}
	}
	return "/"
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

// inbox shows the first case in the inbox beside the list, in place rather
// than by redirect, so / stays the top of the inbox when it is reloaded.
func (s *Server) inbox(w http.ResponseWriter, r *http.Request) {
	cases, err := s.cases()
	if err != nil {
		s.fail(w, err)
		return
	}
	var first *store.Case
	if shown := inboxCases(cases); len(shown) > 0 {
		first = shown[0]
	}
	s.renderCase(w, http.StatusOK, first, cases, true, "", nil)
}

func (s *Server) inboxFragment(w http.ResponseWriter, r *http.Request) {
	cases, err := s.cases()
	if err != nil {
		s.fail(w, err)
		return
	}
	d := newInbox(cases, r.URL.Query().Get("selected"))
	if r.URL.Query().Get("empty") == "1" {
		if len(d.Cards) > 0 {
			w.Header().Set("HX-Redirect", "/")
			w.WriteHeader(http.StatusNoContent)
			return
		}
		d.Empty = true
	}
	s.render(w, http.StatusOK, "case", "inbox-fragment", d)
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
	}{page: page{Title: "done", Blocking: countBlocking(cases), Nav: "done"}}
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

// caseData is the list column and the working area. Case is nil when the
// inbox is empty. Home is set on /, where a stale thread reloads / instead.
type caseData struct {
	page
	Inbox  inboxData
	Home   bool
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
	s.renderCase(w, http.StatusOK, c, cases, false, "", nil)
}

func (s *Server) renderCase(w http.ResponseWriter, status int, c *store.Case, cases []*store.Case, home bool, msg string, form url.Values) {
	d := caseData{Home: home, Error: msg, Form: form}
	if c != nil {
		d.Case, d.Thread = c, thread(c)
		d.Inbox = newInbox(cases, c.ID)
		d.Inbox.Title = c.Title
	} else {
		d.Inbox = newInbox(cases, "")
		d.Inbox.Empty = home
	}
	d.page = d.Inbox.page
	s.render(w, status, "case", "layout.html", d)
}

// threadFragment is polled by the case page. When the case has moved to a
// state other than the one the page was rendered in, the form on the page is
// stale, so it sends htmx to the case page with a fresh GET. A reload would
// be wrong: the page may be the re-rendered result of a failed POST, and
// reloading it would post the form again. On / (home=1) it sends htmx to /,
// which shows whichever case is now first. A change that leaves the state as
// it was, such as an answer and then a note that reopens the case, only
// updates the thread, so what the human has typed is kept: the form's
// revision is out of date by then, and sending it gets a 409.
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
	home := r.URL.Query().Get("home") == "1"
	if string(c.State) != r.URL.Query().Get("state") {
		target := "/cases/" + url.PathEscape(c.ID)
		if home {
			target = "/"
		}
		w.Header().Set("HX-Redirect", target)
		w.WriteHeader(http.StatusNoContent)
		return
	}
	s.render(w, http.StatusOK, "case", "thread", caseData{Home: home, Case: c, Thread: thread(c)})
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
	cases, _ := s.cases()
	next := nextCase(cases, c.ID)
	// A form from a page older than the case is refused before its answer is
	// read, so a mistake in the form cannot show the page again at the new
	// revision without saying the case has changed. The store checks the
	// revision again while the case is locked.
	rev := revisionParam(r.PostForm)
	if rev != c.Revision() {
		s.refuse(w, c, cases, store.ErrStale, r.PostForm)
		return
	}
	rec, park, err := answerFromForm(c, r.PostForm)
	if err == nil {
		if park {
			_, err = store.Park(dir, store.ParkRecord{Note: rec.Note}, store.AtRevision(rev))
		} else {
			_, err = store.Answer(dir, rec, store.AtRevision(rev))
		}
	}
	if err != nil {
		s.refuse(w, c, cases, err, r.PostForm)
		return
	}
	http.Redirect(w, r, next, http.StatusSeeOther)
}

func (s *Server) resume(w http.ResponseWriter, r *http.Request) {
	dir, c, ok := s.loadForPost(w, r)
	if !ok {
		return
	}
	cases, _ := s.cases()
	next := nextCase(cases, c.ID)
	if _, err := store.Resume(dir, store.AuthorHuman, store.ResumeRecord{}, store.AtRevision(revisionParam(r.PostForm))); err != nil {
		s.refuse(w, c, cases, err, nil)
		return
	}
	http.Redirect(w, r, next, http.StatusSeeOther)
}

// revisionParam is the revision of the case a page was rendered from, as the
// page's forms send it back. A form from a page served before forms carried
// one sends none and gets -1, which no case is at.
func revisionParam(v url.Values) int {
	rev, err := strconv.Atoi(v.Get("revision"))
	if err != nil {
		return -1
	}
	return rev
}

// staleForm is the error for a form sent from a page rendered before the case
// last changed. The change may be the same form sent a moment earlier, so it
// does not say that nothing was recorded.
const staleForm = "this case changed after the page was loaded, so this was not recorded. check the thread before trying again"

// refuse shows the case again with the reason a post wrote nothing. A stale
// post is shown the case as it is on disk now, not as the handler loaded it:
// the event that made the post stale can land after that load. The page then
// carries the case's current revision, so the form can be sent again.
func (s *Server) refuse(w http.ResponseWriter, c *store.Case, cases []*store.Case, err error, form url.Values) {
	if !errors.Is(err, store.ErrStale) {
		s.renderCase(w, http.StatusUnprocessableEntity, c, cases, false, err.Error(), form)
		return
	}
	if now, lerr := store.Load(c.Dir); lerr == nil {
		c = now
	}
	s.renderCase(w, http.StatusConflict, c, cases, false, staleForm, form)
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
		// Park is its own button, not an answer; stuck=park is still read.
		if f.Get("park") != "" || f.Get("stuck") == "park" {
			return rec, true, nil
		}
		choice := f.Get("stuck")
		if choice == "" && strings.TrimSpace(f.Get("text")) != "" {
			// Guidance typed without ticking its button still means guidance.
			choice = "text"
		}
		switch choice {
		case "text":
			if rec.Text = strings.TrimSpace(f.Get("text")); rec.Text == "" {
				return rec, false, errors.New("write the guidance, or choose drop")
			}
		case "drop":
			rec.Drop = true
		default:
			return rec, false, errors.New("choose guidance or drop, or park it")
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
