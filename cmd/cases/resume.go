package main

import "github.com/ryanlewis/cases/internal/store"

type ResumeCmd struct {
	ID       string `arg:"" help:"Case id."`
	Agent    bool   `help:"Record the resume as written by the agent rather than the human."`
	Revision *int   `help:"Refuse the resume if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *ResumeCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	author := store.AuthorHuman
	if c.Agent {
		author = store.AuthorAgent
	}
	cs, err := store.Resume(dir, author, store.ResumeRecord{}, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
