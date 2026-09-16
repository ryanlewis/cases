package main

import (
	"errors"
	"fmt"
	"strings"

	"github.com/ryanlewis/cases/internal/store"
)

type AnswerCmd struct {
	ID string `arg:"" help:"Case id."`

	Option  int      `help:"decision: choose option N (1-based)." xor:"response" placeholder:"N"`
	Other   bool     `help:"decision: Other, see note (needs --note)." xor:"response"`
	Row     []string `help:"approval: a verdict per row as id=approve|hold|reject, optionally id=verdict:note. Repeat for every row." xor:"response" sep:"none" placeholder:"ID=VERDICT"`
	Accept  bool     `help:"signoff: accept." xor:"response"`
	Changes bool     `help:"signoff: request changes (needs --note)." xor:"response"`
	Text    string   `help:"stuck: guidance for the agent." xor:"response"`
	Park    bool     `help:"stuck: park the case until someone resumes it." xor:"response"`
	Drop    bool     `help:"stuck: drop the work." xor:"response"`
	Ack     bool     `help:"fyi: acknowledge." xor:"response"`

	Note string `help:"A note to the agent. Allowed with every response."`
}

func (c *AnswerCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	if c.Park {
		cs, err := store.Park(dir, store.ParkRecord{Note: c.Note})
		if err != nil {
			return err
		}
		return d.done(cs)
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
	cs, err := store.Answer(dir, rec)
	if err != nil {
		return err
	}
	return d.done(cs)
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
