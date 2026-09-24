package main

import (
	"context"
	"encoding/json"
	"errors"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
	"github.com/ryanlewis/cases/internal/store/storetest"
)

// startWait runs `cases wait` in the background and returns its result
// channel. It sleeps briefly so the first poll, which fixes what counts as
// already there, happens before the caller writes anything.
func startWait(t *testing.T, storePath string, args ...string) <-chan result {
	t.Helper()
	return startWaitWith(t, nil, storePath, args...)
}

// startWaitWith is startWait with cases as the store the command goes
// through, as for runCasesWith.
func startWaitWith(t *testing.T, cases store.Store, storePath string, args ...string) <-chan result {
	t.Helper()
	ch := make(chan result, 1)
	go func() { ch <- runCasesWith(t, cases, "", append([]string{"--store", storePath, "wait"}, args...)...) }()
	time.Sleep(40 * time.Millisecond)
	return ch
}

type waitedCase struct {
	ID        string    `json:"id"`
	Kind      string    `json:"kind"`
	State     string    `json:"state"`
	UpdatedAt time.Time `json:"updated_at"`
	Revision  int       `json:"revision"`
	NextSince string    `json:"next_since"`
	Fresh     *bool     `json:"fresh"`
	Answer    *struct {
		Choice int  `json:"choice"`
		Ack    bool `json:"ack"`
	} `json:"answer"`
	Pickup *struct {
		By string `json:"by"`
	} `json:"pickup"`
}

// isFresh is a line's fresh, failing the test when the line has none.
func isFresh(t *testing.T, c waitedCase) bool {
	t.Helper()
	if c.Fresh == nil {
		t.Fatalf("line for %s has no fresh", c.ID)
	}
	return *c.Fresh
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
	storePath := newStore(t)
	id := openDecision(t, storePath)
	ch := startWait(t, storePath, "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", id, "--option", "2")

	got := waited(t, <-ch)
	if len(got) != 1 || got[0].ID != id || got[0].Kind != "decision" || got[0].State != "answered" || got[0].Answer == nil || got[0].Answer.Choice != 2 || got[0].Revision != 2 {
		t.Errorf("wait printed %+v", got)
	}
}

func TestWaitTimesOutWithExitTwo(t *testing.T) {
	storePath := newStore(t)
	openDecision(t, storePath)
	start := time.Now()
	r := runCases(t, "", "--store", storePath, "wait", "--timeout", "100ms")
	assertTimedOut(t, r)
	if strings.Count(r.err.Error(), "\n") != 0 || !strings.Contains(r.err.Error(), "within 100ms") {
		t.Errorf("stderr line = %q", r.err.Error())
	}
	if elapsed := time.Since(start); elapsed > 2*time.Second {
		t.Errorf("timeout took %s", elapsed)
	}
}

func TestWaitIgnoresEventsFromBeforeItStarted(t *testing.T) {
	storePath := newStore(t)
	old := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", old, "--option", "1")
	assertTimedOut(t, runCases(t, "", "--store", storePath, "wait", "--timeout", "100ms"))

	// Once something new lands, every case still waiting on the agent is
	// printed, the old answer included.
	fresh := openDecision(t, storePath)
	ch := startWait(t, storePath, "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", fresh, "--option", "2")
	got := waited(t, <-ch)
	if len(got) != 2 || got[0].ID != old || got[1].ID != fresh {
		t.Errorf("wait printed %+v, want %s then %s", got, old, fresh)
	}
}

func TestWaitSinceCountsEarlierEvents(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	before := time.Now().UTC().Add(-time.Minute).Format(time.RFC3339)
	mustRun(t, "--store", storePath, "answer", id, "--option", "1")

	if got := waited(t, runCases(t, "", "--store", storePath, "wait", "--since", before, "--timeout", "1s")); len(got) != 1 || got[0].ID != id {
		t.Errorf("wait printed %+v", got)
	}
	later := time.Now().UTC().Add(time.Minute).Format(time.RFC3339)
	assertTimedOut(t, runCases(t, "", "--store", storePath, "wait", "--since", later, "--timeout", "100ms"))
	if r := runCases(t, "", "--store", storePath, "wait", "--since", "yesterday"); r.err == nil {
		t.Error("bad --since accepted")
	}
}

