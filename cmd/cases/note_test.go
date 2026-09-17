package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestNoteReopensAnAnsweredCase(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	mustRun(t, "--store", root, "pickup", id)

	r := runCases(t, "Pin to which patch?", "--store", root, "note", id, "--body-file", "-")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if r.stdout != id+" open\n" {
		t.Errorf("stdout = %q", r.stdout)
	}
	c := loadCase(t, root, id)
	if c.Answer != nil || len(c.Events) != 4 {
		t.Errorf("answer = %+v, events = %d", c.Answer, len(c.Events))
	}
	mustRun(t, "--store", root, "answer", id, "--option", "2")

	if r := runCases(t, "   \n", "--store", root, "note", id, "--body-file", "-"); r.err == nil || !strings.Contains(r.err.Error(), "empty") {
		t.Errorf("empty note: err = %v", r.err)
	}
	if r := runCases(t, "", "--store", root, "note", id); r.err == nil || !strings.Contains(r.err.Error(), "missing flags: --body=TEXT or --body-file=FILE") {
		t.Errorf("note without a body: err = %v", r.err)
	}
}

// A note made on a case read before the human answered would reopen it and
// throw the answer away; with --revision it is refused.
func TestNoteAtRevision(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	refusedAsStale(t, root, id, 1, 2, "note", id, "--body", "Pin to which patch?", "--revision", "1")
	if c := loadCase(t, root, id); c.Answer == nil {
		t.Error("the refused note cleared the answer")
	}
	if out := mustRun(t, "--store", root, "note", id, "--body", "Pin to which patch?", "--revision", "2"); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
}

func TestNoteInlineBody(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	file := filepath.Join(t.TempDir(), "note.md")
	if err := os.WriteFile(file, []byte("From a file."), 0o644); err != nil {
		t.Fatal(err)
	}
	if r := runCases(t, "", "--store", root, "note", id, "--body", "x", "--body-file", file); r.err == nil || !strings.Contains(r.err.Error(), "--body and --body-file can't be used together") {
		t.Errorf("both forms: err = %v", r.err)
	}
	if r := runCases(t, "", "--store", root, "note", id, "--body", file); r.err == nil || !strings.Contains(r.err.Error(), "names a file; pass it with --body-file") {
		t.Errorf("a file name: err = %v", r.err)
	}
	if r := runCases(t, "", "--store", root, "note", id, "--body", " "); r.err == nil || !strings.Contains(r.err.Error(), "empty") {
		t.Errorf("blank body: err = %v", r.err)
	}
	if n := len(eventFiles(t, root, id)); n != 1 {
		t.Fatalf("refused notes wrote files: %d", n)
	}
	if out := mustRun(t, "--store", root, "note", id, "--body", "Also pin bunx."); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
	if c := loadCase(t, root, id); len(c.Events) != 2 || !strings.Contains(string(c.Events[1].Data), "Also pin bunx.") {
		t.Errorf("events = %+v", c.Events)
	}
}
