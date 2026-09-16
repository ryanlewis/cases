package main

import (
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
	State []string `help:"Only cases in these states (open, answered, pickedup, closed, withdrawn, parked). Repeat or comma-separate." placeholder:"STATE"`
	CaseFilter
	JSON bool `help:"Print JSON." short:"j"`
}

// CaseFilter picks cases by label and worker, for list and wait.
type CaseFilter struct {
	Label  []string `help:"Only cases with this label. Repeat for cases with any of them." sep:"none" placeholder:"TEXT"`
	Worker []string `help:"Only cases from this worker. Repeat for cases from any of them." sep:"none" placeholder:"NAME"`
}

// match reports whether the case has one of the labels and one of the workers
// asked for. A filter left empty matches every case.
func (f CaseFilter) match(c *store.Case) bool {
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
	cases, bad, err := store.List(d.Store)
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

	shown := []*store.Case{}
	for _, cs := range cases {
		d.warn(cs, nil)
		if (len(want) == 0 || slices.Contains(want, cs.State)) && c.match(cs) {
			shown = append(shown, cs)
		}
	}
	store.SortInbox(shown)

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
		fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n", cs.ID, cs.State, cs.Urgency, cs.Kind, web.Age(cs.OpenedAt, time.Now()), strings.Join(cs.Labels, ","), cs.Title)
	}
	return tw.Flush()
}
