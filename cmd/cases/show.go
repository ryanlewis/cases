package main

import (
	"encoding/json"
	"fmt"
	"io"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

type ShowCmd struct {
	ID   string `arg:"" help:"Case id."`
	JSON bool   `help:"Print JSON, including every event file as written." short:"j"`
}

func (c *ShowCmd) Run(d *Deps) error {
	dir, err := d.caseDir(c.ID)
	if err != nil {
		return err
	}
	cs, err := store.Load(dir)
	if err != nil {
		return err
	}
	d.warn(cs)
	if c.JSON {
		enc := json.NewEncoder(d.Stdout)
		enc.SetIndent("", "  ")
		enc.SetEscapeHTML(false)
		return enc.Encode(cs)
	}
	printCase(d.Stdout, cs)
	return nil
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
			fmt.Fprintf(w, "  [%s] %s\n      %s\n", r.ID, r.Label, r.Link)
			for line := range strings.SplitSeq(strings.TrimRight(r.Script, "\n"), "\n") {
				fmt.Fprintf(w, "      | %s\n", line)
			}
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
		for _, line := range eventLines(c, ev) {
			fmt.Fprintf(w, "       %s\n", line)
		}
	}
	for _, p := range c.Problems {
		fmt.Fprintf(w, "  problem: %s\n", p)
	}
}

// eventLines summarises what an event said, for the thread view. The fold has
// already validated the data, so decode errors are not expected here.
func eventLines(c *store.Case, ev store.Event) []string {
	var lines []string
	add := func(format string, args ...any) {
		for line := range strings.SplitSeq(strings.TrimSpace(fmt.Sprintf(format, args...)), "\n") {
			lines = append(lines, line)
		}
	}
	switch ev.Type {
	case store.EventAnswer:
		var a store.AnswerRecord
		_ = json.Unmarshal(ev.Data, &a)
		switch {
		case a.Choice > 0 && a.Choice <= len(c.Options):
			add("chose %d. %s", a.Choice, c.Options[a.Choice-1])
		case a.Other:
			add("chose other")
		case a.Signoff != "":
			add("signoff: %s", a.Signoff)
		case a.Text != "":
			add("guidance: %s", a.Text)
		case a.Drop:
			add("drop")
		case a.Ack:
			add("acknowledged")
		}
		for _, r := range a.Rows {
			if r.Note != "" {
				add("[%s] %s: %s", r.ID, r.Verdict, r.Note)
			} else {
				add("[%s] %s", r.ID, r.Verdict)
			}
		}
		if a.Note != "" {
			add("note: %s", a.Note)
		}
	case store.EventPickup:
		var p store.PickupRecord
		_ = json.Unmarshal(ev.Data, &p)
		if p.By != "" {
			add("by %s", p.By)
		}
	case store.EventNote:
		var n store.NoteRecord
		_ = json.Unmarshal(ev.Data, &n)
		add("%s", n.Body)
	case store.EventClose:
		var cl store.CloseRecord
		_ = json.Unmarshal(ev.Data, &cl)
		add("%s", cl.Outcome)
		for _, l := range cl.Links {
			add("%s", l)
		}
	case store.EventPark:
		var p store.ParkRecord
		_ = json.Unmarshal(ev.Data, &p)
		if p.Note != "" {
			add("note: %s", p.Note)
		}
	}
	return lines
}

func stamp(t time.Time) string {
	if t.IsZero() {
		return ""
	}
	return t.UTC().Format(time.RFC3339)
}
