package web

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
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
	handler http.Handler
	log     *lockedBuffer
}

func newApp(t *testing.T) *testApp {
	t.Helper()
	root := t.TempDir()
	log := &lockedBuffer{}
	s, err := New(root, testAddr, log)
	if err != nil {
		t.Fatal(err)
	}
	return &testApp{root: root, handler: s.Handler(), log: log}
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
	if _, err := New(t.TempDir(), "0.0.0.0:8765", io.Discard); err == nil {
		t.Error("New accepted a non-loopback address")
	}
}

func TestHostSpellings(t *testing.T) {
	for _, tc := range []struct{ listen, host string }{
		{"127.0.0.1:80", "127.0.0.1"},
		{"[::1]:80", "[::1]"},
		{"[0:0:0:0:0:0:0:1]:8765", "[::1]:8765"},
	} {
		s, err := New(t.TempDir(), tc.listen, io.Discard)
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
	form := url.Values{"ack": {"1"}}
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
	for _, good := range []string{"<h1>Heading</h1>", `<a href="https://example.com" target="_blank" rel="noopener noreferrer">ok</a>`, "&lt;b&gt;title&lt;/b&gt;", `href="#ZgotmplZ"`} {
		if !strings.Contains(body, good) {
			t.Errorf("page is missing %q", good)
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
	s, err := New(filepath.Join(t.TempDir(), "not-yet"), testAddr, io.Discard)
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRequest("GET", "/", nil)
	r.Host = testAddr
	w := httptest.NewRecorder()
	s.Handler().ServeHTTP(w, r)
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), "nothing waiting") || !strings.Contains(w.Body.String(), "no open cases.") {
		t.Errorf("%d %s", w.Code, w.Body.String())
	}
}
