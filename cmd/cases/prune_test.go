package main

import (
	"errors"
	"io/fs"
	"maps"
	"os"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
	"github.com/ryanlewis/cases/internal/store/storetest"
)

// pruneStore holds one case in each state prune looks at, all last touched at
// the time given, plus a withdrawn case from an hour ago.
type pruneStore struct {
	path                            string
	closed, withdrawn, open, recent string
}

func newPruneStore(t *testing.T) pruneStore {
	t.Helper()
	path := newStore(t)
	db := openStore(t, path)
	ctx := t.Context()
	old := time.Now().UTC().Add(-60 * 24 * time.Hour)
	s := pruneStore{path: path}

	c := openAt(t, path, "Closed", old, store.OpenRecord{})
	s.closed = c.ID
	if _, err := db.Answer(ctx, c.ID, store.AnswerRecord{Ack: true, AnsweredAt: old}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Pickup(ctx, c.ID, store.PickupRecord{PickedUpAt: old}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Close(ctx, c.ID, store.CloseRecord{Outcome: "done", ClosedAt: old}); err != nil {
		t.Fatal(err)
	}

	c = openAt(t, path, "Withdrawn", old, store.OpenRecord{})
	s.withdrawn = c.ID
	if _, err := db.Withdraw(ctx, c.ID, store.WithdrawRecord{WithdrawnAt: old}); err != nil {
		t.Fatal(err)
	}

	s.open = openAt(t, path, "Open", old, store.OpenRecord{}).ID

	c = openAt(t, path, "Recent", old, store.OpenRecord{})
	s.recent = c.ID
	if _, err := db.Withdraw(ctx, c.ID, store.WithdrawRecord{WithdrawnAt: time.Now().UTC().Add(-time.Hour)}); err != nil {
		t.Fatal(err)
	}
	return s
}

// inStore reports whether the store at path has the case id.
func inStore(t *testing.T, path, id string) bool {
	t.Helper()
	ids, err := openStore(t, path).IDs(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return slices.Contains(ids, id)
}

func TestPruneDryRunWritesNothing(t *testing.T) {
	s := newPruneStore(t)
	before := storeRevisions(t, s.path)

	r := runCases(t, "", "--store", s.path, "prune")
	if r.err != nil {
		t.Fatal(r.err)
	}
	want := s.closed + " closed, would be archived\n" + s.withdrawn + " withdrawn, would be archived\n"
	if r.stdout != want {
		t.Errorf("stdout = %q, want %q", r.stdout, want)
	}
	if !strings.Contains(r.stderr, "2 cases would be moved to the archive in "+s.path+". Pass --yes") {
		t.Errorf("stderr = %q", r.stderr)
	}
	if after := storeRevisions(t, s.path); !maps.Equal(after, before) {
		t.Errorf("dry run wrote to the store: %v, then %v", before, after)
	}
	if archived := storetest.Archived(t, s.path); len(archived) != 0 {
		t.Errorf("dry run archived %v", archived)
	}
}

func TestPruneArchives(t *testing.T) {
	s := newPruneStore(t)
	before := storeRevisions(t, s.path)
	out := mustRun(t, "--store", s.path, "prune", "--yes")
	if want := s.closed + " archived\n" + s.withdrawn + " archived\n"; out != want {
		t.Errorf("stdout = %q, want %q", out, want)
	}
	// Each case moved whole, every event with it.
	want := map[string]int{s.closed: before[s.closed], s.withdrawn: before[s.withdrawn]}
	if archived := storetest.Archived(t, s.path); !maps.Equal(archived, want) {
		t.Errorf("archive = %v, want %v", archived, want)
	}
	for _, id := range []string{s.closed, s.withdrawn} {
		if inStore(t, s.path, id) {
			t.Errorf("%s is still in the store", id)
		}
		if r := runCases(t, "", "--store", s.path, "show", id); r.err == nil || r.err.Error() != `no case "`+id+`"` {
			t.Errorf("show %s: %v", id, r.err)
		}
	}
	for _, id := range []string{s.open, s.recent} {
		if !inStore(t, s.path, id) {
			t.Errorf("%s was pruned", id)
		}
	}

	list := mustRun(t, "--store", s.path, "list", "--all")
	if strings.Contains(list, s.closed) || strings.Contains(list, s.withdrawn) || !strings.Contains(list, s.open) || !strings.Contains(list, s.recent) {
		t.Errorf("list after prune:\n%s", list)
	}

	// --age 0 takes the recent one too, and --state narrows.
	if out := mustRun(t, "--store", s.path, "prune", "--age", "0", "--state", "closed"); out != "" {
		t.Errorf("prune --state closed with nothing closed = %q", out)
	}
	if out := mustRun(t, "--store", s.path, "prune", "--age", "0", "--state", "withdrawn", "--yes"); out != s.recent+" archived\n" {
		t.Errorf("prune --age 0 = %q", out)
	}
}

// A case whose id the archive already has, as when one was put back in the
// store by hand after an earlier prune, is left where it is.
func TestPruneRefusesAnIDInTheArchive(t *testing.T) {
	s := newPruneStore(t)
	db := openStore(t, s.path)
	closed := loadCase(t, s.path, s.closed)
	if err := db.Archive(t.Context(), s.closed); err != nil {
		t.Fatal(err)
	}
	for _, ev := range closed.Events {
		storetest.InsertEvent(t, s.path, s.closed, ev.Seq, string(ev.Author), string(ev.Type), string(ev.Data))
	}

	r := runCases(t, "", "--store", s.path, "prune", "--yes")
	if r.err == nil || !strings.Contains(r.err.Error(), "1 cases were not pruned") || !strings.Contains(r.err.Error(), s.closed+": already in the archive") {
		t.Errorf("err = %v", r.err)
	}
	if r.stdout != s.withdrawn+" archived\n" {
		t.Errorf("stdout = %q", r.stdout)
	}
	if got := loadCase(t, s.path, s.closed); got.State != store.StateClosed || got.Revision() != closed.Revision() {
		t.Errorf("refused case = %s at revision %d", got.State, got.Revision())
	}
}

func TestPruneDelete(t *testing.T) {
	s := newPruneStore(t)
	if out := mustRun(t, "--store", s.path, "prune", "--delete"); !strings.Contains(out, s.closed+" closed, would be deleted\n") {
		t.Errorf("dry run = %q", out)
	}
	if out := mustRun(t, "--store", s.path, "prune", "--delete", "--yes"); out != s.closed+" deleted\n"+s.withdrawn+" deleted\n" {
		t.Errorf("stdout = %q", out)
	}
	for _, id := range []string{s.closed, s.withdrawn} {
		if inStore(t, s.path, id) {
			t.Errorf("%s is still in the store", id)
		}
	}
	if archived := storetest.Archived(t, s.path); len(archived) != 0 {
		t.Errorf("--delete archived %v", archived)
	}
	if !inStore(t, s.path, s.open) {
		t.Error("deleted the open case")
	}
}

func TestPruneSkipsABrokenCase(t *testing.T) {
	s := newPruneStore(t)
	const broken = "2020-01-01T00-00-00Z-broken"
	storetest.InsertEvent(t, s.path, broken, 1, "agent", "open", "{")

	r := runCases(t, "", "--store", s.path, "prune", "--yes", "--age", "0")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if !strings.Contains(r.stderr, "warning: skipped "+broken+": no valid open event") {
		t.Errorf("stderr = %q", r.stderr)
	}
	if strings.Contains(r.stdout, "broken") || !inStore(t, s.path, broken) {
		t.Errorf("pruned the broken case: %q", r.stdout)
	}
}

func TestPruneRefusesOtherStates(t *testing.T) {
	storePath := newStore(t)
	for _, args := range [][]string{{"--state", "open"}, {"--state", "closed,answered"}, {"--state", "nope"}, {"--age", "-1h"}} {
		if r := runCases(t, "", append([]string{"--store", storePath, "prune"}, args...)...); r.err == nil {
			t.Errorf("prune %v was accepted", args)
		}
	}
}

func TestPruneOnMissingStore(t *testing.T) {
	storePath := newStore(t)
	if out := mustRun(t, "--store", storePath, "prune", "--yes"); out != "" {
		t.Errorf("stdout = %q", out)
	}
	if _, err := os.Stat(storePath); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("prune made the store: %v", err)
	}
}

func TestConfigSetsPruneAge(t *testing.T) {
	if cli := parseCases(t, "prune"); cli.Prune.Age != 720*time.Hour {
		t.Errorf("age = %s without the file, want 720h", cli.Prune.Age)
	}
	writeConfig(t, "prune-age = \"1h\"\n")
	if cli := parseCases(t, "prune"); cli.Prune.Age != time.Hour {
		t.Errorf("age = %s, want the file's 1h", cli.Prune.Age)
	}
	if cli := parseCases(t, "prune", "--age", "2h"); cli.Prune.Age != 2*time.Hour {
		t.Errorf("age = %s, want the flag's 2h", cli.Prune.Age)
	}

	// The file's age decides which cases a bare prune takes.
	s := newPruneStore(t)
	writeConfig(t, "prune-age = \"30m\"\n")
	if out := mustRun(t, "--store", s.path, "prune"); !strings.Contains(out, s.recent) {
		t.Errorf("prune with prune-age 30m = %q, want the case withdrawn an hour ago", out)
	}
	if out := mustRun(t, "--store", s.path, "prune", "--age", "720h"); strings.Contains(out, s.recent) {
		t.Errorf("prune --age 720h = %q, want the flag to beat the file", out)
	}

	// The key seeds prune only; another command's flags are untouched.
	writeConfig(t, "prune-age = \"not a duration\"\n")
	mustRun(t, "--store", s.path, "list")
	if r := runCases(t, "", "--store", s.path, "prune"); r.err == nil {
		t.Error("prune accepted a bad prune-age")
	}
}

func TestPruneAgeZeroTakesAFutureCase(t *testing.T) {
	storePath := newStore(t)
	// A withdraw that records a time ahead of this machine's clock.
	c := openAt(t, storePath, "Ahead", time.Now().UTC(), store.OpenRecord{})
	if _, err := openStore(t, storePath).Withdraw(t.Context(), c.ID, store.WithdrawRecord{WithdrawnAt: time.Now().UTC().Add(time.Hour)}); err != nil {
		t.Fatal(err)
	}
	if out := mustRun(t, "--store", storePath, "prune", "--age", "0"); out != c.ID+" withdrawn, would be archived\n" {
		t.Errorf("prune --age 0 = %q", out)
	}
}
