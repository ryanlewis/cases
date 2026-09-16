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

// startWait runs `cases wait` in the background and returns its result
// channel. It sleeps briefly so the first poll, which fixes what counts as
// already there, happens before the caller writes anything.
func startWait(t *testing.T, root string, args ...string) <-chan result {
	t.Helper()
	ch := make(chan result, 1)
	go func() { ch <- runCases(t, "", append([]string{"--store", root, "wait"}, args...)...) }()
	time.Sleep(40 * time.Millisecond)
	return ch
}

type waitedCase struct {
	ID     string `json:"id"`
	Kind   string `json:"kind"`
	State  string `json:"state"`
	Answer *struct {
		Choice int  `json:"choice"`
		Ack    bool `json:"ack"`
	} `json:"answer"`
}

// waited parses wait's JSON lines.
func waited(t *testing.T, r result) []waitedCase {
	t.Helper()
	if r.err != nil {
		t.Fatalf("wait: %v (stderr %q)", r.err, r.stderr)
	}
	var out []waitedCase
	for line := range strings.SplitSeq(strings.TrimSpace(r.stdout), "\n") {
		var c waitedCase
		if err := json.Unmarshal([]byte(line), &c); err != nil {
			t.Fatalf("line %q: %v", line, err)
		}
		out = append(out, c)
	}
	return out
}

func assertTimedOut(t *testing.T, r result) {
	t.Helper()
	var ee *exitError
	if !errors.As(r.err, &ee) || ee.code != 2 {
		t.Fatalf("err = %v, want a timeout with exit 2", r.err)
	}
	if r.stdout != "" {
		t.Errorf("stdout = %q on timeout, want nothing", r.stdout)
	}
}

func TestWaitReturnsAnAnswerThatLands(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	ch := startWait(t, root, "--timeout", "5s")
	mustRun(t, "--store", root, "answer", id, "--option", "2")

	got := waited(t, <-ch)
	if len(got) != 1 || got[0].ID != id || got[0].Kind != "decision" || got[0].State != "answered" || got[0].Answer == nil || got[0].Answer.Choice != 2 {
		t.Errorf("wait printed %+v", got)
	}
}

func TestWaitTimesOutWithExitTwo(t *testing.T) {
	root := t.TempDir()
	openDecision(t, root)
	start := time.Now()
	r := runCases(t, "", "--store", root, "wait", "--timeout", "100ms")
	assertTimedOut(t, r)
	if strings.Count(r.err.Error(), "\n") != 0 || !strings.Contains(r.err.Error(), "within 100ms") {
		t.Errorf("stderr line = %q", r.err.Error())
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Errorf("timeout took %s", elapsed)
	}
}

func TestWaitIgnoresEventsFromBeforeItStarted(t *testing.T) {
	root := t.TempDir()
	old := openDecision(t, root)
	mustRun(t, "--store", root, "answer", old, "--option", "1")
	assertTimedOut(t, runCases(t, "", "--store", root, "wait", "--timeout", "100ms"))

	// Once something new lands, every case still waiting on the agent is
	// printed, the old answer included.
	fresh := openDecision(t, root)
	ch := startWait(t, root, "--timeout", "5s")
	mustRun(t, "--store", root, "answer", fresh, "--option", "2")
	got := waited(t, <-ch)
	if len(got) != 2 || got[0].ID != old || got[1].ID != fresh {
		t.Errorf("wait printed %+v, want %s then %s", got, old, fresh)
	}
}

func TestWaitSinceCountsEarlierEvents(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	before := time.Now().UTC().Add(-time.Minute).Format(time.RFC3339)
	mustRun(t, "--store", root, "answer", id, "--option", "1")

	if got := waited(t, runCases(t, "", "--store", root, "wait", "--since", before, "--timeout", "1s")); len(got) != 1 || got[0].ID != id {
		t.Errorf("wait printed %+v", got)
	}
	later := time.Now().UTC().Add(time.Minute).Format(time.RFC3339)
	assertTimedOut(t, runCases(t, "", "--store", root, "wait", "--since", later, "--timeout", "100ms"))
	if r := runCases(t, "", "--store", root, "wait", "--since", "yesterday"); r.err == nil {
		t.Error("bad --since accepted")
	}
}

func TestWaitParkAndResume(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	ch := startWait(t, root, "--timeout", "5s")
	mustRun(t, "--store", root, "answer", id, "--park")
	if got := waited(t, <-ch); len(got) != 1 || got[0].State != "parked" {
		t.Errorf("after park: %+v", got)
	}

	ch = startWait(t, root, "--timeout", "5s")
	mustRun(t, "--store", root, "resume", id)
	if got := waited(t, <-ch); len(got) != 1 || got[0].State != "open" {
		t.Errorf("after human resume: %+v", got)
	}

	// The agent's own events never wake it.
	mustRun(t, "--store", root, "answer", id, "--park")
	ch = startWait(t, root, "--timeout", "300ms")
	mustRun(t, "--store", root, "resume", id, "--agent")
	mustRun(t, "--store", root, "withdraw", id)
	assertTimedOut(t, <-ch)
}

func TestWaitOnSpecificCases(t *testing.T) {
	root := t.TempDir()
	a := openDecision(t, root)
	b := openDecision(t, root)

	ch := startWait(t, root, "--id", b, "--timeout", "5s")
	mustRun(t, "--store", root, "answer", a, "--option", "1")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", root, "answer", b, "--option", "2")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != b {
		t.Errorf("wait --id %s printed %+v", b, got)
	}
	if r := runCases(t, "", "--store", root, "wait", "--id", "../x"); r.err == nil {
		t.Error("bad --id accepted")
	}
}

func TestWaitForAStoreThatDoesNotExistYet(t *testing.T) {
	root := filepath.Join(t.TempDir(), "not-yet")
	ch := startWait(t, root, "--timeout", "5s")
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id {
		t.Errorf("wait printed %+v", got)
	}
}

func TestWaitReportsAnAnswerWithoutTimestamp(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Hand answered"))
	ch := startWait(t, root, "--timeout", "5s")
	// An answer written by hand or synced from another tool may leave
	// answered_at out; it still counts because its file is new.
	if err := os.WriteFile(filepath.Join(root, id, "0002-human-answer.json"), []byte(`{"ack":true}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id || got[0].Answer == nil || !got[0].Answer.Ack {
		t.Errorf("wait printed %+v", got)
	}
}

func TestWaitWarnsOnceAboutAMalformedAnswer(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	ch := startWait(t, root, "--timeout", "300ms")
	// The answer is skipped when the case loads, so the case never needs the
	// agent; the warning is the only sign that anything arrived.
	if err := os.WriteFile(filepath.Join(root, id, "0002-human-answer.json"), []byte(`{"choice": `), 0o644); err != nil {
		t.Fatal(err)
	}
	r := <-ch
	assertTimedOut(t, r)
	// Wait polls every 10ms in tests, so the problem is seen on many polls.
	want := "warning: " + id + ": 0002-human-answer.json: "
	if n := strings.Count(r.stderr, want); n != 1 {
		t.Errorf("stderr has %d warnings starting %q, want 1:\n%s", n, want, r.stderr)
	}
	if n := strings.Count(r.stderr, "warning: "); n != 1 {
		t.Errorf("stderr has %d warnings, want 1:\n%s", n, r.stderr)
	}
}
