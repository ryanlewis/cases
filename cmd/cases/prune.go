package main

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// archiveDir is the directory inside the store that prune moves cases into.
// Its name starts with a dot, so list, wait and serve do not see it.
const archiveDir = ".archive"

type PruneCmd struct {
	Age    time.Duration `help:"Only cases whose last event is older than this Go duration (720h). 0 means any age." default:"720h" placeholder:"DURATION"`
	State  []string      `help:"Only cases in these states: closed, withdrawn or both. Repeat or comma-separate." default:"closed,withdrawn" placeholder:"STATE"`
	Delete bool          `help:"Delete the case directories instead of moving them to the archive."`
	Yes    bool          `help:"Prune the cases. Without it, prune only prints what it would prune." short:"y"`
}

// Run moves each matching case directory to <store>/.archive/<id>, or removes
// it with --delete. Only closed and withdrawn cases can be pruned: no event is
// accepted in those states, so nothing can be writing to the directory. A
// case that fails to load is reported and left.
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

	cases, bad, err := store.List(d.Store)
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

	verb, done := "archived", "moved to "+filepath.Join(d.Store, archiveDir)
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

	archive := filepath.Join(d.Store, archiveDir)
	if !c.Delete && len(prune) > 0 {
		if err := os.MkdirAll(archive, 0o755); err != nil {
			return err
		}
	}
	var failed []string
	for _, cs := range prune {
		var err error
		if c.Delete {
			err = os.RemoveAll(cs.Dir)
		} else {
			err = moveNew(cs.Dir, filepath.Join(archive, cs.ID))
		}
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

// moveNew renames from to to, refusing when something is already at to.
// os.Rename would replace an empty directory there.
func moveNew(from, to string) error {
	if _, err := os.Lstat(to); err == nil {
		return fmt.Errorf("%s already exists", to)
	} else if !errors.Is(err, fs.ErrNotExist) {
		return err
	}
	return os.Rename(from, to)
}
