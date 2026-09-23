package service

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"testing"
	"time"
)

// fakeRunner records the commands it is given instead of running them. A
// command whose line starts with a key in fail fails with that output.
type fakeRunner struct {
	calls []string
	fail  map[string]string
}

func (f *fakeRunner) run(name string, args ...string) ([]byte, error) {
	line := strings.Join(append([]string{name}, args...), " ")
	f.calls = append(f.calls, line)
	for prefix, out := range f.fail {
		if strings.HasPrefix(line, prefix) {
			return []byte(out), errors.New("exit status 1")
		}
	}
	return nil, nil
}

func newManager(t *testing.T, goos string, fail map[string]string) (*Manager, *fakeRunner) {
	t.Helper()
	f := &fakeRunner{fail: fail}
	return &Manager{GOOS: goos, Dir: filepath.Join(t.TempDir(), "agents"), UID: 501, Run: f.run}, f
}

func testSpec(t *testing.T) Spec {
	return Spec{
		Args: []string{"/opt/my tools/cases", "--store", "/data/a&b <1>.db", "serve", "--no-open", "--listen", "127.0.0.1:8765"},
		Env:  []string{"HOME=/Users/me", "XDG_STATE_HOME=/Users/me/.local/state"},
		Log:  filepath.Join(t.TempDir(), "state", "cases", "serve.log"),
	}
}

func TestPlist(t *testing.T) {
	s := testSpec(t)
	got := Plist(s)
	for _, want := range []string{
		"<key>Label</key>\n\t<string>" + Label + "</string>",
		"\t\t<string>/opt/my tools/cases</string>\n\t\t<string>--store</string>\n\t\t<string>/data/a&amp;b &lt;1&gt;.db</string>\n\t\t<string>serve</string>",
		"<key>HOME</key>\n\t\t<string>/Users/me</string>",
		"<key>XDG_STATE_HOME</key>\n\t\t<string>/Users/me/.local/state</string>",
		"<key>RunAtLoad</key>\n\t<true/>",
		"<key>KeepAlive</key>\n\t<true/>",
		"<key>StandardErrorPath</key>\n\t<string>" + s.Log + "</string>",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("plist lacks %q:\n%s", want, got)
		}
	}
	// plutil ships with macOS; elsewhere the checks above have to do.
	plutil, err := exec.LookPath("plutil")
	if err != nil {
		return
	}
	path := filepath.Join(t.TempDir(), "x.plist")
	if err := os.WriteFile(path, []byte(got), 0o644); err != nil {
		t.Fatal(err)
	}
	if out, err := exec.Command(plutil, "-lint", path).CombinedOutput(); err != nil {
		t.Errorf("plutil -lint: %v: %s", err, out)
	}
}

func TestUnit(t *testing.T) {
	s := Spec{
		Args: []string{"/opt/my tools/cases", "--as", `R "100%" $USER\x`},
		Env:  []string{"HOME=/home/me", "XDG_STATE_HOME=/home/me/50%$"},
	}
	want := `[Unit]
Description=cases web inbox (cases serve)

[Service]
ExecStart="/opt/my tools/cases" "--as" "R \"100%%\" $$USER\\x"
Environment="HOME=/home/me"
Environment="XDG_STATE_HOME=/home/me/50%%$"
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
`
	if got := Unit(s); got != want {
		t.Errorf("unit:\n%s\nwant:\n%s", got, want)
	}
}

