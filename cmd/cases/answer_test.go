package main

import (
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
