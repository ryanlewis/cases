package main

import (
	"testing"
)

func TestWithdraw(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	if out := mustRun(t, "--store", root, "withdraw", id); out != id+" withdrawn\n" {
		t.Errorf("stdout = %q", out)
	}
	if r := runCases(t, "", "--store", root, "answer", id, "--option", "1"); r.err == nil {
		t.Error("answered a withdrawn case")
	}
	if r := runCases(t, "", "--store", root, "withdraw", id); r.err == nil {
		t.Error("withdrew twice")
	}
}
