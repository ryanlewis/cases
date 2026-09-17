package store

import (
	"bytes"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"
)

// fixClock pins the store clock for one test.
func fixClock(t *testing.T, at time.Time) {
	t.Helper()
	orig := now
	now = func() time.Time { return at }
	t.Cleanup(func() { now = orig })
}

// failCommit makes every write's commit fail, without committing, for one
// test.
func failCommit(t *testing.T) {
	t.Helper()
	orig := commit
	commit = func(*sql.Tx) error { return errors.New("disk on fire") }
	t.Cleanup(func() { commit = orig })
}

func TestCreateWritesOpenEvent(t *testing.T) {
	d := newDB(t)
	fixClock(t, time.Date(2026, 9, 15, 9, 12, 3, 0, time.UTC))

	c, err := d.Create(t.Context(), OpenRecord{Kind: KindFYI, Urgency: UrgencyWhenever, Title: "Pin bun or float?"})
	if err != nil {
		t.Fatal(err)
	}
	if c.ID != "2026-09-15T09-12-03Z-pin-bun-or-float" || c.Revision() != 1 || c.Events[0].File != "0001-agent-open.json" {
		t.Errorf("case = %+v", c)
	}
	if got := names(storedRows(t, d, c.ID)); !slices.Equal(got, []string{"0001-agent-open.json"}) {
		t.Errorf("rows = %v", got)
	}
	loaded, err := d.Get(t.Context(), c.ID)
	if err != nil {
		t.Fatal(err)
	}
	if loaded.State != StateOpen || loaded.Title != "Pin bun or float?" || !loaded.OpenedAt.Equal(now()) {
		t.Errorf("loaded = %+v", loaded)
	}
	var openedAt string
	if err := pool(t, d).QueryRowContext(t.Context(), `SELECT opened_at FROM cases WHERE id = ?`, c.ID).Scan(&openedAt); err != nil || openedAt != "2026-09-15T09:12:03Z" {
		t.Errorf("opened_at = %q, %v", openedAt, err)
	}

	// Same second, same title: a second case, not a clash.
	c2, err := d.Create(t.Context(), OpenRecord{Kind: KindFYI, Urgency: UrgencyWhenever, Title: "Pin bun or float?"})
	if err != nil {
		t.Fatal(err)
	}
	if c2.ID != c.ID+"-2" {
		t.Errorf("second id = %s", c2.ID)
	}
}

func TestCreateWritesLabels(t *testing.T) {
	d := newDB(t)
	rec := openOf(KindFYI)
	rec.Labels = []string{"feat-labels", "round 3"}
	c, err := d.Create(t.Context(), rec)
	if err != nil {
		t.Fatal(err)
	}
	raw := storedRows(t, d, c.ID)[0].Data
	var got struct{ Labels []string }
	if err := json.Unmarshal([]byte(raw), &got); err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(got.Labels, rec.Labels) {
		t.Errorf("open record = %s", raw)
	}
	loaded, err := d.Get(t.Context(), c.ID)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(loaded.Labels, rec.Labels) {
		t.Errorf("labels = %q", loaded.Labels)
	}
}

// An open record written before labels existed has no labels key and still
// loads, with no labels.
func TestOpenRecordWithoutLabelsLoads(t *testing.T) {
	d := newDB(t)
	if _, err := d.Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	const id = "2026-09-01T10-00-00Z-old-case"
	insertRow(t, d, id, 1, "agent", "open", `{"kind":"fyi","urgency":"whenever","title":"Old case","worker":"w1","brief":"BRIEF.md","opened_at":"2026-09-01T10:00:00Z"}`)
	c, err := d.Get(t.Context(), id)
	if err != nil {
		t.Fatal(err)
	}
	if c.State != StateOpen || c.Labels != nil || c.Worker != "w1" || len(c.Problems) != 0 {
		t.Errorf("case = %+v, problems %q", c.OpenRecord, c.Problems)
	}
	raw, err := json.Marshal(c)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), `"labels"`) || strings.Contains(string(raw), `"dir"`) {
		t.Errorf("json = %s", raw)
	}
}

