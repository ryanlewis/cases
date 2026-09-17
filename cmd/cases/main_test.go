package main

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/alecthomas/kong"

	"github.com/ryanlewis/cases/internal/store"
)

// result is what one CLI invocation produced.
type result struct {
	stdout, stderr string
	err            error
}

// runCases parses args exactly as main does and runs the command against an
// in-memory stdin and captured output. It never exits the test process.
func runCases(t *testing.T, stdin string, args ...string) result {
	t.Helper()
	var cli CLI
	var stdout, stderr bytes.Buffer
	cfg, cfgErr := loadConfig(args)
	parser, err := newParser(&cli, cfg,
		kong.Writers(&stdout, &stderr),
		kong.Exit(func(code int) { panic(fmt.Sprintf("kong exit %d", code)) }),
	)
	if err != nil {
		t.Fatal(err)
	}
	ctx, err := parser.Parse(args)
	if err != nil {
		return result{stdout.String(), stderr.String(), err}
	}
	if cfgErr != nil && !diagnosesConfig(ctx) {
		return result{stdout.String(), stderr.String(), cfgErr}
	}
	deps := &Deps{
		Store:  cli.Store,
		Stdin:  strings.NewReader(stdin),
		Stdout: &stdout,
		Stderr: &stderr,
		Poll:   10 * time.Millisecond,
		Config: cfg,
	}
	err = ctx.Run(deps)
	return result{stdout.String(), stderr.String(), err}
}

// mustRun runs a command that is expected to succeed and returns its stdout.
func mustRun(t *testing.T, args ...string) string {
	t.Helper()
	r := runCases(t, "", args...)
	if r.err != nil {
		t.Fatalf("cases %s: %v\nstderr: %s", strings.Join(args, " "), r.err, r.stderr)
	}
	return r.stdout
}

// openDecision opens a two-option decision case and returns its id.
func openDecision(t *testing.T, store string) string {
	t.Helper()
	out := mustRun(t, "--store", store, "open", "--kind", "decision", "--urgency", "today",
		"--title", "Pin bun?", "--option", "Pin to 1.2.3", "--option", "Float, with renovate")
	return strings.TrimSpace(out)
}

// TestMain keeps the tests away from the developer's own config file and
// store: HOME and the XDG directories point into a scratch directory.
func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "cases-cli-test")
	if err != nil {
		panic(err)
	}
	os.Setenv("HOME", dir)
	os.Setenv("XDG_CONFIG_HOME", filepath.Join(dir, "config"))
	os.Setenv("XDG_DATA_HOME", filepath.Join(dir, "data"))
	os.Setenv("XDG_STATE_HOME", filepath.Join(dir, "state"))
	os.Unsetenv("CASES_STORE")
	os.Unsetenv("CASES_CONFIG")
	os.Unsetenv("CASES_WORKER")
	os.Unsetenv("CASES_LABEL")
	code := m.Run()
	os.RemoveAll(dir)
	os.Exit(code)
}

func TestStoreDefaultsToDataHome(t *testing.T) {
	data := t.TempDir()
	t.Setenv("XDG_DATA_HOME", data)
	want := filepath.Join(data, "cases")
	for _, env := range []string{"", " "} {
		// CASES_STORE set but blank means unset.
		t.Setenv("CASES_STORE", env)
		id := strings.TrimSpace(mustRun(t, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "Default store"))
		if _, err := os.Stat(filepath.Join(want, id)); err != nil {
			t.Errorf("CASES_STORE=%q: case not in %s: %v", env, want, err)
		}
	}
	if out := mustRun(t, "list"); !strings.Contains(out, "Default store") {
		t.Errorf("list = %q, want the default store's cases", out)
	}

	t.Setenv("XDG_DATA_HOME", "")
	t.Setenv("HOME", t.TempDir())
	out := mustRun(t, "config", "show")
	if want := filepath.Join(os.Getenv("HOME"), ".local", "share", "cases"); !strings.Contains(out, want) {
		t.Errorf("config show = %q, want the store under ~/.local/share", out)
	}
}

func TestListOnMissingStoreIsEmpty(t *testing.T) {
	root := filepath.Join(t.TempDir(), "nothing-here")
	if out := mustRun(t, "--store", root, "list"); !strings.Contains(out, "No cases.") {
		t.Errorf("list = %q, want no cases", out)
	}
	if out := mustRun(t, "--store", root, "list", "--json"); strings.TrimSpace(out) != "[]" {
		t.Errorf("list --json = %q, want []", out)
	}
}

func TestStoreFromEnvironment(t *testing.T) {
	store := t.TempDir()
	t.Setenv("CASES_STORE", store)
	id := strings.TrimSpace(mustRun(t, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "From env"))
	if out := mustRun(t, "list"); !strings.Contains(out, id) {
		t.Errorf("list = %q, want %s", out, id)
	}
}

func TestVersionString(t *testing.T) {
	var cli CLI
	var stdout bytes.Buffer
	exited := -1
	parser, err := newParser(&cli, nil, kong.Writers(&stdout, &stdout), kong.Exit(func(code int) { exited = code }))
	if err != nil {
		t.Fatal(err)
	}
	_, _ = parser.Parse([]string{"--version"})
	if exited != 0 || !strings.HasPrefix(stdout.String(), "cases dev") {
		t.Errorf("exit %d, output %q", exited, stdout.String())
	}
}

func TestReportExitStatus(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	status := func(r result) (int, string) {
		t.Helper()
		if r.err == nil {
			t.Fatal("command succeeded, want an error")
		}
		var buf bytes.Buffer
		code := report(&buf, r.err)
		return code, buf.String()
	}

	code, stderr := status(runCases(t, "", "--store", root, "pickup", id))
	if code != exitTransition || stderr != "Error: cannot pickup a case that is open\n" {
		t.Errorf("pickup of an open case: exit %d, stderr %q; want exit 3", code, stderr)
	}

	stale := runCases(t, "", "--store", root, "answer", id, "--option", "1", "--revision", "5")
	if !errors.Is(stale.err, store.ErrStale) {
		t.Fatalf("answer at an old revision: err = %v, want ErrStale", stale.err)
	}
	code, stderr = status(stale)
	if code != 1 || !strings.HasPrefix(stderr, "Error: ") {
		t.Errorf("stale answer: exit %d, stderr %q; want exit 1", code, stderr)
	}

	code, stderr = status(runCases(t, "", "--store", root, "show", "nope"))
	if code != 1 || !strings.HasPrefix(stderr, "Error: ") {
		t.Errorf("missing case: exit %d, stderr %q; want exit 1", code, stderr)
	}

	code, stderr = status(runCases(t, "", "--store", root, "wait", "--id", id, "--timeout", "10ms"))
	if code != exitTimeout || strings.HasPrefix(stderr, "Error: ") {
		t.Errorf("wait timeout: exit %d, stderr %q; want exit 2 with no Error: line", code, stderr)
	}
}
