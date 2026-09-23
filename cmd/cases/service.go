package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/ryanlewis/cases/internal/instance"
	"github.com/ryanlewis/cases/internal/service"
	"github.com/ryanlewis/cases/internal/web"
)

// ServiceCmd manages cases serve as a user service. Plain `cases service`
// runs the default subcommand, status, which only reads. status is listed
// rather than hidden: kong's command list shows only leaves.
type ServiceCmd struct {
	Status    ServiceStatusCmd    `cmd:"" default:"withargs" help:"Report whether the cases serve service is installed, loaded and answering; exits 1 unless all three. Plain cases service does the same."`
	Install   ServiceInstallCmd   `cmd:"" help:"Run cases serve as a user service that starts at login and restarts when it stops (launchd on macOS, systemd on Linux)."`
	Uninstall ServiceUninstallCmd `cmd:"" help:"Stop the cases serve user service and remove its file."`
}

type ServiceInstallCmd struct {
	Listen string `help:"Address the service's serve listens on. Only loopback addresses are allowed." default:"127.0.0.1:8765" placeholder:"HOST:PORT"`
	As     string `help:"Record answers, parks and resumes from the inbox as written by NAME (the name config key)." placeholder:"NAME"`
}

// Run writes the service with serve's flags baked in: the store's absolute
// path, the config file, --listen, --as and --no-open, since a service has
// no terminal. HOME and XDG_STATE_HOME are set so the service records itself
// where `cases status` looks.
func (c *ServiceInstallCmd) Run(d *Deps) error {
	m, err := d.serviceManager()
	if err != nil {
		return err
	}
	if err := web.CheckLoopback(c.Listen); err != nil {
		return err
	}
	if err := checkNoServe(d, m, c.Listen); err != nil {
		return err
	}
	spec, err := serveSpec(d, c.Listen, c.As)
	if err != nil {
		return err
	}
	path, err := m.Install(spec)
	if err != nil {
		return err
	}
	fmt.Fprintf(d.Stdout, "Installed the cases serve service: %s\n", path)
	fmt.Fprintf(d.Stdout, "It runs: %s\n", shellJoin(spec.Args))
	fmt.Fprintf(d.Stdout, "It starts at login and restarts when it stops. Rerun install after changing the store, config, --listen or --as.\n")
	fmt.Fprintf(d.Stdout, "Check it with: cases service\n")
	fmt.Fprintf(d.Stdout, "Service manager: %s\n", m.Check())
	if spec.Log != "" && m.GOOS == "darwin" {
		fmt.Fprintf(d.Stdout, "Log: %s\n", spec.Log)
	}
	if m.GOOS == "linux" {
		fmt.Fprintf(d.Stdout, "Log: journalctl --user -u %s\n", service.UnitName)
	}
	return nil
}

type ServiceUninstallCmd struct{}

func (ServiceUninstallCmd) Run(d *Deps) error {
	m, err := d.serviceManager()
	if err != nil {
		return err
	}
	path, err := m.Uninstall()
	if err != nil {
		return err
	}
	fmt.Fprintf(d.Stdout, "Stopped the cases serve service and removed %s\n", path)
	return nil
}

func (d *Deps) serviceManager() (*service.Manager, error) {
	if d.Service == nil {
		return nil, errors.New("no service manager")
	}
	return d.Service()
}

// checkNoServe refuses an install while a serve is running for the store, or
// something holds the address: the service would fail on every start and be
// restarted forever. A loaded service already on the same store or address
// is the serve found there, which install restarts, so that check is skipped.
func checkNoServe(d *Deps, m *service.Manager, listen string) error {
	storePath, err := filepath.Abs(d.Store)
	if err != nil {
		return err
	}
	var oldStore, oldListen string
	installed, err := m.Installed()
	if err != nil {
		return err
	}
	if installed {
		active, err := m.Active()
		if err != nil {
			return err
		}
		if active {
			if args, err := m.Args(); err == nil {
				oldStore, oldListen = flagValue(args, "--store"), flagValue(args, "--listen")
			}
		}
	}
	if oldStore != storePath {
		if running, _ := instance.Running(storePath); running != nil {
			return fmt.Errorf("cases serve is already running for %s at %s (pid %d); stop it before installing the service", running.Store, running.URL, running.PID)
		}
	}
	if oldListen == listen {
		return nil
	}
	ln, err := net.Listen("tcp", listen)
	if err != nil {
		return fmt.Errorf("%s is already in use (%v); stop what holds it, or pass another --listen, before installing the service", listen, err)
	}
	return ln.Close()
}

