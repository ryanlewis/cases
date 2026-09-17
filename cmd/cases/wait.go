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
	For     string        `help:"Whose turn to wait for: agent (a human answered, parked or resumed) or human (the agent opened, noted or resumed a case)." enum:"agent,human" default:"agent" placeholder:"SIDE"`
	Since   string        `help:"Also count events written after this time, even if they were already in the store when wait started: an RFC 3339 time, or a case id for the time that case was opened. By default only events that land while waiting count." placeholder:"TIME|ID"`
	Timeout time.Duration `help:"Give up after this long: nothing on stdout, one line on stderr, exit 2. 0 waits forever." default:"0"`
	ID      []string      `help:"Only wait on this case. Repeatable." name:"id" sep:"none" placeholder:"ID"`
	CaseFilter
}

// needsAgent reports whether the case is waiting on the agent, and the event
// that put it there: its last event is a human answer, park or resume. Once
// the agent picks up, notes, amends, closes or withdraws, the case no longer
// counts.
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

// needsHuman reports whether the case is waiting on the human, and the event
// that put it there: the case is open and its last event is the agent's. An
// amend changes a case that was already waiting, so the event reported is the
// agent event before any trailing amends; only an amend that follows a human
// event is reported itself.
func needsHuman(c *store.Case) (store.Event, bool) {
	n := len(c.Events)
	if c.State != store.StateOpen || n == 0 || c.Events[n-1].Author != store.AuthorAgent {
		return store.Event{}, false
	}
	i := n - 1
	for i > 0 && c.Events[i].Type == store.EventAmend && c.Events[i-1].Author == store.AuthorAgent {
		i--
	}
	return c.Events[i], true
}

// sinceTime resolves --since: an RFC 3339 time, or a case id standing for
// that case's opened_at. A value that is neither, or the id of a case that is
// not in the store, is an error.
func (c *WaitCmd) sinceTime(d *Deps) (time.Time, error) {
	if c.Since == "" {
		return time.Time{}, nil
	}
	if t, err := time.Parse(time.RFC3339, c.Since); err == nil {
		return t, nil
	}
	dir, err := d.caseDir(c.Since)
	if err != nil {
		return time.Time{}, fmt.Errorf("--since %q is neither an RFC 3339 time nor a case id", c.Since)
	}
	cs, err := store.Load(dir)
	if errors.Is(err, fs.ErrNotExist) {
		return time.Time{}, fmt.Errorf("--since %q is neither an RFC 3339 time nor a case in the store", c.Since)
	}
	if err != nil {
		return time.Time{}, fmt.Errorf("--since: %w", err)
	}
	return cs.OpenedAt, nil
}

// Run polls the store every second. It returns as soon as a case needs the
// agent because of a human event that is new: its file was not in the store
// on the first poll, or it is later than --since. It then prints every case
// that currently needs the agent, one JSON object per line. With --for human
// it does the same for cases waiting on the human.
//
// New is judged by the file appearing rather than by its timestamp alone,
// because an answer written on another machine can arrive through sync well
// after the time it records.
func (c *WaitCmd) Run(d *Deps) error {
	since, err := c.sinceTime(d)
	if err != nil {
		return err
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

	needs, side := needsAgent, "agent"
	if c.For == "human" {
		needs, side = needsHuman, "human"
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
			if len(c.ID) > 0 && !slices.Contains(c.ID, cs.ID) || !c.match(cs) {
				continue
			}
			ev, ok := needs(cs)
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
				return &exitError{code: exitTimeout, msg: fmt.Sprintf("no case needed the %s within %s", side, c.Timeout)}
			}
			sleep = min(sleep, left)
		}
		time.Sleep(sleep)
	}
}
