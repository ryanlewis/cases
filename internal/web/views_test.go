package web

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
	"github.com/ryanlewis/cases/internal/store/storetest"
)

var (
	approvalRows = []store.Row{
		{ID: "deps", Label: "Install deps", Script: "npm ci --ignore-scripts\nnpm test", Link: "https://example.com/deps"},
		{ID: "mig", Label: "Migrate", Script: "make migrate", Link: "https://example.com/mig"},
	}
	openRecords = map[store.Kind]store.OpenRecord{
		store.KindDecision: {Kind: store.KindDecision, Urgency: store.UrgencyToday, Title: "Pin bun?", Options: []string{"Pin", "Float"}},
		store.KindApproval: {Kind: store.KindApproval, Urgency: store.UrgencyToday, Title: "Scripts", Rows: approvalRows},
		store.KindSignoff:  {Kind: store.KindSignoff, Urgency: store.UrgencyToday, Title: "Handover"},
		store.KindStuck:    {Kind: store.KindStuck, Urgency: store.UrgencyBlocking, Title: "Blocked"},
		store.KindQuestion: {Kind: store.KindQuestion, Urgency: store.UrgencyToday, Title: "Which host?"},
		store.KindFYI:      {Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Heads up"},
	}
)

// idsInOrder returns the case ids linked from a page, in page order.
func idsInOrder(body string) []string {
	var ids []string
	for _, m := range regexp.MustCompile(`href="/cases/([^"/]+)"`).FindAllStringSubmatch(body, -1) {
		if id, err := url.PathUnescape(m[1]); err == nil {
			ids = append(ids, id)
		}
	}
	return ids
}

func TestInboxOrderAndContent(t *testing.T) {
	a := newApp(t)
	later := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Later", Worker: "w-later"})
	time.Sleep(1100 * time.Millisecond) // ids are per second; keep the two blocking cases ordered by age
	oldBlocking := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyBlocking, Title: "Old blocker", Body: "\nFirst line.\nSecond line.\nThird.\nFourth, not shown.", Worker: "bun-pins"})
	today := a.open(t, store.OpenRecord{Kind: store.KindSignoff, Urgency: store.UrgencyToday, Title: "Today"})
	time.Sleep(1100 * time.Millisecond)
	newBlocking := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyBlocking, Title: "New blocker"})
	answered := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyBlocking, Title: "Answered"})
	if _, err := a.db.Answer(t.Context(), answered.ID, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Park(t.Context(), oldBlocking.ID, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}

	body := a.get(t, "/")
	want := []string{oldBlocking.ID, newBlocking.ID, today.ID, later.ID}
	if got := idsInOrder(body); !slices.Equal(got, want) {
		t.Errorf("inbox order = %v\nwant %v", got, want)
	}
	// / shows the first case beside the list, selected. The title is the count
	// and the name: three open cases: the parked one and the answered one do not count.
	if !strings.Contains(body, "<title>(3) cases</title>") {
		t.Errorf("title is not the waiting count and the name:\n%s", body)
	}
	if !strings.Contains(body, `<div class="split home">`) || !strings.Contains(body, `action="/cases/`+oldBlocking.ID+`/resume"`) ||
		!strings.Contains(body, `hx-get="/cases/`+oldBlocking.ID+`/view?state=parked&amp;revision=2&amp;form=2&amp;home=1"`) {
		t.Errorf("/ does not show the first case:\n%s", body)
	}
	if !strings.Contains(body, `selected" href="/cases/`+oldBlocking.ID+`" aria-current="page">`) {
		t.Error("first case is not selected on /")
	}
	if !strings.Contains(body, `hx-get="/fragments/inbox?selected=`+oldBlocking.ID+`" hx-trigger="store-changed from:body, every 60s"`) {
		t.Error("inbox does not refresh with the selection")
	}

	frag := a.do("GET", "/fragments/inbox", nil, map[string]string{"HX-Request": "true"})
	list := frag.Body.String()
	if frag.Code != http.StatusOK || !slices.Equal(idsInOrder(list), want) {
		t.Errorf("fragment %d, ids %v", frag.Code, idsInOrder(list))
	}
	if !strings.Contains(list, "<title>(3) cases</title>") || strings.Contains(list, "<html") || strings.Contains(list, "selected") {
		t.Errorf("fragment is not a bare list with a title and no selection:\n%s", list)
	}
	// The header tally is swapped out of band, so it keeps up with the title.
	// One of each: the answered case is not counted, and the parked blocker
	// counts as parked. Only the blocking figure is red.
	tally := `<span class="hot"><strong>1</strong> blocking</span> · <span><strong>1</strong> today</span> · <span><strong>1</strong> whenever</span> · <span><strong>1</strong> parked</span></span>`
	if !strings.Contains(list, `<span id="tally" class="label tally" hx-swap-oob="true">`+tally) {
		t.Errorf("fragment lacks the out-of-band tally:\n%s", list)
	}
	if !strings.Contains(body, `<span id="tally" class="label tally">`+tally) || !strings.Contains(body, `<a href="/" class="on" aria-current="page">inbox</a>`) {
		t.Error("page header lacks the tally or the current view")
	}
	for _, s := range []string{"First line.", "Second line.", "Third.", "bun-pins", "stuck", "blocking", "parked", "Old blocker"} {
		if !strings.Contains(list, s) {
			t.Errorf("inbox missing %q", s)
		}
	}
	if strings.Contains(list, "Fourth, not shown") {
		t.Error("excerpt shows more than three lines")
	}

	// The refreshed list keeps the selection and the count in the title.
	sel := a.get(t, "/fragments/inbox?selected="+today.ID)
	if !strings.Contains(sel, `class="card urgency-today selected" href="/cases/`+today.ID+`" aria-current="page"`) ||
		strings.Count(sel, "selected") != 2 || !strings.Contains(sel, `hx-get="/fragments/inbox?selected=`+today.ID+`"`) ||
		!strings.Contains(sel, "<title>(3) cases</title>") {
		t.Errorf("fragment lost the selection:\n%s", sel)
	}

	// A case page shows the list beside it, with that case selected.
	page := a.get(t, "/cases/"+today.ID)
	if !strings.Contains(page, `<div class="split">`) || !slices.Equal(idsInOrder(page), want) ||
		!strings.Contains(page, `urgency-today selected" href="/cases/`+today.ID+`" aria-current="page"`) || strings.Count(page, `selected" href=`) != 1 {
		t.Errorf("case page lacks the list with the case selected:\n%s", page)
	}

	// A case opened after the page loaded shows up on the next refresh.
	fresh := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Fresh"})
	if got := a.get(t, "/fragments/inbox"); !strings.Contains(got, fresh.ID) {
		t.Error("refresh did not pick up a new case")
	}
}