// serveSpec is the service for serve on d's store with listen and as.
func serveSpec(d *Deps, listen, as string) (service.Spec, error) {
	exe, err := executable()
	if err != nil {
		return service.Spec{}, err
	}
	storePath, err := filepath.Abs(d.Store)
	if err != nil {
		return service.Spec{}, err
	}
	home := os.Getenv("HOME")
	if home == "" {
		return service.Spec{}, errors.New("$HOME is not set")
	}
	state, err := instance.Dir()
	if err != nil {
		return service.Spec{}, err
	}
	// The service does not start in this directory, so a relative
	// $XDG_STATE_HOME would point it somewhere cases status does not look.
	if state, err = filepath.Abs(state); err != nil {
		return service.Spec{}, err
	}
	args := []string{exe, "--store", storePath}
	if d.Config != nil && d.Config.Path != "" {
		cfg, err := filepath.Abs(d.Config.Path)
		if err != nil {
			return service.Spec{}, err
		}
		args = append(args, "--config", cfg)
	}
	args = append(args, "serve", "--no-open", "--listen", listen)
	if strings.TrimSpace(as) != "" {
		args = append(args, "--as", as)
	}
	return service.Spec{
		Args: args,
		Env:  []string{"HOME=" + home, "XDG_STATE_HOME=" + filepath.Dir(state)},
		Log:  filepath.Join(state, "serve.log"),
	}, nil
}

// executable is this binary's path as it was run, when that names the same
// file as os.Executable. On Linux os.Executable resolves symlinks, and a
// package manager's bin/cases points at a versioned file that an upgrade
// removes; the service keeps the stable link instead.
func executable() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	if p, err := exec.LookPath(os.Args[0]); err == nil && sameFile(p, exe) {
		if abs, err := filepath.Abs(p); err == nil {
			return abs, nil
		}
	}
	return exe, nil
}

// sameFile reports whether a and b both exist and are the same file.
func sameFile(a, b string) bool {
	fa, errA := os.Stat(a)
	fb, errB := os.Stat(b)
	return errA == nil && errB == nil && os.SameFile(fa, fb)
}

// shellJoin joins args for display, single-quoting any a shell would split
// or expand, so the line can be read and pasted as the service runs it.
func shellJoin(args []string) string {
	out := make([]string, len(args))
	for i, a := range args {
		if a == "" || strings.ContainsAny(a, " \t\n'\"\\$`;&|<>()*?[]#~!{}") {
			a = "'" + strings.ReplaceAll(a, "'", `'\''`) + "'"
		}
		out[i] = a
	}
	return strings.Join(out, " ")
}

type ServiceStatusCmd struct {
	JSON bool `help:"Print JSON." short:"j"`
}

// serviceReport is what `cases service` finds, and its --json.
type serviceReport struct {
	Installed bool     `json:"installed"`
	Path      string   `json:"path"`
	Args      []string `json:"args,omitempty"`
	Store     string   `json:"store,omitempty"`
	Listen    string   `json:"listen,omitempty"`
	Loaded    bool     `json:"loaded"`
	Running   bool     `json:"running"`
	URL       string   `json:"url,omitempty"`
	PID       int      `json:"pid,omitempty"`
	Problems  []string `json:"problems,omitempty"`
}

