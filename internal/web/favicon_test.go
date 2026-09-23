package web

import (
	"net/http"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

func TestFaviconDrawsTheCount(t *testing.T) {
	a := newApp(t)
	for _, tc := range []struct {
		target string
		want   []string
		not    []string
	}{
		{"/favicon.svg", []string{`<path d=`, `fill="#fbfbfa"`}, []string{"<text"}},
		{"/favicon.svg?n=0&blocking=1", []string{`<path d=`}, []string{"<text", iconBlocking}},
		{"/favicon.svg?n=3", []string{`fill="` + iconInk + `"`, `font-size="24"`, `>3</text>`}, []string{iconBlocking}},
		{"/favicon.svg?n=12", []string{`>9+</text>`}, []string{"12"}},
		{"/favicon.svg?blocking=1&n=1", []string{`fill="` + iconBlocking + `"`, `>1</text>`}, nil},
	} {
		w := a.do("GET", tc.target, nil, nil)
		if w.Code != http.StatusOK || w.Header().Get("Content-Type") != "image/svg+xml" {
			t.Errorf("%s: %d %q", tc.target, w.Code, w.Header().Get("Content-Type"))
		}
		if cc := w.Header().Get("Cache-Control"); !strings.HasPrefix(cc, "max-age=") {
			t.Errorf("%s: Cache-Control %q, want a max-age", tc.target, cc)
		}
		body := w.Body.String()
		for _, s := range tc.want {
			if !strings.Contains(body, s) {
				t.Errorf("%s lacks %s:\n%s", tc.target, s, body)
			}
		}
		for _, s := range tc.not {
			if strings.Contains(body, s) {
				t.Errorf("%s has %s:\n%s", tc.target, s, body)
			}
		}
	}
	for _, target := range []string{"/favicon.svg?n=x", "/favicon.svg?n=-1", "/favicon.svg?n=1&blocking=yes"} {
		if w := a.do("GET", target, nil, nil); w.Code != http.StatusBadRequest {
			t.Errorf("%s: %d, want 400", target, w.Code)
		}
	}
	// The icon a refresh points at is fetched on its own, like the refresh.
	if log := a.log.String(); strings.Contains(log, "/favicon.svg?n=3") || !strings.Contains(log, "/favicon.svg?n=x") {
		t.Errorf("request log should leave out fetched icons and keep refused ones:\n%s", log)
	}
}

func TestFaviconFollowsTheTitleCount(t *testing.T) {
	a := newApp(t)
	const bare = `<link id="favicon" rel="icon" type="image/svg+xml" href="/favicon.svg">`
	if body := a.get(t, "/"); !strings.Contains(body, bare) {
		t.Errorf("empty inbox icon:\n%s", body)
	}
	whenever := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Later"})
	parked := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyBlocking, Title: "Parked"})
	if _, err := a.db.Park(t.Context(), parked.ID, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	// A parked case is not counted, and a blocking one that is parked does
	// not make the icon red.
	one := `href="/favicon.svg?n=1"`
	hx := map[string]string{"HX-Request": "true"}
	for _, path := range []string{"/", "/done", "/cases/" + parked.ID, "/fragments/inbox", "/fragments/tally"} {
		if body := a.do("GET", path, nil, hx).Body.String(); !strings.Contains(body, one) {
			t.Errorf("%s lacks %s:\n%s", path, one, body)
		}
	}
	a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyBlocking, Title: "Now"})
	// The refreshes carry the link out of band, as they do the tally, and
	// prefs.js cancels that swap and copies the new href onto the page's link.
	two := `<link id="favicon" rel="icon" type="image/svg+xml" href="/favicon.svg?blocking=1&amp;n=2" hx-swap-oob="true">`
	for _, path := range []string{"/fragments/inbox", "/fragments/tally"} {
		if body := a.do("GET", path, nil, hx).Body.String(); !strings.Contains(body, two) {
			t.Errorf("%s lacks %s:\n%s", path, two, body)
		}
	}
	js := a.get(t, "/static/prefs.js")
	for _, want := range []string{`"htmx:oobBeforeSwap"`, `d.target.id !== "favicon"`, `d.shouldSwap = false;`, `d.target.setAttribute("href", href)`} {
		if !strings.Contains(js, want) {
			t.Errorf("prefs.js lacks %s", want)
		}
	}
	if _, err := a.db.Answer(t.Context(), whenever.ID, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	if body := a.get(t, "/"); !strings.Contains(body, `href="/favicon.svg?blocking=1&amp;n=1"`) {
		t.Errorf("icon after an answer:\n%s", body)
	}
}
