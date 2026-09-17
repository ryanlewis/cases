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
	ID        string    `json:"id"`
	Kind      string    `json:"kind"`
	State     string    `json:"state"`
	UpdatedAt time.Time `json:"updated_at"`
	NextSince string    `json:"next_since"`
	Answer    *struct {
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

func TestWaitByLabelAndWorker(t *testing.T) {
	root := t.TempDir()
	open := func(args ...string) string {
		return strings.TrimSpace(mustRun(t, append([]string{"--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Labelled"}, args...)...))
	}
	mine := open("--label", "round-1", "--worker", "w1")
	theirs := open("--label", "round-2", "--worker", "w2")
	otherWorker := open("--label", "round-1", "--worker", "w2")

	// Answers on cases that do not match do not wake it.
	ch := startWait(t, root, "--label", "round-1", "--worker", "w1", "--timeout", "5s")
	mustRun(t, "--store", root, "answer", theirs, "--ack")
	mustRun(t, "--store", root, "answer", otherWorker, "--ack")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", root, "answer", mine, "--ack")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != mine {
		t.Errorf("wait --label --worker printed %+v, want only %s", got, mine)
	}

	// A filter no case matches waits until the timeout.
	ch = startWait(t, root, "--label", "nope", "--timeout", "300ms")
	fresh := open("--label", "round-1")
	mustRun(t, "--store", root, "answer", fresh, "--ack")
	assertTimedOut(t, <-ch)
}

func TestWaitByIDAndLabel(t *testing.T) {
	root := t.TempDir()
	labelled := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "A", "--label", "round-1"))
	unlabelled := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "B"))
	other := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "C", "--label", "round-1"))

	// --id and --label together wait on cases that pass both.
	ch := startWait(t, root, "--id", labelled, "--id", unlabelled, "--label", "round-1", "--timeout", "5s")
	mustRun(t, "--store", root, "answer", unlabelled, "--ack")
	mustRun(t, "--store", root, "answer", other, "--ack")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", root, "answer", labelled, "--ack")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != labelled {
		t.Errorf("wait --id --label printed %+v, want only %s", got, labelled)
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

func TestWaitForHuman(t *testing.T) {
	root := t.TempDir()
	answered := openDecision(t, root)
	mustRun(t, "--store", root, "answer", answered, "--option", "1")
	waiting := openDecision(t, root)

	// A case already waiting on the human when wait starts does not wake it.
	assertTimedOut(t, runCases(t, "", "--store", root, "wait", "--for", "human", "--timeout", "100ms"))

	// A new case does, and every case waiting on the human is printed; the
	// answered one is not.
	ch := startWait(t, root, "--for", "human", "--timeout", "5s")
	opened := openDecision(t, root)
	if got := waited(t, <-ch); len(got) != 2 || got[0].ID != waiting || got[1].ID != opened {
		t.Errorf("wait --for human printed %+v, want %s then %s", got, waiting, opened)
	}

	// A note after an answer hands the case back to the human.
	ch = startWait(t, root, "--for", "human", "--id", answered, "--timeout", "5s")
	mustRun(t, "--store", root, "pickup", answered)
	if r := runCases(t, "Which version?\n", "--store", root, "note", answered, "--body-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != answered || got[0].State != "open" {
		t.Errorf("after note: %+v", got)
	}
}

