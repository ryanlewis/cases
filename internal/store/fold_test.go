package store

import (
	"encoding/json"
	"errors"
	"slices"
	"strings"
	"testing"
)

// step is one event to fold in a test.
type step struct {
	author Author
	typ    EventType
	rec    any
}

func agent(typ EventType, rec any) step { return step{AuthorAgent, typ, rec} }
func human(typ EventType, rec any) step { return step{AuthorHuman, typ, rec} }

// fold applies steps to an empty case in memory and stops at the first error,
// returning the case as it stood and the index of the step that failed.
func fold(t *testing.T, steps ...step) (*Case, int, error) {
	t.Helper()
	c := &Case{}
	for i, s := range steps {
		data, err := json.Marshal(s.rec)
		if err != nil {
			t.Fatalf("marshal step %d: %v", i, err)
		}
		if err := c.apply(Event{Seq: i + 1, Author: s.author, Type: s.typ, Data: data}); err != nil {
			return c, i, err
		}
	}
	return c, -1, nil
}

func openOf(kind Kind) OpenRecord {
	rec := OpenRecord{Kind: kind, Urgency: UrgencyToday, Title: "A " + string(kind) + " case", Body: "Body."}
	switch kind {
	case KindDecision:
		rec.Options = []string{"Pin", "Float"}
	case KindApproval:
		rec.Rows = []Row{
			{ID: "a", Label: "Install deps", Script: "npm ci", Link: "https://example.com/a"},
			{ID: "b", Label: "Migrate", Script: "make migrate", Link: "https://example.com/b"},
		}
	}
	return rec
}

func answerOf(kind Kind) AnswerRecord {
	switch kind {
	case KindDecision:
		return AnswerRecord{Choice: 2}
	case KindApproval:
		return AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictApprove}, {ID: "b", Verdict: VerdictHold, Note: "later"}}}
	case KindSignoff:
		return AnswerRecord{Signoff: SignoffAccept}
	case KindStuck:
		return AnswerRecord{Text: "Try the other mirror."}
	case KindQuestion:
		return AnswerRecord{Text: "The staging one."}
	default:
		return AnswerRecord{Ack: true}
	}
}

var (
	amendStep    = agent(EventAmend, AmendRecord{Context: "Seen on the mirror too."})
	pickupStep   = agent(EventPickup, PickupRecord{By: "bun-pins"})
	noteStep     = agent(EventNote, NoteRecord{Body: "One more thing?"})
	closeStep    = agent(EventClose, CloseRecord{Outcome: "Done."})
	withdrawStep = agent(EventWithdraw, WithdrawRecord{})
	parkStep     = human(EventPark, ParkRecord{Note: "after the release"})
	resumeHuman  = human(EventResume, ResumeRecord{})
	resumeAgent  = agent(EventResume, ResumeRecord{})
)

// TestTransitionMatrix tries every event from every state of a stuck case,
// the one kind that can reach all six states, against the lifecycle diagram.
func TestTransitionMatrix(t *testing.T) {
	open := agent(EventOpen, openOf(KindStuck))
	answer := human(EventAnswer, answerOf(KindStuck))

	paths := map[State][]step{
		StateOpen:      {open},
		StateAnswered:  {open, answer},
		StatePickedUp:  {open, answer, pickupStep},
		StateClosed:    {open, answer, pickupStep, closeStep},
		StateWithdrawn: {open, withdrawStep},
		StateParked:    {open, parkStep},
	}
	events := map[string]step{
		"open":         open,
		"amend":        amendStep,
		"answer":       answer,
		"pickup":       pickupStep,
		"note":         noteStep,
		"close":        closeStep,
		"withdraw":     withdrawStep,
		"park":         parkStep,
		"resume human": resumeHuman,
		"resume agent": resumeAgent,
	}
	// allowed is the diagram: from state, event -> next state. Anything not
	// listed must be refused.
	allowed := map[State]map[string]State{
		StateOpen: {
			"answer":   StateAnswered,
			"withdraw": StateWithdrawn,
			"park":     StateParked,
			"note":     StateOpen,
			"amend":    StateOpen,
		},
		StateAnswered: {
			"pickup": StatePickedUp,
			"note":   StateOpen,
		},
		StatePickedUp: {
			"close": StateClosed,
			"note":  StateOpen,
		},
		StateParked: {
			"resume human": StateOpen,
			"resume agent": StateOpen,
		},
		StateClosed:    {},
		StateWithdrawn: {},
	}

	for from, path := range paths {
		for name, ev := range events {
			t.Run(string(from)+"/"+name, func(t *testing.T) {
				c, failed, err := fold(t, append(append([]step{}, path...), ev)...)
				if failed >= 0 && failed < len(path) {
					t.Fatalf("path to %s failed at step %d: %v", from, failed, err)
				}
				want, ok := allowed[from][name]
				if !ok {
					if err == nil {
						t.Fatalf("%s from %s was accepted, state now %s", name, from, c.State)
					}
					if c.State != from {
						t.Errorf("refused event changed state to %s", c.State)
					}
					if len(c.Events) != len(path) {
						t.Errorf("refused event was recorded: %d events", len(c.Events))
					}
					return
				}
				if err != nil {
					t.Fatalf("%s from %s refused: %v", name, from, err)
				}
				if c.State != want {
					t.Errorf("state = %s, want %s", c.State, want)
				}
			})
		}
	}
}