func TestInstallLaunchd(t *testing.T) {
	m, f := newManager(t, "darwin", map[string]string{"launchctl print": "Could not find service"})
	s := testSpec(t)
	path, err := m.Install(s)
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(m.Dir, Label+".plist"); path != want {
		t.Errorf("path = %s, want %s", path, want)
	}
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != Plist(s) {
		t.Errorf("file:\n%s\nwant:\n%s", body, Plist(s))
	}
	if fi, _ := os.Stat(path); fi.Mode().Perm() != 0o644 {
		t.Errorf("mode = %v, want 0644", fi.Mode().Perm())
	}
	if _, err := os.Stat(filepath.Dir(s.Log)); err != nil {
		t.Errorf("log directory: %v", err)
	}
	want := []string{"launchctl print gui/501/" + Label, "launchctl bootstrap gui/501 " + path}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

func TestInstallLaunchdReplacesLoadedAgent(t *testing.T) {
	m, f := newManager(t, "darwin", nil)
	path, err := m.Install(testSpec(t))
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"launchctl print gui/501/" + Label, "launchctl bootout gui/501/" + Label, "launchctl bootstrap gui/501 " + path}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

func TestInstallSystemd(t *testing.T) {
	m, f := newManager(t, "linux", nil)
	s := testSpec(t)
	path, err := m.Install(s)
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(m.Dir, UnitName); path != want {
		t.Errorf("path = %s, want %s", path, want)
	}
	if body, _ := os.ReadFile(path); string(body) != Unit(s) {
		t.Errorf("file:\n%s\nwant:\n%s", body, Unit(s))
	}
	want := []string{"systemctl --user daemon-reload", "systemctl --user enable " + UnitName, "systemctl --user restart " + UnitName}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

func TestInstallReportsFailure(t *testing.T) {
	m, _ := newManager(t, "linux", map[string]string{"systemctl --user restart": "Failed to connect to bus"})
	_, err := m.Install(testSpec(t))
	if err == nil || !strings.Contains(err.Error(), "systemctl --user restart "+UnitName) || !strings.Contains(err.Error(), "Failed to connect to bus") {
		t.Errorf("err = %v, want the command and its output", err)
	}
}

func TestUninstallLaunchd(t *testing.T) {
	m, f := newManager(t, "darwin", nil)
	path, err := m.Install(testSpec(t))
	if err != nil {
		t.Fatal(err)
	}
	f.calls = nil
	if got, err := m.Uninstall(); err != nil || got != path {
		t.Fatalf("Uninstall = %s, %v", got, err)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("plist still there: %v", err)
	}
	want := []string{"launchctl print gui/501/" + Label, "launchctl bootout gui/501/" + Label}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

func TestUninstallLaunchdNotLoaded(t *testing.T) {
	m, f := newManager(t, "darwin", map[string]string{"launchctl print": "Could not find service"})
	if _, err := m.Uninstall(); !errors.Is(err, ErrNotInstalled) {
		t.Errorf("nothing installed: err = %v, want ErrNotInstalled", err)
	}
	// A plist that was never loaded is still removed, without a bootout.
	path, err := m.Install(testSpec(t))
	if err != nil {
		t.Fatal(err)
	}
	f.calls = nil
	if _, err := m.Uninstall(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("plist still there: %v", err)
	}
	if want := []string{"launchctl print gui/501/" + Label}; !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

func TestUninstallSystemd(t *testing.T) {
	m, f := newManager(t, "linux", map[string]string{"systemctl --user is-active": "inactive"})
	if _, err := m.Uninstall(); !errors.Is(err, ErrNotInstalled) {
		t.Errorf("nothing installed: err = %v, want ErrNotInstalled", err)
	}
	if want := []string{"systemctl --user is-active " + UnitName}; !slices.Equal(f.calls, want) {
		t.Errorf("calls with nothing installed = %q, want %q", f.calls, want)
	}
	path, err := m.Install(testSpec(t))
	if err != nil {
		t.Fatal(err)
	}
	f.calls = nil
	if _, err := m.Uninstall(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("unit still there: %v", err)
	}
	want := []string{"systemctl --user disable --now " + UnitName, "systemctl --user daemon-reload"}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

// A unit whose file is gone but still runs is stopped.
func TestUninstallSystemdWithoutFile(t *testing.T) {
	m, f := newManager(t, "linux", nil)
	path, err := m.Uninstall()
	if err != nil {
		t.Fatal(err)
	}
	if want := filepath.Join(m.Dir, UnitName); path != want {
		t.Errorf("path = %s, want %s", path, want)
	}
	want := []string{"systemctl --user is-active " + UnitName, "systemctl --user stop " + UnitName, "systemctl --user daemon-reload"}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

// After an uninstall launchd can still be tearing the job down, so a first
// bootstrap is retried as one after a bootout is.
func TestInstallLaunchdRetriesFirstBootstrap(t *testing.T) {
	defer func(d time.Duration) { bootstrapRetry = d }(bootstrapRetry)
	bootstrapRetry = time.Millisecond
	m, _ := newManager(t, "darwin", map[string]string{"launchctl print": "Could not find service"})
	fails := 2
	run := m.Run
	m.Run = func(name string, args ...string) ([]byte, error) {
		if name == "launchctl" && args[0] == "bootstrap" && fails > 0 {
			fails--
			return []byte("Bootstrap failed: 5: Input/output error"), errors.New("exit status 5")
		}
		return run(name, args...)
	}
	if _, err := m.Install(testSpec(t)); err != nil || fails != 0 {
		t.Errorf("Install = %v with %d failures left", err, fails)
	}
}

func TestUnsupportedSystem(t *testing.T) {
	m, f := newManager(t, "windows", nil)
	if _, err := m.Install(testSpec(t)); err == nil || !strings.Contains(err.Error(), "not supported on windows") {
		t.Errorf("Install: err = %v", err)
	}
	if _, err := m.Uninstall(); err == nil || !strings.Contains(err.Error(), "not supported on windows") {
		t.Errorf("Uninstall: err = %v", err)
	}
	if len(f.calls) != 0 {
		t.Errorf("calls = %q", f.calls)
	}
}

func TestDefaultDir(t *testing.T) {
	home, cfg := t.TempDir(), t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", cfg)
	m, err := Default()
	if err != nil {
		t.Fatal(err)
	}
	switch runtime.GOOS {
	case "darwin":
		if want := filepath.Join(home, "Library", "LaunchAgents"); m.Dir != want {
			t.Errorf("Dir = %s, want %s", m.Dir, want)
		}
	case "linux":
		if want := filepath.Join(cfg, "systemd", "user"); m.Dir != want {
			t.Errorf("Dir = %s, want %s", m.Dir, want)
		}
		t.Setenv("XDG_CONFIG_HOME", "")
		if m, _ := Default(); m.Dir != filepath.Join(home, ".config", "systemd", "user") {
			t.Errorf("Dir without XDG_CONFIG_HOME = %s", m.Dir)
		}
	}
	if m.UID != os.Getuid() {
		t.Errorf("UID = %d, want %d", m.UID, os.Getuid())
	}
}

func TestInstallLaunchdRetriesBootstrapAfterBootout(t *testing.T) {
	bootstrapRetry = time.Millisecond
	t.Cleanup(func() { bootstrapRetry = 300 * time.Millisecond })
	m, f := newManager(t, "darwin", nil)
	fails := 2
	m.Run = func(name string, args ...string) ([]byte, error) {
		out, err := f.run(name, args...)
		if len(args) > 0 && args[0] == "bootstrap" && fails > 0 {
			fails--
			return []byte("Bootstrap failed: 5: Input/output error"), errors.New("exit status 5")
		}
		return out, err
	}
	path, err := m.Install(testSpec(t))
	if err != nil {
		t.Fatal(err)
	}
	bootstrap := "launchctl bootstrap gui/501 " + path
	want := []string{"launchctl print gui/501/" + Label, "launchctl bootout gui/501/" + Label, bootstrap, bootstrap, bootstrap}
	if !slices.Equal(f.calls, want) {
		t.Errorf("calls = %q, want %q", f.calls, want)
	}
}

// Args reads back what Install wrote, including quoting and escapes.
func TestArgsRoundTrip(t *testing.T) {
	args := []string{"/opt/my tools/cases", "--store", `/d/a&b <1> "q" 50% $HOME\x`, "serve", "--as", "line\nbreak"}
	for _, goos := range []string{"darwin", "linux"} {
		m, _ := newManager(t, goos, map[string]string{"launchctl print": ""})
		if _, err := m.Args(); !errors.Is(err, os.ErrNotExist) {
			t.Errorf("%s: Args with nothing installed: err = %v", goos, err)
		}
		if _, err := m.Install(Spec{Args: args, Env: []string{"HOME=/h"}, Log: filepath.Join(t.TempDir(), "log")}); err != nil {
			t.Fatal(err)
		}
		got, err := m.Args()
		if err != nil || !slices.Equal(got, args) {
			t.Errorf("%s: Args = %q, %v; want %q", goos, got, err, args)
		}
		path, _ := m.Path()
		if err := os.WriteFile(path, []byte("[Service]\nRestart=always\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		if _, err := m.Args(); err == nil {
			t.Errorf("%s: Args of a file with no command: no error", goos)
		}
	}
}

func TestActive(t *testing.T) {
	for _, tc := range []struct {
		goos, query string
	}{
		{"darwin", "launchctl print gui/501/" + Label},
		{"linux", "systemctl --user is-active " + UnitName},
	} {
		m, f := newManager(t, tc.goos, nil)
		if active, err := m.Active(); err != nil || !active {
			t.Errorf("%s: Active = %v, %v", tc.goos, active, err)
		}
		m, f2 := newManager(t, tc.goos, map[string]string{tc.query: "inactive"})
		if active, err := m.Active(); err != nil || active {
			t.Errorf("%s: Active when the query fails = %v, %v", tc.goos, active, err)
		}
		for _, calls := range [][]string{f.calls, f2.calls} {
			if !slices.Equal(calls, []string{tc.query}) {
				t.Errorf("%s: calls = %q, want %q", tc.goos, calls, tc.query)
			}
		}
	}
}

// A unit waiting out RestartSec is still the service manager's to restart.
func TestActiveSystemdWhileRestarting(t *testing.T) {
	m, _ := newManager(t, "linux", map[string]string{"systemctl --user is-active": "activating\n"})
	if active, err := m.Active(); err != nil || !active {
		t.Errorf("Active while activating = %v, %v; want true", active, err)
	}
}
