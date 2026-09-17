package main

import (
	"context"
	"fmt"

	"github.com/ryanlewis/cases/internal/store"
)

type CloseCmd struct {
	ID          string   `arg:"" help:"Case id."`
	Outcome     string   `help:"The outcome as one line of text." xor:"outcome" required:"" placeholder:"TEXT"`
	OutcomeFile string   `help:"Markdown outcome. - reads stdin." name:"outcome-file" xor:"outcome" required:"" placeholder:"FILE"`
	Link        []string `help:"A link to evidence of the outcome. Repeatable." sep:"none" placeholder:"URL"`
	Revision    *int     `help:"Refuse the close if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *CloseCmd) Run(d *Deps) error {
	err := store.ValidID(c.ID)
	if err != nil {
		return err
	}
	var outcome string
	if c.OutcomeFile != "" {
		outcome, err = d.readText(c.OutcomeFile)
	} else {
		outcome, err = inlineText("outcome", c.Outcome)
	}
	if err != nil {
		return fmt.Errorf("outcome: %w", err)
	}
	cs, err := d.cases().Close(context.Background(), c.ID, store.CloseRecord{Outcome: outcome, Links: c.Link}, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