func TestTransitions(t *testing.T) {
	tests := []struct {
		name    string
		steps   []step
		want    State
		wantErr string
	}{
		{
			name:  "decision full lifecycle",
			steps: []step{agent(EventOpen, openOf(KindDecision)), human(EventAnswer, answerOf(KindDecision)), pickupStep, closeStep},
			want:  StateClosed,
		},
		{
			name: "record example from the design: answer, pickup, note reopens, answer again",
			steps: []step{
				agent(EventOpen, openOf(KindDecision)),
				human(EventAnswer, answerOf(KindDecision)),
				pickupStep,
				noteStep,
				human(EventAnswer, AnswerRecord{Choice: 1}),
				pickupStep,
				closeStep,
			},
			want: StateClosed,
		},
		{
			name:  "note after answered reopens",
			steps: []step{agent(EventOpen, openOf(KindFYI)), human(EventAnswer, answerOf(KindFYI)), noteStep},
			want:  StateOpen,
		},
		{
			name:  "park then resume then answer",
			steps: []step{agent(EventOpen, openOf(KindStuck)), parkStep, resumeHuman, human(EventAnswer, AnswerRecord{Drop: true})},
			want:  StateAnswered,
		},
		{
			name:    "park a decision case",
			steps:   []step{agent(EventOpen, openOf(KindDecision)), parkStep},
			wantErr: "only stuck cases park",
		},
		{
			name:  "question full lifecycle",
			steps: []step{agent(EventOpen, openOf(KindQuestion)), human(EventAnswer, answerOf(KindQuestion)), pickupStep, closeStep},
			want:  StateClosed,
		},
		{
			name:    "park a question case",
			steps:   []step{agent(EventOpen, openOf(KindQuestion)), parkStep},
			wantErr: "cannot park a question case; only stuck cases park",
		},
		{
			name:    "park an answered stuck case",
			steps:   []step{agent(EventOpen, openOf(KindStuck)), human(EventAnswer, answerOf(KindStuck)), parkStep},
			wantErr: "cannot park a case that is answered",
		},
		{
			name:    "answer before open",
			steps:   []step{human(EventAnswer, answerOf(KindFYI))},
			wantErr: "has not been opened",
		},
		{
			name:    "close without pickup",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), human(EventAnswer, answerOf(KindFYI)), closeStep},
			wantErr: "cannot close a case that is answered",
		},
		{
			name:    "human cannot pick up",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), human(EventAnswer, answerOf(KindFYI)), human(EventPickup, PickupRecord{})},
			wantErr: "pickup events are not written by the human",
		},
		{
			name:    "agent cannot answer",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), agent(EventAnswer, answerOf(KindFYI))},
			wantErr: "answer events are not written by the agent",
		},
		{
			name:    "agent cannot park",
			steps:   []step{agent(EventOpen, openOf(KindStuck)), agent(EventPark, ParkRecord{})},
			wantErr: "park events are not written by the agent",
		},
		{
			name:    "human cannot amend",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), human(EventAmend, AmendRecord{Body: "Changed."})},
			wantErr: "amend events are not written by the human",
		},
		{
			name:    "amend an answered case",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), human(EventAnswer, answerOf(KindFYI)), agent(EventAmend, AmendRecord{Body: "Changed."})},
			wantErr: "cannot amend a case that is answered",
		},
		{
			name:  "amend a case a note reopened",
			steps: []step{agent(EventOpen, openOf(KindFYI)), human(EventAnswer, answerOf(KindFYI)), noteStep, agent(EventAmend, AmendRecord{Body: "Changed."})},
			want:  StateOpen,
		},
		{
			name:    "unknown event",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), agent("comment", NoteRecord{Body: "x"})},
			wantErr: `unknown event "comment"`,
		},
		{
			name:    "empty note",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), agent(EventNote, NoteRecord{Body: "  "})},
			wantErr: "note body is empty",
		},
		{
			name:    "empty outcome",
			steps:   []step{agent(EventOpen, openOf(KindFYI)), human(EventAnswer, answerOf(KindFYI)), pickupStep, agent(EventClose, CloseRecord{})},
			wantErr: "close outcome is empty",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c, _, err := fold(t, tt.steps...)
			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("err = %v, want %q", err, tt.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("fold: %v", err)
			}
			if c.State != tt.want {
				t.Errorf("state = %s, want %s", c.State, tt.want)
			}
		})
	}
}

