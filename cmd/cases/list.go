package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"slices"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/ryanlewis/cases/internal/store"
	"github.com/ryanlewis/cases/internal/web"
)

type ListCmd struct {
	State     []string      `help:"Only cases in these states (open, answered, pickedup, closed, withdrawn, parked). Repeat or comma-separate. Without it, open and parked." placeholder:"STATE"`
	All       bool          `help:"Cases in every state. --state wins over it."`
	Urgency   []string      `help:"Only cases with this urgency (blocking, today, whenever). Repeat for any of them." enum:"blocking,today,whenever" sep:"none" placeholder:"URGENCY"`
	OlderThan time.Duration `help:"Only cases whose last event is older than this Go duration (30m), so a resumed case counts from its resume. 0 means any age." default:"0" placeholder:"DURATION"`
	CaseFilter
	Count bool `help:"Print only the number of matching cases."`
	JSON  bool `help:"Print JSON." short:"j"`
}

// CaseFilter picks cases by kind, label and worker, for list, wait and sweep.
type CaseFilter struct {
	Kind   []string `help:"Only cases of this kind (decision, approval, signoff, stuck, question, fyi). Repeat for any of them." enum:"decision,approval,signoff,stuck,question,fyi" sep:"none" placeholder:"KIND"`
	Label  []string `help:"Only cases with this label. Repeat for cases with any of them." sep:"none" placeholder:"TEXT"`
	Worker []string `help:"Only cases from this worker. Repeat for cases from any of them." sep:"none" placeholder:"NAME"`
}

// match reports whether the case has one of the kinds, one of the labels and
// one of the workers asked for. A filter left empty matches every case.
func (f CaseFilter) match(c *store.Case) bool {
	if len(f.Kind) > 0 && !slices.Contains(f.Kind, string(c.Kind)) {
		return false
	}
	if len(f.Label) > 0 && !slices.ContainsFunc(c.Labels, func(l string) bool { return slices.Contains(f.Label, l) }) {
		return false
	}
	return len(f.Worker) == 0 || slices.Contains(f.Worker, c.Worker)
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
	if len(want) == 0 && !c.All {
		want = []store.State{store.StateOpen, store.StateParked}
	}
	cases, bad, err := d.cases().List(context.Background())
	if errors.Is(err, fs.ErrNotExist) {
		// A store nobody has written to yet reads as empty.
		cases, bad, err = nil, nil, nil
	}
	if err != nil {
		return err
	}
	for _, b := range bad {
		fmt.Fprintf(d.Stderr, "warning: %v\n", b)
	}

	now := time.Now()
	shown := []*store.Case{}
	for _, cs := range cases {
		d.warn(cs, nil)
		if (len(want) == 0 || slices.Contains(want, cs.State)) &&
			(len(c.Urgency) == 0 || slices.Contains(c.Urgency, string(cs.Urgency))) &&
			now.Sub(cs.UpdatedAt) >= c.OlderThan && c.match(cs) {
			shown = append(shown, cs)
		}
	}
	store.SortInbox(shown)

	if c.Count {
		fmt.Fprintln(d.Stdout, len(shown))
		return nil
	}

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
	// Labels come before the title so a long title does not push them off the line.
	fmt.Fprintln(tw, "ID\tSTATE\tURGENCY\tKIND\tAGE\tLABELS\tTITLE")
	for _, cs := range shown {
		fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n", cs.ID, cs.State, cs.Urgency, cs.Kind, web.Age(cs.OpenedAt, now), strings.Join(cs.Labels, ","), cs.Title)
	}
	return tw.Flush()
}
