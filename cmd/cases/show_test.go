package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestShow(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "2", "--note", "watch the lockfile")

	out := mustRun(t, "--store", root, "show", id)
	for _, want := range []string{"Pin bun?", "state:    answered", "1. Pin to 1.2.3", "other. Other, see note", "0002 human answer", "chose 2. Float, with renovate", "note: watch the lockfile"} {
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
	if err := json.Unmarshal([]byte(mustRun(t, "--store", root, "show", id, "--json")), &c); err != nil {
		t.Fatal(err)
	}
	if c.State != "answered" || c.Answer.Choice != 2 || len(c.Events) != 2 || c.Events[1].File != "0002-human-answer.json" {
		t.Errorf("json = %+v", c)
	}
}

func TestShowQuestionReply(t *testing.T) {
	root := t.TempDir()
	id := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "question", "--urgency", "today", "--title", "Which host?"))
	mustRun(t, "--store", root, "answer", id, "--text", "the staging one")

	out := mustRun(t, "--store", root, "show", id)
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
	root := t.TempDir()
	for _, id := range []string{"../etc", "nope"} {
		if r := runCases(t, "", "--store", root, "show", id); r.err == nil {
			t.Errorf("show %q succeeded", id)
		}
	}
}
