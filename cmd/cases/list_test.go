package main

import (
	"encoding/json"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
	"github.com/ryanlewis/cases/internal/store/storetest"
)

func TestListOrderFilterAndJSON(t *testing.T) {
	storePath := newStore(t)
	later := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "Later"))
	urgent := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "blocking", "--title", "Urgent"))
	answered := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Seen"))
	mustRun(t, "--store", storePath, "answer", answered, "--ack")

	out := mustRun(t, "--store", storePath, "list", "--all")
	iu, ia, il := strings.Index(out, urgent), strings.Index(out, answered), strings.Index(out, later)
	if iu < 0 || ia < 0 || il < 0 || iu >= ia || ia >= il {
		t.Errorf("list is not blocking, today, whenever:\n%s", out)
	}

	out = mustRun(t, "--store", storePath, "list", "--state", "answered", "--json")
	var got []map[string]any
	if err := json.Unmarshal([]byte(out), &got); err != nil {
		t.Fatalf("json: %v\n%s", err, out)
	}
	if len(got) != 1 || got[0]["id"] != answered || got[0]["state"] != "answered" {
		t.Errorf("filtered = %v", got)
	}

	if out := mustRun(t, "--store", storePath, "list", "--state", "closed,withdrawn", "--json"); strings.TrimSpace(out) != "[]" {
		t.Errorf("empty filter = %q, want []", out)
	}
	if r := runCases(t, "", "--store", storePath, "list", "--state", "done"); r.err == nil {
		t.Error("unknown state accepted")
	}
}

func TestListShowsLabels(t *testing.T) {
	storePath := newStore(t)
	labelled := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "blocking", "--title", "Labelled", "--label", "round-1", "--label", "docs"))
	plain := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Plain"))

	lines := strings.Split(strings.TrimRight(mustRun(t, "--store", storePath, "list"), "\n"), "\n")
	if len(lines) != 3 {
		t.Fatalf("list has %d lines, want 3:\n%s", len(lines), strings.Join(lines, "\n"))
	}
	// Every row starts its labels and title where the header does.
	labelsAt, titleAt := strings.Index(lines[0], "LABELS"), strings.Index(lines[0], "TITLE")
	for _, tt := range []struct {
		line, id, labels, title string
	}{
		{lines[1], labelled, "round-1,docs", "Labelled"},
		{lines[2], plain, "", "Plain"},
	} {
		if !strings.HasPrefix(tt.line, tt.id) {
			t.Errorf("row %q is not case %s", tt.line, tt.id)
		}
		if got := strings.TrimSpace(tt.line[labelsAt:titleAt]); got != tt.labels {
			t.Errorf("labels = %q, want %q in %q", got, tt.labels, tt.line)
		}
		if got := tt.line[titleAt:]; got != tt.title {
			t.Errorf("title = %q, want %q in %q", got, tt.title, tt.line)
		}
	}
}

func TestListByLabelAndWorker(t *testing.T) {
	storePath := newStore(t)
	open := func(title string, args ...string) string {
		return strings.TrimSpace(mustRun(t, append([]string{"--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", title}, args...)...))
	}
	a := open("A", "--label", "round-1", "--worker", "w1")
	b := open("B", "--label", "round-1", "--label", "docs", "--worker", "w2")
	c := open("C", "--label", "round-2", "--worker", "w1")
	d := open("D")

	ids := func(args ...string) []string {
		t.Helper()
		var got []struct {
			ID string `json:"id"`
		}
		if err := json.Unmarshal([]byte(mustRun(t, append([]string{"--store", storePath, "list", "--json"}, args...)...)), &got); err != nil {
			t.Fatal(err)
		}
		var out []string
		for _, g := range got {
			out = append(out, g.ID)
		}
		slices.Sort(out)
		return out
	}
	for _, tt := range []struct {
		args []string
		want []string
	}{
		{nil, []string{a, b, c, d}},
		{[]string{"--label", "round-1"}, []string{a, b}},
		{[]string{"--label", "docs", "--label", "round-2"}, []string{b, c}},
		{[]string{"--worker", "w1"}, []string{a, c}},
		{[]string{"--worker", "w1", "--worker", "w2"}, []string{a, b, c}},
		{[]string{"--label", "round-1", "--worker", "w1"}, []string{a}},
		{[]string{"--label", "docs", "--worker", "w1"}, nil},
		{[]string{"--label", "nope"}, nil},
	} {
		if got := ids(tt.args...); !slices.Equal(got, tt.want) {
			t.Errorf("list %q = %q, want %q", tt.args, got, tt.want)
		}
	}
}

