package main

import (
	"fmt"

	"github.com/ryanlewis/cases/internal/store"
)

type CloseCmd struct {
	ID          string   `arg:"" help:"Case id."`
	OutcomeFile string   `help:"Markdown outcome. - reads stdin." name:"outcome-file" required:"" placeholder:"FILE"`
	Link        []string `help:"A link to evidence of the outcome. Repeatable." sep:"none" placeholder:"URL"`
}

func (c *CloseCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	outcome, err := d.readText(c.OutcomeFile)
	if err != nil {
		return fmt.Errorf("outcome: %w", err)
	}
	cs, err := store.Close(dir, store.CloseRecord{Outcome: outcome, Links: c.Link})
	if err != nil {
		return err
	}
	return d.done(cs)
}