func TestEachKindRendersAndAnswers(t *testing.T) {
	tests := []struct {
		kind      store.Kind
		formParts []string
		form      url.Values
		wantState store.State
		wantFile  string
	}{
		{
			kind:      store.KindDecision,
			formParts: []string{`name="choice" value="1"`, `name="choice" value="2"`, `value="other"`, "other, see note", "Pin", "Float"},
			form:      url.Values{"choice": {"2"}, "note": {"go"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind: store.KindApproval,
			formParts: []string{`name="verdict.deps" value="approve"`, `name="verdict.deps" value="hold"`, `name="verdict.mig" value="reject"`,
				`name="note.deps"`, "<pre>npm ci --ignore-scripts\nnpm test</pre>", `href="https://example.com/mig"`},
			form:      url.Values{"verdict.deps": {"approve"}, "verdict.mig": {"hold"}, "note.mig": {"after the release"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindSignoff,
			formParts: []string{`name="signoff" value="accept"`, `name="signoff" value="changes"`, "comment"},
			form:      url.Values{"signoff": {"changes"}, "note": {"rename the flag"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindStuck,
			formParts: []string{`name="stuck" value="text"`, `name="text"`, `name="stuck" value="drop"`, `name="park" value="1" formnovalidate id="respond-park">park</button>`},
			form:      url.Values{"stuck": {"text"}, "text": {"use the mirror"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindQuestion,
			formParts: []string{`<textarea name="text" rows="4" required id="` + fieldID("text") + `" hx-preserve>`, "reply"},
			form:      url.Values{"text": {"the staging one"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindFYI,
			formParts: []string{`name="ack"`, "acknowledge"},
			form:      url.Values{"ack": {"1"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
	}
	for _, tt := range tests {
		t.Run(string(tt.kind), func(t *testing.T) {
			a := newApp(t)
			c := a.open(t, openRecords[tt.kind])
			page := a.get(t, "/cases/"+c.ID)
			for _, part := range append(tt.formParts, `action="/cases/`+c.ID+`/answer"`, `name="note"`) {
				if !strings.Contains(page, part) {
					t.Errorf("form missing %q", part)
				}
			}
			if rev := pageRevision(t, page); rev != 1 {
				t.Errorf("form revision = %d, want 1", rev)
			}

			w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(tt.form, 1), map[string]string{"Origin": "http://" + testAddr})
			// The only case: there is no next one, so back to the inbox.
			if w.Code != http.StatusSeeOther || w.Header().Get("Location") != "/?event=answer&recorded="+c.ID {
				t.Fatalf("post: %d %q %s", w.Code, w.Header().Get("Location"), w.Body.String())
			}
			if files := a.eventFiles(t, c.ID); !slices.Equal(files, []string{"0001-agent-open.json", tt.wantFile}) {
				t.Errorf("files = %v", files)
			}
			loaded, err := a.db.Get(t.Context(), c.ID)
			if err != nil {
				t.Fatal(err)
			}
			if loaded.State != tt.wantState {
				t.Errorf("state = %s", loaded.State)
			}

			after := a.get(t, "/cases/"+c.ID)
			if !strings.Contains(after, `<span class="badge state">`+string(tt.wantState)+`</span>`) || strings.Contains(after, `/answer"`) {
				t.Errorf("page after answer does not show the new state without a form:\n%s", after)
			}
		})
	}
}

// The drop button dismisses a case of every kind but stuck, which drops
// through its own radio. It skips the kind's required fields, so a
// half-filled form sends a drop and nothing else, and a stale page is refused
// like any other answer.
func TestDropAnswersEveryKind(t *testing.T) {
	const button = `<button type="submit" name="drop" value="1" formnovalidate id="respond-drop">drop</button>`
	origin := map[string]string{"Origin": "http://" + testAddr}
	for kind, rec := range openRecords {
		t.Run(string(kind), func(t *testing.T) {
			a := newApp(t)
			c := a.open(t, rec)
			page := a.get(t, "/cases/"+c.ID)
			if kind == store.KindStuck {
				if strings.Contains(page, button) {
					t.Error("stuck form has a drop button beside its drop radio")
				}
				return
			}
			if !strings.Contains(page, button) {
				t.Fatalf("form missing the drop button:\n%s", page)
			}

			form := url.Values{"drop": {"1"}, "note": {"not needed"}, "choice": {"1"}, "signoff": {"accept"}, "text": {"x"}, "ack": {"1"}}
			if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, 0), origin); w.Code != http.StatusConflict {
				t.Errorf("drop from a stale page: %d, want 409", w.Code)
			}
			if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, pageRevision(t, page)), origin); w.Code != http.StatusSeeOther {
				t.Fatalf("drop: %d %s", w.Code, w.Body.String())
			}
			loaded, err := a.db.Get(t.Context(), c.ID)
			if err != nil {
				t.Fatal(err)
			}
			if loaded.State != store.StateAnswered || loaded.Answer == nil || !loaded.Answer.Drop || loaded.Answer.Note != "not needed" ||
				loaded.Answer.Choice != 0 || loaded.Answer.Signoff != "" || loaded.Answer.Text != "" || loaded.Answer.Ack {
				t.Errorf("state %s, answer %+v", loaded.State, loaded.Answer)
			}
		})
	}
}

// A case amended after its page was loaded: the page shows the case as
// amended, a form from the older page is refused, and an answer needs a
// verdict on the added row.
func TestAmendedCase(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	c := a.open(t, openRecords[store.KindApproval])
	before := a.get(t, "/cases/"+c.ID)
	if _, err := a.db.Amend(t.Context(), c.ID, store.AmendRecord{
		Body:  "Now with a **deploy**.",
		Rows:  []store.Row{{ID: "deploy", Label: "Deploy", Script: "make deploy", Link: "https://example.com/deploy"}},
		Links: []string{"https://example.com/log"},
	}); err != nil {
		t.Fatal(err)
	}

	page := a.get(t, "/cases/"+c.ID)
	for _, want := range []string{
		"Now with a <strong>deploy</strong>.",
		`name="verdict.deploy" value="approve"`,
		"<pre>make deploy</pre>",
		`<a href="https://example.com/log"`,
		"<span>amend</span>",
		"<p>replaced the body</p>",
		"<p>added row [deploy] Deploy</p>",
		"<p>added link: https://example.com/log</p>",
	} {
		if !strings.Contains(page, want) {
			t.Errorf("page missing %q:\n%s", want, page)
		}
	}

	// The page loaded before the amend has no added row. Its form is refused
	// as stale, and the case is shown again as amended.
	form := url.Values{"verdict.deps": {"approve"}, "verdict.mig": {"approve"}}
	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, pageRevision(t, before)), origin)
	if body := w.Body.String(); w.Code != http.StatusConflict || !strings.Contains(body, staleForm) || !strings.Contains(body, `name="verdict.deploy"`) {
		t.Errorf("answer from before the amend: %d %s", w.Code, body)
	}
	rev := pageRevision(t, page)
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, rev), origin); w.Code != http.StatusUnprocessableEntity || !strings.Contains(w.Body.String(), "don&#39;t run for &#34;Deploy&#34;") {
		t.Errorf("answer without the added row: %d %s", w.Code, w.Body.String())
	}
	form.Set("verdict.deploy", "hold")
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, rev), origin); w.Code != http.StatusSeeOther {
		t.Errorf("answer with every row: %d %s", w.Code, w.Body.String())
	}
	if files := a.eventFiles(t, c.ID); !slices.Equal(files, []string{"0001-agent-open.json", "0002-agent-amend.json", "0003-human-answer.json"}) {
		t.Errorf("files = %v", files)
	}
}

// The thread shows the body and context an amend replaced, each in a details
// element a refresh of the case view keeps as the human left it: the body as markdown with raw
// HTML dropped, and the context escaped. A case that had no body before has
// no previous body to show.
func TestThreadShowsWhatAnAmendReplaced(t *testing.T) {
	const body, context = "Was **bold** & <script>alert(1)</script> <b>raw</b>", `Release <1.4> & "quoted"`
	a := newApp(t)
	c := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Heads up", Body: body, Context: context})
	if _, err := a.db.Amend(t.Context(), c.ID, store.AmendRecord{Body: "Now plain.", Context: "Release 1.4.1"}); err != nil {
		t.Fatal(err)
	}
	for _, target := range []string{"/cases/" + c.ID, "/cases/" + c.ID + "/view?state=open"} {
		page := a.get(t, target)
		for _, want := range []string{
			`<p>replaced the body</p>`,
			`<details id="previous-body-2-` + textID(body) + `" hx-preserve><summary>previous body</summary><div class="body"><p>Was <strong>bold</strong> &amp; <!-- raw HTML omitted -->alert(1)<!-- raw HTML omitted --> <!-- raw HTML omitted -->raw<!-- raw HTML omitted --></p>`,
			`<p>replaced the context: Release 1.4.1</p>`,
			`<details id="previous-context-2-` + textID(context) + `" hx-preserve><summary>previous context</summary><p class="context">Release &lt;1.4&gt; &amp; &#34;quoted&#34;</p></details>`,
		} {
			if !strings.Contains(page, want) {
				t.Errorf("%s missing %s:\n%s", target, want, page)
			}
		}
		for _, bad := range []string{"<script>", "<b>raw</b>", "Release <1.4>"} {
			if strings.Contains(page, bad) {
				t.Errorf("%s contains %s", target, bad)
			}
		}
	}

	bare := a.open(t, openRecords[store.KindFYI])
	if _, err := a.db.Amend(t.Context(), bare.ID, store.AmendRecord{Body: "A body at last.", Context: "Release 1.4"}); err != nil {
		t.Fatal(err)
	}
	if page := a.get(t, "/cases/"+bare.ID); !strings.Contains(page, "<p>replaced the body</p>") || strings.Contains(page, "<details") {
		t.Errorf("a case with no body or context before shows a previous one:\n%s", page)
	}
}

// An amend stored after a later one, as a hand edit or another tool could
// store it, changes what the later one replaced. The details under the later
// one then gets a new id, so a refresh of the case view does not keep the old
// text in its place.
func TestThreadDetailsFollowTheText(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Heads up", Body: "First."})
	arrive := func(seq int, data string) {
		t.Helper()
		storetest.InsertEvent(t, a.db.Path, c.ID, seq, "agent", "amend", data)
	}
	thread := "/cases/" + c.ID + "/view?state=open"

	arrive(3, `{"body":"Third."}`)
	stale := `<details id="previous-body-3-` + textID("First.") + `" hx-preserve>`
	if page := a.get(t, thread); !strings.Contains(page, stale) {
		t.Fatalf("thread missing %s:\n%s", stale, page)
	}

	arrive(2, `{"body":"Second."}`)
	page := a.get(t, thread)
	for _, want := range []string{
		`<details id="previous-body-2-` + textID("First.") + `" hx-preserve>`,
		`<details id="previous-body-3-` + textID("Second.") + `" hx-preserve>`,
	} {
		if !strings.Contains(page, want) {
			t.Errorf("thread missing %s:\n%s", want, page)
		}
	}
	if strings.Contains(page, stale) {
		t.Errorf("thread keeps the id of text amend 0003 no longer replaced:\n%s", page)
	}
}

func TestBodyRendersGFMTable(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{
		Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Table",
		Body: "| Package | Version |\n| --- | --- |\n| bun | 1.2.3 |\n",
	})

	page := a.get(t, "/cases/"+c.ID)
	for _, part := range []string{"<table>", "<th>Package</th>", "<th>Version</th>", "<td>bun</td>", "<td>1.2.3</td>"} {
		if !strings.Contains(page, part) {
			t.Errorf("page missing %q:\n%s", part, page)
		}
	}
}

// TestBodyAlignsTableColumnsByClass checks that an aligned column is aligned
// by a class the stylesheet styles, not by an inline style the CSP blocks.
func TestBodyAlignsTableColumnsByClass(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{
		Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Aligned",
		Body: "| Name | Size | Count | Note |\n| :--- | :---: | ---: | --- |\n| bun | 1.2 | 3 | ok |\n",
	})

	page := a.get(t, "/cases/"+c.ID)
	for _, part := range []string{
		`<th class="align-left">Name</th>`, `<th class="align-center">Size</th>`, `<th class="align-right">Count</th>`, "<th>Note</th>",
		`<td class="align-left">bun</td>`, `<td class="align-center">1.2</td>`, `<td class="align-right">3</td>`, "<td>ok</td>",
	} {
		if !strings.Contains(page, part) {
			t.Errorf("page missing %q:\n%s", part, page)
		}
	}
	if strings.Contains(page, "style=") || strings.Contains(page, " align=") {
		t.Errorf("page has an inline style or align attribute:\n%s", page)
	}

	css := a.get(t, "/static/style.css")
	for _, rule := range []string{".body .align-left { text-align: left; }", ".body .align-center { text-align: center; }", ".body .align-right { text-align: right; }"} {
		if !strings.Contains(css, rule) {
			t.Errorf("style.css lacks %s", rule)
		}
	}
}

func TestStuckParkAndResume(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindStuck])
	origin := map[string]string{"Origin": "http://" + testAddr}

	// The park button parks even with guidance ticked: the browser sends both.
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"stuck": {"text"}, "park": {"1"}, "note": {"after release"}}, 1), origin); w.Code != http.StatusSeeOther {
		t.Fatalf("park: %d %s", w.Code, w.Body.String())
	}
	if files := a.eventFiles(t, c.ID); !slices.Equal(files, []string{"0001-agent-open.json", "0002-human-park.json"}) {
		t.Errorf("files = %v", files)
	}
	page := a.get(t, "/cases/"+c.ID)
	if !strings.Contains(page, `action="/cases/`+c.ID+`/resume"`) || pageRevision(t, page) != 2 || !strings.Contains(page, "note: after release") ||
		!strings.Contains(page, `<span class="badge state parked">parked</span>`) || !strings.Contains(page, `class="card urgency-blocking parked selected"`) {
		t.Errorf("parked page:\n%s", page)
	}
	if w := a.do("POST", "/cases/"+c.ID+"/resume", withRevision(nil, 2), origin); w.Code != http.StatusSeeOther {
		t.Fatalf("resume: %d", w.Code)
	}
	loaded, err := a.db.Get(t.Context(), c.ID)
	if err != nil || loaded.State != store.StateOpen || loaded.Events[2].Author != store.AuthorHuman {
		t.Errorf("after resume: %+v %v", loaded, err)
	}
	// Resuming an open case is refused and writes nothing.
	if w := a.do("POST", "/cases/"+c.ID+"/resume", withRevision(nil, 3), origin); w.Code != http.StatusUnprocessableEntity {
		t.Errorf("resume of open case: %d", w.Code)
	}
	if n := len(a.eventFiles(t, c.ID)); n != 3 {
		t.Errorf("%d files", n)
	}
}