func TestWaitParkAndResume(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	ch := startWait(t, storePath, "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", id, "--park")
	if got := waited(t, <-ch); len(got) != 1 || got[0].State != "parked" {
		t.Errorf("after park: %+v", got)
	}

	ch = startWait(t, storePath, "--timeout", "5s")
	mustRun(t, "--store", storePath, "resume", id)
	if got := waited(t, <-ch); len(got) != 1 || got[0].State != "open" {
		t.Errorf("after human resume: %+v", got)
	}

	// The agent's own events never wake it.
	mustRun(t, "--store", storePath, "answer", id, "--park")
	ch = startWait(t, storePath, "--timeout", "300ms")
	mustRun(t, "--store", storePath, "resume", id, "--agent")
	mustRun(t, "--store", storePath, "withdraw", id)
	assertTimedOut(t, <-ch)
}

func TestWaitOnSpecificCases(t *testing.T) {
	storePath := newStore(t)
	a := openDecision(t, storePath)
	b := openDecision(t, storePath)

	ch := startWait(t, storePath, "--id", b, "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", a, "--option", "1")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", storePath, "answer", b, "--option", "2")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != b {
		t.Errorf("wait --id %s printed %+v", b, got)
	}
	if r := runCases(t, "", "--store", storePath, "wait", "--id", "../x"); r.err == nil {
		t.Error("bad --id accepted")
	}
}

func TestWaitByLabelAndWorker(t *testing.T) {
	storePath := newStore(t)
	open := func(args ...string) string {
		return strings.TrimSpace(mustRun(t, append([]string{"--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Labelled"}, args...)...))
	}
	mine := open("--label", "round-1", "--worker", "w1")
	theirs := open("--label", "round-2", "--worker", "w2")
	otherWorker := open("--label", "round-1", "--worker", "w2")

	// Answers on cases that do not match do not wake it.
	ch := startWait(t, storePath, "--label", "round-1", "--worker", "w1", "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", theirs, "--ack")
	mustRun(t, "--store", storePath, "answer", otherWorker, "--ack")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", storePath, "answer", mine, "--ack")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != mine {
		t.Errorf("wait --label --worker printed %+v, want only %s", got, mine)
	}

	// A filter no case matches waits until the timeout.
	ch = startWait(t, storePath, "--label", "nope", "--timeout", "300ms")
	fresh := open("--label", "round-1")
	mustRun(t, "--store", storePath, "answer", fresh, "--ack")
	assertTimedOut(t, <-ch)
}

func TestWaitByIDAndLabel(t *testing.T) {
	storePath := newStore(t)
	labelled := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "A", "--label", "round-1"))
	unlabelled := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "B"))
	other := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "C", "--label", "round-1"))

	// --id and --label together wait on cases that pass both.
	ch := startWait(t, storePath, "--id", labelled, "--id", unlabelled, "--label", "round-1", "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", unlabelled, "--ack")
	mustRun(t, "--store", storePath, "answer", other, "--ack")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", storePath, "answer", labelled, "--ack")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != labelled {
		t.Errorf("wait --id --label printed %+v, want only %s", got, labelled)
	}
}

func TestWaitForAStoreThatDoesNotExistYet(t *testing.T) {
	storePath := filepath.Join(t.TempDir(), "not-yet", "cases.db")
	ch := startWait(t, storePath, "--timeout", "5s")
	id := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", id, "--option", "1")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id {
		t.Errorf("wait printed %+v", got)
	}
}

func TestWaitReportsAnAnswerWithoutTimestamp(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Hand answered"))
	ch := startWait(t, storePath, "--timeout", "5s")
	// An answer written by hand or by another tool may leave answered_at
	// out; it still counts because it is new to the store.
	storetest.InsertEvent(t, storePath, id, 2, "human", "answer", `{"ack":true}`)
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id || got[0].Answer == nil || !got[0].Answer.Ack {
		t.Errorf("wait printed %+v", got)
	}
}

