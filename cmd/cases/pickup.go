package main

import (
	"context"

	"github.com/ryanlewis/cases/internal/store"
)

type PickupCmd struct {
	ID       string `arg:"" help:"Case id."`
	By       string `help:"Who picked it up, e.g. the agent session name."`
	Revision *int   `help:"Refuse the pickup if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *PickupCmd) Run(d *Deps) error {
	if err := store.ValidID(c.ID); err != nil {
		return err
	}
	cs, err := d.cases().Pickup(context.Background(), c.ID, store.PickupRecord{By: c.By, Actor: d.workerActor(c.ID)}, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
