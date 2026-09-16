//go:build unix

package instance

import (
	"errors"
	"syscall"
)

// alive reports whether a process with this pid exists. EPERM means it
// exists but belongs to another user.
func alive(pid int) bool {
	err := syscall.Kill(pid, 0)
	return err == nil || errors.Is(err, syscall.EPERM)
}
