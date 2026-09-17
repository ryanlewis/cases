package store

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func writeFile(t *testing.T, dir, name, content string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestLoadToleratesMalformedFiles(t *testing.T) {
	c, err := Create(t.TempDir(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, c.Dir, "0002-human-answer.json", `{"ack": tru`)
	writeFile(t, c.Dir, ".tmp-12345", `half a fi`)
	writeFile(t, c.Dir, "README.md", "stray")
	writeFile(t, c.Dir, "0003-bob-note.json", `{}`)

	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if loaded.State != StateOpen {
		t.Errorf("state = %s", loaded.State)
	}
	problems := strings.Join(loaded.Problems, "\n")
	for _, want := range []string{"0002-human-answer.json: malformed answer event", "README.md: not an event file name", "0003-bob-note.json: not an event file name"} {
		if !strings.Contains(problems, want) {
			t.Errorf("problems %q missing %q", problems, want)
		}
	}
	if strings.Contains(problems, ".tmp") {
		t.Errorf("temp file reported: %q", problems)
	}

	// The next write goes after the malformed file, not on top of it.
	after, err := Answer(c.Dir, AnswerRecord{Ack: true})
	if err != nil {
		t.Fatal(err)
	}
	if last := after.Events[len(after.Events)-1]; last.File != "0003-human-answer.json" {
		t.Errorf("next file = %s", last.File)
	}
}

func TestLoadPreservesUnknownFields(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "2026-09-15T09-00-00Z-x")
	writeFile(t, dir, "0001-agent-open.json", `{"kind":"fyi","urgency":"today","title":"x","opened_at":"2026-09-15T09:00:00Z","colour":"teal","extra":{"n":1}}`)

	c, err := Load(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(c.Problems) != 0 {
		t.Errorf("problems = %v", c.Problems)
	}
	var data map[string]any
	if err := json.Unmarshal(c.Events[0].Data, &data); err != nil {
		t.Fatal(err)
	}
	if data["colour"] != "teal" || data["extra"] == nil {
		t.Errorf("data = %v, want unknown fields kept", data)
	}
	out, err := json.Marshal(c)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(out), `"colour":"teal"`) {
		t.Errorf("case JSON dropped unknown field: %s", out)
	}
}

func TestLoadSyncRaceIsDeterministic(t *testing.T) {
	// Two sides wrote sequence 0002 on different machines: the agent withdrew
	// while the human answered. Filename order puts the agent first, so the
	// withdraw stands and the answer is reported, never silently applied.
	c, err := Create(t.TempDir(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, c.Dir, "0002-agent-withdraw.json", `{"withdrawn_at":"2026-09-15T10:00:00Z"}`)
	writeFile(t, c.Dir, "0002-human-answer.json", `{"ack":true,"answered_at":"2026-09-15T10:00:01Z"}`)

	loaded, err := Load(c.Dir)
	if err != nil {
		t.Fatal(err)
	}
	if loaded.State != StateWithdrawn {
		t.Errorf("state = %s", loaded.State)
	}
	problems := strings.Join(loaded.Problems, "\n")
	if !strings.Contains(problems, "more than one file") || !strings.Contains(problems, "cannot answer a case that is withdrawn") {
		t.Errorf("problems = %q", problems)
	}
}

func TestLoadWithoutOpenFails(t *testing.T) {
	dir := t.TempDir()
	if _, err := Load(dir); err == nil || !strings.Contains(err.Error(), "no open event") {
		t.Errorf("empty dir: err = %v", err)
	}
	writeFile(t, dir, "0001-agent-open.json", `not json`)
	if _, err := Load(dir); err == nil || !strings.Contains(err.Error(), "no valid open event") {
		t.Errorf("malformed open: err = %v", err)
	}
}

func TestListSkipsBrokenCases(t *testing.T) {
	root := t.TempDir()
	good, err := Create(root, openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(root, "2026-01-01T00-00-00Z-broken"), "0001-agent-open.json", `{"kind":`)
	writeFile(t, root, "stray.txt", "not a case")
	if err := os.Mkdir(filepath.Join(root, ".obsidian"), 0o755); err != nil {
		t.Fatal(err)
	}
	// A case directory caught between Create's mkdir and its open event.
	writeFile(t, filepath.Join(root, "2026-09-15T10-00-00Z-being-created"), ".tmp-1", "{")

	cases, bad, err := List(root)
	if err != nil {
		t.Fatal(err)
	}
	if len(cases) != 1 || cases[0].ID != good.ID {
		t.Errorf("cases = %v", cases)
	}
	if len(bad) != 1 || !strings.Contains(bad[0].Error(), "broken") {
		t.Errorf("bad = %v", bad)
	}
	if _, pbad, err := NewPoller(root).Poll(); err != nil || len(pbad) != 1 {
		t.Errorf("poller bad = %v, err = %v", pbad, err)
	}
}

func TestListMissingRoot(t *testing.T) {
	if _, _, err := List(filepath.Join(t.TempDir(), "nope")); err == nil {
		t.Error("want an error")
	}
}

func TestPollerSeesNewEventsAndCases(t *testing.T) {
	root := t.TempDir()
	first, err := Create(root, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	p := NewPoller(root)
	cases, _, err := p.Poll()
	if err != nil || len(cases) != 1 || cases[0].State != StateOpen {
		t.Fatalf("first poll: %v %v", cases, err)
	}

	if _, err := Answer(first.Dir, AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	second, err := Create(root, openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	cases, _, err = p.Poll()
	if err != nil {
		t.Fatal(err)
	}
	states := map[string]State{}
	for _, c := range cases {
		states[c.ID] = c.State
	}
	if states[first.ID] != StateAnswered || states[second.ID] != StateOpen {
		t.Errorf("states = %v", states)
	}

	// Once mtimes have settled, an unchanged directory is served from cache.
	old := time.Now().Add(-time.Hour)
	for _, dir := range []string{root, first.Dir, second.Dir} {
		if err := os.Chtimes(dir, old, old); err != nil {
			t.Fatal(err)
		}
	}
	if _, _, err := p.Poll(); err != nil {
		t.Fatal(err)
	}
	cached := p.cache[first.Dir].c
	if _, _, err := p.Poll(); err != nil {
		t.Fatal(err)
	}
	if p.cache[first.Dir].c != cached {
		t.Error("unchanged case was reloaded")
	}

	if err := os.RemoveAll(second.Dir); err != nil {
		t.Fatal(err)
	}
	cases, _, err = p.Poll()
	if err != nil || len(cases) != 1 {
		t.Errorf("after removal: %d cases, err %v", len(cases), err)
	}
}

// A filesystem with 2s mtimes (FAT) can add a file to a directory without
// changing the directory's mtime. A poll that read the directory before the
// file arrived must not go on serving that read once settle has passed.
func TestPollerSeesEntriesAddedWithinOneMtimeStep(t *testing.T) {
	root := t.TempDir()
	first, err := Create(root, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	// The filesystem floors every mtime to the start of one step. The clock is
	// pinned, so nothing here depends on how long the test takes.
	step := time.Date(2020, 1, 2, 3, 4, 6, 0, time.UTC)
	coarse := func(dirs ...string) {
		t.Helper()
		for _, dir := range dirs {
			if err := os.Chtimes(dir, step, step); err != nil {
				t.Fatal(err)
			}
		}
	}
	coarse(root, first.Dir)
	fixClock(t, step.Add(500*time.Millisecond))
	p := NewPoller(root)
	if _, _, err := p.Poll(); err != nil {
		t.Fatal(err)
	}

	// Both writes land inside the same step, after the poll read the store.
	if _, err := Answer(first.Dir, AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	second, err := Create(root, openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	coarse(root, first.Dir, second.Dir)
	fixClock(t, step.Add(settle))

	cases, _, err := p.Poll()
	if err != nil {
		t.Fatal(err)
	}
	states := map[string]State{}
	for _, c := range cases {
		states[c.ID] = c.State
	}
	if states[first.ID] != StateAnswered || states[second.ID] != StateOpen {
		t.Errorf("states = %v, want the answer and the new case", states)
	}

	// A read that started at least settle after the mtime is kept.
	cached := p.cache[first.Dir].c
	if _, _, err := p.Poll(); err != nil {
		t.Fatal(err)
	}
	if p.cache[first.Dir].c != cached {
		t.Error("settled case was reloaded")
	}
}

func TestCaseIDs(t *testing.T) {
	root := t.TempDir()
	for _, name := range []string{"b-case", "a-case", ".archive"} {
		if err := os.Mkdir(filepath.Join(root, name), 0o755); err != nil {
			t.Fatal(err)
		}
	}
	writeFile(t, root, "stray.json", "{}")
	ids, err := CaseIDs(root)
	if err != nil || strings.Join(ids, ",") != "a-case,b-case" {
		t.Errorf("CaseIDs = %v, %v", ids, err)
	}
}
