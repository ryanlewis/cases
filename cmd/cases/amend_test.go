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

func openApproval(t *testing.T, root string) string {
	t.Helper()
	out := mustRun(t, "--store", root, "open", "--kind", "approval", "--urgency", "today", "--title", "Scripts",
		"--context", "Release 1.4", "--label", "release", "--row", `{"id":"deps","label":"Install deps","script":"npm ci","link":"https://example.com/deps"}`)
	return strings.TrimSpace(out)
}

func TestAmendRoundTrip(t *testing.T) {
	root := t.TempDir()
	id := openApproval(t, root)

	r := runCases(t, "Two scripts now.\n", "--store", root, "amend", id, "--body-file", "-",
		"--row", `{"id":"mig","label":"Migrate","script":"make migrate","link":"https://example.com/mig"}`,
		"--link", "https://example.com/a,b", "--context", "Release 1.4, then 1.4.1")
	if r.err != nil {
		t.Fatal(r.err)
	}
	if r.stdout != id+" open\n" {
		t.Errorf("stdout = %q", r.stdout)
	}

	// show --json has the amended fields on the case, and the open event as
	// it was written beside the amend.
	var shown struct {
		State   string      `json:"state"`
		Body    string      `json:"body"`
		Context string      `json:"context"`
		Rows    []store.Row `json:"rows"`
		Links   []string    `json:"links"`
		Events  []struct {
			File string          `json:"file"`
			Data json.RawMessage `json:"data"`
		} `json:"events"`
	}
	if err := json.Unmarshal([]byte(mustRun(t, "--store", root, "show", id, "--json")), &shown); err != nil {
		t.Fatal(err)
	}
	if shown.State != "open" || shown.Body != "Two scripts now.\n" || shown.Context != "Release 1.4, then 1.4.1" ||
		len(shown.Rows) != 2 || shown.Rows[1].ID != "mig" || !slices.Equal(shown.Links, []string{"https://example.com/a,b"}) {
		t.Errorf("case = %+v", shown)
	}
	if len(shown.Events) != 2 || shown.Events[0].File != "0001-agent-open.json" || shown.Events[1].File != "0002-agent-amend.json" {
		t.Fatalf("events = %+v", shown.Events)
	}
	var opened store.OpenRecord
	if err := json.Unmarshal(shown.Events[0].Data, &opened); err != nil {
		t.Fatal(err)
	}
	if opened.Body != "" || len(opened.Rows) != 1 || opened.Context != "Release 1.4" {
		t.Errorf("open event = %+v", opened)
	}

	out := mustRun(t, "--store", root, "show", id)
	for _, want := range []string{"Two scripts now.", "[mig] Migrate", "0002 agent amend", "replaced the body", "added row [mig] Migrate",
		"added link: https://example.com/a,b", "replaced the context: Release 1.4, then 1.4.1"} {
		if !strings.Contains(out, want) {
			t.Errorf("show output missing %q:\n%s", want, out)
		}
	}

	// The answer needs a verdict on the added row too.
	mustRun(t, "--store", root, "answer", id, "--row", "deps=approve", "--row", "mig=hold")
	r = runCases(t, "", "--store", root, "amend", id, "--link", "https://example.com/late")
	if r.err == nil || !strings.Contains(r.err.Error(), "cannot amend a case that is answered") {
		t.Errorf("amend after the answer: err = %v", r.err)
	}
}

// show prints the body and context an amend replaced, in full, under the line
// saying so. The first amend replaced no body, so it has none to print.
func TestShowPrintsWhatAnAmendReplaced(t *testing.T) {
	root := t.TempDir()
	id := openApproval(t, root)
	if r := runCases(t, "# Scripts\n\nTwo scripts now.\n", "--store", root, "amend", id, "--body-file", "-"); r.err != nil {
		t.Fatal(r.err)
	}
	if r := runCases(t, "Three scripts now.\n", "--store", root, "amend", id, "--body-file", "-", "--context", "Release 1.4.1"); r.err != nil {
		t.Fatal(r.err)
	}
	out := mustRun(t, "--store", root, "show", id)
	want := "       replaced the body\n" +
		"         previous body:\n" +
		"         | # Scripts\n" +
		"         | \n" +
		"         | Two scripts now.\n" +
		"       replaced the context: Release 1.4.1\n" +
		"         previous context:\n" +
		"         | Release 1.4\n"
	if !strings.Contains(out, want) || strings.Count(out, "previous body:") != 1 {
		t.Errorf("show output lacks the replaced text, or has it twice:\n%s", out)
	}
}

func TestAmendAtRevision(t *testing.T) {
	root := t.TempDir()
	id := openApproval(t, root)
	mustRun(t, "--store", root, "amend", id, "--link", "https://example.com/a")
	refusedAsStale(t, root, id, 1, 2, "amend", id, "--body", "Replaced.", "--revision", "1")
	if out := mustRun(t, "--store", root, "amend", id, "--body", "Replaced.", "--revision", "2"); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
}

