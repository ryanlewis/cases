package main

import (
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/instance"
	"github.com/ryanlewis/cases/internal/service"
)

// serviceRunner records the launchctl and systemctl commands a test's
// service manager is given, and runs none of them. `launchctl print` and
// `systemctl is-active` succeed only when active is set.
type serviceRunner struct {
	calls  []string
	active bool
}

func (r *serviceRunner) run(name string, args ...string) ([]byte, error) {
	r.calls = append(r.calls, strings.Join(append([]string{name}, args...), " "))
	query := (name == "launchctl" && args[0] == "print") || (name == "systemctl" && args[1] == "is-active")
	if query && !r.active {
		return []byte("Could not find service"), errors.New("exit status 113")
	}
	return nil, nil
}

// runService runs the arguments with a service manager for goos that writes
// into dir and records its commands in r.
func runService(t *testing.T, goos, dir string, r *serviceRunner, args ...string) result {
	t.Helper()
	m := &service.Manager{GOOS: goos, Dir: dir, UID: 501, Run: r.run}
	return runCasesDeps(t, nil, func(d *Deps) {
		d.Service = func() (*service.Manager, error) { return m, nil }
	}, "", args...)
}

func TestServiceInstallWritesUnit(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{}
	addr := freeAddr(t)
	res := runService(t, "linux", dir, r, "--store", storePath, "service", "install", "--listen", addr, "--as", "Ryan Lewis")
	if res.err != nil {
		t.Fatalf("install: %v\n%s", res.err, res.stderr)
	}
	path := filepath.Join(dir, service.UnitName)
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	exe, _ := executable()
	cfg := filepath.Join(os.Getenv("XDG_CONFIG_HOME"), "cases", "config.toml")
	state := os.Getenv("XDG_STATE_HOME")
	for _, want := range []string{
		`ExecStart="` + exe + `" "--store" "` + storePath + `" "--config" "` + cfg + `" "serve" "--no-open" "--listen" "` + addr + `" "--as" "Ryan Lewis"` + "\n",
		`Environment="HOME=` + os.Getenv("HOME") + `"`,
		`Environment="XDG_STATE_HOME=` + state + `"`,
	} {
		if !strings.Contains(string(body), want) {
			t.Errorf("unit lacks %q:\n%s", want, body)
		}
	}
	if len(r.calls) != 3 || r.calls[2] != "systemctl --user restart "+service.UnitName {
		t.Errorf("calls = %q", r.calls)
	}
	for _, want := range []string{"Installed the cases serve service: " + path, "--listen " + addr + " --as 'Ryan Lewis'\n", "cases service\n", "journalctl --user -u " + service.UnitName} {
		if !strings.Contains(res.stdout, want) {
			t.Errorf("stdout lacks %q:\n%s", want, res.stdout)
		}
	}
}

func TestServiceInstallWritesPlist(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{}
	addr := freeAddr(t)
	res := runService(t, "darwin", dir, r, "--store", storePath, "service", "install", "--listen", addr)
	if res.err != nil {
		t.Fatalf("install: %v\n%s", res.err, res.stderr)
	}
	path := filepath.Join(dir, service.Label+".plist")
	body, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	log := filepath.Join(os.Getenv("XDG_STATE_HOME"), "cases", "serve.log")
	for _, want := range []string{
		"<string>" + storePath + "</string>\n",
		"<string>serve</string>\n\t\t<string>--no-open</string>\n\t\t<string>--listen</string>\n\t\t<string>" + addr + "</string>\n\t</array>",
		"<key>StandardOutPath</key>\n\t<string>" + log + "</string>",
	} {
		if !strings.Contains(string(body), want) {
			t.Errorf("plist lacks %q:\n%s", want, body)
		}
	}
	if strings.Contains(string(body), "--as") {
		t.Errorf("plist names --as without one set:\n%s", body)
	}
	if want := "launchctl bootstrap gui/501 " + path; len(r.calls) != 2 || r.calls[1] != want {
		t.Errorf("calls = %q, want print then %q", r.calls, want)
	}
	for _, want := range []string{path, "cases service\n", "launchctl print gui/501/" + service.Label, "Log: " + log} {
		if !strings.Contains(res.stdout, want) {
			t.Errorf("stdout lacks %q:\n%s", want, res.stdout)
		}
	}
}

