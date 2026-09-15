package main

import "github.com/ryanlewis/cases/internal/store"

type WithdrawCmd struct {
	ID string `arg:"" help:"Case id."`
}

func (c *WithdrawCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	cs, err := store.Withdraw(dir, store.WithdrawRecord{})
	if err != nil {
		return err
	}
	return d.done(cs)
}
