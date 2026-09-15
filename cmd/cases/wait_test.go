package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestWaitPrintsAlreadyAnsweredCases(t *testing.T) {
	root := t.TempDir()
	a := openDecision(t, root)
	b := openDecision(t, root)
	openDecision(t, root) // stays open
	mustRun(t, "--store", root, "answer", a, "--option", "1")
	mustRun(t, "--store", root, "answer", b, "--other", "--note", "neither")

	lines := strings.Split(strings.TrimSpace(mustRun(t, "--store", root, "wait", "--timeout", "1s")), "\n")
	if len(lines) != 2 {
		t.Fatalf("lines = %q", lines)
	}
	var first struct {
		ID    string `json:"id"`
		State string `json:"state"`
	}
	if err := json.Unmarshal([]byte(lines[0]), &first); err != nil {
		t.Fatal(err)
	}
	if first.ID != a || first.State != "answered" {
		t.Errorf("first = %+v, want %s answered first", first, a)
	}
}

func TestWaitSeesAnAnswerLand(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	go func() {
		time.Sleep(50 * time.Millisecond)
		if r := runCases(t, "", "--store", root, "answer", id, "--option", "2"); r.err != nil {
			t.Error(r.err)
		}
	}()
	out := mustRun(t, "--store", root, "wait", "--timeout", "5s")
	if !strings.Contains(out, id) {
		t.Errorf("wait output %q, want %s", out, id)
	}
}

func TestWaitSinceAndTimeout(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")

	since := time.Now().UTC().Add(time.Minute).Format(time.RFC3339)
	start := time.Now()
	r := runCases(t, "", "--store", root, "wait", "--since", since, "--timeout", "100ms")
	var ee *exitError
	if !errors.As(r.err, &ee) || ee.code != exitTimeout {
		t.Fatalf("err = %v, want exit %d", r.err, exitTimeout)
	}
	if r.stdout != "" {
		t.Errorf("stdout = %q on timeout", r.stdout)
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Errorf("timeout took %s", elapsed)
	}

	if r := runCases(t, "", "--store", root, "wait", "--since", "yesterday"); r.err == nil {
		t.Error("bad --since accepted")
	}
}

func TestWaitReportsAnswerWithoutTimestamp(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Hand answered"))
	// An answer written by hand or by another tool may leave answered_at out.
	if err := os.WriteFile(filepath.Join(root, id, "0002-human-answer.json"), []byte(`{"ack":true}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if out := mustRun(t, "--store", root, "wait", "--timeout", "1s"); !strings.Contains(out, id) {
		t.Errorf("wait output %q, want %s", out, id)
	}
}