func TestServiceInstallTakesConfig(t *testing.T) {
	addr := freeAddr(t)
	cfg := writeConfig(t, "listen = \""+addr+"\"\nname = \"From File\"\n")
	dir := t.TempDir()
	res := runService(t, "linux", dir, &serviceRunner{}, "--store", newStore(t), "service", "install")
	if res.err != nil {
		t.Fatalf("install: %v\n%s", res.err, res.stderr)
	}
	body, _ := os.ReadFile(filepath.Join(dir, service.UnitName))
	want := `"--config" "` + cfg + `" "serve" "--no-open" "--listen" "` + addr + `" "--as" "From File"`
	if !strings.Contains(string(body), want) {
		t.Errorf("unit lacks %q:\n%s", want, body)
	}
}

func TestServiceInstallRefusesNonLoopback(t *testing.T) {
	dir := t.TempDir()
	r := &serviceRunner{}
	res := runService(t, "linux", dir, r, "--store", newStore(t), "service", "install", "--listen", "0.0.0.0:8765")
	if res.err == nil || !strings.Contains(res.err.Error(), "loopback") {
		t.Errorf("err = %v, want a loopback refusal", res.err)
	}
	if entries, _ := os.ReadDir(dir); len(entries) != 0 || len(r.calls) != 0 {
		t.Errorf("wrote %v and ran %q", entries, r.calls)
	}
}

func TestServiceUninstall(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{}
	if res := runService(t, "linux", dir, r, "--store", storePath, "service", "install", "--listen", freeAddr(t)); res.err != nil {
		t.Fatal(res.err)
	}
	r.calls = nil
	res := runService(t, "linux", dir, r, "--store", storePath, "service", "uninstall")
	if res.err != nil {
		t.Fatalf("uninstall: %v", res.err)
	}
	path := filepath.Join(dir, service.UnitName)
	if !strings.Contains(res.stdout, "removed "+path) {
		t.Errorf("stdout = %q", res.stdout)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("unit still there: %v", err)
	}
	if len(r.calls) != 2 || r.calls[0] != "systemctl --user disable --now "+service.UnitName {
		t.Errorf("calls = %q", r.calls)
	}
	res = runService(t, "linux", dir, r, "--store", storePath, "service", "uninstall")
	if !errors.Is(res.err, service.ErrNotInstalled) {
		t.Errorf("second uninstall: err = %v, want ErrNotInstalled", res.err)
	}
}

// runCases sets no service manager, so no test can reach the real one.
func TestServiceInstallWithoutManager(t *testing.T) {
	for _, sub := range []string{"install", "uninstall"} {
		if r := runCases(t, "", "--store", newStore(t), "service", sub); r.err == nil || !strings.Contains(r.err.Error(), "no service manager") {
			t.Errorf("serve %s: err = %v", sub, r.err)
		}
	}
}

// freeAddr is a loopback address nothing listens on, since install refuses
// one that is taken.
func freeAddr(t *testing.T) string {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	addr := ln.Addr().String()
	ln.Close()
	return addr
}

// fakeServe records a live serve for storePath, as `cases serve` does, with
// this process's pid and a listener that accepts connections.
func fakeServe(t *testing.T, storePath string) instance.Info {
	t.Helper()
	return fakeServeAt(t, storePath, "127.0.0.1:0")
}

// fakeServeAt is fakeServe listening on addr.
func fakeServeAt(t *testing.T, storePath, addr string) instance.Info {
	t.Helper()
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { ln.Close() })
	info := instance.Info{PID: os.Getpid(), URL: "http://" + ln.Addr().String() + "/", Addr: ln.Addr().String(), Store: storePath, StartedAt: time.Now().UTC()}
	if err := instance.Write(info); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = instance.Remove(storePath, info.PID) })
	return info
}

func TestServiceInstallRefusesWhileServeRuns(t *testing.T) {
	storePath := newStore(t)
	info := fakeServe(t, storePath)
	dir := t.TempDir()
	r := &serviceRunner{}
	res := runService(t, "darwin", dir, r, "--store", storePath, "service", "install", "--listen", freeAddr(t))
	if res.err == nil || !strings.Contains(res.err.Error(), info.URL) || !strings.Contains(res.err.Error(), "stop it") {
		t.Errorf("err = %v, want a refusal naming %s", res.err, info.URL)
	}
	if entries, _ := os.ReadDir(dir); len(entries) != 0 || len(r.calls) != 0 {
		t.Errorf("wrote %v and ran %q", entries, r.calls)
	}
}

