package store

import (
	"context"
	"errors"
	"io/fs"
	"os"
	"slices"
	"testing"
	"time"
)

// archivedRows reads the archive's rows for the case in sequence order.
func archivedRows(t *testing.T, d *DB, id string) []storedRow {
	t.Helper()
	return tableRows(t, d, "archive_events", id)
}

// withdrawn opens a case and withdraws it.
func withdrawn(t *testing.T, d *DB) *Case {
	t.Helper()
	c, err := d.Create(t.Context(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	if c, err = d.Withdraw(t.Context(), c.ID, WithdrawRecord{Reason: "not needed"}); err != nil {
		t.Fatal(err)
	}
	return c
}

func TestArchiveMovesTheCaseOutOfSight(t *testing.T) {
	fixClock(t, time.Date(2026, 9, 17, 8, 0, 0, 0, time.UTC))
	d := newDB(t)
	ctx := t.Context()
	c := withdrawn(t, d)
	kept, err := d.Create(ctx, openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	before := storedRows(t, d, c.ID)

	if err := d.Archive(ctx, c.ID, AtRevision(c.Revision())); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Get(ctx, c.ID); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Get archived case: %v, want fs.ErrNotExist", err)
	}
	if ids, err := d.IDs(ctx); err != nil || !slices.Equal(ids, []string{kept.ID}) {
		t.Errorf("IDs = %v, %v", ids, err)
	}
	if got := storedRows(t, d, c.ID); len(got) != 0 {
		t.Errorf("events left behind: %v", names(got))
	}
	if got := archivedRows(t, d, c.ID); !slices.Equal(got, before) {
		t.Errorf("archive holds %v, want the rows as written: %v", got, before)
	}
	var archivedAt string
	if err := pool(t, d).QueryRowContext(ctx, `SELECT archived_at FROM archive_cases WHERE id = ?`, c.ID).Scan(&archivedAt); err != nil || archivedAt != "2026-09-17T08:00:00Z" {
		t.Errorf("archived_at = %q, %v", archivedAt, err)
	}

	// An id in the archive is not given to a new case.
	again, err := d.Create(ctx, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	if again.ID != c.ID+"-2" {
		t.Errorf("new case id = %s, want %s-2", again.ID, c.ID)
	}
}

// A case whose id the archive already holds, as after a restore by hand, is
// refused and left where it is.
func TestArchiveRefusesAnIDTheArchiveHas(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	c := withdrawn(t, d)
	if err := d.Archive(ctx, c.ID); err != nil {
		t.Fatal(err)
	}
	insertRow(t, d, c.ID, 1, "agent", "open", archivedRows(t, d, c.ID)[0].Data)

	if err := d.Archive(ctx, c.ID); !errors.Is(err, ErrArchived) {
		t.Fatalf("err = %v, want ErrArchived", err)
	}
	if got, err := d.Get(ctx, c.ID); err != nil || got.State != StateOpen {
		t.Errorf("refused case = %+v, %v", got, err)
	}
	if got := archivedRows(t, d, c.ID); len(got) != 2 {
		t.Errorf("archive changed: %v", names(got))
	}
}

func TestDeleteRemovesTheCase(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	c := withdrawn(t, d)
	if err := d.Delete(ctx, c.ID, AtRevision(2)); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Get(ctx, c.ID); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Get deleted case: %v, want fs.ErrNotExist", err)
	}
	if got := archivedRows(t, d, c.ID); len(got) != 0 {
		t.Errorf("delete archived the case: %v", names(got))
	}
}

func TestArchiveAndDeleteRefuse(t *testing.T) {
	for name, method := range map[string]func(d *DB, ctx context.Context, id string, pre ...Precondition) error{
		"archive": (*DB).Archive,
		"delete":  (*DB).Delete,
	} {
		t.Run(name, func(t *testing.T) {
			ctx := t.Context()
			remove := func(d *DB, id string, pre ...Precondition) error { return method(d, ctx, id, pre...) }
			d := newDB(t)
			if err := remove(d, "2026-01-01T00-00-00Z-x"); !errors.Is(err, fs.ErrNotExist) {
				t.Errorf("no store: %v, want fs.ErrNotExist", err)
			}
			if _, err := os.Stat(d.Path); !errors.Is(err, fs.ErrNotExist) {
				t.Errorf("made the store: %v", err)
			}
			c := withdrawn(t, d)
			if err := remove(d, "2026-01-01T00-00-00Z-x"); !errors.Is(err, fs.ErrNotExist) {
				t.Errorf("no case: %v, want fs.ErrNotExist", err)
			}
			if err := remove(d, "../x"); err == nil || err.Error() != `invalid case id "../x"` {
				t.Errorf("invalid id: %v", err)
			}
			// Changed since it was read at revision 1.
			if err := remove(d, c.ID, AtRevision(1)); !errors.Is(err, ErrStale) {
				t.Errorf("stale: %v, want ErrStale", err)
			}
			if got, err := d.Get(ctx, c.ID); err != nil || got.Revision() != 2 {
				t.Errorf("refused case = %+v, %v", got, err)
			}
			if got := archivedRows(t, d, c.ID); len(got) != 0 {
				t.Errorf("archive = %v", names(got))
			}
		})
	}
}
