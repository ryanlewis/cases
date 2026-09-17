package store

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"syscall"
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

func fileNames(t *testing.T, dir string) []string {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, e := range entries {
		names = append(names, e.Name())
	}
	return names
}

func TestCreateWritesOpenEvent(t *testing.T) {
	root := filepath.Join(t.TempDir(), "cases")
	fixClock(t, time.Date(2026, 9, 15, 9, 12, 3, 0, time.UTC))

	c, err := Create(root, OpenRecord{Kind: KindFYI, Urgency: UrgencyWhenever, Title: "Pin bun or float?"})
	if err != nil {
		t.Fatal(err)
	}
	if c.ID != "2026-09-15T09-12-03Z-pin-bun-or-float" {
		t.Errorf("id = %s", c.ID)
	}
	if got := fileNames(t, c.Dir); !slices.Equal(got, []string{"0001-agent-open.json"}) {
		t.Errorf("files = %v", got)
	}
	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if loaded.State != StateOpen || loaded.Title != "Pin bun or float?" || !loaded.OpenedAt.Equal(now()) {
		t.Errorf("loaded = %+v", loaded)
	}

	// Same second, same title: a second directory, not a clash.
	c2, err := Create(root, OpenRecord{Kind: KindFYI, Urgency: UrgencyWhenever, Title: "Pin bun or float?"})
	if err != nil {
		t.Fatal(err)
	}
	if c2.ID != c.ID+"-2" {
		t.Errorf("second id = %s", c2.ID)
	}
}

func TestCreateWritesLabels(t *testing.T) {
	rec := openOf(KindFYI)
	rec.Labels = []string{"feat-labels", "round 3"}
	c, err := Create(t.TempDir(), rec)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(c.Dir, "0001-agent-open.json"))
	if err != nil {
		t.Fatal(err)
	}
	var got struct{ Labels []string }
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(got.Labels, rec.Labels) {
		t.Errorf("open file = %s", raw)
	}
	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(loaded.Labels, rec.Labels) {
		t.Errorf("labels = %q", loaded.Labels)
	}
}

// An open file written before labels existed has no labels key and still
// loads, with no labels.
func TestOpenFileWithoutLabelsLoads(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "2026-09-01T10-00-00Z-old-case")
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	writeFile(t, dir, "0001-agent-open.json", `{"kind":"fyi","urgency":"whenever","title":"Old case","worker":"w1","brief":"BRIEF.md","opened_at":"2026-09-01T10:00:00Z"}`)
	c, err := Load(dir)
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
	if strings.Contains(string(raw), `"labels"`) {
		t.Errorf("json = %s", raw)
	}
}