func TestWaitWarnsOnceAboutAMalformedAnswer(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	ch := startWait(t, storePath, "--timeout", "300ms")
	// The answer is skipped when the case loads, so the case never needs the
	// agent; the warning is the only sign that anything arrived.
	storetest.InsertEvent(t, storePath, id, 2, "human", "answer", `{"choice": `)
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
	storePath := newStore(t)
	answered := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", answered, "--option", "1")
	waiting := openDecision(t, storePath)

	// A case already waiting on the human when wait starts does not wake it.
	assertTimedOut(t, runCases(t, "", "--store", storePath, "wait", "--for", "human", "--timeout", "100ms"))

	// A new case does, and every case waiting on the human is printed; the
	// answered one is not.
	ch := startWait(t, storePath, "--for", "human", "--timeout", "5s")
	opened := openDecision(t, storePath)
	if got := waited(t, <-ch); len(got) != 2 || got[0].ID != waiting || got[1].ID != opened {
		t.Errorf("wait --for human printed %+v, want %s then %s", got, waiting, opened)
	}

	// A note after an answer hands the case back to the human.
	ch = startWait(t, storePath, "--for", "human", "--id", answered, "--timeout", "5s")
	mustRun(t, "--store", storePath, "pickup", answered)
	if r := runCases(t, "Which version?\n", "--store", storePath, "note", answered, "--body-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != answered || got[0].State != "open" {
		t.Errorf("after note: %+v", got)
	}
}

func TestWaitForHumanAmendAndResume(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	// An amend changes a case already waiting on the human; it does not
	// announce it again.
	ch := startWait(t, storePath, "--for", "human", "--timeout", "300ms")
	mustRun(t, "--store", storePath, "amend", id, "--context", "Seen on the mirror too.")
	assertTimedOut(t, <-ch)

	// Human events never wake it, and a parked case is not waiting on anyone.
	ch = startWait(t, storePath, "--for", "human", "--timeout", "300ms")
	mustRun(t, "--store", storePath, "answer", id, "--park")
	assertTimedOut(t, <-ch)

	// The agent resuming a parked case hands it back to the human.
	ch = startWait(t, storePath, "--for", "human", "--timeout", "5s")
	mustRun(t, "--store", storePath, "resume", id, "--agent")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id || got[0].State != "open" {
		t.Errorf("after agent resume: %+v", got)
	}

	// A human resume leaves the case with the agent, until the agent amends it.
	mustRun(t, "--store", storePath, "answer", id, "--park")
	mustRun(t, "--store", storePath, "resume", id)
	ch = startWait(t, storePath, "--for", "human", "--timeout", "5s")
	mustRun(t, "--store", storePath, "amend", id, "--context", "Tried the mirror.")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != id {
		t.Errorf("after human resume and amend: %+v", got)
	}

	if r := runCases(t, "", "--store", storePath, "wait", "--for", "nobody"); r.err == nil {
		t.Error("bad --for accepted")
	}
	r := runCases(t, "", "--store", storePath, "wait", "--for", "human", "--id", id, "--timeout", "50ms")
	if !strings.Contains(r.err.Error(), "no case needed the human within 50ms") {
		t.Errorf("timeout message = %q", r.err)
	}
}

func TestWaitSinceCaseID(t *testing.T) {
	storePath := newStore(t)
	// The answer lands between open and wait; with the case id as --since,
	// wait still reports it.
	id := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", id, "--option", "1")
	if got := waited(t, runCases(t, "", "--store", storePath, "wait", "--id", id, "--since", id, "--timeout", "1s")); len(got) != 1 || got[0].ID != id {
		t.Errorf("wait --since %s printed %+v", id, got)
	}

	// A case opened after the answer is a later baseline.
	later := openDecision(t, storePath)
	assertTimedOut(t, runCases(t, "", "--store", storePath, "wait", "--id", id, "--since", later, "--timeout", "100ms"))

	for _, bad := range []string{"2026-09-17T00-00-00Z-no-such-case", "../x"} {
		r := runCases(t, "", "--store", storePath, "wait", "--since", bad, "--timeout", "100ms")
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
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))
	other := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", other, "--option", "1")

	ch := startWait(t, storePath, "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", id, "--park")
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
	assertTimedOut(t, runCases(t, "", "--store", storePath, "wait", "--since", next, "--timeout", "100ms"))
	mustRun(t, "--store", storePath, "resume", id)
	if got := waited(t, runCases(t, "", "--store", storePath, "wait", "--since", next, "--timeout", "1s")); len(got) != 2 || got[1].ID != id || got[1].State != "open" {
		t.Errorf("after resume: %+v", got)
	}

	// A --since later than every printed event is kept.
	future := time.Now().UTC().Add(time.Hour).Format(time.RFC3339)
	ch = startWait(t, storePath, "--since", future, "--id", other, "--timeout", "5s")
	mustRun(t, "--store", storePath, "pickup", other)
	if r := runCases(t, "Which version?\n", "--store", storePath, "note", other, "--body-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}
	// The note hands the case to the human; answer it again so it wakes.
	mustRun(t, "--store", storePath, "answer", other, "--option", "2")
	if got := waited(t, <-ch); len(got) != 1 || got[0].NextSince != future {
		t.Errorf("with a later --since: %+v, want next_since %s", got, future)
	}
}

