package main

import "github.com/ryanlewis/cases/internal/store"

type PickupCmd struct {
	ID string `arg:"" help:"Case id."`
	By string `help:"Who picked it up, e.g. the agent session name."`
}

func (c *PickupCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	cs, err := store.Pickup(dir, store.PickupRecord{By: c.By})
	if err != nil {
		return err
	}
	return d.done(cs)
}
