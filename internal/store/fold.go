package store

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"time"
)

// Event is one event as read from the store.
type Event struct {
	Seq    int       `json:"seq"`
	Author Author    `json:"author"`
	Type   EventType `json:"event"`
	// File is the name the event had as a file, NNNN-<author>-<event>.json.
	// It is unique within the case, and wait and the notifier key on it.
	File string    `json:"file"`
	At   time.Time `json:"at"`
	// Actor is who the event says wrote it, or nil when it does not say.
	Actor *Actor `json:"actor,omitempty"`
	// Data is the record exactly as written, so fields this version does not
	// know about survive a read.
	Data json.RawMessage `json:"data"`

	// replacedBody and replacedContext are the body and context an amend
	// replaced, as the fold found them, for Describe.
	replacedBody, replacedContext string
	// from is the state the case was in before the event, as the fold found
	// it. It is empty for the open event.
	from State
}

// From is the state the case was in before the event. It tells a note that
// reopened an answered case from a note on an open one. It is empty for the
// open event and for an event that did not come from a fold.
func (ev Event) From() State { return ev.from }

// Case is the fold of a case's events.
type Case struct {
	ID    string `json:"id"`
	State State  `json:"state"`
	OpenRecord
	UpdatedAt time.Time `json:"updated_at"`

	// Answer is the answer the case currently stands on. A follow-up note
	// reopens the case and clears it; the earlier answer stays in Events.
	Answer *AnswerRecord `json:"answer,omitempty"`
	Pickup *PickupRecord `json:"pickup,omitempty"`
	Park   *ParkRecord   `json:"park,omitempty"`
	Close  *CloseRecord  `json:"close,omitempty"`

	// Events are the events that folded cleanly, in order.
	Events []Event `json:"events"`
	// Problems name events that were skipped: malformed, or an event the
	// case was not in a state to accept, such as one written by a newer
	// cases.
	Problems []string `json:"problems,omitempty"`

	lastSeq int
	events  int
	// amendSeq is the sequence number of the last amend a later answer must
	// have seen. Every amend counts except one that only adds labels: an
	// answer is never checked against labels, and they do not change the
	// question. Options and rows are what an answer is checked against. A
	// body, context or link can change the question, such as the PR a signoff
	// accepts, and every build with amend has counted links, so exempting
	// them would change how existing stores fold. A body or context counts
	// whenever it is set, even to the text the case already has. (An answer
	// written with AtRevision is still refused, as for any new event.)
	amendSeq int
}

// Revision is the number of events stored for the case, counting any the
// fold skipped. Every event written raises it. A caller that keeps the
// revision it read can have a later write refused if the case has changed
// since; see AtRevision. Writes number events one after another, so for a
// case only cases has written it is also the latest event's sequence number.
func (c *Case) Revision() int { return c.events }

// AmendSeq is the sequence number of the last amend that changed the
// question, the one a later answer must have seen, or 0 when there is none.
// An amend that only adds labels does not count.
func (c *Case) AmendSeq() int { return c.amendSeq }

// TransitionError is an event the case's current state does not allow.
type TransitionError struct {
	Event EventType
	From  State
	Kind  Kind
}

func (e *TransitionError) Error() string {
	if e.From == "" {
		return fmt.Sprintf("cannot %s a case that has not been opened", e.Event)
	}
	if e.Event == EventPark && e.From == StateOpen {
		return fmt.Sprintf("cannot park a %s case; only stuck cases park", e.Kind)
	}
	return fmt.Sprintf("cannot %s a case that is %s", e.Event, e.From)
}

// unknown is the error for an event, kind or urgency this build does not
// know. The likely cause is a newer cases writing to the same store, but a
// row that cases did not write reads the same, so the update is offered as a
// guess.
func unknown(what, value string) error {
	return fmt.Errorf("unknown %s %q: perhaps written by a newer cases, or not by cases at all; if newer, update cases on this machine with go install github.com/ryanlewis/cases/cmd/cases@latest", what, value)
}

// fileName is the name an event had as a file, which Event.File keeps.
func fileName(seq int, author Author, typ EventType) string {
	return fmt.Sprintf("%04d-%s-%s.json", seq, author, typ)
}

// authors says who may write each event. An event missing from the map is
// unknown to this version.
var authors = map[EventType][]Author{
	EventOpen:     {AuthorAgent},
	EventAmend:    {AuthorAgent},
	EventAnswer:   {AuthorHuman},
	EventPickup:   {AuthorAgent},
	EventNote:     {AuthorAgent},
	EventClose:    {AuthorAgent},
	EventWithdraw: {AuthorAgent},
	EventPark:     {AuthorHuman},
	EventResume:   {AuthorAgent, AuthorHuman},
}

// newRecord returns an empty body for an event type.
func newRecord(t EventType) record {
	switch t {
	case EventOpen:
		return &OpenRecord{}
	case EventAmend:
		return &AmendRecord{}
	case EventAnswer:
		return &AnswerRecord{}
	case EventPickup:
		return &PickupRecord{}
	case EventNote:
		return &NoteRecord{}
	case EventClose:
		return &CloseRecord{}
	case EventWithdraw:
		return &WithdrawRecord{}
	case EventPark:
		return &ParkRecord{}
	case EventResume:
		return &ResumeRecord{}
	}
	return nil
}

