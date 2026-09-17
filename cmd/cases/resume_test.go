package main

import (
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

func TestResume(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	if r := runCases(t, "", "--store", storePath, "resume", id); r.err == nil {
		t.Error("resumed an open case")
	}
	mustRun(t, "--store", storePath, "answer", id, "--park")
	if out := mustRun(t, "--store", storePath, "resume", id); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
	mustRun(t, "--store", storePath, "answer", id, "--park")
	mustRun(t, "--store", storePath, "resume", id, "--agent")

	c := loadCase(t, storePath, id)
	last := c.Events[len(c.Events)-1]
	if c.State != store.StateOpen || last.Author != store.AuthorAgent || last.File != "0005-agent-resume.json" {
		t.Errorf("state %s, last event %+v", c.State, last)
	}
}

func TestResumeAtRevision(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))
	mustRun(t, "--store", storePath, "answer", id, "--park")
	// A resume and a second park elsewhere leave the case parked, as it was
	// read, but at a later revision.
	mustRun(t, "--store", storePath, "resume", id, "--agent")
	mustRun(t, "--store", storePath, "answer", id, "--park")

	refusedAsStale(t, storePath, id, 2, 4, "resume", id, "--revision", "2")
	if out := mustRun(t, "--store", storePath, "resume", id, "--revision", "4"); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
	if c := loadCase(t, storePath, id); c.State != store.StateOpen || c.Revision() != 5 {
		t.Errorf("state %s, revision %d", c.State, c.Revision())
	}
}

func TestResumeTakesPartOfAnID(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked on mirror"))
	mustRun(t, "--store", storePath, "answer", "mirror", "--park")
	if out := mustRun(t, "--store", storePath, "resume", "mirror"); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
}
