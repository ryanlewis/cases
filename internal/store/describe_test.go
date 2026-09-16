package store

import (
	"slices"
	"testing"
)

func TestDescribeWithdraw(t *testing.T) {
	for _, tc := range []struct {
		rec  WithdrawRecord
		want []string
	}{
		{WithdrawRecord{}, nil},
		{WithdrawRecord{Reason: "found it in the lockfile"}, []string{"reason: found it in the lockfile"}},
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
		want []string
	}{
		{KindDecision, AmendRecord{Options: []string{"Vendor it"}}, []string{"added option: Vendor it"}},
		{KindFYI, AmendRecord{Labels: []string{"feat-labels", "round 3"}}, []string{"added label: feat-labels", "added label: round 3"}},
		{
			KindApproval,
			AmendRecord{
				Body:    "Three scripts now.",
				Rows:    []Row{{ID: "c", Label: "Deploy", Script: "make deploy", Link: "https://example.com/c"}},
				Links:   []string{"https://example.com/log"},
				Context: "Release 1.4.1\nafter the freeze",
			},
			[]string{"replaced the body", "added row [c] Deploy", "added link: https://example.com/log", "replaced the context: Release 1.4.1", "after the freeze"},
		},
	} {
		c, _, err := fold(t, agent(EventOpen, openOf(tc.kind)), agent(EventAmend, tc.rec))
		if err != nil {
			t.Fatal(err)
		}
		if got := c.Describe(c.Events[len(c.Events)-1]); !slices.Equal(got, tc.want) {
			t.Errorf("Describe(%+v) = %q, want %q", tc.rec, got, tc.want)
		}
	}
}
