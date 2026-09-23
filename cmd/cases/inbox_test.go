package main

import (
	"errors"
	"strings"
	"testing"
)

func TestInboxPrintsTheRunningURL(t *testing.T) {
	storePath := newStore(t)
	info := fakeServe(t, storePath)

	out := mustRun(t, "--store", storePath, "inbox", "--print")
	if got := strings.TrimSpace(out); got != info.URL {
		t.Errorf("inbox --print = %q, want %q", got, info.URL)
	}
}

func TestInboxPrintsACasesPage(t *testing.T) {
	storePath := newStore(t)
	info := fakeServe(t, storePath)
	id := openDecision(t, storePath)

	out := mustRun(t, "--store", storePath, "inbox", "--print", id)
	want := strings.TrimSuffix(info.URL, "/") + "/cases/" + id
	if got := strings.TrimSpace(out); got != want {
		t.Errorf("inbox --print %s = %q, want %q", id, got, want)
	}
}

func TestInboxPrintUnknownCase(t *testing.T) {
	storePath := newStore(t)
	fakeServe(t, storePath)

	r := runCases(t, "", "--store", storePath, "inbox", "--print", "nope")
	if r.err == nil || !strings.Contains(r.err.Error(), "no case") {
		t.Errorf("err = %v, want a refusal naming the missing case", r.err)
	}
}

func TestInboxOpensTheBrowserWhenNotPrinting(t *testing.T) {
	storePath := newStore(t)
	info := fakeServe(t, storePath)

	var opened []string
	r := runCasesDeps(t, nil, func(d *Deps) {
		d.OpenURL = func(url string) error { opened = append(opened, url); return nil }
	}, "", "--store", storePath, "inbox")
	if r.err != nil {
		t.Fatalf("inbox: %v\nstderr: %s", r.err, r.stderr)
	}
	if len(opened) != 1 || opened[0] != info.URL {
		t.Errorf("opened = %v, want [%s]", opened, info.URL)
	}
	if r.stdout != "" {
		t.Errorf("stdout = %q, want nothing printed", r.stdout)
	}
}

func TestInboxWithoutServe(t *testing.T) {
	storePath := newStore(t)

	r := runCases(t, "", "--store", storePath, "inbox", "--print")
	var ee *exitError
	if !errors.As(r.err, &ee) || ee.code != 1 || !strings.Contains(ee.msg, "not running") || !strings.Contains(ee.msg, "cases serve") {
		t.Errorf("inbox without serve: err = %v, want exit 1 naming cases serve", r.err)
	}
}