func TestListReportsBrokenCaseAndListsTheRest(t *testing.T) {
	storePath := newStore(t)
	good := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Good"))
	const broken = "2026-01-01T00-00-00Z-broken"
	storetest.InsertEvent(t, storePath, broken, 1, "agent", "open", "{")
	r := runCases(t, "", "--store", storePath, "list")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if !strings.Contains(r.stdout, good) || !strings.Contains(r.stderr, "warning: "+broken+": no valid open event") {
		t.Errorf("stdout %q\nstderr %q", r.stdout, r.stderr)
	}
}

// listIDs runs list --json with args and returns the ids it printed, sorted.
func listIDs(t *testing.T, storePath string, args ...string) []string {
	t.Helper()
	var got []struct {
		ID string `json:"id"`
	}
	if err := json.Unmarshal([]byte(mustRun(t, append([]string{"--store", storePath, "list", "--json"}, args...)...)), &got); err != nil {
		t.Fatal(err)
	}
	var out []string
	for _, g := range got {
		out = append(out, g.ID)
	}
	slices.Sort(out)
	return out
}

func sorted(ids ...string) []string {
	slices.Sort(ids)
	return ids
}

func TestListDefaultsToOpenAndParked(t *testing.T) {
	storePath := newStore(t)
	open := func(title string) string {
		return strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "stuck", "--urgency", "today", "--title", title))
	}
	opened := open("Open")
	parked := open("Parked")
	mustRun(t, "--store", storePath, "answer", parked, "--park")
	answered := open("Answered")
	mustRun(t, "--store", storePath, "answer", answered, "--text", "Try again")
	withdrawn := open("Withdrawn")
	mustRun(t, "--store", storePath, "withdraw", withdrawn)

	for _, tt := range []struct {
		args []string
		want []string
	}{
		{nil, sorted(opened, parked)},
		{[]string{"--all"}, sorted(opened, parked, answered, withdrawn)},
		{[]string{"--state", "withdrawn"}, []string{withdrawn}},
		{[]string{"--state", "answered", "--all"}, []string{answered}},
		{[]string{"--state", "closed"}, nil},
	} {
		if got := listIDs(t, storePath, tt.args...); !slices.Equal(got, tt.want) {
			t.Errorf("list %q = %q, want %q", tt.args, got, tt.want)
		}
	}

	out := mustRun(t, "--store", storePath, "list")
	if strings.Contains(out, answered) || !strings.Contains(out, opened) || !strings.Contains(out, parked) {
		t.Errorf("list table:\n%s", out)
	}
}

func TestListByKindAndUrgency(t *testing.T) {
	storePath := newStore(t)
	open := func(kind, urgency string) string {
		args := []string{"--store", storePath, "open", "--kind", kind, "--urgency", urgency, "--title", kind + " " + urgency}
		if kind == "decision" {
			args = append(args, "--option", "Yes", "--option", "No")
		}
		return strings.TrimSpace(mustRun(t, args...))
	}
	fyiToday := open("fyi", "today")
	fyiBlocking := open("fyi", "blocking")
	decision := open("decision", "blocking")
	question := open("question", "whenever")

	for _, tt := range []struct {
		args []string
		want []string
	}{
		{[]string{"--kind", "fyi"}, sorted(fyiToday, fyiBlocking)},
		{[]string{"--kind", "decision", "--kind", "question"}, sorted(decision, question)},
		{[]string{"--kind", "stuck"}, nil},
		{[]string{"--urgency", "blocking"}, sorted(fyiBlocking, decision)},
		{[]string{"--urgency", "today", "--urgency", "whenever"}, sorted(fyiToday, question)},
		{[]string{"--kind", "fyi", "--urgency", "blocking"}, []string{fyiBlocking}},
	} {
		if got := listIDs(t, storePath, tt.args...); !slices.Equal(got, tt.want) {
			t.Errorf("list %q = %q, want %q", tt.args, got, tt.want)
		}
	}

	// A typo is refused rather than matching nothing, on every command that
	// shares the filter.
	for _, args := range [][]string{
		{"list", "--kind", "decison"},
		{"list", "--kind", "fyi,decision"},
		{"list", "--urgency", "urgent"},
		{"sweep", "--kind", "decison"},
		{"wait", "--kind", "decison", "--timeout", "1ms"},
	} {
		if r := runCases(t, "", append([]string{"--store", storePath}, args...)...); r.err == nil || !strings.Contains(r.err.Error(), "must be one of") {
			t.Errorf("%q: err = %v, want an enum error", args, r.err)
		}
	}
}

