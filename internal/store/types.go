// Package store reads and writes the case store: one directory per case, one
// JSON file per event, named NNNN-<author>-<event>.json. A case's state is the
// fold of its event files in filename order. Files are only ever added, never
// edited in place.
package store

import (
	"fmt"
	"regexp"
	"slices"
	"time"
)

// Kind is what sort of answer a case asks for.
type Kind string

const (
	KindDecision Kind = "decision"
	KindApproval Kind = "approval"
	KindSignoff  Kind = "signoff"
	KindStuck    Kind = "stuck"
	KindQuestion Kind = "question"
	KindFYI      Kind = "fyi"
)

// Kinds lists every kind in the order the docs present them.
var Kinds = []Kind{KindDecision, KindApproval, KindSignoff, KindStuck, KindQuestion, KindFYI}

// Urgency orders the inbox.
type Urgency string

const (
	UrgencyBlocking Urgency = "blocking"
	UrgencyToday    Urgency = "today"
	UrgencyWhenever Urgency = "whenever"
)

// Urgencies lists every urgency, most urgent first.
var Urgencies = []Urgency{UrgencyBlocking, UrgencyToday, UrgencyWhenever}

// Rank is the urgency's position in Urgencies, for sorting. An unknown value
// sorts last.
func (u Urgency) Rank() int {
	if i := slices.Index(Urgencies, u); i >= 0 {
		return i
	}
	return len(Urgencies)
}

// State is where a case is in its lifecycle. It is never stored; it is derived
// by folding the event files.
type State string

const (
	StateOpen      State = "open"
	StateAnswered  State = "answered"
	StatePickedUp  State = "pickedup"
	StateClosed    State = "closed"
	StateWithdrawn State = "withdrawn"
	StateParked    State = "parked"
)

// States lists every state.
var States = []State{StateOpen, StateAnswered, StatePickedUp, StateClosed, StateWithdrawn, StateParked}

// Author is who wrote an event. It is part of the file name.
type Author string

const (
	AuthorAgent Author = "agent"
	AuthorHuman Author = "human"
)

// EventType is the event a file records. It is part of the file name.
type EventType string

const (
	EventOpen     EventType = "open"
	EventAnswer   EventType = "answer"
	EventPickup   EventType = "pickup"
	EventNote     EventType = "note"
	EventClose    EventType = "close"
	EventWithdraw EventType = "withdraw"
	EventPark     EventType = "park"
	EventResume   EventType = "resume"
)

// Row verdicts for an approval answer.
const (
	VerdictApprove = "approve"
	VerdictHold    = "hold"
	VerdictReject  = "reject"
)

// Signoff responses.
const (
	SignoffAccept  = "accept"
	SignoffChanges = "changes"
)

// rowIDPattern keeps row ids safe to type on a command line as id=verdict.
var rowIDPattern = regexp.MustCompile(`^[A-Za-z0-9._-]+$`)

// Row is one line of an approval case: a script or gated action. The script
// text is carried inline so it can be read on a phone, and the link points at
// where it lives. Note is optional context for the human, shown under the
// label.
type Row struct {
	ID     string `json:"id"`
	Label  string `json:"label"`
	Note   string `json:"note,omitempty"`
	Script string `json:"script"`
	Link   string `json:"link"`
}

// OpenRecord is the body of NNNN-agent-open.json.
type OpenRecord struct {
	Kind     Kind      `json:"kind"`
	Urgency  Urgency   `json:"urgency"`
	Title    string    `json:"title"`
	Body     string    `json:"body,omitempty"`
	Options  []string  `json:"options,omitempty"`
	Rows     []Row     `json:"rows,omitempty"`
	Links    []string  `json:"links,omitempty"`
	Worker   string    `json:"worker,omitempty"`
	Brief    string    `json:"brief,omitempty"`
	Context  string    `json:"context,omitempty"`
	OpenedAt time.Time `json:"opened_at"`
}

// RowAnswer is the verdict on one approval row.
type RowAnswer struct {
	ID      string `json:"id"`
	Verdict string `json:"verdict"`
	Note    string `json:"note,omitempty"`
}