func TestTransitionErrorType(t *testing.T) {
	_, _, err := fold(t, agent(EventOpen, openOf(KindFYI)), pickupStep)
	var te *TransitionError
	if !errors.As(err, &te) {
		t.Fatalf("err = %v, want *TransitionError", err)
	}
	if te.Event != EventPickup || te.From != StateOpen {
		t.Errorf("got %+v", te)
	}
}

func TestReopenClearsAnswerAndPickup(t *testing.T) {
	c, _, err := fold(t,
		agent(EventOpen, openOf(KindDecision)),
		human(EventAnswer, answerOf(KindDecision)),
		pickupStep,
		noteStep,
	)
	if err != nil {
		t.Fatal(err)
	}
	if c.Answer != nil || c.Pickup != nil {
		t.Errorf("answer = %+v, pickup = %+v; want both cleared after reopen", c.Answer, c.Pickup)
	}
	if len(c.Events) != 4 {
		t.Errorf("events = %d, want the whole thread of 4", len(c.Events))
	}
}

func TestOpenValidation(t *testing.T) {
	tests := []struct {
		name    string
		mutate  func(*OpenRecord)
		kind    Kind
		wantErr string
	}{
		{name: "valid decision", kind: KindDecision},
		{name: "valid approval", kind: KindApproval},
		{name: "valid signoff", kind: KindSignoff},
		{name: "valid stuck", kind: KindStuck},
		{name: "valid question", kind: KindQuestion},
		{name: "valid fyi", kind: KindFYI},
		{name: "unknown kind", kind: KindFYI, mutate: func(r *OpenRecord) { r.Kind = "poll" }, wantErr: `unknown kind "poll"`},
		{name: "unknown urgency", kind: KindFYI, mutate: func(r *OpenRecord) { r.Urgency = "asap" }, wantErr: `unknown urgency "asap"`},
		{name: "empty title", kind: KindFYI, mutate: func(r *OpenRecord) { r.Title = " " }, wantErr: "title is empty"},
		{name: "decision without options", kind: KindDecision, mutate: func(r *OpenRecord) { r.Options = nil }, wantErr: "at least one option"},
		{name: "decision with empty option", kind: KindDecision, mutate: func(r *OpenRecord) { r.Options[1] = "" }, wantErr: "option 2 is empty"},
		{name: "options on fyi", kind: KindFYI, mutate: func(r *OpenRecord) { r.Options = []string{"x"} }, wantErr: "options are for decision cases"},
		{name: "options on question", kind: KindQuestion, mutate: func(r *OpenRecord) { r.Options = []string{"x"} }, wantErr: "options are for decision cases, not question"},
		{name: "rows on question", kind: KindQuestion, mutate: func(r *OpenRecord) { r.Rows = openOf(KindApproval).Rows }, wantErr: "rows are for approval cases, not question"},
		{name: "rows on decision", kind: KindDecision, mutate: func(r *OpenRecord) { r.Rows = openOf(KindApproval).Rows }, wantErr: "rows are for approval cases"},
		{name: "approval without rows", kind: KindApproval, mutate: func(r *OpenRecord) { r.Rows = nil }, wantErr: "at least one row"},
		{name: "row id with =", kind: KindApproval, mutate: func(r *OpenRecord) { r.Rows[0].ID = "a=b" }, wantErr: "must be letters"},
		{name: "duplicate row id", kind: KindApproval, mutate: func(r *OpenRecord) { r.Rows[1].ID = "a" }, wantErr: "used twice"},
		{name: "row without script", kind: KindApproval, mutate: func(r *OpenRecord) { r.Rows[0].Script = "" }, wantErr: "script is empty"},
		{name: "row without link", kind: KindApproval, mutate: func(r *OpenRecord) { r.Rows[0].Link = "" }, wantErr: "link is empty"},
		{name: "row without label", kind: KindApproval, mutate: func(r *OpenRecord) { r.Rows[0].Label = "" }, wantErr: "label is empty"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			rec := openOf(tt.kind)
			if tt.mutate != nil {
				tt.mutate(&rec)
			}
			err := rec.validate()
			checkErr(t, err, tt.wantErr)
		})
	}
}

