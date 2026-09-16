package store

import (
	"encoding/json"
	"fmt"
	"strings"
)

// Line is one line of what an event said. When an amend replaced the body or
// the context, the text the case had before is in PreviousBody or
// PreviousContext on the last line about that change. A context with line
// breaks takes several lines, and the old text is on the last of them. There
// is no old text when it was empty or only space.
type Line struct {
	Text            string
	PreviousBody    string
	PreviousContext string
}

// Describe summarises what an event said, as plain lines for a thread view.
// The fold has already validated the data, so decode errors are not expected.
func (c *Case) Describe(ev Event) []Line {
	switch ev.Type {
	case EventAmend:
		return describeAmend(ev)
	case EventAnswer:
		return c.describeAnswer(ev)
	case EventPickup:
		return describePickup(ev)
	case EventNote:
		return describeNote(ev)
	case EventClose:
		return describeClose(ev)
	case EventWithdraw:
		return describeWithdraw(ev)
	case EventPark:
		return describePark(ev)
	}
	return nil
}

// addLines appends the formatted text to lines, trimmed and one Line per line of it.
func addLines(lines []Line, format string, args ...any) []Line {
	for line := range strings.SplitSeq(strings.TrimSpace(fmt.Sprintf(format, args...)), "\n") {
		lines = append(lines, Line{Text: line})
	}
	return lines
}

func describeAmend(ev Event) []Line {
	var a AmendRecord
	_ = json.Unmarshal(ev.Data, &a)
	var lines []Line
	// An amend that sets the body or context the case already has changed
	// nothing there, so no line says it replaced it.
	if a.Body != "" && a.Body != ev.replacedBody {
		lines = addLines(lines, "replaced the body")
		if strings.TrimSpace(ev.replacedBody) != "" {
			lines[len(lines)-1].PreviousBody = ev.replacedBody
		}
	}
	for _, o := range a.Options {
		lines = addLines(lines, "added option: %s", o)
	}
	for _, r := range a.Rows {
		lines = addLines(lines, "added row [%s] %s", r.ID, r.Label)
	}
	for _, l := range a.Links {
		lines = addLines(lines, "added link: %s", l)
	}
	for _, l := range a.Labels {
		lines = addLines(lines, "added label: %s", l)
	}
	if a.Context != "" && a.Context != ev.replacedContext {
		lines = addLines(lines, "replaced the context: %s", a.Context)
		if strings.TrimSpace(ev.replacedContext) != "" {
			lines[len(lines)-1].PreviousContext = ev.replacedContext
		}
	}
	return lines
}

func (c *Case) describeAnswer(ev Event) []Line {
	var a AnswerRecord
	_ = json.Unmarshal(ev.Data, &a)
	var lines []Line
	switch {
	case a.Choice > 0 && a.Choice <= len(c.Options):
		lines = addLines(lines, "chose %d. %s", a.Choice, c.Options[a.Choice-1])
	case a.Other:
		lines = addLines(lines, "chose other")
	case a.Signoff != "":
		lines = addLines(lines, "signoff: %s", a.Signoff)
	case a.Text != "" && c.Kind == KindQuestion:
		lines = addLines(lines, "reply: %s", a.Text)
	case a.Text != "":
		lines = addLines(lines, "guidance: %s", a.Text)
	case a.Drop:
		lines = addLines(lines, "drop")
	case a.Ack:
		lines = addLines(lines, "acknowledged")
	}
	for _, r := range a.Rows {
		if r.Note != "" {
			lines = addLines(lines, "[%s] %s: %s", r.ID, r.Verdict, r.Note)
		} else {
			lines = addLines(lines, "[%s] %s", r.ID, r.Verdict)
		}
	}
	if a.Note != "" {
		lines = addLines(lines, "note: %s", a.Note)
	}
	return lines
}

func describePickup(ev Event) []Line {
	var p PickupRecord
	_ = json.Unmarshal(ev.Data, &p)
	if p.By == "" {
		return nil
	}
	return addLines(nil, "by %s", p.By)
}

func describeNote(ev Event) []Line {
	var n NoteRecord
	_ = json.Unmarshal(ev.Data, &n)
	return addLines(nil, "%s", n.Body)
}

func describeClose(ev Event) []Line {
	var cl CloseRecord
	_ = json.Unmarshal(ev.Data, &cl)
	lines := addLines(nil, "%s", cl.Outcome)
	for _, l := range cl.Links {
		lines = addLines(lines, "%s", l)
	}
	return lines
}

func describeWithdraw(ev Event) []Line {
	var wd WithdrawRecord
	_ = json.Unmarshal(ev.Data, &wd)
	if wd.Reason == "" {
		return nil
	}
	return addLines(nil, "reason: %s", wd.Reason)
}

func describePark(ev Event) []Line {
	var p ParkRecord
	_ = json.Unmarshal(ev.Data, &p)
	if p.Note == "" {
		return nil
	}
	return addLines(nil, "note: %s", p.Note)
}