func TestPostsGoToTheNextCase(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	// Inbox order: blocked, decision, parked, zed. The two whenever cases sort
	// by id: open time, then title.
	blocked := a.open(t, openRecords[store.KindStuck])
	decision := a.open(t, openRecords[store.KindDecision])
	parked := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyWhenever, Title: "Parked"})
	if _, err := a.db.Park(t.Context(), parked.ID, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	zed := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Zed"})

	steps := []struct {
		name, target string
		form         url.Values
		want         string
	}{
		// A parked case stays in the inbox, and the page still moves on.
		{"park", "/cases/" + blocked.ID + "/answer", withRevision(url.Values{"stuck": {"park"}}, 1), "/cases/" + decision.ID + "?event=park&recorded=" + blocked.ID},
		{"answer", "/cases/" + decision.ID + "/answer", withRevision(url.Values{"choice": {"1"}}, 1), "/cases/" + parked.ID + "?event=answer&recorded=" + decision.ID},
		{"resume", "/cases/" + parked.ID + "/resume", withRevision(nil, 2), "/cases/" + zed.ID + "?event=resume&recorded=" + parked.ID},
		{"answer the last", "/cases/" + zed.ID + "/answer", withRevision(url.Values{"ack": {"1"}}, 1), "/?event=answer&recorded=" + zed.ID},
		{"resume the first", "/cases/" + blocked.ID + "/resume", withRevision(nil, 2), "/cases/" + parked.ID + "?event=resume&recorded=" + blocked.ID},
	}
	for _, st := range steps {
		w := a.do("POST", st.target, st.form, origin)
		if w.Code != http.StatusSeeOther || w.Header().Get("Location") != st.want {
			t.Errorf("%s: %d %q, want %q %s", st.name, w.Code, w.Header().Get("Location"), st.want, w.Body.String())
		}
	}
}

func TestRedirectConfirmsWhatWasRecorded(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	first := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Ship <v2> & tell"})
	a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Second"})
	line := func(dismiss string) string {
		return `<p class="recorded" role="status"><span>answer recorded on <a href="/cases/` + first.ID + `">Ship &lt;v2&gt; &amp; tell</a></span>` +
			`<span class="chips dismiss"><a href="` + dismiss + `" aria-label="dismiss this confirmation">dismiss</a></span></p>`
	}

	w := a.do("POST", "/cases/"+first.ID+"/answer", withRevision(url.Values{"ack": {"1"}}, 1), origin)
	if w.Code != http.StatusSeeOther {
		t.Fatalf("post: %d %s", w.Code, w.Body.String())
	}
	next := w.Header().Get("Location")
	nextPath, _, _ := strings.Cut(next, "?")
	if page := a.get(t, next); !strings.Contains(page, line(nextPath)) {
		t.Errorf("next case missing %s:\n%s", line(nextPath), page)
	}
	// Answering the last case lands on /, which has no case block.
	if page := a.get(t, "/?event=answer&recorded="+first.ID); !strings.Contains(page, line("/")) {
		t.Errorf("/ missing %s", line("/"))
	}
	// Dismissing keeps any other query parameter, and the page it goes to has no line.
	withOthers := nextPath + "?b=2&event=answer&a=x+y&recorded=" + first.ID
	if page := a.get(t, withOthers); !strings.Contains(page, line(nextPath+"?a=x&#43;y&amp;b=2")) {
		t.Errorf("%s missing %s:\n%s", withOthers, line(nextPath+"?a=x&#43;y&amp;b=2"), page)
	}
	if page := a.get(t, nextPath+"?a=x+y&b=2"); strings.Contains(page, `class="recorded"`) || strings.Contains(page, "dismiss this confirmation") {
		t.Error("the dismissed page still shows the line")
	}

	for _, target := range []string{
		"/cases/" + first.ID,
		"/?event=answer&recorded=2026-01-01T00-00-00Z-no-such-case",
		"/?event=answer&recorded=",
		"/?event=close&recorded=" + first.ID,
		"/?recorded=" + first.ID,
		"/?event=answer",
	} {
		if page := a.get(t, target); strings.Contains(page, `class="recorded"`) || strings.Contains(page, "dismiss this confirmation") {
			t.Errorf("%s: shows a recorded line", target)
		}
	}
	// A refused post names nothing as recorded.
	w = a.do("POST", "/cases/"+first.ID+"/answer", withRevision(url.Values{"ack": {"1"}}, 1), origin)
	if w.Code == http.StatusSeeOther || strings.Contains(w.Body.String(), `class="recorded"`) {
		t.Errorf("refused post: %d, recorded line %t", w.Code, strings.Contains(w.Body.String(), `class="recorded"`))
	}
}

func TestInvalidAnswerWritesNothing(t *testing.T) {
	tests := []struct {
		kind    store.Kind
		form    url.Values
		wantErr string
		keeps   string
	}{
		{store.KindDecision, url.Values{"note": {"hm"}}, "choose an option", "hm"},
		{store.KindDecision, url.Values{"choice": {"other"}}, "other needs a note", ""},
		{store.KindDecision, url.Values{"choice": {"7"}}, "choice 7 is not an option", ""},
		{store.KindApproval, url.Values{"verdict.deps": {"approve"}, "note.deps": {"fine"}}, "choose approve, hold or don&#39;t run for &#34;Migrate&#34;", `<textarea name="note.deps" rows="1" id="` + fieldID("note.deps") + `" hx-preserve>fine</textarea>`},
		{store.KindApproval, url.Values{"verdict.deps": {"approve"}, "verdict.mig": {"maybe"}}, "verdict &#34;maybe&#34;", ""},
		{store.KindSignoff, url.Values{"signoff": {"changes"}}, "requesting changes needs a note", `value="changes" checked`},
		{store.KindStuck, url.Values{"stuck": {"text"}}, "write the guidance", ""},
		{store.KindQuestion, url.Values{"text": {"  "}, "note": {"hm"}}, "write the reply", "hm"},
		{store.KindFYI, url.Values{}, "no fyi response", ""},
	}
	for _, tt := range tests {
		t.Run(string(tt.kind)+"/"+tt.wantErr, func(t *testing.T) {
			a := newApp(t)
			c := a.open(t, openRecords[tt.kind])
			w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(tt.form, 1), nil)
			if w.Code != http.StatusUnprocessableEntity {
				t.Fatalf("status %d", w.Code)
			}
			body := w.Body.String()
			if !strings.Contains(refusal(t, body), tt.wantErr) {
				t.Errorf("body lacks error %q:\n%s", tt.wantErr, body)
			}
			if tt.keeps != "" && !strings.Contains(body, tt.keeps) {
				t.Errorf("re-rendered form lost %q", tt.keeps)
			}
			if files := a.eventFiles(t, c.ID); len(files) != 1 {
				t.Errorf("files = %v", files)
			}
		})
	}
}

func TestAnswerOnAClosedCaseOrUnknownCase(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindFYI])
	if _, err := a.db.Withdraw(t.Context(), c.ID, store.WithdrawRecord{}); err != nil {
		t.Fatal(err)
	}
	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"ack": {"1"}}, 2), nil)
	if w.Code != http.StatusUnprocessableEntity || !strings.Contains(w.Body.String(), "cannot answer a case that is withdrawn") {
		t.Errorf("%d %s", w.Code, w.Body.String())
	}
	for _, target := range []string{"/cases/nope", "/cases/nope/view"} {
		if w := a.do("GET", target, nil, nil); w.Code != http.StatusNotFound {
			t.Errorf("GET %s: %d", target, w.Code)
		}
	}
	for _, target := range []string{"/cases/nope/answer", "/cases/..%2Fx/answer"} {
		if w := a.do("POST", target, url.Values{"ack": {"1"}}, nil); w.Code != http.StatusNotFound {
			t.Errorf("POST %s: %d", target, w.Code)
		}
	}
}

func TestStaleTabCannotAnswerAReopenedCase(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	c := a.open(t, openRecords[store.KindDecision])
	target := "/cases/" + c.ID + "/answer"

	// Two tabs show the open case, and tab A answers it.
	tabA, tabB := a.get(t, "/cases/"+c.ID), a.get(t, "/cases/"+c.ID)
	if w := a.do("POST", target, withRevision(url.Values{"choice": {"1"}}, pageRevision(t, tabA)), origin); w.Code != http.StatusSeeOther {
		t.Fatalf("tab A: %d %s", w.Code, w.Body.String())
	}
	// The agent picks the answer up and asks a follow-up, which reopens the case.
	if _, err := a.db.Pickup(t.Context(), c.ID, store.PickupRecord{By: "bun-pins"}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "Pin to which **patch**?"}); err != nil {
		t.Fatal(err)
	}
	written := a.eventFiles(t, c.ID)

	// Tab B's form is from before the answer. The case is open again, but the
	// form is refused and nothing is written.
	w := a.do("POST", target, withRevision(url.Values{"choice": {"2"}, "note": {"float it"}}, pageRevision(t, tabB)), origin)
	if w.Code != http.StatusConflict {
		t.Fatalf("tab B: %d, want 409\n%s", w.Code, w.Body.String())
	}
	if files := a.eventFiles(t, c.ID); !slices.Equal(files, written) {
		t.Errorf("files = %v, want %v", files, written)
	}
	// Tab B gets the case as it is now, with what it typed, and a form at the
	// current revision.
	body := w.Body.String()
	for _, want := range []string{`<p class="error" role="alert" id="refusal" hx-preserve>` + staleForm + `</p>`, "Pin to which <strong>patch</strong>?", "float it"} {
		if !strings.Contains(body, want) {
			t.Errorf("refused page missing %q:\n%s", want, body)
		}
	}
	if got := pageRevision(t, body); got != 4 {
		t.Errorf("refused page's revision = %d, want 4", got)
	}
	// Sent again from that page, the answer is recorded.
	if w := a.do("POST", target, withRevision(url.Values{"choice": {"2"}, "note": {"float it"}}, pageRevision(t, body)), origin); w.Code != http.StatusSeeOther {
		t.Fatalf("sent again: %d %s", w.Code, w.Body.String())
	}
	if files := a.eventFiles(t, c.ID); len(files) != 5 || files[4] != "0005-human-answer.json" {
		t.Errorf("files = %v", files)
	}
}