func TestWaitByKind(t *testing.T) {
	storePath := newStore(t)
	notice := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Notice"))
	decision := openDecision(t, storePath)

	ch := startWait(t, storePath, "--kind", "decision", "--timeout", "5s")
	mustRun(t, "--store", storePath, "answer", notice, "--ack")
	time.Sleep(60 * time.Millisecond)
	mustRun(t, "--store", storePath, "answer", decision, "--option", "1")
	if got := waited(t, <-ch); len(got) != 1 || got[0].ID != decision {
		t.Errorf("wait --kind decision printed %+v, want only %s", got, decision)
	}
}

func TestListCount(t *testing.T) {
	storePath := newStore(t)
	for _, args := range [][]string{{"list", "--count"}, {"list", "--count", "--json"}} {
		if r := runCases(t, "", append([]string{"--store", filepath.Join(storePath, "missing")}, args...)...); r.err != nil || r.stdout != "0\n" {
			t.Errorf("%q on a missing store = %q, %v, want 0", args, r.stdout, r.err)
		}
	}

	mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "blocking", "--title", "A")
	mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "blocking", "--title", "B")
	answered := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "C"))
	mustRun(t, "--store", storePath, "answer", answered, "--ack")

	for _, tt := range []struct {
		args []string
		want string
	}{
		{nil, "2\n"},
		{[]string{"--all"}, "3\n"},
		{[]string{"--urgency", "blocking"}, "2\n"},
		{[]string{"--urgency", "whenever"}, "0\n"},
		{[]string{"--json"}, "2\n"},
	} {
		if out := mustRun(t, append([]string{"--store", storePath, "list", "--count"}, tt.args...)...); out != tt.want {
			t.Errorf("list --count %q = %q, want %q", tt.args, out, tt.want)
		}
	}
}

func TestListOlderThanReadsTheLastEvent(t *testing.T) {
	storePath := newStore(t)
	db := openStore(t, storePath)
	now := time.Now().UTC()
	stuck := func(title string) *store.Case {
		c, err := db.Create(t.Context(), store.OpenRecord{Kind: store.KindStuck, Urgency: store.UrgencyToday, Title: title, OpenedAt: now.Add(-48 * time.Hour)})
		if err != nil {
			t.Fatal(err)
		}
		return c
	}
	old := openAt(t, storePath, "Old", now.Add(-48*time.Hour), store.OpenRecord{})
	recent := openAt(t, storePath, "Recent", now.Add(-time.Minute), store.OpenRecord{})
	// Opened and parked long ago, resumed a minute ago: it has not waited 30m.
	resumed := stuck("Resumed")
	if _, err := db.Park(t.Context(), resumed.ID, store.ParkRecord{ParkedAt: now.Add(-47 * time.Hour)}); err != nil {
		t.Fatal(err)
	}
	if _, err := db.Resume(t.Context(), resumed.ID, store.AuthorHuman, store.ResumeRecord{ResumedAt: now.Add(-time.Minute)}); err != nil {
		t.Fatal(err)
	}
	// Parked long ago and left parked: it has.
	parked := stuck("Parked")
	if _, err := db.Park(t.Context(), parked.ID, store.ParkRecord{ParkedAt: now.Add(-47 * time.Hour)}); err != nil {
		t.Fatal(err)
	}

	for _, tt := range []struct {
		args []string
		want []string
	}{
		{nil, sorted(old.ID, recent.ID, resumed.ID, parked.ID)},
		{[]string{"--older-than", "0"}, sorted(old.ID, recent.ID, resumed.ID, parked.ID)},
		{[]string{"--older-than", "30m"}, sorted(old.ID, parked.ID)},
		{[]string{"--older-than", "30m", "--state", "open"}, []string{old.ID}},
		{[]string{"--older-than", "72h"}, nil},
	} {
		if got := listIDs(t, storePath, tt.args...); !slices.Equal(got, tt.want) {
			t.Errorf("list %q = %q, want %q", tt.args, got, tt.want)
		}
	}
	if r := runCases(t, "", "--store", storePath, "list", "--older-than", "soon"); r.err == nil {
		t.Error("a bad duration was accepted")
	}
}
