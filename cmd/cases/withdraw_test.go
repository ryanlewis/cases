package main

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func TestWithdraw(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	if out := mustRun(t, "--store", root, "withdraw", id); out != id+" withdrawn\n" {
		t.Errorf("stdout = %q", out)
	}
	if r := runCases(t, "", "--store", root, "answer", id, "--option", "1"); r.err == nil {
		t.Error("answered a withdrawn case")
	}
	if r := runCases(t, "", "--store", root, "withdraw", id); r.err == nil {
		t.Error("withdrew twice")
	}
}

func TestWithdrawAtRevision(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "amend", id, "--link", "https://example.com/log")
	refusedAsStale(t, root, id, 1, 2, "withdraw", id, "--revision", "1")
	if out := mustRun(t, "--store", root, "withdraw", id, "--revision", "2"); out != id+" withdrawn\n" {
		t.Errorf("stdout = %q", out)
	}
}

func TestWithdrawReason(t *testing.T) {
	withdrawFile := func(t *testing.T, root, id string) string {
		t.Helper()
		data, err := os.ReadFile(filepath.Join(root, id, "0002-agent-withdraw.json"))
		if err != nil {
			t.Fatal(err)
		}
		return string(data)
	}

	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "withdraw", id)
	// Without a reason the file is what earlier versions wrote.
	if got := withdrawFile(t, root, id); !regexp.MustCompile(`^\{\s*"withdrawn_at": "[^"]+"\s*\}\s*$`).MatchString(got) {
		t.Errorf("file without a reason = %s", got)
	}
	if out := mustRun(t, "--store", root, "show", id); strings.Contains(out, "reason:") {
		t.Errorf("show without a reason:\n%s", out)
	}

	root = t.TempDir()
	id = openDecision(t, root)
	mustRun(t, "--store", root, "withdraw", id, "--reason", "found it in the lockfile")
	if got := withdrawFile(t, root, id); !strings.Contains(got, `"reason": "found it in the lockfile"`) {
		t.Errorf("file with a reason = %s", got)
	}
	if out := mustRun(t, "--store", root, "show", id); !strings.Contains(out, "0002 agent withdraw") || !strings.Contains(out, "       reason: found it in the lockfile\n") {
		t.Errorf("show with a reason:\n%s", out)
	}
	if c := loadCase(t, root, id); c.State != "withdrawn" {
		t.Errorf("state = %s", c.State)
	}
}
