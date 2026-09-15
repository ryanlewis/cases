package main

import (
	"fmt"

	"github.com/ryanlewis/cases/internal/store"
)

type NoteCmd struct {
	ID       string `arg:"" help:"Case id."`
	BodyFile string `help:"Markdown follow-up. - reads stdin." name:"body-file" required:"" placeholder:"FILE"`
}

func (c *NoteCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	body, err := d.readText(c.BodyFile)
	if err != nil {
		return fmt.Errorf("body: %w", err)
	}
	cs, err := store.Note(dir, store.NoteRecord{Body: body})
	if err != nil {
		return err
	}
	return d.done(cs)
}
