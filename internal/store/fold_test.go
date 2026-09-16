package store

import (
	"encoding/json"
	"errors"
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
