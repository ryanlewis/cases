//go:build unix

package store

import (
	"os"
	"syscall"
)

// lockDir takes an exclusive flock on the case directory itself, so no lock
// file is added to the case. It only serialises writers on this machine; the
// fold copes with what a sync from elsewhere can produce.
func lockDir(dir string) (unlock func(), err error) {
	f, err := os.Open(dir)
	if err != nil {
		return nil, err
	}
	fd := int(f.Fd())
	if err := syscall.Flock(fd, syscall.LOCK_EX); err != nil {
		_ = f.Close()
		return nil, err
	}
	return func() {
		_ = syscall.Flock(fd, syscall.LOCK_UN)
		_ = f.Close()
	}, nil
}
