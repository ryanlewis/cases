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
	// amendSeq is the sequence number of the last amend folded in.
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
		return fmt.Errorf("unknown event %q", ev.Type)
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
		// fields, and answers are checked against them. An answer is never
		// checked against labels, so an answer written without seeing an
		// amend that only adds labels still folds. (A web form sent with
		// AtRevision is still refused, as for any new event.)
		if r.Body != "" || len(r.Options) > 0 || len(r.Rows) > 0 || len(r.Links) > 0 || r.Context != "" {
			c.amendSeq = ev.Seq
		}
		c.Options = append(c.Options, r.Options...)
		c.Rows = append(c.Rows, r.Rows...)
		c.Links = append(c.Links, r.Links...)
		c.Labels = append(c.Labels, r.Labels...)
		if r.Body != "" {
			c.Body = r.Body
		}
		if r.Context != "" {
			c.Context = r.Context
		}

	case *AnswerRecord:
		if c.State != StateOpen {
			return refuse
		}
		// Every writer numbers its event after the latest one it has read, so
		// an answer numbered at or below the last amend was written without
		// it, such as on a machine the amend had not synced to yet.
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

func (r *OpenRecord) validate() error {
	if !slices.Contains(Kinds, r.Kind) {
		return fmt.Errorf("unknown kind %q", r.Kind)
	}
	if !slices.Contains(Urgencies, r.Urgency) {
		return fmt.Errorf("unknown urgency %q", r.Urgency)
	}
	if strings.TrimSpace(r.Title) == "" {
		return errors.New("title is empty")
	}
	if err := checkKindFields(r.Kind, r.Options, r.Rows); err != nil {
		return err
	}
	switch r.Kind {
	case KindDecision:
		if len(r.Options) == 0 {
			return errors.New("a decision case needs at least one option")
		}
		if err := checkOptions(r.Options); err != nil {
			return err
		}
	case KindApproval:
		if len(r.Rows) == 0 {
			return errors.New("an approval case needs at least one row")
		}
		if err := checkRows(r.Rows, nil); err != nil {
			return err
		}
	}
	return checkLabels(r.Labels, nil)
}

// validate checks the amend against cur, the case as it stands: the open
// event and any earlier amends.
func (r *AmendRecord) validate(cur *OpenRecord) error {
	if r.Body == "" && len(r.Options) == 0 && len(r.Rows) == 0 && len(r.Links) == 0 && len(r.Labels) == 0 && r.Context == "" {
		return errors.New("the amend has no body, options, rows, links, labels or context")
	}
	if err := checkKindFields(cur.Kind, r.Options, r.Rows); err != nil {
		return err
	}
	// An empty body or context leaves the case's as it was, so one that is
	// only whitespace is a mistake rather than a way to clear it.
	if r.Body != "" && strings.TrimSpace(r.Body) == "" {
		return errors.New("amend body is empty")
	}
	if r.Context != "" && strings.TrimSpace(r.Context) == "" {
		return errors.New("amend context is empty")
	}
	if err := checkOptions(r.Options); err != nil {
		return err
	}
	if err := checkRows(r.Rows, cur.Rows); err != nil {
		return err
	}
	for i, l := range r.Links {
		if strings.TrimSpace(l) == "" {
			return fmt.Errorf("link %d is empty", i+1)
		}
	}
	// An option or link the case already has adds nothing, as when an amend
	// is sent again.
	if err := checkNew("option", r.Options, cur.Options); err != nil {
		return err
	}
	if err := checkNew("link", r.Links, cur.Links); err != nil {
		return err
	}
	if err := checkLabels(r.Labels, cur.Labels); err != nil {
		return err
	}
	if len(r.Options) == 0 && len(r.Rows) == 0 && len(r.Links) == 0 && len(r.Labels) == 0 &&
		(r.Body == "" || r.Body == cur.Body) && (r.Context == "" || r.Context == cur.Context) {
		return errors.New("the amend changes nothing")
	}
	return nil
}

// checkKindFields checks that options are only on a decision case and rows
// only on an approval case.
func checkKindFields(kind Kind, options []string, rows []Row) error {
	if kind != KindDecision && len(options) > 0 {
		return fmt.Errorf("options are for decision cases, not %s", kind)
	}
	if kind != KindApproval && len(rows) > 0 {
		return fmt.Errorf("rows are for approval cases, not %s", kind)
	}
	return nil
}

// checkNew checks that each option or link an amend adds is not on the case
// already, in have, and is not added twice.
func checkNew(what string, added, have []string) error {
	for i, s := range added {
		switch {
		case slices.Contains(added[:i], s):
			return fmt.Errorf("%s %q is used twice", what, s)
		case slices.Contains(have, s):
			return fmt.Errorf("%s %q is already on the case", what, s)
		}
	}
	return nil
}