func TestWaitMarksWhichCasesAreFresh(t *testing.T) {
	storePath := newStore(t)
	old := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", old, "--option", "1")
	late := openDecision(t, storePath)
	earlier := time.Now().UTC().Format(time.RFC3339Nano)
	time.Sleep(10 * time.Millisecond)
	mustRun(t, "--store", storePath, "answer", late, "--option", "1")
	idle := openDecision(t, storePath)

	// Without --since only the event that landed while waiting is fresh.
	ch := startWait(t, storePath, "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", idle, "--option", "2")
	got := waited(t, <-ch)
	if len(got) != 3 || got[0].ID != old || isFresh(t, got[0]) || got[1].ID != late || isFresh(t, got[1]) || got[2].ID != idle || !isFresh(t, got[2]) {
		t.Errorf("wait printed %+v, want only %s fresh", got, idle)
	}

	// With --since, an event already in the store and later than it is fresh
	// too.
	got = waited(t, runCases(t, "", "--store", storePath, "wait", "--since", earlier, "--timeout", "1s"))
	if len(got) != 3 || isFresh(t, got[0]) || !isFresh(t, got[1]) || !isFresh(t, got[2]) {
		t.Errorf("wait --since printed %+v, want %s and %s fresh", got, late, idle)
	}
}

// jsonObject decodes one JSON object, for comparing what two commands print.
func jsonObject(t *testing.T, s string) map[string]any {
	t.Helper()
	var m map[string]any
	if err := json.Unmarshal([]byte(s), &m); err != nil {
		t.Fatalf("%q: %v", s, err)
	}
	return m
}

func TestWaitPickupPicksUpTheAnswer(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "decision", "--urgency", "today", "--worker", "bun-pins",
		"--title", "Pin bun?", "--option", "Pin to 1.2.3", "--option", "Float"))
	ch := startWait(t, storePath, "--id", id, "--since", id, "--pickup", "--by", "bun-pins", "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", id, "--option", "1")

	r := <-ch
	got := waited(t, r)
	if len(got) != 1 || got[0].State != "pickedup" || got[0].Answer == nil || got[0].Answer.Choice != 1 ||
		got[0].Pickup == nil || got[0].Pickup.By != "bun-pins" || got[0].Revision != 3 || !isFresh(t, got[0]) {
		t.Fatalf("wait --pickup printed %+v", got)
	}
	if r.stderr != "" {
		t.Errorf("stderr = %q", r.stderr)
	}

	// The pickup is the one cases pickup writes, the worker as its actor.
	if names := eventFiles(t, storePath, id); !slices.Equal(names, []string{"0001-agent-open.json", "0002-human-answer.json", "0003-agent-pickup.json"}) {
		t.Errorf("events = %v", names)
	}
	c := loadCase(t, storePath, id)
	if c.Pickup == nil || c.Pickup.By != "bun-pins" || c.Events[2].Actor == nil || c.Events[2].Actor.Name != "bun-pins" {
		t.Errorf("pickup = %+v, actor %+v", c.Pickup, c.Events[2].Actor)
	}

	// The line is the case after the pickup, as show --json prints it without
	// its url, and its revision is the one close takes.
	line := jsonObject(t, r.stdout)
	delete(line, "fresh")
	delete(line, "next_since")
	shown := jsonObject(t, mustRun(t, "--store", storePath, "show", id, "--json"))
	delete(shown, "url")
	if !reflect.DeepEqual(line, shown) {
		t.Errorf("wait line %v\ndiffers from show --json %v", line, shown)
	}
	if out := mustRun(t, "--store", storePath, "close", id, "--outcome", "Pinned.", "--revision", "3"); out != id+" closed\n" {
		t.Errorf("close = %q", out)
	}
}

