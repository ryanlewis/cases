package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/alecthomas/kong"

	"github.com/ryanlewis/cases/internal/config"
	"github.com/ryanlewis/cases/internal/store"
)

var (
	version = "dev"
	commit  = "none"
	date    = "unknown"
)

// exitTimeout is the status `cases wait` exits with when --timeout passes with
// nothing for the agent. It differs from 1, which every other failure uses.
const exitTimeout = 2

type CLI struct {
	Store   string           `help:"Case store directory (default ${default})." env:"CASES_STORE" default:"${store}" placeholder:"DIR"`
	Config  string           `help:"TOML config file that supplies flag defaults (default ${config})." placeholder:"PATH"`
	Version kong.VersionFlag `help:"Print version and exit." short:"v"`

	Open     OpenCmd     `cmd:"" help:"Open a case (agent)."`
	List     ListCmd     `cmd:"" help:"List cases."`
	Show     ShowCmd     `cmd:"" help:"Show one case and its thread."`
	Wait     WaitCmd     `cmd:"" help:"Block until a human answers, parks or resumes a case, then print the cases waiting on the agent as JSON lines (agent)."`
	Pickup   PickupCmd   `cmd:"" help:"Record that the answer has been read (agent)."`
	Note     NoteCmd     `cmd:"" help:"Add a follow-up to the thread; reopens an answered case (agent)."`
	Close    CloseCmd    `cmd:"" help:"Record the outcome of a picked-up case (agent)."`
	Withdraw WithdrawCmd `cmd:"" help:"Withdraw an open case that is no longer needed (agent)."`
	Answer   AnswerCmd   `cmd:"" help:"Answer an open case (human)."`
	Resume   ResumeCmd   `cmd:"" help:"Reopen a parked case (human, or agent with --agent)."`
	Serve    ServeCmd    `cmd:"" help:"Serve the local web inbox on a loopback address."`
	Status   StatusCmd   `cmd:"" help:"Print where cases serve is running for the store; exits 1 when it is not."`
	Conf     ConfigCmd   `cmd:"" name:"config" help:"Inspect and create the config file that supplies flag defaults."`
	Skill    SkillCmd    `cmd:"" help:"Install, show and list the bundled agent skill."`
}

// AfterApply settles the store path: an empty value (CASES_STORE set but
// empty) falls back to the default, and a leading ~ is expanded so the path
// can be written that way in the config file.
func (c *CLI) AfterApply(vars kong.Vars) error {
	c.Store = strings.TrimSpace(c.Store)
	if c.Store == "" {
		c.Store = vars["store"]
	}
	if c.Store == "" {
		return errors.New("--store or CASES_STORE must name the case store directory")
	}
	c.Store = config.ExpandHome(c.Store)
	return nil
}

// Deps carries what every command needs, so tests can swap the streams.
type Deps struct {
	Store  string
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer
	// Poll is how often `wait` checks the store.
	Poll time.Duration
	// Config is the config file that seeded the flag defaults, for the
	// config commands.
	Config *config.File
	// Context ends serve. When nil, serve stops on SIGINT or SIGTERM. Signals
	// are caught only inside serve, so every other command keeps the default
	// behaviour of exiting on them.
	Context context.Context
	// OpenURL opens the inbox in a browser. When nil, serve opens nothing.
	OpenURL func(url string) error
}

// exitError ends the process with a specific status and no "Error:" line.
type exitError struct {
	code int
	msg  string
}

func (e *exitError) Error() string { return e.msg }

// newParser builds the kong parser. cfg seeds flag defaults; main and the
// test harness both build from here so they cannot drift.
func newParser(cli *CLI, cfg *config.File, opts ...kong.Option) (*kong.Kong, error) {
	configPath, _ := config.DefaultPath()
	return kong.New(cli, append([]kong.Option{
		kong.Name("cases"),
		kong.Description("Raise, answer, pick up and close cases in a file-per-event store."),
		kong.UsageOnError(),
		kong.Vars{
			"version": fmt.Sprintf("cases %s (commit %s, built %s)", version, commit, date),
			"store":   config.DefaultStore(),
			"config":  configPath,
		},
		kong.Resolvers(cfg.Resolver()),
	}, opts...)...)
}

func main() {
	var cli CLI
	// A file that could not be read supplies no defaults; the failure is
	// reported once the command is known, because `cases config` is how
	// you find out what is wrong with it.
	cfg, cfgErr := loadConfig(os.Args[1:])
	parser, err := newParser(&cli, cfg)
	if err != nil {
		panic(err)
	}
	ctx, err := parser.Parse(os.Args[1:])
	parser.FatalIfErrorf(err)
	if cfgErr != nil && !diagnosesConfig(ctx) {
		fmt.Fprintf(os.Stderr, "Error: %v\n", cfgErr)
		os.Exit(1)
	}

	deps := &Deps{Store: cli.Store, Stdin: os.Stdin, Stdout: os.Stdout, Stderr: os.Stderr, Poll: time.Second, Config: cfg, OpenURL: openBrowser}
	if err := ctx.Run(deps); err != nil {
		var ee *exitError
		if errors.As(err, &ee) {
			if ee.msg != "" {
				fmt.Fprintln(os.Stderr, ee.msg)
			}
			os.Exit(ee.code)
		}
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}

// caseDir resolves a case id to its directory in the store.
func (d *Deps) caseDir(id string) (string, error) {
	return store.CaseDir(d.Store, id)
}

// readText reads a flag's file argument; "-" means stdin.
func (d *Deps) readText(path string) (string, error) {
	if path == "-" {
		b, err := io.ReadAll(d.Stdin)
		return string(b), err
	}
	b, err := os.ReadFile(path)
	return string(b), err
}

// warn prints problems found while loading, without failing the command.
// When seen is not nil, a warning already in it is skipped and each one
// printed is added, so a command that reloads the store says each only once.
func (d *Deps) warn(c *store.Case, seen map[string]bool) {
	for _, p := range c.Problems {
		msg := c.ID + ": " + p
		if seen != nil {
			if seen[msg] {
				continue
			}
			seen[msg] = true
		}
		fmt.Fprintf(d.Stderr, "warning: %s\n", msg)
	}
}

// done is what every write command prints: the case id and its new state.
func (d *Deps) done(c *store.Case) error {
	fmt.Fprintf(d.Stdout, "%s %s\n", c.ID, c.State)
	return nil
}
