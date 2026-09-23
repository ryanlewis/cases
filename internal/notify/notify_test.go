package notify

import (
	"encoding/json"
	"errors"
	"io/fs"
	"path/filepath"
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

// newDB returns a store in a file of its own that does not exist yet.
func newDB(t *testing.T) *store.DB {
	t.Helper()
	db := store.NewDB(filepath.Join(t.TempDir(), "cases.db"))
	t.Cleanup(func() { _ = db.Disconnect() })
	return db
}

// poll lists the store the way serve's poller would: a store nothing has
// been written to yet has no cases.
func poll(t *testing.T, db *store.DB) []*store.Case {
	t.Helper()
	cases, bad, err := db.List(t.Context())
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		t.Fatal(err)
	}
	if len(bad) > 0 {
		t.Fatalf("bad cases: %v", bad)
	}
	return cases
}

func create(t *testing.T, db *store.DB, rec store.OpenRecord) *store.Case {
	t.Helper()
	c, err := db.Create(t.Context(), rec)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// must fails the test when a store write does: must(t)(db.Note(...)).
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
	db := newDB(t)
	create(t, db, openRec(store.KindFYI, store.UrgencyBlocking, "Already here"))
	feed := NewFeed()
	e := New(feed, fixedNow)
	if n := e.Observe(poll(t, db)); n != 0 {
		t.Errorf("first poll added %d, want 0", n)
	}
	if n := e.Observe(poll(t, db)); n != 0 {
		t.Errorf("unchanged poll added %d, want 0", n)
	}
	if p := feed.After(0); p.Latest != 0 || len(p.Items) != 0 {
		t.Errorf("feed = %+v, want empty", p)
	}
}

func TestEmptyFirstPollStillBaselines(t *testing.T) {
	db := newDB(t)
	feed := NewFeed()
	e := New(feed, fixedNow)
	e.Observe(nil) // the store does not exist yet
	create(t, db, openRec(store.KindFYI, store.UrgencyToday, "First"))
	if n := e.Observe(poll(t, db)); n != 1 {
		t.Errorf("added %d, want 1 for a case opened after an empty first poll", n)
	}
}