func TestAnswerValidation(t *testing.T) {
	tests := []struct {
		name    string
		kind    Kind
		answer  AnswerRecord
		wantErr string
	}{
		{name: "decision choice", kind: KindDecision, answer: AnswerRecord{Choice: 1}},
		{name: "decision last choice", kind: KindDecision, answer: AnswerRecord{Choice: 2, Note: "fine"}},
		{name: "decision other with note", kind: KindDecision, answer: AnswerRecord{Other: true, Note: "neither"}},
		{name: "decision other without note", kind: KindDecision, answer: AnswerRecord{Other: true}, wantErr: "other needs a note"},
		{name: "decision choice out of range", kind: KindDecision, answer: AnswerRecord{Choice: 3}, wantErr: "choice 3 is not an option (1-2)"},
		{name: "decision negative choice", kind: KindDecision, answer: AnswerRecord{Choice: -1}, wantErr: "is not an option"},
		{name: "decision choice and other", kind: KindDecision, answer: AnswerRecord{Choice: 1, Other: true, Note: "x"}, wantErr: "not both"},
		{name: "decision nothing", kind: KindDecision, answer: AnswerRecord{Note: "hm"}, wantErr: "no decision response"},
		{name: "decision with ack", kind: KindDecision, answer: AnswerRecord{Ack: true}, wantErr: "decision answers cannot set ack"},

		{name: "approval all rows", kind: KindApproval, answer: answerOf(KindApproval)},
		{name: "approval missing row", kind: KindApproval, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictReject}}}, wantErr: `row "b" has no verdict`},
		{name: "approval unknown row", kind: KindApproval, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictApprove}, {ID: "b", Verdict: VerdictApprove}, {ID: "c", Verdict: VerdictApprove}}}, wantErr: `row "c" is not on this case`},
		{name: "approval row twice", kind: KindApproval, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictApprove}, {ID: "a", Verdict: VerdictHold}}}, wantErr: "answered twice"},
		{name: "approval bad verdict", kind: KindApproval, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: "yes"}, {ID: "b", Verdict: VerdictApprove}}}, wantErr: `verdict "yes"`},
		{name: "approval with choice", kind: KindApproval, answer: AnswerRecord{Choice: 1}, wantErr: "approval answers cannot set choice"},

		{name: "signoff accept", kind: KindSignoff, answer: AnswerRecord{Signoff: SignoffAccept}},
		{name: "signoff changes with note", kind: KindSignoff, answer: AnswerRecord{Signoff: SignoffChanges, Note: "rename it"}},
		{name: "signoff changes without note", kind: KindSignoff, answer: AnswerRecord{Signoff: SignoffChanges}, wantErr: "needs a note"},
		{name: "signoff bad value", kind: KindSignoff, answer: AnswerRecord{Signoff: "ok"}, wantErr: `signoff "ok"`},

		{name: "stuck text", kind: KindStuck, answer: AnswerRecord{Text: "use v2"}},
		{name: "stuck drop", kind: KindStuck, answer: AnswerRecord{Drop: true}},
		{name: "stuck blank text", kind: KindStuck, answer: AnswerRecord{Text: "  "}, wantErr: "guidance text is empty"},
		{name: "stuck text and drop", kind: KindStuck, answer: AnswerRecord{Text: "x", Drop: true}, wantErr: "not both"},

		{name: "question text", kind: KindQuestion, answer: AnswerRecord{Text: "the staging one"}},
		{name: "question text with note", kind: KindQuestion, answer: AnswerRecord{Text: "the staging one", Note: "ask again if it moves"}},
		{name: "question nothing", kind: KindQuestion, answer: AnswerRecord{Note: "hm"}, wantErr: "no question response"},
		{name: "question blank text", kind: KindQuestion, answer: AnswerRecord{Text: " \n "}, wantErr: "reply text is empty"},
		{name: "question with drop", kind: KindQuestion, answer: AnswerRecord{Drop: true}, wantErr: "question answers cannot set drop"},
		{name: "question text and drop", kind: KindQuestion, answer: AnswerRecord{Text: "x", Drop: true}, wantErr: "question answers cannot set drop"},
		{name: "question with choice", kind: KindQuestion, answer: AnswerRecord{Text: "x", Choice: 1}, wantErr: "question answers cannot set choice"},
		{name: "question with other", kind: KindQuestion, answer: AnswerRecord{Text: "x", Other: true}, wantErr: "question answers cannot set other"},
		{name: "question with rows", kind: KindQuestion, answer: AnswerRecord{Text: "x", Rows: answerOf(KindApproval).Rows}, wantErr: "question answers cannot set rows"},
		{name: "question with signoff", kind: KindQuestion, answer: AnswerRecord{Text: "x", Signoff: SignoffAccept}, wantErr: "question answers cannot set signoff"},
		{name: "question with ack", kind: KindQuestion, answer: AnswerRecord{Text: "x", Ack: true}, wantErr: "question answers cannot set ack"},

		{name: "fyi ack", kind: KindFYI, answer: AnswerRecord{Ack: true, Note: "thanks"}},
		{name: "fyi nothing", kind: KindFYI, answer: AnswerRecord{}, wantErr: "no fyi response"},
		{name: "fyi with text", kind: KindFYI, answer: AnswerRecord{Ack: true, Text: "x"}, wantErr: "fyi answers cannot set text"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			open := openOf(tt.kind)
			checkErr(t, tt.answer.validate(&open), tt.wantErr)
		})
	}
}

