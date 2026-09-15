package store

import (
	"encoding/json"
	"fmt"
	"strings"
)

// Describe summarises what an event said, as plain lines for a thread view.
// The fold has already validated the data, so decode errors are not expected.
func (c *Case) Describe(ev Event) []string {
	var lines []string
	add := func(format string, args ...any) {
		for line := range strings.SplitSeq(strings.TrimSpace(fmt.Sprintf(format, args...)), "\n") {
			lines = append(lines, line)
		}
	}
	switch ev.Type {
	case EventAnswer:
		var a AnswerRecord
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
	case EventPickup:
		var p PickupRecord
		_ = json.Unmarshal(ev.Data, &p)
		if p.By != "" {
			add("by %s", p.By)
		}
	case EventNote:
		var n NoteRecord
		_ = json.Unmarshal(ev.Data, &n)
		add("%s", n.Body)
	case EventClose:
		var cl CloseRecord
		_ = json.Unmarshal(ev.Data, &cl)
		add("%s", cl.Outcome)
		for _, l := range cl.Links {
			add("%s", l)
		}
	case EventPark:
		var p ParkRecord
		_ = json.Unmarshal(ev.Data, &p)
		if p.Note != "" {
			add("note: %s", p.Note)
		}
	}
	return lines
}
