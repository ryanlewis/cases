package main

import (
	"testing"
)

func TestPickupAtRevision(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	refusedAsStale(t, root, id, 1, 2, "pickup", id, "--revision", "1")
	if out := mustRun(t, "--store", root, "pickup", id, "--revision", "2"); out != id+" pickedup\n" {
		t.Errorf("stdout = %q", out)
	}
}

func TestPickup(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)
	if r := runCases(t, "", "--store", root, "pickup", id); r.err == nil || r.err.Error() != "cannot pickup a case that is open" {
		t.Errorf("pickup of an open case: err = %v", r.err)
	}
	mustRun(t, "--store", root, "answer", id, "--option", "1")
	if out := mustRun(t, "--store", root, "pickup", id, "--by", "agent-inbox-bd"); out != id+" pickedup\n" {
		t.Errorf("stdout = %q", out)
	}
	if c := loadCase(t, root, id); c.Pickup == nil || c.Pickup.By != "agent-inbox-bd" {
		t.Errorf("pickup = %+v", c.Pickup)
	}
}
