package main

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

func TestAnswerByKind(t *testing.T) {
	approvalRows := []string{
		"--row", `{"id":"a","label":"A","script":"echo a","link":"https://x/a"}`,
		"--row", `{"id":"b","label":"B","script":"echo b","link":"https://x/b"}`,
	}
	tests := []struct {
		name      string
		open      []string
		answer    []string
		wantState store.State
		wantErr   string
		check     func(t *testing.T, c *store.Case)
	}{
		{
			name: "decision option", open: []string{"--kind", "decision", "--option", "x", "--option", "y"},
			answer: []string{"--option", "2"}, wantState: store.StateAnswered,
			check: func(t *testing.T, c *store.Case) {
				if c.Answer.Choice != 2 {
					t.Errorf("choice = %d", c.Answer.Choice)
				}
			},
		},
		{name: "decision other needs note", open: []string{"--kind", "decision", "--option", "x"}, answer: []string{"--other"}, wantErr: "other needs a note"},
		{name: "decision out of range", open: []string{"--kind", "decision", "--option", "x"}, answer: []string{"--option", "5"}, wantErr: "not an option"},
		{name: "decision two responses", open: []string{"--kind", "decision", "--option", "x"}, answer: []string{"--option", "1", "--ack"}, wantErr: "can't be used together"},
		{
			name: "approval rows with notes", open: append([]string{"--kind", "approval"}, approvalRows...),
			answer: []string{"--row", "a=approve", "--row", "b=hold:needs a dry run: first"}, wantState: store.StateAnswered,
			check: func(t *testing.T, c *store.Case) {
				if c.Answer.Rows[1].Verdict != "hold" || c.Answer.Rows[1].Note != "needs a dry run: first" {
					t.Errorf("rows = %+v", c.Answer.Rows)
				}
			},
		},
		{name: "approval missing row", open: append([]string{"--kind", "approval"}, approvalRows...), answer: []string{"--row", "a=approve"}, wantErr: `row "b" has no verdict`},
		{name: "approval malformed row", open: append([]string{"--kind", "approval"}, approvalRows...), answer: []string{"--row", "approve"}, wantErr: "want id=approve|hold|reject"},
		{name: "signoff accept", open: []string{"--kind", "signoff"}, answer: []string{"--accept"}, wantState: store.StateAnswered},
		{name: "signoff changes needs note", open: []string{"--kind", "signoff"}, answer: []string{"--changes"}, wantErr: "needs a note"},
		{name: "signoff changes", open: []string{"--kind", "signoff"}, answer: []string{"--changes", "--note", "rename it"}, wantState: store.StateAnswered},
		{name: "stuck text", open: []string{"--kind", "stuck"}, answer: []string{"--text", "use the mirror"}, wantState: store.StateAnswered},
		{name: "stuck drop", open: []string{"--kind", "stuck"}, answer: []string{"--drop"}, wantState: store.StateAnswered},
		{
			name: "stuck park", open: []string{"--kind", "stuck"}, answer: []string{"--park", "--note", "after release"}, wantState: store.StateParked,
			check: func(t *testing.T, c *store.Case) {
				if c.Park == nil || c.Park.Note != "after release" || c.Answer != nil {
					t.Errorf("park = %+v, answer = %+v", c.Park, c.Answer)
				}
			},
		},
		{
			name: "question text", open: []string{"--kind", "question"}, answer: []string{"--text", "the staging one", "--note", "ask again if it moves"}, wantState: store.StateAnswered,
			check: func(t *testing.T, c *store.Case) {
				if c.Answer.Text != "the staging one" || c.Answer.Note != "ask again if it moves" {
					t.Errorf("answer = %+v", c.Answer)
				}
			},
		},
		{name: "question blank text", open: []string{"--kind", "question"}, answer: []string{"--text", "  "}, wantErr: "reply text is empty"},
		{name: "question no response", open: []string{"--kind", "question"}, answer: []string{"--note", "hm"}, wantErr: "no question response"},
		{name: "question drop", open: []string{"--kind", "question"}, answer: []string{"--drop", "--note", "not needed"}, wantState: store.StateAnswered},
		{name: "decision drop", open: []string{"--kind", "decision", "--option", "x"}, answer: []string{"--drop"}, wantState: store.StateAnswered},
		{name: "fyi drop", open: []string{"--kind", "fyi"}, answer: []string{"--drop"}, wantState: store.StateAnswered},
		{name: "question ack", open: []string{"--kind", "question"}, answer: []string{"--ack"}, wantErr: "question answers cannot set ack"},
		{name: "park a question", open: []string{"--kind", "question"}, answer: []string{"--park"}, wantErr: "only stuck cases park"},
		{name: "park a decision", open: []string{"--kind", "decision", "--option", "x"}, answer: []string{"--park"}, wantErr: "only stuck cases park"},
		{name: "fyi ack", open: []string{"--kind", "fyi"}, answer: []string{"--ack", "--note", "thanks"}, wantState: store.StateAnswered},
		{name: "fyi with signoff flag", open: []string{"--kind", "fyi"}, answer: []string{"--accept"}, wantErr: "fyi answers cannot set signoff"},
		{name: "no response", open: []string{"--kind", "fyi"}, answer: []string{"--note", "hm"}, wantErr: "no fyi response"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			root := t.TempDir()
			id := strings.TrimSpace(mustRun(t, append([]string{"--store", root, "open", "--urgency", "today", "--title", "T"}, tt.open...)...))
			r := runCases(t, "", append([]string{"--store", root, "answer", id}, tt.answer...)...)
			c := loadCase(t, root, id)
			if tt.wantErr != "" {
				if r.err == nil || !strings.Contains(r.err.Error(), tt.wantErr) {
					t.Fatalf("err = %v, want %q", r.err, tt.wantErr)
				}
				if c.State != store.StateOpen || len(c.Events) != 1 {
					t.Errorf("refused answer changed the case: %s, %d events", c.State, len(c.Events))
				}
				return
			}
			if r.err != nil {
				t.Fatal(r.err)
			}
			if r.stdout != id+" "+string(tt.wantState)+"\n" {
				t.Errorf("stdout = %q", r.stdout)
			}
			if c.State != tt.wantState {
				t.Errorf("state = %s, want %s", c.State, tt.wantState)
			}
			if tt.check != nil {
				tt.check(t, c)
			}
		})
	}
}

