package web

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"maps"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

const testAddr = "127.0.0.1:8765"

// lockedBuffer is a log sink safe for concurrent handlers.
type lockedBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *lockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

type testApp struct {
	root    string
	server  *Server
	handler http.Handler
	log     *lockedBuffer
}

func newApp(t *testing.T) *testApp {
	t.Helper()
	return newAppWith(t, func(s store.Store) store.Store { return s })
}

// newAppWith is newApp with the server's store wrapped by wrap, such as to
// make one method fail. Test helpers such as open still write to the
// directory.
func newAppWith(t *testing.T, wrap func(store.Store) store.Store) *testApp {
	t.Helper()
	root := t.TempDir()
	log := &lockedBuffer{}
	s, err := New(wrap(store.NewDir(root)), testAddr, log)
	if err != nil {
		t.Fatal(err)
	}
	return &testApp{root: root, server: s, handler: s.Handler(), log: log}
}

// do sends a request addressed to the server's own host unless headers say
// otherwise.
func (a *testApp) do(method, target string, form url.Values, headers map[string]string) *httptest.ResponseRecorder {
	var body io.Reader
	if form != nil {
		body = strings.NewReader(form.Encode())
	}
	r := httptest.NewRequest(method, target, body)
	r.Host = testAddr
	if form != nil {
		r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	for k, v := range headers {
		if k == "Host" {
			r.Host = v
			continue
		}
		r.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	a.handler.ServeHTTP(w, r)
	return w
}

func (a *testApp) get(t *testing.T, target string) string {
	t.Helper()
	w := a.do("GET", target, nil, nil)
	if w.Code != http.StatusOK {
		t.Fatalf("GET %s: %d %s", target, w.Code, w.Body.String())
	}
	return w.Body.String()
}

func (a *testApp) open(t *testing.T, rec store.OpenRecord) *store.Case {
	t.Helper()
	c, err := store.Create(a.root, rec)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// withRevision returns a copy of form carrying rev, as the form on a page
// rendered from the case at that revision does. form may be nil.
func withRevision(form url.Values, rev int) url.Values {
	out := url.Values{}
	maps.Copy(out, form)
	out.Set("revision", strconv.Itoa(rev))
	return out
}

// revisionField is the hidden field a page's form carries the revision in.
var revisionField = regexp.MustCompile(`<input type="hidden" name="revision" value="(\d+)">`)

// pageRevision returns the revision the form on a page carries.
func pageRevision(t *testing.T, page string) int {
	t.Helper()
	m := revisionField.FindStringSubmatch(page)
	if m == nil {
		t.Fatalf("page has no revision field:\n%s", page)
	}
	rev, err := strconv.Atoi(m[1])
	if err != nil {
		t.Fatal(err)
	}
	return rev
}

func eventFiles(t *testing.T, dir string) []string {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, e := range entries {
		names = append(names, e.Name())
	}
	return names
}

func TestCheckLoopback(t *testing.T) {
	for _, ok := range []string{"127.0.0.1:8765", "127.0.0.1:0", "[::1]:9000", "localhost:8765", "127.1.2.3:80"} {
		if err := CheckLoopback(ok); err != nil {
			t.Errorf("%s refused: %v", ok, err)
		}
	}
	for _, bad := range []string{"0.0.0.0:8765", ":8765", "192.168.1.10:8765", "[::]:8765", "example.com:80", "127.0.0.1", "localhost:"} {
		if err := CheckLoopback(bad); err == nil {
			t.Errorf("%s accepted", bad)
		}
	}
	if _, err := New(store.NewDir(t.TempDir()), "0.0.0.0:8765", io.Discard); err == nil {
		t.Error("New accepted a non-loopback address")
	}
}

func TestHostSpellings(t *testing.T) {
	for _, tc := range []struct{ listen, host string }{
		{"127.0.0.1:80", "127.0.0.1"},
		{"[::1]:80", "[::1]"},
		{"[0:0:0:0:0:0:0:1]:8765", "[::1]:8765"},
	} {
		s, err := New(store.NewDir(t.TempDir()), tc.listen, io.Discard)
		if err != nil {
			t.Fatal(err)
		}
		r := httptest.NewRequest("GET", "/", nil)
		r.Host = tc.host
		w := httptest.NewRecorder()
		s.Handler().ServeHTTP(w, r)
		if w.Code != http.StatusOK {
			t.Errorf("listen %s, Host %s: %d", tc.listen, tc.host, w.Code)
		}
	}
}

func TestSecurityHeaders(t *testing.T) {
	a := newApp(t)
	w := a.do("GET", "/", nil, nil)
	if csp := w.Header().Get("Content-Security-Policy"); !strings.HasPrefix(csp, "default-src 'self'") {
		t.Errorf("CSP = %q", csp)
	}
	if strings.Contains(w.Body.String(), "<script>") {
		t.Error("page has an inline script")
	}
}

func TestHostMustBeTheServer(t *testing.T) {
	a := newApp(t)
	for _, host := range []string{"evil.example:8765", "127.0.0.1:9999", "192.168.1.10:8765"} {
		if w := a.do("GET", "/", nil, map[string]string{"Host": host}); w.Code != http.StatusForbidden {
			t.Errorf("Host %s: %d", host, w.Code)
		}
	}
	for _, host := range []string{"127.0.0.1:8765", "localhost:8765", "LOCALHOST:8765"} {
		if w := a.do("GET", "/", nil, map[string]string{"Host": host}); w.Code != http.StatusOK {
			t.Errorf("Host %s: %d", host, w.Code)
		}
	}
}

func TestCrossSitePostsAreRejected(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Heads up"})
	form := withRevision(url.Values{"ack": {"1"}}, 1)
	target := "/cases/" + c.ID + "/answer"

	for name, headers := range map[string]map[string]string{
		"foreign origin":       {"Origin": "http://evil.example"},
		"origin on other port": {"Origin": "http://127.0.0.1:9999"},
		"null origin":          {"Origin": "null"},
		"cross-site fetch":     {"Sec-Fetch-Site": "cross-site"},
		"same-site fetch":      {"Sec-Fetch-Site": "same-site"},
		"rebinding host":       {"Host": "evil.example:8765", "Origin": "http://evil.example:8765"},
	} {
		if w := a.do("POST", target, form, headers); w.Code != http.StatusForbidden {
			t.Errorf("%s: status %d", name, w.Code)
		}
	}
	if files := eventFiles(t, c.Dir); len(files) != 1 {
		t.Fatalf("rejected posts wrote files: %v", files)
	}

	w := a.do("POST", target, form, map[string]string{"Origin": "http://" + testAddr, "Sec-Fetch-Site": "same-origin"})
	if w.Code != http.StatusSeeOther {
		t.Fatalf("same-origin post: %d %s", w.Code, w.Body.String())
	}
}

func TestStaticHTMXIsTheRecordedRelease(t *testing.T) {
	a := newApp(t)
	w := a.do("GET", "/static/htmx.min.js", nil, nil)
	if w.Code != http.StatusOK || !strings.Contains(w.Header().Get("Content-Type"), "javascript") {
		t.Fatalf("status %d, type %q", w.Code, w.Header().Get("Content-Type"))
	}
	sum := sha256.Sum256(w.Body.Bytes())
	if got := hex.EncodeToString(sum[:]); got != htmxSHA256 {
		t.Errorf("htmx.min.js sha256 = %s, want %s", got, htmxSHA256)
	}
	if w := a.do("GET", "/static/style.css", nil, nil); w.Code != http.StatusOK {
		t.Errorf("style.css: %d", w.Code)
	}
}

// TestStyleHasViewTransitions checks that page changes opt in to cross-document
// view transitions and that reduced motion turns them off again.
func TestStyleHasViewTransitions(t *testing.T) {
	a := newApp(t)
	css := a.do("GET", "/static/style.css", nil, nil).Body.String()
	on := strings.Index(css, "@view-transition { navigation: auto; }")
	off := strings.Index(css, "@media (prefers-reduced-motion: reduce) {\n  @view-transition { navigation: none; }\n}")
	if on < 0 || off < 0 || off < on {
		t.Errorf("style.css lacks the view transition rule, or the reduced-motion guard after it (at %d and %d)", on, off)
	}
	for _, want := range []string{"header.top { view-transition-name: masthead; }", "::view-transition-group(masthead) { animation: none; }", ".split > .list { view-transition-name: inbox-list; }"} {
		if !strings.Contains(css, want) {
			t.Errorf("style.css lacks %s", want)
		}
	}
}

// TestStyleScalesWithTextSize keeps lengths in rem, so the size option, which
// sets the root size, scales the whole page. Hairline rules, offsets and media
// query breakpoints (which the root size does not move) may stay in px.
func TestStyleScalesWithTextSize(t *testing.T) {
	a := newApp(t)
	css := a.do("GET", "/static/style.css", nil, nil).Body.String()
	for _, want := range []string{`:root[data-size="small"] { font-size: 87.5%; }`, `:root[data-size="large"] { font-size: 125%; }`} {
		if !strings.Contains(css, want) {
			t.Errorf("style.css lacks %s", want)
		}
	}
	// Blank the comments but keep their newlines, so line numbers hold.
	css = regexp.MustCompile(`(?s)/\*.*?\*/`).ReplaceAllStringFunc(css, func(c string) string {
		return strings.Repeat("\n", strings.Count(c, "\n"))
	})
	for i, line := range strings.Split(css, "\n") {
		if strings.Contains(line, "@media") {
			continue
		}
		for _, px := range regexp.MustCompile(`-?\d*\.?\d+px`).FindAllString(line, -1) {
			switch px {
			case "1px", "2px", "-1px", "-3px", "-5px":
			case "16px":
				if strings.Contains(line, "max(1rem, 16px)") {
					continue
				}
				fallthrough
			default:
				t.Errorf("style.css line %d uses %s, want rem: %s", i+1, px, strings.TrimSpace(line))
			}
		}
	}
}

func TestMarkdownCannotInjectMarkup(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{
		Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: `<b>title</b>`,
		Body:  "# Heading\n\n<script>alert(1)</script>\n\n[click](javascript:alert(1)) [ok](https://example.com)\n\n<img src=x onerror=alert(1)>",
		Links: []string{"javascript:alert(2)"},
	})
	body := a.get(t, "/cases/"+c.ID)
	for _, bad := range []string{"<script>alert", `href="javascript`, "onerror=", "<b>title</b>"} {
		if strings.Contains(body, bad) {
			t.Errorf("page contains %q", bad)
		}
	}
	for _, good := range []string{"<h1>Heading</h1>", `<a href="https://example.com" target="_blank" rel="noopener noreferrer">ok</a>`, "&lt;b&gt;title&lt;/b&gt;", "<li>javascript:alert(2)</li>"} {
		if !strings.Contains(body, good) {
			t.Errorf("page is missing %q", good)
		}
	}
}

func TestLabelsAreShownAsText(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{
		Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Labelled",
		Labels: []string{"feat-labels", `<i>round</i> "3"`},
	})
	want := []string{`<span class="tag">feat-labels</span>`, `<span class="tag">&lt;i&gt;round&lt;/i&gt; &#34;3&#34;</span>`}
	for _, page := range []string{"/", "/fragments/inbox", "/cases/" + c.ID} {
		body := a.get(t, page)
		if strings.Contains(body, "<i>round</i>") {
			t.Errorf("%s has the label unescaped", page)
		}
		for _, w := range want {
			// The case page also lists the case in the inbox beside it.
			if n := strings.Count(body, w); n == 0 || page == "/cases/"+c.ID && n != 2 {
				t.Errorf("%s has %q %d times", page, w, n)
			}
		}
	}

	if _, err := store.Withdraw(c.Dir, store.WithdrawRecord{}); err != nil {
		t.Fatal(err)
	}
	body := a.get(t, "/done")
	if !strings.Contains(body, c.ID) || strings.Contains(body, "<i>round</i>") {
		t.Errorf("/done does not list the case, or has the label unescaped:\n%s", body)
	}
	for _, w := range want {
		if n := strings.Count(body, w); n != 1 {
			t.Errorf("/done has %q %d times", w, n)
		}
	}
}

func TestRequestsAreLogged(t *testing.T) {
	a := newApp(t)
	a.get(t, "/done")
	w := a.do("GET", "/fragments/inbox", nil, map[string]string{"HX-Request": "true"})
	if w.Code != http.StatusOK {
		t.Fatal(w.Code)
	}
	a.log.mu.Lock()
	log := a.log.buf.String()
	a.log.mu.Unlock()
	if !strings.Contains(log, "GET /done 200") || strings.Contains(log, "/fragments/inbox") {
		t.Errorf("log = %q", log)
	}
}

func TestMissingStoreShowsAnEmptyInbox(t *testing.T) {
	s, err := New(store.NewDir(filepath.Join(t.TempDir(), "not-yet")), testAddr, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRequest("GET", "/", nil)
	r.Host = testAddr
	w := httptest.NewRecorder()
	s.Handler().ServeHTTP(w, r)
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "<h1>inbox zero.</h1>") || !strings.Contains(w.Body.String(), "nothing has needed you yet.") {
		t.Errorf("%d %s", w.Code, w.Body.String())
	}
}

func TestOptionsOverlayAndPrefsScript(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindFYI])
	w := a.do("GET", "/static/prefs.js", nil, nil)
	if w.Code != http.StatusOK || !strings.Contains(w.Header().Get("Content-Type"), "javascript") {
		t.Fatalf("prefs.js: status %d, type %q", w.Code, w.Header().Get("Content-Type"))
	}
	// The size option's first value is the default, the one that leaves
	// data-size off, so it must be the step the dialog marks checked.
	if !strings.Contains(w.Body.String(), `{ name: "size", values: ["medium", "small", "large"] }`) {
		t.Error("prefs.js lacks the size option with medium as its default")
	}

	for _, target := range []string{"/", "/done", "/cases/" + c.ID} {
		w := a.do("GET", target, nil, nil)
		body := w.Body.String()
		dialog := body[strings.Index(body, `<dialog id="options-dialog"`)+1:]
		if !strings.Contains(body, `<dialog id="options-dialog"`) {
			t.Fatalf("%s: no options dialog", target)
		}
		for _, want := range []string{
			`<form id="options" method="dialog">`,
			`name="theme" value="system" checked`, `name="theme" value="light"`, `name="theme" value="dark"`,
			`name="face" value="mono" checked`, `name="face" value="sans"`, `name="face" value="serif"`,
			`name="size" value="small"`, `name="size" value="medium" checked`, `name="size" value="large"`,
			`name="links" value="new" checked`, `name="links" value="same"`,
			`<button type="button" id="options-reset">reset</button>`,
			`<button type="submit">close</button>`,
		} {
			if !strings.Contains(dialog, want) {
				t.Errorf("%s: dialog missing %s", target, want)
			}
		}
		// The chip opens the dialog; it is not a view, so it is never current.
		if !strings.Contains(body, `<button type="button" id="options-open" aria-haspopup="dialog" aria-controls="options-dialog">options</button>`) {
			t.Errorf("%s: no options chip", target)
		}
		// The script is a file loaded before the stylesheet, so stored choices
		// apply before paint, and the page still has no inline script.
		script := strings.Index(body, `<script src="/static/prefs.js"></script>`)
		if script < 0 || script > strings.Index(body, `<link rel="stylesheet"`) || strings.Contains(body, "<script>") || strings.Contains(body, " onclick=") {
			t.Errorf("%s: prefs.js not loaded before the stylesheet, or an inline script", target)
		}
		if csp := w.Header().Get("Content-Security-Policy"); !strings.HasPrefix(csp, "default-src 'self'") || strings.Contains(csp, "unsafe") {
			t.Errorf("%s: CSP = %q", target, csp)
		}
	}
	if w := a.do("GET", "/options", nil, nil); w.Code != http.StatusNotFound {
		t.Errorf("/options: %d, want 404 now that options are an overlay", w.Code)
	}
}
