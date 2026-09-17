package notify

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

var clock = time.Date(2026, 9, 17, 9, 0, 0, 0, time.UTC)

func fixedNow() time.Time { return clock }

func openRec(kind store.Kind, urgency store.Urgency, title string) store.OpenRecord {
	rec := store.OpenRecord{Kind: kind, Urgency: urgency, Title: title, Worker: "bun-pins"}
	if kind == store.KindDecision {
		rec.Options = []string{"Pin", "Float"}
	}
	return rec
}

// poll lists the store the way serve's poller would.
func poll(t *testing.T, root string) []*store.Case {
	t.Helper()
	cases, bad, err := store.List(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(bad) > 0 {
		t.Fatalf("bad cases: %v", bad)
	}
	return cases
}

func create(t *testing.T, root string, rec store.OpenRecord) *store.Case {
	t.Helper()
	c, err := store.Create(root, rec)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// must fails the test when a store write does: must(t)(store.Note(...)).
func must(t *testing.T) func(*store.Case, error) {
	return func(_ *store.Case, err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
}

func names(p Page) []string {
	var out []string
	for _, it := range p.Items {
		out = append(out, it.Case.Title+":"+it.Event.Name)
	}
	return out
}

func TestFirstPollOnlyRecordsTheStore(t *testing.T) {
	root := t.TempDir()
	create(t, root, openRec(store.KindFYI, store.UrgencyBlocking, "Already here"))
	feed := NewFeed()
	e := New(feed, fixedNow)
	if n := e.Observe(poll(t, root)); n != 0 {
		t.Errorf("first poll added %d, want 0", n)
	}
	if n := e.Observe(poll(t, root)); n != 0 {
		t.Errorf("unchanged poll added %d, want 0", n)
	}
	if p := feed.After(0); p.Latest != 0 || len(p.Items) != 0 {
		t.Errorf("feed = %+v, want empty", p)
	}
}

func TestEmptyFirstPollStillBaselines(t *testing.T) {
	root := t.TempDir()
	feed := NewFeed()
	e := New(feed, fixedNow)
	e.Observe(nil) // the store does not exist yet
	create(t, root, openRec(store.KindFYI, store.UrgencyToday, "First"))
	if n := e.Observe(poll(t, root)); n != 1 {
		t.Errorf("added %d, want 1 for a case opened after an empty first poll", n)
	}
}

func TestWhatLandsOnTheHuman(t *testing.T) {
	root := t.TempDir()
	answered := create(t, root, openRec(store.KindDecision, store.UrgencyToday, "Answered"))
	pickedUp := create(t, root, openRec(store.KindDecision, store.UrgencyToday, "Picked up"))
	stuck := create(t, root, openRec(store.KindStuck, store.UrgencyBlocking, "Stuck"))
	parkedByHuman := create(t, root, openRec(store.KindStuck, store.UrgencyWhenever, "Human resumes"))
	openNote := create(t, root, openRec(store.KindQuestion, store.UrgencyToday, "Noted while open"))
	withdrawn := create(t, root, openRec(store.KindFYI, store.UrgencyToday, "Withdrawn"))
	must(t)(store.Answer(answered.Dir, store.AnswerRecord{Choice: 1}))
	must(t)(store.Answer(pickedUp.Dir, store.AnswerRecord{Choice: 1}))
	must(t)(store.Pickup(pickedUp.Dir, store.PickupRecord{}))
	must(t)(store.Park(stuck.Dir, store.ParkRecord{}))
	must(t)(store.Park(parkedByHuman.Dir, store.ParkRecord{}))

	feed := NewFeed()
	e := New(feed, fixedNow)
	e.Observe(poll(t, root))

	note := store.NoteRecord{Body: "One more thing?"}
	must(t)(store.Note(answered.Dir, note))
	must(t)(store.Note(pickedUp.Dir, note))
	must(t)(store.Resume(stuck.Dir, store.AuthorAgent, store.ResumeRecord{}))
	must(t)(store.Resume(parkedByHuman.Dir, store.AuthorHuman, store.ResumeRecord{}))
	must(t)(store.Note(openNote.Dir, note))
	must(t)(store.Amend(openNote.Dir, store.AmendRecord{Context: "More."}))
	must(t)(store.Withdraw(withdrawn.Dir, store.WithdrawRecord{}))
	create(t, root, openRec(store.KindFYI, store.UrgencyWhenever, "New"))

	if n := e.Observe(poll(t, root)); n != 4 {
		t.Errorf("added %d, want 4", n)
	}
	got := strings.Join(names(feed.After(0)), ", ")
	for _, want := range []string{"Answered:reopen", "Picked up:reopen", "Stuck:resume", "New:open"} {
		if !strings.Contains(got, want) {
			t.Errorf("feed %q lacks %s", got, want)
		}
	}
	for _, not := range []string{"Human resumes", "Noted while open", "Withdrawn"} {
		if strings.Contains(got, not) {
			t.Errorf("feed %q has %s", got, not)
		}
	}
}

func TestACaseOpenedAndFollowedUpBetweenPollsFiresOnce(t *testing.T) {
	root := t.TempDir()
	e := New(NewFeed(), fixedNow)
	e.Observe(poll(t, root))
	c := create(t, root, openRec(store.KindQuestion, store.UrgencyBlocking, "Quick"))
	must(t)(store.Amend(c.Dir, store.AmendRecord{Context: "Seen on the mirror."}))
	must(t)(store.Note(c.Dir, store.NoteRecord{Body: "Still there?"}))
	if n := e.Observe(poll(t, root)); n != 1 {
		t.Fatalf("added %d, want 1", n)
	}
	p := e.feed.After(0)
	if ev := p.Items[0].Event; ev.Name != EventOpen || ev.File != "0001-agent-open.json" {
		t.Errorf("event = %+v, want the open", ev)
	}
}

func TestACaseAlreadyAnsweredWhenSeenDoesNotFire(t *testing.T) {
	root := t.TempDir()
	e := New(NewFeed(), fixedNow)
	e.Observe(poll(t, root))
	c := create(t, root, openRec(store.KindFYI, store.UrgencyBlocking, "Gone by"))
	must(t)(store.Answer(c.Dir, store.AnswerRecord{Ack: true}))
	if n := e.Observe(poll(t, root)); n != 0 {
		t.Errorf("added %d, want 0 for a case the human answered before the poll", n)
	}
}

func TestItemPayload(t *testing.T) {
	root := t.TempDir()
	feed := NewFeed()
	e := New(feed, fixedNow)
	e.Observe(nil)
	c := create(t, root, openRec(store.KindDecision, store.UrgencyBlocking, "Pin bun?"))
	e.Observe(poll(t, root))

	p := feed.After(0)
	if len(p.Items) != 1 {
		t.Fatalf("items = %d", len(p.Items))
	}
	it := p.Items[0]
	if it.ID != 1 || p.Latest != 1 || it.Sink != SinkBrowser {
		t.Errorf("id %d, latest %d, sink %q", it.ID, p.Latest, it.Sink)
	}
	if it.Title != "Pin bun?" || it.Body != "blocking decision · from bun-pins" {
		t.Errorf("title %q, body %q", it.Title, it.Body)
	}
	if it.URL != "/cases/"+c.ID || it.Tag != c.ID+"/0001-agent-open.json" {
		t.Errorf("url %q, tag %q", it.URL, it.Tag)
	}
	if !it.QueuedAt.Equal(clock) {
		t.Errorf("queued at %v, want the engine clock", it.QueuedAt)
	}
	data, err := json.Marshal(it)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{`"event":{"name":"open","seq":1,"author":"agent","event":"open","file":"0001-agent-open.json"`, `"case":{"id":"` + c.ID + `"`, `"urgency":"blocking"`} {
		if !strings.Contains(string(data), want) {
			t.Errorf("payload lacks %s:\n%s", want, data)
		}
	}
}

func TestFeedKeepsTheLastItemsAndCountsOn(t *testing.T) {
	feed := NewFeed()
	c := &store.Case{ID: "x"}
	for range FeedSize + 7 {
		feed.add(Item{Case: c})
	}
	p := feed.After(0)
	if p.Latest != FeedSize+7 || len(p.Items) != FeedSize || p.Items[0].ID != 8 {
		t.Errorf("latest %d, %d items from id %d", p.Latest, len(p.Items), p.Items[0].ID)
	}
	if p := feed.After(FeedSize + 5); len(p.Items) != 2 || p.Items[0].ID != FeedSize+6 {
		t.Errorf("after %d: %d items", FeedSize+5, len(p.Items))
	}
	if p := feed.After(1000); p.Items == nil || len(p.Items) != 0 {
		t.Errorf("after a later id: %+v, want an empty list", p.Items)
	}
	if other := NewFeed(); other.boot == "" || other.boot == feed.boot {
		t.Errorf("boot ids %q and %q, want two different ids", feed.boot, other.boot)
	}
}