func TestWaitForHumanAmendAndResume(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	// An amend changes a case already waiting on the human; it does not
	// announce it again.
	ch := startWait(t, root, "--for", "human", "--timeout", "300ms")
	mustRun(t, "--store", root, "amend", id, "--context", "Seen on the mirror too.")
	assertTimedOut(t, <-ch)

	// Human events never wake it, and a parked case is not waiting on anyone.
	ch = startWait(t, root, "--for", "human", "--timeout", "300ms")
	mustRun(t, "--store", root, "answer", id, "--park")
	assertTimedOut(t, <-ch)

	// The agent resuming a parked case hands it back to the human.
	ch = startWait(t, root, "--for", "human", "--timeout", "5s")
	mustRun(t, "--store", root, "resume", id, "--agent")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id || got[0].State != "open" {
		t.Errorf("after agent resume: %+v", got)
	}

	// A human resume leaves the case with the agent, until the agent amends it.
	mustRun(t, "--store", root, "answer", id, "--park")
	mustRun(t, "--store", root, "resume", id)
	ch = startWait(t, root, "--for", "human", "--timeout", "5s")
	mustRun(t, "--store", root, "amend", id, "--context", "Tried the mirror.")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id {
		t.Errorf("after human resume and amend: %+v", got)
	}

	if r := runCases(t, "", "--store", root, "wait", "--for", "nobody"); r.err == nil {
		t.Error("bad --for accepted")
	}
	r := runCases(t, "", "--store", root, "wait", "--for", "human", "--id", id, "--timeout", "50ms")
	if !strings.Contains(r.err.Error(), "no case needed the human within 50ms") {
		t.Errorf("timeout message = %q", r.err)
	}
}

func TestWaitSinceCaseID(t *testing.T) {
	root := t.TempDir()
	// The answer lands between open and wait; with the case id as --since,
	// wait still reports it.
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	if got := waited(t, runCases(t, "", "--store", root, "wait", "--id", id, "--since", id, "--timeout", "1s")); len(got) != 1 || got[0].ID != id {
		t.Errorf("wait --since %s printed %+v", id, got)
	}

	// A case opened after the answer is a later baseline.
	later := openDecision(t, root)
	assertTimedOut(t, runCases(t, "", "--store", root, "wait", "--id", id, "--since", later, "--timeout", "100ms"))

	for _, bad := range []string{"2026-09-17T00-00-00Z-no-such-case", "../x"} {
		r := runCases(t, "", "--store", root, "wait", "--since", bad, "--timeout", "100ms")
		var ee *exitError
		if r.err == nil || errors.As(r.err, &ee) {
			t.Errorf("--since %q: err = %v, want an error that exits 1", bad, r.err)
			continue
		}
		if !strings.Contains(r.err.Error(), "neither an RFC 3339 time nor a case") {
			t.Errorf("--since %q: err = %q", bad, r.err)
		}
	}
}

func TestWaitPrintsTheNextSince(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))
	other := openDecision(t, root)
	mustRun(t, "--store", root, "answer", other, "--option", "1")

	ch := startWait(t, root, "--timeout", "5s")
	mustRun(t, "--store", root, "answer", id, "--park")
	got := waited(t, <-ch)
	if len(got) != 2 {
		t.Fatalf("wait printed %+v", got)
	}
	// Every line carries the same value, the time of the latest event that
	// put a printed case there: the park.
	next := got[1].NextSince
	if got[0].NextSince != next || got[1].ID != id {
		t.Fatalf("next_since %q and %q, want both from %s", got[0].NextSince, next, id)
	}
	if at, err := time.Parse(time.RFC3339, next); err != nil || !at.Equal(got[1].UpdatedAt) {
		t.Errorf("next_since = %q (%v), want the park at %s", next, err, got[1].UpdatedAt)
	}

	// Passed back, it does not wake on the park or the older answer, but a
	// resume that lands before the next wait starts does wake it.
	assertTimedOut(t, runCases(t, "", "--store", root, "wait", "--since", next, "--timeout", "100ms"))
	mustRun(t, "--store", root, "resume", id)
	if got := waited(t, runCases(t, "", "--store", root, "wait", "--since", next, "--timeout", "1s")); len(got) != 2 || got[1].ID != id || got[1].State != "open" {
		t.Errorf("after resume: %+v", got)
	}

	// A --since later than every printed event is kept.
	future := time.Now().UTC().Add(time.Hour).Format(time.RFC3339)
	ch = startWait(t, root, "--since", future, "--id", other, "--timeout", "5s")
	mustRun(t, "--store", root, "pickup", other)
	if r := runCases(t, "Which version?\n", "--store", root, "note", other, "--body-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}
	// The note hands the case to the human; answer it again so it wakes.
	mustRun(t, "--store", root, "answer", other, "--option", "2")
	if got := waited(t, <-ch); len(got) != 1 || got[0].NextSince != future {
		t.Errorf("with a later --since: %+v, want next_since %s", got, future)
	}
}
