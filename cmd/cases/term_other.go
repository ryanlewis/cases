//go:build !darwin && !linux

package main

import (
	"errors"
	"os"
)

// keysRaw is not supported here; serve reads keys a line at a time instead.
func keysRaw(*os.File) (func(), error) {
	return nil, errors.New("raw key input is not supported on this platform")
}

// hasTermios cannot tell here; a character device is taken as a terminal.
func hasTermios(*os.File) bool { return true }

// isForeground cannot tell here; the process is taken to be in the foreground.
func isForeground(*os.File) bool { return true }

// termRows cannot tell here; 0 means the height is unknown.
func termRows(*os.File) int { return 0 }
