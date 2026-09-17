package main

import (
	"context"
	"fmt"

	"github.com/ryanlewis/cases/internal/store"
)

type NoteCmd struct {
	ID       string `arg:"" help:"Case id."`
	Body     string `help:"The follow-up as one line of text." xor:"body" required:"" placeholder:"TEXT"`
	BodyFile string `help:"Markdown follow-up. - reads stdin." name:"body-file" xor:"body" required:"" placeholder:"FILE"`
	Revision *int   `help:"Refuse the note if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *NoteCmd) Run(d *Deps) error {
	err := store.ValidID(c.ID)
	if err != nil {
		return err
	}
	var body string
	if c.BodyFile != "" {
		body, err = d.readText(c.BodyFile)
	} else {
		body, err = inlineText("body", c.Body)
	}
	if err != nil {
		return fmt.Errorf("body: %w", err)
	}
	cs, err := d.cases().Note(context.Background(), c.ID, store.NoteRecord{Body: body, Actor: d.workerActor(c.ID)}, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
