package store

import (
	"encoding/json"
	"slices"
	"testing"
)

func TestActorAndForFold(t *testing.T) {
	ryan := &Actor{Name: "Ryan", Kind: "human"}
	open := openOf(KindDecision)
	open.For = "Ryan"
	open.Actor = &Actor{Name: "bun-pins", Kind: "agent"}
	answer := answerOf(KindDecision)
	answer.Actor = ryan

	c, i, err := fold(t, agent(EventOpen, open), human(EventAnswer, answer), agent(EventPickup, PickupRecord{}))
	if err != nil {
		t.Fatalf("step %d: %v", i, err)
	}
	if c.For != "Ryan" {
		t.Errorf("For = %q", c.For)
	}
	if got := c.Events[0].Actor; got == nil || *got != *open.Actor {
		t.Errorf("open actor = %+v", got)
	}
	if got := c.Events[1].Actor; got == nil || *got != *ryan || c.Answer.Actor == nil || *c.Answer.Actor != *ryan {
		t.Errorf("answer actor = %+v, case answer actor = %+v", got, c.Answer.Actor)
	}
	if c.Events[2].Actor != nil {
		t.Errorf("pickup without an actor has one: %+v", c.Events[2].Actor)
	}
}

func TestActorAndForValidation(t *testing.T) {
	blankFor := openOf(KindFYI)
	blankFor.For = "  "
	_, _, err := fold(t, agent(EventOpen, blankFor))
	checkErr(t, err, "for is empty")

	for _, tc := range []struct {
		actor *Actor
		want  string
	}{
		{&Actor{Name: " ", Kind: "human"}, "actor name is empty"},
		{&Actor{Name: "Ryan"}, "actor kind is empty"},
	} {
		_, _, err := fold(t, agent(EventOpen, openOf(KindFYI)), human(EventAnswer, AnswerRecord{Ack: true, Actor: tc.actor}))
		checkErr(t, err, tc.want)
	}
}

// A later cases may add fields to an actor or record kinds this one does not
// know; the event still folds, and Data keeps what was written.
func TestActorFromALaterBuildFolds(t *testing.T) {
	c, _, err := fold(t, agent(EventOpen, openOf(KindFYI)))
	if err != nil {
		t.Fatal(err)
	}
	data := []byte(`{"ack":true,"actor":{"id":"u_123","name":"Ryan","kind":"member"},"answered_at":"2026-09-17T10:00:00Z"}`)
	if err := c.apply(Event{Seq: 2, Author: AuthorHuman, Type: EventAnswer, Data: data}); err != nil {
		t.Fatal(err)
	}
	if got := c.Events[1].Actor; got == nil || got.Name != "Ryan" || got.Kind != "member" {
		t.Errorf("actor = %+v", got)
	}
	if !json.Valid(c.Events[1].Data) || string(c.Events[1].Data) != string(data) {
		t.Errorf("Data = %s", c.Events[1].Data)
	}
}

// A build from before actor and for decodes the records without them: the
// same bytes fold on a record type that lacks the fields.
func TestActorAndForAreIgnoredByAnEarlierBuild(t *testing.T) {
	type earlierOpen struct {
		Kind    Kind     `json:"kind"`
		Urgency Urgency  `json:"urgency"`
		Title   string   `json:"title"`
		Options []string `json:"options,omitempty"`
	}
	open := openOf(KindDecision)
	open.For = "Ryan"
	open.Actor = &Actor{Name: "bun-pins", Kind: "agent"}
	data, err := json.Marshal(open)
	if err != nil {
		t.Fatal(err)
	}
	var old earlierOpen
	if err := json.Unmarshal(data, &old); err != nil {
		t.Fatalf("an earlier build cannot decode the open record: %v", err)
	}
	if old.Title != open.Title || !slices.Equal(old.Options, open.Options) {
		t.Errorf("earlier decode = %+v", old)
	}
}

func TestDescribeActor(t *testing.T) {
	ryan := &Actor{Name: "Ryan", Kind: "human"}
	pins := &Actor{Name: "bun-pins", Kind: "agent"}
	c, _, err := fold(t,
		agent(EventOpen, openOf(KindDecision)),
		human(EventAnswer, AnswerRecord{Choice: 1, Actor: ryan}),
		agent(EventPickup, PickupRecord{By: "bun-pins", Actor: pins}),
		agent(EventNote, NoteRecord{Body: "Which patch?", Actor: pins}),
		human(EventAnswer, AnswerRecord{Choice: 2}),
		agent(EventPickup, PickupRecord{By: "session-2", Actor: pins}),
	)
	if err != nil {
		t.Fatal(err)
	}
	for i, want := range [][]Line{
		nil,
		{{Text: "by Ryan"}, {Text: "chose 1. Pin"}},
		{{Text: "by bun-pins"}},
		{{Text: "by bun-pins"}, {Text: "Which patch?"}},
		{{Text: "chose 2. Float"}},
		{{Text: "by bun-pins"}, {Text: "by session-2"}},
	} {
		if got := c.Describe(c.Events[i]); !slices.Equal(got, want) {
			t.Errorf("Describe(event %d) = %q, want %q", i+1, got, want)
		}
	}
}