func TestWaitPickupLeavesParkedAndResumedCases(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))

	// Each wait passes --since, so the park and the resume wake it whether
	// they land before or after its first poll. An empty stderr shows wait
	// did not try to pick up a case that has no answer.
	ch := startWait(t, storePath, "--id", id, "--since", id, "--pickup", "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", id, "--park")
	r := <-ch
	got := waited(t, r)
	if len(got) != 1 || got[0].State != "parked" || got[0].Pickup != nil || got[0].Revision != 2 || r.stderr != "" {
		t.Fatalf("after park: %+v, stderr %q", got, r.stderr)
	}

	ch = startWait(t, storePath, "--id", id, "--since", got[0].NextSince, "--pickup", "--timeout", "5s")
	mustRun(t, "--store", storePath, "resume", id)
	r = <-ch
	if got := waited(t, r); len(got) != 1 || got[0].State != "open" || got[0].Pickup != nil || got[0].Revision != 3 || r.stderr != "" {
		t.Errorf("after resume: %+v, stderr %q", got, r.stderr)
	}
	if names := eventFiles(t, storePath, id); !slices.Equal(names, []string{"0001-agent-open.json", "0002-human-park.json", "0003-human-resume.json"}) {
		t.Errorf("events = %v", names)
	}
}

// A case with an event written after wait read it is not picked up: wait
// prints it as it read it, still answered at the revision it read, and says
// on stderr why and what state the case is in now. The other cases it prints
// are picked up as usual.
func TestWaitPickupSkipsACaseChangedSinceItWasRead(t *testing.T) {
	note := func(ctx context.Context, s store.Store, id string) error {
		_, err := s.Note(ctx, id, store.NoteRecord{Body: "1.2.3 has a CVE. Pin to which patch?"})
		return err
	}
	for _, tt := range []struct {
		name   string
		change func(ctx context.Context, s store.Store, id string) error
		want   []string
		now    store.State
	}{
		{"noted", note, []string{"0001-agent-open.json", "0002-human-answer.json", "0003-agent-note.json"}, store.StateOpen},
		{"noted and answered again", func(ctx context.Context, s store.Store, id string) error {
			if err := note(ctx, s, id); err != nil {
				return err
			}
			_, err := s.Answer(ctx, id, store.AnswerRecord{Choice: 2})
			return err
		}, []string{"0001-agent-open.json", "0002-human-answer.json", "0003-agent-note.json", "0004-human-answer.json"}, store.StateAnswered},
		{"picked up by another wait", func(ctx context.Context, s store.Store, id string) error {
			_, err := s.Pickup(ctx, id, store.PickupRecord{By: "other"})
			return err
		}, []string{"0001-agent-open.json", "0002-human-answer.json", "0003-agent-pickup.json"}, store.StatePickedUp},
	} {
		t.Run(tt.name, func(t *testing.T) {
			storePath := newStore(t)
			changed := openDecision(t, storePath)
			fine := openDecision(t, storePath)
			mustRun(t, "--store", storePath, "answer", changed, "--option", "1")
			mustRun(t, "--store", storePath, "answer", fine, "--option", "2")

			r := runCasesWith(t, changeFirst{openStore(t, storePath), changed, tt.change}, "", "--store", storePath,
				"wait", "--id", changed, "--id", fine, "--since", changed, "--pickup", "--by", "me", "--timeout", "5s")
			got := waited(t, r)
			if len(got) != 2 || got[0].ID != changed || got[1].ID != fine {
				t.Fatalf("wait printed %+v", got)
			}
			if c := got[0]; c.State != "answered" || c.Revision != 2 || c.Pickup != nil || c.Answer == nil || c.Answer.Choice != 1 {
				t.Errorf("changed case printed as %+v, want it as read: answered at revision 2", c)
			}
			if c := got[1]; c.State != "pickedup" || c.Revision != 3 || c.Pickup == nil || c.Pickup.By != "me" {
				t.Errorf("other case printed as %+v, want it picked up", c)
			}
			want := "warning: " + changed + ": not picked up: " + store.ErrStale.Error() + ": read at revision 2, now at "
			if !strings.HasPrefix(r.stderr, want) || !strings.HasSuffix(r.stderr, "; it is now "+string(tt.now)+"\n") || strings.Count(r.stderr, "\n") != 1 {
				t.Errorf("stderr = %q, want one line starting %q and naming the state it is now in, %s", r.stderr, want, tt.now)
			}
			if names := eventFiles(t, storePath, changed); !slices.Equal(names, tt.want) {
				t.Errorf("changed case events = %v, want %v", names, tt.want)
			}
			if c := loadCase(t, storePath, fine); c.State != store.StatePickedUp || c.Revision() != 3 {
				t.Errorf("other case is %s at revision %d", c.State, c.Revision())
			}
		})
	}
}

