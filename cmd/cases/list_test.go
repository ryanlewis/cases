package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestListOrderFilterAndJSON(t *testing.T) {
	root := t.TempDir()
	later := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "Later"))
	urgent := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "blocking", "--title", "Urgent"))
	answered := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Seen"))
	mustRun(t, "--store", root, "answer", answered, "--ack")

	out := mustRun(t, "--store", root, "list")
	iu, ia, il := strings.Index(out, urgent), strings.Index(out, answered), strings.Index(out, later)
	if iu < 0 || ia < 0 || il < 0 || iu >= ia || ia >= il {
		t.Errorf("list is not blocking, today, whenever:\n%s", out)
	}

	out = mustRun(t, "--store", root, "list", "--state", "answered", "--json")
	var got []map[string]any
	if err := json.Unmarshal([]byte(out), &got); err != nil {
		t.Fatalf("json: %v\n%s", err, out)
	}
	if len(got) != 1 || got[0]["id"] != answered || got[0]["state"] != "answered" {
		t.Errorf("filtered = %v", got)
	}

	if out := mustRun(t, "--store", root, "list", "--state", "closed,withdrawn", "--json"); strings.TrimSpace(out) != "[]" {
		t.Errorf("empty filter = %q, want []", out)
	}
	if r := runCases(t, "", "--store", root, "list", "--state", "done"); r.err == nil {
		t.Error("unknown state accepted")
	}
}

func TestListReportsBrokenCaseAndListsTheRest(t *testing.T) {
	root := t.TempDir()
	good := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Good"))
	broken := filepath.Join(root, "2026-01-01T00-00-00Z-broken")
	if err := os.Mkdir(broken, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(broken, "0001-agent-open.json"), []byte("{"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := runCases(t, "", "--store", root, "list")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if !strings.Contains(r.stdout, good) || !strings.Contains(r.stderr, "warning: "+broken) {
		t.Errorf("stdout %q\nstderr %q", r.stdout, r.stderr)
	}
}
