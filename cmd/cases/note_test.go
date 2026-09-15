package main

import (
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
	if r := runCases(t, "", "--store", root, "note", id); r.err == nil {
		t.Error("note without --body-file accepted")
	}
}
