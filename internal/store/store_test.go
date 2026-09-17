package store

import (
	"context"
	"errors"
	"io/fs"
	"path/filepath"
	"slices"
	"testing"
)

// TestDirWritesAndReadsByID runs a case through every Dir method, by id.
func TestDirWritesAndReadsByID(t *testing.T) {
	ctx := context.Background()
	d := NewDir(filepath.Join(t.TempDir(), "cases"))

	stuck, err := d.Create(ctx, OpenRecord{Kind: KindStuck, Urgency: UrgencyToday, Title: "Stuck on CI"})
	if err != nil {
		t.Fatal(err)
	}
	decision, err := d.Create(ctx, OpenRecord{Kind: KindDecision, Urgency: UrgencyToday, Title: "Pin bun?", Options: []string{"Pin", "Float"}})
	if err != nil {
		t.Fatal(err)
	}
	id := decision.ID

	steps := []struct {
		name string
		want State
		do   func() (*Case, error)
	}{
		{"amend", StateOpen, func() (*Case, error) { return d.Amend(ctx, id, AmendRecord{Labels: []string{"deps"}}) }},
		{"answer", StateAnswered, func() (*Case, error) { return d.Answer(ctx, id, AnswerRecord{Choice: 1}, AtRevision(2)) }},
		{"pickup", StatePickedUp, func() (*Case, error) { return d.Pickup(ctx, id, PickupRecord{By: "test"}) }},
		{"note", StateOpen, func() (*Case, error) { return d.Note(ctx, id, NoteRecord{Body: "Which patch?"}) }},
		{"answer again", StateAnswered, func() (*Case, error) { return d.Answer(ctx, id, AnswerRecord{Choice: 2}) }},
		{"pickup again", StatePickedUp, func() (*Case, error) { return d.Pickup(ctx, id, PickupRecord{}) }},
		{"close", StateClosed, func() (*Case, error) { return d.Close(ctx, id, CloseRecord{Outcome: "Floated."}) }},
		{"park", StateParked, func() (*Case, error) { return d.Park(ctx, stuck.ID, ParkRecord{}) }},
		{"resume", StateOpen, func() (*Case, error) { return d.Resume(ctx, stuck.ID, AuthorHuman, ResumeRecord{}) }},
		{"withdraw", StateWithdrawn, func() (*Case, error) { return d.Withdraw(ctx, stuck.ID, WithdrawRecord{}) }},
	}
	for _, s := range steps {
		c, err := s.do()
		if err != nil {
			t.Fatalf("%s: %v", s.name, err)
		}
		if c.State != s.want {
			t.Fatalf("%s: state %s, want %s", s.name, c.State, s.want)
		}
	}

	got, err := d.Get(ctx, id)
	if err != nil || got.State != StateClosed || got.Revision() != 8 {
		t.Fatalf("Get = %+v, %v; want closed at revision 8", got, err)
	}
	ids, err := d.IDs(ctx)
	if want := slices.Sorted(slices.Values([]string{decision.ID, stuck.ID})); err != nil || !slices.Equal(ids, want) {
		t.Errorf("IDs = %v, %v", ids, err)
	}
	cases, bad, err := d.List(ctx)
	if err != nil || len(cases) != 2 || len(bad) != 0 {
		t.Errorf("List = %d cases, %v, %v", len(cases), bad, err)
	}
	if cases, _, err := d.NewPoller().Poll(); err != nil || len(cases) != 2 {
		t.Errorf("Poll = %d cases, %v", len(cases), err)
	}
}

// TestDirKeepsTheDirectoryErrors checks the errors callers branch on come
// through Dir unchanged.
func TestDirKeepsTheDirectoryErrors(t *testing.T) {
	ctx := context.Background()
	d := NewDir(t.TempDir())
	c, err := d.Create(ctx, OpenRecord{Kind: KindFYI, Urgency: UrgencyWhenever, Title: "Heads up"})
	if err != nil {
		t.Fatal(err)
	}

	if _, err := d.Get(ctx, "2026-01-01T00-00-00Z-missing"); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Get missing: %v, want fs.ErrNotExist", err)
	}
	if _, err := d.Get(ctx, "../elsewhere"); err == nil || err.Error() != `invalid case id "../elsewhere"` {
		t.Errorf("Get invalid id: %v", err)
	}
	if _, err := d.Pickup(ctx, "../elsewhere", PickupRecord{}); err == nil || err.Error() != `invalid case id "../elsewhere"` {
		t.Errorf("Pickup invalid id: %v", err)
	}
	if _, err := d.Answer(ctx, c.ID, AnswerRecord{Ack: true}, AtRevision(0)); !errors.Is(err, ErrStale) {
		t.Errorf("stale answer: %v, want ErrStale", err)
	}
	var te *TransitionError
	if _, err := d.Close(ctx, c.ID, CloseRecord{Outcome: "done"}); !errors.As(err, &te) {
		t.Errorf("close an open case: %v, want a TransitionError", err)
	}
	if _, _, err := NewDir(filepath.Join(t.TempDir(), "absent")).List(ctx); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("List absent store: %v, want fs.ErrNotExist", err)
	}
}
