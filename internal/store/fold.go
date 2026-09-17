package store

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"time"
)

// Event is one event file as read from disk.
type Event struct {
	Seq    int       `json:"seq"`
	Author Author    `json:"author"`
	Type   EventType `json:"event"`
	File   string    `json:"file"`
	At     time.Time `json:"at"`
	// Data is the file exactly as written, so fields this version does not
	// know about survive a read.
	Data json.RawMessage `json:"data"`

	// replacedBody and replacedContext are the body and context an amend
	// replaced, as the fold found them, for Describe.
	replacedBody, replacedContext string
}

// Case is the fold of a case directory.
type Case struct {
	ID    string `json:"id"`
	Dir   string `json:"dir"`
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
	// Problems name files that were skipped: unreadable, malformed, or an
	// event the case was not in a state to accept.
	Problems []string `json:"problems,omitempty"`

	lastSeq int
	files   int
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

// Revision is the number of event files in the case, counting any the fold
// skipped. Every event added raises it: one written here, and one a sync
// brings in with a sequence number the case already has or below its latest.
// A caller that keeps the revision it read can have a later write refused if
// the case has changed since; see AtRevision. For a case whose events were
// all written on one machine it is also the sequence number of the latest.
func (c *Case) Revision() int { return c.files }

// ErrNoEvents is a case directory with no event files in it yet. Create makes
// the directory a moment before it writes the open event, so a listing can
// catch one in between; List and Poller skip such a directory silently.
var ErrNoEvents = errors.New("no open event")

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
// know. The likely cause is a newer cases writing to a shared store, but a
// file name with a typo in it reads the same, so the update is offered as a
// guess.
func unknown(what, value string) error {
	return fmt.Errorf("unknown %s %q: perhaps written by a newer cases, or not by cases at all; if newer, update cases on this machine with go install github.com/ryanlewis/cases/cmd/cases@latest", what, value)
}

// eventFile matches an event file name: sequence, author, event.
var eventFile = regexp.MustCompile(`^(\d{4,})-(agent|human)-([a-z]+)\.json$`)

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

// Load folds the case directory dir. Files that cannot be read or folded are
// skipped and listed in Problems rather than failing the load; Load fails only
// when the directory cannot be read or holds no valid open event.
func Load(dir string) (*Case, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, err
	}
	c := &Case{ID: filepath.Base(dir), Dir: dir}

	type file struct {
		seq    int
		author Author
		event  EventType
		name   string
	}
	var files []file
	for _, e := range entries {
		name := e.Name()
		if strings.HasPrefix(name, ".") || e.IsDir() {
			continue
		}
		m := eventFile.FindStringSubmatch(name)
		if m == nil {
			c.Problems = append(c.Problems, name+": not an event file name")
			continue
		}
		seq, err := strconv.Atoi(m[1])
		if err != nil {
			c.Problems = append(c.Problems, name+": "+err.Error())
			continue
		}
		files = append(files, file{seq, Author(m[2]), EventType(m[3]), name})
	}
	slices.SortFunc(files, func(a, b file) int {
		if a.seq != b.seq {
			return a.seq - b.seq
		}
		return strings.Compare(a.name, b.name)
	})
	c.files = len(files)

	for i, f := range files {
		c.lastSeq = max(c.lastSeq, f.seq)
		if i > 0 && files[i-1].seq == f.seq {
			c.Problems = append(c.Problems, fmt.Sprintf("%s: sequence %04d is used by more than one file", f.name, f.seq))
		}
		data, err := os.ReadFile(filepath.Join(dir, f.name))
		if err != nil {
			c.Problems = append(c.Problems, f.name+": "+err.Error())
			continue
		}
		ev := Event{Seq: f.seq, Author: f.author, Type: f.event, File: f.name, Data: data}
		if err := c.apply(ev); err != nil {
			c.Problems = append(c.Problems, f.name+": "+err.Error())
		}
	}
	if c.State == "" {
		if len(c.Problems) > 0 {
			return nil, fmt.Errorf("no valid open event (%s)", strings.Join(c.Problems, "; "))
		}
		return nil, ErrNoEvents
	}
	return c, nil
}

// apply folds one event into the case. It checks everything before changing
// anything, so an event it refuses leaves the case as it was. The writer runs
// the same check before a file is written, so a refused event never reaches
// disk through this package.
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
	if c.State == "" && ev.Type != EventOpen {
		return &TransitionError{Event: ev.Type}
	}
	refuse := &TransitionError{Event: ev.Type, From: c.State, Kind: c.Kind}

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
		// amend, such as on a machine the amend had not synced to yet.
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
	c.Events = append(c.Events, ev)
	if ev.At.After(c.UpdatedAt) {
		c.UpdatedAt = ev.At
	}
	return nil
}