// row is one stored event, as read from the events table.
type row struct {
	seq    int
	author Author
	event  EventType
	data   []byte
}

// foldRows folds the case id from its event rows, in sequence order. An event
// that cannot be folded is skipped and listed in Problems rather than
// failing the fold; foldRows fails only when no row is a valid open event.
func foldRows(id string, rows []row) (*Case, error) {
	c := &Case{ID: id, events: len(rows)}
	for _, r := range rows {
		c.lastSeq = max(c.lastSeq, r.seq)
		name := fileName(r.seq, r.author, r.event)
		ev := Event{Seq: r.seq, Author: r.author, Type: r.event, File: name, Data: r.data}
		if err := c.apply(ev); err != nil {
			c.Problems = append(c.Problems, name+": "+err.Error())
		}
	}
	if c.State == "" {
		if len(c.Problems) > 0 {
			return nil, fmt.Errorf("no valid open event (%s)", strings.Join(c.Problems, "; "))
		}
		return nil, errors.New("no open event")
	}
	return c, nil
}

// apply folds one event into the case. It checks everything before changing
// anything, so an event it refuses leaves the case as it was. The writer runs
// the same check before an event is stored, so a refused event never reaches
// the store through this package.
func (c *Case) apply(ev Event) error {
	allowed, known := authors[ev.Type]
	if !known {
		return unknown("event", string(ev.Type))
	}
	if !slices.Contains(allowed, ev.Author) {
		return fmt.Errorf("%s events are not written by the %s", ev.Type, ev.Author)
	}
	rec := newRecord(ev.Type)
	if err := json.Unmarshal(ev.Data, rec); err != nil {
		return fmt.Errorf("malformed %s event: %w", ev.Type, err)
	}
	if err := checkActor(rec.actor()); err != nil {
		return err
	}
	if c.State == "" && ev.Type != EventOpen {
		return &TransitionError{Event: ev.Type}
	}
	from := c.State
	refuse := &TransitionError{Event: ev.Type, From: from, Kind: c.Kind}

	switch r := rec.(type) {
	case *OpenRecord:
		if c.State != "" {
			return fmt.Errorf("case is already open")
		}
		if err := r.validate(); err != nil {
			return err
		}
		c.OpenRecord = *r
		c.State = StateOpen

	case *AmendRecord:
		if c.State != StateOpen {
			return refuse
		}
		if err := r.validate(&c.OpenRecord); err != nil {
			return err
		}
		// The open event keeps what it said; the case shows the amended
		// fields, and answers are checked against them.
		if r.Body != "" || len(r.Options) > 0 || len(r.Rows) > 0 || len(r.Links) > 0 || r.Context != "" {
			c.amendSeq = ev.Seq
		}
		c.Options = append(c.Options, r.Options...)
		c.Rows = append(c.Rows, r.Rows...)
		c.Links = append(c.Links, r.Links...)
		c.Labels = append(c.Labels, r.Labels...)
		if r.Body != "" {
			ev.replacedBody = c.Body
			c.Body = r.Body
		}
		if r.Context != "" {
			ev.replacedContext = c.Context
			c.Context = r.Context
		}

	case *AnswerRecord:
		if c.State != StateOpen {
			return refuse
		}
		// Every writer numbers its event after the latest one it has read, so
		// an answer numbered at or below amendSeq was written without that
		// amend. The store gives each event of a case its own number, in
		// order, so it holds no such answer; the check is from the directory
		// store, where a sync could bring in a file with an amend's number.
		if c.amendSeq > 0 && ev.Seq <= c.amendSeq {
			return fmt.Errorf("the answer was written without seeing amend %04d", c.amendSeq)
		}
		if err := r.validate(&c.OpenRecord); err != nil {
			return err
		}
		c.Answer = r
		c.State = StateAnswered

	case *PickupRecord:
		if c.State != StateAnswered {
			return refuse
		}
		c.Pickup = r
		c.State = StatePickedUp

	case *NoteRecord:
		switch c.State {
		case StateOpen, StateAnswered, StatePickedUp:
		default:
			return refuse
		}
		if strings.TrimSpace(r.Body) == "" {
			return errors.New("note body is empty")
		}
		// A follow-up after an answer reopens the case for another answer.
		c.Answer = nil
		c.Pickup = nil
		c.State = StateOpen

	case *CloseRecord:
		if c.State != StatePickedUp {
			return refuse
		}
		if strings.TrimSpace(r.Outcome) == "" {
			return errors.New("close outcome is empty")
		}
		c.Close = r
		c.State = StateClosed

	case *WithdrawRecord:
		if c.State != StateOpen {
			return refuse
		}
		c.State = StateWithdrawn

	case *ParkRecord:
		if c.State != StateOpen || c.Kind != KindStuck {
			return refuse
		}
		c.Park = r
		c.State = StateParked

	case *ResumeRecord:
		if c.State != StateParked {
			return refuse
		}
		c.Park = nil
		c.State = StateOpen
	}

	ev.At = rec.at()
	ev.Actor = rec.actor()
	ev.from = from
	c.Events = append(c.Events, ev)
	if ev.At.After(c.UpdatedAt) {
		c.UpdatedAt = ev.At
	}
	return nil
}