// pickupBarrier holds each pickup until every wait sharing it has asked for
// one, so that they all read the case before any of them picks it up.
type pickupBarrier struct {
	store.Store
	arrived *sync.WaitGroup
}

func (b pickupBarrier) Pickup(ctx context.Context, id string, rec store.PickupRecord, pre ...store.Precondition) (*store.Case, error) {
	b.arrived.Done()
	all := make(chan struct{})
	go func() { b.arrived.Wait(); close(all) }()
	select {
	case <-all:
	case <-time.After(5 * time.Second):
	}
	return b.Store.Pickup(ctx, id, rec, pre...)
}

// Two waits that read the same answer both try to pick it up at the revision
// they read. The store lets the first through and refuses the second, so the
// answer is picked up once and only one wait says it picked it up.
func TestWaitPickupByTwoWaitsPicksUpOnce(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	var arrived sync.WaitGroup
	arrived.Add(2)
	// --since makes the answer wake both waits, whether it lands before or
	// after their first poll.
	a := startWaitWith(t, pickupBarrier{openStore(t, storePath), &arrived}, storePath, "--id", id, "--since", id, "--pickup", "--by", "a", "--timeout", "10s")
	b := startWaitWith(t, pickupBarrier{openStore(t, storePath), &arrived}, storePath, "--id", id, "--since", id, "--pickup", "--by", "b", "--timeout", "10s")
	mustRun(t, "--store", storePath, "answer", id, "--option", "1")

	var picked []string
	for _, r := range []result{<-a, <-b} {
		got := waited(t, r)
		if len(got) != 1 {
			t.Fatalf("wait printed %+v", got)
		}
		switch c := got[0]; {
		case c.State == "pickedup" && c.Pickup != nil && c.Revision == 3 && r.stderr == "":
			picked = append(picked, c.Pickup.By)
		case c.State == "answered" && c.Pickup == nil && c.Revision == 2 && strings.Contains(r.stderr, "read at revision 2, now at 3; it is now pickedup"):
		default:
			t.Errorf("wait printed %+v, stderr %q", c, r.stderr)
		}
	}
	if len(picked) != 1 {
		t.Fatalf("picked up by %v, want one wait", picked)
	}
	if names := eventFiles(t, storePath, id); !slices.Equal(names, []string{"0001-agent-open.json", "0002-human-answer.json", "0003-agent-pickup.json"}) {
		t.Errorf("events = %v", names)
	}
	if c := loadCase(t, storePath, id); c.Pickup.By != picked[0] {
		t.Errorf("pickup by %q, but the wait that said so was %q", c.Pickup.By, picked[0])
	}
}

