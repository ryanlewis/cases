package main

import (
	"os"
	"path/filepath"
	"slices"
	"strings"
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
	if r := runCases(t, "", "--store", root, "close", id); r.err == nil || !strings.Contains(r.err.Error(), "missing flags: --outcome=TEXT or --outcome-file=FILE") {
		t.Errorf("close without an outcome: err = %v", r.err)
	}
}

func TestCloseAtRevision(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	mustRun(t, "--store", root, "pickup", id)
	refusedAsStale(t, root, id, 2, 3, "close", id, "--outcome", "Pinned.", "--revision", "2")
	if out := mustRun(t, "--store", root, "close", id, "--outcome", "Pinned.", "--revision", "3"); out != id+" closed\n" {
		t.Errorf("stdout = %q", out)
	}
}

func TestCloseInlineOutcome(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	mustRun(t, "--store", root, "pickup", id)

	file := filepath.Join(t.TempDir(), "outcome.md")
	if err := os.WriteFile(file, []byte("Pinned in #12."), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, tt := range []struct {
		name    string
		args    []string
		wantErr string
	}{
		{"both forms", []string{"--outcome", "Acknowledged", "--outcome-file", file}, "--outcome and --outcome-file can't be used together"},
		{"a file name", []string{"--outcome", file}, "names a file; pass it with --outcome-file"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			r := runCases(t, "", append([]string{"--store", root, "close", id}, tt.args...)...)
			if r.err == nil || !strings.Contains(r.err.Error(), tt.wantErr) {
				t.Errorf("err = %v, want %q", r.err, tt.wantErr)
			}
			if c := loadCase(t, root, id); c.Close != nil {
				t.Errorf("close = %+v", c.Close)
			}
		})
	}

	if out := mustRun(t, "--store", root, "close", id, "--outcome", "Acknowledged"); out != id+" closed\n" {
		t.Errorf("stdout = %q", out)
	}
	if c := loadCase(t, root, id); c.Close == nil || c.Close.Outcome != "Acknowledged" {
		t.Errorf("close = %+v", c.Close)
	}
}
