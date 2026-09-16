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

// fixClock pins the event clock for one test.
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