func TestWhatLandsOnTheHuman(t *testing.T) {
	db := newDB(t)
	answered := create(t, db, openRec(store.KindDecision, store.UrgencyToday, "Answered"))
	pickedUp := create(t, db, openRec(store.KindDecision, store.UrgencyToday, "Picked up"))
	stuck := create(t, db, openRec(store.KindStuck, store.UrgencyBlocking, "Stuck"))
	parkedByHuman := create(t, db, openRec(store.KindStuck, store.UrgencyWhenever, "Human resumes"))
	openNote := create(t, db, openRec(store.KindQuestion, store.UrgencyToday, "Noted while open"))
	withdrawn := create(t, db, openRec(store.KindFYI, store.UrgencyToday, "Withdrawn"))
	must(t)(db.Answer(t.Context(), answered.ID, store.AnswerRecord{Choice: 1}))
	must(t)(db.Answer(t.Context(), pickedUp.ID, store.AnswerRecord{Choice: 1}))
	must(t)(db.Pickup(t.Context(), pickedUp.ID, store.PickupRecord{}))
	must(t)(db.Park(t.Context(), stuck.ID, store.ParkRecord{}))
	must(t)(db.Park(t.Context(), parkedByHuman.ID, store.ParkRecord{}))

	feed := NewFeed()
	e := New(feed, fixedNow)
	e.Observe(poll(t, db))

	note := store.NoteRecord{Body: "One more thing?"}
	must(t)(db.Note(t.Context(), answered.ID, note))
	must(t)(db.Note(t.Context(), pickedUp.ID, note))
	must(t)(db.Resume(t.Context(), stuck.ID, store.AuthorAgent, store.ResumeRecord{}))
	must(t)(db.Resume(t.Context(), parkedByHuman.ID, store.AuthorHuman, store.ResumeRecord{}))
	must(t)(db.Note(t.Context(), openNote.ID, note))
	must(t)(db.Amend(t.Context(), openNote.ID, store.AmendRecord{Context: "More."}))
	must(t)(db.Withdraw(t.Context(), withdrawn.ID, store.WithdrawRecord{}))
	create(t, db, openRec(store.KindFYI, store.UrgencyWhenever, "New"))

	if n := e.Observe(poll(t, db)); n != 4 {
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

	// The agent's reply to the human's resume lands the case on the human;
	// its next note, or an amend, does not fire again.
	must(t)(db.Amend(t.Context(), parkedByHuman.ID, store.AmendRecord{Context: "Looking."}))
	must(t)(db.Note(t.Context(), parkedByHuman.ID, note))
	if n := e.Observe(poll(t, db)); n != 1 {
		t.Errorf("reply: added %d, want 1", n)
	}
	last := feed.After(4).Items
	if len(last) != 1 || last[0].Event.Name != EventReply || last[0].Event.File != "0005-agent-note.json" || last[0].Body != "whenever stuck · the agent replied" {
		t.Errorf("reply items = %+v", last)
	}
	must(t)(db.Note(t.Context(), parkedByHuman.ID, note))
	if n := e.Observe(poll(t, db)); n != 0 {
		t.Errorf("second note after the reply: added %d, want 0", n)
	}
}

func TestACaseOpenedAndFollowedUpBetweenPollsFiresOnce(t *testing.T) {
	db := newDB(t)
	e := New(NewFeed(), fixedNow)
	e.Observe(poll(t, db))
	c := create(t, db, openRec(store.KindQuestion, store.UrgencyBlocking, "Quick"))
	must(t)(db.Amend(t.Context(), c.ID, store.AmendRecord{Context: "Seen on the mirror."}))
	must(t)(db.Note(t.Context(), c.ID, store.NoteRecord{Body: "Still there?"}))
	if n := e.Observe(poll(t, db)); n != 1 {
		t.Fatalf("added %d, want 1", n)
	}
	p := e.feed.After(0)
	if ev := p.Items[0].Event; ev.Name != EventOpen || ev.File != "0001-agent-open.json" {
		t.Errorf("event = %+v, want the open", ev)
	}
}

func TestACaseAlreadyAnsweredWhenSeenDoesNotFire(t *testing.T) {
	db := newDB(t)
	e := New(NewFeed(), fixedNow)
	e.Observe(poll(t, db))
	c := create(t, db, openRec(store.KindFYI, store.UrgencyBlocking, "Gone by"))
	must(t)(db.Answer(t.Context(), c.ID, store.AnswerRecord{Ack: true}))
	if n := e.Observe(poll(t, db)); n != 0 {
		t.Errorf("added %d, want 0 for a case the human answered before the poll", n)
	}
}

// The engine forgets a case once it has left the store, so an always-on
// serve that sees cases opened, closed and pruned keeps only what the live
// store holds.
func TestSeenForgetsCasesThatLeaveTheStore(t *testing.T) {
	db := newDB(t)
	e := New(NewFeed(), fixedNow)
	keep := create(t, db, openRec(store.KindFYI, store.UrgencyToday, "Stays"))
	e.Observe(poll(t, db))
	for i := range 20 {
		c := create(t, db, openRec(store.KindFYI, store.UrgencyBlocking, "Passing"))
		if n := e.Observe(poll(t, db)); n != 1 {
			t.Fatalf("round %d: opening added %d, want 1", i, n)
		}
		must(t)(db.Answer(t.Context(), c.ID, store.AnswerRecord{Ack: true}))
		must(t)(db.Pickup(t.Context(), c.ID, store.PickupRecord{}))
		must(t)(db.Close(t.Context(), c.ID, store.CloseRecord{Outcome: "Done."}))
		e.Observe(poll(t, db))
		if err := db.Archive(t.Context(), c.ID); err != nil {
			t.Fatal(err)
		}
		if n := e.Observe(poll(t, db)); n != 0 {
			t.Fatalf("round %d: pruning added %d, want 0", i, n)
		}
		if _, ok := e.seen[keep.ID]; len(e.seen) != 1 || !ok {
			t.Fatalf("round %d: seen = %v, want only %s", i, e.seen, keep.ID)
		}
	}
	// A case still in the store is not new again.
	if n := e.Observe(poll(t, db)); n != 0 {
		t.Errorf("unchanged poll added %d, want 0", n)
	}
}

func TestItemPayload(t *testing.T) {
	db := newDB(t)
	feed := NewFeed()
	e := New(feed, fixedNow)
	e.Observe(nil)
	c := create(t, db, openRec(store.KindDecision, store.UrgencyBlocking, "Pin bun?"))
	e.Observe(poll(t, db))

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
