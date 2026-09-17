package main

import (
	"encoding/json"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/instance"
	"github.com/ryanlewis/cases/internal/store/storetest"
)

func TestShow(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	mustRun(t, "--store", storePath, "answer", id, "--option", "2", "--note", "watch the lockfile")

	out := mustRun(t, "--store", storePath, "show", id)
	for _, want := range []string{"Pin bun?", "state:    answered", "1. Pin to 1.2.3", "other. Other, see note", "revision: 2", "0002 human answer", "chose 2. Float, with renovate", "note: watch the lockfile"} {
		if !strings.Contains(out, want) {
			t.Errorf("show output missing %q:\n%s", want, out)
		}
	}

	var c struct {
		State  string `json:"state"`
		Answer struct {
			Choice int `json:"choice"`
		} `json:"answer"`
		Events []struct {
			File string          `json:"file"`
			Data json.RawMessage `json:"data"`
		} `json:"events"`
	}
	if err := json.Unmarshal([]byte(mustRun(t, "--store", storePath, "show", id, "--json")), &c); err != nil {
		t.Fatal(err)
	}
	if c.State != "answered" || c.Answer.Choice != 2 || len(c.Events) != 2 || c.Events[1].File != "0002-human-answer.json" {
		t.Errorf("json = %+v", c)
	}
}

