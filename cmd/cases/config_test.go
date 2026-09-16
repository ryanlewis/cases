package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/alecthomas/kong"

	"github.com/ryanlewis/cases/internal/config"
)

// writeConfig puts a config file at the default location for this test.
func writeConfig(t *testing.T, body string) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	path := filepath.Join(dir, "cases", "config.toml")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

// parseCases parses without running, for checking what a flag resolved to.
func parseCases(t *testing.T, args ...string) *CLI {
	t.Helper()
	var cli CLI
	cfg, cfgErr := loadConfig(args)
	if cfgErr != nil {
		t.Fatal(cfgErr)
	}
	parser, err := newParser(&cli, cfg)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := parser.Parse(args); err != nil {
		t.Fatalf("parse %v: %v", args, err)
	}
	return &cli
}

func TestConfigKeysMatchFlags(t *testing.T) {
	var cli CLI
	parser, err := newParser(&cli, nil)
	if err != nil {
		t.Fatal(err)
	}
	flags := map[string]bool{}
	_ = kong.Visit(parser.Model.Node, func(n kong.Visitable, next kong.Next) error {
		if f, ok := n.(*kong.Flag); ok {
			flags[f.Name] = true
		}
		return next(nil)
	})
	for _, k := range config.Keys {
		if !flags[k.FlagName()] {
			t.Errorf("config key %q names no flag", k.Name)
		}
	}
}

func TestConfigSetsStore(t *testing.T) {
	root := t.TempDir()
	writeConfig(t, "store = \""+root+"\"\n")
	id := strings.TrimSpace(mustRun(t, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "From config"))
	if _, err := os.Stat(filepath.Join(root, id)); err != nil {
		t.Fatalf("case not in the configured store: %v", err)
	}
	if out := mustRun(t, "list"); !strings.Contains(out, id) {
		t.Errorf("list = %q, want %s", out, id)
	}

	// Precedence: flag > environment > file.
	other := t.TempDir()
	t.Setenv("CASES_STORE", other)
	if cli := parseCases(t, "list"); cli.Store != other {
		t.Errorf("store with CASES_STORE = %s, want the environment to beat the file", cli.Store)
	}
	third := t.TempDir()
	if cli := parseCases(t, "--store", third, "list"); cli.Store != third {
		t.Errorf("store with --store = %s, want the flag to win", cli.Store)
	}
	t.Setenv("CASES_STORE", "")
	if cli := parseCases(t, "list"); cli.Store != root {
		t.Errorf("store with blank CASES_STORE = %s, want the file's", cli.Store)
	}
}

func TestConfigExpandsHome(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	writeConfig(t, "store = \"~/cases\"\n")
	if cli := parseCases(t, "list"); cli.Store != filepath.Join(home, "cases") {
		t.Errorf("store = %s", cli.Store)
	}
}

func TestConfigSetsListen(t *testing.T) {
	writeConfig(t, "listen = \"0.0.0.0:1\"\n")
	if cli := parseCases(t, "serve"); cli.Serve.Listen != "0.0.0.0:1" {
		t.Errorf("listen = %s, want the file's", cli.Serve.Listen)
	}
	if cli := parseCases(t, "serve", "--listen", "localhost:2"); cli.Serve.Listen != "localhost:2" {
		t.Errorf("listen = %s, want the flag's", cli.Serve.Listen)
	}
	// The address the file supplies is checked like a flag.
	if r := runCases(t, "", "--store", t.TempDir(), "serve"); r.err == nil || !strings.Contains(r.err.Error(), "loopback") {
		t.Errorf("serve with a non-loopback listen from the file: err = %v", r.err)
	}
}

func TestConfigSetsNoOpen(t *testing.T) {
	if cli := parseCases(t, "serve"); cli.Serve.NoOpen {
		t.Error("no-open is set without the file")
	}
	writeConfig(t, "no-open = \"true\"\n")
	if cli := parseCases(t, "serve"); !cli.Serve.NoOpen {
		t.Error("no-open = false, want the file's true")
	}
	if cli := parseCases(t, "serve", "--no-open=false"); cli.Serve.NoOpen {
		t.Error("no-open = true, want the flag's false")
	}
}

