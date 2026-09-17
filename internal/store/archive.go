package store

import (
	"context"
	"database/sql"
	"errors"
)

// ErrArchived refuses to archive a case whose id the archive already holds.
var ErrArchived = errors.New("already in the archive")

// Archive moves the case id and its events into the archive tables, in one
// transaction. List, IDs, Get and the poller no longer see it, and its events
// are kept as they were written. The preconditions are checked first, as for
// a write. A case whose id is already in the archive is refused with
// ErrArchived and left where it is.
func (d *DB) Archive(ctx context.Context, id string, pre ...Precondition) error {
	return d.remove(ctx, id, pre, true)
}

// Delete removes the case id and its events from the store, in one
// transaction. It cannot be undone. The preconditions are checked first, as
// for a write.
func (d *DB) Delete(ctx context.Context, id string, pre ...Precondition) error {
	return d.remove(ctx, id, pre, false)
}

func (d *DB) remove(ctx context.Context, id string, pre []Precondition, archive bool) error {
	if err := ValidID(id); err != nil {
		return err
	}
	return d.write(ctx, false, func(tx *sql.Tx) error {
		var found bool
		var rev int
		err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM cases WHERE id = ?1), (SELECT count(*) FROM events WHERE case_id = ?1)`, id).Scan(&found, &rev)
		if err != nil {
			return err
		}
		if !found {
			return noCase(id)
		}
		if err := checkPreconditions(pre, rev); err != nil {
			return err
		}
		if archive {
			var archived bool
			if err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM archive_cases WHERE id = ?)`, id).Scan(&archived); err != nil {
				return err
			}
			if archived {
				return ErrArchived
			}
			archivedAt := now().Format(atLayout)
			for _, stmt := range []string{
				`INSERT INTO archive_cases (id, opened_at, archived_at) SELECT id, opened_at, ?2 FROM cases WHERE id = ?1`,
				`INSERT INTO archive_events (change, case_id, seq, author, event, at, data) SELECT change, case_id, seq, author, event, at, data FROM events WHERE case_id = ?1`,
			} {
				if _, err := tx.ExecContext(ctx, stmt, id, archivedAt); err != nil {
					return err
				}
			}
		}
		for _, stmt := range []string{
			`DELETE FROM events WHERE case_id = ?`,
			`DELETE FROM cases WHERE id = ?`,
		} {
			if _, err := tx.ExecContext(ctx, stmt, id); err != nil {
				return err
			}
		}
		return nil
	})
}
