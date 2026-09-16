package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"slices"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type WaitCmd struct {
	Since   string        `help:"Also count human events written after this RFC 3339 time, even if they were already in the store when wait started. By default only events that land while waiting count." placeholder:"TIME"`
	Timeout time.Duration `help:"Give up after this long: nothing on stdout, one line on stderr, exit 2. 0 waits forever." default:"0"`
	ID      []string      `help:"Only wait on this case. Repeatable." name:"id" sep:"none" placeholder:"ID"`
}

// needsAgent reports whether the case is waiting on the agent, and the event
// that put it there: its last event is a human answer, park or resume. Once
// the agent picks up, notes, closes or withdraws, the case no longer counts.
func needsAgent(c *store.Case) (store.Event, bool) {
	if len(c.Events) == 0 {
		return store.Event{}, false
	}
	last := c.Events[len(c.Events)-1]
	if last.Author != store.AuthorHuman {
		return last, false
	}
	switch last.Type {
	case store.EventAnswer, store.EventPark, store.EventResume:
		return last, true
	}
	return last, false
}

// Run polls the store every second. It returns as soon as a case needs the
// agent because of a human event that is new: its file was not in the store
// on the first poll, or it is later than --since. It then prints every case
// that currently needs the agent, one JSON object per line.
//
// New is judged by the file appearing rather than by its timestamp alone,
// because an answer written on another machine can arrive through sync well
// after the time it records.
func (c *WaitCmd) Run(d *Deps) error {
	var since time.Time
	if c.Since != "" {
		t, err := time.Parse(time.RFC3339, c.Since)
		if err != nil {
			return fmt.Errorf("--since: %w", err)
		}
		since = t
	}
	for _, id := range c.ID {
		if _, err := d.caseDir(id); err != nil {
			return err
		}
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
	// seen holds the event files present on the first poll; nil until then.
	var seen map[string]bool
	for {
		cases, bad, err := poller.Poll()
		if errors.Is(err, fs.ErrNotExist) {
			// The store may not exist yet; keep waiting for it.
			cases, bad, err = nil, nil, nil
		}
		if err != nil {
			return err
		}
		for _, b := range bad {
			if msg := b.Error(); !warned[msg] {
				warned[msg] = true
				fmt.Fprintf(d.Stderr, "warning: %s\n", msg)
			}
		}
		for _, cs := range cases {
			d.warn(cs, warned)
		}

		type ready struct {
			c  *store.Case
			ev store.Event
		}
		var waiting []ready
		fresh := false
		for _, cs := range cases {
			if len(c.ID) > 0 && !slices.Contains(c.ID, cs.ID) {
				continue
			}
			ev, ok := needsAgent(cs)
			if !ok {
				continue
			}
			waiting = append(waiting, ready{cs, ev})
			if (seen != nil && !seen[cs.ID+"/"+ev.File]) || (!since.IsZero() && ev.At.After(since)) {
				fresh = true
			}
		}
		if seen == nil {
			seen = map[string]bool{}
			for _, cs := range cases {
				for _, ev := range cs.Events {
					seen[cs.ID+"/"+ev.File] = true
				}
			}
		}

		if fresh {
			slices.SortStableFunc(waiting, func(a, b ready) int {
				if n := a.ev.At.Compare(b.ev.At); n != 0 {
					return n
				}
				return strings.Compare(a.c.ID, b.c.ID)
			})
			enc := json.NewEncoder(d.Stdout)
			enc.SetEscapeHTML(false)
			for _, r := range waiting {
				if err := enc.Encode(r.c); err != nil {
					return err
				}
			}
			return nil
		}

		sleep := poll
		if !deadline.IsZero() {
			left := time.Until(deadline)
			if left <= 0 {
				return &exitError{code: exitTimeout, msg: fmt.Sprintf("no case needed the agent within %s", c.Timeout)}
			}
			sleep = min(sleep, left)
		}
		time.Sleep(sleep)
	}
}
