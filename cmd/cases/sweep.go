package main

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type SweepCmd struct {
	Reason    string        `help:"Reason recorded on each withdraw." default:"swept" placeholder:"TEXT"`
	OlderThan time.Duration `help:"Only cases opened longer ago than this Go duration (72h). 0 means any age." default:"0" placeholder:"DURATION"`
	CaseFilter
	Yes bool `help:"Withdraw the cases. Without it, sweep only prints what it would withdraw." short:"y"`
}

// Run withdraws the open cases that match. Withdraw is allowed only on an open
// case, so an answered or parked case that matches is listed and left alone.
// A case that refuses the withdraw does not stop the rest; sweep fails at the
// end if any did.
func (c *SweepCmd) Run(d *Deps) error {
	cases, bad, err := d.Cases.List(context.Background())
	if errors.Is(err, fs.ErrNotExist) {
		cases, bad, err = nil, nil, nil
	}
	if err != nil {
		return err
	}
	for _, b := range bad {
		fmt.Fprintf(d.Stderr, "warning: %v\n", b)
	}

	now := time.Now()
	var sweep []*store.Case
	for _, cs := range cases {
		if !c.match(cs) || now.Sub(cs.OpenedAt) < c.OlderThan {
			continue
		}
		switch cs.State {
		case store.StateOpen:
			sweep = append(sweep, cs)
		case store.StateAnswered, store.StateParked:
			fmt.Fprintf(d.Stdout, "%s %s, left\n", cs.ID, cs.State)
		}
	}

	if !c.Yes {
		for _, cs := range sweep {
			fmt.Fprintf(d.Stdout, "%s would be withdrawn: %s\n", cs.ID, cs.Title)
		}
		if len(sweep) > 0 {
			fmt.Fprintf(d.Stderr, "%d cases would be withdrawn. Pass --yes to withdraw them.\n", len(sweep))
		}
		return nil
	}

	var failed []string
	for _, cs := range sweep {
		withdrawn, err := d.Cases.Withdraw(context.Background(), cs.ID, store.WithdrawRecord{Reason: c.Reason})
		time.Sleep(writePause)
		if err != nil {
			failed = append(failed, cs.ID+": "+err.Error())
			continue
		}
		_ = d.done(withdrawn)
	}
	if len(failed) > 0 {
		return fmt.Errorf("%d cases were not withdrawn:\n  %s", len(failed), strings.Join(failed, "\n  "))
	}
	return nil
}
