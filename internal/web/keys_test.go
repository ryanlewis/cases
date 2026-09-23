package web

import (
	"net/http"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

// TestKeyboardShortcuts checks what keys.js relies on: every page loads it
// and has the list ? opens, and the case view has the form, send button and
// inbox cards it looks for. The keys themselves are tried in a browser.
func TestKeyboardShortcuts(t *testing.T) {
	a := newApp(t)
	w := a.do("GET", "/static/keys.js", nil, nil)
	if w.Code != http.StatusOK || !strings.Contains(w.Header().Get("Content-Type"), "javascript") {
		t.Fatalf("keys.js: status %d, type %q", w.Code, w.Header().Get("Content-Type"))
	}
	for _, want := range []string{
		`document.querySelector('#case form.respond[action$="/answer"]')`,
		`form.requestSubmit(form.querySelector("#respond-send"))`,
		`document.querySelectorAll("#inbox a.card")`,
		`document.getElementById("keys-dialog")`,
		`form.querySelector("fieldset.row")`,
	} {
		if !strings.Contains(w.Body.String(), want) {
			t.Errorf("keys.js lacks %s", want)
		}
	}

	dec := a.open(t, openRecords[store.KindDecision])
	for _, target := range []string{"/", "/done", "/cases/" + dec.ID} {
		body := a.get(t, target)
		if !strings.Contains(body, `<script src="/static/keys.js" defer></script>`) || strings.Contains(body, "<script>") {
			t.Errorf("%s: keys.js not loaded, or an inline script", target)
		}
		if !strings.Contains(body, `<dialog id="keys-dialog" class="options" aria-labelledby="keys-title">`) {
			t.Errorf("%s: no keys dialog", target)
		}
	}

	// Number keys pick among the one set of choices, in page order.
	body := a.get(t, "/cases/"+dec.ID)
	for _, want := range []string{`<div id="case"`, `<form method="post" action="/cases/` + dec.ID + `/answer" class="respond">`, `id="respond-send"`, `<a class="card`} {
		if !strings.Contains(body, want) {
			t.Errorf("decision page lacks %s", want)
		}
	}
	_, form, ok := strings.Cut(body, `class="respond">`)
	form, _, _ = strings.Cut(form, "</form>")
	if !ok {
		t.Fatal("decision page has no answer form")
	}
	if strings.Contains(form, `class="row"`) {
		t.Error("decision form has an approval row, so number keys leave it alone")
	}
	if n, all := strings.Count(form, `type="radio"`), strings.Count(form, `type="radio" required name="choice"`); n == 0 || n != all {
		t.Errorf("decision page has %d radios, %d of them choices", n, all)
	}

	// An approval's rows have a set of choices each, which number keys leave alone.
	appr := a.open(t, openRecords[store.KindApproval])
	if body := a.get(t, "/cases/"+appr.ID); !strings.Contains(body, `<fieldset class="row">`) {
		t.Error("approval page has no fieldset.row")
	}
}
