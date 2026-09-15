package main

import (
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

func TestResume(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	if r := runCases(t, "", "--store", root, "resume", id); r.err == nil {
		t.Error("resumed an open case")
	}
	mustRun(t, "--store", root, "answer", id, "--park")
	if out := mustRun(t, "--store", root, "resume", id); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
	mustRun(t, "--store", root, "answer", id, "--park")
	mustRun(t, "--store", root, "resume", id, "--agent")

	c := loadCase(t, root, id)
	last := c.Events[len(c.Events)-1]
	if c.State != store.StateOpen || last.Author != store.AuthorAgent || last.File != "0005-agent-resume.json" {
		t.Errorf("state %s, last event %+v", c.State, last)
	}
}
