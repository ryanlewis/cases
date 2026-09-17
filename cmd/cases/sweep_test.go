package main

import (
	"context"
	"errors"
	"maps"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// openAt opens an fyi case in the store at storePath with its open event
// dated at.
func openAt(t *testing.T, storePath, title string, at time.Time, rec store.OpenRecord) *store.Case {
	t.Helper()
	rec.Kind, rec.Urgency, rec.Title, rec.OpenedAt = store.KindFYI, store.UrgencyWhenever, title, at
	c, err := openStore(t, storePath).Create(t.Context(), rec)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// storeRevisions maps each case id in the store to its revision, so a test
// can check nothing was written.
func storeRevisions(t *testing.T, storePath string) map[string]int {
	t.Helper()
	cases, bad, err := openStore(t, storePath).List(t.Context())
	if err != nil || len(bad) > 0 {
		t.Fatalf("list: %v, bad %v", err, bad)
	}
	revisions := map[string]int{}
	for _, c := range cases {
		revisions[c.ID] = c.Revision()
	}
	return revisions
}

func TestSweepDryRunWritesNothing(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	before := storeRevisions(t, storePath)

	r := runCases(t, "", "--store", storePath, "sweep")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if r.stdout != id+" would be withdrawn: Pin bun?\n" {
		t.Errorf("stdout = %q", r.stdout)
	}
	if !strings.Contains(r.stderr, "1 cases would be withdrawn. Pass --yes") {
		t.Errorf("stderr = %q", r.stderr)
	}
	if after := storeRevisions(t, storePath); !maps.Equal(after, before) {
		t.Errorf("dry run wrote to the store: %v, then %v", before, after)
	}
}

func TestSweepWithdrawsOpenCasesAndLeavesTheRest(t *testing.T) {
	storePath := newStore(t)
	open := openDecision(t, storePath)
	answered := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", answered, "--option", "1")
	parked := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "today", "--title", "Stuck"))
	mustRun(t, "--store", storePath, "answer", parked, "--park")
	closed := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", closed, "--option", "1")
	mustRun(t, "--store", storePath, "pickup", closed)
	if r := runCases(t, "done", "--store", storePath, "close", closed, "--outcome-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}

	out := mustRun(t, "--store", storePath, "sweep", "--yes", "--reason", "clearing the inbox")
	for _, want := range []string{open + " withdrawn\n", answered + " answered, left\n", parked + " parked, left\n"} {
		if !strings.Contains(out, want) {
			t.Errorf("sweep --yes lacks %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, closed) {
		t.Errorf("sweep --yes mentions the closed case:\n%s", out)
	}
	c := loadCase(t, storePath, open)
	if c.State != store.StateWithdrawn {
		t.Errorf("open case state = %s", c.State)
	}
	if last := c.Events[len(c.Events)-1]; !strings.Contains(string(last.Data), `"reason": "clearing the inbox"`) {
		t.Errorf("withdraw event = %s", last.Data)
	}
	for id, want := range map[string]store.State{answered: store.StateAnswered, parked: store.StateParked, closed: store.StateClosed} {
		if got := loadCase(t, storePath, id).State; got != want {
			t.Errorf("%s state = %s, want %s", id, got, want)
		}
	}

	// The default reason, and a second sweep has nothing to withdraw.
	other := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "sweep", "-y")
	c = loadCase(t, storePath, other)
	if last := c.Events[len(c.Events)-1]; !strings.Contains(string(last.Data), `"reason": "swept"`) {
		t.Errorf("withdraw event = %s", last.Data)
	}
	if out := mustRun(t, "--store", storePath, "sweep", "--yes"); strings.Contains(out, "withdrawn") {
		t.Errorf("second sweep = %q", out)
	}
}

func TestSweepFilters(t *testing.T) {
	storePath := newStore(t)
	now := time.Now().UTC()
	old := openAt(t, storePath, "Old", now.Add(-48*time.Hour), store.OpenRecord{Labels: []string{"round-1"}, Worker: "w1"})
	recent := openAt(t, storePath, "Recent", now.Add(-time.Hour), store.OpenRecord{Labels: []string{"round-1"}, Worker: "w1"})
	other := openAt(t, storePath, "Other", now.Add(-48*time.Hour), store.OpenRecord{Labels: []string{"round-2"}, Worker: "w2"})

	for _, tt := range []struct {
		args []string
		want []string
	}{
		{nil, []string{old.ID, recent.ID, other.ID}},
		{[]string{"--label", "round-1"}, []string{old.ID, recent.ID}},
		{[]string{"--worker", "w2"}, []string{other.ID}},
		{[]string{"--older-than", "24h"}, []string{old.ID, other.ID}},
		{[]string{"--older-than", "24h", "--label", "round-1"}, []string{old.ID}},
		{[]string{"--label", "none"}, nil},
	} {
		out := mustRun(t, append([]string{"--store", storePath, "sweep"}, tt.args...)...)
		for _, id := range []string{old.ID, recent.ID, other.ID} {
			if strings.Contains(out, id) != slices.Contains(tt.want, id) {
				t.Errorf("sweep %v:\n%s", tt.args, out)
				break
			}
		}
	}

	mustRun(t, "--store", storePath, "sweep", "--older-than", "24h", "--label", "round-1", "--yes")
	for id, want := range map[string]store.State{old.ID: store.StateWithdrawn, recent.ID: store.StateOpen, other.ID: store.StateOpen} {
		if got := loadCase(t, storePath, id).State; got != want {
			t.Errorf("%s state = %s, want %s", id, got, want)
		}
	}
}

// answerFirst is the store with the human answering one case just before
// sweep withdraws it, as can happen between sweep listing the cases and
// withdrawing them.
type answerFirst struct {
	store.Store
	id string
}

func (a answerFirst) Withdraw(ctx context.Context, id string, rec store.WithdrawRecord, pre ...store.Precondition) (*store.Case, error) {
	if id == a.id {
		if _, err := a.Answer(ctx, id, store.AnswerRecord{Choice: 1}); err != nil {
			return nil, err
		}
	}
	return a.Store.Withdraw(ctx, id, rec, pre...)
}

func TestSweepContinuesPastARefusal(t *testing.T) {
	storePath := newStore(t)
	stuck := openDecision(t, storePath)
	fine := openDecision(t, storePath)

	r := runCasesWith(t, answerFirst{openStore(t, storePath), stuck}, "", "--store", storePath, "sweep", "--yes")
	if r.err == nil || !strings.Contains(r.err.Error(), "1 cases were not withdrawn") || !strings.Contains(r.err.Error(), stuck+": cannot withdraw a case that is answered") {
		t.Errorf("err = %v", r.err)
	}
	if !strings.Contains(r.stdout, fine+" withdrawn\n") {
		t.Errorf("stdout = %q", r.stdout)
	}
	if got := loadCase(t, storePath, stuck).State; got != store.StateAnswered {
		t.Errorf("refused case state = %s", got)
	}
	if got := loadCase(t, storePath, fine).State; got != store.StateWithdrawn {
		t.Errorf("other case state = %s", got)
	}
}

func TestSweepByKind(t *testing.T) {
	storePath := newStore(t)
	notice := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "Notice"))
	decision := openDecision(t, storePath)

	mustRun(t, "--store", storePath, "sweep", "--kind", "fyi", "--yes")
	for id, want := range map[string]store.State{notice: store.StateWithdrawn, decision: store.StateOpen} {
		if got := loadCase(t, storePath, id).State; got != want {
			t.Errorf("%s state = %s, want %s", id, got, want)
		}
	}
}

// failWithdraw is the store with Withdraw refused for one case.
type failWithdraw struct {
	store.Store
	id string
}

func (f failWithdraw) Withdraw(ctx context.Context, id string, rec store.WithdrawRecord, pre ...store.Precondition) (*store.Case, error) {
	if id == f.id {
		return nil, errors.New("store unavailable")
	}
	return f.Store.Withdraw(ctx, id, rec, pre...)
}

func TestSweepWithdrawsThroughTheStore(t *testing.T) {
	storePath := newStore(t)
	failing := openDecision(t, storePath)
	fine := openDecision(t, storePath)

	r := runCasesWith(t, failWithdraw{openStore(t, storePath), failing}, "", "--store", storePath, "sweep", "--yes")
	if r.err == nil || !strings.Contains(r.err.Error(), failing+": store unavailable") {
		t.Errorf("err = %v", r.err)
	}
	if !strings.Contains(r.stdout, fine+" withdrawn\n") {
		t.Errorf("stdout = %q", r.stdout)
	}
	if got := loadCase(t, storePath, failing).State; got != store.StateOpen {
		t.Errorf("failed case state = %s", got)
	}
}
