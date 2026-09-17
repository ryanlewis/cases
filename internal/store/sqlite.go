package store

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"io/fs"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"modernc.org/sqlite" // also registers the "sqlite" driver
	sqlite3 "modernc.org/sqlite/lib"
)

// schemaVersion is the version of the schema below. A store with a higher
// version was made by a newer cases and is refused.
const schemaVersion = 1

// schema is the store's tables. events.change is the rowid under a name: it
// numbers every event row across the store and, being AUTOINCREMENT, only
// grows and is never reused after a delete, so the poller can key on it.
// (case_id, seq) is unique, as the event file name was. data is the event
// record's JSON exactly as written. The archive tables have the same columns
// and hold the cases prune moves out of sight.
const schema = `
CREATE TABLE meta (
	schema_version INTEGER NOT NULL
) STRICT;

CREATE TABLE cases (
	id        TEXT NOT NULL PRIMARY KEY,
	opened_at TEXT NOT NULL
) STRICT;

CREATE TABLE events (
	change  INTEGER PRIMARY KEY AUTOINCREMENT,
	case_id TEXT NOT NULL REFERENCES cases (id),
	seq     INTEGER NOT NULL,
	author  TEXT NOT NULL,
	event   TEXT NOT NULL,
	at      TEXT NOT NULL,
	data    BLOB NOT NULL,
	UNIQUE (case_id, seq)
) STRICT;

CREATE TABLE archive_cases (
	id          TEXT NOT NULL PRIMARY KEY,
	opened_at   TEXT NOT NULL,
	archived_at TEXT NOT NULL
) STRICT;

CREATE TABLE archive_events (
	change  INTEGER PRIMARY KEY,
	case_id TEXT NOT NULL REFERENCES archive_cases (id),
	seq     INTEGER NOT NULL,
	author  TEXT NOT NULL,
	event   TEXT NOT NULL,
	at      TEXT NOT NULL,
	data    BLOB NOT NULL,
	UNIQUE (case_id, seq)
) STRICT;
`

// busyTimeout is how long, in milliseconds, a connection waits for another
// writer to finish before it fails with SQLITE_BUSY. A write holds the lock
// for one load, check and insert.
const busyTimeout = 5000

// maxOpenConns bounds each DB's connection pool. Nothing holds one connection
// while it waits for another, so the bound cannot deadlock.
const maxOpenConns = 4

// notFound is a store or case that is not there. It matches fs.ErrNotExist,
// which callers branch on, and reads as its own message.
type notFound struct{ msg string }

func (e *notFound) Error() string { return e.msg }

func (e *notFound) Is(target error) bool { return target == fs.ErrNotExist }

// DB is the store in the SQLite file at Path. The file is created, with its
// directory, by the first Create; reads of a missing file, or of one no
// cases has written yet, fail with fs.ErrNotExist and write nothing.
//
// A DB opens its connection pool on first use and keeps it until Disconnect.
// It looks at the file before every read and write, and opens a new pool when
// the file at Path has been removed or replaced, so a long-running serve or
// wait follows a store that was thrown away and started again rather than
// writing to the old file. It is safe for concurrent use.
type DB struct {
	Path string

	mu      sync.Mutex
	pool    *sql.DB
	file    os.FileInfo // the file as it was before pool opened it
	gen     int         // raised each time a pool is opened
	retired *sql.DB     // the last pool for a file no longer at Path
}

var _ Store = (*DB)(nil)

// NewDB returns the store in the SQLite file at path. It does not touch the
// file.
func NewDB(path string) *DB {
	return &DB{Path: path}
}

// Disconnect closes the connections to the database file. (Close is the
// close event.) A later read or write opens them again.
func (d *DB) Disconnect() error {
	d.mu.Lock()
	defer d.mu.Unlock()
	var errs []error
	for _, p := range []*sql.DB{d.retired, d.pool} {
		if p != nil {
			errs = append(errs, p.Close())
		}
	}
	d.pool, d.file, d.retired = nil, nil, nil
	return errors.Join(errs...)
}