func TestCreateRefusesInvalidOpenWithoutLeavingADirectory(t *testing.T) {
	root := t.TempDir()
	if _, err := Create(root, OpenRecord{Kind: KindDecision, Urgency: UrgencyToday, Title: "No options"}); err == nil {
		t.Fatal("want an error")
	}
	if names := fileNames(t, root); len(names) != 0 {
		t.Errorf("store holds %v after a refused open", names)
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

func TestCaseDirRejectsPaths(t *testing.T) {
	for _, id := range []string{"", ".", "..", "../x", "a/b", `a\b`, ".hidden"} {
		if _, err := CaseDir("/store", id); err == nil {
			t.Errorf("CaseDir(%q) accepted", id)
		}
	}
	if dir, err := CaseDir("/store", "2026-09-15T09-12-03Z-x"); err != nil || dir != "/store/2026-09-15T09-12-03Z-x" {
		t.Errorf("dir = %q, err = %v", dir, err)
	}
}

func TestAppendNumbersFilesInOrder(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	steps := []func() (*Case, error){
		func() (*Case, error) { return Answer(c.Dir, AnswerRecord{Choice: 1}) },
		func() (*Case, error) { return Pickup(c.Dir, PickupRecord{By: "mgr"}) },
		func() (*Case, error) { return Note(c.Dir, NoteRecord{Body: "and?"}) },
		func() (*Case, error) { return Answer(c.Dir, AnswerRecord{Other: true, Note: "neither"}) },
		func() (*Case, error) { return Pickup(c.Dir, PickupRecord{}) },
		func() (*Case, error) {
			return Close(c.Dir, CloseRecord{Outcome: "went with neither", Links: []string{"https://x"}})
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
	if got := fileNames(t, c.Dir); !slices.Equal(got, want) {
		t.Errorf("files = %v", got)
	}
	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if loaded.State != StateClosed || !loaded.Answer.Other || loaded.Close.Outcome != "went with neither" {
		t.Errorf("loaded = %+v", loaded)
	}
}

func TestInvalidTransitionIsNotWritten(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	checks := []struct {
		name string
		call func() (*Case, error)
	}{
		{"pickup an open case", func() (*Case, error) { return Pickup(c.Dir, PickupRecord{}) }},
		{"close an open case", func() (*Case, error) { return Close(c.Dir, CloseRecord{Outcome: "x"}) }},
		{"park a decision", func() (*Case, error) { return Park(c.Dir, ParkRecord{}) }},
		{"resume an open case", func() (*Case, error) { return Resume(c.Dir, AuthorHuman, ResumeRecord{}) }},
		{"answer with a bad choice", func() (*Case, error) { return Answer(c.Dir, AnswerRecord{Choice: 9}) }},
		{"resume by an unknown author", func() (*Case, error) { return Resume(c.Dir, "robot", ResumeRecord{}) }},
		{"amend with nothing", func() (*Case, error) { return Amend(c.Dir, AmendRecord{}) }},
		{"amend a decision with rows", func() (*Case, error) { return Amend(c.Dir, AmendRecord{Rows: openOf(KindApproval).Rows}) }},
	}
	for _, ch := range checks {
		if _, err := ch.call(); err == nil {
			t.Errorf("%s: accepted", ch.name)
		}
	}
	if got := fileNames(t, c.Dir); !slices.Equal(got, []string{"0001-agent-open.json"}) {
		t.Errorf("files after refused writes = %v", got)
	}
}

func TestAmend(t *testing.T) {
	fixClock(t, time.Date(2026, 9, 16, 11, 0, 0, 0, time.UTC))
	c, err := Create(t.TempDir(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	openFile := filepath.Join(c.Dir, "0001-agent-open.json")
	opened, err := os.ReadFile(openFile)
	if err != nil {
		t.Fatal(err)
	}

	amended, err := Amend(c.Dir, AmendRecord{Options: []string{"Vendor it"}, Links: []string{"https://example.com/log"}})
	if err != nil {
		t.Fatal(err)
	}
	if amended.State != StateOpen || !slices.Equal(amended.Options, []string{"Pin", "Float", "Vendor it"}) {
		t.Errorf("amended = %+v", amended)
	}
	if got := fileNames(t, c.Dir); !slices.Equal(got, []string{"0001-agent-open.json", "0002-agent-amend.json"}) {
		t.Errorf("files = %v", got)
	}
	if now, _ := os.ReadFile(openFile); string(now) != string(opened) {
		t.Errorf("open event changed from %s to %s", opened, now)
	}
	// Only the fields the amend sets are written.
	raw, err := os.ReadFile(filepath.Join(c.Dir, "0002-agent-amend.json"))
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	if len(got) != 3 || got["amended_at"] != "2026-09-16T11:00:00Z" || got["options"] == nil || got["links"] == nil {
		t.Errorf("record = %v", got)
	}
	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Options) != 3 || len(loaded.Links) != 1 || len(loaded.Problems) != 0 {
		t.Errorf("loaded = %+v, problems %v", loaded.OpenRecord, loaded.Problems)
	}
}

func TestAppendToMissingCase(t *testing.T) {
	_, err := Pickup(filepath.Join(t.TempDir(), "nope"), PickupRecord{})
	if !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("err = %v, want not exist", err)
	}
}

// Without hard links the last step is a rename. When it fails, the write fails
// and leaves nothing behind.
func TestAtomicWriteLeavesNothingOnFailure(t *testing.T) {
	dir := t.TempDir()
	origLink, origRename := link, rename
	link = linkFails(syscall.EPERM)
	rename = func(string, string) error { return errors.New("disk on fire") }
	t.Cleanup(func() { link, rename = origLink, origRename })

	if err := writeFileAtomic(dir, "0001-agent-open.json", []byte("{}")); err == nil {
		t.Fatal("want an error")
	}
	if names := fileNames(t, dir); len(names) != 0 {
		t.Errorf("dir holds %v after a failed write", names)
	}
}

// linkFails returns a link that makes no link and fails with err.
func linkFails(err error) func(string, string) error {
	return func(oldname, newname string) error {
		return &os.LinkError{Op: "link", Old: oldname, New: newname, Err: err}
	}
}

// A sync client can add the event file after writeFileAtomic has checked that
// the name is free. The last step must refuse it, not replace it: when the
// link reports the name exists, and when the link is refused before the name
// is looked up, as a macOS sandbox that denies hard links does.
func TestAtomicWriteRefusesAFileAddedDuringTheWrite(t *testing.T) {
	for _, tc := range []struct {
		name string
		link func(string, string) error
	}{
		{"link finds the file", os.Link},
		{"link denied", linkFails(syscall.EPERM)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			const synced = `{"choice":1}` + "\n"
			orig := link
			link = func(oldname, newname string) error {
				writeFile(t, dir, "0002-human-answer.json", synced)
				return tc.link(oldname, newname)
			}
			t.Cleanup(func() { link = orig })

			checkErr(t, writeFileAtomic(dir, "0002-human-answer.json", []byte(`{"choice":2}`+"\n")), "already exists")
			if got, _ := os.ReadFile(filepath.Join(dir, "0002-human-answer.json")); string(got) != synced {
				t.Errorf("synced file changed to %q", got)
			}
			if names := fileNames(t, dir); !slices.Equal(names, []string{"0002-human-answer.json"}) {
				t.Errorf("files = %v, want only the synced file (no temp left behind)", names)
			}
		})
	}
}

// On a network mount a link can go through and still report an error, as when
// a retried request finds the link the first one made. The event is written,
// so the write must not fail: a retry would write the event twice.
func TestAtomicWriteAcceptsItsOwnLink(t *testing.T) {
	for _, tc := range []struct {
		name string
		err  error
	}{
		{"EEXIST", syscall.EEXIST},
		{"EIO", syscall.EIO},
	} {
		t.Run(tc.name, func(t *testing.T) {
			orig := link
			link = func(oldname, newname string) error {
				if err := os.Link(oldname, newname); err != nil {
					return err
				}
				return linkFails(tc.err)(oldname, newname)
			}
			t.Cleanup(func() { link = orig })

			dir := t.TempDir()
			data := []byte(`{"body":"all of it"}` + "\n")
			if err := writeFileAtomic(dir, "0001-agent-note.json", data); err != nil {
				t.Fatal(err)
			}
			if names := fileNames(t, dir); !slices.Equal(names, []string{"0001-agent-note.json"}) {
				t.Errorf("files = %v, want only the final file (no temp left behind)", names)
			}
			if got, err := os.ReadFile(filepath.Join(dir, "0001-agent-note.json")); err != nil || string(got) != string(data) {
				t.Errorf("content = %q, err = %v", got, err)
			}
		})
	}
}

// Removing the temporary name after the link only tidies up. If it fails, the
// event is written all the same, and the write must say so.
func TestAtomicWriteSucceedsWhenTheTemporaryNameStays(t *testing.T) {
	orig := remove
	remove = func(string) error { return errors.New("disk on fire") }
	t.Cleanup(func() { remove = orig })

	dir := t.TempDir()
	data := []byte(`{"body":"all of it"}` + "\n")
	if err := writeFileAtomic(dir, "0001-agent-note.json", data); err != nil {
		t.Fatal(err)
	}
	if got, err := os.ReadFile(filepath.Join(dir, "0001-agent-note.json")); err != nil || string(got) != string(data) {
		t.Errorf("content = %q, err = %v", got, err)
	}
}

// TestAtomicWrite runs with hard links and without them, where publish renames
// instead. Without hard links Linux fails the link with EPERM, macOS with
// ENOTSUP, which is errors.ErrUnsupported.
func TestAtomicWrite(t *testing.T) {
	for _, tc := range []struct {
		name string
		link func(string, string) error
	}{
		{"hard links", os.Link},
		{"no hard links EPERM", linkFails(syscall.EPERM)},
		{"no hard links ENOTSUP", linkFails(errors.ErrUnsupported)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			orig := link
			link = tc.link
			t.Cleanup(func() { link = orig })

			dir := t.TempDir()
			data := []byte(`{"body":"all of it"}` + "\n")
			if err := writeFileAtomic(dir, "0001-agent-note.json", data); err != nil {
				t.Fatal(err)
			}
			if names := fileNames(t, dir); !slices.Equal(names, []string{"0001-agent-note.json"}) {
				t.Errorf("files = %v, want only the final file (no temp left behind)", names)
			}
			got, err := os.ReadFile(filepath.Join(dir, "0001-agent-note.json"))
			if err != nil || string(got) != string(data) {
				t.Errorf("content = %q, err = %v", got, err)
			}
			info, _ := os.Stat(filepath.Join(dir, "0001-agent-note.json"))
			if info.Mode().Perm() != 0o644 {
				t.Errorf("mode = %v", info.Mode().Perm())
			}
			if err := writeFileAtomic(dir, "0001-agent-note.json", []byte("{}")); err == nil {
				t.Error("overwrote an existing event file")
			}
			got, _ = os.ReadFile(filepath.Join(dir, "0001-agent-note.json"))
			if string(got) != string(data) {
				t.Errorf("existing file changed to %q", got)
			}
		})
	}
}

func TestConcurrentAppendsTakeDistinctSequenceNumbers(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	const n = 20
	var wg sync.WaitGroup
	errs := make(chan error, n)
	for range n {
		wg.Go(func() {
			if _, err := Note(c.Dir, NoteRecord{Body: "more"}); err != nil {
				errs <- err
			}
		})
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Error(err)
	}
	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(loaded.Events) != n+1 || len(loaded.Problems) != 0 {
		t.Errorf("events = %d, problems = %v", len(loaded.Events), loaded.Problems)
	}
}

func TestAtRevision(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	read := c.Revision()
	// Answered and reopened by a follow-up since it was read: the case is open
	// again, as it was then.
	if _, err := Answer(c.Dir, answerOf(KindStuck)); err != nil {
		t.Fatal(err)
	}
	if _, err := Note(c.Dir, NoteRecord{Body: "Which mirror?"}); err != nil {
		t.Fatal(err)
	}
	for name, call := range map[string]func() (*Case, error){
		"answer at the revision read": func() (*Case, error) { return Answer(c.Dir, AnswerRecord{Drop: true}, AtRevision(read)) },
		"park at the revision read":   func() (*Case, error) { return Park(c.Dir, ParkRecord{}, AtRevision(read)) },
		"park ahead of the case":      func() (*Case, error) { return Park(c.Dir, ParkRecord{}, AtRevision(4)) },
		"answer at revision 0":        func() (*Case, error) { return Answer(c.Dir, AnswerRecord{Drop: true}, AtRevision(0)) },
	} {
		if _, err := call(); !errors.Is(err, ErrStale) {
			t.Errorf("%s: err = %v, want ErrStale", name, err)
		}
	}
	if got := fileNames(t, c.Dir); len(got) != 3 {
		t.Fatalf("files after refused writes = %v", got)
	}

	// At the current revision the write goes through and raises it.
	parked, err := Park(c.Dir, ParkRecord{}, AtRevision(3))
	if err != nil {
		t.Fatal(err)
	}
	if parked.Revision() != 4 {
		t.Errorf("revision after park = %d, want 4", parked.Revision())
	}
	// Without a precondition a write is unconditional, as the CLI makes it.
	if _, err := Resume(c.Dir, AuthorAgent, ResumeRecord{}); err != nil {
		t.Fatal(err)
	}
	// Resumed since revision 4, the case is open: a resume at 4 is refused as
	// stale rather than as a resume of an open case.
	if _, err := Resume(c.Dir, AuthorHuman, ResumeRecord{}, AtRevision(4)); !errors.Is(err, ErrStale) {
		t.Errorf("resume at revision 4: err = %v, want ErrStale", err)
	}
	if _, err := Park(c.Dir, ParkRecord{}, AtRevision(5)); err != nil {
		t.Fatal(err)
	}
	if _, err := Resume(c.Dir, AuthorHuman, ResumeRecord{}, AtRevision(6)); err != nil {
		t.Fatal(err)
	}
	answered, err := Answer(c.Dir, AnswerRecord{Drop: true}, AtRevision(7))
	if err != nil {
		t.Fatal(err)
	}
	if answered.State != StateAnswered || answered.Revision() != 8 || len(fileNames(t, c.Dir)) != 8 {
		t.Errorf("after answer: state %s, revision %d, files %v", answered.State, answered.Revision(), fileNames(t, c.Dir))
	}
}

// The agent's writes take the same precondition. Each is refused at a
// revision the case has moved on from, and goes through at its current one.
func TestAgentWritesAtRevision(t *testing.T) {
	writes := []struct {
		name  string
		setup func(dir string) error
		write func(dir string, pre ...Precondition) (*Case, error)
	}{
		{"amend", nil, func(dir string, pre ...Precondition) (*Case, error) {
			return Amend(dir, AmendRecord{Links: []string{"https://example.com/log"}}, pre...)
		}},
		{"pickup", func(dir string) error { _, err := Answer(dir, answerOf(KindStuck)); return err },
			func(dir string, pre ...Precondition) (*Case, error) { return Pickup(dir, PickupRecord{}, pre...) }},
		{"note", func(dir string) error { _, err := Answer(dir, answerOf(KindStuck)); return err },
			func(dir string, pre ...Precondition) (*Case, error) {
				return Note(dir, NoteRecord{Body: "Which mirror?"}, pre...)
			}},
		{"close", func(dir string) error {
			if _, err := Answer(dir, answerOf(KindStuck)); err != nil {
				return err
			}
			_, err := Pickup(dir, PickupRecord{})
			return err
		}, func(dir string, pre ...Precondition) (*Case, error) {
			return Close(dir, CloseRecord{Outcome: "Done."}, pre...)
		}},
		{"withdraw", nil, func(dir string, pre ...Precondition) (*Case, error) { return Withdraw(dir, WithdrawRecord{}, pre...) }},
	}
	for _, w := range writes {
		t.Run(w.name, func(t *testing.T) {
			c, err := Create(t.TempDir(), openOf(KindStuck))
			if err != nil {
				t.Fatal(err)
			}
			if w.setup != nil {
				if err := w.setup(c.Dir); err != nil {
					t.Fatal(err)
				}
			}
			rev := len(fileNames(t, c.Dir))
			if _, err := w.write(c.Dir, AtRevision(rev-1)); !errors.Is(err, ErrStale) {
				t.Errorf("at revision %d: err = %v, want ErrStale", rev-1, err)
			}
			if got := fileNames(t, c.Dir); len(got) != rev {
				t.Fatalf("files after a refused write = %v", got)
			}
			done, err := w.write(c.Dir, AtRevision(rev))
			if err != nil {
				t.Fatal(err)
			}
			if done.Revision() != rev+1 {
				t.Errorf("revision = %d, want %d", done.Revision(), rev+1)
			}
		})
	}
}

// The flock does nothing across machines, so a sync can bring in an event with
// a sequence number the case already has, or one below its latest. Either one
// changes the case, and a write at the revision read before it is refused.
func TestAtRevisionSeesSyncedEvents(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	for range 2 {
		if _, err := Note(c.Dir, NoteRecord{Body: "More detail."}); err != nil {
			t.Fatal(err)
		}
	}
	read, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	// Answered on another machine that had only seen the open event. The last
	// note reopens the case, so it is open again, as it was when read.
	writeFile(t, c.Dir, "0002-human-answer.json", `{"choice":1}`)
	if _, err := Answer(c.Dir, AnswerRecord{Choice: 1}, AtRevision(read.Revision())); !errors.Is(err, ErrStale) {
		t.Errorf("same sequence number: err = %v, want ErrStale", err)
	}

	// Synced out of order: a later event arrives before the ones below it.
	d, err := Create(t.TempDir(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, d.Dir, "0004-agent-note.json", `{"body":"Still waiting."}`)
	read, err = Load(d.Dir)
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, d.Dir, "0002-human-answer.json", `{"choice":1}`)
	writeFile(t, d.Dir, "0003-agent-pickup.json", `{}`)
	if _, err := Answer(d.Dir, AnswerRecord{Choice: 1}, AtRevision(read.Revision())); !errors.Is(err, ErrStale) {
		t.Errorf("out of order: err = %v, want ErrStale", err)
	}
	if got := fileNames(t, d.Dir); len(got) != 4 {
		t.Errorf("files after refused write = %v", got)
	}
}

// An answer from a machine the amend had not synced to yet has the amend's
// sequence number or a lower one. It answers the case as it was before the
// amend, so it is refused and the case stays open for another answer.
func TestAnswerWrittenWithoutAnAmendIsRefused(t *testing.T) {
	for _, kind := range []Kind{KindDecision, KindApproval} {
		t.Run(string(kind), func(t *testing.T) {
			c, err := Create(t.TempDir(), openOf(kind))
			if err != nil {
				t.Fatal(err)
			}
			if _, err := Amend(c.Dir, AmendRecord{Body: "The question has changed."}); err != nil {
				t.Fatal(err)
			}
			answer, err := json.Marshal(answerOf(kind))
			if err != nil {
				t.Fatal(err)
			}
			writeFile(t, c.Dir, "0002-human-answer.json", string(answer))
			loaded, err := Load(c.Dir)
			if err != nil {
				t.Fatal(err)
			}
			refused := slices.ContainsFunc(loaded.Problems, func(p string) bool {
				return p == "0002-human-answer.json: the answer was written without seeing amend 0002"
			})
			if loaded.State != StateOpen || loaded.Answer != nil || !refused {
				t.Errorf("state %s, answer %+v, problems %q", loaded.State, loaded.Answer, loaded.Problems)
			}

			// Answered again after the amend, the answer is recorded.
			answered, err := Answer(c.Dir, answerOf(kind))
			if err != nil {
				t.Fatal(err)
			}
			if answered.State != StateAnswered {
				t.Errorf("state after answering again = %s", answered.State)
			}
		})
	}
}

func TestAtRevisionIsCheckedUnderTheLock(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	unlock, err := lockDir(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	errc := make(chan error, 1)
	go func() {
		_, err := Answer(c.Dir, AnswerRecord{Ack: true}, AtRevision(c.Revision()))
		errc <- err
	}()
	// Another writer holds the lock and adds a note while the answer waits for
	// it. The pause does not decide the result; it gives a check made before
	// taking the lock time to run and pass, so the test would catch one.
	time.Sleep(50 * time.Millisecond)
	writeFile(t, c.Dir, "0002-agent-note.json", `{"body":"One more thing."}`)
	unlock()
	if err := <-errc; !errors.Is(err, ErrStale) {
		t.Fatalf("err = %v, want ErrStale", err)
	}
	if got := fileNames(t, c.Dir); len(got) != 2 {
		t.Errorf("files = %v", got)
	}
}

func TestWrittenRecordShape(t *testing.T) {
	fixClock(t, time.Date(2026, 9, 15, 10, 0, 0, 0, time.UTC))
	c, err := Create(t.TempDir(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Answer(c.Dir, AnswerRecord{Choice: 2, Note: "go"}); err != nil {
		t.Fatal(err)
	}
	raw, err := os.ReadFile(filepath.Join(c.Dir, "0002-human-answer.json"))
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(raw, &got); err != nil {
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
}
