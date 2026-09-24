package main

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"slices"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type PruneCmd struct {
	Age    time.Duration `help:"Only cases whose last event is older than this Go duration (720h). 0 means any age." default:"720h" placeholder:"DURATION"`
	State  []string      `help:"Only cases in these states: closed, withdrawn or both. Repeat or comma-separate." default:"closed,withdrawn" placeholder:"STATE"`
	Delete bool          `help:"Delete the cases instead of moving them to the archive."`
	Yes    bool          `help:"Prune the cases. Without it, prune only prints what it would prune." short:"y"`
}

// pruner is a store that can take cases out of itself, as *store.DB can.
type pruner interface {
	store.Store
	Archive(ctx context.Context, id string, pre ...store.Precondition) error
	Delete(ctx context.Context, id string, pre ...store.Precondition) error
}

// writePause is how long sweep, prune and wait --pickup wait after each
// write. A write from elsewhere, such as an agent's note, polls for the
// store's lock with growing pauses between tries, and without a gap it can
// miss every moment a long run of writes lets the lock go, and fail.
const writePause = time.Millisecond

// Run moves each matching case into the store's archive tables, or deletes it
// with --delete, each in a transaction of its own. Only closed and withdrawn
// cases can be pruned: no event is accepted in those states. A case that has
// had an event written since prune read it is refused and left, as is one
// that fails to load.
func (c *PruneCmd) Run(d *Deps) error {
	var want []store.State
	for _, s := range c.State {
		st, err := store.ParseState(s)
		if err != nil {
			return err
		}
		if st != store.StateClosed && st != store.StateWithdrawn {
			return fmt.Errorf("--state: prune takes closed or withdrawn cases, not %s", st)
		}
		want = append(want, st)
	}
	if c.Age < 0 {
		return errors.New("--age must not be negative")
	}

	// Archiving and deleting are not on the Store interface.
	db, ok := d.Cases.(pruner)
	if !ok {
		return fmt.Errorf("the store at %s cannot prune cases", d.Store)
	}
	ctx := context.Background()
	cases, bad, err := db.List(ctx)
	if errors.Is(err, fs.ErrNotExist) {
		cases, bad, err = nil, nil, nil
	}
	if err != nil {
		return err
	}
	for _, b := range bad {
		fmt.Fprintf(d.Stderr, "warning: skipped %v\n", b)
	}

	now := time.Now()
	var prune []*store.Case
	for _, cs := range cases {
		if slices.Contains(want, cs.State) && (c.Age == 0 || now.Sub(cs.UpdatedAt) >= c.Age) {
			prune = append(prune, cs)
		}
	}

	verb, done := "archived", "moved to the archive in "+d.Store
	if c.Delete {
		verb, done = "deleted", "deleted"
	}
	if !c.Yes {
		for _, cs := range prune {
			fmt.Fprintf(d.Stdout, "%s %s, would be %s\n", cs.ID, cs.State, verb)
		}
		if len(prune) > 0 {
			fmt.Fprintf(d.Stderr, "%d cases would be %s. Pass --yes to prune them.\n", len(prune), done)
		}
		return nil
	}

	var failed []string
	for _, cs := range prune {
		remove := db.Archive
		if c.Delete {
			remove = db.Delete
		}
		err := remove(ctx, cs.ID, store.AtRevision(cs.Revision()))
		time.Sleep(writePause)
		if err != nil {
			failed = append(failed, cs.ID+": "+err.Error())
			continue
		}
		fmt.Fprintf(d.Stdout, "%s %s\n", cs.ID, verb)
	}
	if len(failed) > 0 {
		return fmt.Errorf("%d cases were not pruned:\n  %s", len(failed), strings.Join(failed, "\n  "))
	}
	return nil
}