func TestConfigFlagAndEnv(t *testing.T) {
	root := t.TempDir()
	path := filepath.Join(t.TempDir(), "mine.toml")
	if err := os.WriteFile(path, []byte("store = \""+root+"\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"--config", path, "list"}, {"--config=" + path, "list"}, {"list", "--config", path}} {
		if cli := parseCases(t, args...); cli.Store != root {
			t.Errorf("%v: store = %s", args, cli.Store)
		}
	}
	t.Setenv("CASES_CONFIG", path)
	if cli := parseCases(t, "list"); cli.Store != root {
		t.Errorf("CASES_CONFIG: store = %s", cli.Store)
	}
	if out := mustRun(t, "config", "path"); !strings.HasPrefix(out, path+" (exists)") {
		t.Errorf("config path = %q", out)
	}

	missing := filepath.Join(t.TempDir(), "none.toml")
	if r := runCases(t, "", "--config", missing, "config", "path"); r.err != nil || !strings.Contains(r.stdout, "(not found)") {
		t.Errorf("--config to a missing file: %+v", r)
	}
}

func TestConfigFlagLastWins(t *testing.T) {
	storeFile := func(name string) (path, root string) {
		root = t.TempDir()
		path = filepath.Join(t.TempDir(), name)
		if err := os.WriteFile(path, []byte("store = \""+root+"\"\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		return path, root
	}
	first, _ := storeFile("first.toml")
	second, secondRoot := storeFile("second.toml")
	env, envRoot := storeFile("env.toml")
	t.Setenv("CASES_CONFIG", env)

	for _, tc := range []struct {
		name string
		args []string
		want string
	}{
		{"two flags", []string{"--config", first, "list", "--config", second}, secondRoot},
		{"two flags, = spelling", []string{"--config=" + first, "--config=" + second, "list"}, secondRoot},
		{"empty last flag falls through to env", []string{"--config", first, "--config=", "list"}, envRoot},
		{"nothing after --", []string{"--config", second, "show", "--", "--config=" + first}, secondRoot},
	} {
		if cli := parseCases(t, tc.args...); cli.Store != tc.want {
			t.Errorf("%s: store = %s, want %s", tc.name, cli.Store, tc.want)
		}
	}
}

func TestBrokenConfigStopsCommandsButNotDiagnosis(t *testing.T) {
	path := writeConfig(t, "stor = \"x\"\n")
	r := runCases(t, "", "--store", t.TempDir(), "list")
	if r.err == nil || !strings.Contains(r.err.Error(), `unknown key "stor"`) {
		t.Errorf("list with a broken file: err = %v", r.err)
	}
	r = runCases(t, "", "config", "path")
	if r.err != nil || !strings.HasPrefix(r.stdout, path+" (exists)") || !strings.Contains(r.stderr, "cannot be used") {
		t.Errorf("config path with a broken file: %+v", r)
	}
	r = runCases(t, "", "config", "show")
	if r.err == nil || !strings.Contains(r.stdout, path) {
		t.Errorf("config show with a broken file: %+v", r)
	}
	r = runCases(t, "", "config", "init")
	if r.err == nil || !strings.Contains(r.err.Error(), "already exists") {
		t.Errorf("config init over a broken file: err = %v", r.err)
	}
}

func TestConfigShow(t *testing.T) {
	root := t.TempDir()
	path := writeConfig(t, "store = \""+root+"\"\n")
	out := mustRun(t, "config", "show")
	for _, want := range []string{"config: " + path + " (exists)", "store      " + root + "  config", "listen     127.0.0.1:8765", "no-open    false", "prune-age  720h", "  default"} {
		if !strings.Contains(out, want) {
			t.Errorf("config show lacks %q:\n%s", want, out)
		}
	}
	t.Setenv("CASES_STORE", "/from/env")
	if out := mustRun(t, "config", "show"); !strings.Contains(out, "/from/env") || !strings.Contains(out, "  env") {
		t.Errorf("config show with CASES_STORE:\n%s", out)
	}
}

func TestConfigInit(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	path := filepath.Join(dir, "cases", "config.toml")
	if out := mustRun(t, "config", "init"); !strings.Contains(out, path) {
		t.Errorf("config init = %q", out)
	}
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != config.Template() {
		t.Error("config init wrote something other than the template")
	}
	if r := runCases(t, "", "config", "init"); r.err == nil || !strings.Contains(r.err.Error(), "--force") {
		t.Errorf("second init: err = %v, want a refusal", r.err)
	}
	if err := os.WriteFile(path, []byte("store = \"/x\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	mustRun(t, "config", "init", "--force")
	if body, _ := os.ReadFile(path); string(body) != config.Template() {
		t.Error("init --force did not replace the file")
	}
	// The template is a valid, empty config.
	if out := mustRun(t, "config", "show"); !strings.Contains(out, "  default") {
		t.Errorf("config show after init:\n%s", out)
	}
}
