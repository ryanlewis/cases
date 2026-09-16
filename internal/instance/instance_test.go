package instance

import (
	"encoding/json"
	"errors"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestPathPerStore(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	a, _ := Path("/data/work/Cases")
	b, _ := Path("/data/home/Cases")
	again, _ := Path("/data/work/../work/Cases")
	if a == b || a != again {
		t.Errorf("paths: %s, %s, %s", a, b, again)
	}
	if dir := filepath.Join(os.Getenv("XDG_STATE_HOME"), "cases"); filepath.Dir(a) != dir || !strings.HasPrefix(filepath.Base(a), "serve-cases-") {
		t.Errorf("path = %s, want serve-cases-<hash>.json in %s", a, dir)
	}

	t.Setenv("XDG_STATE_HOME", "")
	t.Setenv("HOME", "/home/me")
	if p, _ := Path("/s"); !strings.HasPrefix(p, "/home/me/.local/state/cases/") {
		t.Errorf("fallback path = %s", p)
	}
}

func TestRemoveLeavesAnotherServesFile(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	root := t.TempDir()
	if err := Write(Info{PID: 42, Store: root}); err != nil {
		t.Fatal(err)
	}
	path, _ := Path(root)
	if err := Remove(root, 43); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Errorf("file for pid 42 removed by pid 43: %v", err)
	}
	if err := Remove(root, 42); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Errorf("file not removed: %v", err)
	}
}

// listening returns an address that accepts connections until the test ends.
func listening(t *testing.T) string {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ln.Close() })
	return ln.Addr().String()
}

// refusing returns an address that nothing listens on.
func refusing(t *testing.T) string {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close()
	return addr
}

// deadPID returns the pid of a process that has exited and been reaped.
func deadPID(t *testing.T) int {
	t.Helper()
	cmd := exec.Command("true")
	if err := cmd.Run(); err != nil {
		t.Skip("cannot run true:", err)
	}
	return cmd.Process.Pid
}

func TestRunning(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	live := listening(t)
	started := time.Date(2026, 9, 17, 9, 0, 0, 0, time.UTC)

	tests := []struct {
		name    string
		info    Info
		running bool
	}{
		{"live pid, listening address", Info{PID: os.Getpid(), Addr: live}, true},
		{"live pid, address refuses connections", Info{PID: os.Getpid(), Addr: refusing(t)}, false},
		{"dead pid, listening address", Info{PID: deadPID(t), Addr: live}, false},
		{"no pid", Info{PID: 0, Addr: live}, false},
		{"negative pid", Info{PID: -1, Addr: live}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			root := t.TempDir()
			tt.info.Store, tt.info.URL, tt.info.StartedAt, tt.info.Version = root, "http://"+tt.info.Addr+"/", started, "v1"
			if err := Write(tt.info); err != nil {
				t.Fatal(err)
			}
			got, err := Running(root)
			if err != nil {
				t.Fatal(err)
			}
			switch {
			case tt.running && (got == nil || *got != tt.info):
				t.Errorf("Running = %+v, want %+v", got, tt.info)
			case !tt.running && got != nil:
				t.Errorf("Running = %+v, want nil", got)
			}
		})
	}
}

func TestRunningWithoutFile(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	if got, err := Running(t.TempDir()); got != nil || err != nil {
		t.Errorf("Running = %+v, %v; want nil, nil", got, err)
	}
}

func TestRunningOnMalformedFile(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	root := t.TempDir()
	path, _ := Path(root)
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("{\"pid\": "), 0o600); err != nil {
		t.Fatal(err)
	}
	got, err := Running(root)
	var pe *os.PathError
	if got != nil || !errors.As(err, &pe) || pe.Path != path || pe.Op != "read" {
		t.Errorf("Running = %+v, %v; want a read error naming %s", got, err, path)
	}

	// Remove cannot tell whose file it is, so it removes it.
	if err := Remove(root, os.Getpid()); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("malformed file not removed: %v", err)
	}
}

func TestRunningOnUnreadableFile(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	root := t.TempDir()
	path, _ := Path(root)
	// A directory in the file's place cannot be read as one.
	if err := os.MkdirAll(path, 0o700); err != nil {
		t.Fatal(err)
	}
	if got, err := Running(root); got != nil || err == nil {
		t.Errorf("Running = %+v, %v; want an error", got, err)
	}
	if err := Remove(root, os.Getpid()); err == nil {
		t.Error("Remove on a directory: no error")
	}
}

func TestNoStateDirectory(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", "")
	t.Setenv("HOME", "")
	if _, err := Dir(); err == nil || !strings.Contains(err.Error(), "cannot locate the state directory") {
		t.Errorf("Dir: err = %v", err)
	}
	if got, err := Running(t.TempDir()); got != nil || err == nil {
		t.Errorf("Running = %+v, %v; want the state directory error", got, err)
	}
	if err := Write(Info{PID: 1, Store: t.TempDir()}); err == nil {
		t.Error("Write: no error")
	}
	if err := Remove(t.TempDir(), 1); err == nil {
		t.Error("Remove: no error")
	}
}

func TestWriteReplacesAndLeavesNoTemporaryFiles(t *testing.T) {
	t.Setenv("XDG_STATE_HOME", t.TempDir())
	root := t.TempDir()
	for _, pid := range []int{41, 42} {
		if err := Write(Info{PID: pid, Store: root}); err != nil {
			t.Fatal(err)
		}
	}
	path, _ := Path(root)
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var info Info
	if err := json.Unmarshal(data, &info); err != nil || info.PID != 42 || !strings.HasSuffix(string(data), "}\n") {
		t.Errorf("file = %q, %v; want pid 42 and a trailing newline", data, err)
	}
	entries, err := os.ReadDir(filepath.Dir(path))
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Errorf("state directory holds %d entries, want only %s", len(entries), filepath.Base(path))
	}
}

func TestWriteFailsWhenStateDirectoryIsAFile(t *testing.T) {
	file := filepath.Join(t.TempDir(), "state")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_STATE_HOME", file)
	if err := Write(Info{PID: 1, Store: t.TempDir()}); err == nil {
		t.Error("Write under a file: no error")
	}
}