func TestAnswerTextFile(t *testing.T) {
	file := filepath.Join(t.TempDir(), "reply.md")
	if err := os.WriteFile(file, []byte("# The staging one\n\nIt has the fixtures.\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	empty := filepath.Join(t.TempDir(), "empty.md")
	if err := os.WriteFile(empty, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name     string
		stdin    string
		args     []string
		wantText string
		wantErr  string
	}{
		{name: "file", args: []string{"--text-file", file}, wantText: "# The staging one\n\nIt has the fixtures.\n"},
		{name: "stdin", stdin: "use the mirror\n", args: []string{"--text-file", "-"}, wantText: "use the mirror\n"},
		{name: "empty file", args: []string{"--text-file", empty}, wantErr: "answer text is empty"},
		{name: "empty stdin", args: []string{"--text-file", "-"}, wantErr: "answer text is empty"},
		{name: "missing file", args: []string{"--text-file", filepath.Join(t.TempDir(), "missing.md")}, wantErr: "text:"},
		{name: "with --text", args: []string{"--text", "x", "--text-file", file}, wantErr: "--text and --text-file can't be used together"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			root := t.TempDir()
			id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "question", "--urgency", "today", "--title", "T"))
			r := runCases(t, tt.stdin, append([]string{"--store", root, "answer", id}, tt.args...)...)
			c := loadCase(t, root, id)
			if tt.wantErr != "" {
				if r.err == nil || !strings.Contains(r.err.Error(), tt.wantErr) {
					t.Fatalf("err = %v, want %q", r.err, tt.wantErr)
				}
				if len(c.Events) != 1 {
					t.Errorf("refused answer wrote %d events", len(c.Events))
				}
				return
			}
			if r.err != nil {
				t.Fatal(r.err)
			}
			if c.Answer == nil || c.Answer.Text != tt.wantText {
				t.Errorf("answer = %+v", c.Answer)
			}
		})
	}
}

// eventFiles names the files in a case directory, so a test can tell that a
// refused write left nothing behind.
func eventFiles(t *testing.T, root, id string) []string {
	t.Helper()
	entries, err := os.ReadDir(filepath.Join(root, id))
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, e := range entries {
		names = append(names, e.Name())
	}
	return names
}