func TestAmendValidation(t *testing.T) {
	row := func(id string) Row {
		return Row{ID: id, Label: "Deploy", Script: "make deploy", Link: "https://example.com/" + id}
	}
	tests := []struct {
		name    string
		kind    Kind
		amend   AmendRecord
		wantErr string
	}{
		{name: "body on stuck", kind: KindStuck, amend: AmendRecord{Body: "Now with the logs."}},
		{name: "context on signoff", kind: KindSignoff, amend: AmendRecord{Context: "PR #12"}},
		{name: "links on fyi", kind: KindFYI, amend: AmendRecord{Links: []string{"https://example.com/log"}}},
		{name: "options on decision", kind: KindDecision, amend: AmendRecord{Options: []string{"Vendor it"}}},
		{name: "rows on approval", kind: KindApproval, amend: AmendRecord{Rows: []Row{row("c"), row("d")}}},

		{name: "nothing", kind: KindFYI, amend: AmendRecord{}, wantErr: "the amend has no body, options, rows, links or context"},
		{name: "blank body", kind: KindFYI, amend: AmendRecord{Body: " \n", Links: []string{"https://example.com/log"}}, wantErr: "amend body is empty"},
		{name: "blank context", kind: KindFYI, amend: AmendRecord{Context: "  "}, wantErr: "amend context is empty"},
		{name: "options on approval", kind: KindApproval, amend: AmendRecord{Options: []string{"x"}}, wantErr: "options are for decision cases, not approval"},
		{name: "rows on decision", kind: KindDecision, amend: AmendRecord{Rows: []Row{row("c")}}, wantErr: "rows are for approval cases, not decision"},
		{name: "empty option", kind: KindDecision, amend: AmendRecord{Options: []string{"Vendor it", " "}}, wantErr: "option 2 is empty"},
		{name: "row id already on the case", kind: KindApproval, amend: AmendRecord{Rows: []Row{row("c"), row("b")}}, wantErr: `row 2: id "b" is already on the case`},
		{name: "row id twice in the amend", kind: KindApproval, amend: AmendRecord{Rows: []Row{row("c"), row("c")}}, wantErr: `row 2: id "c" is used twice`},
		{name: "row id with =", kind: KindApproval, amend: AmendRecord{Rows: []Row{row("c=d")}}, wantErr: "must be letters"},
		{name: "row without script", kind: KindApproval, amend: AmendRecord{Rows: []Row{{ID: "c", Label: "Deploy", Link: "https://example.com/c"}}}, wantErr: `row "c": script is empty`},
		{name: "empty link", kind: KindFYI, amend: AmendRecord{Links: []string{""}}, wantErr: "link 1 is empty"},
		{name: "blank link", kind: KindFYI, amend: AmendRecord{Links: []string{"https://example.com/log", "  "}}, wantErr: "link 2 is empty"},

		// An amend sent again, or one that repeats the case, adds nothing.
		{name: "option already on the case", kind: KindDecision, amend: AmendRecord{Options: []string{"Vendor it", "Pin"}}, wantErr: `option "Pin" is already on the case`},
		{name: "option twice in the amend", kind: KindDecision, amend: AmendRecord{Options: []string{"Vendor it", "Vendor it"}}, wantErr: `option "Vendor it" is used twice`},
		{name: "link already on the case", kind: KindFYI, amend: AmendRecord{Links: []string{"https://example.com/pr"}}, wantErr: `link "https://example.com/pr" is already on the case`},
		{name: "link twice in the amend", kind: KindFYI, amend: AmendRecord{Links: []string{"https://example.com/log", "https://example.com/log"}}, wantErr: `link "https://example.com/log" is used twice`},
		{name: "the body the case has", kind: KindFYI, amend: AmendRecord{Body: "Body."}, wantErr: "the amend changes nothing"},
		{name: "the body and context the case has", kind: KindSignoff, amend: AmendRecord{Body: "Body.", Context: "Release 1.4"}, wantErr: "the amend changes nothing"},
		{name: "the body the case has, and a link", kind: KindFYI, amend: AmendRecord{Body: "Body.", Links: []string{"https://example.com/log"}}},
		{name: "the context the case has, and a new body", kind: KindStuck, amend: AmendRecord{Body: "Now with the logs.", Context: "Release 1.4"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			open := openOf(tt.kind)
			open.Links = []string{"https://example.com/pr"}
			open.Context = "Release 1.4"
			checkErr(t, tt.amend.validate(&open), tt.wantErr)
		})
	}
}