func TestStaleParkAndResumeAreRefused(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	c := a.open(t, openRecords[store.KindStuck])
	park := func() {
		t.Helper()
		if _, err := a.db.Park(t.Context(), c.ID, store.ParkRecord{}); err != nil {
			t.Fatal(err)
		}
	}
	resume := func() {
		t.Helper()
		if _, err := a.db.Resume(t.Context(), c.ID, store.AuthorAgent, store.ResumeRecord{}); err != nil {
			t.Fatal(err)
		}
	}
	refused := func(name, action string, form url.Values) {
		t.Helper()
		before := a.eventFiles(t, c.ID)
		w := a.do("POST", "/cases/"+c.ID+"/"+action, form, origin)
		if w.Code != http.StatusConflict || !strings.Contains(w.Body.String(), staleForm) {
			t.Errorf("%s: %d, want 409 with the stale form error\n%s", name, w.Code, w.Body.String())
		}
		if files := a.eventFiles(t, c.ID); !slices.Equal(files, before) {
			t.Errorf("%s: files = %v, want %v", name, files, before)
		}
	}

	// Parked and resumed elsewhere after the page was loaded.
	open := a.get(t, "/cases/"+c.ID)
	park()
	resume()
	refused("park", "answer", withRevision(url.Values{"park": {"1"}}, pageRevision(t, open)))
	// A stale form with a mistake in it is refused as stale, not for the
	// mistake: that page would carry the new revision without saying why.
	refused("guidance left empty", "answer", withRevision(url.Values{"stuck": {"text"}}, pageRevision(t, open)))

	// Resumed and parked again elsewhere after the page was loaded.
	park()
	parked := a.get(t, "/cases/"+c.ID)
	resume()
	park()
	refused("resume", "resume", withRevision(nil, pageRevision(t, parked)))
	// A form from a page served before forms carried a revision.
	refused("resume without a revision", "resume", url.Values{})
}

// The event that makes a post stale can land after the handler has loaded the
// case and before the store locks it, as when a form is sent twice in quick
// succession. The refusal shows the case as it is after that event.
func TestStaleRefusalShowsTheCaseAsItIsNow(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindDecision])
	loaded, err := a.db.Get(t.Context(), c.ID)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "Pin to which **patch**?"}); err != nil {
		t.Fatal(err)
	}
	_, err = a.db.Answer(t.Context(), c.ID, store.AnswerRecord{Choice: 1}, store.AtRevision(loaded.Revision()))
	if !errors.Is(err, store.ErrStale) {
		t.Fatalf("err = %v, want ErrStale", err)
	}

	w := httptest.NewRecorder()
	a.server.refuse(w, httptest.NewRequest(http.MethodPost, "/", nil), loaded, nil, err, url.Values{"choice": {"1"}, "note": {"float it"}})
	if w.Code != http.StatusConflict {
		t.Fatalf("status %d, want 409", w.Code)
	}
	body := w.Body.String()
	for _, want := range []string{staleForm, "Pin to which <strong>patch</strong>?", "float it"} {
		if !strings.Contains(body, want) {
			t.Errorf("refused page missing %q:\n%s", want, body)
		}
	}
	if got := pageRevision(t, body); got != 2 {
		t.Errorf("refused page's revision = %d, want 2", got)
	}
}

// The case view refreshes when its case changes. A page drawn while the case
// was open shows it open still after an answer and a note that reopened it,
// so the view is drawn again in place: the header, the thread and a form at
// the case's revision, which sends without a 409 because the human has seen
// the case as it now is. A view at the case's revision is left as it is, and
// a change of state loads the page again.
func TestCaseViewRefresh(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	hx := map[string]string{"HX-Request": "true"}
	c := a.open(t, openRecords[store.KindDecision])
	page := a.get(t, "/cases/"+c.ID)
	if want := `<div id="case" hx-get="/cases/` + c.ID + `/view?state=open&amp;revision=1&amp;form=1" hx-trigger="store-changed from:body, every 60s" hx-sync="this:replace" hx-swap="outerHTML settle:0ms">`; !strings.Contains(page, want) {
		t.Errorf("case page lacks %s", want)
	}
	view := "/cases/" + c.ID + "/view?state=open&revision=1"
	if w := a.do("GET", view, nil, hx); w.Code != http.StatusNoContent || w.Body.Len() != 0 {
		t.Errorf("unchanged case: %d %q, want 204 and nothing", w.Code, w.Body.String())
	}

	// Answered in another tab, picked up, and reopened by the agent's note.
	if _, err := a.db.Answer(t.Context(), c.ID, store.AnswerRecord{Choice: 1, Note: "ship it"}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Pickup(t.Context(), c.ID, store.PickupRecord{By: "bun-pins"}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "Which **patch**?"}); err != nil {
		t.Fatal(err)
	}
	w := a.do("GET", view, nil, hx)
	frag := w.Body.String()
	if w.Code != http.StatusOK {
		t.Fatalf("changed case: %d %s", w.Code, frag)
	}
	for _, want := range []string{
		`<div id="case" hx-get="/cases/` + c.ID + `/view?state=open&amp;revision=4&amp;form=4"`,
		"<h1>Pin bun?</h1>",
		`action="/cases/` + c.ID + `/answer"`,
		"chose 1. Pin", "note: ship it", "by bun-pins", "Which <strong>patch</strong>?",
	} {
		if !strings.Contains(frag, want) {
			t.Errorf("view missing %q:\n%s", want, frag)
		}
	}
	if strings.Contains(frag, "<html") || strings.Contains(frag, `aria-label="inbox"`) {
		t.Error("the view is a full page, or has the list")
	}
	if rev := pageRevision(t, frag); rev != 4 {
		t.Errorf("view's form revision = %d, want 4", rev)
	}
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"choice": {"2"}}, pageRevision(t, frag)), origin); w.Code != http.StatusSeeOther {
		t.Fatalf("answer from the refreshed view: %d %s", w.Code, w.Body.String())
	}

	// Answered now: the view sends the browser to the case page with a GET,
	// and on / to /, which shows whichever case is first now.
	w = a.do("GET", "/cases/"+c.ID+"/view?state=open&revision=4", nil, hx)
	if w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/cases/"+c.ID || w.Header().Get("HX-Refresh") != "" {
		t.Errorf("changed state: %d, headers %v", w.Code, w.Header())
	}
	w = a.do("GET", "/cases/"+c.ID+"/view?state=open&revision=4&home=1", nil, hx)
	if w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/" {
		t.Errorf("changed state on /: %d, headers %v", w.Code, w.Header())
	}
}

// fieldIDs are the ids of the fields a page's form keeps when its case view
// is drawn again (hx-preserve) whose id starts with prefix, in page order.
func fieldIDs(t *testing.T, page, prefix string) []string {
	t.Helper()
	var ids []string
	for _, m := range regexp.MustCompile(`id="([^"]*)" hx-preserve[ >]`).FindAllStringSubmatch(page, -1) {
		if !regexp.MustCompile(`^[a-z][a-z0-9-]*$`).MatchString(m[1]) {
			t.Errorf("id %q is not a plain CSS selector", m[1])
		}
		if strings.HasPrefix(m[1], prefix+"-") {
			ids = append(ids, m[1])
		}
	}
	return ids
}

// refusalLine is the line a page's case view says a form was refused on.
var refusalLine = regexp.MustCompile(`<p class="error" role="alert" id="refusal" hx-preserve( hidden)?>([^<]*)</p>`)

// refusal returns what the refusal line on a page says, or "" when it is
// hidden. It fails the test when the line is missing, or hidden with text.
func refusal(t *testing.T, page string) string {
	t.Helper()
	m := refusalLine.FindStringSubmatch(page)
	if m == nil || (m[1] != "") != (m[2] == "") {
		t.Fatalf("page has no refusal line, or a wrong one:\n%s", page)
	}
	return m[2]
}

// The fields keep their ids when the case view is drawn again, so htmx keeps
// them as the human left them: the text typed and the choices made. The form
// takes the case's new revision, except after an amend that changes the
// question: then it keeps the one it had through later refreshes, so its
// next send is refused and the case shown again to be checked, with what was
// sent, and sending from there goes through.
func TestCaseViewKeepsTheFormAsLeft(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	c := a.open(t, openRecords[store.KindApproval])
	page := a.get(t, "/cases/"+c.ID)
	texts, choices := fieldIDs(t, page, "field"), fieldIDs(t, page, "choice")
	// A note on each of the two rows and the note to the agent; three
	// verdicts on each row.
	if len(texts) != 3 || len(choices) != 6 {
		t.Fatalf("%d text fields and %d choices, want 3 and 6:\n%s", len(texts), len(choices), page)
	}
	all := append(slices.Clone(texts), choices...)
	slices.Sort(all)
	if len(slices.Compact(all)) != 9 {
		t.Errorf("ids are not unique: %v", all)
	}
	refresh := func(view, form int) string {
		t.Helper()
		return a.get(t, fmt.Sprintf("/cases/%s/view?state=open&revision=%d&form=%d", c.ID, view, form))
	}
	drawn := func(frag string, view, form int) {
		t.Helper()
		if want := fmt.Sprintf("/view?state=open&amp;revision=%d&amp;form=%d", view, form); !strings.Contains(frag, want) {
			t.Errorf("view does not ask with %s", want)
		}
		if got := pageRevision(t, frag); got != form {
			t.Errorf("form revision = %d, want %d", got, form)
		}
	}

	// A note from the agent, and an amend that only adds labels, leave the
	// question as it was.
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "The migration is the slow one."}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Amend(t.Context(), c.ID, store.AmendRecord{Labels: []string{"round 2"}}); err != nil {
		t.Fatal(err)
	}
	frag := refresh(1, 1)
	if got := fieldIDs(t, frag, "field"); !slices.Equal(got, texts) {
		t.Errorf("text fields after a note = %v, want %v", got, texts)
	}
	if got := fieldIDs(t, frag, "choice"); !slices.Equal(got, choices) {
		t.Errorf("choices after a note = %v, want %v", got, choices)
	}
	drawn(frag, 3, 3)

	// An amend that adds a row changes the question. The fields there were
	// keep their ids, the row's are new, and the form keeps revision 3.
	row := store.Row{ID: "deploy", Label: "Deploy", Script: "make deploy", Link: "https://example.com/deploy"}
	if _, err := a.db.Amend(t.Context(), c.ID, store.AmendRecord{Rows: []store.Row{row}}); err != nil {
		t.Fatal(err)
	}
	frag = refresh(3, 3)
	gotTexts, gotChoices := fieldIDs(t, frag, "field"), fieldIDs(t, frag, "choice")
	if len(gotTexts) != 4 || len(gotChoices) != 9 || !slices.Contains(gotTexts, fieldID("note.deploy")) {
		t.Errorf("after the amend: text fields %v, choices %v", gotTexts, gotChoices)
	}
	for _, id := range append(slices.Clone(texts), choices...) {
		if !slices.Contains(append(slices.Clone(gotTexts), gotChoices...), id) {
			t.Errorf("field %s lost its id in the amend", id)
		}
	}
	drawn(frag, 4, 3)
	// A later note leaves the form where it was.
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "Deploy last."}); err != nil {
		t.Fatal(err)
	}
	frag = refresh(4, 3)
	drawn(frag, 5, 3)

	form := url.Values{"verdict.deps": {"approve"}, "verdict.mig": {"hold"}, "verdict.deploy": {"hold"}, "note.deploy": {"after the release"}}
	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, pageRevision(t, frag)), origin)
	if w.Code != http.StatusConflict || refusal(t, w.Body.String()) != staleForm {
		t.Fatalf("send from the held form: %d, want 409 with the stale form line\n%s", w.Code, w.Body.String())
	}
	if body := w.Body.String(); pageRevision(t, body) != 5 || !strings.Contains(body, ">after the release</textarea>") {
		t.Errorf("refused page is not at revision 5 with what was sent:\n%s", body)
	}
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, 5), origin); w.Code != http.StatusSeeOther {
		t.Errorf("sent again: %d %s", w.Code, w.Body.String())
	}
}