// checkLabels checks labels added to a case that already has the labels in
// have: none is blank, on the case already, or added twice.
func checkLabels(added, have []string) error {
	for i, l := range added {
		if strings.TrimSpace(l) == "" {
			return fmt.Errorf("label %d is empty", i+1)
		}
	}
	return checkNew("label", added, have)
}

// checkOptions checks that no option is blank.
func checkOptions(options []string) error {
	for i, o := range options {
		if strings.TrimSpace(o) == "" {
			return fmt.Errorf("option %d is empty", i+1)
		}
	}
	return nil
}

// checkRows checks rows added to a case that already has the rows in have:
// each id is safe to type as id=verdict and names one row on the case, and
// the label, script and link are set.
func checkRows(rows, have []Row) error {
	seen := map[string]bool{}
	for i, row := range rows {
		switch {
		case !rowIDPattern.MatchString(row.ID):
			return fmt.Errorf("row %d: id %q must be letters, digits, dot, dash or underscore", i+1, row.ID)
		case seen[row.ID]:
			return fmt.Errorf("row %d: id %q is used twice", i+1, row.ID)
		case slices.ContainsFunc(have, func(h Row) bool { return h.ID == row.ID }):
			return fmt.Errorf("row %d: id %q is already on the case", i+1, row.ID)
		case strings.TrimSpace(row.Label) == "":
			return fmt.Errorf("row %q: label is empty", row.ID)
		case strings.TrimSpace(row.Script) == "":
			return fmt.Errorf("row %q: script is empty", row.ID)
		case strings.TrimSpace(row.Link) == "":
			return fmt.Errorf("row %q: link is empty", row.ID)
		}
		seen[row.ID] = true
	}
	return nil
}

// answerFields names the fields each kind's answer may set, besides the note.
var answerFields = map[Kind][]string{
	KindDecision: {"choice", "other"},
	KindApproval: {"rows"},
	KindSignoff:  {"signoff"},
	KindStuck:    {"text", "drop"},
	KindQuestion: {"text"},
	KindFYI:      {"ack"},
}

// set lists the response fields the answer sets.
func (r *AnswerRecord) set() []string {
	var f []string
	add := func(name string, on bool) {
		if on {
			f = append(f, name)
		}
	}
	add("choice", r.Choice != 0)
	add("other", r.Other)
	add("rows", len(r.Rows) > 0)
	add("signoff", r.Signoff != "")
	add("text", r.Text != "")
	add("drop", r.Drop)
	add("ack", r.Ack)
	return f
}

// validate checks the answer against the shape the case's kind asks for, as
// the case stands in cur after any amends.
func (r *AnswerRecord) validate(cur *OpenRecord) error {
	set := r.set()
	for _, name := range set {
		if !slices.Contains(answerFields[cur.Kind], name) {
			return fmt.Errorf("%s answers cannot set %s", cur.Kind, name)
		}
	}
	if len(set) == 0 {
		return fmt.Errorf("the answer has no %s response", cur.Kind)
	}
	hasNote := strings.TrimSpace(r.Note) != ""

	switch cur.Kind {
	case KindDecision:
		if len(set) > 1 {
			return errors.New("choose one option or other, not both")
		}
		if r.Other && !hasNote {
			return errors.New("other needs a note")
		}
		if !r.Other && (r.Choice < 1 || r.Choice > len(cur.Options)) {
			return fmt.Errorf("choice %d is not an option (1-%d)", r.Choice, len(cur.Options))
		}
	case KindApproval:
		got := map[string]bool{}
		for _, row := range r.Rows {
			if !slices.ContainsFunc(cur.Rows, func(o Row) bool { return o.ID == row.ID }) {
				return fmt.Errorf("row %q is not on this case", row.ID)
			}
			if got[row.ID] {
				return fmt.Errorf("row %q is answered twice", row.ID)
			}
			switch row.Verdict {
			case VerdictApprove, VerdictHold, VerdictReject:
			default:
				return fmt.Errorf("row %q: verdict %q is not approve, hold or reject", row.ID, row.Verdict)
			}
			got[row.ID] = true
		}
		for _, o := range cur.Rows {
			if !got[o.ID] {
				return fmt.Errorf("row %q has no verdict", o.ID)
			}
		}
	case KindSignoff:
		switch r.Signoff {
		case SignoffAccept:
		case SignoffChanges:
			if !hasNote {
				return errors.New("requesting changes needs a note")
			}
		default:
			return fmt.Errorf("signoff %q is not accept or changes", r.Signoff)
		}
	case KindStuck:
		if len(set) > 1 {
			return errors.New("give guidance text or drop, not both")
		}
		if !r.Drop && strings.TrimSpace(r.Text) == "" {
			return errors.New("guidance text is empty")
		}
	case KindQuestion:
		if strings.TrimSpace(r.Text) == "" {
			return errors.New("reply text is empty")
		}
	case KindFYI:
		// Ack is the only field allowed and at least one field is set.
	}
	return nil
}
