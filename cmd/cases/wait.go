package main

import (
	"encoding/json"
	"fmt"
	"slices"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type WaitCmd struct {
	Since   string        `help:"Only report answers written after this RFC 3339 time. Without it, every case currently answered is reported." placeholder:"TIME"`
	Timeout time.Duration `help:"Give up after this long and exit 124. 0 waits forever." default:"0"`
}

func (c *WaitCmd) Run(d *Deps) error {
	var since time.Time
	if c.Since != "" {
		t, err := time.Parse(time.RFC3339, c.Since)
		if err != nil {
			return fmt.Errorf("--since: %w", err)
		}
		since = t
	}
	var deadline time.Time
	if c.Timeout > 0 {
		deadline = time.Now().Add(c.Timeout)
	}
	poll := d.Poll
	if poll <= 0 {
		poll = time.Second
	}

	poller := store.NewPoller(d.Store)
	warned := map[string]bool{}
	for {
		cases, bad, err := poller.Poll()
		if err != nil {
			return err
		}
		for _, b := range bad {
			if msg := b.Error(); !warned[msg] {
				warned[msg] = true
				fmt.Fprintf(d.Stderr, "warning: %s\n", msg)
			}
		}

		var answered []*store.Case
		for _, cs := range cases {
			if cs.State == store.StateAnswered && (since.IsZero() || cs.Answer.AnsweredAt.After(since)) {
				answered = append(answered, cs)
			}
		}
		if len(answered) > 0 {
			slices.SortStableFunc(answered, func(a, b *store.Case) int {
				return a.Answer.AnsweredAt.Compare(b.Answer.AnsweredAt)
			})
			enc := json.NewEncoder(d.Stdout)
			enc.SetEscapeHTML(false)
			for _, cs := range answered {
				if err := enc.Encode(cs); err != nil {
					return err
				}
			}
			return nil
		}

		sleep := poll
		if !deadline.IsZero() {
			left := time.Until(deadline)
			if left <= 0 {
				return &exitError{code: exitTimeout, msg: fmt.Sprintf("no answered case within %s", c.Timeout)}
			}
			sleep = min(sleep, left)
		}
		time.Sleep(sleep)
	}
}
