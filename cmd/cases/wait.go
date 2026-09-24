package main

import (
	"context"
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
	Pickup  bool          `help:"Pick up each answered case before printing it, as pickup does. A case that changed after wait read it is not picked up and is printed as read, with a warning. Needs --id, --label or --worker, and refuses a blank --worker."`
	By      string        `help:"With --pickup: who picked the cases up, e.g. the agent session name." placeholder:"NAME"`
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
	if err := store.ValidID(c.Since); err != nil {
		return time.Time{}, fmt.Errorf("--since %q is neither an RFC 3339 time nor a case id", c.Since)
	}
	cs, err := d.Cases.Get(context.Background(), c.Since)
	if errors.Is(err, fs.ErrNotExist) {
		return time.Time{}, fmt.Errorf("--since %q is neither an RFC 3339 time nor a case in the store", c.Since)
	}
	if err != nil {
		return time.Time{}, fmt.Errorf("--since: %w", err)
	}
	return cs.OpenedAt, nil
}

// waitLine is a case as wait prints it: the case's own JSON and its revision,
// whether the event that put it there is new and so could have woken this
// wait, and the --since to pass to the next wait. That is the latest time
// among the events that put the printed cases there, or the --since given if
// it is later, so the next wait does not wake again on what this one printed.
type waitLine struct {
	*store.Case
	Revision  int       `json:"revision"`
	Fresh     bool      `json:"fresh"`
	NextSince time.Time `json:"next_since,omitzero"`
}

// checkPickup refuses --by without --pickup, and --pickup where it has
// nothing to pick up or could pick up other agents' cases. A blank --worker,
// such as "$CASES_WORKER" with the variable unset, matches every case opened
// without a worker, so it does not count as naming your own cases.
func (c *WaitCmd) checkPickup() error {
	switch {
	case c.By != "" && !c.Pickup:
		return errors.New("--by needs --pickup")
	case !c.Pickup:
		return nil
	case c.For == "human":
		return errors.New("--pickup is only for --for agent: a case waiting on the human has no answer to pick up")
	case slices.ContainsFunc(c.Worker, func(w string) bool { return strings.TrimSpace(w) == "" }):
		return errors.New("--pickup refuses a blank --worker: it would pick up every case opened without a worker, other agents' included")
	case len(c.ID) == 0 && len(c.Label) == 0 && len(c.Worker) == 0:
		return errors.New("--pickup needs --id, --label or --worker, so that it picks up only your own cases")
	}
	return nil
}

// pickUp records the pickup of an answered case at the revision wait read it,
// and returns the case after the pickup. The revision refuses the pickup once
// another event has landed, such as a note or another pickup, so wait never
// picks up an answer other than the one it read. A case not picked up, for
// that or any other reason, is returned as read, still answered, and the
// reason goes to stderr, with the state a changed case is in now.
func (c *WaitCmd) pickUp(d *Deps, cs *store.Case) *store.Case {
	rec := store.PickupRecord{By: c.By, Actor: agentActor(cs.Worker)}
	picked, err := d.Cases.Pickup(context.Background(), cs.ID, rec, store.AtRevision(cs.Revision()))
	time.Sleep(writePause)
	if err != nil {
		fmt.Fprintf(d.Stderr, "warning: %s: not picked up: %v%s\n", cs.ID, err, d.nowState(cs.ID, err))
		return cs
	}
	return picked
}

// Run polls the store every second. It returns as soon as a case needs the
// agent because of a human event that is new: it was not in the store on the
// first poll, or it is later than --since. It then prints every case that
// currently needs the agent, one JSON object per line. With --for human it
// does the same for cases waiting on the human. With --pickup it picks up each
// answered case before printing it.
//
// New is judged by the event appearing rather than by its timestamp alone,
// because an event can record a time well before it was stored, such as an
// answer written with an answered_at of its own.
func (c *WaitCmd) Run(d *Deps) error {
	if err := c.checkPickup(); err != nil {
		return err
	}
	since, err := c.sinceTime(d)
	if err != nil {
		return err
	}
	for _, id := range c.ID {
		if err := store.ValidID(id); err != nil {
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

	poller := d.Cases.NewPoller()
	warned := map[string]bool{}
	// seen holds the events, by case id and file name, present on the first
	// poll; nil until then.
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
			c     *store.Case
			ev    store.Event
			fresh bool
		}
		var waiting []ready
		woke := false
		for _, cs := range cases {
			if len(c.ID) > 0 && !slices.Contains(c.ID, cs.ID) || !c.match(cs) {
				continue
			}
			ev, ok := needs(cs)
			if !ok {
				continue
			}
			fresh := (seen != nil && !seen[cs.ID+"/"+ev.File]) || (!since.IsZero() && ev.At.After(since))
			waiting = append(waiting, ready{cs, ev, fresh})
			woke = woke || fresh
		}
		if seen == nil {
			seen = map[string]bool{}
			for _, cs := range cases {
				for _, ev := range cs.Events {
					seen[cs.ID+"/"+ev.File] = true
				}
			}
		}

		if woke {
			slices.SortStableFunc(waiting, func(a, b ready) int {
				if n := a.ev.At.Compare(b.ev.At); n != 0 {
					return n
				}
				return strings.Compare(a.c.ID, b.c.ID)
			})
			next := since
			for _, r := range waiting {
				if r.ev.At.After(next) {
					next = r.ev.At
				}
			}
			enc := json.NewEncoder(d.Stdout)
			enc.SetEscapeHTML(false)
			for _, r := range waiting {
				cs := r.c
				if c.Pickup && cs.State == store.StateAnswered {
					cs = c.pickUp(d, cs)
				}
				if err := enc.Encode(waitLine{cs, cs.Revision(), r.fresh, next}); err != nil {
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