func TestServiceInstallRefusesTakenAddress(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	dir := t.TempDir()
	r := &serviceRunner{}
	res := runService(t, "linux", dir, r, "--store", newStore(t), "service", "install", "--listen", ln.Addr().String())
	if res.err == nil || !strings.Contains(res.err.Error(), ln.Addr().String()+" is already in use") {
		t.Errorf("err = %v, want the address named as in use", res.err)
	}
	if entries, _ := os.ReadDir(dir); len(entries) != 0 || len(r.calls) != 0 {
		t.Errorf("wrote %v and ran %q", entries, r.calls)
	}
}

// On a reinstall the running serve is the service, which install restarts.
func TestServiceReinstallWhileServiceRuns(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	addr := freeAddr(t)
	if res := runService(t, "linux", dir, &serviceRunner{}, "--store", storePath, "service", "install", "--listen", addr); res.err != nil {
		t.Fatal(res.err)
	}
	fakeServeAt(t, storePath, addr)
	res := runService(t, "linux", dir, &serviceRunner{active: true}, "--store", storePath, "service", "install", "--listen", addr)
	if res.err != nil {
		t.Errorf("reinstall: %v", res.err)
	}
}

// A reinstall still checks what the loaded service does not cover: a new
// address someone else holds, a serve on a new store, and anything at all
// when the service is not loaded.
func TestServiceReinstallChecksWhatChanged(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	addr := freeAddr(t)
	if res := runService(t, "linux", dir, &serviceRunner{}, "--store", storePath, "service", "install", "--listen", addr); res.err != nil {
		t.Fatal(res.err)
	}
	fakeServeAt(t, storePath, addr)

	held, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer held.Close()
	res := runService(t, "linux", dir, &serviceRunner{active: true}, "--store", storePath, "service", "install", "--listen", held.Addr().String())
	if res.err == nil || !strings.Contains(res.err.Error(), "is already in use") {
		t.Errorf("new held address: err = %v, want it named as in use", res.err)
	}

	other := newStore(t)
	info := fakeServe(t, other)
	res = runService(t, "linux", dir, &serviceRunner{active: true}, "--store", other, "service", "install", "--listen", addr)
	if res.err == nil || !strings.Contains(res.err.Error(), info.URL) {
		t.Errorf("new store with a serve: err = %v, want a refusal naming %s", res.err, info.URL)
	}

	res = runService(t, "linux", dir, &serviceRunner{}, "--store", storePath, "service", "install", "--listen", addr)
	if res.err == nil || !strings.Contains(res.err.Error(), "stop it") {
		t.Errorf("file installed but not loaded: err = %v, want a refusal", res.err)
	}
}

// serviceStatus runs bare `cases service` on storePath and returns what it
// printed and whether it exited 0.
func serviceStatus(t *testing.T, goos, dir string, r *serviceRunner, storePath string, args ...string) (string, bool) {
	t.Helper()
	r.calls = nil
	res := runService(t, goos, dir, r, append([]string{"--store", storePath, "service"}, args...)...)
	var ee *exitError
	if res.err != nil && (!errors.As(res.err, &ee) || ee.code != 1) {
		t.Fatalf("cases service: %v", res.err)
	}
	for _, c := range r.calls {
		if !strings.HasPrefix(c, "launchctl print ") && !strings.HasPrefix(c, "systemctl --user is-active ") {
			t.Errorf("cases service ran %q, which is not a query", c)
		}
	}
	t.Logf("cases service (exit ok %v):\n%s", res.err == nil, res.stdout)
	return res.stdout, res.err == nil
}

// installService installs the service for storePath through the CLI and
// returns the address it listens on.
func installService(t *testing.T, goos, dir string, r *serviceRunner, storePath string) string {
	t.Helper()
	addr := freeAddr(t)
	if res := runService(t, goos, dir, r, "--store", storePath, "service", "install", "--listen", addr); res.err != nil {
		t.Fatalf("install: %v", res.err)
	}
	return addr
}

func wantLines(t *testing.T, out string, want ...string) {
	t.Helper()
	for _, w := range want {
		if !strings.Contains(out, w) {
			t.Errorf("output lacks %q:\n%s", w, out)
		}
	}
}

func TestServiceStatusNotInstalled(t *testing.T) {
	dir := t.TempDir()
	out, ok := serviceStatus(t, "darwin", dir, &serviceRunner{}, newStore(t))
	if ok {
		t.Error("exit 0 with nothing installed")
	}
	wantLines(t, out, "Installed: no ("+filepath.Join(dir, service.Label+".plist")+")", "Loaded:    no (launchctl print gui/501/"+service.Label+")", "Inbox:     not running")
	if strings.Contains(out, "Problem") {
		t.Errorf("problem reported with nothing installed:\n%s", out)
	}
}

