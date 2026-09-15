package store

import (
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"
)

// LoadError is a case directory that could not be loaded.
type LoadError struct {
	Dir string
	Err error
}

func (e *LoadError) Error() string { return e.Dir + ": " + e.Err.Error() }

func (e *LoadError) Unwrap() error { return e.Err }

// List loads every case in root, sorted by id (which is by open time). A case
// that fails to load is returned in bad and does not stop the others. err is
// set only when root itself cannot be read.
func List(root string) (cases []*Case, bad []*LoadError, err error) {
	dirs, err := caseDirs(root)
	if err != nil {
		return nil, nil, err
	}
	for _, dir := range dirs {
		c, err := Load(dir)
		if errors.Is(err, ErrNoEvents) {
			continue
		}
		if err != nil {
			bad = append(bad, &LoadError{Dir: dir, Err: err})
			continue
		}
		cases = append(cases, c)
	}
	return cases, bad, nil
}

// caseDirs lists the case directories in root, sorted by name.
func caseDirs(root string) ([]string, error) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return nil, err
	}
	var dirs []string
	for _, e := range entries {
		if e.IsDir() && !strings.HasPrefix(e.Name(), ".") {
			dirs = append(dirs, filepath.Join(root, e.Name()))
		}
	}
	return dirs, nil
}

// settle is how long after a directory's mtime the poller keeps reloading it
// regardless. A filesystem with coarse mtimes can record two writes a moment
// apart under one mtime, and the second would otherwise go unseen.
const settle = 2 * time.Second

// Poller lists a store repeatedly, reloading only the case directories whose
// mtime has changed since the last poll. Adding an event file renames it into
// the case directory, which changes the directory's mtime.
type Poller struct {
	root    string
	rootMod time.Time
	dirs    []string
	cache   map[string]polled
}

type polled struct {
	mod time.Time
	c   *Case
	err error
}

// NewPoller returns a Poller for root.
func NewPoller(root string) *Poller {
	return &Poller{root: root, cache: map[string]polled{}}
}

// Poll returns the same as List, from cache where nothing has changed.
func (p *Poller) Poll() (cases []*Case, bad []*LoadError, err error) {
	info, err := os.Stat(p.root)
	if err != nil {
		return nil, nil, err
	}
	if p.dirs == nil || changed(p.rootMod, info.ModTime()) {
		dirs, err := caseDirs(p.root)
		if err != nil {
			return nil, nil, err
		}
		p.dirs, p.rootMod = dirs, info.ModTime()
		for dir := range p.cache {
			if !slices.Contains(dirs, dir) {
				delete(p.cache, dir)
			}
		}
	}
	for _, dir := range p.dirs {
		info, err := os.Stat(dir)
		if err != nil {
			// Removed since the directory was listed.
			delete(p.cache, dir)
			continue
		}
		entry, ok := p.cache[dir]
		if !ok || changed(entry.mod, info.ModTime()) {
			c, err := Load(dir)
			entry = polled{mod: info.ModTime(), c: c, err: err}
			p.cache[dir] = entry
		}
		if errors.Is(entry.err, ErrNoEvents) {
			continue
		}
		if entry.err != nil {
			bad = append(bad, &LoadError{Dir: dir, Err: entry.err})
			continue
		}
		cases = append(cases, entry.c)
	}
	return cases, bad, nil
}

func changed(cached, current time.Time) bool {
	return !cached.Equal(current) || time.Since(current) < settle
}
