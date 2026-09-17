package web

import (
	"bytes"
	"crypto/sha256"
	"embed"
	"encoding/hex"
	"encoding/json"
	"errors"
	"html/template"
	"io/fs"
	"net/http"
	"net/url"
	"slices"
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

// textID returns a short hex hash of s, for an element id that must change
// when the text in the element does. Hex keeps the id a CSS selector that
// needs no escaping, which is how htmx looks up a preserved element.
func textID(s string) string {
	sum := sha256.Sum256([]byte(s))
	return hex.EncodeToString(sum[:8])
}

// isWebLink reports whether a case or row link is an http or https URL. Other
// links, such as a filesystem path, are shown as text: they would not open.
func isWebLink(s string) bool {
	lower := strings.ToLower(s)
	return strings.HasPrefix(lower, "http://") || strings.HasPrefix(lower, "https://")
}

// zone is the time zone pages show timestamps in: the serve process's local
// zone. The store keeps them in UTC. Tests pin it.
var zone = time.Local

var funcs = template.FuncMap{
	"markdown":   renderMarkdown,
	"pathEscape": url.PathEscape,
	"textID":     textID,
	"webLink":    isWebLink,
	"stamp": func(t time.Time) string {
		if t.IsZero() {
			return ""
		}
		return t.In(zone).Format("2006-01-02 15:04 MST")
	},
	// age is how long ago t was, as a phrase: "5m ago", or "now".
	"age": func(t time.Time) string {
		if t.IsZero() {
			return ""
		}
		if a := Age(t, time.Now()); a != "now" {
			return a + " ago"
		}
		return "now"
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
	pages["case"] = template.Must(template.New("layout.html").Funcs(funcs).ParseFS(templateFS, "templates/layout.html", "templates/inbox.html", "templates/done-list.html", "templates/case.html"))
	pages["done"] = template.Must(template.New("layout.html").Funcs(funcs).ParseFS(templateFS, "templates/layout.html", "templates/done-list.html", "templates/done.html"))
}

// page is what every full page carries for the layout.
type page struct {
	Title string
	tally
	Nav string // the header link to mark as current
}

// tally is the header's counts: open cases by urgency, and parked cases.
type tally struct {
	Blocking, Today, Whenever, Parked int
}

// Waiting counts the open cases, of any urgency: the ones waiting on the
// human. Parked cases wait on nobody. The page title leads with it.
func (t tally) Waiting() int {
	return t.Blocking + t.Today + t.Whenever
}

// tallyItem is one figure on the header line.
type tallyItem struct {
	N     int
	Label string
	Hot   bool
}

// Items returns the non-zero counts in header order. Only blocking is hot:
// the open blocking cases are the ones an agent is idle on.
func (t tally) Items() []tallyItem {
	var items []tallyItem
	for _, it := range []tallyItem{
		{t.Blocking, "blocking", true},
		{t.Today, "today", false},
		{t.Whenever, "whenever", false},
		{t.Parked, "parked", false},
	} {
		if it.N > 0 {
			items = append(items, it)
		}
	}
	return items
}

func countTally(cases []*store.Case) tally {
	var t tally
	for _, c := range cases {
		switch {
		case c.State == store.StateParked:
			t.Parked++
		case c.State != store.StateOpen:
		case c.Urgency == store.UrgencyBlocking:
			t.Blocking++
		case c.Urgency == store.UrgencyToday:
			t.Today++
		case c.Urgency == store.UrgencyWhenever:
			t.Whenever++
		}
	}
	return t
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
	// Zero is what was got through, set with Empty while the inbox is empty.
	Zero *zeroStats
}

// clock is the clock for the inbox-zero figures and the done page. Tests pin it.
var clock = time.Now

// startOfDay is midnight at the start of t's day, in the zone pages show times in.
func startOfDay(t time.Time) time.Time {
	t = t.In(zone)
	return time.Date(t.Year(), t.Month(), t.Day(), 0, 0, 0, 0, zone)
}

// inFlight reports whether c is answered or picked up: an agent has it and
// has not closed it.
func inFlight(c *store.Case) bool {
	return c.State == store.StateAnswered || c.State == store.StatePickedUp
}

// closedSince reports whether c is closed by a close event at or after t.
func closedSince(c *store.Case, t time.Time) bool {
	if c.State != store.StateClosed {
		return false
	}
	for _, ev := range c.Events {
		if ev.Type == store.EventClose && !ev.At.Before(t) {
			return true
		}
	}
	return false
}

// zeroStats is what the inbox-zero panel says. Day and week start at
// midnight, and on Monday, in the zone pages show times in.
type zeroStats struct {
	// Today and Week count the cases with a human answer, park or resume in
	// that span.
	Today, Week int
	// Closed counts the cases closed today.
	Closed int
	// WithAgent counts answered and picked-up cases: the agent has not closed them.
	WithAgent int
	// LastAnswer is how long ago the human last answered a case, as Age
	// gives it; empty if never.
	LastAnswer string
}

// setEmpty marks d as / with no case to show, with the figures for the panel.
func (d *inboxData) setEmpty(cases []*store.Case) {
	d.Empty = true
	if len(d.Cards) > 0 {
		return
	}
	t := clock().In(zone)
	day := startOfDay(t)
	week := day.AddDate(0, 0, -(int(day.Weekday())+6)%7)
	z := &zeroStats{}
	var last time.Time
	for _, c := range cases {
		if inFlight(c) {
			z.WithAgent++
		}
		if closedSince(c, day) {
			z.Closed++
		}
		var today, thisWeek bool
		for _, ev := range c.Events {
			switch {
			case ev.Author != store.AuthorHuman:
			case ev.Type == store.EventAnswer || ev.Type == store.EventPark || ev.Type == store.EventResume:
				today = today || !ev.At.Before(day)
				thisWeek = thisWeek || !ev.At.Before(week)
				if ev.Type == store.EventAnswer && ev.At.After(last) {
					last = ev.At
				}
			}
		}
		if today {
			z.Today++
		}
		if thisWeek {
			z.Week++
		}
	}
	if !last.IsZero() {
		z.LastAnswer = Age(last, t)
	}
	d.Zero = z
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
	d := inboxData{page: page{Title: "inbox", tally: countTally(cases), Nav: "inbox"}, Selected: selected}
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

// recorded is what a successful answer, park or resume wrote, for the page
// the post redirects to to confirm.
type recorded struct {
	Event store.EventType
	Case  *store.Case
	// Dismiss is the page's own URL without the query withRecorded added,
	// so following it drops the line and keeps everything else.
	Dismiss string
}

// withRecorded adds the event just written on the case with id to target, the
// page a post redirects to.
func withRecorded(target string, event store.EventType, id string) string {
	return target + "?" + url.Values{"recorded": {id}, "event": {string(event)}}.Encode()
}

// findRecorded resolves the query withRecorded wrote on u against the loaded
// cases. It returns nil unless the event is one a form writes and the id names
// a loaded case, so a made-up query shows nothing.
func findRecorded(u *url.URL, cases []*store.Case) *recorded {
	q := u.Query()
	event := store.EventType(q.Get("event"))
	switch event {
	case store.EventAnswer, store.EventPark, store.EventResume:
	default:
		return nil
	}
	id := q.Get("recorded")
	for _, c := range cases {
		if c.ID == id {
			q.Del("event")
			q.Del("recorded")
			dismiss := u.EscapedPath()
			if len(q) > 0 {
				dismiss += "?" + q.Encode()
			}
			return &recorded{Event: event, Case: c, Dismiss: dismiss}
		}
	}
	return nil
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
	s.renderCase(w, http.StatusOK, first, cases, true, findRecorded(r.URL, cases), "", nil)
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
		d.setEmpty(cases)
	}
	s.render(w, http.StatusOK, "case", "inbox-fragment", d)
}

// doneTitle is the title of a done case's page.
func doneTitle(c *store.Case) string {
	return c.Title + " · done"
}

// tallyFragment is polled by a done case's page, whose list does not poll,
// to keep the header tally and the title's count current. selected is the
// case the page shows.
func (s *Server) tallyFragment(w http.ResponseWriter, r *http.Request) {
	cases, err := s.cases()
	if err != nil {
		s.fail(w, err)
		return
	}
	p := page{Title: "done", tally: countTally(cases)}
	selected := r.URL.Query().Get("selected")
	for _, c := range cases {
		if c.ID == selected {
			p.Title = doneTitle(c)
			break
		}
	}
	s.render(w, http.StatusOK, "case", "tally-fragment", p)
}

type doneCard struct {
	*store.Case
	Age string
}

// flightCard is a case on the in-flight list: who has it, and the answer or
// pickup that put it there and how long ago.
type flightCard struct {
	*store.Case
	Who  string
	Last string // "answered" or "picked up"
	Age  string
	at   time.Time
}

// doneFilter is one chip at the top of /done.
type doneFilter struct {
	Show, Label string
	N           int
}

// Href is the chip's link. all is /done itself.
func (f doneFilter) Href() string {
	if f.Show == "all" {
		return "/done"
	}
	return "/done?show=" + f.Show
}

func newFlightCard(c *store.Case, now time.Time) flightCard {
	f := flightCard{Case: c, Who: c.Worker, Last: "answered"}
	if c.Answer != nil {
		f.at = c.Answer.AnsweredAt
	}
	if c.State == store.StatePickedUp && c.Pickup != nil {
		f.Last, f.at = "picked up", c.Pickup.PickedUpAt
		if c.Pickup.By != "" {
			f.Who = c.Pickup.By
		}
	}
	if f.Who == "" {
		f.Who = "an agent"
	}
	f.Age = Age(f.at, now)
	return f
}

// doneData is the done list: the filter chips and the cards under the one
// chosen. Selected is the id of the case beside the list on a case page, and
// is empty on /done.
type doneData struct {
	page
	Show     string
	Filters  []doneFilter
	Cards    []doneCard
	Flight   []flightCard
	Selected string
}

// newDone builds the done list. show picks a filter: inflight (answered or
// picked up), closed-today, or all, which is the default and what an unknown
// value gets.
func newDone(cases []*store.Case, show, selected string) doneData {
	now := clock()
	day := startOfDay(now)
	var all, closedToday []*store.Case
	var flight []flightCard
	for _, c := range cases {
		if isDone(c) {
			all = append(all, c)
		}
		if inFlight(c) {
			flight = append(flight, newFlightCard(c, now))
		}
		if closedSince(c, day) {
			closedToday = append(closedToday, c)
		}
	}
	d := doneData{
		page: page{Title: "done", tally: countTally(cases), Nav: "done"},
		Filters: []doneFilter{
			{"inflight", "in flight", len(flight)},
			{"closed-today", "closed today", len(closedToday)},
			{"all", "all", len(all)},
		},
		Selected: selected,
	}
	shown := all
	switch show {
	case "inflight":
		d.Title = "in flight · done"
		slices.SortStableFunc(flight, func(a, b flightCard) int { return b.at.Compare(a.at) })
		d.Flight = flight
	case "closed-today":
		d.Title = "closed today · done"
		shown = closedToday
	default:
		show = "all"
	}
	d.Show = show
	if show != "inflight" {
		slices.SortStableFunc(shown, func(a, b *store.Case) int { return b.UpdatedAt.Compare(a.UpdatedAt) })
		for _, c := range shown {
			d.Cards = append(d.Cards, doneCard{Case: c, Age: Age(c.UpdatedAt, now)})
		}
	}
	return d
}

// isDone reports whether c is one the human is through with: answered, picked
// up, closed or withdrawn.
func isDone(c *store.Case) bool {
	switch c.State {
	case store.StateAnswered, store.StatePickedUp, store.StateClosed, store.StateWithdrawn:
		return true
	}
	return false
}

// done lists the cases the human is through with, filtered by show.
func (s *Server) done(w http.ResponseWriter, r *http.Request) {
	cases, err := s.cases()
	if err != nil {
		s.fail(w, err)
		return
	}
	s.render(w, http.StatusOK, "done", "layout.html", newDone(cases, r.URL.Query().Get("show"), ""))
}

type threadEntry struct {
	store.Event
	Lines    []store.Line
	Markdown template.HTML
}

// caseData is the list column and the working area. Case is nil when the
// inbox is empty. Home is set on /, where a stale thread reloads / instead.
// Done is set in place of Inbox when the case is done: the done list is
// beside it.
type caseData struct {
	page
	Inbox inboxData
	Done  *doneData
	Home  bool
	// Recorded is set when the page is where a post went after writing.
	Recorded *recorded
	Case     *store.Case
	Thread   []threadEntry
	Error    string
	Form     url.Values
}

func thread(c *store.Case) []threadEntry {
	var out []threadEntry
	for _, ev := range c.Events {
		e := threadEntry{Event: ev}
		switch ev.Type {
		case store.EventOpen:
			// The body is shown at the top of the page.
			e.Lines = store.ActorLines(ev)
		case store.EventNote:
			var n store.NoteRecord
			_ = json.Unmarshal(ev.Data, &n)
			e.Lines = store.ActorLines(ev)
			e.Markdown = renderMarkdown(n.Body)
		case store.EventClose:
			var cl store.CloseRecord
			_ = json.Unmarshal(ev.Data, &cl)
			e.Markdown = renderMarkdown(cl.Outcome)
			e.Lines = store.ActorLines(ev)
			for _, l := range cl.Links {
				e.Lines = append(e.Lines, store.Line{Text: l})
			}
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
	s.renderCase(w, http.StatusOK, c, cases, false, findRecorded(r.URL, cases), "", nil)
}

func (s *Server) renderCase(w http.ResponseWriter, status int, c *store.Case, cases []*store.Case, home bool, rec *recorded, msg string, form url.Values) {
	d := caseData{Home: home, Recorded: rec, Error: msg, Form: form}
	switch {
	case c != nil && isDone(c):
		// A done case sits beside the done list, under the filter it is on.
		// The list does not poll, as on /done, but the tally and title do;
		// the thread poll reloads the page when the case changes state,
		// which picks the list again.
		show := "all"
		if inFlight(c) {
			show = "inflight"
		}
		done := newDone(cases, show, c.ID)
		done.Title = doneTitle(c)
		d.Case, d.Thread, d.Done = c, thread(c), &done
		d.page = done.page
	case c != nil:
		d.Case, d.Thread = c, thread(c)
		d.Inbox = newInbox(cases, c.ID)
		d.Inbox.Title = c.Title
		d.page = d.Inbox.page
	default:
		d.Inbox = newInbox(cases, "")
		if home {
			d.Inbox.setEmpty(cases)
		}
		d.page = d.Inbox.page
	}
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

// loadForPost resolves the case a form posts to, straight from the store.
func (s *Server) loadForPost(w http.ResponseWriter, r *http.Request) (*store.Case, bool) {
	id := r.PathValue("id")
	if store.ValidID(id) != nil {
		http.NotFound(w, r)
		return nil, false
	}
	c, err := s.store.Get(r.Context(), id)
	if errors.Is(err, fs.ErrNotExist) {
		http.NotFound(w, r)
		return nil, false
	}
	if err != nil {
		s.fail(w, err)
		return nil, false
	}
	r.Body = http.MaxBytesReader(w, r.Body, maxForm)
	if err := r.ParseForm(); err != nil {
		http.Error(w, "bad form: "+err.Error(), http.StatusBadRequest)
		return nil, false
	}
	return c, true
}

func (s *Server) answer(w http.ResponseWriter, r *http.Request) {
	c, ok := s.loadForPost(w, r)
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
		s.refuse(w, r, c, cases, store.ErrStale, r.PostForm)
		return
	}
	rec, park, err := answerFromForm(c, r.PostForm)
	event := store.EventAnswer
	if err == nil {
		if park {
			event = store.EventPark
			_, err = s.store.Park(r.Context(), c.ID, store.ParkRecord{Note: rec.Note, Actor: s.Actor}, store.AtRevision(rev))
		} else {
			rec.Actor = s.Actor
			_, err = s.store.Answer(r.Context(), c.ID, rec, store.AtRevision(rev))
		}
	}
	if err != nil {
		s.refuse(w, r, c, cases, err, r.PostForm)
		return
	}
	http.Redirect(w, r, withRecorded(next, event, c.ID), http.StatusSeeOther)
}

func (s *Server) resume(w http.ResponseWriter, r *http.Request) {
	c, ok := s.loadForPost(w, r)
	if !ok {
		return
	}
	cases, _ := s.cases()
	next := nextCase(cases, c.ID)
	if _, err := s.store.Resume(r.Context(), c.ID, store.AuthorHuman, store.ResumeRecord{Actor: s.Actor}, store.AtRevision(revisionParam(r.PostForm))); err != nil {
		s.refuse(w, r, c, cases, err, nil)
		return
	}
	http.Redirect(w, r, withRecorded(next, store.EventResume, c.ID), http.StatusSeeOther)
}

// staleForm is the error for a form sent from a page rendered before the case
// last changed. The change may be the same form sent a moment earlier, so it
// does not say that nothing was recorded.
const staleForm = "this case changed after the page was loaded, so this was not recorded. check the thread before trying again"

// refuse shows the case again with the reason a post wrote nothing. A stale
// post is shown the case as it is in the store now, not as the handler loaded it:
// the event that made the post stale can land after that load. The page then
// carries the case's current revision, so the form can be sent again.
func (s *Server) refuse(w http.ResponseWriter, r *http.Request, c *store.Case, cases []*store.Case, err error, form url.Values) {
	if !errors.Is(err, store.ErrStale) {
		s.renderCase(w, http.StatusUnprocessableEntity, c, cases, false, nil, err.Error(), form)
		return
	}
	if now, lerr := s.store.Get(r.Context(), c.ID); lerr == nil {
		c = now
	}
	s.renderCase(w, http.StatusConflict, c, cases, false, nil, staleForm, form)
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
