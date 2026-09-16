package store

import (
	"path/filepath"
	"slices"
	"testing"
)

func TestDescribeWithdraw(t *testing.T) {
	for _, tc := range []struct {
		rec  WithdrawRecord
		want []Line
	}{
		{WithdrawRecord{}, nil},
		{WithdrawRecord{Reason: "found it in the lockfile"}, []Line{{Text: "reason: found it in the lockfile"}}},
	} {
		c, _, err := fold(t, agent(EventOpen, openOf(KindFYI)), agent(EventWithdraw, tc.rec))
		if err != nil {
			t.Fatal(err)
		}
		if got := c.Describe(c.Events[len(c.Events)-1]); !slices.Equal(got, tc.want) {
			t.Errorf("Describe(%+v) = %q, want %q", tc.rec, got, tc.want)
		}
	}
}

func TestDescribeAmend(t *testing.T) {
	for _, tc := range []struct {
		kind Kind
		rec  AmendRecord
		want []Line
	}{
		{KindDecision, AmendRecord{Options: []string{"Vendor it"}}, []Line{{Text: "added option: Vendor it"}}},
		{KindFYI, AmendRecord{Labels: []string{"feat-labels", "round 3"}}, []Line{{Text: "added label: feat-labels"}, {Text: "added label: round 3"}}},
		{
			KindApproval,
			AmendRecord{
				Body:    "Three scripts now.",
				Rows:    []Row{{ID: "c", Label: "Deploy", Script: "make deploy", Link: "https://example.com/c"}},
				Links:   []string{"https://example.com/log"},
				Context: "Release 1.4.1\nafter the freeze",
			},
			// The text replaced goes with the last line saying so.
			[]Line{
				{Text: "replaced the body", PreviousBody: "Body."},
				{Text: "added row [c] Deploy"},
				{Text: "added link: https://example.com/log"},
				{Text: "replaced the context: Release 1.4.1"},
				{Text: "after the freeze", PreviousContext: "Release 1.4"},
			},
		},
	} {
		open := openOf(tc.kind)
		open.Context = "Release 1.4"
		c, _, err := fold(t, agent(EventOpen, open), agent(EventAmend, tc.rec))
		if err != nil {
			t.Fatal(err)
		}
		if got := c.Describe(c.Events[len(c.Events)-1]); !slices.Equal(got, tc.want) {
			t.Errorf("Describe(%+v) = %q, want %q", tc.rec, got, tc.want)
		}
	}
}

// Each amend carries the body and context it replaced: the open event's, or
// the last amend's before it. A case that had none, or only space, has
// nothing to carry.
func TestDescribeAmendPreviousText(t *testing.T) {
	open := openOf(KindStuck)
	open.Body = " \n"
	c, _, err := fold(t,
		agent(EventOpen, open),
		agent(EventAmend, AmendRecord{Body: "Now with the logs.", Context: "Release 1.4"}),
		agent(EventAmend, AmendRecord{Links: []string{"https://example.com/log"}}),
		agent(EventAmend, AmendRecord{Body: "Now with the fix.", Context: "Release 1.4.1"}),
	)
	if err != nil {
		t.Fatal(err)
	}
	want := [][]Line{
		nil,
		{{Text: "replaced the body"}, {Text: "replaced the context: Release 1.4"}},
		{{Text: "added link: https://example.com/log"}},
		{
			{Text: "replaced the body", PreviousBody: "Now with the logs."},
			{Text: "replaced the context: Release 1.4.1", PreviousContext: "Release 1.4"},
		},
	}
	for i, ev := range c.Events {
		if got := c.Describe(ev); !slices.Equal(got, want[i]) {
			t.Errorf("event %d: Describe = %q, want %q", i+1, got, want[i])
		}
	}
}

// An amend that sets the body and context the case already has, beside a new
// link, changed only the links, so the thread says only that.
func TestDescribeAmendSameText(t *testing.T) {
	open := openOf(KindFYI)
	open.Context = "Release 1.4"
	c, _, err := fold(t,
		agent(EventOpen, open),
		agent(EventAmend, AmendRecord{Body: open.Body, Context: open.Context, Links: []string{"https://example.com/log"}}),
	)
	if err != nil {
		t.Fatal(err)
	}
	want := []Line{{Text: "added link: https://example.com/log"}}
	if got := c.Describe(c.Events[1]); !slices.Equal(got, want) {
		t.Errorf("Describe = %q, want %q", got, want)
	}
}

// Files written by the build that added labels to amend, byte for byte, fold
// to the case that build showed, and the thread now has what each amend
// replaced.
func TestAmendFilesFromAnEarlierBuild(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "2026-09-16T22-15-41Z-pin-bun-or-float")
	writeFile(t, dir, "0001-agent-open.json", `{
  "kind": "decision",
  "urgency": "today",
  "title": "Pin bun or float",
  "body": "# Pin bun?\n\nWe use bun 1.2 in CI.\n\n- **Pin**: stable\n- Float: \u003cnewer\u003e \u0026 riskier\n",
  "options": [
    "Pin to 1.2.3",
    "Float"
  ],
  "links": [
    "https://example.com/pr"
  ],
  "labels": [
    "round-3"
  ],
  "context": "Release 1.4",
  "opened_at": "2026-09-16T22:15:41.605108Z"
}
`)
	writeFile(t, dir, "0002-agent-amend.json", `{
  "body": "# Pin bun?\n\nNow with the lockfile diff.\n",
  "links": [
    "https://example.com/log"
  ],
  "context": "Release 1.4.1\nafter the freeze",
  "amended_at": "2026-09-16T22:15:41.632062Z"
}
`)
	writeFile(t, dir, "0003-agent-amend.json", `{
  "links": [
    "https://example.com/diff"
  ],
  "amended_at": "2026-09-16T22:15:41.652736Z"
}
`)
	writeFile(t, dir, "0004-agent-amend.json", `{
  "labels": [
    "review"
  ],
  "amended_at": "2026-09-16T22:15:41.668651Z"
}
`)
	c, err := Load(dir)
	if err != nil {
		t.Fatal(err)
	}
	if c.State != StateOpen || c.Revision() != 4 || len(c.Events) != 4 || len(c.Problems) != 0 {
		t.Fatalf("state %s, revision %d, %d events, problems %q", c.State, c.Revision(), len(c.Events), c.Problems)
	}
	if c.Body != "# Pin bun?\n\nNow with the lockfile diff.\n" || c.Context != "Release 1.4.1\nafter the freeze" ||
		!slices.Equal(c.Options, []string{"Pin to 1.2.3", "Float"}) ||
		!slices.Equal(c.Links, []string{"https://example.com/pr", "https://example.com/log", "https://example.com/diff"}) ||
		!slices.Equal(c.Labels, []string{"round-3", "review"}) {
		t.Errorf("case = %+v", c.OpenRecord)
	}
	want := [][]Line{
		nil,
		{
			{Text: "replaced the body", PreviousBody: "# Pin bun?\n\nWe use bun 1.2 in CI.\n\n- **Pin**: stable\n- Float: <newer> & riskier\n"},
			{Text: "added link: https://example.com/log"},
			{Text: "replaced the context: Release 1.4.1"},
			{Text: "after the freeze", PreviousContext: "Release 1.4"},
		},
		{{Text: "added link: https://example.com/diff"}},
		{{Text: "added label: review"}},
	}
	for i, ev := range c.Events {
		if got := c.Describe(ev); !slices.Equal(got, want[i]) {
			t.Errorf("%s: Describe = %q, want %q", ev.File, got, want[i])
		}
	}
}
