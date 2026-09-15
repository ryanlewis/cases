package main

import (
	"bytes"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/alecthomas/kong"
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
	parser, err := newParser(&cli,
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
	deps := &Deps{
		Store:  cli.Store,
		Stdin:  strings.NewReader(stdin),
		Stdout: &stdout,
		Stderr: &stderr,
		Poll:   10 * time.Millisecond,
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

func TestStoreIsRequired(t *testing.T) {
	t.Setenv("CASES_STORE", "")
	for _, args := range [][]string{{"list"}, {"--store", " ", "list"}} {
		r := runCases(t, "", args...)
		if r.err == nil || !strings.Contains(r.err.Error(), "CASES_STORE") {
			t.Errorf("%v: err = %v, want the store demanded", args, r.err)
		}
	}
	os.Unsetenv("CASES_STORE")
	if r := runCases(t, "", "list"); r.err == nil || !strings.Contains(r.err.Error(), "--store") {
		t.Errorf("unset: err = %v, want missing --store", r.err)
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
	parser, err := newParser(&cli, kong.Writers(&stdout, &stdout), kong.Exit(func(code int) { exited = code }))
	if err != nil {
		t.Fatal(err)
	}
	_, _ = parser.Parse([]string{"--version"})
	if exited != 0 || !strings.HasPrefix(stdout.String(), "cases dev") {
		t.Errorf("exit %d, output %q", exited, stdout.String())
	}
}
