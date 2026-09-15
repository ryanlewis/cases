package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

// TestDecisionEndToEnd drives a decision case through the CLI the way the
// manager and the human would: open, answer, wait, pickup, close.
func TestDecisionEndToEnd(t *testing.T) {
	root := filepath.Join(t.TempDir(), "cases")
	body := filepath.Join(t.TempDir(), "body.md")
	if err := os.WriteFile(body, []byte("Bun 1.2.4 breaks the lockfile.\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	// The manager opens the case and starts waiting before anyone answers.
	id := strings.TrimSpace(mustRun(t, "--store", root, "open",
		"--kind", "decision", "--urgency", "blocking", "--worker", "bun-pins", "--brief", "/briefs/bun-pins.md",
		"--title", "Pin bun or float?", "--body-file", body,
		"--option", "Pin to 1.2.3", "--option", "Float and fix the lockfile", "--link", "https://github.com/oven-sh/bun/issues/1"))
	start := time.Now().UTC().Add(-time.Second).Format(time.RFC3339)

	waited := make(chan result, 1)
	go func() { waited <- runCases(t, "", "--store", root, "wait", "--since", start, "--timeout", "10s") }()

	// The human answers from the other side of the store.
	time.Sleep(30 * time.Millisecond)
	mustRun(t, "--store", root, "answer", id, "--option", "1", "--note", "Revisit after 1.3")

	w := <-waited
	if w.err != nil {
		t.Fatalf("wait: %v", w.err)
	}
	var got struct {
		ID     string `json:"id"`
		State  string `json:"state"`
		Worker string `json:"worker"`
		Brief  string `json:"brief"`
		Answer struct {
			Choice int    `json:"choice"`
			Note   string `json:"note"`
		} `json:"answer"`
	}
	if err := json.Unmarshal([]byte(strings.TrimSpace(w.stdout)), &got); err != nil {
		t.Fatalf("wait output %q: %v", w.stdout, err)
	}
	if got.ID != id || got.State != "answered" || got.Answer.Choice != 1 || got.Answer.Note != "Revisit after 1.3" || got.Brief != "/briefs/bun-pins.md" {
		t.Errorf("wait printed %+v", got)
	}

	mustRun(t, "--store", root, "pickup", id, "--by", "manager")
	if r := runCases(t, "Pinned in #12.\n", "--store", root, "close", id, "--outcome-file", "-"); r.err != nil {
		t.Fatalf("close: %v", r.err)
	}

	// A closed case is no longer waited on.
	if r := runCases(t, "", "--store", root, "wait", "--timeout", "50ms"); r.err == nil {
		t.Errorf("wait returned %q for a store with nothing answered", r.stdout)
	}

	entries, err := os.ReadDir(filepath.Join(root, id))
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, e := range entries {
		names = append(names, e.Name())
	}
	want := []string{"0001-agent-open.json", "0002-human-answer.json", "0003-agent-pickup.json", "0004-agent-close.json"}
	if !slices.Equal(names, want) {
		t.Errorf("files = %v, want %v", names, want)
	}
	if out := mustRun(t, "--store", root, "list", "--state", "closed"); !strings.Contains(out, id) {
		t.Errorf("list closed = %q", out)
	}
}
