package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// pruneStore holds one case in each state prune looks at, all last touched at
// the time given, plus a withdrawn case from an hour ago.
type pruneStore struct {
	root                            string
	closed, withdrawn, open, recent string
}

func newPruneStore(t *testing.T) pruneStore {
	t.Helper()
	root := t.TempDir()
	old := time.Now().UTC().Add(-60 * 24 * time.Hour)
	s := pruneStore{root: root}

	c := openAt(t, root, "Closed", old, store.OpenRecord{})
	s.closed = c.ID
	if _, err := store.Answer(c.Dir, store.AnswerRecord{Ack: true, AnsweredAt: old}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Pickup(c.Dir, store.PickupRecord{PickedUpAt: old}); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Close(c.Dir, store.CloseRecord{Outcome: "done", ClosedAt: old}); err != nil {
		t.Fatal(err)
	}

	c = openAt(t, root, "Withdrawn", old, store.OpenRecord{})
	s.withdrawn = c.ID
	if _, err := store.Withdraw(c.Dir, store.WithdrawRecord{WithdrawnAt: old}); err != nil {
		t.Fatal(err)
	}

	s.open = openAt(t, root, "Open", old, store.OpenRecord{}).ID

	c = openAt(t, root, "Recent", old, store.OpenRecord{})
	s.recent = c.ID
	if _, err := store.Withdraw(c.Dir, store.WithdrawRecord{WithdrawnAt: time.Now().UTC().Add(-time.Hour)}); err != nil {
		t.Fatal(err)
	}
	return s
}

func exists(t *testing.T, path string) bool {
	t.Helper()
	_, err := os.Stat(path)
	if err != nil && !os.IsNotExist(err) {
		t.Fatal(err)
	}
	return err == nil
}

func TestPruneDryRunWritesNothing(t *testing.T) {
	s := newPruneStore(t)
	before := storeFiles(t, s.root)

	r := runCases(t, "", "--store", s.root, "prune")
	if r.err != nil {
		t.Fatal(r.err)
	}
	want := s.closed + " closed, would be archived\n" + s.withdrawn + " withdrawn, would be archived\n"
	if r.stdout != want {
		t.Errorf("stdout = %q, want %q", r.stdout, want)
	}
	if !strings.Contains(r.stderr, "2 cases would be moved to "+filepath.Join(s.root, ".archive")) {
		t.Errorf("stderr = %q", r.stderr)
	}
	if after := storeFiles(t, s.root); len(after) != len(before) {
		t.Errorf("dry run wrote files: %v", after)
	}
	if exists(t, filepath.Join(s.root, ".archive")) {
		t.Error("dry run made the archive directory")
	}
}

func TestPruneArchives(t *testing.T) {
	s := newPruneStore(t)
	out := mustRun(t, "--store", s.root, "prune", "--yes")
	if want := s.closed + " archived\n" + s.withdrawn + " archived\n"; out != want {
		t.Errorf("stdout = %q, want %q", out, want)
	}
	for _, id := range []string{s.closed, s.withdrawn} {
		if exists(t, filepath.Join(s.root, id)) {
			t.Errorf("%s is still in the store", id)
		}
		// The whole directory moved, event files and all, and still folds.
		if c, err := store.Load(filepath.Join(s.root, ".archive", id)); err != nil || len(c.Problems) > 0 {
			t.Errorf("archived %s: %v %v", id, err, c)
		}
	}
	for _, id := range []string{s.open, s.recent} {
		if !exists(t, filepath.Join(s.root, id)) {
			t.Errorf("%s was pruned", id)
		}
	}

	list := mustRun(t, "--store", s.root, "list")
	if strings.Contains(list, s.closed) || strings.Contains(list, s.withdrawn) || !strings.Contains(list, s.open) || !strings.Contains(list, s.recent) {
		t.Errorf("list after prune:\n%s", list)
	}

	// --age 0 takes the recent one too, and --state narrows.
	if out := mustRun(t, "--store", s.root, "prune", "--age", "0", "--state", "closed"); out != "" {
		t.Errorf("prune --state closed with nothing closed = %q", out)
	}
	if out := mustRun(t, "--store", s.root, "prune", "--age", "0", "--state", "withdrawn", "--yes"); out != s.recent+" archived\n" {
		t.Errorf("prune --age 0 = %q", out)
	}
}

func TestPruneRefusesAnExistingTarget(t *testing.T) {
	s := newPruneStore(t)
	taken := filepath.Join(s.root, ".archive", s.closed)
	if err := os.MkdirAll(taken, 0o755); err != nil {
		t.Fatal(err)
	}

	r := runCases(t, "", "--store", s.root, "prune", "--yes")
	if r.err == nil || !strings.Contains(r.err.Error(), "1 cases were not pruned") || !strings.Contains(r.err.Error(), s.closed+": "+taken+" already exists") {
		t.Errorf("err = %v", r.err)
	}
	if r.stdout != s.withdrawn+" archived\n" {
		t.Errorf("stdout = %q", r.stdout)
	}
	if got := loadCase(t, s.root, s.closed).State; got != store.StateClosed {
		t.Errorf("refused case state = %s", got)
	}
	if entries, _ := os.ReadDir(taken); len(entries) != 0 {
		t.Errorf("wrote into the existing target: %v", entries)
	}
}

func TestPruneDelete(t *testing.T) {
	s := newPruneStore(t)
	if out := mustRun(t, "--store", s.root, "prune", "--delete"); !strings.Contains(out, s.closed+" closed, would be deleted\n") {
		t.Errorf("dry run = %q", out)
	}
	if out := mustRun(t, "--store", s.root, "prune", "--delete", "--yes"); out != s.closed+" deleted\n"+s.withdrawn+" deleted\n" {
		t.Errorf("stdout = %q", out)
	}
	for _, id := range []string{s.closed, s.withdrawn} {
		if exists(t, filepath.Join(s.root, id)) {
			t.Errorf("%s is still in the store", id)
		}
	}
	if exists(t, filepath.Join(s.root, ".archive")) {
		t.Error("--delete made the archive directory")
	}
	if !exists(t, filepath.Join(s.root, s.open)) {
		t.Error("deleted the open case")
	}
}

func TestPruneSkipsABrokenCase(t *testing.T) {
	s := newPruneStore(t)
	broken := filepath.Join(s.root, "2020-01-01T00-00-00Z-broken")
	if err := os.Mkdir(broken, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(broken, "0001-agent-open.json"), []byte("{"), 0o644); err != nil {
		t.Fatal(err)
	}

	r := runCases(t, "", "--store", s.root, "prune", "--yes", "--age", "0")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if !strings.Contains(r.stderr, "warning: skipped "+broken) {
		t.Errorf("stderr = %q", r.stderr)
	}
	if strings.Contains(r.stdout, "broken") || !exists(t, filepath.Join(broken, "0001-agent-open.json")) {
		t.Errorf("pruned the broken case: %q", r.stdout)
	}
}

func TestPruneRefusesOtherStates(t *testing.T) {
	root := t.TempDir()
	for _, args := range [][]string{{"--state", "open"}, {"--state", "closed,answered"}, {"--state", "nope"}, {"--age", "-1h"}} {
		if r := runCases(t, "", append([]string{"--store", root, "prune"}, args...)...); r.err == nil {
			t.Errorf("prune %v was accepted", args)
		}
	}
}

func TestPruneOnMissingStore(t *testing.T) {
	root := filepath.Join(t.TempDir(), "nothing-here")
	if out := mustRun(t, "--store", root, "prune", "--yes"); out != "" {
		t.Errorf("stdout = %q", out)
	}
	if exists(t, root) {
		t.Error("prune made the store")
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
	if out := mustRun(t, "--store", s.root, "prune"); !strings.Contains(out, s.recent) {
		t.Errorf("prune with prune-age 30m = %q, want the case withdrawn an hour ago", out)
	}
	if out := mustRun(t, "--store", s.root, "prune", "--age", "720h"); strings.Contains(out, s.recent) {
		t.Errorf("prune --age 720h = %q, want the flag to beat the file", out)
	}

	// The key seeds prune only; another command's flags are untouched.
	writeConfig(t, "prune-age = \"not a duration\"\n")
	mustRun(t, "--store", s.root, "list")
	if r := runCases(t, "", "--store", s.root, "prune"); r.err == nil {
		t.Error("prune accepted a bad prune-age")
	}
}

func TestPruneAgeZeroTakesAFutureCase(t *testing.T) {
	root := t.TempDir()
	// A withdraw synced from a machine whose clock runs ahead.
	c := openAt(t, root, "Ahead", time.Now().UTC(), store.OpenRecord{})
	if _, err := store.Withdraw(c.Dir, store.WithdrawRecord{WithdrawnAt: time.Now().UTC().Add(time.Hour)}); err != nil {
		t.Fatal(err)
	}
	if out := mustRun(t, "--store", root, "prune", "--age", "0"); out != c.ID+" withdrawn, would be archived\n" {
		t.Errorf("prune --age 0 = %q", out)
	}
}
