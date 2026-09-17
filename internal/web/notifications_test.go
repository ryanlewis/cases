package web

import (
	"encoding/json"
	"fmt"
	"net/http"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/notify"
	"github.com/ryanlewis/cases/internal/store"
)

func (a *testApp) notifications(t *testing.T, target string) notify.Page {
	t.Helper()
	w := a.do("GET", target, nil, nil)
	if w.Code != http.StatusOK {
		t.Fatalf("GET %s: %d %s", target, w.Code, w.Body.String())
	}
	if ct := w.Header().Get("Content-Type"); ct != "application/json" {
		t.Errorf("GET %s: Content-Type %q", target, ct)
	}
	if cc := w.Header().Get("Cache-Control"); cc != "no-store" {
		t.Errorf("GET %s: Cache-Control %q", target, cc)
	}
	var p notify.Page
	if err := json.Unmarshal(w.Body.Bytes(), &p); err != nil {
		t.Fatalf("GET %s: %v\n%s", target, err, w.Body.String())
	}
	return p
}

func TestNotificationsFeed(t *testing.T) {
	a := newApp(t)
	a.open(t, openRecords[store.KindFYI])

	// The first poll records the store as it is.
	first := a.notifications(t, "/notifications")
	if first.Boot == "" || first.Latest != 0 || first.Items == nil || len(first.Items) != 0 {
		t.Fatalf("first page = %+v, want a boot id and no items", first)
	}

	rec := openRecords[store.KindDecision]
	rec.Urgency = store.UrgencyBlocking
	c := a.open(t, rec)
	p := a.notifications(t, "/notifications?after=0")
	if p.Boot != first.Boot || p.Latest != 1 || len(p.Items) != 1 {
		t.Fatalf("page = %+v, want one item on the same boot", p)
	}
	it := p.Items[0]
	if it.ID != 1 || it.Event.Name != notify.EventOpen || it.Title != rec.Title || it.URL != "/cases/"+c.ID || it.Tag != c.ID+"/0001-agent-open.json" {
		t.Errorf("item = %+v", it)
	}
	if it.Case == nil || it.Case.Urgency != store.UrgencyBlocking {
		t.Errorf("item case = %+v, want the case with its urgency", it.Case)
	}

	if p := a.notifications(t, "/notifications?after=1"); p.Latest != 1 || len(p.Items) != 0 {
		t.Errorf("after=1: %+v, want nothing new", p)
	}
	if a.server.Notified() != 1 {
		t.Errorf("Notified = %d, want 1", a.server.Notified())
	}
	// The tab's checks are polls: not logged when they succeed.
	if log := a.log.buf.String(); strings.Contains(log, "/notifications") {
		t.Errorf("log has the notifications polls:\n%s", log)
	}
}

func TestNotificationsKeepTheLastFifty(t *testing.T) {
	a := newApp(t)
	if err := a.server.Poll(); err != nil {
		t.Fatal(err)
	}
	rec := openRecords[store.KindFYI]
	for i := range notify.FeedSize + 3 {
		rec.Title = fmt.Sprintf("Case %d", i)
		a.open(t, rec)
		if err := a.server.Poll(); err != nil {
			t.Fatal(err)
		}
	}
	p := a.notifications(t, "/notifications")
	if p.Latest != notify.FeedSize+3 || len(p.Items) != notify.FeedSize || p.Items[0].ID != 4 {
		t.Errorf("latest %d, %d items, first id %d", p.Latest, len(p.Items), p.Items[0].ID)
	}
	if p := a.notifications(t, fmt.Sprintf("/notifications?after=%d", notify.FeedSize+1)); len(p.Items) != 2 {
		t.Errorf("after %d: %d items, want 2", notify.FeedSize+1, len(p.Items))
	}
}

func TestNotificationsBootIDChangesWithTheServer(t *testing.T) {
	a, b := newApp(t), newApp(t)
	if pa, pb := a.notifications(t, "/notifications"), b.notifications(t, "/notifications"); pa.Boot == pb.Boot {
		t.Errorf("two servers share boot id %q", pa.Boot)
	}
}

func TestNotificationsRefusals(t *testing.T) {
	a := newApp(t)
	for _, target := range []string{"/notifications?after=x", "/notifications?after=-1"} {
		if w := a.do("GET", target, nil, nil); w.Code != http.StatusBadRequest {
			t.Errorf("%s: %d, want 400", target, w.Code)
		}
	}
	if w := a.do("GET", "/notifications", nil, map[string]string{"Host": "evil.example:8765"}); w.Code != http.StatusForbidden {
		t.Errorf("foreign Host: %d, want 403", w.Code)
	}
	if w := a.do("POST", "/notifications", nil, nil); w.Code != http.StatusMethodNotAllowed {
		t.Errorf("POST: %d, want 405", w.Code)
	}
}

func TestNotificationsWhenTheStoreArrivesLater(t *testing.T) {
	a := newApp(t)
	a.db = newDB(t, filepath.Join(t.TempDir(), "later", "cases.db"))
	s, err := New(a.db, testAddr, a.log)
	if err != nil {
		t.Fatal(err)
	}
	a.server, a.handler = s, s.Handler()
	if p := a.notifications(t, "/notifications"); len(p.Items) != 0 {
		t.Fatalf("missing store: %+v", p)
	}
	a.open(t, openRecords[store.KindFYI])
	if p := a.notifications(t, "/notifications"); len(p.Items) != 1 {
		t.Errorf("the first case in a new store: %d items, want 1", len(p.Items))
	}
}