// Every field the case view keeps carries the id made from its own name and
// value, so a refresh keeps each in its place, and no two share one.
func TestFormIDsMatchTheirFields(t *testing.T) {
	a := newApp(t)
	radio := regexp.MustCompile(`<input type="radio"[^>]* name="([^"]+)" value="([^"]+)"[^>]* id="([^"]+)" hx-preserve>`)
	textarea := regexp.MustCompile(`<textarea name="([^"]+)"[^>]* id="([^"]+)" hx-preserve>`)
	for kind, rec := range openRecords {
		c := a.open(t, rec)
		page := a.get(t, "/cases/"+c.ID)
		start := strings.Index(page, `class="respond"`)
		form := page[start : start+strings.Index(page[start:], "</form>")]
		ids := map[string]bool{}
		unique := func(id string) {
			if ids[id] {
				t.Errorf("%s: id %s is used twice", kind, id)
			}
			ids[id] = true
		}
		radios := radio.FindAllStringSubmatch(form, -1)
		for _, m := range radios {
			if want := choiceID(m[1], m[2]); m[3] != want {
				t.Errorf("%s: radio %s=%s has id %s, want %s", kind, m[1], m[2], m[3], want)
			}
			unique(m[3])
		}
		texts := textarea.FindAllStringSubmatch(form, -1)
		for _, m := range texts {
			if want := fieldID(m[1]); m[2] != want {
				t.Errorf("%s: textarea %s has id %s, want %s", kind, m[1], m[2], want)
			}
			unique(m[2])
		}
		if len(radios) != strings.Count(form, `type="radio"`) || len(texts) != strings.Count(form, "<textarea") {
			t.Errorf("%s: a field the view does not keep:\n%s", kind, form)
		}
	}
}

// The line saying a form was refused is in the case view on every page,
// hidden and empty unless a form was refused, and a view drawn again carries
// it that way: htmx keeps the page's own line in its place (hx-preserve), so
// a refresh does not wipe a refusal the human has not read.
func TestRefusalLineOutlivesARefresh(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindDecision])
	if got := refusal(t, a.get(t, "/cases/"+c.ID)); got != "" {
		t.Errorf("a page no form was sent from says %q", got)
	}
	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"choice": {"1"}}, 0), nil)
	if w.Code != http.StatusConflict || refusal(t, w.Body.String()) != staleForm {
		t.Fatalf("stale send: %d\n%s", w.Code, w.Body.String())
	}
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "Pin to which patch?"}); err != nil {
		t.Fatal(err)
	}
	if got := refusal(t, a.get(t, "/cases/"+c.ID+"/view?state=open&revision=1&form=1")); got != "" {
		t.Errorf("the view drawn again says %q, want an empty line for htmx to keep the page's in place of", got)
	}
}

// A tab served before pages followed the store asks
// /cases/{id}/thread?state=S every two seconds until it is reloaded. It gets
// 204, which is not logged, until the case changes state, and then goes to
// the case page, or to / from /.
func TestOldThreadPoll(t *testing.T) {
	a := newApp(t)
	hx := map[string]string{"HX-Request": "true"}
	c := a.open(t, openRecords[store.KindFYI])
	old := "/cases/" + c.ID + "/thread?state=open"
	if w := a.do("GET", old, nil, hx); w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "" || w.Body.Len() != 0 {
		t.Errorf("unchanged: %d, headers %v", w.Code, w.Header())
	}
	if _, err := a.db.Answer(t.Context(), c.ID, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	if w := a.do("GET", old, nil, hx); w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/cases/"+c.ID {
		t.Errorf("changed state: %d, headers %v", w.Code, w.Header())
	}
	if w := a.do("GET", old+"&home=1", nil, hx); w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/" {
		t.Errorf("changed state on /: %d, headers %v", w.Code, w.Header())
	}
	if w := a.do("GET", "/cases/nope/thread?state=open", nil, hx); w.Code != http.StatusNotFound {
		t.Errorf("unknown case: %d", w.Code)
	}
	if log := a.log.String(); strings.Contains(log, c.ID+"/thread") {
		t.Errorf("the old tab's requests are logged:\n%s", log)
	}
}

// noteOnGet is a store that has the agent write a note on a case just after
// the case is read, as when a note lands while a form is being sent.
type noteOnGet struct{ store.Store }

func (s noteOnGet) Get(ctx context.Context, id string) (*store.Case, error) {
	c, err := s.Store.Get(ctx, id)
	if err == nil {
		_, err = s.Note(ctx, id, store.NoteRecord{Body: "Written while the form was sent."})
	}
	return c, err
}

// A refused form's page shows the case as it was read, so a note that lands
// just after is not on it. The page's event stream starts from the list read
// before the case, so it reports the note at once.
func TestRefusedPageFollowsAChangeMadeWhileItWasSent(t *testing.T) {
	a := newAppWith(t, func(s store.Store) store.Store { return noteOnGet{s} })
	c := a.open(t, openRecords[store.KindStuck])
	// Guidance chosen and not written: refused before anything is written.
	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"stuck": {"text"}}, 1), nil)
	if w.Code != http.StatusUnprocessableEntity || strings.Contains(w.Body.String(), "Written while the form was sent.") {
		t.Fatalf("status %d, or the note already on the page:\n%s", w.Code, w.Body.String())
	}
	if pageVersion(t, w.Body.String()) == pageVersion(t, a.get(t, "/")) {
		t.Error("the refused page starts its stream at the version with the note, so it is not told of it")
	}
}

func TestDoneView(t *testing.T) {
	a := newApp(t)
	answered := a.open(t, openRecords[store.KindFYI])
	closed := a.open(t, openRecords[store.KindDecision])
	withdrawn := a.open(t, openRecords[store.KindSignoff])
	open := a.open(t, openRecords[store.KindStuck])

	steps := []func() error{
		func() error { _, err := a.db.Answer(t.Context(), closed.ID, store.AnswerRecord{Choice: 1}); return err },
		func() error { _, err := a.db.Pickup(t.Context(), closed.ID, store.PickupRecord{}); return err },
		func() error { _, err := a.db.Withdraw(t.Context(), withdrawn.ID, store.WithdrawRecord{}); return err },
		func() error {
			_, err := a.db.Close(t.Context(), closed.ID, store.CloseRecord{Outcome: "Pinned in **#12**."})
			return err
		},
		func() error {
			_, err := a.db.Answer(t.Context(), answered.ID, store.AnswerRecord{Ack: true})
			return err
		},
	}
	for _, step := range steps {
		time.Sleep(5 * time.Millisecond)
		if err := step(); err != nil {
			t.Fatal(err)
		}
	}

	body := a.get(t, "/done")
	want := []string{answered.ID, closed.ID, withdrawn.ID}
	if got := idsInOrder(body); !slices.Equal(got, want) {
		t.Errorf("done order = %v\nwant %v", got, want)
	}
	if strings.Contains(body, open.ID) {
		t.Error("open case listed as done")
	}
	if !strings.Contains(body, "Pinned in <strong>#12</strong>.") {
		t.Errorf("closed outcome missing:\n%s", body)
	}
}

func TestDoneFilters(t *testing.T) {
	pinZeroClock(t)
	a := newApp(t)
	withWorker := func(title, worker string) store.OpenRecord {
		return store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyBlocking, Title: title, Worker: worker, Labels: []string{"deps"}, OpenedAt: utc(1, 9, 0)}
	}
	// Answered 12m ago, by a case with a worker.
	answered, err := a.db.Create(t.Context(), withWorker("Answered", "bun-pins"))
	if err != nil {
		t.Fatal(err)
	}
	if err := a.answerAt(utc(16, 19, 48))(answered.ID); err != nil {
		t.Fatal(err)
	}
	// Picked up 3m ago by a named agent: the pickup's by wins over the worker.
	picked, err := a.db.Create(t.Context(), withWorker("Picked up", "opener"))
	if err != nil {
		t.Fatal(err)
	}
	if err := a.answerAt(utc(16, 19, 0))(picked.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Pickup(t.Context(), picked.ID, store.PickupRecord{By: "rel-notes", PickedUpAt: utc(16, 19, 57)}); err != nil {
		t.Fatal(err)
	}
	// Picked up an hour ago with no worker and no by.
	anon := a.through(t, store.KindFYI, "Nobody named", a.answerAt(utc(16, 18, 0)), a.pickupAt(utc(16, 19, 0)))
	closedToday := a.through(t, store.KindFYI, "Closed today", a.answerAt(utc(16, 9, 0)), a.pickupAt(utc(16, 9, 5)), a.closeAt(utc(16, 10, 0)))
	closedYesterday := a.through(t, store.KindFYI, "Closed yesterday", a.answerAt(utc(15, 9, 0)), a.pickupAt(utc(15, 9, 5)), a.closeAt(utc(16, 4, 59)))
	withdrawn := a.through(t, store.KindFYI, "Withdrawn", a.withdrawStep)
	open := a.through(t, store.KindFYI, "Still open")

	chips := []string{
		`<a href="/done?show=inflight"%s>in flight (3)</a>`,
		`<a href="/done?show=closed-today"%s>closed today (1)</a>`,
		`<a href="/done"%s>all (6)</a>`,
	}
	for _, tc := range []struct {
		target, title string
		on            int
		ids           []string
	}{
		{"/done?show=inflight", "<title>(1) cases</title>", 0, []string{picked.ID, answered.ID, anon.ID}},
		{"/done?show=closed-today", "<title>(1) cases</title>", 1, []string{closedToday.ID}},
		{"/done", "<title>(1) cases</title>", 2, nil},
		{"/done?show=all", "<title>(1) cases</title>", 2, nil},
		{"/done?show=bogus", "<title>(1) cases</title>", 2, nil},
	} {
		body := a.get(t, tc.target)
		if !strings.Contains(body, tc.title) {
			t.Errorf("%s: title is not %s", tc.target, tc.title)
		}
		for i, chip := range chips {
			mark := ""
			if i == tc.on {
				mark = ` class="on" aria-current="page"`
			}
			if want := fmt.Sprintf(chip, mark); !strings.Contains(body, want) {
				t.Errorf("%s lacks chip %s", tc.target, want)
			}
		}
		if tc.ids == nil {
			tc.ids = []string{picked.ID, answered.ID, anon.ID, closedToday.ID, closedYesterday.ID, withdrawn.ID}
			got := idsInOrder(body)
			slices.Sort(got)
			slices.Sort(tc.ids)
			if !slices.Equal(got, tc.ids) {
				t.Errorf("%s lists %v\nwant %v", tc.target, got, tc.ids)
			}
		} else if got := idsInOrder(body); !slices.Equal(got, tc.ids) {
			t.Errorf("%s lists %v\nwant %v", tc.target, got, tc.ids)
		}
		if strings.Contains(body, open.ID) {
			t.Errorf("%s lists an open case", tc.target)
		}
	}

	flight := a.get(t, "/done?show=inflight")
	for _, want := range []string{
		`<p class="flight">with <strong>bun-pins</strong> · answered 12m ago</p>`,
		`<p class="flight">with <strong>rel-notes</strong> · picked up 3m ago</p>`,
		`<p class="flight">with <strong>an agent</strong> · picked up 1h ago</p>`,
		`<span class="badge">blocking</span>`,
		`<span class="tag">deps</span>`,
	} {
		if !strings.Contains(flight, want) {
			t.Errorf("in-flight view lacks %s", want)
		}
	}
	if strings.Contains(flight, "opener") {
		t.Error("in-flight view names the worker of a case picked up by someone else")
	}
}

