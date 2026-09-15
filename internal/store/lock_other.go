//go:build !unix

package store

import "os"

// lockDir only checks the directory exists on platforms without flock.
func lockDir(dir string) (unlock func(), err error) {
	if _, err := os.Stat(dir); err != nil {
		return nil, err
	}
	return func() {}, nil
}
