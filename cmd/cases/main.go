package main

import (
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/alecthomas/kong"

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
	Store   string           `help:"Case store directory. Required; there is no default." env:"CASES_STORE" required:"" placeholder:"DIR"`
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
}

// Validate refuses an empty store, which kong's required check lets through
// when CASES_STORE is set but empty.
func (c *CLI) Validate() error {
	if strings.TrimSpace(c.Store) == "" {
		return errors.New("--store or CASES_STORE must name the case store directory")
	}
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
}

// exitError ends the process with a specific status and no "Error:" line.
type exitError struct {
	code int
	msg  string
}

func (e *exitError) Error() string { return e.msg }

func newParser(cli *CLI, opts ...kong.Option) (*kong.Kong, error) {
	return kong.New(cli, append([]kong.Option{
		kong.Name("cases"),
		kong.Description("Raise, answer, pick up and close cases in a file-per-event store."),
		kong.UsageOnError(),
		kong.Vars{"version": fmt.Sprintf("cases %s (commit %s, built %s)", version, commit, date)},
	}, opts...)...)
}

func main() {
	var cli CLI
	parser, err := newParser(&cli)
	if err != nil {
		panic(err)
	}
	ctx, err := parser.Parse(os.Args[1:])
	parser.FatalIfErrorf(err)

	deps := &Deps{Store: cli.Store, Stdin: os.Stdin, Stdout: os.Stdout, Stderr: os.Stderr, Poll: time.Second}
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
func (d *Deps) warn(c *store.Case) {
	for _, p := range c.Problems {
		fmt.Fprintf(d.Stderr, "warning: %s: %s\n", c.ID, p)
	}
}

// done is what every write command prints: the case id and its new state.
func (d *Deps) done(c *store.Case) error {
	fmt.Fprintf(d.Stdout, "%s %s\n", c.ID, c.State)
	return nil
}
