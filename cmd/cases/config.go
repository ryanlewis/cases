package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/alecthomas/kong"

	"github.com/ryanlewis/cases/internal/config"
)

// loadConfig finds the config file for this invocation and reads it. The
// file supplies the defaults kong parses against, so it is found before
// kong runs and --config is read straight off the argv. The File is never
// nil: alongside an error it names the file that failed, which is what the
// config commands report.
func loadConfig(args []string) (*config.File, error) {
	path, source, err := config.ResolvePath(configPathFromArgs(args))
	if err != nil {
		return &config.File{Source: config.SourceDefault, Err: err}, err
	}
	f, loadErr := config.Load(path)
	f.Source = source
	return f, loadErr
}

// configPathFromArgs returns the value of the last --config flag in args,
// or "". It reads the flag as kong would: both spellings, the last
// occurrence wins, and nothing after --.
func configPathFromArgs(args []string) string {
	path := ""
	for i := 0; i < len(args); i++ {
		a := args[i]
		switch {
		case a == "--":
			return path
		case a == "--config":
			if i+1 >= len(args) {
				return ""
			}
			i++
			path = args[i]
		case strings.HasPrefix(a, "--config="):
			path = strings.TrimPrefix(a, "--config=")
		}
	}
	return path
}

// diagnosesConfig reports whether the selected command is one that exists
// to tell the user about the config file. Those run even when the file
// could not be read; refusing would leave no way to find out why.
func diagnosesConfig(ctx *kong.Context) bool {
	if ctx == nil || ctx.Selected() == nil {
		return false
	}
	target := ctx.Selected().Target
	if !target.CanAddr() {
		return false
	}
	_, ok := target.Addr().Interface().(configDiagnostic)
	return ok
}

// configDiagnostic marks such a command.
type configDiagnostic interface{ diagnosesConfig() }

type ConfigCmd struct {
	Path ConfigPathCmd `cmd:"" help:"Print the config file in use and whether it exists."`
	Show ConfigShowCmd `cmd:"" help:"Print the defaults the environment and the config file establish."`
	Init ConfigInitCmd `cmd:"" help:"Write a commented config file template."`
}

type ConfigPathCmd struct{}

func (*ConfigPathCmd) diagnosesConfig() {}

func (c *ConfigPathCmd) Run(d *Deps) error {
	cfg := d.Config
	if cfg.Path == "" {
		return cfg.Err
	}
	fmt.Fprintf(d.Stdout, "%s (%s)\n", cfg.Path, existence(cfg))
	if cfg.Err != nil {
		fmt.Fprintf(d.Stderr, "warning: this file cannot be used: %v\n", cfg.Err)
	}
	return nil
}

type ConfigShowCmd struct{}

func (*ConfigShowCmd) diagnosesConfig() {}

func (c *ConfigShowCmd) Run(d *Deps) error {
	cfg := d.Config
	if cfg.Path != "" {
		fmt.Fprintf(d.Stdout, "config: %s (%s)\n", cfg.Path, existence(cfg))
	}
	if cfg.Err != nil {
		return cfg.Err
	}
	fmt.Fprintln(d.Stdout, "These apply when no flag overrides them.")
	fmt.Fprintln(d.Stdout)
	settings := cfg.Settings()
	keyW, valW := 0, 0
	for _, s := range settings {
		keyW = max(keyW, len(s.Key))
		valW = max(valW, len(s.Value))
	}
	for _, s := range settings {
		fmt.Fprintf(d.Stdout, "  %-*s  %-*s  %s\n", keyW, s.Key, valW, s.Value, s.Source)
	}
	return nil
}

func existence(cfg *config.File) string {
	if cfg.Exists {
		return "exists"
	}
	return "not found"
}

type ConfigInitCmd struct {
	Force bool `help:"Overwrite an existing config file." short:"f"`
}

func (*ConfigInitCmd) diagnosesConfig() {}

func (c *ConfigInitCmd) Run(d *Deps) error {
	cfg := d.Config
	if cfg.Path == "" {
		return cfg.Err
	}
	if cfg.Exists && !c.Force {
		return fmt.Errorf("config file already exists: %s (pass --force to overwrite)", cfg.Path)
	}
	if err := os.MkdirAll(filepath.Dir(cfg.Path), 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(cfg.Path, []byte(config.Template()), 0o644); err != nil {
		return err
	}
	fmt.Fprintf(d.Stdout, "Wrote config template to %s\n", cfg.Path)
	return nil
}