func TestAmendChangesTheCase(t *testing.T) {
	open := openOf(KindApproval)
	open.Links = []string{"https://example.com/pr"}
	open.Context = "Release 1.4"
	added := Row{ID: "c", Label: "Deploy", Script: "make deploy", Link: "https://example.com/c"}
	c, _, err := fold(t,
		agent(EventOpen, open),
		agent(EventAmend, AmendRecord{
			Body:    "Three scripts now.",
			Rows:    []Row{added},
			Links:   []string{"https://example.com/log"},
			Context: "Release 1.4.1",
		}),
		// An amend that sets one field leaves the others as they are.
		agent(EventAmend, AmendRecord{Links: []string{"https://example.com/diff"}}),
	)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(c.Rows, append(openOf(KindApproval).Rows, added)) || c.Body != "Three scripts now." || c.Context != "Release 1.4.1" ||
		!slices.Equal(c.Links, []string{"https://example.com/pr", "https://example.com/log", "https://example.com/diff"}) {
		t.Errorf("case = %+v", c.OpenRecord)
	}
	if c.State != StateOpen || len(c.Events) != 3 || c.Events[1].Type != EventAmend {
		t.Errorf("state %s, events %+v", c.State, c.Events)
	}
	// The open event is kept as it was written.
	var first OpenRecord
	if err := json.Unmarshal(c.Events[0].Data, &first); err != nil {
		t.Fatal(err)
	}
	if first.Body != "Body." || len(first.Rows) != 2 || len(first.Links) != 1 || first.Context != "Release 1.4" {
		t.Errorf("open event = %+v", first)
	}

	// A refused amend changes nothing, not even the fields that were valid. Its
	// row id was added by the first amend, so it is checked against the case
	// as amended, not as opened.
	before := c.OpenRecord
	err = c.apply(Event{Seq: 4, Author: AuthorAgent, Type: EventAmend, Data: []byte(`{"body":"Four scripts.","links":["https://example.com/x"],"rows":[{"id":"c","label":"Again","script":"true","link":"https://example.com/c"}]}`)})
	if err == nil || !strings.Contains(err.Error(), "already on the case") {
		t.Fatalf("err = %v", err)
	}
	if c.Body != before.Body || len(c.Rows) != 3 || len(c.Links) != 3 || len(c.Events) != 3 {
		t.Errorf("refused amend changed the case: %+v", c.OpenRecord)
	}
}

