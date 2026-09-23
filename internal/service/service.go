// Package service installs `cases serve` as a user service that starts at
// login and is restarted when it stops: a launchd agent on macOS, a systemd
// user unit on Linux. The service manager is driven through a Runner, so
// tests never reach the real one.
package service

import (
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

const (
	// Label is the launchd job's label and the plist's base name.
	Label = "com.github.ryanlewis.cases.serve"
	// UnitName is the systemd user unit.
	UnitName = "cases-serve.service"
)

// bootstrapTries and bootstrapRetry bound how long Install waits for launchd
// to accept the agent while a job booted out just before is torn down.
var (
	bootstrapTries = 10
	bootstrapRetry = 300 * time.Millisecond
)

// ErrNotInstalled is returned by Uninstall when there is nothing to remove.
var ErrNotInstalled = errors.New("the cases serve service is not installed")

// Spec is what the service runs.
type Spec struct {
	// Args is the program and its arguments.
	Args []string
	// Env is the service's environment, as KEY=VALUE.
	Env []string
	// Log is the file launchd appends the output to. systemd sends it to the
	// journal and ignores Log.
	Log string
}

// Runner runs a service manager command, launchctl or systemctl, and returns
// its combined output.
type Runner func(name string, args ...string) ([]byte, error)

// Manager installs and removes the service for one operating system.
type Manager struct {
	// GOOS picks launchd ("darwin") or systemd ("linux").
	GOOS string
	// Dir is where the plist or unit file goes.
	Dir string
	// UID is the user whose launchd domain (gui/UID) the agent is loaded in.
	UID int
	Run Runner
}

// Default is the manager for this machine and user: the plist in
// ~/Library/LaunchAgents on macOS, the unit in $XDG_CONFIG_HOME/systemd/user
// (or ~/.config/systemd/user) on Linux, and the real launchctl or systemctl.
func Default() (*Manager, error) {
	m := &Manager{GOOS: runtime.GOOS, UID: os.Getuid(), Run: execRunner}
	home := os.Getenv("HOME")
	switch m.GOOS {
	case "darwin":
		if home == "" {
			return nil, errors.New("cannot locate ~/Library/LaunchAgents: $HOME is not set")
		}
		m.Dir = filepath.Join(home, "Library", "LaunchAgents")
	case "linux":
		base := os.Getenv("XDG_CONFIG_HOME")
		if base == "" {
			if home == "" {
				return nil, errors.New("cannot locate the systemd user directory: neither $XDG_CONFIG_HOME nor $HOME is set")
			}
			base = filepath.Join(home, ".config")
		}
		m.Dir = filepath.Join(base, "systemd", "user")
	}
	return m, nil
}

func execRunner(name string, args ...string) ([]byte, error) {
	return exec.Command(name, args...).CombinedOutput()
}

// Path is the plist or unit file.
func (m *Manager) Path() (string, error) {
	switch m.GOOS {
	case "darwin":
		return filepath.Join(m.Dir, Label+".plist"), nil
	case "linux":
		return filepath.Join(m.Dir, UnitName), nil
	}
	return "", fmt.Errorf("a cases serve service is not supported on %s; run cases serve under your own supervisor", m.GOOS)
}

// Render is the plist or unit file for s.
func (m *Manager) Render(s Spec) (string, error) {
	switch m.GOOS {
	case "darwin":
		return Plist(s), nil
	case "linux":
		return Unit(s), nil
	}
	_, err := m.Path()
	return "", err
}

// Check is the command that shows the service manager's view of the service.
func (m *Manager) Check() string {
	if m.GOOS == "darwin" {
		return fmt.Sprintf("launchctl print gui/%d/%s", m.UID, Label)
	}
	return "systemctl --user status " + UnitName
}

// Installed reports whether the plist or unit file is there.
func (m *Manager) Installed() (bool, error) {
	path, err := m.Path()
	if err != nil {
		return false, err
	}
	_, err = os.Stat(path)
	if errors.Is(err, fs.ErrNotExist) {
		return false, nil
	}
	return err == nil, err
}

// Install writes the file for s and has the service manager start it,
// replacing a service already installed. It returns the file's path.
func (m *Manager) Install(s Spec) (string, error) {
	path, err := m.Path()
	if err != nil {
		return "", err
	}
	body, err := m.Render(s)
	if err != nil {
		return "", err
	}
	if m.GOOS == "darwin" && s.Log != "" {
		// launchd opens the log but does not make its directory.
		if err := os.MkdirAll(filepath.Dir(s.Log), 0o700); err != nil {
			return "", err
		}
	}
	// Ask launchd before the file is replaced, so a query that fails leaves
	// the installed file matching the agent that runs.
	loaded := false
	if m.GOOS == "darwin" {
		if loaded, err = m.loaded(); err != nil {
			return "", err
		}
	}
	if err := os.MkdirAll(m.Dir, 0o755); err != nil {
		return "", err
	}
	if err := writeFile(path, body); err != nil {
		return "", err
	}
	if m.GOOS == "darwin" {
		// A loaded agent keeps the old plist until it is booted out, and
		// bootstrap refuses a label that is already loaded.
		if loaded {
			if err := m.run("launchctl", "bootout", m.target()); err != nil {
				return "", err
			}
		}
		// bootout can return while launchd is still tearing the job down,
		// and a bootstrap in that window fails (5: Input/output error). The
		// window also follows an uninstall run just before, whose job print
		// no longer shows. Give the new job a few tries rather than leave
		// nothing running until the next login.
		for try := 1; ; try++ {
			err := m.run("launchctl", "bootstrap", m.domain(), path)
			if err == nil || try == bootstrapTries {
				return path, err
			}
			time.Sleep(bootstrapRetry)
		}
	}
	for _, args := range [][]string{
		{"--user", "daemon-reload"},
		{"--user", "enable", UnitName},
		// restart starts a stopped unit and picks up a changed one.
		{"--user", "restart", UnitName},
	} {
		if err := m.run("systemctl", args...); err != nil {
			return "", err
		}
	}
	return path, nil
}

// Uninstall stops the service and removes its file. It returns the file's
// path, or ErrNotInstalled when there was no file and nothing loaded.
func (m *Manager) Uninstall() (string, error) {
	path, err := m.Path()
	if err != nil {
		return "", err
	}
	_, statErr := os.Stat(path)
	exists := statErr == nil
	if statErr != nil && !errors.Is(statErr, fs.ErrNotExist) {
		return "", statErr
	}
	if m.GOOS == "darwin" {
		loaded, err := m.loaded()
		if err != nil {
			return "", err
		}
		if !exists && !loaded {
			return "", ErrNotInstalled
		}
		if loaded {
			if err := m.run("launchctl", "bootout", m.target()); err != nil {
				return "", err
			}
		}
		if err := os.Remove(path); err != nil && !errors.Is(err, fs.ErrNotExist) {
			return "", err
		}
		return path, nil
	}
	if !exists {
		// A unit whose file was removed keeps running until it is stopped;
		// cases service points here for it.
		active, err := m.Active()
		if err != nil {
			return "", err
		}
		if !active {
			return "", ErrNotInstalled
		}
		if err := m.run("systemctl", "--user", "stop", UnitName); err != nil {
			return "", err
		}
		return path, m.run("systemctl", "--user", "daemon-reload")
	}
	if err := m.run("systemctl", "--user", "disable", "--now", UnitName); err != nil {
		return "", err
	}
	if err := os.Remove(path); err != nil {
		return "", err
	}
	return path, m.run("systemctl", "--user", "daemon-reload")
}

func (m *Manager) domain() string { return fmt.Sprintf("gui/%d", m.UID) }
func (m *Manager) target() string { return m.domain() + "/" + Label }

// launchdNoService is the status `launchctl print` exits with for a label
// launchd does not know ("Could not find service").
const launchdNoService = 113

// loaded reports whether launchd has the agent. Only `launchctl print`
// exiting launchdNoService means it does not; any other failure, such as
// launchctl not running at all, is returned as an error.
func (m *Manager) loaded() (bool, error) {
	args := []string{"print", m.target()}
	out, err := m.Run("launchctl", args...)
	if err == nil {
		return true, nil
	}
	if exitCode(err) == launchdNoService {
		return false, nil
	}
	return false, commandError("launchctl", args, out, err)
}

// exitCode is the status a command that ran exited with, or -1 when err is
// not an exit status, as when the command could not be started.
func exitCode(err error) int {
	var e interface{ ExitCode() int }
	if errors.As(err, &e) {
		return e.ExitCode()
	}
	return -1
}

func (m *Manager) run(name string, args ...string) error {
	out, err := m.Run(name, args...)
	if err != nil {
		return commandError(name, args, out, err)
	}
	return nil
}

// commandError names the command that failed and adds its output.
func commandError(name string, args []string, out []byte, err error) error {
	msg := strings.TrimSpace(string(out))
	if msg != "" {
		return fmt.Errorf("%s %s: %w: %s", name, strings.Join(args, " "), err, msg)
	}
	return fmt.Errorf("%s %s: %w", name, strings.Join(args, " "), err)
}

// writeFile replaces path with body through a temporary file renamed into
// place, so the service manager never reads half a file.
func writeFile(path, body string) (err error) {
	f, err := os.CreateTemp(filepath.Dir(path), ".tmp-*")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer func() {
		if err != nil {
			_ = os.Remove(tmp)
		}
	}()
	if _, err = f.WriteString(body); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	if err = os.Chmod(tmp, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// Plist is the launchd agent for s. It is loaded at login (RunAtLoad) and
// started again whenever it exits (KeepAlive).
func Plist(s Spec) string {
	var b strings.Builder
	str := func(indent, v string) {
		b.WriteString(indent + "<string>")
		_ = xml.EscapeText(&b, []byte(v))
		b.WriteString("</string>\n")
	}
	key := func(indent, k string) {
		b.WriteString(indent + "<key>")
		_ = xml.EscapeText(&b, []byte(k))
		b.WriteString("</key>\n")
	}
	b.WriteString(`<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
`)
	key("\t", "Label")
	str("\t", Label)
	key("\t", "ProgramArguments")
	b.WriteString("\t<array>\n")
	for _, a := range s.Args {
		str("\t\t", a)
	}
	b.WriteString("\t</array>\n")
	if len(s.Env) > 0 {
		key("\t", "EnvironmentVariables")
		b.WriteString("\t<dict>\n")
		for _, kv := range s.Env {
			k, v, _ := strings.Cut(kv, "=")
			key("\t\t", k)
			str("\t\t", v)
		}
		b.WriteString("\t</dict>\n")
	}
	key("\t", "RunAtLoad")
	b.WriteString("\t<true/>\n")
	key("\t", "KeepAlive")
	b.WriteString("\t<true/>\n")
	if s.Log != "" {
		key("\t", "StandardOutPath")
		str("\t", s.Log)
		key("\t", "StandardErrorPath")
		str("\t", s.Log)
	}
	b.WriteString("</dict>\n</plist>\n")
	return b.String()
}

// Unit is the systemd user unit for s. Enabled, it starts with the user's
// session (default.target), and it is restarted whenever it exits.
func Unit(s Spec) string {
	args := make([]string, len(s.Args))
	for i, a := range s.Args {
		args[i] = unitQuote(a, true)
	}
	var b strings.Builder
	b.WriteString("[Unit]\nDescription=cases web inbox (cases serve)\n\n[Service]\n")
	b.WriteString("ExecStart=" + strings.Join(args, " ") + "\n")
	for _, kv := range s.Env {
		b.WriteString("Environment=" + unitQuote(kv, false) + "\n")
	}
	b.WriteString("Restart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n")
	return b.String()
}

// unitQuote double-quotes v for a unit file. % starts a specifier everywhere
// and $ a variable only in command lines (exec), so each is doubled where it
// is special.
func unitQuote(v string, exec bool) string {
	r := []string{`\`, `\\`, `"`, `\"`, "%", "%%", "\n", `\n`}
	if exec {
		r = append(r, "$", "$$")
	}
	return `"` + strings.NewReplacer(r...).Replace(v) + `"`
}

// Active reports whether the service manager has the service: loaded in
// launchd (`launchctl print`), or active in systemd
// (`systemctl --user is-active`), counting a unit waiting out RestartSec
// (activating) as active, as launchd counts a job it will restart. A query
// that fails without naming a state, as when the service manager cannot be
// reached, is an error, not an inactive service. It only reads.
func (m *Manager) Active() (bool, error) {
	switch m.GOOS {
	case "darwin":
		return m.loaded()
	case "linux":
		args := []string{"--user", "is-active", UnitName}
		out, err := m.Run("systemctl", args...)
		if err == nil {
			return true, nil
		}
		switch strings.TrimSpace(string(out)) {
		case "activating":
			return true, nil
		case "inactive", "failed", "deactivating", "maintenance", "unknown":
			return false, nil
		}
		return false, commandError("systemctl", args, out, err)
	}
	_, err := m.Path()
	return false, err
}

// Args reads back the program and arguments from the installed plist or
// unit file, as Install wrote them.
func (m *Manager) Args() ([]string, error) {
	path, err := m.Path()
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var args []string
	if m.GOOS == "darwin" {
		args, err = plistArgs(data)
	} else {
		args, err = unitArgs(string(data))
	}
	if err == nil && len(args) == 0 {
		err = errors.New("it names no program")
	}
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	return args, nil
}

// plistArgs is the ProgramArguments array of a plist.
func plistArgs(data []byte) ([]string, error) {
	dec := xml.NewDecoder(strings.NewReader(string(data)))
	var key string
	var args []string
	inArgs := false
	for {
		tok, err := dec.Token()
		if errors.Is(err, io.EOF) {
			return args, nil
		}
		if err != nil {
			return nil, err
		}
		switch t := tok.(type) {
		case xml.StartElement:
			switch t.Name.Local {
			case "key":
				if err := dec.DecodeElement(&key, &t); err != nil {
					return nil, err
				}
			case "array":
				inArgs = key == "ProgramArguments"
			case "string":
				var s string
				if err := dec.DecodeElement(&s, &t); err != nil {
					return nil, err
				}
				if inArgs {
					args = append(args, s)
				}
			}
		case xml.EndElement:
			if t.Name.Local == "array" {
				inArgs = false
			}
		}
	}
}

// unitArgs splits the ExecStart line of a unit written by Unit.
func unitArgs(unit string) ([]string, error) {
	for line := range strings.Lines(unit) {
		rest, ok := strings.CutPrefix(strings.TrimRight(line, "\n"), "ExecStart=")
		if !ok {
			continue
		}
		var args []string
		for rest = strings.TrimSpace(rest); rest != ""; rest = strings.TrimSpace(rest) {
			if rest[0] != '"' {
				return nil, fmt.Errorf("unquoted argument in ExecStart: %s", rest)
			}
			var b strings.Builder
			i := 1
			for ; i < len(rest) && rest[i] != '"'; i++ {
				c := rest[i]
				if (c == '\\' || c == '%' || c == '$') && i+1 < len(rest) {
					next := rest[i+1]
					switch {
					case c == '\\' && next == 'n':
						c = '\n'
						i++
					case c == '\\', next == c:
						c = next
						i++
					}
				}
				b.WriteByte(c)
			}
			if i >= len(rest) {
				return nil, errors.New("unterminated quote in ExecStart")
			}
			args = append(args, b.String())
			rest = rest[i+1:]
		}
		return args, nil
	}
	return nil, errors.New("no ExecStart line")
}