func TestDoneFiltersEmpty(t *testing.T) {
	pinZeroClock(t)
	a := newApp(t)
	for target, want := range map[string]string{
		"/done?show=inflight":     "nothing is with an agent.",
		"/done?show=closed-today": "nothing closed today.",
		"/done":                   "nothing done yet.",
	} {
		if body := a.get(t, target); !strings.Contains(body, `<p class="empty">`+want+`</p>`) {
			t.Errorf("%s lacks %q", target, want)
		}
	}
}

func TestDoneCaseSitsBesideTheDoneList(t *testing.T) {
	pinZeroClock(t)
	a := newApp(t)
	answered := a.through(t, store.KindFYI, "Answered", a.answerAt(utc(16, 19, 48)))
	picked := a.through(t, store.KindFYI, "Picked up", a.answerAt(utc(16, 19, 0)), a.pickupAt(utc(16, 19, 57)))
	closed := a.through(t, store.KindFYI, "Closed", a.answerAt(utc(16, 9, 0)), a.pickupAt(utc(16, 9, 5)), a.closeAt(utc(16, 10, 0)))
	withdrawn := a.through(t, store.KindFYI, "Withdrawn", a.withdrawStep)
	open := a.through(t, store.KindFYI, "Still open")

	selected := func(id string) string {
		return `<a href="/cases/` + id + `" aria-current="page">`
	}
	doneNav := `<a href="/done" class="on" aria-current="page">done</a>`
	for _, tc := range []struct {
		c     *store.Case
		state store.State
		title string
		chip  string
		ids   []string
	}{
		{answered, store.StateAnswered, "<title>(1) cases</title>", `<a href="/done?show=inflight" class="on" aria-current="true">in flight (2)</a>`, []string{picked.ID, answered.ID}},
		{picked, store.StatePickedUp, "<title>(1) cases</title>", `<a href="/done?show=inflight" class="on" aria-current="true">in flight (2)</a>`, []string{picked.ID, answered.ID}},
		{closed, store.StateClosed, "<title>(1) cases</title>", `<a href="/done" class="on" aria-current="true">all (4)</a>`, nil},
		{withdrawn, store.StateWithdrawn, "<title>(1) cases</title>", `<a href="/done" class="on" aria-current="true">all (4)</a>`, nil},
	} {
		target := "/cases/" + tc.c.ID
		body := a.get(t, target)
		for _, want := range []string{tc.title, tc.chip, doneNav, `<aside class="list" aria-label="done">`, `<article class="card selected">`, selected(tc.c.ID)} {
			if !strings.Contains(body, want) {
				t.Errorf("%s lacks %s", target, want)
			}
		}
		if n := strings.Count(body, "card selected"); n != 1 {
			t.Errorf("%s has %d selected cards, want 1", target, n)
		}
		for _, not := range []string{`aria-label="inbox"`, "/fragments/inbox", `class="on" aria-current="page">inbox`, open.ID} {
			if strings.Contains(body, not) {
				t.Errorf("%s has %s", target, not)
			}
		}
		// The list is not refreshed; the tally and the case view are.
		tallyRefresh := `<div hx-get="/fragments/tally" hx-trigger="store-changed from:body" hx-sync="this:replace" hx-swap="none" hidden></div>`
		caseRefresh := `hx-get="/cases/` + tc.c.ID + `/view?state=` + string(tc.state) + `&amp;revision=`
		if n := strings.Count(body, `hx-get=`); n != 2 || !strings.Contains(body, tallyRefresh) || !strings.Contains(body, caseRefresh) {
			t.Errorf("%s has %d refreshing regions, want only the tally and the case view", target, n)
		}
		if tc.ids != nil {
			// The list, then the case's own link in the recorded line or thread, if any.
			if got := idsInOrder(body); len(got) < len(tc.ids) || !slices.Equal(got[:len(tc.ids)], tc.ids) {
				t.Errorf("%s lists %v\nwant %v first", target, got, tc.ids)
			}
		} else {
			for _, id := range []string{answered.ID, picked.ID, closed.ID, withdrawn.ID} {
				if !strings.Contains(body, `href="/cases/`+id+`"`) {
					t.Errorf("%s does not list %s", target, id)
				}
			}
		}
		// A change of state reloads the same case page, which picks the list again.
		w := a.do("GET", target+"/view?state=open&revision=1", nil, map[string]string{"HX-Request": "true"})
		if w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != target {
			t.Errorf("%s case view after a state change: %d, headers %v", target, w.Code, w.Header())
		}
	}

	// The tally refresh carries the title, the icon and the count, and no list.
	frag := a.do("GET", "/fragments/tally", nil, map[string]string{"HX-Request": "true"}).Body.String()
	if want := "<title>(1) cases</title>\n" + `<link id="favicon" rel="icon" type="image/svg+xml" href="/favicon.svg?n=1" hx-swap-oob="true">` + "\n" + `<span id="tally" class="label tally" hx-swap-oob="true"><span><strong>1</strong> today</span></span>`; frag != want {
		t.Errorf("tally fragment = %q\nwant %q", frag, want)
	}

	// An open case is beside the inbox list, as before.
	body := a.get(t, "/cases/"+open.ID)
	for _, want := range []string{`<aside class="list" aria-label="inbox">`, `hx-get="/fragments/inbox?selected=` + open.ID + `"`, `<a href="/" class="on" aria-current="page">inbox</a>`, "<title>(1) cases</title>"} {
		if !strings.Contains(body, want) {
			t.Errorf("open case lacks %s", want)
		}
	}
	if strings.Contains(body, `aria-label="done"`) || strings.Contains(body, "chips filters") {
		t.Error("open case is beside the done list")
	}
	// /done itself marks its filter as the page, and nothing as selected.
	if done := a.get(t, "/done"); !strings.Contains(done, `<a href="/done" class="on" aria-current="page">all (4)</a>`) || strings.Contains(done, "selected") {
		t.Errorf("/done marks its filter or cards wrongly:\n%s", done)
	}
}

func TestEmptyHomeReloadsWhenACaseArrives(t *testing.T) {
	a := newApp(t)
	body := a.get(t, "/")
	if !strings.Contains(body, "<h1>inbox zero.</h1>") || !strings.Contains(body, `hx-get="/fragments/inbox?empty=1"`) {
		t.Fatalf("empty / does not refresh for a first case:\n%s", body)
	}
	hx := map[string]string{"HX-Request": "true"}
	if w := a.do("GET", "/fragments/inbox?empty=1", nil, hx); w.Code != http.StatusOK || !strings.Contains(w.Body.String(), `hx-get="/fragments/inbox?empty=1"`) || !strings.Contains(w.Body.String(), "<h1>inbox zero.</h1>") {
		t.Errorf("still empty: %d %s", w.Code, w.Body.String())
	}
	a.open(t, openRecords[store.KindDecision])
	if w := a.do("GET", "/fragments/inbox?empty=1", nil, hx); w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/" {
		t.Errorf("case arrived: %d, headers %v", w.Code, w.Header())
	}
	// A case page never refreshes with empty=1.
	if w := a.do("GET", "/fragments/inbox", nil, hx); w.Code != http.StatusOK {
		t.Errorf("plain fragment: %d", w.Code)
	}
}

func TestThreadShowsWithdrawReason(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindSignoff])
	if _, err := a.db.Withdraw(t.Context(), c.ID, store.WithdrawRecord{Reason: "superseded by <the other case>"}); err != nil {
		t.Fatal(err)
	}
	if page := a.get(t, "/cases/"+c.ID); !strings.Contains(page, "<p>reason: superseded by &lt;the other case&gt;</p>") {
		t.Errorf("thread missing the reason:\n%s", page)
	}
}

func TestOnlyWebLinksAreAnchors(t *testing.T) {
	a := newApp(t)
	rows := slices.Clone(approvalRows)
	rows[0].Link = "/Users/me/dev/app/setup.sh"
	c := a.open(t, store.OpenRecord{
		Kind: store.KindApproval, Urgency: store.UrgencyToday, Title: "Scripts", Rows: rows,
		Links: []string{"~/notes/plan.md", "javascript:alert(1)", "HTTPS://example.com/upper", "http://example.com/plain"},
	})
	page := a.get(t, "/cases/"+c.ID)
	for _, want := range []string{
		"<p>/Users/me/dev/app/setup.sh</p>",
		"<li>~/notes/plan.md</li>",
		"<li>javascript:alert(1)</li>",
		`<a href="HTTPS://example.com/upper" target="_blank"`,
		`<a href="http://example.com/plain" target="_blank"`,
		`<a href="https://example.com/mig" target="_blank"`,
	} {
		if !strings.Contains(page, want) {
			t.Errorf("page missing %s", want)
		}
	}
	for _, text := range []string{"setup.sh", "plan.md", "javascript:"} {
		if regexp.MustCompile(`href="[^"]*` + regexp.QuoteMeta(text)).MatchString(page) {
			t.Errorf("%s rendered as a link", text)
		}
	}
	if got := strings.Count(page, `target="_blank"`); got != 3 {
		t.Errorf("%d links with a target, want 3", got)
	}
}

