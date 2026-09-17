package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"
)

// idTimeLayout is the timestamp at the front of a case id. Colons are left out
// so the id is safe to type, and to use in a URL or a file name.
const idTimeLayout = "2006-01-02T15-04-05Z"

// maxSlug caps the title part of a case id.
const maxSlug = 48

// atLayout is how the at, opened_at and archived_at columns write a time.
const atLayout = time.RFC3339Nano

// now is the store's clock: event timestamps and when archived. Tests
// replace it.
var now = func() time.Time { return time.Now().UTC() }

// commit commits a write transaction, replaceable so tests can fail the last
// step of a write.
var commit = func(tx *sql.Tx) error { return tx.Commit() }

// ValidID refuses an id that Create could not have minted and that would be
// unsafe to put in a path or a URL: empty, starting with a dot, or holding a
// slash. It does not look in the store.
func ValidID(id string) error {
	if id == "" || strings.HasPrefix(id, ".") || strings.ContainsAny(id, `/\`) {
		return fmt.Errorf("invalid case id %q", id)
	}
	return nil
}

// IsWholeID reports whether id has the shape Create gives a case id: the open
// time, a dash and a slug.
func IsWholeID(id string) bool {
	n := len(idTimeLayout)
	if len(id) < n+2 || id[n] != '-' {
		return false
	}
	_, err := time.Parse(idTimeLayout, id[:n])
	return err == nil
}

// Create opens a new case, creating the store's file if it does not exist.
// The case id is the open time and a slug of the title; if a case, live or
// archived, already has that id, a numeric suffix is added. The id is chosen
// and the open event stored in one transaction.
func (d *DB) Create(ctx context.Context, rec OpenRecord) (*Case, error) {
	if err := rec.validate(); err != nil {
		return nil, err
	}
	var c *Case
	err := d.write(ctx, true, func(tx *sql.Tx) error {
		rec.stamp(now())
		base := rec.OpenedAt.Format(idTimeLayout) + "-" + Slug(rec.Title)
		id := ""
		for n := 1; id == ""; n++ {
			if n > 100 {
				return fmt.Errorf("cannot choose an id for the case: %s to %s-100 are all taken", base, base)
			}
			try := base
			if n > 1 {
				try = fmt.Sprintf("%s-%d", base, n)
			}
			var taken bool
			err := tx.QueryRowContext(ctx, `SELECT EXISTS (SELECT 1 FROM cases WHERE id = ?1) OR EXISTS (SELECT 1 FROM archive_cases WHERE id = ?1)`, try).Scan(&taken)
			if err != nil {
				return err
			}
			if !taken {
				id = try
			}
		}
		if _, err := tx.ExecContext(ctx, `INSERT INTO cases (id, opened_at) VALUES (?, ?)`, id, rec.OpenedAt.Format(atLayout)); err != nil {
			return err
		}
		var err error
		c, err = appendEvent(ctx, tx, id, AuthorAgent, EventOpen, &rec, nil)
		return err
	})
	if err != nil {
		return nil, err
	}
	return c, nil
}

// ErrStale refuses a write made with AtRevision: an event has been written to
// the case since the caller read it.
var ErrStale = errors.New("the case has changed since it was read")

// Precondition is a check on the case as it is in the store, made inside the
// write's transaction and before the event is checked. Make one with
// AtRevision.
type Precondition struct {
	revision int
}

// AtRevision is the precondition that the case is still at revision rev: the
// Revision of the case as the caller read it. Otherwise the write fails with
// ErrStale and nothing is written.
func AtRevision(rev int) Precondition {
	return Precondition{revision: rev}
}

// checkPreconditions checks pre against a case now at revision rev.
func checkPreconditions(pre []Precondition, rev int) error {
	for _, p := range pre {
		if rev != p.revision {
			return fmt.Errorf("%w: read at revision %d, now at %d", ErrStale, p.revision, rev)
		}
	}
	return nil
}

// noCase is the error for a case id the store does not have.
func noCase(id string) error {
	return &notFound{fmt.Sprintf("no case %q", id)}
}

// Amend records the agent changing an open case: adding options, rows,
// links or labels, or replacing the body or context.
func (d *DB) Amend(ctx context.Context, id string, rec AmendRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorAgent, EventAmend, &rec, pre)
}

// Answer records the human's answer.
func (d *DB) Answer(ctx context.Context, id string, rec AnswerRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorHuman, EventAnswer, &rec, pre)
}

// Pickup records that the agent has read the answer.
func (d *DB) Pickup(ctx context.Context, id string, rec PickupRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorAgent, EventPickup, &rec, pre)
}

// Note records a follow-up from the agent. It reopens an answered or picked-up
// case.
func (d *DB) Note(ctx context.Context, id string, rec NoteRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorAgent, EventNote, &rec, pre)
}

// Close records the outcome of a picked-up case.
func (d *DB) Close(ctx context.Context, id string, rec CloseRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorAgent, EventClose, &rec, pre)
}

// Withdraw records that the agent no longer needs an open case answered.
func (d *DB) Withdraw(ctx context.Context, id string, rec WithdrawRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorAgent, EventWithdraw, &rec, pre)
}

// Park records the human parking an open stuck case.
func (d *DB) Park(ctx context.Context, id string, rec ParkRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, AuthorHuman, EventPark, &rec, pre)
}

// Resume reopens a parked case. Either side may resume.
func (d *DB) Resume(ctx context.Context, id string, author Author, rec ResumeRecord, pre ...Precondition) (*Case, error) {
	return d.append(ctx, id, author, EventResume, &rec, pre)
}

// append stores one event on the case id in a transaction of its own.
func (d *DB) append(ctx context.Context, id string, author Author, typ EventType, rec record, pre []Precondition) (*Case, error) {
	if err := ValidID(id); err != nil {
		return nil, err
	}
	var c *Case
	err := d.write(ctx, false, func(tx *sql.Tx) error {
		// Stamped once the write holds the store's lock, so events are timed
		// in the order they are stored however long the wait for the lock: a
		// wait --since an event's time sees every event stored after it.
		rec.stamp(now())
		var err error
		c, err = appendEvent(ctx, tx, id, author, typ, rec, pre)
		return err
	})
	if err != nil {
		return nil, err
	}
	return c, nil
}

// write runs fn in a write transaction on the store and commits only when fn
// succeeds. With create it creates the store if it does not exist; without it
// a missing store is fs.ErrNotExist. The transaction begins immediate: it
// takes the store's write lock before its first read, so writers, in this
// process or another, run one at a time, and each reads the case as the
// writer before it left it.
func (d *DB) write(ctx context.Context, create bool, fn func(tx *sql.Tx) error) error {
	pool, _, release, err := d.open(ctx, create)
	if err != nil {
		return err
	}
	defer release()
	tx, err := pool.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() { _ = tx.Rollback() }()
	if err := checkVersion(ctx, tx, d.Path); err != nil {
		return err
	}
	if err := fn(tx); err != nil {
		return err
	}
	return commit(tx)
}

// appendEvent folds the case in tx, checks the preconditions and then the new
// event against it with the same code the fold uses, and only then inserts
// the event, numbered after the case's latest. The caller's transaction holds
// the write lock throughout, so two writers cannot take the same number, and
// a precondition cannot pass on a case that changes before the insert.
func appendEvent(ctx context.Context, tx *sql.Tx, id string, author Author, typ EventType, rec record, pre []Precondition) (*Case, error) {
	data, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		return nil, err
	}
	data = append(data, '\n')

	c := &Case{ID: id}
	if typ != EventOpen {
		if c, err = loadCase(ctx, tx, id); err != nil {
			return nil, err
		}
	}
	if err := checkPreconditions(pre, c.Revision()); err != nil {
		return nil, err
	}
	seq := c.lastSeq + 1
	if err := c.apply(Event{Seq: seq, Author: author, Type: typ, File: fileName(seq, author, typ), Data: data}); err != nil {
		return nil, err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO events (case_id, seq, author, event, at, data) VALUES (?, ?, ?, ?, ?, ?)`,
		id, seq, string(author), string(typ), rec.at().UTC().Format(atLayout), data)
	if err != nil {
		return nil, err
	}
	c.lastSeq = seq
	c.events++
	return c, nil
}

// Slug turns a title into the lowercase ASCII words-and-dashes part of a case
// id.
func Slug(title string) string {
	var b strings.Builder
	dash := false
	for _, r := range strings.ToLower(title) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
			dash = false
			continue
		}
		if b.Len() > 0 && !dash {
			b.WriteByte('-')
			dash = true
		}
	}
	s := strings.TrimRight(b.String(), "-")
	if len(s) > maxSlug {
		s = s[:maxSlug]
		if i := strings.LastIndexByte(s, '-'); i > 0 {
			s = s[:i]
		}
	}
	if s == "" {
		return "case"
	}
	return s
}
