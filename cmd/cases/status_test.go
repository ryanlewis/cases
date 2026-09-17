package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/instance"
)

// startServe runs serve on storePath with port 0 and returns its URL and a
// function that stops it and returns what Run returned.
func startServe(t *testing.T, storePath string) (url string, stop func() error) {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	stdout := &syncBuffer{}
	done := make(chan error, 1)
	go func() {
		cmd := &ServeCmd{Listen: "127.0.0.1:0"}
		done <- cmd.Run(&Deps{Store: storePath, Cases: openStore(t, storePath), Stdout: stdout, Stderr: io.Discard, Context: ctx})
	}()
	stop = func() error {
		cancel()
		select {
		case err := <-done:
			return err
		case <-time.After(5 * time.Second):
			t.Fatal("serve did not stop")
			return nil
		}
	}
	urlPattern := regexp.MustCompile(`http://127\.0\.0\.1:\d+/`)
	for deadline := time.Now().Add(5 * time.Second); url == "" && time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
		url = urlPattern.FindString(stdout.String())
	}
	if url == "" {
		_ = stop()
		t.Fatalf("serve printed no URL: %q", stdout.String())
	}
	return url, stop
}

func TestServeRecordsInstanceForStatus(t *testing.T) {
	storePath := newStore(t)
	url, stop := startServe(t, storePath)

	path, err := instance.Path(storePath)
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("no state file: %v", err)
	}
	var info instance.Info
	if err := json.Unmarshal(data, &info); err != nil {
		t.Fatal(err)
	}
	if info.PID != os.Getpid() || info.URL != url || "http://"+info.Addr+"/" != url || info.Store != storePath ||
		info.Version != version || time.Since(info.StartedAt) > time.Minute || info.StartedAt.Location() != time.UTC {
		t.Errorf("state file = %s", data)
	}

	out := mustRun(t, "--store", storePath, "status")
	if !strings.Contains(out, url) || !strings.Contains(out, "pid "+strconv.Itoa(os.Getpid())) {
		t.Errorf("status = %q, want %s and our pid", out, url)
	}
	var got instance.Info
	if err := json.Unmarshal([]byte(mustRun(t, "--store", storePath, "status", "--json")), &got); err != nil || got.URL != url {
		t.Errorf("status --json: %+v %v", got, err)
	}

	if err := stop(); err != nil {
		t.Errorf("serve returned %v", err)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("state file left after stop: %v", err)
	}
	assertNotRunning(t, storePath)
}

func TestStatusWithoutServe(t *testing.T) {
	assertNotRunning(t, newStore(t))
}

func TestStaleInstanceIsNotRunning(t *testing.T) {
	gone := exec.Command("true")
	if err := gone.Run(); err != nil {
		t.Skip("cannot run true:", err)
	}
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	closedAddr := ln.Addr().String()
	ln.Close()

	for name, info := range map[string]instance.Info{
		"dead pid":             {PID: gone.Process.Pid, Addr: closedAddr},
		"reused pid, no serve": {PID: os.Getpid(), Addr: closedAddr},
	} {
		t.Run(name, func(t *testing.T) {
			storePath := newStore(t)
			info.Store, info.URL = storePath, "http://"+closedAddr+"/"
			if err := instance.Write(info); err != nil {
				t.Fatal(err)
			}
			assertNotRunning(t, storePath)

			// A fresh serve replaces the stale file.
			url, stop := startServe(t, storePath)
			if out := mustRun(t, "--store", storePath, "status"); !strings.Contains(out, url) {
				t.Errorf("status = %q, want %s", out, url)
			}
			if err := stop(); err != nil {
				t.Errorf("serve returned %v", err)
			}
		})
	}
}

func TestSecondServeOnSameStoreIsRefused(t *testing.T) {
	storePath := newStore(t)
	url, stop := startServe(t, storePath)
	defer stop()

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	cmd := &ServeCmd{Listen: "127.0.0.1:0"}
	err := cmd.Run(&Deps{Store: storePath, Cases: openStore(t, storePath), Stdout: io.Discard, Stderr: io.Discard, Context: ctx})
	if err == nil || !strings.Contains(err.Error(), "already running") || !strings.Contains(err.Error(), url) ||
		!strings.Contains(err.Error(), "pid "+strconv.Itoa(os.Getpid())) {
		t.Errorf("second serve: err = %v", err)
	}
	if out := mustRun(t, "--store", storePath, "status"); !strings.Contains(out, url) {
		t.Errorf("status after refused serve = %q, want %s", out, url)
	}

	// Another store is independent.
	other := newStore(t)
	otherURL, stopOther := startServe(t, other)
	defer stopOther()
	if out := mustRun(t, "--store", other, "status"); !strings.Contains(out, otherURL) {
		t.Errorf("status on the other store = %q, want %s", out, otherURL)
	}
}

func assertNotRunning(t *testing.T, storePath string) {
	t.Helper()
	r := runCases(t, "", "--store", storePath, "status")
	var ee *exitError
	if !errors.As(r.err, &ee) || ee.code != 1 || ee.msg != "not running" || r.stdout != "" {
		t.Errorf("status: err = %v, stdout = %q; want exit 1, not running", r.err, r.stdout)
	}
}