func TestShowQuestionReply(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "question", "--urgency", "today", "--title", "Which host?"))
	mustRun(t, "--store", storePath, "answer", id, "--text", "the staging one")

	out := mustRun(t, "--store", storePath, "show", id)
	for _, want := range []string{"kind:     question", "0002 human answer", "reply: the staging one"} {
		if !strings.Contains(out, want) {
			t.Errorf("show output missing %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, "guidance:") {
		t.Errorf("question reply described as guidance:\n%s", out)
	}
}

func TestShowRefusesPathsAndMissingCases(t *testing.T) {
	storePath := newStore(t)
	for _, id := range []string{"../etc", "nope"} {
		if r := runCases(t, "", "--store", storePath, "show", id); r.err == nil {
			t.Errorf("show %q succeeded", id)
		}
	}
}

func TestShowJSONRevisionCountsSkippedFiles(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	// A malformed event is skipped by the fold but still counts: the next
	// write goes after it.
	storetest.InsertEvent(t, storePath, id, 2, "human", "answer", `{"choice": 2`)

	r := runCases(t, "", "--store", storePath, "show", id, "--json")
	if r.err != nil {
		t.Fatal(r.err)
	}
	var c struct {
		Revision *int              `json:"revision"`
		Events   []json.RawMessage `json:"events"`
		Problems []string          `json:"problems"`
	}
	if err := json.Unmarshal([]byte(r.stdout), &c); err != nil {
		t.Fatal(err)
	}
	if c.Revision == nil || *c.Revision != 2 || len(c.Events) != 1 || len(c.Problems) != 1 {
		t.Fatalf("revision %v, %d events, problems %q:\n%s", c.Revision, len(c.Events), c.Problems, r.stdout)
	}
	if n := len(loadCase(t, storePath, id).Events); n != 1 {
		t.Errorf("%d events folded, want the open event only", n)
	}
	mustRun(t, "--store", storePath, "answer", id, "--option", "1", "--revision", "2")
}

func TestShowTakesPartOfAnID(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	if out := mustRun(t, "--store", storePath, "show", "pin-bun"); !strings.Contains(out, "id:       "+id+"\n") {
		t.Errorf("show pin-bun:\n%s", out)
	}
}

func TestShowReportsAnUnknownEventAsVersionSkew(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	storetest.InsertEvent(t, storePath, id, 2, "agent", "comment", `{"body":"x"}`)
	r := runCases(t, "", "--store", storePath, "show", id)
	if r.err != nil {
		t.Fatal(r.err)
	}
	const problem = `0002-agent-comment.json: unknown event "comment": perhaps written by a newer cases, or not by cases at all; if newer, update cases on this machine with go install github.com/ryanlewis/cases/cmd/cases@latest`
	if !strings.Contains(r.stdout, "  problem: "+problem+"\n") {
		t.Errorf("stdout missing the problem:\n%s", r.stdout)
	}
	if r.stderr != "warning: "+id+": "+problem+"\n" {
		t.Errorf("stderr = %q", r.stderr)
	}
}

func TestShowURLOfTheRunningInbox(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	showURL := func() string {
		t.Helper()
		var c struct {
			URL *string `json:"url"`
		}
		if err := json.Unmarshal([]byte(mustRun(t, "--store", storePath, "show", id, "--json")), &c); err != nil {
			t.Fatal(err)
		}
		if c.URL == nil {
			t.Fatal("show --json has no url field")
		}
		return *c.URL
	}

	if got := showURL(); got != "" {
		t.Errorf("url with no inbox running = %q, want empty", got)
	}
	if out := mustRun(t, "--store", storePath, "show", id); strings.Contains(out, "url:") {
		t.Errorf("url line with no inbox running:\n%s", out)
	}

	base, stop := startServe(t, storePath)
	want := base + "cases/" + id
	if got := showURL(); got != want {
		t.Errorf("url = %q, want %q", got, want)
	}
	if out := mustRun(t, "--store", storePath, "show", id); !strings.Contains(out, "url:      "+want+"\n") {
		t.Errorf("show missing the url line %q:\n%s", want, out)
	}
	if err := stop(); err != nil {
		t.Fatal(err)
	}
	if got := showURL(); got != "" {
		t.Errorf("url after the inbox stopped = %q, want empty", got)
	}
}

func TestShowWarnsOnADamagedInstanceFile(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	path, err := instance.Path(storePath)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("{not json"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := runCases(t, "", "--store", storePath, "show", id)
	if r.err != nil || !strings.Contains(r.stdout, "id:       "+id) || strings.Contains(r.stdout, "url:") {
		t.Errorf("show with a damaged instance file: err %v\n%s", r.err, r.stdout)
	}
	if !strings.HasPrefix(r.stderr, "warning: read "+path+": ") {
		t.Errorf("stderr = %q", r.stderr)
	}
}

func TestShowAnswer(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	shown := func() map[string]json.RawMessage {
		t.Helper()
		var m map[string]json.RawMessage
		if err := json.Unmarshal([]byte(mustRun(t, "--store", storePath, "show", id, "--answer")), &m); err != nil {
			t.Fatal(err)
		}
		keys := slices.Sorted(maps.Keys(m))
		if !slices.Equal(keys, []string{"answer", "kind", "revision", "state"}) {
			t.Errorf("keys = %v", keys)
		}
		return m
	}

	m := shown()
	if string(m["state"]) != `"open"` || string(m["kind"]) != `"decision"` || string(m["revision"]) != "1" || string(m["answer"]) != "null" {
		t.Errorf("open case: %s", m)
	}

	mustRun(t, "--store", storePath, "answer", id, "--option", "2", "--note", "watch the lockfile")
	m = shown()
	var answer struct {
		Choice int    `json:"choice"`
		Note   string `json:"note"`
	}
	if err := json.Unmarshal(m["answer"], &answer); err != nil {
		t.Fatal(err)
	}
	if string(m["state"]) != `"answered"` || string(m["revision"]) != "2" || answer.Choice != 2 || answer.Note != "watch the lockfile" {
		t.Errorf("answered case: %s", m)
	}

	mustRun(t, "--store", storePath, "pickup", id)
	if r := runCases(t, "Which lockfile?", "--store", storePath, "note", id, "--body-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}
	m = shown()
	if string(m["state"]) != `"open"` || string(m["revision"]) != "4" || string(m["answer"]) != "null" {
		t.Errorf("case reopened by a note: %s", m)
	}

	if r := runCases(t, "", "--store", storePath, "show", id, "--answer", "--json"); r.err == nil {
		t.Error("show took --answer with --json")
	}
}
