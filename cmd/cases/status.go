package main

import (
	"encoding/json"
	"fmt"
	"time"

	"github.com/ryanlewis/cases/internal/instance"
)

type StatusCmd struct {
	JSON bool `help:"Print JSON." short:"j"`
}

func (c *StatusCmd) Run(d *Deps) error {
	info, err := instance.Running(d.Store)
	if err != nil {
		return err
	}
	if info == nil {
		return &exitError{code: 1, msg: "not running"}
	}
	if c.JSON {
		out, err := json.MarshalIndent(info, "", "  ")
		if err != nil {
			return err
		}
		fmt.Fprintln(d.Stdout, string(out))
		return nil
	}
	fmt.Fprintf(d.Stdout, "Serving %s at %s (pid %d, since %s)\n", info.Store, info.URL, info.PID, info.StartedAt.Format(time.RFC3339))
	return nil
}
