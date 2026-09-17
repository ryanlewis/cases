package main

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

// loadCase folds the case id in the store at storePath.
func loadCase(t *testing.T, storePath, id string) *store.Case {
	t.Helper()
	c, err := openStore(t, storePath).Get(t.Context(), id)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestOpenDecisionKeepsCommasInOptions(t *testing.T) {
	storePath := newStore(t)
	id := openDecision(t, storePath)
	c := loadCase(t, storePath, id)
	if want := []string{"Pin to 1.2.3", "Float, with renovate"}; !slices.Equal(c.Options, want) {
		t.Errorf("options = %q, want %q", c.Options, want)
	}
	if !strings.HasSuffix(id, "-pin-bun") || c.State != store.StateOpen {
		t.Errorf("id = %s, state = %s", id, c.State)
	}
}

func TestOpenBody(t *testing.T) {
	storePath := newStore(t)
	file := filepath.Join(t.TempDir(), "body.md")
	if err := os.WriteFile(file, []byte("# From a file\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	fromFile := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "File", "--body-file", file))
	if got := loadCase(t, storePath, fromFile).Body; got != "# From a file\n" {
		t.Errorf("body = %q", got)
	}

	r := runCases(t, "from stdin", "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Stdin", "--body-file", "-",
		"--worker", "bun-pins", "--brief", "/briefs/bun.md", "--link", "https://a,b", "--context", "ctx")
	if r.err != nil {
		t.Fatal(r.err)
	}
	c := loadCase(t, storePath, strings.TrimSpace(r.stdout))
	if c.Body != "from stdin" || c.Worker != "bun-pins" || c.Brief != "/briefs/bun.md" || c.Context != "ctx" || !slices.Equal(c.Links, []string{"https://a,b"}) {
		t.Errorf("case = %+v", c.OpenRecord)
	}
}

func TestOpenInlineBody(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Inline", "--body", "Bun is pinned."))
	if got := loadCase(t, storePath, id).Body; got != "Bun is pinned." {
		t.Errorf("body = %q", got)
	}
}

func TestOpenApprovalRows(t *testing.T) {
	storePath := newStore(t)
	out := mustRun(t, "--store", storePath, "open", "--kind", "approval", "--urgency", "blocking", "--title", "Scripts",
		"--row", `{"id":"deps","label":"Install deps","script":"npm ci --ignore-scripts\nnpm test","link":"https://example.com/deps"}`,
		"--row", `{"id":"mig","label":"Migrate","note":"takes the site down for a minute","script":"make migrate","link":"https://example.com/mig"}`)
	id := strings.TrimSpace(out)
	c := loadCase(t, storePath, id)
	if len(c.Rows) != 2 || c.Rows[0].Script != "npm ci --ignore-scripts\nnpm test" || c.Rows[1].ID != "mig" {
		t.Errorf("rows = %+v", c.Rows)
	}

	var shown struct {
		Rows []struct {
			ID   string  `json:"id"`
			Note *string `json:"note"`
		} `json:"rows"`
	}
	if err := json.Unmarshal([]byte(mustRun(t, "--store", storePath, "show", id, "--json")), &shown); err != nil {
		t.Fatal(err)
	}
	// A row without a note is written as before, with no note key.
	if len(shown.Rows) != 2 || shown.Rows[0].Note != nil || shown.Rows[1].Note == nil || *shown.Rows[1].Note != "takes the site down for a minute" {
		t.Errorf("show --json rows = %+v", shown.Rows)
	}
	if text := mustRun(t, "--store", storePath, "show", id); !strings.Contains(text, "  [mig] Migrate\n      note: takes the site down for a minute\n      https://example.com/mig\n") ||
		strings.Count(text, "note:") != 1 {
		t.Errorf("show output:\n%s", text)
	}
}

func TestOpenLabels(t *testing.T) {
	storePath := newStore(t)
	id := strings.TrimSpace(mustRun(t, "--store", storePath, "open", "--kind", "fyi", "--urgency", "whenever", "--title", "Labelled",
		"--label", "feat-labels", "--label", "round 3, part 2", "--worker", "w1", "--brief", "Resume from LEDGER.md"))
	var shown struct {
		Labels []string `json:"labels"`
		Brief  string   `json:"brief"`
	}
	if err := json.Unmarshal([]byte(mustRun(t, "--store", storePath, "show", id, "--json")), &shown); err != nil {
		t.Fatal(err)
	}
	if want := []string{"feat-labels", "round 3, part 2"}; !slices.Equal(shown.Labels, want) || shown.Brief != "Resume from LEDGER.md" {
		t.Errorf("show --json = %+v, want labels %q", shown, want)
	}
	if out := mustRun(t, "--store", storePath, "show", id); !strings.Contains(out, "labels:   feat-labels, round 3, part 2\n") {
		t.Errorf("show:\n%s", out)
	}
}

func TestOpenWorkerAndLabelFromTheEnvironment(t *testing.T) {
	open := func(t *testing.T, storePath string, args ...string) *store.Case {
		t.Helper()
		out := mustRun(t, append([]string{"--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Env"}, args...)...)
		return loadCase(t, storePath, strings.TrimSpace(out))
	}
	storePath := newStore(t)
	t.Setenv("CASES_WORKER", "bun-pins")
	// The whole value is one label, commas and all.
	t.Setenv("CASES_LABEL", "feat-labels, round 3")
	if c := open(t, storePath); c.Worker != "bun-pins" || !slices.Equal(c.Labels, []string{"feat-labels, round 3"}) {
		t.Errorf("worker = %q, labels = %q", c.Worker, c.Labels)
	}
	// A flag replaces the environment's value; it does not add to it.
	if c := open(t, storePath, "--worker", "other", "--label", "a", "--label", "b"); c.Worker != "other" || !slices.Equal(c.Labels, []string{"a", "b"}) {
		t.Errorf("worker = %q, labels = %q", c.Worker, c.Labels)
	}
	// Set but empty, CASES_LABEL is one blank label, which the store refuses.
	t.Setenv("CASES_LABEL", "")
	if r := runCases(t, "", "--store", storePath, "open", "--kind", "fyi", "--urgency", "today", "--title", "Env"); r.err == nil || !strings.Contains(r.err.Error(), "label 1 is empty") {
		t.Errorf("empty CASES_LABEL: err = %v", r.err)
	}
	t.Setenv("CASES_WORKER", "")
	if c := open(t, storePath, "--label", "a"); c.Worker != "" {
		t.Errorf("empty CASES_WORKER: worker = %q", c.Worker)
	}
}

func TestOpenRefusals(t *testing.T) {
	tests := []struct {
		name    string
		args    []string
		wantErr string
	}{
		{"unknown kind", []string{"--kind", "poll", "--urgency", "today", "--title", "x"}, "--kind"},
		{"decision without options", []string{"--kind", "decision", "--urgency", "today", "--title", "x"}, "at least one option"},
		{"row that is not JSON", []string{"--kind", "approval", "--urgency", "today", "--title", "x", "--row", "deps=npm ci"}, "--row 1"},
		{"row with an unknown field", []string{"--kind", "approval", "--urgency", "today", "--title", "x", "--row", `{"id":"a","label":"l","script":"s","link":"k","cmd":"x"}`}, `unknown field "cmd"`},
		{"two rows in one flag", []string{"--kind", "approval", "--urgency", "today", "--title", "x", "--row", `{"id":"a","label":"l","script":"s","link":"k"},{"id":"b","label":"l","script":"s","link":"k"}`}, "one --row per row"},
		{"two rows copied from an array", []string{"--kind", "approval", "--urgency", "today", "--title", "x", "--row", `{"id":"a","label":"l","script":"s","link":"k"}}, {"id":"b","label":"l","script":"s","link":"k"}`}, "one --row per row"},
		{"row without a link", []string{"--kind", "approval", "--urgency", "today", "--title", "x", "--row", `{"id":"a","label":"l","script":"s"}`}, "link is empty"},
		{"options on question", []string{"--kind", "question", "--urgency", "today", "--title", "x", "--option", "a"}, "options are for decision cases, not question"},
		{"rows on question", []string{"--kind", "question", "--urgency", "today", "--title", "x", "--row", `{"id":"a","label":"l","script":"s","link":"k"}`}, "rows are for approval cases, not question"},
		{"options on fyi", []string{"--kind", "fyi", "--urgency", "today", "--title", "x", "--option", "a"}, "options are for decision cases"},
		{"blank label", []string{"--kind", "fyi", "--urgency", "today", "--title", "x", "--label", "a", "--label", " "}, "label 2 is empty"},
		{"body and body file", []string{"--kind", "fyi", "--urgency", "today", "--title", "x", "--body", "x", "--body-file", "-"}, "--body and --body-file can't be used together"},
		{"body that names a file", []string{"--kind", "fyi", "--urgency", "today", "--title", "x", "--body", "open_test.go"}, "names a file; pass it with --body-file"},
		{"the same label twice", []string{"--kind", "fyi", "--urgency", "today", "--title", "x", "--label", "a", "--label", "a"}, `label "a" is used twice`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			storePath := newStore(t)
			r := runCases(t, "", append([]string{"--store", storePath, "open"}, tt.args...)...)
			if r.err == nil || !strings.Contains(r.err.Error(), tt.wantErr) {
				t.Fatalf("err = %v, want %q", r.err, tt.wantErr)
			}
			if _, err := os.Stat(storePath); !errors.Is(err, fs.ErrNotExist) {
				t.Errorf("a refused open made the store: %v", err)
			}
		})
	}
}