// open returns the pool for the file at Path and its generation. With create
// it makes the file, its directory and the schema when they are missing;
// without it a missing file, or one with no tables, is fs.ErrNotExist.
func (d *DB) open(ctx context.Context, create bool) (*sql.DB, int, error) {
	d.mu.Lock()
	defer d.mu.Unlock()

	info, err := os.Stat(d.Path)
	switch {
	case err == nil && info.IsDir():
		return nil, 0, fmt.Errorf("store %s is a directory; the store is now one SQLite file, and directory stores are not read: name a file such as %s", d.Path, filepath.Join(d.Path, "cases.db"))
	case err == nil && d.pool != nil && os.SameFile(info, d.file):
		return d.pool, d.gen, nil
	case err != nil && !errors.Is(err, fs.ErrNotExist):
		return nil, 0, err
	}
	// The file is missing, or is not the one the pool has open. A query may be
	// about to start on the pool, so it is retired rather than closed, and the
	// pool retired before it, which has had that chance, is closed. Closing a
	// pool on a removed file does not delete the WAL of a file now at Path.
	if d.pool != nil {
		if old := d.retired; old != nil {
			go func() { _ = old.Close() }()
		}
		d.pool.SetMaxIdleConns(0)
		d.retired, d.pool, d.file = d.pool, nil, nil
	}
	if err != nil && !create {
		return nil, 0, d.missing()
	}
	if err != nil {
		if err := checkLeftovers(d.Path); err != nil {
			return nil, 0, err
		}
		if err := createFile(d.Path); err != nil {
			return nil, 0, err
		}
	}

	// The pool is recorded with the file only when the file at Path is the
	// same one before and after the pool connects to it. Otherwise the pool's
	// connections may be on a file that is no longer there, while every later
	// look at Path finds the file recorded, so the pool is closed and opened
	// again.
	for tries := 1; ; tries++ {
		before, err := os.Stat(d.Path)
		if errors.Is(err, fs.ErrNotExist) {
			return nil, 0, d.missing()
		}
		if err != nil {
			return nil, 0, err
		}
		pool, err := sql.Open("sqlite", dsn(d.Path))
		if err != nil {
			return nil, 0, err
		}
		pool.SetMaxOpenConns(maxOpenConns)
		ready, err := setup(ctx, pool, d.Path, create)
		if err == nil && !ready {
			err = d.missing()
		}
		if err != nil {
			_ = pool.Close()
			return nil, 0, err
		}
		after, err := os.Stat(d.Path)
		if err == nil && os.SameFile(before, after) {
			d.pool, d.file = pool, after
			d.gen++
			return pool, d.gen, nil
		}
		_ = pool.Close()
		if err != nil && !errors.Is(err, fs.ErrNotExist) {
			return nil, 0, err
		}
		if tries == 3 {
			return nil, 0, fmt.Errorf("store %s was replaced three times while it was being opened", d.Path)
		}
	}
}

func (d *DB) missing() error {
	return &notFound{"no store at " + d.Path}
}

// checkLeftovers refuses to make a store at path while a -wal or -shm file of
// a store that was there is still beside it. Those are left when the store
// was removed or moved on its own while a process, such as cases serve, had
// it open. A new store would pair with them by name, and share them with that
// process, and its writes could be lost.
func checkLeftovers(path string) error {
	for _, suffix := range []string{"-wal", "-shm"} {
		if _, err := os.Stat(path + suffix); err != nil {
			continue
		}
		// Another writer may have made the store, and these files with it,
		// since this one found the store missing.
		if _, err := os.Stat(path); err == nil {
			return nil
		}
		return fmt.Errorf("no store at %s, but %s from an earlier store is still there: stop cases serve and any cases wait, then delete %s-wal and %s-shm, or put the store back", path, path+suffix, path, path)
	}
	return nil
}

// createFile makes an empty file at path, and its directory. An empty file
// is an empty SQLite database. A file that is already there is left alone:
// another writer made it first.
func createFile(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	f, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE|os.O_EXCL, 0o644)
	if errors.Is(err, fs.ErrExist) {
		return nil
	}
	if err != nil {
		return err
	}
	return f.Close()
}

// dsn is the connection string for the file at path. mode=rw never creates
// the file, which createFile does. Write transactions begin immediate, taking
// the write lock before they read, so two writers wait for each other rather
// than both reading the same case and one failing. synchronous=FULL syncs
// the WAL on every commit, and fullfsync makes that an F_FULLFSYNC on macOS,
// so a write that has returned survives a power loss.
func dsn(path string) string {
	abs, err := filepath.Abs(path)
	if err != nil {
		abs = path
	}
	abs = filepath.ToSlash(abs)
	if runtime.GOOS == "windows" && !strings.HasPrefix(abs, "/") {
		abs = "/" + abs
	}
	q := url.Values{}
	q.Set("mode", "rw")
	q.Set("_txlock", "immediate")
	q.Set("_busy_timeout", fmt.Sprint(busyTimeout))
	q.Set("_foreign_keys", "1")
	q.Set("_synchronous", "FULL")
	q.Add("_pragma", "fullfsync(1)")
	return (&url.URL{Scheme: "file", Path: abs, RawQuery: q.Encode()}).String()
}

