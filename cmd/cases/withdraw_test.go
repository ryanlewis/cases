package main

import (
	"regexp"
	"strings"
	"testing"
)

func TestWithdraw(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	if out := mustRun(t, "--store", storePath, "withdraw", id); out != id+" withdrawn\n" {
		t.Errorf("stdout = %q", out)
	}
	if r := runCases(t, "", "--store", storePath, "answer", id, "--option", "1"); r.err == nil {
		t.Error("answered a withdrawn case")
	}
	if r := runCases(t, "", "--store", storePath, "withdraw", id); r.err == nil {
		t.Error("withdrew twice")
	}
}

func TestWithdrawAtRevision(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "amend", id, "--link", "https://example.com/log")
	refusedAsStale(t, storePath, id, 1, 2, "withdraw", id, "--revision", "1")
	if out := mustRun(t, "--store", storePath, "withdraw", id, "--revision", "2"); out != id+" withdrawn\n" {
		t.Errorf("stdout = %q", out)
	}
}

func TestWithdrawReason(t *testing.T) {
	withdrawFile := func(t *testing.T, storePath, id string) string {
		t.Helper()
		c := loadCase(t, storePath, id)
		if ev := c.Events[len(c.Events)-1]; ev.File == "0002-agent-withdraw.json" {
			return string(ev.Data)
		}
		t.Fatalf("no withdraw event: %+v", c.Events)
		return ""
	}

	storePath := newStore(t)
	id := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "withdraw", id)
	// Without a reason the record is what earlier versions wrote.
	if got := withdrawFile(t, storePath, id); !regexp.MustCompile(`^\{\s*"withdrawn_at": "[^"]+"\s*\}\s*$`).MatchString(got) {
		t.Errorf("record without a reason = %s", got)
	}
	if out := mustRun(t, "--store", storePath, "show", id); strings.Contains(out, "reason:") {
		t.Errorf("show without a reason:\n%s", out)
	}

	storePath = newStore(t)
	id = openDecision(t, storePath)
	mustRun(t, "--store", storePath, "withdraw", id, "--reason", "found it in the lockfile")
	if got := withdrawFile(t, storePath, id); !strings.Contains(got, `"reason": "found it in the lockfile"`) {
		t.Errorf("record with a reason = %s", got)
	}
	if out := mustRun(t, "--store", storePath, "show", id); !strings.Contains(out, "0002 agent withdraw") || !strings.Contains(out, "       reason: found it in the lockfile\n") {
		t.Errorf("show with a reason:\n%s", out)
	}
	if c := loadCase(t, storePath, id); c.State != "withdrawn" {
		t.Errorf("state = %s", c.State)
	}
}
