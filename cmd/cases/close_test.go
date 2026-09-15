package main

import (
	"os"
	"path/filepath"
	"slices"
	"testing"
)

func TestClose(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	outcome := filepath.Join(t.TempDir(), "outcome.md")
	if err := os.WriteFile(outcome, []byte("Pinned in #12."), 0o644); err != nil {
		t.Fatal(err)
	}
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	if r := runCases(t, "", "--store", root, "close", id, "--outcome-file", outcome); r.err == nil {
		t.Error("closed a case that was not picked up")
	}
	mustRun(t, "--store", root, "pickup", id)
	if out := mustRun(t, "--store", root, "close", id, "--outcome-file", outcome, "--link", "https://github.com/x/y/pull/12"); out != id+" closed\n" {
		t.Errorf("stdout = %q", out)
	}
	c := loadCase(t, root, id)
	if c.Close.Outcome != "Pinned in #12." || !slices.Equal(c.Close.Links, []string{"https://github.com/x/y/pull/12"}) {
		t.Errorf("close = %+v", c.Close)
	}
	if r := runCases(t, "", "--store", root, "close", id, "--outcome-file", filepath.Join(root, "missing.md")); r.err == nil {
		t.Error("missing outcome file accepted")
	}
}
