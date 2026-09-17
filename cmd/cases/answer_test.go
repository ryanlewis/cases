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