// setup checks the schema in the file and, with create, makes it in an empty
// file. It reports whether the file holds a cases store. A file with other
// tables and no meta table is not a cases store and is refused, as is a
// schema newer than this build reads.
func setup(ctx context.Context, pool *sql.DB, path string, create bool) (ready bool, err error) {
	ready, err = checkSchema(ctx, pool, path)
	if err != nil || ready || !create {
		return ready, err
	}
	if err := useWAL(ctx, pool, path); err != nil {
		return false, err
	}
	tx, err := pool.BeginTx(ctx, nil)
	if err != nil {
		return false, err
	}
	defer func() { _ = tx.Rollback() }()
	// Another writer may have made the schema while this one waited.
	if ready, err = checkSchema(ctx, tx, path); err != nil || ready {
		return ready, err
	}
	if _, err := tx.ExecContext(ctx, schema); err != nil {
		return false, err
	}
	if _, err := tx.ExecContext(ctx, "INSERT INTO meta (schema_version) VALUES (?)", schemaVersion); err != nil {
		return false, err
	}
	return true, tx.Commit()
}

// useWAL switches the file to write-ahead logging, which lets reads go on
// while a write happens. The mode is kept in the file, so it is set once,
// before the first table. The switch turns a read lock into a write lock, and
// SQLite refuses that at once with SQLITE_BUSY rather than waiting out the
// busy timeout, so a writer that races another to make the store tries again
// until the timeout. Once the other writer has switched, the mode is set.
func useWAL(ctx context.Context, pool *sql.DB, path string) error {
	deadline := time.Now().Add(busyTimeout * time.Millisecond)
	for {
		var mode string
		err := pool.QueryRowContext(ctx, "PRAGMA journal_mode = WAL").Scan(&mode)
		var serr *sqlite.Error
		if errors.As(err, &serr) && serr.Code()&0xff == sqlite3.SQLITE_BUSY && time.Now().Before(deadline) {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(10 * time.Millisecond):
			}
			continue
		}
		if err != nil {
			return err
		}
		if !strings.EqualFold(mode, "wal") {
			return fmt.Errorf("store %s: cannot use write-ahead logging (journal mode is %s)", path, mode)
		}
		return nil
	}
}

// querier is a pool or a transaction.
type querier interface {
	QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error)
	QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row
}

// checkSchema reports whether the database holds the cases schema at a
// version this build reads. No tables at all is not an error: the file is
// new.
func checkSchema(ctx context.Context, q querier, path string) (bool, error) {
	var tables int
	var hasMeta bool
	err := q.QueryRowContext(ctx, `SELECT count(*), coalesce(sum(name = 'meta'), 0) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\_%' ESCAPE '\'`).Scan(&tables, &hasMeta)
	if err != nil {
		return false, fmt.Errorf("store %s: %w", path, err)
	}
	if tables == 0 {
		return false, nil
	}
	if !hasMeta {
		return false, fmt.Errorf("store %s is not a cases store: it has tables but no meta table", path)
	}
	return true, checkVersion(ctx, q, path)
}

// checkVersion refuses a store whose schema version this build does not
// read. Writes and polls check it again in their transactions, so a serve or
// wait that is running when a newer cases changes the schema stops there.
func checkVersion(ctx context.Context, q querier, path string) error {
	var version int
	switch err := q.QueryRowContext(ctx, "SELECT schema_version FROM meta").Scan(&version); {
	case errors.Is(err, sql.ErrNoRows):
		return fmt.Errorf("store %s is damaged: its meta table has no schema version", path)
	case err != nil:
		return fmt.Errorf("store %s: %w", path, err)
	case version > schemaVersion:
		return fmt.Errorf("store %s has schema version %d, newer than this cases reads (%d): update cases on this machine with go install github.com/ryanlewis/cases/cmd/cases@latest", path, version, schemaVersion)
	case version < 1:
		return fmt.Errorf("store %s is damaged: schema version %d", path, version)
	}
	return nil
}