// refusedAsStale runs a write that should be refused because the case has
// moved on from revision read to revision now, and checks that it wrote
// nothing.
func refusedAsStale(t *testing.T, root, id string, read, now int, args ...string) {
	t.Helper()
	before := eventFiles(t, root, id)
	r := runCases(t, "", append([]string{"--store", root}, args...)...)
	if !errors.Is(r.err, store.ErrStale) {
		t.Fatalf("err = %v, want ErrStale", r.err)
	}
	want := fmt.Sprintf("read at revision %d, now at %d", read, now)
	if msg := r.err.Error(); !strings.Contains(msg, want) || strings.Contains(msg, "\n") {
		t.Errorf("err = %q, want one line containing %q", msg, want)
	}
	if after := eventFiles(t, root, id); len(after) != len(before) {
		t.Errorf("refused write changed the files: %v, then %v", before, after)
	}
}

func TestAnswerAtRevision(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "amend", id, "--option", "Vendor it")

	refusedAsStale(t, root, id, 1, 2, "answer", id, "--option", "3", "--revision", "1")
	refusedAsStale(t, root, id, 0, 2, "answer", id, "--option", "3", "--revision", "0")
	if out := mustRun(t, "--store", root, "answer", id, "--option", "3", "--revision", "2"); out != id+" answered\n" {
		t.Errorf("stdout = %q", out)
	}
	if c := loadCase(t, root, id); c.Answer == nil || c.Answer.Choice != 3 || c.Revision() != 3 {
		t.Errorf("answer = %+v, revision %d", c.Answer, c.Revision())
	}
}

func TestAnswerParkAtRevision(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "stuck", "--urgency", "blocking", "--title", "Blocked"))
	mustRun(t, "--store", root, "amend", id, "--context", "CI is red")

	refusedAsStale(t, root, id, 1, 2, "answer", id, "--park", "--revision", "1")
	if out := mustRun(t, "--store", root, "answer", id, "--park", "--note", "after release", "--revision", "2"); out != id+" parked\n" {
		t.Errorf("stdout = %q", out)
	}
	if c := loadCase(t, root, id); c.State != store.StateParked || c.Revision() != 3 {
		t.Errorf("state %s, revision %d", c.State, c.Revision())
	}
}

func TestAnswerTakesPartOfAnID(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	if out := mustRun(t, "--store", root, "answer", "pin-bun", "--option", "1"); out != id+" answered\n" {
		t.Errorf("stdout = %q", out)
	}
	// Agent commands still take the exact id only.
	if r := runCases(t, "", "--store", root, "pickup", "pin-bun"); r.err == nil {
		t.Error("pickup took part of an id")
	}
}

func TestNameConfigStampsTheHumanOnAnswer(t *testing.T) {
	root := t.TempDir()
	writeConfig(t, "name = \"Ryan\"\n")
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "decision", "--urgency", "today",
		"--title", "Pin bun?", "--option", "Pin", "--worker", "bun-pins", "--for", "Ryan"))
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	mustRun(t, "--store", root, "pickup", id)

	c := loadCase(t, root, id)
	if c.For != "Ryan" {
		t.Errorf("for = %q", c.For)
	}
	for i, want := range []store.Actor{{Name: "bun-pins", Kind: "agent"}, {Name: "Ryan", Kind: "human"}, {Name: "bun-pins", Kind: "agent"}} {
		if got := c.Events[i].Actor; got == nil || *got != want {
			t.Errorf("event %d actor = %+v, want %+v", i+1, got, want)
		}
	}
	out := mustRun(t, "--store", root, "show", id)
	for _, want := range []string{"for:      Ryan", "by Ryan"} {
		if !strings.Contains(out, want) {
			t.Errorf("show missing %q:\n%s", want, out)
		}
	}

	// --as beats the config file; without a worker an agent event records no actor.
	other := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Heads up"))
	mustRun(t, "--store", root, "answer", other, "--ack", "--as", "Sam")
	c = loadCase(t, root, other)
	if c.Events[0].Actor != nil {
		t.Errorf("open without a worker has actor %+v", c.Events[0].Actor)
	}
	if got := c.Events[1].Actor; got == nil || got.Name != "Sam" {
		t.Errorf("answer --as actor = %+v", got)
	}
}

// A blank worker, which open has always accepted, records no actor rather than
// one the store refuses.
func TestBlankWorkerRecordsNoActor(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today",
		"--title", "Heads up", "--worker", " "))
	mustRun(t, "--store", root, "note", id, "--body", "more")
	c := loadCase(t, root, id)
	for i, ev := range c.Events {
		if ev.Actor != nil {
			t.Errorf("event %d actor = %+v", i+1, ev.Actor)
		}
	}
}
