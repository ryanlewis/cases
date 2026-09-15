package main

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/ryanlewis/cases/internal/store"
)

type OpenCmd struct {
	Kind     string   `help:"decision, approval, signoff, stuck or fyi." required:"" enum:"decision,approval,signoff,stuck,fyi" placeholder:"KIND"`
	Urgency  string   `help:"blocking, today or whenever." required:"" enum:"blocking,today,whenever" placeholder:"URGENCY"`
	Title    string   `help:"One-line title; also names the case directory." required:""`
	BodyFile string   `help:"Markdown body. - reads stdin." name:"body-file" placeholder:"FILE"`
	Option   []string `help:"An option for a decision case. Repeat per option; \"Other, see note\" is always offered." sep:"none" placeholder:"TEXT"`
	Row      []string `help:"A row for an approval case, as a JSON object with id, label, script and link, e.g. '{\"id\":\"deps\",\"label\":\"Install deps\",\"script\":\"npm ci\",\"link\":\"https://…\"}'. Repeat per row." sep:"none" placeholder:"JSON"`
	Link     []string `help:"A link to show with the case. Repeatable." sep:"none" placeholder:"URL"`
	Worker   string   `help:"The worker waiting on this case."`
	Brief    string   `help:"Path to the worker's brief, for re-briefing after parking."`
	Context  string   `help:"Free-text context for the manager."`
}

func (c *OpenCmd) Run(d *Deps) error {
	rec := store.OpenRecord{
		Kind:    store.Kind(c.Kind),
		Urgency: store.Urgency(c.Urgency),
		Title:   c.Title,
		Options: c.Option,
		Links:   c.Link,
		Worker:  c.Worker,
		Brief:   c.Brief,
		Context: c.Context,
	}
	if c.BodyFile != "" {
		body, err := d.readText(c.BodyFile)
		if err != nil {
			return fmt.Errorf("body: %w", err)
		}
		rec.Body = body
	}
	for i, raw := range c.Row {
		var row store.Row
		dec := json.NewDecoder(strings.NewReader(raw))
		dec.DisallowUnknownFields()
		if err := dec.Decode(&row); err != nil {
			return fmt.Errorf("--row %d: %w", i+1, err)
		}
		rec.Rows = append(rec.Rows, row)
	}
	created, err := store.Create(d.Store, rec)
	if err != nil {
		return err
	}
	fmt.Fprintln(d.Stdout, created.ID)
	return nil
}
