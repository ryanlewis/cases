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