// A wait by label picks up every answered case it prints, those already
// waiting before its --since included, and leaves parked cases and cases with
// other labels alone.
func TestWaitPickupByLabel(t *testing.T) {
	storePath := newStore(t)
	open := func(args ...string) string {
		return strings.TrimSpace(mustRun(t, append([]string{"--store", storePath, "open", "--urgency", "today"}, args...)...))
	}
	early := open("--kind", "fyi", "--title", "Early", "--label", "round-1")
	parked := open("--kind", "stuck", "--title", "Blocked", "--label", "round-1")
	late := open("--kind", "fyi", "--title", "Late", "--label", "round-1")
	theirs := open("--kind", "fyi", "--title", "Theirs", "--label", "round-2")
	mustRun(t, "--store", storePath, "answer", early, "--ack")
	mustRun(t, "--store", storePath, "answer", parked, "--park")
	// A --since after the answer and the park, so that those two are already
	// waiting, and the answer to late is new, whenever the first poll runs.
	since := time.Now().UTC().Format(time.RFC3339Nano)

	ch := startWait(t, storePath, "--label", "round-1", "--since", since, "--pickup", "--by", "w1", "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", theirs, "--ack")
	mustRun(t, "--store", storePath, "answer", late, "--ack")
	r := <-ch
	got := waited(t, r)
	if len(got) != 3 || got[0].ID != early || got[1].ID != parked || got[2].ID != late {
		t.Fatalf("wait printed %+v, want %s, %s and %s", got, early, parked, late)
	}
	if r.stderr != "" {
		t.Errorf("stderr = %q, want nothing: the parked case is not tried", r.stderr)
	}
	for i, want := range []struct {
		state string
		fresh bool
	}{{"pickedup", false}, {"parked", false}, {"pickedup", true}} {
		if c := got[i]; c.State != want.state || isFresh(t, c) != want.fresh || (c.Pickup != nil) != (want.state == "pickedup") {
			t.Errorf("line %d = %+v, want %s with fresh %v", i, c, want.state, want.fresh)
		}
	}
	for id, want := range map[string]store.State{early: store.StatePickedUp, parked: store.StateParked, late: store.StatePickedUp, theirs: store.StateAnswered} {
		if c := loadCase(t, storePath, id); c.State != want {
			t.Errorf("%s is %s, want %s", id, c.State, want)
		}
	}
}

func TestWaitPickupFlags(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", id, "--option", "1")
	for _, tt := range []struct {
		args []string
		want string
	}{
		{[]string{"--id", id, "--by", "me"}, "--by needs --pickup"},
		{[]string{"--pickup", "--for", "human", "--id", id}, "--pickup is only for --for agent"},
		{[]string{"--pickup"}, "--pickup needs --id, --label or --worker"},
		{[]string{"--pickup", "--kind", "decision"}, "--pickup needs --id, --label or --worker"},
		// A blank worker, as "$CASES_WORKER" gives when it is not set, would
		// match every case opened without one, id's included.
		{[]string{"--pickup", "--worker", ""}, "--pickup refuses a blank --worker"},
		{[]string{"--pickup", "--id", id, "--worker", " "}, "--pickup refuses a blank --worker"},
	} {
		r := runCases(t, "", append([]string{"--store", storePath, "wait", "--since", id, "--timeout", "1s"}, tt.args...)...)
		var ee *exitError
		if r.err == nil || errors.As(r.err, &ee) || !strings.Contains(r.err.Error(), tt.want) || r.stdout != "" {
			t.Errorf("wait %v: err = %v, stdout %q; want an error saying %q", tt.args, r.err, r.stdout, tt.want)
		}
	}
	if c := loadCase(t, storePath, id); c.State != store.StateAnswered {
		t.Errorf("a refused wait --pickup left the case %s", c.State)
	}

	// A worker is enough to scope it, and --by can be left out, as for pickup.
	mine := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Mine", "--worker", "w1"))
	mustRun(t, "--store", storePath, "answer", mine, "--ack")
	got := waited(t, runCases(t, "", "--store", storePath, "wait", "--since", id, "--worker", "w1", "--pickup", "--timeout", "1s"))
	if len(got) != 1 || got[0].ID != mine || got[0].State != "pickedup" || got[0].Pickup == nil || got[0].Pickup.By != "" {
		t.Errorf("wait --worker --pickup printed %+v", got)
	}
	if c := loadCase(t, storePath, id); c.State != store.StateAnswered {
		t.Errorf("wait --worker w1 --pickup left the case with no worker %s", c.State)
	}
}