// The answer is checked against the case as amended, not as it was opened.
func TestAnswerIsCheckedAgainstTheAmendedCase(t *testing.T) {
	approval := openOf(KindApproval)
	approval.Rows = approval.Rows[:1]
	amended := []step{
		agent(EventOpen, approval),
		agent(EventAmend, AmendRecord{Rows: []Row{{ID: "b", Label: "Migrate", Script: "make migrate", Link: "https://example.com/b"}}}),
	}
	decision := []step{
		agent(EventOpen, openOf(KindDecision)),
		agent(EventAmend, AmendRecord{Options: []string{"Vendor it"}}),
	}
	tests := []struct {
		name    string
		steps   []step
		answer  AnswerRecord
		wantErr string
	}{
		{name: "every row, the added one too", steps: amended, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictApprove}, {ID: "b", Verdict: VerdictHold}}}},
		{name: "only the rows it opened with", steps: amended, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictApprove}}}, wantErr: `row "b" has no verdict`},
		{name: "a row that is not on the case", steps: amended, answer: AnswerRecord{Rows: []RowAnswer{{ID: "a", Verdict: VerdictApprove}, {ID: "b", Verdict: VerdictApprove}, {ID: "c", Verdict: VerdictApprove}}}, wantErr: `row "c" is not on this case`},
		{name: "the added option", steps: decision, answer: AnswerRecord{Choice: 3}},
		{name: "past the added option", steps: decision, answer: AnswerRecord{Choice: 4}, wantErr: "choice 4 is not an option (1-3)"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			c, _, err := fold(t, append(slices.Clone(tt.steps), human(EventAnswer, tt.answer))...)
			checkErr(t, err, tt.wantErr)
			if tt.wantErr == "" && c.State != StateAnswered {
				t.Errorf("state = %s", c.State)
			}
		})
	}
}

// Only an amend makes an answer's number matter. A case with no amend takes an
// answer numbered 0000, as a hand-numbered store can have.
func TestAnswerNumberIsOnlyCheckedAfterAnAmend(t *testing.T) {
	open, err := json.Marshal(openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	c := &Case{}
	for _, ev := range []Event{
		{Seq: 0, Author: AuthorAgent, Type: EventOpen, Data: open},
		{Seq: 0, Author: AuthorHuman, Type: EventAnswer, Data: []byte(`{"ack":true}`)},
	} {
		if err := c.apply(ev); err != nil {
			t.Fatalf("%s: %v", ev.Type, err)
		}
	}
	if c.State != StateAnswered {
		t.Errorf("state = %s", c.State)
	}
}

func checkErr(t *testing.T, err error, want string) {
	t.Helper()
	if want == "" {
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		return
	}
	if err == nil || !strings.Contains(err.Error(), want) {
		t.Fatalf("err = %v, want it to contain %q", err, want)
	}
}