// AnswerRecord is the body of NNNN-human-answer.json. Which fields may be set
// depends on the case kind:
//
//   - decision: Choice (1-based index into the options) or Other with a Note
//   - approval: Rows, one verdict per row of the case
//   - signoff: Signoff "accept", or "changes" with a Note
//   - stuck: Text (guidance) or Drop; parking is a separate park event
//   - question: Text (the reply)
//   - fyi: Ack
//
// Note is allowed on every kind.
type AnswerRecord struct {
	Choice     int         `json:"choice,omitempty"`
	Other      bool        `json:"other,omitempty"`
	Rows       []RowAnswer `json:"rows,omitempty"`
	Signoff    string      `json:"signoff,omitempty"`
	Text       string      `json:"text,omitempty"`
	Drop       bool        `json:"drop,omitempty"`
	Ack        bool        `json:"ack,omitempty"`
	Note       string      `json:"note,omitempty"`
	AnsweredAt time.Time   `json:"answered_at"`
}

// PickupRecord is the body of NNNN-agent-pickup.json.
type PickupRecord struct {
	By         string    `json:"by,omitempty"`
	PickedUpAt time.Time `json:"picked_up_at"`
}

// NoteRecord is the body of NNNN-agent-note.json: a follow-up in the thread.
type NoteRecord struct {
	Body    string    `json:"body"`
	NotedAt time.Time `json:"noted_at"`
}

// CloseRecord is the body of NNNN-agent-close.json.
type CloseRecord struct {
	Outcome  string    `json:"outcome"`
	Links    []string  `json:"links,omitempty"`
	ClosedAt time.Time `json:"closed_at"`
}

// WithdrawRecord is the body of NNNN-agent-withdraw.json.
type WithdrawRecord struct {
	Reason      string    `json:"reason,omitempty"`
	WithdrawnAt time.Time `json:"withdrawn_at"`
}

// ParkRecord is the body of NNNN-human-park.json.
type ParkRecord struct {
	Note     string    `json:"note,omitempty"`
	ParkedAt time.Time `json:"parked_at"`
}

// ResumeRecord is the body of NNNN-<author>-resume.json.
type ResumeRecord struct {
	ResumedAt time.Time `json:"resumed_at"`
}

// record is implemented by every event body so the fold and the writer can
// read and set its timestamp without a type switch.
type record interface {
	at() time.Time
	stamp(t time.Time)
}

func (r *OpenRecord) at() time.Time     { return r.OpenedAt }
func (r *AnswerRecord) at() time.Time   { return r.AnsweredAt }
func (r *PickupRecord) at() time.Time   { return r.PickedUpAt }
func (r *NoteRecord) at() time.Time     { return r.NotedAt }
func (r *CloseRecord) at() time.Time    { return r.ClosedAt }
func (r *WithdrawRecord) at() time.Time { return r.WithdrawnAt }
func (r *ParkRecord) at() time.Time     { return r.ParkedAt }
func (r *ResumeRecord) at() time.Time   { return r.ResumedAt }

func (r *OpenRecord) stamp(t time.Time)     { r.OpenedAt = stampTime(r.OpenedAt, t) }
func (r *AnswerRecord) stamp(t time.Time)   { r.AnsweredAt = stampTime(r.AnsweredAt, t) }
func (r *PickupRecord) stamp(t time.Time)   { r.PickedUpAt = stampTime(r.PickedUpAt, t) }
func (r *NoteRecord) stamp(t time.Time)     { r.NotedAt = stampTime(r.NotedAt, t) }
func (r *CloseRecord) stamp(t time.Time)    { r.ClosedAt = stampTime(r.ClosedAt, t) }
func (r *WithdrawRecord) stamp(t time.Time) { r.WithdrawnAt = stampTime(r.WithdrawnAt, t) }
func (r *ParkRecord) stamp(t time.Time)     { r.ParkedAt = stampTime(r.ParkedAt, t) }
func (r *ResumeRecord) stamp(t time.Time)   { r.ResumedAt = stampTime(r.ResumedAt, t) }

// stampTime keeps a timestamp the caller set, in UTC, and fills in t otherwise.
func stampTime(have, t time.Time) time.Time {
	if have.IsZero() {
		return t.UTC()
	}
	return have.UTC()
}

// ParseState checks s against States.
func ParseState(s string) (State, error) {
	if st := State(s); slices.Contains(States, st) {
		return st, nil
	}
	return "", fmt.Errorf("unknown state %q", s)
}
