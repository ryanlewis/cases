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
	case EventAmend:
		var a AmendRecord
		_ = json.Unmarshal(ev.Data, &a)
		if a.Body != "" {
			add("replaced the body")
		}
		for _, o := range a.Options {
			add("added option: %s", o)
		}
		for _, r := range a.Rows {
			add("added row [%s] %s", r.ID, r.Label)
		}
		for _, l := range a.Links {
			add("added link: %s", l)
		}
		for _, l := range a.Labels {
			add("added label: %s", l)
		}
		if a.Context != "" {
			add("replaced the context: %s", a.Context)
		}
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
		case a.Text != "" && c.Kind == KindQuestion:
			add("reply: %s", a.Text)
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
	case EventWithdraw:
		var wd WithdrawRecord
		_ = json.Unmarshal(ev.Data, &wd)
		if wd.Reason != "" {
			add("reason: %s", wd.Reason)
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