func TestCreateRefusesInvalidOpenWithoutMakingTheStore(t *testing.T) {
	d := newDB(t)
	if _, err := d.Create(t.Context(), OpenRecord{Kind: KindDecision, Urgency: UrgencyToday, Title: "No options"}); err == nil {
		t.Fatal("want an error")
	}
	if _, err := os.Stat(d.Path); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("store after a refused open: %v", err)
	}
}

// Create adds the case before its open event. When the event is refused, or
// the commit fails, the transaction takes the case back, and its id is free
// for the next Create.
func TestCreateRollsBackTheCase(t *testing.T) {
	fixClock(t, time.Date(2026, 9, 15, 9, 12, 3, 0, time.UTC))
	d := newDB(t)
	if _, err := d.Create(t.Context(), openOf(KindDecision)); err != nil {
		t.Fatal(err)
	}
	blankActor := openOf(KindFYI)
	blankActor.Actor = &Actor{Name: " ", Kind: "agent"}
	// The record validates; only the fold refuses the actor.
	if err := blankActor.validate(); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Create(t.Context(), blankActor); err == nil || err.Error() != "actor name is empty" {
		t.Fatalf("err = %v", err)
	}
	t.Run("failed commit", func(t *testing.T) {
		failCommit(t)
		if _, err := d.Create(t.Context(), openOf(KindFYI)); err == nil || err.Error() != "disk on fire" {
			t.Fatalf("err = %v", err)
		}
	})
	ids, err := d.IDs(t.Context())
	if err != nil || len(ids) != 1 {
		t.Fatalf("ids = %v, %v; want only the first case", ids, err)
	}
	c, err := d.Create(t.Context(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	if c.ID != "2026-09-15T09-12-03Z-a-fyi-case" {
		t.Errorf("id = %s, want the id the refused creates did not keep", c.ID)
	}
}

// A write whose commit fails leaves the case as it was and releases the
// write lock, and the next write takes the number it did not keep.
func TestAFailedCommitWritesNothing(t *testing.T) {
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	t.Run("failed commit", func(t *testing.T) {
		failCommit(t)
		if _, err := d.Answer(t.Context(), c.ID, AnswerRecord{Choice: 1}); err == nil || err.Error() != "disk on fire" {
			t.Fatalf("err = %v", err)
		}
	})
	if got := names(storedRows(t, d, c.ID)); !slices.Equal(got, []string{"0001-agent-open.json"}) {
		t.Fatalf("rows after a failed commit = %v", got)
	}
	// Another connection can write, so the lock was let go.
	answered, err := openDB(t, d.Path).Answer(t.Context(), c.ID, AnswerRecord{Choice: 1})
	if err != nil {
		t.Fatal(err)
	}
	if last := answered.Events[len(answered.Events)-1]; last.File != "0002-human-answer.json" || answered.Revision() != 2 {
		t.Errorf("next write = %s at revision %d", last.File, answered.Revision())
	}
}

func TestSlug(t *testing.T) {
	tests := map[string]string{
		"Pin bun or float?":       "pin-bun-or-float",
		"  Bun pins -- 1.2.3  ":   "bun-pins-1-2-3",
		"ÉTÉ café":                "t-caf",
		"!!!":                     "case",
		"":                        "case",
		strings.Repeat("ab ", 40): strings.TrimSuffix(strings.Repeat("ab-", 16), "-"),
	}
	for in, want := range tests {
		if got := Slug(in); got != want {
			t.Errorf("Slug(%q) = %q, want %q", in, got, want)
		}
		if got := Slug(in); len(got) > maxSlug {
			t.Errorf("Slug(%q) is %d long", in, len(got))
		}
	}
}

func TestValidIDRejectsPaths(t *testing.T) {
	for _, id := range []string{"", ".", "..", "../x", "a/b", `a\b`, ".hidden"} {
		if err := ValidID(id); err == nil {
			t.Errorf("ValidID(%q) accepted", id)
		}
	}
	if err := ValidID("2026-09-15T09-12-03Z-x"); err != nil {
		t.Errorf("err = %v", err)
	}
}

func TestAppendNumbersEventsInOrder(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	c, err := d.Create(ctx, openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	steps := []func() (*Case, error){
		func() (*Case, error) { return d.Answer(ctx, c.ID, AnswerRecord{Choice: 1}) },
		func() (*Case, error) { return d.Pickup(ctx, c.ID, PickupRecord{By: "mgr"}) },
		func() (*Case, error) { return d.Note(ctx, c.ID, NoteRecord{Body: "and?"}) },
		func() (*Case, error) { return d.Answer(ctx, c.ID, AnswerRecord{Other: true, Note: "neither"}) },
		func() (*Case, error) { return d.Pickup(ctx, c.ID, PickupRecord{}) },
		func() (*Case, error) {
			return d.Close(ctx, c.ID, CloseRecord{Outcome: "went with neither", Links: []string{"https://x"}})
		},
	}
	for i, s := range steps {
		if _, err := s(); err != nil {
			t.Fatalf("step %d: %v", i, err)
		}
	}
	want := []string{
		"0001-agent-open.json",
		"0002-human-answer.json",
		"0003-agent-pickup.json",
		"0004-agent-note.json",
		"0005-human-answer.json",
		"0006-agent-pickup.json",
		"0007-agent-close.json",
	}
	if got := names(storedRows(t, d, c.ID)); !slices.Equal(got, want) {
		t.Errorf("rows = %v", got)
	}
	loaded, err := d.Get(ctx, c.ID)
	if err != nil {
		t.Fatal(err)
	}
	if loaded.State != StateClosed || !loaded.Answer.Other || loaded.Close.Outcome != "went with neither" {
		t.Errorf("loaded = %+v", loaded)
	}
}

func TestInvalidTransitionIsNotWritten(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	c, err := d.Create(ctx, openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	checks := []struct {
		name string
		call func() (*Case, error)
	}{
		{"pickup an open case", func() (*Case, error) { return d.Pickup(ctx, c.ID, PickupRecord{}) }},
		{"close an open case", func() (*Case, error) { return d.Close(ctx, c.ID, CloseRecord{Outcome: "x"}) }},
		{"park a decision", func() (*Case, error) { return d.Park(ctx, c.ID, ParkRecord{}) }},
		{"resume an open case", func() (*Case, error) { return d.Resume(ctx, c.ID, AuthorHuman, ResumeRecord{}) }},
		{"answer with a bad choice", func() (*Case, error) { return d.Answer(ctx, c.ID, AnswerRecord{Choice: 9}) }},
		{"resume by an unknown author", func() (*Case, error) { return d.Resume(ctx, c.ID, "robot", ResumeRecord{}) }},
		{"amend with nothing", func() (*Case, error) { return d.Amend(ctx, c.ID, AmendRecord{}) }},
		{"amend a decision with rows", func() (*Case, error) {
			return d.Amend(ctx, c.ID, AmendRecord{Rows: openOf(KindApproval).Rows})
		}},
	}
	for _, ch := range checks {
		if _, err := ch.call(); err == nil {
			t.Errorf("%s: accepted", ch.name)
		}
	}
	if got := names(storedRows(t, d, c.ID)); !slices.Equal(got, []string{"0001-agent-open.json"}) {
		t.Errorf("rows after refused writes = %v", got)
	}
}

func TestAmend(t *testing.T) {
	fixClock(t, time.Date(2026, 9, 16, 11, 0, 0, 0, time.UTC))
	d := newDB(t)
	ctx := t.Context()
	c, err := d.Create(ctx, openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	opened := storedRows(t, d, c.ID)[0].Data

	amended, err := d.Amend(ctx, c.ID, AmendRecord{Options: []string{"Vendor it"}, Links: []string{"https://example.com/log"}})
	if err != nil {
		t.Fatal(err)
	}
	if amended.State != StateOpen || !slices.Equal(amended.Options, []string{"Pin", "Float", "Vendor it"}) {
		t.Errorf("amended = %+v", amended)
	}
	rows := storedRows(t, d, c.ID)
	if got := names(rows); !slices.Equal(got, []string{"0001-agent-open.json", "0002-agent-amend.json"}) {
		t.Errorf("rows = %v", got)
	}
	if rows[0].Data != opened {
		t.Errorf("open event changed from %s to %s", opened, rows[0].Data)
	}
	// Only the fields the amend sets are written.
	var got map[string]any
	if err := json.Unmarshal([]byte(rows[1].Data), &got); err != nil {
		t.Fatal(err)
	}
	if len(got) != 3 || got["amended_at"] != "2026-09-16T11:00:00Z" || got["options"] == nil || got["links"] == nil {
		t.Errorf("record = %v", got)
	}
	loaded, err := d.Get(ctx, c.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Options) != 3 || len(loaded.Links) != 1 || len(loaded.Problems) != 0 {
		t.Errorf("loaded = %+v, problems %v", loaded.OpenRecord, loaded.Problems)
	}
}

func TestAppendToMissingCase(t *testing.T) {
	d := newDB(t)
	const id = "2026-01-01T00-00-00Z-nope"
	if _, err := d.Pickup(t.Context(), id, PickupRecord{}); !errors.Is(err, fs.ErrNotExist) || !strings.HasPrefix(err.Error(), "no store at ") {
		t.Errorf("no store: err = %v, want not exist", err)
	}
	if _, err := d.Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Pickup(t.Context(), id, PickupRecord{}); !errors.Is(err, fs.ErrNotExist) || err.Error() != `no case "`+id+`"` {
		t.Errorf("no case: err = %v, want not exist", err)
	}
}

func TestConcurrentAppendsTakeDistinctSequenceNumbers(t *testing.T) {
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	const n = 20
	var wg sync.WaitGroup
	errs := make(chan error, n)
	for range n {
		wg.Go(func() {
			if _, err := d.Note(t.Context(), c.ID, NoteRecord{Body: "more"}); err != nil {
				errs <- err
			}
		})
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Error(err)
	}
	checkSequence(t, d, c.ID, n+1)
}

// checkSequence checks that the case's events are numbered 1 to n with no gap
// and no number used twice, and that they all fold. It logs how often the
// writer changed from one note to the next, which shows the writers raced.
func checkSequence(t *testing.T, d *DB, id string, n int) {
	t.Helper()
	rows := storedRows(t, d, id)
	var seqs []int
	switches, last := 0, ""
	for _, r := range rows {
		seqs = append(seqs, r.Seq)
		if writer, _, ok := strings.Cut(r.Data, ", note"); ok {
			if last != "" && writer != last {
				switches++
			}
			last = writer
		}
	}
	t.Logf("the writer changed %d times in %d events", switches, len(rows))
	want := make([]int, n)
	for i := range want {
		want[i] = i + 1
	}
	if !slices.Equal(seqs, want) {
		t.Errorf("sequence numbers = %v, want 1 to %d", seqs, n)
	}
	loaded, err := d.Get(t.Context(), id)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Events) != n || loaded.Revision() != n || len(loaded.Problems) != 0 {
		t.Errorf("events = %d, revision %d, problems = %v", len(loaded.Events), loaded.Revision(), loaded.Problems)
	}
}

// Two writers with their own connections to one file, as two commands on one
// store are, take turns: each reads the case the other left, so the events
// are numbered 1 to n with no gap and none twice, and no write fails.
func TestTwoWritersOnOneFileSerialise(t *testing.T) {
	first := newDB(t)
	second := openDB(t, first.Path)
	c, err := first.Create(t.Context(), openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	const n = 25
	var wg sync.WaitGroup
	errs := make(chan error, 2*n)
	start := make(chan struct{})
	for i, d := range []*DB{first, second} {
		wg.Go(func() {
			<-start
			for j := range n {
				if _, err := d.Note(t.Context(), c.ID, NoteRecord{Body: fmt.Sprintf("writer %d, note %d", i, j)}); err != nil {
					errs <- err
				}
			}
		})
	}
	close(start)
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Error(err)
	}
	checkSequence(t, first, c.ID, 2*n+1)
}

// writerEnv hands TestWriterProcess its store, case and count.
const writerEnv = "CASES_STORE_TEST_WRITER"

// The same race between two processes, which is what the file's locks are
// for: each process has its own connections and its own view of the locks.
func TestTwoProcessesOnOneFileSerialise(t *testing.T) {
	if testing.Short() {
		t.Skip("starts two processes")
	}
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	const n = 20
	start := filepath.Join(t.TempDir(), "start")
	var cmds []*exec.Cmd
	var outs []*bytes.Buffer
	for range 2 {
		cmd := exec.CommandContext(t.Context(), os.Args[0], "-test.run=^TestWriterProcess$", "-test.count=1")
		cmd.Env = append(os.Environ(), writerEnv+"="+strings.Join([]string{d.Path, c.ID, strconv.Itoa(n), start}, "\n"))
		out := &bytes.Buffer{}
		cmd.Stdout, cmd.Stderr = out, out
		if err := cmd.Start(); err != nil {
			t.Fatal(err)
		}
		cmds, outs = append(cmds, cmd), append(outs, out)
	}
	// Both processes wait for this file, so their writes overlap.
	if err := os.WriteFile(start, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	for i, cmd := range cmds {
		if err := cmd.Wait(); err != nil {
			t.Errorf("writer %d: %v\n%s", i, err, outs[i])
		}
	}
	checkSequence(t, d, c.ID, 2*n+1)
}

// TestWriterProcess is one writer for TestTwoProcessesOnOneFileSerialise, run
// in a process of its own.
func TestWriterProcess(t *testing.T) {
	spec := os.Getenv(writerEnv)
	if spec == "" {
		t.Skip("run by TestTwoProcessesOnOneFileSerialise")
	}
	parts := strings.Split(spec, "\n")
	path, id, start := parts[0], parts[1], parts[3]
	n, err := strconv.Atoi(parts[2])
	if err != nil {
		t.Fatal(err)
	}
	for deadline := time.Now().Add(10 * time.Second); ; time.Sleep(time.Millisecond) {
		if _, err := os.Stat(start); err == nil {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("never told to start")
		}
	}
	d := openDB(t, path)
	for i := range n {
		if _, err := d.Note(t.Context(), id, NoteRecord{Body: fmt.Sprintf("process %d, note %d", os.Getpid(), i)}); err != nil {
			t.Fatal(err)
		}
	}
}

func TestAtRevision(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	c, err := d.Create(ctx, openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	read := c.Revision()
	// Answered and reopened by a follow-up since it was read: the case is open
	// again, as it was then.
	if _, err := d.Answer(ctx, c.ID, answerOf(KindStuck)); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Note(ctx, c.ID, NoteRecord{Body: "Which mirror?"}); err != nil {
		t.Fatal(err)
	}
	for name, call := range map[string]func() (*Case, error){
		"answer at the revision read": func() (*Case, error) {
			return d.Answer(ctx, c.ID, AnswerRecord{Drop: true}, AtRevision(read))
		},
		"park at the revision read": func() (*Case, error) { return d.Park(ctx, c.ID, ParkRecord{}, AtRevision(read)) },
		"park ahead of the case":    func() (*Case, error) { return d.Park(ctx, c.ID, ParkRecord{}, AtRevision(4)) },
		"answer at revision 0": func() (*Case, error) {
			return d.Answer(ctx, c.ID, AnswerRecord{Drop: true}, AtRevision(0))
		},
	} {
		if _, err := call(); !errors.Is(err, ErrStale) {
			t.Errorf("%s: err = %v, want ErrStale", name, err)
		}
	}
	if got := storedRows(t, d, c.ID); len(got) != 3 {
		t.Fatalf("rows after refused writes = %v", names(got))
	}

	// At the current revision the write goes through and raises it.
	parked, err := d.Park(ctx, c.ID, ParkRecord{}, AtRevision(3))
	if err != nil {
		t.Fatal(err)
	}
	if parked.Revision() != 4 {
		t.Errorf("revision after park = %d, want 4", parked.Revision())
	}
	// Without a precondition a write is unconditional, as the CLI makes it.
	if _, err := d.Resume(ctx, c.ID, AuthorAgent, ResumeRecord{}); err != nil {
		t.Fatal(err)
	}
	// Resumed since revision 4, the case is open: a resume at 4 is refused as
	// stale rather than as a resume of an open case.
	if _, err := d.Resume(ctx, c.ID, AuthorHuman, ResumeRecord{}, AtRevision(4)); !errors.Is(err, ErrStale) {
		t.Errorf("resume at revision 4: err = %v, want ErrStale", err)
	}
	if _, err := d.Park(ctx, c.ID, ParkRecord{}, AtRevision(5)); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Resume(ctx, c.ID, AuthorHuman, ResumeRecord{}, AtRevision(6)); err != nil {
		t.Fatal(err)
	}
	answered, err := d.Answer(ctx, c.ID, AnswerRecord{Drop: true}, AtRevision(7))
	if err != nil {
		t.Fatal(err)
	}
	if answered.State != StateAnswered || answered.Revision() != 8 || len(storedRows(t, d, c.ID)) != 8 {
		t.Errorf("after answer: state %s, revision %d, rows %v", answered.State, answered.Revision(), names(storedRows(t, d, c.ID)))
	}
}

// The agent's writes take the same precondition. Each is refused at a
// revision the case has moved on from, and goes through at its current one.
func TestAgentWritesAtRevision(t *testing.T) {
	writes := []struct {
		name  string
		setup func(d *DB, id string) error
		write func(d *DB, id string, pre ...Precondition) (*Case, error)
	}{
		{"amend", nil, func(d *DB, id string, pre ...Precondition) (*Case, error) {
			return d.Amend(t.Context(), id, AmendRecord{Links: []string{"https://example.com/log"}}, pre...)
		}},
		{"pickup", func(d *DB, id string) error { _, err := d.Answer(t.Context(), id, answerOf(KindStuck)); return err },
			func(d *DB, id string, pre ...Precondition) (*Case, error) {
				return d.Pickup(t.Context(), id, PickupRecord{}, pre...)
			}},
		{"note", func(d *DB, id string) error { _, err := d.Answer(t.Context(), id, answerOf(KindStuck)); return err },
			func(d *DB, id string, pre ...Precondition) (*Case, error) {
				return d.Note(t.Context(), id, NoteRecord{Body: "Which mirror?"}, pre...)
			}},
		{"close", func(d *DB, id string) error {
			if _, err := d.Answer(t.Context(), id, answerOf(KindStuck)); err != nil {
				return err
			}
			_, err := d.Pickup(t.Context(), id, PickupRecord{})
			return err
		}, func(d *DB, id string, pre ...Precondition) (*Case, error) {
			return d.Close(t.Context(), id, CloseRecord{Outcome: "Done."}, pre...)
		}},
		{"withdraw", nil, func(d *DB, id string, pre ...Precondition) (*Case, error) {
			return d.Withdraw(t.Context(), id, WithdrawRecord{}, pre...)
		}},
	}
	for _, w := range writes {
		t.Run(w.name, func(t *testing.T) {
			d := newDB(t)
			c, err := d.Create(t.Context(), openOf(KindStuck))
			if err != nil {
				t.Fatal(err)
			}
			if w.setup != nil {
				if err := w.setup(d, c.ID); err != nil {
					t.Fatal(err)
				}
			}
			rev := len(storedRows(t, d, c.ID))
			if _, err := w.write(d, c.ID, AtRevision(rev-1)); !errors.Is(err, ErrStale) {
				t.Errorf("at revision %d: err = %v, want ErrStale", rev-1, err)
			}
			if got := storedRows(t, d, c.ID); len(got) != rev {
				t.Fatalf("rows after a refused write = %v", names(got))
			}
			done, err := w.write(d, c.ID, AtRevision(rev))
			if err != nil {
				t.Fatal(err)
			}
			if done.Revision() != rev+1 {
				t.Errorf("revision = %d, want %d", done.Revision(), rev+1)
			}
		})
	}
}

// The precondition is checked inside the write's transaction. Another writer
// holds the write lock and adds a note while the answer waits for it; the
// answer must then see the note and be refused, not pass a check made before
// it had the lock.
func TestAtRevisionIsCheckedInsideTheWriteTransaction(t *testing.T) {
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	other, err := pool(t, openDB(t, d.Path)).BeginTx(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer other.Rollback()
	if _, err := other.ExecContext(t.Context(), `INSERT INTO events (case_id, seq, author, event, at, data) VALUES (?, 2, 'agent', 'note', '', ?)`, c.ID, []byte(`{"body":"One more thing."}`)); err != nil {
		t.Fatal(err)
	}

	errc := make(chan error, 1)
	go func() {
		_, err := d.Answer(t.Context(), c.ID, AnswerRecord{Ack: true}, AtRevision(c.Revision()))
		errc <- err
	}()
	// The pause does not decide the result; it gives an answer that did not
	// wait for the lock time to go through, so the test would catch one.
	time.Sleep(50 * time.Millisecond)
	select {
	case err := <-errc:
		t.Fatalf("the answer did not wait for the other writer: %v", err)
	default:
	}
	if err := other.Commit(); err != nil {
		t.Fatal(err)
	}
	if err := <-errc; !errors.Is(err, ErrStale) {
		t.Fatalf("err = %v, want ErrStale", err)
	}
	if got := names(storedRows(t, d, c.ID)); !slices.Equal(got, []string{"0001-agent-open.json", "0002-agent-note.json"}) {
		t.Errorf("rows = %v", got)
	}
}

// An event is stamped once its write holds the store's lock, so an event
// stored after another never records an earlier time, however long it waited.
func TestEventsAreStampedOnceTheWriteHoldsTheLock(t *testing.T) {
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	other, err := pool(t, openDB(t, d.Path)).BeginTx(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	type answered struct {
		c   *Case
		err error
	}
	done := make(chan answered, 1)
	go func() {
		c, err := d.Answer(t.Context(), c.ID, AnswerRecord{Ack: true})
		done <- answered{c, err}
	}()
	time.Sleep(100 * time.Millisecond)
	released := time.Now().UTC()
	if err := other.Rollback(); err != nil {
		t.Fatal(err)
	}
	got := <-done
	if got.err != nil {
		t.Fatal(got.err)
	}
	if at := got.c.Answer.AnsweredAt; at.Before(released) {
		t.Errorf("answered_at %s is before the lock was released at %s", at, released)
	}
}

func TestWrittenRecordShape(t *testing.T) {
	fixClock(t, time.Date(2026, 9, 15, 10, 0, 0, 0, time.UTC))
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := d.Answer(t.Context(), c.ID, AnswerRecord{Choice: 2, Note: "go"}); err != nil {
		t.Fatal(err)
	}
	rows := storedRows(t, d, c.ID)
	if rows[1].Author != "human" || rows[1].Event != "answer" {
		t.Errorf("row = %+v", rows[1])
	}
	var got map[string]any
	if err := json.Unmarshal([]byte(rows[1].Data), &got); err != nil {
		t.Fatal(err)
	}
	want := map[string]any{"choice": float64(2), "note": "go", "answered_at": "2026-09-15T10:00:00Z"}
	if len(got) != len(want) {
		t.Errorf("record = %v, want %v", got, want)
	}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("%s = %v, want %v", k, got[k], v)
		}
	}
	var at string
	if err := pool(t, d).QueryRowContext(t.Context(), `SELECT at FROM events WHERE case_id = ? AND seq = 2`, c.ID).Scan(&at); err != nil || at != "2026-09-15T10:00:00Z" {
		t.Errorf("at = %q, %v", at, err)
	}
}

func TestIsWholeID(t *testing.T) {
	for id, want := range map[string]bool{
		"2026-09-16T11-17-41Z-cannot-reach-mirror": true,
		"2026-09-16T11-17-41Z-pin-bun-2":           true,
		"2026-09-16T11-17-41Z-":                    false,
		"2026-09-16T11-17-41Z":                     false,
		"mirror":                                   false,
		"16T11-17-41Z-cannot-reach-mirror":         false,
		"2026-13-16T11-17-41Z-bad-month":           false,
	} {
		if got := IsWholeID(id); got != want {
			t.Errorf("IsWholeID(%q) = %v, want %v", id, got, want)
		}
	}
}
