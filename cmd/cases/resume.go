package main

import (
	"context"

	"github.com/ryanlewis/cases/internal/store"
)

type ResumeCmd struct {
	ID       string `arg:"" help:"Case id, or any part of it that names one case."`
	Agent    bool   `help:"Record the resume as written by the agent rather than the human."`
	Revision *int   `help:"Refuse the resume if the case's revision (from show --json) is no longer N." placeholder:"N"`
	As       string `help:"Record the resume as written by NAME (the name config key). With --agent, the case's worker is recorded instead." placeholder:"NAME"`
}

func (c *ResumeCmd) Run(d *Deps) error {
	id, err := d.findCase(c.ID)
	if err != nil {
		return err
	}
	author, actor := store.AuthorHuman, humanActor(c.As)
	if c.Agent {
		author, actor = store.AuthorAgent, d.workerActor(id)
	}
	cs, err := d.cases().Resume(context.Background(), id, author, store.ResumeRecord{Actor: actor}, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