func TestServiceStatusHandStartedServe(t *testing.T) {
	storePath := newStore(t)
	info := fakeServe(t, storePath)
	out, ok := serviceStatus(t, "linux", t.TempDir(), &serviceRunner{}, storePath)
	if ok {
		t.Error("exit 0 with nothing installed")
	}
	wantLines(t, out, "Inbox:     "+info.URL, "Problem:   not installed, but a cases serve started by hand is running")
}

func TestServiceStatusRunning(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{}
	addr := installService(t, "linux", dir, r, storePath)
	r.active = true
	info := fakeServe(t, storePath)
	out, ok := serviceStatus(t, "linux", dir, r, storePath)
	if !ok {
		t.Error("exit 1 with the service installed, active and answering")
	}
	exe, _ := executable()
	wantLines(t, out, "Installed: "+filepath.Join(dir, service.UnitName)+"\n", "Runs:      "+exe+" --store "+storePath,
		"Store:     "+storePath+"\n", "Listen:    "+addr+"\n", "Loaded:    yes (systemctl --user status "+service.UnitName+")", "Inbox:     "+info.URL)
	if strings.Contains(out, "Problem") {
		t.Errorf("problem reported:\n%s", out)
	}
	if want := "systemctl --user is-active " + service.UnitName; len(r.calls) != 1 || r.calls[0] != want {
		t.Errorf("calls = %q, want %q", r.calls, want)
	}

	var got serviceReport
	res := runService(t, "linux", dir, r, "--store", storePath, "service", "--json")
	if err := json.Unmarshal([]byte(res.stdout), &got); err != nil || res.err != nil {
		t.Fatalf("--json: %v, %v: %s", res.err, err, res.stdout)
	}
	if !got.Installed || !got.Loaded || !got.Running || got.Store != storePath || got.Listen != addr || got.PID != info.PID || len(got.Problems) != 0 {
		t.Errorf("--json = %+v", got)
	}
}

func TestServiceStatusNotLoaded(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{}
	installService(t, "darwin", dir, r, storePath)
	out, ok := serviceStatus(t, "darwin", dir, r, storePath)
	if ok {
		t.Error("exit 0 with the agent not loaded")
	}
	wantLines(t, out, "Loaded:    no", "Problem:   installed but not loaded in launchd")
}

func TestServiceStatusNotAnswering(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{}
	installService(t, "darwin", dir, r, storePath)
	r.active = true
	out, ok := serviceStatus(t, "darwin", dir, r, storePath)
	if ok {
		t.Error("exit 0 with the inbox not answering")
	}
	wantLines(t, out, "Loaded:    yes", "Inbox:     not running", "Problem:   loaded but the inbox does not answer; see launchctl print")
}

func TestServiceStatusLoadedWithoutFile(t *testing.T) {
	out, ok := serviceStatus(t, "darwin", t.TempDir(), &serviceRunner{active: true}, newStore(t))
	if ok {
		t.Error("exit 0 with no plist")
	}
	wantLines(t, out, "Installed: no", "Problem:   loaded in launchd but its file is missing")
}

func TestServiceStatusOtherStoreAndBinary(t *testing.T) {
	storePath := newStore(t)
	dir := t.TempDir()
	r := &serviceRunner{active: true}
	installService(t, "linux", dir, r, storePath)
	// The service's own store is the one checked for a running inbox.
	fakeServe(t, storePath)
	other := newStore(t)
	out, ok := serviceStatus(t, "linux", dir, r, other)
	if !ok {
		t.Error("exit 1 with the service running for its own store")
	}
	wantLines(t, out, "Store:     "+storePath+"\n", "Problem:   installed for another store: "+storePath+" (this command uses "+other+")")

	m := &service.Manager{GOOS: "linux", Dir: dir, Run: r.run}
	if _, err := m.Install(service.Spec{Args: []string{"/nowhere/cases", "--store", storePath, "serve", "--listen", "127.0.0.1:1"}}); err != nil {
		t.Fatal(err)
	}
	out, _ = serviceStatus(t, "linux", dir, r, storePath)
	wantLines(t, out, "Problem:   installed for another binary: /nowhere/cases")
}
