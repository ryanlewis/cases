package main

import (
	"encoding/json"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type ShowCmd struct {
	ID   string `arg:"" help:"Case id, or any part of it that names one case."`
	JSON bool   `help:"Print JSON, including every event file as written." short:"j"`
}

func (c *ShowCmd) Run(d *Deps) error {
	dir, err := d.findCase(c.ID)
	if err != nil {
		return err
	}
	cs, err := store.Load(dir)
	if err != nil {
		return err
	}
	d.warn(cs, nil)
	if c.JSON {
		enc := json.NewEncoder(d.Stdout)
		enc.SetIndent("", "  ")
		enc.SetEscapeHTML(false)
		return enc.Encode(shownCase{cs, cs.Revision()})
	}
	printCase(d.Stdout, cs)
	return nil
}

// shownCase is the case as show --json prints it: the case's own JSON with its
// revision beside it, which answer and resume take as --revision.
type shownCase struct {
	*store.Case
	Revision int `json:"revision"`
}

func printCase(w io.Writer, c *store.Case) {
	fmt.Fprintf(w, "%s\n\n", c.Title)
	field := func(name, value string) {
		if value != "" {
			fmt.Fprintf(w, "%-9s %s\n", name+":", value)
		}
	}
	field("id", c.ID)
	field("state", string(c.State))
	field("kind", string(c.Kind))
	field("urgency", string(c.Urgency))
	field("opened", stamp(c.OpenedAt))
	field("revision", strconv.Itoa(c.Revision()))
	field("labels", strings.Join(c.Labels, ", "))
	field("worker", c.Worker)
	field("brief", c.Brief)
	field("context", c.Context)

	if body := strings.TrimSpace(c.Body); body != "" {
		fmt.Fprintf(w, "\n%s\n", body)
	}
	if len(c.Options) > 0 {
		fmt.Fprintln(w, "\nOptions:")
		for i, o := range c.Options {
			fmt.Fprintf(w, "  %d. %s\n", i+1, o)
		}
		fmt.Fprintln(w, "  other. Other, see note")
	}
	if len(c.Rows) > 0 {
		fmt.Fprintln(w, "\nRows:")
		for _, r := range c.Rows {
			fmt.Fprintf(w, "  [%s] %s\n", r.ID, r.Label)
			if r.Note != "" {
				fmt.Fprintf(w, "      note: %s\n", r.Note)
			}
			fmt.Fprintf(w, "      %s\n", r.Link)
			quote(w, "      ", strings.TrimRight(r.Script, "\n"))
		}
	}
	if len(c.Links) > 0 {
		fmt.Fprintln(w, "\nLinks:")
		for _, l := range c.Links {
			fmt.Fprintf(w, "  %s\n", l)
		}
	}

	fmt.Fprintln(w, "\nThread:")
	for _, ev := range c.Events {
		fmt.Fprintf(w, "  %04d %-5s %-8s %s\n", ev.Seq, ev.Author, ev.Type, stamp(ev.At))
		for _, line := range c.Describe(ev) {
			fmt.Fprintf(w, "       %s\n", line.Text)
			printPrevious(w, "body", line.PreviousBody)
			printPrevious(w, "context", line.PreviousContext)
		}
	}
	for _, p := range c.Problems {
		fmt.Fprintf(w, "  problem: %s\n", p)
	}
}

// printPrevious prints, in full, the body or context an amend replaced, under
// the thread line that says so.
func printPrevious(w io.Writer, field, text string) {
	if text == "" {
		return
	}
	fmt.Fprintf(w, "         previous %s:\n", field)
	quote(w, "         ", strings.TrimSpace(text))
}

// quote prints each line of text after indent and "| ".
func quote(w io.Writer, indent, text string) {
	for line := range strings.SplitSeq(text, "\n") {
		fmt.Fprintf(w, "%s| %s\n", indent, line)
	}
}

func stamp(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339)
}