func TestAmendInlineBody(t *testing.T) {
	root := t.TempDir()
	id := openApproval(t, root)
	if out := mustRun(t, "--store", root, "amend", id, "--body", "One script."); out != id+" open\n" {
		t.Errorf("stdout = %q", out)
	}
	if got := loadCase(t, root, id).Body; got != "One script." {
		t.Errorf("body = %q", got)
	}
}

func TestAmendAddsOptions(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "amend", id, "--option", "Vendor it, for now")
	if got, want := loadCase(t, root, id).Options, []string{"Pin to 1.2.3", "Float, with renovate", "Vendor it, for now"}; !slices.Equal(got, want) {
		t.Errorf("options = %q, want %q", got, want)
	}
	if out := mustRun(t, "--store", root, "answer", id, "--option", "3"); out != id+" answered\n" {
		t.Errorf("answer = %q", out)
	}
	if out := mustRun(t, "--store", root, "show", id); !strings.Contains(out, "added option: Vendor it, for now") || !strings.Contains(out, "chose 3. Vendor it, for now") {
		t.Errorf("show output:\n%s", out)
	}
}

func TestAmendAddsLabels(t *testing.T) {
	root := t.TempDir()
	id := openApproval(t, root)
	mustRun(t, "--store", root, "amend", id, "--label", "round 3", "--label", "a,b")
	if c := loadCase(t, root, id); !slices.Equal(c.Labels, []string{"release", "round 3", "a,b"}) {
		t.Errorf("labels = %q", c.Labels)
	}
	if out := mustRun(t, "--store", root, "show", id); !strings.Contains(out, "added label: round 3") {
		t.Errorf("show:\n%s", out)
	}
}

func TestAmendRefusals(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "missing.md")
	tests := []struct {
		name    string
		stdin   string
		args    []string
		wantErr string
	}{
		{"nothing to amend", "", nil, "the amend has no body, options, rows, links, labels or context"},
		{"blank body", " \n", []string{"--body-file", "-"}, "amend body is empty"},
		// The store would take an empty body as no change and add the link.
		{"empty body file with a link", "", []string{"--body-file", "-", "--link", "https://example.com/log"}, "amend body is empty"},
		{"empty inline body with a link", "", []string{"--body", "", "--link", "https://example.com/log"}, "amend body is empty"},
		{"blank inline body", "", []string{"--body", " "}, "amend body is empty"},
		{"inline body that names a file", "", []string{"--body", "amend_test.go"}, "names a file; pass it with --body-file"},
		{"body and body file", "x", []string{"--body", "x", "--body-file", "-"}, "--body and --body-file can't be used together"},
		{"missing body file", "", []string{"--body-file", missing}, "body:"},
		{"body file with no name", "", []string{"--body-file", "", "--link", "https://example.com/log"}, "body:"},
		{"options on approval", "", []string{"--option", "Skip it"}, "options are for decision cases"},
		{"row that is not JSON", "", []string{"--row", "mig=make migrate"}, "--row 1"},
		{"row id already on the case", "", []string{"--row", `{"id":"deps","label":"Again","script":"npm ci","link":"https://example.com/deps"}`}, `id "deps" is already on the case`},
		{"row with a stray bracket", "", []string{"--row", `{"id":"mig","label":"Migrate","script":"make migrate","link":"https://example.com/mig"}] {"id":"x"}`}, "pass one --row per row"},
		// The store would take an empty context as no change and add the link.
		{"empty context with a link", "", []string{"--context", "", "--link", "https://example.com/log"}, "amend context is empty"},
		{"blank link", "", []string{"--link", " "}, "link 1 is empty"},
		{"the context the case has", "", []string{"--context", "Release 1.4"}, "the amend changes nothing"},
		{"the same link twice", "", []string{"--link", "https://example.com/log", "--link", "https://example.com/log"}, "is used twice"},
		{"blank label", "", []string{"--label", ""}, "label 1 is empty"},
		{"label already on the case", "", []string{"--label", "release"}, `label "release" is already on the case`},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			root := t.TempDir()
			id := openApproval(t, root)
			r := runCases(t, tt.stdin, append([]string{"--store", root, "amend", id}, tt.args...)...)
			if r.err == nil || !strings.Contains(r.err.Error(), tt.wantErr) {
				t.Fatalf("err = %v, want %q", r.err, tt.wantErr)
			}
			if entries, _ := os.ReadDir(filepath.Join(root, id)); len(entries) != 1 {
				t.Errorf("case has %d files after a refused amend", len(entries))
			}
		})
	}
	root := t.TempDir()
	if r := runCases(t, "", "--store", root, "amend", "../etc", "--context", "x"); r.err == nil || !strings.Contains(r.err.Error(), "invalid case id") {
		t.Errorf("amend ../etc: err = %v", r.err)
	}
	if r := runCases(t, "", "--store", root, "amend", "nope", "--context", "x"); r.err == nil {
		t.Error("amend of a missing case succeeded")
	}
}
