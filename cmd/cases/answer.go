package main

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/ryanlewis/cases/internal/store"
)

type AnswerCmd struct {
	ID string `arg:"" help:"Case id, or any part of it that names one case."`

	Option   int      `help:"decision: choose option N (1-based)." xor:"response" placeholder:"N"`
	Other    bool     `help:"decision: Other, see note (needs --note)." xor:"response"`
	Row      []string `help:"approval: a verdict per row as id=approve|hold|reject, optionally id=verdict:note. Repeat for every row." xor:"response" sep:"none" placeholder:"ID=VERDICT"`
	Accept   bool     `help:"signoff: accept." xor:"response"`
	Changes  bool     `help:"signoff: request changes (needs --note)." xor:"response"`
	Text     string   `help:"stuck: guidance for the agent. question: the reply." xor:"response"`
	TextFile string   `help:"--text read from a file. - reads stdin." name:"text-file" xor:"response" placeholder:"FILE"`
	Park     bool     `help:"stuck: park the case until someone resumes it." xor:"response"`
	Drop     bool     `help:"any kind: drop the case." xor:"response"`
	Ack      bool     `help:"fyi: acknowledge." xor:"response"`

	Note     string `help:"A note to the agent. Allowed with every response."`
	Revision *int   `help:"Refuse the answer if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *AnswerCmd) Run(d *Deps) error {
	id, err := d.findCase(c.ID)
	if err != nil {
		return err
	}
	if c.Park {
		cs, err := d.cases().Park(context.Background(), id, store.ParkRecord{Note: c.Note}, atRevision(c.Revision)...)
		if err != nil {
			return err
		}
		return d.done(cs)
	}

	// The store reads empty text as no response given, which is not what an
	// empty --text-file is.
	if c.TextFile != "" {
		if c.Text, err = d.readText(c.TextFile); err != nil {
			return fmt.Errorf("text: %w", err)
		}
		if c.Text == "" {
			return errors.New("answer text is empty")
		}
	}

	rec := store.AnswerRecord{
		Choice: c.Option,
		Other:  c.Other,
		Text:   c.Text,
		Drop:   c.Drop,
		Ack:    c.Ack,
		Note:   c.Note,
	}
	switch {
	case c.Accept:
		rec.Signoff = store.SignoffAccept
	case c.Changes:
		rec.Signoff = store.SignoffChanges
	}
	for _, raw := range c.Row {
		row, err := parseRowVerdict(raw)
		if err != nil {
			return err
		}
		rec.Rows = append(rec.Rows, row)
	}
	cs, err := d.cases().Answer(context.Background(), id, rec, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}

// atRevision is the precondition for a --revision flag, or none when it was
// not given.
func atRevision(rev *int) []store.Precondition {
	if rev == nil {
		return nil
	}
	return []store.Precondition{store.AtRevision(*rev)}
}

// parseRowVerdict reads id=verdict or id=verdict:note.
func parseRowVerdict(s string) (store.RowAnswer, error) {
	id, rest, ok := strings.Cut(s, "=")
	if !ok || id == "" {
		return store.RowAnswer{}, fmt.Errorf("--row %q: want id=approve|hold|reject", s)
	}
	verdict, note, _ := strings.Cut(rest, ":")
	if verdict == "" {
		return store.RowAnswer{}, errors.New("--row " + id + ": verdict is empty")
	}
	return store.RowAnswer{ID: id, Verdict: verdict, Note: note}, nil
}
