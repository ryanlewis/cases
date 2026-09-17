package main

import (
	"context"
	"errors"
	"fmt"

	"github.com/ryanlewis/cases/internal/store"
)

// AmendCmd's Body, BodyFile and Context are pointers so that a flag given an empty
// value can be told from a flag left out.
type AmendCmd struct {
	ID       string   `arg:"" help:"Case id."`
	Body     *string  `help:"One line of text that replaces the body." xor:"body" placeholder:"TEXT"`
	BodyFile *string  `help:"Markdown that replaces the body. - reads stdin." name:"body-file" xor:"body" placeholder:"FILE"`
	Option   []string `help:"An option to add to a decision case, numbered after the options it has. Repeat per option." sep:"none" placeholder:"TEXT"`
	Row      []string `help:"A row to add to an approval case, as a JSON object with id, label, script, link and an optional note. The id must not be on the case already. Repeat per row." sep:"none" placeholder:"JSON"`
	Link     []string `help:"A link to add to the case. Repeatable." sep:"none" placeholder:"URL"`
	Label    []string `help:"A label to add to the case. Repeatable." sep:"none" placeholder:"TEXT"`
	Context  *string  `help:"Free-text context that replaces the case's." placeholder:"STRING"`
	Revision *int     `help:"Refuse the amend if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *AmendCmd) Run(d *Deps) error {
	if err := store.ValidID(c.ID); err != nil {
		return err
	}
	rows, err := parseRows(c.Row)
	if err != nil {
		return err
	}
	rec := store.AmendRecord{Options: c.Option, Rows: rows, Links: c.Link, Labels: c.Label}
	// The store reads an empty body or context as no change, which is not
	// what an empty --body, --body-file or --context asks for. The store
	// refuses one that is only whitespace.
	if c.Body != nil {
		if rec.Body, err = inlineText("body", *c.Body); err != nil {
			return fmt.Errorf("body: %w", err)
		}
		if rec.Body == "" {
			return errors.New("amend body is empty")
		}
	}
	if c.BodyFile != nil {
		if rec.Body, err = d.readText(*c.BodyFile); err != nil {
			return fmt.Errorf("body: %w", err)
		}
		if rec.Body == "" {
			return errors.New("amend body is empty")
		}
	}
	if c.Context != nil {
		if *c.Context == "" {
			return errors.New("amend context is empty")
		}
		rec.Context = *c.Context
	}
	cs, err := d.cases().Amend(context.Background(), c.ID, rec, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
