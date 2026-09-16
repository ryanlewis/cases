package main

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// openAt opens an fyi case in root with its open event dated at.
func openAt(t *testing.T, root, title string, at time.Time, rec store.OpenRecord) *store.Case {
	t.Helper()
	rec.Kind, rec.Urgency, rec.Title, rec.OpenedAt = store.KindFYI, store.UrgencyWhenever, title, at
	c, err := store.Create(root, rec)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// storeFiles lists every file under root, so a test can check nothing was written.
func storeFiles(t *testing.T, root string) []string {
	t.Helper()
	var files []string
	err := filepath.WalkDir(root, func(path string, e os.DirEntry, err error) error {
		if err == nil && !e.IsDir() {
			files = append(files, path)
		}
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	return files
}

func TestSweepDryRunWritesNothing(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	before := storeFiles(t, root)

	r := runCases(t, "", "--store", root, "sweep")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if r.stdout != id+" would be withdrawn: Pin bun?\n" {
		t.Errorf("stdout = %q", r.stdout)
	}
	if !strings.Contains(r.stderr, "1 cases would be withdrawn. Pass --yes") {
		t.Errorf("stderr = %q", r.stderr)
	}
	if after := storeFiles(t, root); len(after) != len(before) {
		t.Errorf("dry run wrote files: %v", after)
	}
}

func TestSweepWithdrawsOpenCasesAndLeavesTheRest(t *testing.T) {
	root := t.TempDir()
	open := openDecision(t, root)
	answered := openDecision(t, root)
	mustRun(t, "--store", root, "answer", answered, "--option", "1")
	parked := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "stuck", "--urgency", "today", "--title", "Stuck"))
	mustRun(t, "--store", root, "answer", parked, "--park")
	closed := openDecision(t, root)
	mustRun(t, "--store", root, "answer", closed, "--option", "1")
	mustRun(t, "--store", root, "pickup", closed)
	if r := runCases(t, "done", "--store", root, "close", closed, "--outcome-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}

	out := mustRun(t, "--store", root, "sweep", "--yes", "--reason", "clearing the inbox")
	for _, want := range []string{open + " withdrawn\n", answered + " answered, left\n", parked + " parked, left\n"} {
		if !strings.Contains(out, want) {
			t.Errorf("sweep --yes lacks %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, closed) {
		t.Errorf("sweep --yes mentions the closed case:\n%s", out)
	}
	c := loadCase(t, root, open)
	if c.State != store.StateWithdrawn {
		t.Errorf("open case state = %s", c.State)
	}
	if last := c.Events[len(c.Events)-1]; !strings.Contains(string(last.Data), `"reason": "clearing the inbox"`) {
		t.Errorf("withdraw event = %s", last.Data)
	}
	for id, want := range map[string]store.State{answered: store.StateAnswered, parked: store.StateParked, closed: store.StateClosed} {
		if got := loadCase(t, root, id).State; got != want {
			t.Errorf("%s state = %s, want %s", id, got, want)
		}
	}

	// The default reason, and a second sweep has nothing to withdraw.
	other := openDecision(t, root)
	mustRun(t, "--store", root, "sweep", "-y")
	c = loadCase(t, root, other)
	if last := c.Events[len(c.Events)-1]; !strings.Contains(string(last.Data), `"reason": "swept"`) {
		t.Errorf("withdraw event = %s", last.Data)
	}
	if out := mustRun(t, "--store", root, "sweep", "--yes"); strings.Contains(out, "withdrawn") {
		t.Errorf("second sweep = %q", out)
	}
}

func TestSweepFilters(t *testing.T) {
	root := t.TempDir()
	now := time.Now().UTC()
	old := openAt(t, root, "Old", now.Add(-48*time.Hour), store.OpenRecord{Labels: []string{"round-1"}, Worker: "w1"})
	recent := openAt(t, root, "Recent", now.Add(-time.Hour), store.OpenRecord{Labels: []string{"round-1"}, Worker: "w1"})
	other := openAt(t, root, "Other", now.Add(-48*time.Hour), store.OpenRecord{Labels: []string{"round-2"}, Worker: "w2"})

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
		out := mustRun(t, append([]string{"--store", root, "sweep"}, tt.args...)...)
		for _, id := range []string{old.ID, recent.ID, other.ID} {
			if strings.Contains(out, id) != slices.Contains(tt.want, id) {
				t.Errorf("sweep %v:\n%s", tt.args, out)
				break
			}
		}
	}

	mustRun(t, "--store", root, "sweep", "--older-than", "24h", "--label", "round-1", "--yes")
	for id, want := range map[string]store.State{old.ID: store.StateWithdrawn, recent.ID: store.StateOpen, other.ID: store.StateOpen} {
		if got := loadCase(t, root, id).State; got != want {
			t.Errorf("%s state = %s, want %s", id, got, want)
		}
	}
}

func TestSweepContinuesPastARefusal(t *testing.T) {
	root := t.TempDir()
	stuck := openDecision(t, root)
	fine := openDecision(t, root)
	// A case directory that cannot be written to refuses the withdraw.
	if err := os.Chmod(filepath.Join(root, stuck), 0o555); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chmod(filepath.Join(root, stuck), 0o755) })

	r := runCases(t, "", "--store", root, "sweep", "--yes")
	if r.err == nil || !strings.Contains(r.err.Error(), "1 cases were not withdrawn") || !strings.Contains(r.err.Error(), stuck+": ") {
		t.Errorf("err = %v", r.err)
	}
	if !strings.Contains(r.stdout, fine+" withdrawn\n") {
		t.Errorf("stdout = %q", r.stdout)
	}
	if got := loadCase(t, root, stuck).State; got != store.StateOpen {
		t.Errorf("refused case state = %s", got)
	}
	if got := loadCase(t, root, fine).State; got != store.StateWithdrawn {
		t.Errorf("other case state = %s", got)
	}
}
