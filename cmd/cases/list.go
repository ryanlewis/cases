package main

import (
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type ListCmd struct {
	State []string `help:"Only cases in these states (open, answered, pickedup, closed, withdrawn, parked). Repeat or comma-separate." placeholder:"STATE"`
	JSON  bool     `help:"Print JSON." short:"j"`
}

func (c *ListCmd) Run(d *Deps) error {
	var want []store.State
	for _, s := range c.State {
		st, err := store.ParseState(s)
		if err != nil {
			return err
		}
		want = append(want, st)
	}
	cases, bad, err := store.List(d.Store)
	if err != nil {
		return err
	}
	for _, b := range bad {
		fmt.Fprintf(d.Stderr, "warning: %v\n", b)
	}

	shown := []*store.Case{}
	for _, cs := range cases {
		d.warn(cs)
		if len(want) == 0 || slices.Contains(want, cs.State) {
			shown = append(shown, cs)
		}
	}
	// Inbox order: most urgent first, then oldest first.
	slices.SortStableFunc(shown, func(a, b *store.Case) int {
		if r := a.Urgency.Rank() - b.Urgency.Rank(); r != 0 {
			return r
		}
		return strings.Compare(a.ID, b.ID)
	})

	if c.JSON {
		enc := json.NewEncoder(d.Stdout)
		enc.SetIndent("", "  ")
		enc.SetEscapeHTML(false)
		return enc.Encode(shown)
	}
	if len(shown) == 0 {
		fmt.Fprintln(d.Stdout, "No cases.")
		return nil
	}
	tw := tabwriter.NewWriter(d.Stdout, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "ID\tSTATE\tURGENCY\tKIND\tAGE\tTITLE")
	for _, cs := range shown {
		fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\n", cs.ID, cs.State, cs.Urgency, cs.Kind, age(cs.OpenedAt), cs.Title)
	}
	return tw.Flush()
}

// age renders how long ago t was, coarsely.
func age(t time.Time) string {
	if t.IsZero() {
		return "-"
	}
	d := time.Since(t)
	switch {
	case d < time.Minute:
		return "now"
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	case d < 48*time.Hour:
		return fmt.Sprintf("%dh", int(d.Hours()))
	default:
		return fmt.Sprintf("%dd", int(d.Hours()/24))
	}
}
