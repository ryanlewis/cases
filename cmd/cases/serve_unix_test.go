//go:build unix

package main

import (
	"io"
	"os"
	"syscall"
	"testing"
	"time"
)

// TestServeStopsOnSignal runs serve with no context, as main does, so it
// stops on SIGTERM. The signal is sent only once the Serving line shows the
// handler is installed; before that it would kill the test binary.
func TestServeStopsOnSignal(t *testing.T) {
	storePath := newStore(t)
	stdout := &syncBuffer{}
	done := make(chan error, 1)
	go func() {
		cmd := &ServeCmd{Listen: "127.0.0.1:0"}
		done <- cmd.Run(&Deps{Store: storePath, Cases: openStore(t, storePath), Stdout: stdout, Stderr: io.Discard})
	}()
	waitForURL(t, stdout)
	if err := syscall.Kill(os.Getpid(), syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("serve returned %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("serve did not stop on SIGTERM")
	}
	assertNotRunning(t, storePath)
}
