package main

import "github.com/ryanlewis/cases/internal/store"

type ResumeCmd struct {
	ID    string `arg:"" help:"Case id."`
	Agent bool   `help:"Record the resume as written by the agent rather than the human."`
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
	cs, err := store.Resume(dir, author, store.ResumeRecord{})
	if err != nil {
		return err
	}
	return d.done(cs)
}
