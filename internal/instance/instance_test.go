package instance

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
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
