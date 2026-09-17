package web

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
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
	if _, err := store.Answer(answered.Dir, store.AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Park(oldBlocking.Dir, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}

	body := a.get(t, "/")
	want := []string{oldBlocking.ID, newBlocking.ID, today.ID, later.ID}
	if got := idsInOrder(body); !slices.Equal(got, want) {
		t.Errorf("inbox order = %v\nwant %v", got, want)
	}
	// / shows the first case beside the list, selected, and titled after it.
	// One open blocking case: the parked one and the answered one do not count.
	if !strings.Contains(body, "<title>(1) Old blocker · cases</title>") {
		t.Errorf("title missing blocking count or case title:\n%s", body)
	}
	if !strings.Contains(body, `<div class="split home">`) || !strings.Contains(body, `action="/cases/`+oldBlocking.ID+`/resume"`) ||
		!strings.Contains(body, `hx-get="/cases/`+oldBlocking.ID+`/thread?state=parked&amp;home=1"`) {
		t.Errorf("/ does not show the first case:\n%s", body)
	}
	if !strings.Contains(body, `selected" href="/cases/`+oldBlocking.ID+`" aria-current="page">`) {
		t.Error("first case is not selected on /")
	}
	if !strings.Contains(body, `hx-get="/fragments/inbox?selected=`+oldBlocking.ID+`"`) || !strings.Contains(body, `hx-trigger="every 2s"`) {
		t.Error("inbox does not poll with the selection")
	}

	frag := a.do("GET", "/fragments/inbox", nil, map[string]string{"HX-Request": "true"})
	list := frag.Body.String()
	if frag.Code != http.StatusOK || !slices.Equal(idsInOrder(list), want) {
		t.Errorf("fragment %d, ids %v", frag.Code, idsInOrder(list))
	}
	if !strings.Contains(list, "<title>(1) inbox · cases</title>") || strings.Contains(list, "<html") || strings.Contains(list, "selected") {
		t.Errorf("fragment is not a bare list with a title and no selection:\n%s", list)
	}
	// The header tally is swapped out of band, so it keeps up with the title.
	if !strings.Contains(list, `<span id="tally" class="label tally hot" hx-swap-oob="true"><strong>1</strong> blocking</span>`) {
		t.Errorf("fragment lacks the out-of-band tally:\n%s", list)
	}
	if !strings.Contains(body, `<span id="tally" class="label tally hot"><strong>1</strong> blocking</span>`) || !strings.Contains(body, `<a href="/" class="on" aria-current="page">inbox</a>`) {
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

	// The polled list keeps the selection and the selected case's title.
	sel := a.get(t, "/fragments/inbox?selected="+today.ID)
	if !strings.Contains(sel, `class="card urgency-today selected" href="/cases/`+today.ID+`" aria-current="page"`) ||
		strings.Count(sel, "selected") != 2 || !strings.Contains(sel, `hx-get="/fragments/inbox?selected=`+today.ID+`"`) ||
		!strings.Contains(sel, "<title>(1) Today · cases</title>") {
		t.Errorf("fragment lost the selection:\n%s", sel)
	}

	// A case page shows the list beside it, with that case selected.
	page := a.get(t, "/cases/"+today.ID)
	if !strings.Contains(page, `<div class="split">`) || !slices.Equal(idsInOrder(page), want) ||
		!strings.Contains(page, `urgency-today selected" href="/cases/`+today.ID+`" aria-current="page"`) || strings.Count(page, `selected" href=`) != 1 {
		t.Errorf("case page lacks the list with the case selected:\n%s", page)
	}

	// A case opened after the page loaded shows up on the next poll.
	fresh := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Fresh"})
	if got := a.get(t, "/fragments/inbox"); !strings.Contains(got, fresh.ID) {
		t.Error("poll did not pick up a new case")
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
			formParts: []string{`name="stuck" value="text"`, `name="text"`, `name="stuck" value="drop"`, `name="park" value="1" formnovalidate>park</button>`},
			form:      url.Values{"stuck": {"text"}, "text": {"use the mirror"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindQuestion,
			formParts: []string{`<textarea name="text" rows="4" required>`, "reply"},
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
			if w.Code != http.StatusSeeOther || w.Header().Get("Location") != "/" {
				t.Fatalf("post: %d %q %s", w.Code, w.Header().Get("Location"), w.Body.String())
			}
			if files := eventFiles(t, c.Dir); !slices.Equal(files, []string{"0001-agent-open.json", tt.wantFile}) {
				t.Errorf("files = %v", files)
			}
			loaded, err := store.Load(c.Dir)
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

// A case amended after its page was loaded: the page shows the case as
// amended, a form from the older page is refused, and an answer needs a
// verdict on the added row.
func TestAmendedCase(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	c := a.open(t, openRecords[store.KindApproval])
	before := a.get(t, "/cases/"+c.ID)
	if _, err := store.Amend(c.Dir, store.AmendRecord{
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
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, rev), origin); w.Code != http.StatusUnprocessableEntity || !strings.Contains(w.Body.String(), "reject for &#34;Deploy&#34;") {
		t.Errorf("answer without the added row: %d %s", w.Code, w.Body.String())
	}
	form.Set("verdict.deploy", "hold")
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(form, rev), origin); w.Code != http.StatusSeeOther {
		t.Errorf("answer with every row: %d %s", w.Code, w.Body.String())
	}
	if files := eventFiles(t, c.Dir); !slices.Equal(files, []string{"0001-agent-open.json", "0002-agent-amend.json", "0003-human-answer.json"}) {
		t.Errorf("files = %v", files)
	}
}

// The thread shows the body and context an amend replaced, each in a details
// element the poll keeps as the human left it: the body as markdown with raw
// HTML dropped, and the context escaped. A case that had no body before has
// no previous body to show.
func TestThreadShowsWhatAnAmendReplaced(t *testing.T) {
	const body, context = "Was **bold** & <script>alert(1)</script> <b>raw</b>", `Release <1.4> & "quoted"`
	a := newApp(t)
	c := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Heads up", Body: body, Context: context})
	if _, err := store.Amend(c.Dir, store.AmendRecord{Body: "Now plain.", Context: "Release 1.4.1"}); err != nil {
		t.Fatal(err)
	}
	for _, target := range []string{"/cases/" + c.ID, "/cases/" + c.ID + "/thread?state=open"} {
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
	if _, err := store.Amend(bare.Dir, store.AmendRecord{Body: "A body at last.", Context: "Release 1.4"}); err != nil {
		t.Fatal(err)
	}
	if page := a.get(t, "/cases/"+bare.ID); !strings.Contains(page, "<p>replaced the body</p>") || strings.Contains(page, "<details") {
		t.Errorf("a case with no body or context before shows a previous one:\n%s", page)
	}
}

// An amend that syncs in after a later one changes what the later one
// replaced. The details under the later one then gets a new id, so the poll
// does not keep the old text in its place.
func TestThreadDetailsFollowTheText(t *testing.T) {
	a := newApp(t)
	c := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyToday, Title: "Heads up", Body: "First."})
	arrive := func(name, data string) {
		t.Helper()
		if err := os.WriteFile(filepath.Join(c.Dir, name), []byte(data), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	thread := "/cases/" + c.ID + "/thread?state=open"

	arrive("0003-agent-amend.json", `{"body":"Third."}`)
	stale := `<details id="previous-body-3-` + textID("First.") + `" hx-preserve>`
	if page := a.get(t, thread); !strings.Contains(page, stale) {
		t.Fatalf("thread missing %s:\n%s", stale, page)
	}

	arrive("0002-agent-amend.json", `{"body":"Second."}`)
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

func TestStuckParkAndResume(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindStuck])
	origin := map[string]string{"Origin": "http://" + testAddr}

	// The park button parks even with guidance ticked: the browser sends both.
	if w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"stuck": {"text"}, "park": {"1"}, "note": {"after release"}}, 1), origin); w.Code != http.StatusSeeOther {
		t.Fatalf("park: %d %s", w.Code, w.Body.String())
	}
	if files := eventFiles(t, c.Dir); !slices.Equal(files, []string{"0001-agent-open.json", "0002-human-park.json"}) {
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
	loaded, err := store.Load(c.Dir)
	if err != nil || loaded.State != store.StateOpen || loaded.Events[2].Author != store.AuthorHuman {
		t.Errorf("after resume: %+v %v", loaded, err)
	}
	// Resuming an open case is refused and writes nothing.
	if w := a.do("POST", "/cases/"+c.ID+"/resume", withRevision(nil, 3), origin); w.Code != http.StatusUnprocessableEntity {
		t.Errorf("resume of open case: %d", w.Code)
	}
	if n := len(eventFiles(t, c.Dir)); n != 3 {
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
	if _, err := store.Park(parked.Dir, store.ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	zed := a.open(t, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyWhenever, Title: "Zed"})

	steps := []struct {
		name, target string
		form         url.Values
		want         string
	}{
		// A parked case stays in the inbox, and the page still moves on.
		{"park", "/cases/" + blocked.ID + "/answer", withRevision(url.Values{"stuck": {"park"}}, 1), "/cases/" + decision.ID},
		{"answer", "/cases/" + decision.ID + "/answer", withRevision(url.Values{"choice": {"1"}}, 1), "/cases/" + parked.ID},
		{"resume", "/cases/" + parked.ID + "/resume", withRevision(nil, 2), "/cases/" + zed.ID},
		{"answer the last", "/cases/" + zed.ID + "/answer", withRevision(url.Values{"ack": {"1"}}, 1), "/"},
		{"resume the first", "/cases/" + blocked.ID + "/resume", withRevision(nil, 2), "/cases/" + parked.ID},
	}
	for _, st := range steps {
		w := a.do("POST", st.target, st.form, origin)
		if w.Code != http.StatusSeeOther || w.Header().Get("Location") != st.want {
			t.Errorf("%s: %d %q, want %q %s", st.name, w.Code, w.Header().Get("Location"), st.want, w.Body.String())
		}
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
		{store.KindApproval, url.Values{"verdict.deps": {"approve"}, "note.deps": {"fine"}}, "choose approve, hold or reject for &#34;Migrate&#34;", `value="fine"`},
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
			if !strings.Contains(body, tt.wantErr) || !strings.Contains(body, `class="error"`) {
				t.Errorf("body lacks error %q:\n%s", tt.wantErr, body)
			}
			if tt.keeps != "" && !strings.Contains(body, tt.keeps) {
				t.Errorf("re-rendered form lost %q", tt.keeps)
			}
			if files := eventFiles(t, c.Dir); len(files) != 1 {
				t.Errorf("files = %v", files)
			}
		})
	}
}

func TestAnswerOnAClosedCaseOrUnknownCase(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindFYI])
	if _, err := store.Withdraw(c.Dir, store.WithdrawRecord{}); err != nil {
		t.Fatal(err)
	}
	w := a.do("POST", "/cases/"+c.ID+"/answer", withRevision(url.Values{"ack": {"1"}}, 2), nil)
	if w.Code != http.StatusUnprocessableEntity || !strings.Contains(w.Body.String(), "cannot answer a case that is withdrawn") {
		t.Errorf("%d %s", w.Code, w.Body.String())
	}
	for _, target := range []string{"/cases/nope", "/cases/nope/thread"} {
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
	if _, err := store.Pickup(c.Dir, store.PickupRecord{By: "bun-pins"}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Note(c.Dir, store.NoteRecord{Body: "Pin to which **patch**?"}); err != nil {
		t.Fatal(err)
	}
	written := eventFiles(t, c.Dir)

	// Tab B's form is from before the answer. The case is open again, but the
	// form is refused and nothing is written.
	w := a.do("POST", target, withRevision(url.Values{"choice": {"2"}, "note": {"float it"}}, pageRevision(t, tabB)), origin)
	if w.Code != http.StatusConflict {
		t.Fatalf("tab B: %d, want 409\n%s", w.Code, w.Body.String())
	}
	if files := eventFiles(t, c.Dir); !slices.Equal(files, written) {
		t.Errorf("files = %v, want %v", files, written)
	}
	// Tab B gets the case as it is now, with what it typed, and a form at the
	// current revision.
	body := w.Body.String()
	for _, want := range []string{`<p class="error" role="alert">` + staleForm + `</p>`, "Pin to which <strong>patch</strong>?", "float it"} {
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
	if files := eventFiles(t, c.Dir); len(files) != 5 || files[4] != "0005-human-answer.json" {
		t.Errorf("files = %v", files)
	}
}

func TestStaleParkAndResumeAreRefused(t *testing.T) {
	a := newApp(t)
	origin := map[string]string{"Origin": "http://" + testAddr}
	c := a.open(t, openRecords[store.KindStuck])
	park := func() {
		t.Helper()
		if _, err := store.Park(c.Dir, store.ParkRecord{}); err != nil {
			t.Fatal(err)
		}
	}
	resume := func() {
		t.Helper()
		if _, err := store.Resume(c.Dir, store.AuthorAgent, store.ResumeRecord{}); err != nil {
			t.Fatal(err)
		}
	}
	refused := func(name, action string, form url.Values) {
		t.Helper()
		before := eventFiles(t, c.Dir)
		w := a.do("POST", "/cases/"+c.ID+"/"+action, form, origin)
		if w.Code != http.StatusConflict || !strings.Contains(w.Body.String(), staleForm) {
			t.Errorf("%s: %d, want 409 with the stale form error\n%s", name, w.Code, w.Body.String())
		}
		if files := eventFiles(t, c.Dir); !slices.Equal(files, before) {
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
	loaded, err := store.Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := store.Note(c.Dir, store.NoteRecord{Body: "Pin to which **patch**?"}); err != nil {
		t.Fatal(err)
	}
	_, err = store.Answer(c.Dir, store.AnswerRecord{Choice: 1}, store.AtRevision(loaded.Revision()))
	if !errors.Is(err, store.ErrStale) {
		t.Fatalf("err = %v, want ErrStale", err)
	}

	w := httptest.NewRecorder()
	a.server.refuse(w, loaded, nil, err, url.Values{"choice": {"1"}, "note": {"float it"}})
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

func TestThreadFragment(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindDecision])
	if _, err := store.Answer(c.Dir, store.AnswerRecord{Choice: 1, Note: "ship it"}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Pickup(c.Dir, store.PickupRecord{By: "bun-pins"}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Note(c.Dir, store.NoteRecord{Body: "Which **patch**?"}); err != nil {
		t.Fatal(err)
	}

	page := a.get(t, "/cases/"+c.ID)
	if !strings.Contains(page, `hx-get="/cases/`+c.ID+`/thread?state=open"`) {
		t.Error("case page does not poll its thread")
	}
	// A page rendered while the case was first open polls the same way: the
	// answer and the note that reopened the case leave the state as the page
	// shows it, so the thread updates in place and the form keeps what was
	// typed. Sending that form is refused (TestStaleTabCannotAnswerAReopenedCase).
	frag := a.get(t, "/cases/"+c.ID+"/thread?state=open")
	for _, want := range []string{"chose 1. Pin", "note: ship it", "by bun-pins", "Which <strong>patch</strong>?", "human", "agent"} {
		if !strings.Contains(frag, want) {
			t.Errorf("thread missing %q:\n%s", want, frag)
		}
	}
	if strings.Contains(frag, "<html") {
		t.Error("thread fragment is a full page")
	}

	// The page was rendered while answered; the case has since reopened, so
	// the fragment sends the browser back to the case page with a GET.
	w := a.do("GET", "/cases/"+c.ID+"/thread?state=answered", nil, map[string]string{"HX-Request": "true"})
	if w.Header().Get("HX-Redirect") != "/cases/"+c.ID || w.Header().Get("HX-Refresh") != "" {
		t.Errorf("stale state: %d, headers %v", w.Code, w.Header())
	}
	// On / it sends the browser to /, which shows whichever case is first now.
	w = a.do("GET", "/cases/"+c.ID+"/thread?state=answered&home=1", nil, map[string]string{"HX-Request": "true"})
	if w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/" {
		t.Errorf("stale state on /: %d, headers %v", w.Code, w.Header())
	}
}

func TestDoneView(t *testing.T) {
	a := newApp(t)
	answered := a.open(t, openRecords[store.KindFYI])
	closed := a.open(t, openRecords[store.KindDecision])
	withdrawn := a.open(t, openRecords[store.KindSignoff])
	open := a.open(t, openRecords[store.KindStuck])

	steps := []func() error{
		func() error { _, err := store.Answer(closed.Dir, store.AnswerRecord{Choice: 1}); return err },
		func() error { _, err := store.Pickup(closed.Dir, store.PickupRecord{}); return err },
		func() error { _, err := store.Withdraw(withdrawn.Dir, store.WithdrawRecord{}); return err },
		func() error {
			_, err := store.Close(closed.Dir, store.CloseRecord{Outcome: "Pinned in **#12**."})
			return err
		},
		func() error { _, err := store.Answer(answered.Dir, store.AnswerRecord{Ack: true}); return err },
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

func TestEmptyHomeReloadsWhenACaseArrives(t *testing.T) {
	a := newApp(t)
	body := a.get(t, "/")
	if !strings.Contains(body, "no open cases.") || !strings.Contains(body, `hx-get="/fragments/inbox?empty=1"`) {
		t.Fatalf("empty / does not poll for a first case:\n%s", body)
	}
	hx := map[string]string{"HX-Request": "true"}
	if w := a.do("GET", "/fragments/inbox?empty=1", nil, hx); w.Code != http.StatusOK || !strings.Contains(w.Body.String(), `hx-get="/fragments/inbox?empty=1"`) {
		t.Errorf("still empty: %d %s", w.Code, w.Body.String())
	}
	a.open(t, openRecords[store.KindDecision])
	if w := a.do("GET", "/fragments/inbox?empty=1", nil, hx); w.Code != http.StatusNoContent || w.Header().Get("HX-Redirect") != "/" {
		t.Errorf("case arrived: %d, headers %v", w.Code, w.Header())
	}
	// A case page never polls with empty=1.
	if w := a.do("GET", "/fragments/inbox", nil, hx); w.Code != http.StatusOK {
		t.Errorf("plain fragment: %d", w.Code)
	}
}

func TestThreadShowsWithdrawReason(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindSignoff])
	if _, err := store.Withdraw(c.Dir, store.WithdrawRecord{Reason: "superseded by <the other case>"}); err != nil {
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
	if _, err := store.Note(c.Dir, store.NoteRecord{Body: "see [the note link](https://example.com/note)"}); err != nil {
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
		func() error { _, err := store.Answer(closed.Dir, store.AnswerRecord{Ack: true}); return err },
		func() error { _, err := store.Pickup(closed.Dir, store.PickupRecord{}); return err },
		func() error {
			_, err := store.Close(closed.Dir, store.CloseRecord{Outcome: "Shipped in [#12](https://example.com/pr/12)."})
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
				if w.Code != http.StatusUnprocessableEntity || !strings.Contains(w.Body.String(), `class="error"`) {
					t.Errorf("status %d, want 422 with an error:\n%s", w.Code, w.Body.String())
				}
				if files := eventFiles(t, c.Dir); len(files) != 1 {
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
