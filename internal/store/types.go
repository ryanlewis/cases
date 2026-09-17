// Package store reads and writes the case store: one SQLite file holding a
// row per case and a row per event, the event's JSON record as written. Each
// event is numbered within its case and keeps the name it had as a file,
// NNNN-<author>-<event>.json. A case's state is the fold of its events in
// number order. Events are only ever added, never changed.
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
// by folding the events.
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

// Author is who wrote an event. It is part of the event's file name.
type Author string

const (
	AuthorAgent Author = "agent"
	AuthorHuman Author = "human"
)

// EventType is the event a record is. It is part of the event's file name.
type EventType string

const (
	EventOpen     EventType = "open"
	EventAmend    EventType = "amend"
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

// OpenRecord is the record of an open event, NNNN-agent-open.json.
type OpenRecord struct {
	Kind    Kind     `json:"kind"`
	Urgency Urgency  `json:"urgency"`
	Title   string   `json:"title"`
	Body    string   `json:"body,omitempty"`
	Options []string `json:"options,omitempty"`
	Rows    []Row    `json:"rows,omitempty"`
	Links   []string `json:"links,omitempty"`
	Labels  []string `json:"labels,omitempty"`
	Worker  string   `json:"worker,omitempty"`
	Brief   string   `json:"brief,omitempty"`
	Context string   `json:"context,omitempty"`
	// For names who the case is addressed to, such as the human expected to
	// answer it. Nothing checks it against who does.
	For      string    `json:"for,omitempty"`
	Actor    *Actor    `json:"actor,omitempty"`
	OpenedAt time.Time `json:"opened_at"`
}

// AmendRecord is the record of an amend event, NNNN-agent-amend.json: a
// change to an open case.
// Options, rows, links and labels are added after the case's own; a body or
// context replaces the case's. A field left empty leaves the case's as it was.
type AmendRecord struct {
	Body      string    `json:"body,omitempty"`
	Options   []string  `json:"options,omitempty"`
	Rows      []Row     `json:"rows,omitempty"`
	Links     []string  `json:"links,omitempty"`
	Labels    []string  `json:"labels,omitempty"`
	Context   string    `json:"context,omitempty"`
	Actor     *Actor    `json:"actor,omitempty"`
	AmendedAt time.Time `json:"amended_at"`
}

// RowAnswer is the verdict on one approval row.
type RowAnswer struct {
	ID      string `json:"id"`
	Verdict string `json:"verdict"`
	Note    string `json:"note,omitempty"`
}

// AnswerRecord is the record of an answer event, NNNN-human-answer.json.
// Which fields may be set depends on the case kind:
//
//   - decision: Choice (1-based index into the options) or Other with a Note
//   - approval: Rows, one verdict per row of the case
//   - signoff: Signoff "accept", or "changes" with a Note
//   - stuck: Text (guidance); parking is a separate park event
//   - question: Text (the reply)
//   - fyi: Ack
//
// Drop, which dismisses the case, may stand in for the response on every
// kind. Note is allowed on every kind.
type AnswerRecord struct {
	Choice     int         `json:"choice,omitempty"`
	Other      bool        `json:"other,omitempty"`
	Rows       []RowAnswer `json:"rows,omitempty"`
	Signoff    string      `json:"signoff,omitempty"`
	Text       string      `json:"text,omitempty"`
	Drop       bool        `json:"drop,omitempty"`
	Ack        bool        `json:"ack,omitempty"`
	Note       string      `json:"note,omitempty"`
	Actor      *Actor      `json:"actor,omitempty"`
	AnsweredAt time.Time   `json:"answered_at"`
}

// PickupRecord is the record of a pickup event, NNNN-agent-pickup.json.
type PickupRecord struct {
	By         string    `json:"by,omitempty"`
	Actor      *Actor    `json:"actor,omitempty"`
	PickedUpAt time.Time `json:"picked_up_at"`
}

// NoteRecord is the record of a note event, NNNN-agent-note.json: a
// follow-up in the thread.
type NoteRecord struct {
	Body    string    `json:"body"`
	Actor   *Actor    `json:"actor,omitempty"`
	NotedAt time.Time `json:"noted_at"`
}

// CloseRecord is the record of a close event, NNNN-agent-close.json.
type CloseRecord struct {
	Outcome  string    `json:"outcome"`
	Links    []string  `json:"links,omitempty"`
	Actor    *Actor    `json:"actor,omitempty"`
	ClosedAt time.Time `json:"closed_at"`
}

// WithdrawRecord is the record of a withdraw event, NNNN-agent-withdraw.json.
type WithdrawRecord struct {
	Reason      string    `json:"reason,omitempty"`
	Actor       *Actor    `json:"actor,omitempty"`
	WithdrawnAt time.Time `json:"withdrawn_at"`
}

// ParkRecord is the record of a park event, NNNN-human-park.json.
type ParkRecord struct {
	Note     string    `json:"note,omitempty"`
	Actor    *Actor    `json:"actor,omitempty"`
	ParkedAt time.Time `json:"parked_at"`
}

// ResumeRecord is the record of a resume event, NNNN-<author>-resume.json.
type ResumeRecord struct {
	Actor     *Actor    `json:"actor,omitempty"`
	ResumedAt time.Time `json:"resumed_at"`
}

// Actor is who wrote an event: the human's name, or the agent session's. It is
// optional and recorded as given; the store does not check it against the
// event's author or decide who may write what.
type Actor struct {
	Name string `json:"name"`
	Kind string `json:"kind"`
}

// record is implemented by every event body so the fold and the writer can
// read and set its timestamp and actor without a type switch.
type record interface {
	at() time.Time
	stamp(t time.Time)
	actor() *Actor
}

func (r *OpenRecord) at() time.Time     { return r.OpenedAt }
func (r *AmendRecord) at() time.Time    { return r.AmendedAt }
func (r *AnswerRecord) at() time.Time   { return r.AnsweredAt }
func (r *PickupRecord) at() time.Time   { return r.PickedUpAt }
func (r *NoteRecord) at() time.Time     { return r.NotedAt }
func (r *CloseRecord) at() time.Time    { return r.ClosedAt }
func (r *WithdrawRecord) at() time.Time { return r.WithdrawnAt }
func (r *ParkRecord) at() time.Time     { return r.ParkedAt }
func (r *ResumeRecord) at() time.Time   { return r.ResumedAt }

func (r *OpenRecord) stamp(t time.Time)     { r.OpenedAt = stampTime(r.OpenedAt, t) }
func (r *AmendRecord) stamp(t time.Time)    { r.AmendedAt = stampTime(r.AmendedAt, t) }
func (r *AnswerRecord) stamp(t time.Time)   { r.AnsweredAt = stampTime(r.AnsweredAt, t) }
func (r *PickupRecord) stamp(t time.Time)   { r.PickedUpAt = stampTime(r.PickedUpAt, t) }
func (r *NoteRecord) stamp(t time.Time)     { r.NotedAt = stampTime(r.NotedAt, t) }
func (r *CloseRecord) stamp(t time.Time)    { r.ClosedAt = stampTime(r.ClosedAt, t) }
func (r *WithdrawRecord) stamp(t time.Time) { r.WithdrawnAt = stampTime(r.WithdrawnAt, t) }
func (r *ParkRecord) stamp(t time.Time)     { r.ParkedAt = stampTime(r.ParkedAt, t) }
func (r *ResumeRecord) stamp(t time.Time)   { r.ResumedAt = stampTime(r.ResumedAt, t) }

func (r *OpenRecord) actor() *Actor     { return r.Actor }
func (r *AmendRecord) actor() *Actor    { return r.Actor }
func (r *AnswerRecord) actor() *Actor   { return r.Actor }
func (r *PickupRecord) actor() *Actor   { return r.Actor }
func (r *NoteRecord) actor() *Actor     { return r.Actor }
func (r *CloseRecord) actor() *Actor    { return r.Actor }
func (r *WithdrawRecord) actor() *Actor { return r.Actor }
func (r *ParkRecord) actor() *Actor     { return r.Actor }
func (r *ResumeRecord) actor() *Actor   { return r.Actor }

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
