package main

import (
	"fmt"

	"github.com/ryanlewis/cases/internal/instance"
)

type InboxCmd struct {
	ID    string `arg:"" optional:"" help:"Case id, or any part of it that names one case."`
	Print bool   `help:"Print the URL instead of opening it."`
}

func (c *InboxCmd) Run(d *Deps) error {
	info, err := instance.Running(d.Store)
	if err != nil {
		return err
	}
	if info == nil {
		return &exitError{code: 1, msg: "not running; start one with `cases serve`, or `cases service install` for an always-on inbox"}
	}
	target := info.URL
	if c.ID != "" {
		id, err := d.findCase(c.ID)
		if err != nil {
			return err
		}
		target = caseURL(info.URL, id)
	}
	if c.Print {
		fmt.Fprintln(d.Stdout, target)
		return nil
	}
	if d.OpenURL == nil {
		return fmt.Errorf("no browser opener is set")
	}
	return d.OpenURL(target)
}
