//go:build unix

package instance

import (
	"os"
	"testing"
)

func TestAlive(t *testing.T) {
	if !alive(os.Getpid()) {
		t.Error("own pid not alive")
	}
	if alive(deadPID(t)) {
		t.Error("reaped pid alive")
	}
	// pid 1 belongs to root, so a signal to it fails with EPERM.
	if os.Geteuid() != 0 && !alive(1) {
		t.Error("pid 1 not alive")
	}
}