func TestCasePageShowsTheBrief(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyToday, Title: "Stuck", Brief: "restart from <step 3> & rerun"})
	bare := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyToday, Title: "No brief"})

	page := a.get(t, "/cases/"+c.ID)
	if want := `<p class="brief"><span class="label">brief</span> restart from &lt;step 3&gt; &amp; rerun</p>`; !strings.Contains(page, want) {
		t.Errorf("page missing %s:\n%s", want, page)
	}
	if strings.Count(page, "restart from") != 1 {
		t.Error("brief shown more than once: the inbox cards leave it out")
	}
	if page := a.get(t, "/cases/"+bare.ID); strings.Contains(page, `class="brief"`) {
		t.Error("brief line on a case without a brief")
	}
}

func TestTimestampsAreLocalWithAnAge(t *testing.T) {
	old := zone
	zone = time.FixedZone("XST", -5*60*60)
	t.Cleanup(func() { zone = old })

	a := newApp(t)
	opened := time.Date(2026, 1, 2, 3, 4, 0, 0, time.UTC)
	c := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyToday, Title: "Stuck", OpenedAt: opened})
	if _, err := a.db.Park(t.Context(), c.ID, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	page := a.get(t, "/cases/"+c.ID)
	for _, want := range []*regexp.Regexp{
		// The header: the day before in XST, and days old.
		regexp.MustCompile(`<span class="age">opened 2026-01-01 22:04 XST, \d+d ago</span>`),
		// The thread: the open event, and the park just written.
		regexp.MustCompile(`<span>open</span> <span class="age">2026-01-01 22:04 XST, \d+d ago</span>`),
		regexp.MustCompile(`<span>park</span> <span class="age">\d{4}-\d\d-\d\d \d\d:\d\d XST, now</span>`),
	} {
		if !want.MatchString(page) {
			t.Errorf("page does not match %s:\n%s", want, page)
		}
	}
	if strings.Contains(page, "UTC") {
		t.Error("page still shows a UTC stamp")
	}
}

func TestApprovalRowNote(t *testing.T) {
	a := newApp(t)
	rows := slices.Clone(approvalRows)
	rows[1].Note = "takes the site down <briefly>"
	c := a.open(t, store.OpenRecord{Kind: store.KindApproval, Urgency: store.UrgencyToday, Title: "Scripts", Rows: rows})
	page := a.get(t, "/cases/"+c.ID)
	if want := `<legend>Migrate</legend>
    <p class="context">takes the site down &lt;briefly&gt;</p>`; !strings.Contains(page, want) {
		t.Errorf("page missing %s:\n%s", want, page)
	}
	if got := strings.Count(page, `<p class="context">`); got != 1 {
		t.Errorf("%d notes, want 1", got)
	}
}

func TestExternalLinksOpenInANewTab(t *testing.T) {
	a := newApp(t)
	const newTab = `target="_blank" rel="noopener noreferrer"`
	c := a.open(t, store.OpenRecord{
		Kind: store.KindApproval, Urgency: store.UrgencyToday, Title: "Links", Rows: approvalRows,
		Body:  "[ext](https://example.com/body) <https://example.com/auto> [inbox](/done) [top](#top) [pct](https://example.com/100%) [rel](//example.com/rel)\n\n[forged](https://example.com/forged){target=_self}",
		Links: []string{"https://example.com/listed"},
	})
	if _, err := a.db.Note(t.Context(), c.ID, store.NoteRecord{Body: "see [the note link](https://example.com/note)"}); err != nil {
		t.Fatal(err)
	}
	page := a.get(t, "/cases/"+c.ID)
	for _, want := range []string{
		`<a href="https://example.com/body" ` + newTab + `>ext</a>`,
		`<a href="https://example.com/auto" ` + newTab + `>https://example.com/auto</a>`,
		// The body cannot set its own attributes: that syntax stays text.
		`<a href="https://example.com/forged" ` + newTab + `>forged</a>{target=_self}`,
		`<a href="https://example.com/note" ` + newTab + `>the note link</a>`,
		`<a href="https://example.com/listed" ` + newTab + `>`,
		`<a href="https://example.com/deps" ` + newTab + `>`,
		`<a href="/done">inbox</a>`,
		`<a href="#top">top</a>`,
		`<a href="https://example.com/100%25" ` + newTab + `>pct</a>`,
		`<a href="//example.com/rel" ` + newTab + `>rel</a>`,
	} {
		if !strings.Contains(page, want) {
			t.Errorf("page missing %s", want)
		}
	}
	// Only the external links, both approval rows included: nothing internal
	// opens a new tab.
	if got := strings.Count(page, `target="_blank"`); got != 9 {
		t.Errorf("%d links with a target, want 9:\n%s", got, page)
	}

	closed := a.open(t, openRecords[store.KindFYI])
	for _, step := range []func() error{
		func() error { _, err := a.db.Answer(t.Context(), closed.ID, store.AnswerRecord{Ack: true}); return err },
		func() error { _, err := a.db.Pickup(t.Context(), closed.ID, store.PickupRecord{}); return err },
		func() error {
			_, err := a.db.Close(t.Context(), closed.ID, store.CloseRecord{Outcome: "Shipped in [#12](https://example.com/pr/12)."})
			return err
		},
	} {
		if err := step(); err != nil {
			t.Fatal(err)
		}
	}
	done := a.get(t, "/done")
	if !strings.Contains(done, `<a href="https://example.com/pr/12" `+newTab+`>#12</a>`) || strings.Count(done, `target="_blank"`) != 1 {
		t.Errorf("done page links:\n%s", done)
	}
}

func TestEmptyAnswersAreRefused(t *testing.T) {
	blank := "   "
	forms := map[string]url.Values{
		"nothing sent": {},
		"blank fields": {"choice": {""}, "signoff": {""}, "stuck": {""}, "verdict.deps": {""}, "verdict.mig": {""}, "text": {""}, "note": {""}},
		"whitespace":   {"text": {blank}, "note": {blank}, "note.deps": {blank}, "note.mig": {blank}},
		"note only":    {"note": {"just a note"}},
	}
	for _, kind := range store.Kinds {
		for name, form := range forms {
			t.Run(string(kind)+"/"+name, func(t *testing.T) {
				a := newApp(t)
				c := a.open(t, openRecords[kind])
				w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, 1), nil)
				if w.Code != http.StatusUnprocessableEntity || refusal(t, w.Body.String()) == "" {
					t.Errorf("status %d, want 422 with an error:\n%s", w.Code, w.Body.String())
				}
				if files := a.eventFiles(t, c.ID); len(files) != 1 {
					t.Errorf("files = %v", files)
				}
			})
		}
	}
}

func TestChoicesAreRequiredInTheBrowser(t *testing.T) {
	a := newApp(t)
	for kind, radios := range map[store.Kind]int{store.KindDecision: 3, store.KindApproval: 6, store.KindSignoff: 2, store.KindStuck: 0, store.KindQuestion: 0, store.KindFYI: 0} {
		c := a.open(t, openRecords[kind])
		page := a.get(t, "/cases/"+c.ID)
		// Stuck is left to the server: guidance typed without ticking its
		// button still counts, which a required radio group would block.
		// fyi has nothing to choose.
		if got := strings.Count(page, `type="radio" required`); got != radios {
			t.Errorf("%s: %d required radios, want %d", kind, got, radios)
		}
	}
}

func TestTallyHidesZeros(t *testing.T) {
	a := newApp(t)
	// No cases: no figures, and the span stays for the out-of-band swap.
	if body := a.get(t, "/done"); !strings.Contains(body, `<span id="tally" class="label tally"></span>`) || !strings.Contains(body, "<title>cases</title>") {
		t.Errorf("empty store tally or title:\n%s", body)
	}
	a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Later"})
	a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Much later"})
	body := a.get(t, "/done")
	if !strings.Contains(body, `<span id="tally" class="label tally"><span><strong>2</strong> whenever</span></span>`) {
		t.Errorf("tally is not just the whenever count:\n%s", body)
	}
	// The title counts the open cases, so it leads with the two here.
	if strings.Contains(body, `class="hot"`) || !strings.Contains(body, "<title>(2) cases</title>") {
		t.Errorf("a tally without blocking cases is red, or its title lacks the open count:\n%s", body)
	}
}

func TestThreadShowsAnUnknownEventAsVersionSkew(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindFYI])
	storetest.InsertEvent(t, a.db.Path, c.ID, 2, "agent", "comment", `{"body":"x"}`)
	page := a.get(t, "/cases/"+c.ID)
	if !strings.Contains(page, `<p class="error">0002-agent-comment.json: unknown event &#34;comment&#34;: perhaps written by a newer cases, or not by cases at all; if newer, update cases on this machine with go install github.com/ryanlewis/cases/cmd/cases@latest</p>`) {
		t.Errorf("thread missing the version skew problem:\n%s", page)
	}
}

// failAnswer is a store whose Answer always fails.
type failAnswer struct{ store.Store }

func (failAnswer) Answer(context.Context, string, store.AnswerRecord, ...store.Precondition) (*store.Case, error) {
	return nil, errors.New("store unavailable")
}

func TestAnswerWritesThroughTheStore(t *testing.T) {
	a := newAppWith(t, func(s store.Store) store.Store { return failAnswer{s} })
	c := a.open(t, openRecords[store.KindDecision])

	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"choice": {"1"}}, c.Revision()), nil)
	if w.Code != http.StatusUnprocessableEntity || !strings.Contains(w.Body.String(), "store unavailable") {
		t.Errorf("status %d, body:\n%s", w.Code, w.Body.String())
	}
	if files := a.eventFiles(t, c.ID); len(files) != 1 {
		t.Errorf("files = %v, want only the open event", files)
	}
}

// cancelOnAnswer is a store that cancels the request's context as the answer
// is written, as a browser that goes away mid-post does.
type cancelOnAnswer struct {
	store.Store
	cancel context.CancelFunc
}

func (s cancelOnAnswer) Answer(ctx context.Context, id string, rec store.AnswerRecord, pre ...store.Precondition) (*store.Case, error) {
	s.cancel()
	return s.Store.Answer(ctx, id, rec, pre...)
}

// A browser that goes away once it has sent the form, such as a tab closed
// straight after, cancels the request. The answer is still recorded.
func TestAnswerIsRecordedWhenTheBrowserGoesAway(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	a := newAppWith(t, func(s store.Store) store.Store { return cancelOnAnswer{s, cancel} })
	c := a.open(t, openRecords[store.KindDecision])

	form := withRevision(url.Values{"choice": {"1"}}, c.Revision())
	r := httptest.NewRequestWithContext(ctx, "POST", "/cases/"+c.ID+"/answer", strings.NewReader(form.Encode()))
	r.Host = testAddr
	r.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	w := httptest.NewRecorder()
	a.handler.ServeHTTP(w, r)
	if w.Code != http.StatusSeeOther {
		t.Errorf("status %d, body:\n%s", w.Code, w.Body.String())
	}
	if got, err := a.db.Get(t.Context(), c.ID); err != nil || got.State != store.StateAnswered {
		t.Errorf("case after the browser went away: %+v, %v; want answered", got, err)
	}
}

