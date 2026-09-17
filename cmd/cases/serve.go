package main

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"runtime"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/ryanlewis/cases/internal/instance"
	"github.com/ryanlewis/cases/internal/web"
)

type ServeCmd struct {
	Listen string `help:"Address to listen on. Only loopback addresses are allowed." default:"127.0.0.1:8765" placeholder:"HOST:PORT"`
	NoOpen bool   `help:"Do not open the inbox in the browser. It is only opened when stdout is a terminal."`
	As     string `help:"Record answers, parks and resumes from the inbox as written by NAME (the name config key)." placeholder:"NAME"`
}

func (c *ServeCmd) Run(d *Deps) error {
	if err := web.CheckLoopback(c.Listen); err != nil {
		return err
	}
	storeDir, err := filepath.Abs(d.Store)
	if err != nil {
		return err
	}
	// A file that cannot be read is treated as stale and replaced below.
	if running, _ := instance.Running(storeDir); running != nil {
		return fmt.Errorf("cases serve is already running for %s at %s (pid %d)", running.Store, running.URL, running.PID)
	}
	ln, err := net.Listen("tcp", c.Listen)
	if err != nil {
		return err
	}
	// Keep the host as given (localhost stays localhost) with the bound port,
	// which differs from the flag when it asked for port 0.
	host, _, _ := net.SplitHostPort(c.Listen)
	_, port, _ := net.SplitHostPort(ln.Addr().String())
	addr := net.JoinHostPort(host, port)
	url := "http://" + addr + "/"

	// On a terminal the status screen owns stdout, and the request log is
	// shown on it rather than scrolling it away.
	term := isTerminal(d.Stdout)
	var screen *statusScreen
	logw := d.Stderr
	if term {
		screen = newStatusScreen(d.Store, d.cases(), url, d.Stdout, os.Getenv("NO_COLOR") == "")
		logw = screen.logWriter(d.Stderr, !isTerminal(d.Stderr))
	}

	// Record the instance for `cases status`. Serving goes ahead without it.
	pid := os.Getpid()
	if err := instance.Write(instance.Info{PID: pid, URL: url, Addr: addr, Store: storeDir, StartedAt: time.Now().UTC().Truncate(time.Second), Version: version}); err != nil {
		fmt.Fprintf(logw, "could not record the instance for cases status: %v\n", err)
	} else {
		defer func() { _ = instance.Remove(storeDir, pid) }()
	}

	srv, err := web.New(d.cases(), addr, logw)
	if err != nil {
		_ = ln.Close()
		return err
	}
	srv.Actor = humanActor(c.As)
	if screen != nil {
		screen.notified = srv.Notified
	}
	ctx := d.Context
	if ctx == nil {
		var stop context.CancelFunc
		ctx, stop = signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
		defer stop()
		// After the first signal, restore the default so a second one kills
		// the process instead of waiting out a slow shutdown.
		context.AfterFunc(ctx, stop)
	}
	ctx, quit := context.WithCancel(ctx)
	defer quit()

	var requests atomic.Int64
	handler := srv.Handler()
	counted := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Leave out the refreshes and notification checks an open tab makes.
		if !web.IsPoll(r) {
			requests.Add(1)
		}
		handler.ServeHTTP(w, r)
	})

	if !term {
		fmt.Fprintf(d.Stdout, "Serving %s at %s\n", d.Store, url)
	}
	if term && !c.NoOpen && d.OpenURL != nil {
		if err := d.OpenURL(url); err != nil {
			fmt.Fprintf(logw, "could not open the browser: %v\n", err)
		}
	}
	if screen != nil {
		restore := screen.run(ctx, quit, d.Stdin, &requests)
		defer restore()
	}
	polled := make(chan struct{})
	go func() {
		defer close(polled)
		pollForNotifications(ctx, srv, notifyPoll)
	}()
	// Serve returns before ctx ends when the listener fails; stop the poll
	// and wait for it either way.
	defer func() {
		quit()
		<-polled
	}()
	return web.Serve(ctx, ln, counted)
}

// notifyPoll is how often serve reads the store for notifications when no tab
// is asking.
const notifyPoll = 2 * time.Second

// pollForNotifications reads the store through the server every interval
// until ctx ends, so cases that land on the human are queued for the browser
// while no tab is open. A failed read is left to the requests and the status
// screen to report.
func pollForNotifications(ctx context.Context, srv *web.Server, interval time.Duration) {
	tick := time.NewTicker(interval)
	defer tick.Stop()
	for {
		_ = srv.Poll()
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
		}
	}
}

// openBrowser opens url with the desktop's handler and does not wait for it.
func openBrowser(url string) error {
	name := "xdg-open"
	if runtime.GOOS == "darwin" {
		name = "open"
	}
	cmd := exec.Command(name, url)
	if err := cmd.Start(); err != nil {
		return err
	}
	go func() { _ = cmd.Wait() }()
	return nil
}

// isTerminal reports whether w is a terminal: a character device that also
// answers a terminal query, which /dev/null does not.
func isTerminal(w any) bool {
	f, ok := w.(*os.File)
	if !ok {
		return false
	}
	info, err := f.Stat()
	return err == nil && info.Mode()&os.ModeCharDevice != 0 && hasTermios(f)
}
