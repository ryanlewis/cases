package main

import (
	"context"
	"fmt"
	"net"
	"os"
	"os/signal"
	"syscall"

	"github.com/ryanlewis/cases/internal/web"
)

type ServeCmd struct {
	Listen string `help:"Address to listen on. Only loopback addresses are allowed." default:"127.0.0.1:8765" placeholder:"HOST:PORT"`
}

func (c *ServeCmd) Run(d *Deps) error {
	if err := web.CheckLoopback(c.Listen); err != nil {
		return err
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

	srv, err := web.New(d.Store, addr, d.Stderr)
	if err != nil {
		_ = ln.Close()
		return err
	}
	fmt.Fprintf(d.Stdout, "Serving %s at http://%s/\n", d.Store, addr)
	ctx := d.Context
	if ctx == nil {
		var stop context.CancelFunc
		ctx, stop = signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
		defer stop()
		// After the first signal, restore the default so a second one kills
		// the process instead of waiting out a slow shutdown.
		context.AfterFunc(ctx, stop)
	}
	return web.Serve(ctx, ln, srv.Handler())
}
