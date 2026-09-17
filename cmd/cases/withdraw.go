package main

import (
	"context"

	"github.com/ryanlewis/cases/internal/store"
)

type WithdrawCmd struct {
	ID       string `arg:"" help:"Case id."`
	Reason   string `help:"Why the case no longer needs an answer." placeholder:"TEXT"`
	Revision *int   `help:"Refuse the withdraw if the case's revision (from show --json) is no longer N." placeholder:"N"`
}

func (c *WithdrawCmd) Run(d *Deps) error {
	if err := store.ValidID(c.ID); err != nil {
		return err
	}
	cs, err := d.Cases.Withdraw(context.Background(), c.ID, store.WithdrawRecord{Reason: c.Reason, Actor: d.workerActor(c.ID)}, atRevision(c.Revision)...)
	if err != nil {
		return err
	}
	return d.done(cs)
}