// pinZeroClock puts the inbox-zero clock at Wednesday 2026-09-16 15:00 in a
// zone five hours behind UTC, so today starts at 05:00 UTC that day and the
// week at 05:00 UTC on Monday the 14th.
func pinZeroClock(t *testing.T) {
	t.Helper()
	oldZone, oldClock := zone, clock
	zone = time.FixedZone("XST", -5*60*60)
	clock = func() time.Time { return time.Date(2026, 9, 16, 20, 0, 0, 0, time.UTC) }
	t.Cleanup(func() { zone, clock = oldZone, oldClock })
}

// utc is a time on 2026-09-dd in UTC.
func utc(day, hour, minute int) time.Time {
	return time.Date(2026, 9, day, hour, minute, 0, 0, time.UTC)
}

// through opens a case of kind and applies steps to it, by id, failing on
// any error.
func (a *testApp) through(t *testing.T, kind store.Kind, title string, steps ...func(id string) error) *store.Case {
	t.Helper()
	c := a.open(t, store.OpenRecord{Kind: kind, Urgency: store.UrgencyToday, Title: title, OpenedAt: utc(1, 9, 0)})
	for _, step := range steps {
		if err := step(c.ID); err != nil {
			t.Fatalf("%s: %v", title, err)
		}
	}
	return c
}

// answerAt answers an fyi case, or with text any other.
func (a *testApp) answerAt(at time.Time) func(string) error {
	return func(id string) error {
		ctx := context.Background()
		rec := store.AnswerRecord{Ack: true, AnsweredAt: at}
		if c, err := a.db.Get(ctx, id); err == nil && c.Kind != store.KindFYI {
			rec = store.AnswerRecord{Text: "go on", AnsweredAt: at}
		}
		_, err := a.db.Answer(ctx, id, rec)
		return err
	}
}

func (a *testApp) pickupAt(at time.Time) func(string) error {
	return func(id string) error {
		_, err := a.db.Pickup(context.Background(), id, store.PickupRecord{PickedUpAt: at})
		return err
	}
}

func (a *testApp) closeAt(at time.Time) func(string) error {
	return func(id string) error {
		_, err := a.db.Close(context.Background(), id, store.CloseRecord{Outcome: "done", ClosedAt: at})
		return err
	}
}

// withdrawStep withdraws the case.
func (a *testApp) withdrawStep(id string) error {
	_, err := a.db.Withdraw(context.Background(), id, store.WithdrawRecord{})
	return err
}

func TestInboxZeroCountsWhatWasGotThrough(t *testing.T) {
	pinZeroClock(t)
	a := newApp(t)
	// Answered, picked up and closed today; the last answer, an hour ago.
	a.through(t, store.KindFYI, "Today closed", a.answerAt(utc(16, 19, 0)), a.pickupAt(utc(16, 19, 10)), a.closeAt(utc(16, 19, 30)))
	// Answered on Monday and not picked up: this week, and with an agent.
	a.through(t, store.KindFYI, "Monday", a.answerAt(utc(14, 15, 0)))
	// Answered last week, picked up: only with an agent.
	a.through(t, store.KindFYI, "Last week", a.answerAt(utc(10, 15, 0)), a.pickupAt(utc(10, 16, 0)))
	// Closed yesterday: counts nowhere.
	a.through(t, store.KindFYI, "Yesterday", a.answerAt(utc(8, 15, 0)), a.pickupAt(utc(8, 16, 0)), a.closeAt(utc(15, 20, 0)))
	// Parked and resumed today by the human, then answered: one case today.
	a.through(t, store.KindStuck, "Parked first",
		func(id string) error {
			_, err := a.db.Park(context.Background(), id, store.ParkRecord{ParkedAt: utc(16, 17, 0)})
			return err
		},
		func(id string) error {
			_, err := a.db.Resume(context.Background(), id, store.AuthorHuman, store.ResumeRecord{ResumedAt: utc(16, 17, 30)})
			return err
		},
		a.answerAt(utc(16, 18, 0)))
	// Just after midnight before today, in the zone: yesterday there, so not today.
	a.through(t, store.KindFYI, "Late night", a.answerAt(utc(16, 4, 59)), a.pickupAt(utc(16, 5, 0)), a.closeAt(utc(16, 4, 59)))
	// Withdrawn: not the human's doing.
	a.through(t, store.KindFYI, "Withdrawn", a.withdrawStep)

	body := a.get(t, "/")
	for _, want := range []string{
		`<section class="zero" aria-label="inbox">`,
		"<h1>inbox zero.</h1>",
		"<p>you got through <strong>2</strong> cases today, <strong>4</strong> this week.</p>",
		`<p><a href="/done?show=closed-today"><strong>1</strong> closed</a> by agents today.</p>`,
		`<p><a href="/done?show=inflight"><strong>3</strong> answered</a> and still with an agent.</p>`,
		`<p class="age">last answered 1h ago.</p>`,
		`hx-get="/fragments/inbox?empty=1"`,
	} {
		if !strings.Contains(body, want) {
			t.Errorf("/ lacks %s", want)
		}
	}
	if strings.Contains(body, `class="split`) || strings.Contains(body, "nothing has needed you yet.") || strings.Contains(body, "no open cases.") {
		t.Errorf("/ at inbox zero still shows the columns or the fresh-store line:\n%s", body)
	}
	// The refresh renders the same panel.
	frag := a.do("GET", "/fragments/inbox?empty=1", nil, map[string]string{"HX-Request": "true"}).Body.String()
	if !strings.Contains(frag, "<p>you got through <strong>2</strong> cases today, <strong>4</strong> this week.</p>") {
		t.Errorf("fragment lacks the panel:\n%s", frag)
	}
}

func TestInboxZeroHidesZeroLines(t *testing.T) {
	pinZeroClock(t)
	a := newApp(t)
	body := a.get(t, "/")
	if !strings.Contains(body, "<h1>inbox zero.</h1>") || !strings.Contains(body, "<p>nothing has needed you yet.</p>") {
		t.Errorf("fresh store:\n%s", body)
	}

	// One case answered and closed on Monday: a week figure and an age, nothing else.
	a.through(t, store.KindFYI, "Monday", a.answerAt(utc(14, 15, 0)), a.pickupAt(utc(14, 15, 5)), a.closeAt(utc(14, 16, 0)))
	body = a.get(t, "/")
	if !strings.Contains(body, "<p>you got through <strong>1</strong> case this week.</p>") || !strings.Contains(body, `<p class="age">last answered 2d ago.</p>`) {
		t.Errorf("week line or age missing:\n%s", body)
	}
	for _, unwanted := range []string{"today", "closed</a>", "still with an agent", "nothing has needed you yet."} {
		if strings.Contains(body[strings.Index(body, `<section class="zero"`):], unwanted) {
			t.Errorf("panel shows %q", unwanted)
		}
	}

	// A case arrives: the two columns are back, and no panel.
	a.open(t, openRecords[store.KindDecision])
	body = a.get(t, "/")
	if !strings.Contains(body, `<div class="split home">`) || strings.Contains(body, "inbox zero.") {
		t.Errorf("/ with an open case:\n%s", body)
	}
}

func TestThreadShowsActorAndFor(t *testing.T) {
	a := newApp(t)
	a.server.Actor = &store.Actor{Name: "Ryan", Kind: "human"}
	rec := openRecords[store.KindFYI]
	rec.For = "Ryan"
	rec.Worker = "bun-pins"
	rec.Actor = &store.Actor{Name: "bun-pins", Kind: "agent"}
	c := a.open(t, rec)

	for _, page := range []string{a.get(t, "/"), a.get(t, "/cases/"+c.ID)} {
		if !strings.Contains(page, "<span>for Ryan</span>") {
			t.Errorf("page missing for:\n%s", page)
		}
	}
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"ack": {"1"}}, c.Revision()), nil); w.Code != http.StatusSeeOther {
		t.Fatalf("answer: %d %s", w.Code, w.Body.String())
	}
	loaded, err := a.db.Get(t.Context(), c.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got := loaded.Events[1].Actor; got == nil || *got != *a.server.Actor {
		t.Errorf("answer actor = %+v", got)
	}
	page := a.get(t, "/cases/"+c.ID)
	for _, want := range []string{"<p>by bun-pins</p>", "<p>by Ryan</p>"} {
		if !strings.Contains(page, want) {
			t.Errorf("thread missing %s:\n%s", want, page)
		}
	}
}

func TestTitleCountsCasesWaitingOnTheHuman(t *testing.T) {
	a := newApp(t)
	first := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyBlocking, Title: "First"})
	second := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Second"})
	parked := a.open(t, store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyToday, Title: "Parked"})
	if _, err := a.db.Park(t.Context(), parked.ID, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	hx := map[string]string{"HX-Request": "true"}
	// Two open cases of any urgency; the parked one waits on nobody.
	for path, want := range map[string]string{
		"/":                   "<title>(2) cases</title>",
		"/done":               "<title>(2) cases</title>",
		"/cases/" + parked.ID: "<title>(2) cases</title>",
		"/fragments/inbox":    "<title>(2) cases</title>",
	} {
		if body := a.do("GET", path, nil, hx).Body.String(); !strings.Contains(body, want) {
			t.Errorf("%s lacks %s", path, want)
		}
	}

	// The refresh carries the new count once a case is answered, and a bare
	// title once none is waiting.
	if _, err := a.db.Answer(t.Context(), first.ID, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	if body := a.do("GET", "/fragments/inbox", nil, hx).Body.String(); !strings.HasPrefix(body, "<title>(1) cases</title>") {
		t.Errorf("fragment after an answer:\n%s", body)
	}
	if _, err := a.db.Answer(t.Context(), second.ID, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	for path, want := range map[string]string{
		"/":                  "<title>cases</title>",
		"/done":              "<title>cases</title>",
		"/cases/" + first.ID: "<title>cases</title>",
		"/fragments/inbox":   "<title>cases</title>",
	} {
		if body := a.do("GET", path, nil, hx).Body.String(); !strings.Contains(body, want) {
			t.Errorf("%s at zero lacks %s", path, want)
		}
	}
	// With the parked case resumed and answered too, / is the inbox zero
	// page, titled bare.
	if _, err := a.db.Resume(t.Context(), parked.ID, store.AuthorHuman, store.ResumeRecord{}); err != nil {
		t.Fatal(err)
	}
	if _, err := a.db.Answer(t.Context(), parked.ID, store.AnswerRecord{Text: "go on"}); err != nil {
		t.Fatal(err)
	}
	if body := a.get(t, "/"); !strings.Contains(body, "<title>cases</title>") || !strings.Contains(body, "inbox zero.") {
		t.Errorf("inbox zero title:\n%s", body)
	}
}
