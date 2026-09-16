package web

import (
	"net/http"
	"net/url"
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
	if !strings.Contains(list, "<title>(1) Inbox · cases</title>") || strings.Contains(list, "<html") || strings.Contains(list, "selected") {
		t.Errorf("fragment is not a bare list with a title and no selection:\n%s", list)
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
		!strings.Contains(page, `urgency-today selected" href="/cases/`+today.ID+`" aria-current="page"`) || strings.Count(page, `aria-current`) != 1 {
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
			formParts: []string{`name="choice" value="1"`, `name="choice" value="2"`, `value="other"`, "Other, see note", "Pin", "Float"},
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
			formParts: []string{`name="signoff" value="accept"`, `name="signoff" value="changes"`, "Comment"},
			form:      url.Values{"signoff": {"changes"}, "note": {"rename the flag"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindStuck,
			formParts: []string{`name="stuck" value="text"`, `name="text"`, `name="stuck" value="park"`, `name="stuck" value="drop"`},
			form:      url.Values{"stuck": {"text"}, "text": {"use the mirror"}},
			wantState: store.StateAnswered, wantFile: "0002-human-answer.json",
		},
		{
			kind:      store.KindFYI,
			formParts: []string{`name="ack"`, "Acknowledge"},
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

			w := a.do("POST", "/cases/"+c.ID+"/answer", tt.form, map[string]string{"Origin": "http://" + testAddr})
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

	if w := a.do("POST", "/cases/"+c.ID+"/answer", url.Values{"stuck": {"park"}, "note": {"after release"}}, origin); w.Code != http.StatusSeeOther {
		t.Fatalf("park: %d %s", w.Code, w.Body.String())
	}
	if files := eventFiles(t, c.Dir); !slices.Equal(files, []string{"0001-agent-open.json", "0002-human-park.json"}) {
		t.Errorf("files = %v", files)
	}
	page := a.get(t, "/cases/"+c.ID)
	if !strings.Contains(page, `action="/cases/`+c.ID+`/resume"`) || !strings.Contains(page, "note: after release") {
		t.Errorf("parked page:\n%s", page)
	}
	if w := a.do("POST", "/cases/"+c.ID+"/resume", url.Values{}, origin); w.Code != http.StatusSeeOther {
		t.Fatalf("resume: %d", w.Code)
	}
	loaded, err := store.Load(c.Dir)
	if err != nil || loaded.State != store.StateOpen || loaded.Events[2].Author != store.AuthorHuman {
		t.Errorf("after resume: %+v %v", loaded, err)
	}
	// Resuming an open case is refused and writes nothing.
	if w := a.do("POST", "/cases/"+c.ID+"/resume", url.Values{}, origin); w.Code != http.StatusUnprocessableEntity {
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
		{"park", "/cases/" + blocked.ID + "/answer", url.Values{"stuck": {"park"}}, "/cases/" + decision.ID},
		{"answer", "/cases/" + decision.ID + "/answer", url.Values{"choice": {"1"}}, "/cases/" + parked.ID},
		{"resume", "/cases/" + parked.ID + "/resume", url.Values{}, "/cases/" + zed.ID},
		{"answer the last", "/cases/" + zed.ID + "/answer", url.Values{"ack": {"1"}}, "/"},
		{"resume the first", "/cases/" + blocked.ID + "/resume", url.Values{}, "/cases/" + parked.ID},
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
		{store.KindFYI, url.Values{}, "no fyi response", ""},
	}
	for _, tt := range tests {
		t.Run(string(tt.kind)+"/"+tt.wantErr, func(t *testing.T) {
			a := newApp(t)
			c := a.open(t, openRecords[tt.kind])
			w := a.do("POST", "/cases/"+c.ID+"/answer", tt.form, nil)
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
	w := a.do("POST", "/cases/"+c.ID+"/answer", url.Values{"ack": {"1"}}, nil)
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

func TestThreadFragment(t *testing.T) {
	a := newApp(t)
	c := a.open(t, openRecords[store.KindDecision])
	if _, err := store.Answer(c.Dir, store.AnswerRecord{Choice: 1, Note: "ship it"}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Pickup(c.Dir, store.PickupRecord{By: "manager"}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Note(c.Dir, store.NoteRecord{Body: "Which **patch**?"}); err != nil {
		t.Fatal(err)
	}

	page := a.get(t, "/cases/"+c.ID)
	if !strings.Contains(page, `hx-get="/cases/`+c.ID+`/thread?state=open"`) {
		t.Error("case page does not poll its thread")
	}
	frag := a.get(t, "/cases/"+c.ID+"/thread?state=open")
	for _, want := range []string{"chose 1. Pin", "note: ship it", "by manager", "Which <strong>patch</strong>?", "human", "agent"} {
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
	if !strings.Contains(body, "No open cases.") || !strings.Contains(body, `hx-get="/fragments/inbox?empty=1"`) {
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
