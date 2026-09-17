// Package storetest reads and writes a case store's tables directly, with
// none of the checks the store makes, as a newer cases or a hand edit could,
// so tests can check how the CLI and the web inbox show what they find. Only
// tests import it.
package storetest

import (
	"database/sql"
	"net/url"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	_ "modernc.org/sqlite" // registers the "sqlite" driver
)

// open connects to the store at path, which must exist, for one test step.
// It cannot use the store's own connection string, as the store's tests use
// this package.
func open(t testing.TB, path string) *sql.DB {
	t.Helper()
	abs, err := filepath.Abs(path)
	if err != nil {
		t.Fatal(err)
	}
	abs = filepath.ToSlash(abs)
	if runtime.GOOS == "windows" && !strings.HasPrefix(abs, "/") {
		abs = "/" + abs
	}
	dsn := (&url.URL{Scheme: "file", Path: abs, RawQuery: "mode=rw&_busy_timeout=5000&_foreign_keys=1"}).String()
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		t.Fatal(err)
	}
	return db
}

// InsertEvent stores an event row on the case id in the store at path, as
// given, adding the case when the store has no case with that id.
func InsertEvent(t testing.TB, path, id string, seq int, author, event, data string) {
	t.Helper()
	db := open(t, path)
	defer func() { _ = db.Close() }()
	at := time.Now().UTC().Format(time.RFC3339Nano)
	if _, err := db.ExecContext(t.Context(), `INSERT OR IGNORE INTO cases (id, opened_at) VALUES (?, ?)`, id, at); err != nil {
		t.Fatal(err)
	}
	if _, err := db.ExecContext(t.Context(), `INSERT INTO events (case_id, seq, author, event, at, data) VALUES (?, ?, ?, ?, ?, ?)`, id, seq, author, event, at, []byte(data)); err != nil {
		t.Fatal(err)
	}
}

// Archived maps each case id in the archive of the store at path to the
// number of events archived with it.
func Archived(t testing.TB, path string) map[string]int {
	t.Helper()
	db := open(t, path)
	defer func() { _ = db.Close() }()
	rows, err := db.QueryContext(t.Context(), `SELECT c.id, count(e.case_id) FROM archive_cases c LEFT JOIN archive_events e ON e.case_id = c.id GROUP BY c.id`)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = rows.Close() }()
	archived := map[string]int{}
	for rows.Next() {
		var id string
		var n int
		if err := rows.Scan(&id, &n); err != nil {
			t.Fatal(err)
		}
		archived[id] = n
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return archived
}
