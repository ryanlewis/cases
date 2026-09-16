package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/store"
)

func loadCase(t *testing.T, root, id string) *store.Case {
	t.Helper()
	c, err := store.Load(filepath.Join(root, id))
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestOpenDecisionKeepsCommasInOptions(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	c := loadCase(t, root, id)
	if want := []string{"Pin to 1.2.3", "Float, with renovate"}; !slices.Equal(c.Options, want) {
		t.Errorf("options = %q, want %q", c.Options, want)
	}
	if !strings.HasSuffix(id, "-pin-bun") || c.State != store.StateOpen {
		t.Errorf("id = %s, state = %s", id, c.State)
	}
}

func TestOpenBody(t *testing.T) {
	root := t.TempDir()
	file := filepath.Join(t.TempDir(), "body.md")
	if err := os.WriteFile(file, []byte("# From a file\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	fromFile := strings.TrimSpace(mustRun(t, "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "File", "--body-file", file))
	if got := loadCase(t, root, fromFile).Body; got != "# From a file\n" {
		t.Errorf("body = %q", got)
	}

	r := runCases(t, "from stdin", "--store", root, "open", "--kind", "fyi", "--urgency", "today", "--title", "Stdin", "--body-file", "-",
		"--worker", "bun-pins", "--brief", "/briefs/bun.md", "--link", "https://a,b", "--context", "ctx")
	if r.err != nil {
		t.Fatal(r.err)
	}
	c := loadCase(t, root, strings.TrimSpace(r.stdout))
	if c.Body != "from stdin" || c.Worker != "bun-pins" || c.Brief != "/briefs/bun.md" || c.Context != "ctx" || !slices.Equal(c.Links, []string{"https://a,b"}) {
		t.Errorf("case = %+v", c.OpenRecord)
	}
}

func TestOpenApprovalRows(t *testing.T) {
	root := t.TempDir()
	out := mustRun(t, "--store", root, "open", "--kind", "approval", "--urgency", "blocking", "--title", "Scripts",
		"--row", `{"id":"deps","label":"Install deps","script":"npm ci --ignore-scripts\nnpm test","link":"https://example.com/deps"}`,
		"--row", `{"id":"mig","label":"Migrate","note":"takes the site down for a minute","script":"make migrate","link":"https://example.com/mig"}`)
	id := strings.TrimSpace(out)
	c := loadCase(t, root, id)
	if len(c.Rows) != 2 || c.Rows[0].Script != "npm ci --ignore-scripts\nnpm test" || c.Rows[1].ID != "mig" {
		t.Errorf("rows = %+v", c.Rows)
	}

	var shown struct {
		Rows []struct {
			ID   string  `json:"id"`
			Note *string `json:"note"`
		} `json:"rows"`
	}
	if err := json.Unmarshal([]byte(mustRun(t, "--store", root, "show", id, "--json")), &shown); err != nil {
		t.Fatal(err)
	}
	// A row without a note is written as before, with no note key.
	if len(shown.Rows) != 2 || shown.Rows[0].Note != nil || shown.Rows[1].Note == nil || *shown.Rows[1].Note != "takes the site down for a minute" {
		t.Errorf("show --json rows = %+v", shown.Rows)
	}
	if text := mustRun(t, "--store", root, "show", id); !strings.Contains(text, "  [mig] Migrate\n      note: takes the site down for a minute\n      https://example.com/mig\n") ||
		strings.Count(text, "note:") != 1 {
		t.Errorf("show output:\n%s", text)
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
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			root := t.TempDir()
			r := runCases(t, "", append([]string{"--store", root, "open"}, tt.args...)...)
			if r.err == nil || !strings.Contains(r.err.Error(), tt.wantErr) {
				t.Fatalf("err = %v, want %q", r.err, tt.wantErr)
			}
			if entries, _ := os.ReadDir(root); len(entries) != 0 {
				t.Errorf("store has %d entries after a refused open", len(entries))
			}
		})
	}
}
