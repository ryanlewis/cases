package main

import (
	"fmt"

	"github.com/ryanlewis/cases/internal/store"
)

type NoteCmd struct {
	ID       string `arg:"" help:"Case id."`
	Body     string `help:"The follow-up as one line of text." xor:"body" required:"" placeholder:"TEXT"`
	BodyFile string `help:"Markdown follow-up. - reads stdin." name:"body-file" xor:"body" required:"" placeholder:"FILE"`
}

func (c *NoteCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
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
	cs, err := store.Note(dir, store.NoteRecord{Body: body})
	if err != nil {
		return err
	}
	return d.done(cs)
}
