package web

import (
	"bufio"
	"context"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// eventsAttr is where a page names its event stream.
var eventsAttr = regexp.MustCompile(`<body data-events="/events\?after=([0-9a-f]{16})">`)

// pageVersion returns the version a page's event stream starts from.
func pageVersion(t *testing.T, page string) string {
	t.Helper()
	m := eventsAttr.FindStringSubmatch(page)
	if m == nil {
		t.Fatalf("page has no event stream:\n%s", page)
	}
	return m[1]
}

// testStream is an event stream a test holds open.
type testStream struct {
	ids    chan string // the id of each message, in order
	ended  chan error  // the read's error once the stream ends; nil at a clean end
	cancel context.CancelFunc
}

// openStream opens the event stream at url, as the browser does, sending
// lastID as Last-Event-ID unless it is empty. The stream is closed when the
// test ends.
func openStream(t *testing.T, url, lastID string) *testStream {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	req, err := http.NewRequestWithContext(ctx, "GET", url, nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Host = testAddr
	if lastID != "" {
		req.Header.Set("Last-Event-ID", lastID)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusOK || resp.Header.Get("Content-Type") != "text/event-stream" || resp.Header.Get("Cache-Control") != "no-store" {
		cancel()
		resp.Body.Close()
		t.Fatalf("GET %s: %d, headers %v", url, resp.StatusCode, resp.Header)
	}
	s := &testStream{ids: make(chan string, 16), ended: make(chan error, 1), cancel: cancel}
	go func() {
		defer resp.Body.Close()
		sc := bufio.NewScanner(resp.Body)
		var id string
		for sc.Scan() {
			line := sc.Text()
			switch {
			case strings.HasPrefix(line, "id: "):
				id = strings.TrimPrefix(line, "id: ")
			case line == "data: "+id:
			case line == "":
				s.ids <- id
				id = ""
			default:
				s.ids <- "unexpected line " + line
			}
		}
		s.ended <- sc.Err()
	}()
	t.Cleanup(cancel)
	return s
}

// next returns the id of the stream's next message.
func (s *testStream) next(t *testing.T) string {
	t.Helper()
	select {
	case id := <-s.ids:
		return id
	case err := <-s.ended:
		t.Fatalf("the stream ended (%v) before a message", err)
	case <-time.After(5 * time.Second):
		t.Fatal("no message on the stream")
	}
	return ""
}

// serveApp serves the app on a real listener, for the tests that need a
// connection, and ends its streams before the server closes.
func serveApp(t *testing.T, a *testApp, ts *httptest.Server) string {
	t.Helper()
	if ts == nil {
		ts = httptest.NewServer(a.handler)
	} else {
		ts.Start()
	}
	// Cleanups run last first: a stream that failed to end must not hang
	// Close, which waits for every request.
	t.Cleanup(ts.Close)
	t.Cleanup(a.server.EndStreams)
	return ts.URL
}

// A page's event stream sends the store's version each time it changes, and
// not the version the page was drawn from. A page that reconnects says which
// version it had, and one that missed a change is sent it at once.
func TestEventStreamSendsEachChange(t *testing.T) {
	a := newApp(t)
	base := serveApp(t, a, nil)
	c := a.open(t, openRecords[store.KindFYI])
	v1 := pageVersion(t, a.get(t, "/"))
	s := openStream(t, base+"/events?after="+v1, "")

	// An answer from the CLI, found by serve's next poll.
	if _, err := a.db.Answer(t.Context(), c.ID, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	if err := a.server.Poll(); err != nil {
		t.Fatal(err)
	}
	v2 := pageVersion(t, a.get(t, "/"))
	if got := s.next(t); v2 == v1 || got != v2 {
		t.Fatalf("first message %q, want the version after the answer %q (the page's was %q)", got, v2, v1)
	}

	// A reconnect sends the last id it had, which counts over the page's.
	again := openStream(t, base+"/events?after="+v1, v2)
	a.open(t, openRecords[store.KindDecision])
	if err := a.server.Poll(); err != nil {
		t.Fatal(err)
	}
	v3 := pageVersion(t, a.get(t, "/"))
	for name, st := range map[string]*testStream{"open stream": s, "reconnected stream": again} {
		if got := st.next(t); got != v3 {
			t.Errorf("%s: message %q, want %q", name, got, v3)
		}
	}
	// A page drawn before changes it has not been sent is sent the version
	// at once.
	if got := openStream(t, base+"/events?after="+v1, "").next(t); got != v3 {
		t.Errorf("stale page: message %q, want %q", got, v3)
	}
}

// A stream holds each message back streamGap after the one before, then
// sends the version as it is by then: a burst of writes, as from a sweep,
// makes one refresh, not one a write.
func TestEventStreamHoldsMessagesApart(t *testing.T) {
	a := newApp(t)
	base := serveApp(t, a, nil)
	c := a.open(t, openRecords[store.KindQuestion])
	s := openStream(t, base+"/events?after="+pageVersion(t, a.get(t, "/")), "")
	note := func() {
		t.Helper()
		if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "More."}); err != nil {
			t.Fatal(err)
		}
		if err := a.server.Poll(); err != nil {
			t.Fatal(err)
		}
	}
	note()
	s.next(t)
	first := time.Now()
	for range 3 {
		note()
	}
	last := pageVersion(t, a.get(t, "/"))
	if got := s.next(t); got != last {
		t.Errorf("message after the burst = %q, want the version after it, %q", got, last)
	}
	// Measured from when the first arrived, which is after it was sent.
	if waited := time.Since(first); waited < streamGap-100*time.Millisecond {
		t.Errorf("second message %v after the first, want %v", waited, streamGap)
	}
}

// The version depends only on the store, so a page keeps it across a
// restart of serve and is sent nothing if nothing changed meanwhile.
func TestEventVersionOutlivesTheServer(t *testing.T) {
	a := newApp(t)
	a.open(t, openRecords[store.KindFYI])
	s, err := New(a.db, testAddr, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	b := &testApp{db: a.db, server: s, handler: s.Handler()}
	if va, vb := pageVersion(t, a.get(t, "/")), pageVersion(t, b.get(t, "/")); va != vb {
		t.Errorf("two servers on one store give versions %s and %s", va, vb)
	}
}

// The stream's handler returns when the page goes away, as when its tab is
// closed: the server then closes the connection, which it cannot do while
// the handler runs.
func TestEventStreamEndsWhenThePageGoes(t *testing.T) {
	a := newApp(t)
	gone := make(chan struct{})
	var once sync.Once
	ts := httptest.NewUnstartedServer(a.handler)
	ts.Config.ConnState = func(_ net.Conn, st http.ConnState) {
		if st == http.StateClosed {
			once.Do(func() { close(gone) })
		}
	}
	base := serveApp(t, a, ts)
	s := openStream(t, base+"/events?after="+pageVersion(t, a.get(t, "/")), "")
	s.cancel()
	select {
	case <-gone:
	case <-time.After(5 * time.Second):
		t.Fatal("the stream's handler did not end when the page went away")
	}
}

// Serve, given EndStreams, ends the open streams as it shuts down, so it
// does not wait out its shutdown timeout on them, and the streams end
// cleanly: the browser then reconnects to the next serve.
func TestServeEndsOpenStreams(t *testing.T) {
	a := newApp(t)
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	served := make(chan error, 1)
	go func() { served <- Serve(ctx, ln, a.handler, a.server.EndStreams) }()
	s := openStream(t, "http://"+ln.Addr().String()+"/events?after=0000000000000000", "")
	s.next(t) // the version, at once, as for any page drawn before it

	cancel()
	select {
	case err := <-served:
		if err != nil {
			t.Errorf("Serve returned %v", err)
		}
	case <-time.After(4 * time.Second): // under Serve's 5s shutdown timeout
		t.Fatal("Serve waited on the open stream")
	}
	select {
	case err := <-s.ended:
		if err != nil {
			t.Errorf("the stream was cut, not ended: %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the stream did not end")
	}

	// A stream opened once they have been ended ends at once, with the
	// headers that make the browser try again rather than give up.
	w := a.do("GET", "/events?after=0000000000000000", nil, nil)
	if w.Code != http.StatusOK || w.Header().Get("Content-Type") != "text/event-stream" {
		t.Errorf("stream after EndStreams: %d, headers %v", w.Code, w.Header())
	}
}

// Every page that shows the inbox or a case names its event stream, and the
// regions that follow the store refresh on the event prefs.js fires for each
// message, not on a timer. /done does not follow the store. The stream is
// not logged, as the refreshes are not.
func TestPagesFollowTheStore(t *testing.T) {
	a := newApp(t)
	empty := a.get(t, "/")
	pageVersion(t, empty)
	if want := `hx-get="/fragments/inbox?empty=1" hx-trigger="store-changed from:body, every 60s" hx-sync="this:replace"`; !strings.Contains(empty, want) {
		t.Errorf("inbox zero lacks %s", want)
	}
	open := a.open(t, openRecords[store.KindDecision])
	done := a.through(t, store.KindFYI, "Answered", a.answerAt(time.Now()))
	pages := map[string]string{
		"/":                   a.get(t, "/"),
		"/cases/" + open.ID:   a.get(t, "/cases/"+open.ID),
		"/cases/" + done.ID:   a.get(t, "/cases/"+done.ID),
		"refused answer page": a.do("POST", "/cases/"+open.ID+"/answer", withRevision(url.Values{"choice": {"1"}}, 0), nil).Body.String(),
	}
	for name, page := range pages {
		pageVersion(t, page)
		if strings.Contains(page, "every 2s") {
			t.Errorf("%s still polls every two seconds", name)
		}
		for _, trigger := range regexp.MustCompile(`hx-trigger="([^"]*)"`).FindAllStringSubmatch(page, -1) {
			if !strings.HasPrefix(trigger[1], "store-changed from:body") {
				t.Errorf("%s has a region that does not refresh on the stream: %s", name, trigger[0])
			}
		}
	}
	if body := a.get(t, "/done"); strings.Contains(body, "data-events") || strings.Contains(body, "hx-trigger") {
		t.Errorf("/done follows the store:\n%s", body)
	}

	// prefs.js opens the stream only in a tab that is shown and passes each
	// version to the other tabs, and once a form is sent it drops the
	// refreshes, so the one its own write sets off cannot send the browser
	// elsewhere mid-post.
	js := a.get(t, "/static/prefs.js")
	for _, want := range []string{
		`document.body.dataset.events`, `new EventSource(at.pathname + at.search)`, `document.body.dispatchEvent(new CustomEvent("store-changed"))`,
		`new BroadcastChannel("cases-store")`, `if (stream || (channel && document.hidden)) return;`, `document.addEventListener("visibilitychange"`,
		`leaving = setTimeout(stay, 10000);`, `["htmx:beforeRequest", "htmx:beforeOnLoad"]`, `if (leaving) e.preventDefault();`,
	} {
		if !strings.Contains(js, want) {
			t.Errorf("prefs.js lacks %s", want)
		}
	}

	// A page that has gone by the time its stream is served: the handler
	// returns, and the log line would be written, before ServeHTTP does.
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	r := httptest.NewRequestWithContext(ctx, "GET", "/events?after="+pageVersion(t, pages["/"]), nil)
	r.Host = testAddr
	a.handler.ServeHTTP(httptest.NewRecorder(), r)
	if log := a.log.String(); strings.Contains(log, "/events") {
		t.Errorf("the stream is logged:\n%s", log)
	}
}