// Run reports the service's file, whether the service manager has it and
// whether its inbox answers, and names any disagreement between them. It
// changes nothing. It exits 1 unless the service is installed, loaded and
// answering.
func (c *ServiceStatusCmd) Run(d *Deps) error {
	m, err := d.serviceManager()
	if err != nil {
		return err
	}
	r, err := serviceState(d, m)
	if err != nil {
		return err
	}
	if c.JSON {
		out, err := json.MarshalIndent(r, "", "  ")
		if err != nil {
			return err
		}
		fmt.Fprintln(d.Stdout, string(out))
	} else {
		printServiceReport(d, m, r)
	}
	if r.Installed && r.Loaded && r.Running {
		return nil
	}
	return &exitError{code: 1}
}

func serviceState(d *Deps, m *service.Manager) (serviceReport, error) {
	var r serviceReport
	var err error
	if r.Path, err = m.Path(); err != nil {
		return r, err
	}
	if r.Installed, err = m.Installed(); err != nil {
		return r, err
	}
	if r.Loaded, err = m.Active(); err != nil {
		return r, err
	}
	manager := "launchd"
	if m.GOOS == "linux" {
		manager = "systemd"
	}
	storePath, err := filepath.Abs(d.Store)
	if err != nil {
		return r, err
	}
	// The inbox to look for is the installed service's; this command's
	// store stands in when there is none.
	r.Store = storePath
	if r.Installed {
		args, err := m.Args()
		if err != nil {
			r.Problems = append(r.Problems, err.Error())
		} else {
			r.Args = args
			r.Store, r.Listen = flagValue(args, "--store"), flagValue(args, "--listen")
			if exe, err := executable(); err == nil && !sameFile(args[0], exe) {
				r.Problems = append(r.Problems, fmt.Sprintf("installed for another binary: %s (this is %s)", args[0], exe))
			}
			if r.Store != storePath {
				r.Problems = append(r.Problems, fmt.Sprintf("installed for another store: %s (this command uses %s)", r.Store, storePath))
			}
		}
	}
	running, err := instance.Running(r.Store)
	if err != nil {
		r.Problems = append(r.Problems, err.Error())
	}
	if running != nil {
		r.Running, r.URL, r.PID = true, running.URL, running.PID
	}
	switch {
	case r.Installed && !r.Loaded:
		r.Problems = append(r.Problems, "installed but not loaded in "+manager+"; cases service install loads it again")
	case !r.Installed && r.Loaded:
		r.Problems = append(r.Problems, "loaded in "+manager+" but its file is missing; cases service uninstall stops it")
	case r.Installed && !r.Running:
		r.Problems = append(r.Problems, "loaded but the inbox does not answer; see "+m.Check())
	case !r.Installed && r.Running:
		r.Problems = append(r.Problems, fmt.Sprintf("not installed, but a cases serve started by hand is running (pid %d); stop it before cases service install", r.PID))
	}
	return r, nil
}

func printServiceReport(d *Deps, m *service.Manager, r serviceReport) {
	yes := map[bool]string{true: "yes", false: "no"}
	if r.Installed {
		fmt.Fprintf(d.Stdout, "Installed: %s\n", r.Path)
	} else {
		fmt.Fprintf(d.Stdout, "Installed: no (%s)\n", r.Path)
	}
	if r.Args != nil {
		fmt.Fprintf(d.Stdout, "Runs:      %s\n", shellJoin(r.Args))
		fmt.Fprintf(d.Stdout, "Store:     %s\n", r.Store)
		fmt.Fprintf(d.Stdout, "Listen:    %s\n", r.Listen)
	}
	fmt.Fprintf(d.Stdout, "Loaded:    %s (%s)\n", yes[r.Loaded], m.Check())
	if r.Running {
		fmt.Fprintf(d.Stdout, "Inbox:     %s (pid %d)\n", r.URL, r.PID)
	} else {
		fmt.Fprintf(d.Stdout, "Inbox:     not running\n")
	}
	for _, p := range r.Problems {
		fmt.Fprintf(d.Stdout, "Problem:   %s\n", p)
	}
}

// flagValue is the value after flag in args, or "".
func flagValue(args []string, flag string) string {
	for i, a := range args[:len(args)-1] {
		if a == flag {
			return args[i+1]
		}
	}
	return ""
}
