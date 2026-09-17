package store

import (
	"database/sql"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store/storetest"
)

// newDB returns a store in a file of its own that does not exist yet.
func newDB(t *testing.T) *DB {
	t.Helper()
	return openDB(t, filepath.Join(t.TempDir(), "cases.db"))
}

// openDB returns the store at path, disconnected when the test ends.
func openDB(t *testing.T, path string) *DB {
	t.Helper()
	d := NewDB(path)
	t.Cleanup(func() { _ = d.Disconnect() })
	return d
}

// pool returns the store's connection pool, for a test to read or write rows
// directly. The store must exist.
func pool(t *testing.T, d *DB) *sql.DB {
	t.Helper()
	p, _, err := d.open(t.Context(), false)
	if err != nil {
		t.Fatal(err)
	}
	return p
}

// insertRow stores an event row as given, with none of the checks a write
// makes, as a newer cases or a hand edit could. It adds the case row when the
// store has none.
func insertRow(t *testing.T, d *DB, id string, seq int, author, event, data string) {
	t.Helper()
	storetest.InsertEvent(t, d.Path, id, seq, author, event, data)
}

// storedRow is an event row as the events table holds it.
type storedRow struct {
	Seq    int
	Author string
	Event  string
	Data   string
}

// storedRows reads the case's event rows in sequence order.
func storedRows(t *testing.T, d *DB, id string) []storedRow {
	t.Helper()
	return tableRows(t, d, "events", id)
}

// tableRows reads the case's rows from events or archive_events in sequence
// order.
func tableRows(t *testing.T, d *DB, table, id string) []storedRow {
	t.Helper()
	rows, err := pool(t, d).QueryContext(t.Context(), `SELECT seq, author, event, data FROM `+table+` WHERE case_id = ? ORDER BY seq`, id)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var out []storedRow
	for rows.Next() {
		var r storedRow
		var data []byte
		if err := rows.Scan(&r.Seq, &r.Author, &r.Event, &data); err != nil {
			t.Fatal(err)
		}
		r.Data = string(data)
		out = append(out, r)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return out
}

// names are the file names the rows would have had.
func names(rows []storedRow) []string {
	var out []string
	for _, r := range rows {
		out = append(out, fileName(r.Seq, Author(r.Author), EventType(r.Event)))
	}
	return out
}

// TestStoreContract runs a case through every Store method, by id, and checks
// the errors callers branch on.
func TestStoreContract(t *testing.T) {
	for name, newStore := range map[string]func(t *testing.T) Store{
		"DB": func(t *testing.T) Store { return newDB(t) },
	} {
		t.Run(name, func(t *testing.T) {
			t.Run("lifecycle", func(t *testing.T) { testStoreLifecycle(t, newStore(t)) })
			t.Run("errors", func(t *testing.T) { testStoreErrors(t, newStore(t), newStore(t)) })
		})
	}
}

func testStoreLifecycle(t *testing.T, s Store) {
	ctx := t.Context()
	stuck, err := s.Create(ctx, OpenRecord{Kind: KindStuck, Urgency: UrgencyToday, Title: "Stuck on CI"})
	if err != nil {
		t.Fatal(err)
	}
	decision, err := s.Create(ctx, OpenRecord{Kind: KindDecision, Urgency: UrgencyToday, Title: "Pin bun?", Options: []string{"Pin", "Float"}})
	if err != nil {
		t.Fatal(err)
	}
	id := decision.ID

	steps := []struct {
		name string
		want State
		do   func() (*Case, error)
	}{
		{"amend", StateOpen, func() (*Case, error) { return s.Amend(ctx, id, AmendRecord{Labels: []string{"deps"}}) }},
		{"answer", StateAnswered, func() (*Case, error) { return s.Answer(ctx, id, AnswerRecord{Choice: 1}, AtRevision(2)) }},
		{"pickup", StatePickedUp, func() (*Case, error) { return s.Pickup(ctx, id, PickupRecord{By: "test"}) }},
		{"note", StateOpen, func() (*Case, error) { return s.Note(ctx, id, NoteRecord{Body: "Which patch?"}) }},
		{"answer again", StateAnswered, func() (*Case, error) { return s.Answer(ctx, id, AnswerRecord{Choice: 2}) }},
		{"pickup again", StatePickedUp, func() (*Case, error) { return s.Pickup(ctx, id, PickupRecord{}) }},
		{"close", StateClosed, func() (*Case, error) { return s.Close(ctx, id, CloseRecord{Outcome: "Floated."}) }},
		{"park", StateParked, func() (*Case, error) { return s.Park(ctx, stuck.ID, ParkRecord{}) }},
		{"resume", StateOpen, func() (*Case, error) { return s.Resume(ctx, stuck.ID, AuthorHuman, ResumeRecord{}) }},
		{"withdraw", StateWithdrawn, func() (*Case, error) { return s.Withdraw(ctx, stuck.ID, WithdrawRecord{}) }},
	}
	for i, st := range steps {
		c, err := st.do()
		if err != nil {
			t.Fatalf("%s: %v", st.name, err)
		}
		if c.State != st.want {
			t.Fatalf("%s: state %s, want %s", st.name, c.State, st.want)
		}
		// The case a write returns is the case as stored, event and all.
		if got, err := s.Get(ctx, c.ID); err != nil || got.State != c.State || got.Revision() != c.Revision() || len(got.Events) != len(c.Events) {
			t.Fatalf("step %d %s: Get = %+v, %v; the write returned %+v", i, st.name, got, err, c)
		}
	}

	got, err := s.Get(ctx, id)
	if err != nil || got.State != StateClosed || got.Revision() != 8 {
		t.Fatalf("Get = %+v, %v; want closed at revision 8", got, err)
	}
	wantFiles := []string{"0001-agent-open.json", "0002-agent-amend.json", "0003-human-answer.json", "0004-agent-pickup.json",
		"0005-agent-note.json", "0006-human-answer.json", "0007-agent-pickup.json", "0008-agent-close.json"}
	var files []string
	for _, ev := range got.Events {
		files = append(files, ev.File)
	}
	if !slices.Equal(files, wantFiles) {
		t.Errorf("event files = %v", files)
	}
	ids, err := s.IDs(ctx)
	if want := slices.Sorted(slices.Values([]string{decision.ID, stuck.ID})); err != nil || !slices.Equal(ids, want) {
		t.Errorf("IDs = %v, %v", ids, err)
	}
	cases, bad, err := s.List(ctx)
	if err != nil || len(cases) != 2 || len(bad) != 0 || cases[0].ID > cases[1].ID {
		t.Errorf("List = %d cases, %v, %v", len(cases), bad, err)
	}
	if cases, _, err := s.NewPoller().Poll(); err != nil || len(cases) != 2 {
		t.Errorf("Poll = %d cases, %v", len(cases), err)
	}
}

// testStoreErrors checks the errors callers branch on. absent is a store
// nothing has been written to.
func testStoreErrors(t *testing.T, s, absent Store) {
	ctx := t.Context()
	c, err := s.Create(ctx, OpenRecord{Kind: KindFYI, Urgency: UrgencyWhenever, Title: "Heads up"})
	if err != nil {
		t.Fatal(err)
	}

	if _, err := s.Get(ctx, "2026-01-01T00-00-00Z-missing"); !errors.Is(err, fs.ErrNotExist) || err.Error() != `no case "2026-01-01T00-00-00Z-missing"` {
		t.Errorf("Get missing: %v, want fs.ErrNotExist", err)
	}
	if _, err := s.Pickup(ctx, "2026-01-01T00-00-00Z-missing", PickupRecord{}); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Pickup missing: %v, want fs.ErrNotExist", err)
	}
	if _, err := s.Get(ctx, "../elsewhere"); err == nil || err.Error() != `invalid case id "../elsewhere"` {
		t.Errorf("Get invalid id: %v", err)
	}
	if _, err := s.Pickup(ctx, "../elsewhere", PickupRecord{}); err == nil || err.Error() != `invalid case id "../elsewhere"` {
		t.Errorf("Pickup invalid id: %v", err)
	}
	if _, err := s.Answer(ctx, c.ID, AnswerRecord{Ack: true}, AtRevision(0)); !errors.Is(err, ErrStale) {
		t.Errorf("stale answer: %v, want ErrStale", err)
	}
	var te *TransitionError
	if _, err := s.Close(ctx, c.ID, CloseRecord{Outcome: "done"}); !errors.As(err, &te) {
		t.Errorf("close an open case: %v, want a TransitionError", err)
	}

	if _, _, err := absent.List(ctx); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("List absent store: %v, want fs.ErrNotExist", err)
	}
	if _, err := absent.IDs(ctx); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("IDs absent store: %v, want fs.ErrNotExist", err)
	}
	if _, err := absent.Get(ctx, c.ID); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Get absent store: %v, want fs.ErrNotExist", err)
	}
	if _, _, err := absent.NewPoller().Poll(); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Poll absent store: %v, want fs.ErrNotExist", err)
	}
	if _, err := absent.Answer(ctx, c.ID, AnswerRecord{Ack: true}); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Answer absent store: %v, want fs.ErrNotExist", err)
	}
}

// Reads, and writes to a case, never make the file; the first Create makes
// it and its directory.
func TestOnlyCreateMakesTheStore(t *testing.T) {
	ctx := t.Context()
	path := filepath.Join(t.TempDir(), "new", "dir", "cases.db")
	d := openDB(t, path)
	d.List(ctx)
	d.IDs(ctx)
	d.Get(ctx, "2026-01-01T00-00-00Z-x")
	d.NewPoller().Poll()
	d.Pickup(ctx, "2026-01-01T00-00-00Z-x", PickupRecord{})
	if _, err := os.Stat(filepath.Dir(path)); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("a read made the store's directory: %v", err)
	}
	if _, err := d.Create(ctx, openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Stat(path); err != nil || info.Mode().Perm() != 0o644 {
		t.Fatalf("store file after Create: %v, %v", info, err)
	}
}

// A file that exists but that no cases has written to, such as one being made
// by another writer, reads as a missing store and is left as it was.
func TestAnEmptyFileReadsAsNoStore(t *testing.T) {
	path := filepath.Join(t.TempDir(), "cases.db")
	if err := os.WriteFile(path, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	d := openDB(t, path)
	if _, _, err := d.List(t.Context()); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("List: %v, want fs.ErrNotExist", err)
	}
	if info, err := os.Stat(path); err != nil || info.Size() != 0 {
		t.Errorf("the read wrote to the file: %v, %v", info, err)
	}
	if _, err := d.Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatalf("Create in the empty file: %v", err)
	}
	if cases, _, err := d.List(t.Context()); err != nil || len(cases) != 1 {
		t.Errorf("List after Create = %d cases, %v", len(cases), err)
	}
}

// Two writers making the store at once: one switches the new file to
// write-ahead logging while the other holds the file's write lock. SQLite
// refuses the switch at once rather than waiting, so Create must wait for the
// other writer itself.
func TestCreateWaitsForAnotherWriterMakingTheStore(t *testing.T) {
	path := filepath.Join(t.TempDir(), "cases.db")
	if err := os.WriteFile(path, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	other, err := sql.Open("sqlite", dsn(path))
	if err != nil {
		t.Fatal(err)
	}
	defer other.Close()
	tx, err := other.BeginTx(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	release := time.AfterFunc(200*time.Millisecond, func() { _ = tx.Rollback() })
	defer release.Stop()

	if _, err := openDB(t, path).Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatalf("Create while another writer held the new file: %v", err)
	}
}

// A store removed on its own while a process still has it open, as when
// cases.db is deleted while cases serve runs, leaves its -wal and -shm files
// behind. A new store must not pair with them.
func TestCreateRefusesTheFilesOfARemovedStore(t *testing.T) {
	ctx := t.Context()
	path := filepath.Join(t.TempDir(), "cases.db")
	serving := openDB(t, path)
	if _, err := serving.Create(ctx, openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	if _, _, err := serving.NewPoller().Poll(); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}

	agent := openDB(t, path)
	if _, err := agent.Create(ctx, openOf(KindFYI)); err == nil || !strings.Contains(err.Error(), "-wal from an earlier store is still there") {
		t.Fatalf("Create beside a removed store's files: %v, want a refusal naming them", err)
	}
	if _, err := os.Stat(path); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("the refused Create made the store: %v", err)
	}

	for _, suffix := range []string{"-wal", "-shm"} {
		if err := os.Remove(path + suffix); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := agent.Create(ctx, openOf(KindFYI)); err != nil {
		t.Fatalf("Create once the files are gone: %v", err)
	}
}

// A serve or wait that is running when a newer cases changes the schema
// stops writing and reading there, as a command started afterwards would.
func TestARunningDBRefusesANewerSchema(t *testing.T) {
	ctx := t.Context()
	d := newDB(t)
	c, err := d.Create(ctx, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	p := d.NewPoller()
	if _, _, err := p.Poll(); err != nil {
		t.Fatal(err)
	}
	if _, err := pool(t, openDB(t, d.Path)).ExecContext(ctx, "UPDATE meta SET schema_version = 2"); err != nil {
		t.Fatal(err)
	}
	const refusal = "has schema version 2, newer than this cases reads (1)"
	if _, err := d.Answer(ctx, c.ID, AnswerRecord{Ack: true}); err == nil || !strings.Contains(err.Error(), refusal) {
		t.Errorf("Answer: %v", err)
	}
	if _, _, err := p.Poll(); err == nil || !strings.Contains(err.Error(), refusal) {
		t.Errorf("Poll: %v", err)
	}
	if got := storedRows(t, d, c.ID); len(got) != 1 {
		t.Errorf("rows = %v", names(got))
	}
}

// Each time the file at the path is replaced, the pool on the old file is
// retired, and the one retired before it is closed, so a long-running serve
// holds at most one old pool.
func TestRetiredPoolsAreClosed(t *testing.T) {
	ctx := t.Context()
	d := newDB(t)
	if _, err := d.Create(ctx, openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	var pools []*sql.DB
	for range 3 {
		pools = append(pools, pool(t, d))
		for _, suffix := range []string{"", "-wal", "-shm"} {
			if err := os.Remove(d.Path + suffix); err != nil && !errors.Is(err, fs.ErrNotExist) {
				t.Fatal(err)
			}
		}
		again := NewDB(d.Path)
		if _, err := again.Create(ctx, openOf(KindFYI)); err != nil {
			t.Fatal(err)
		}
		if err := again.Disconnect(); err != nil {
			t.Fatal(err)
		}
		if _, err := d.IDs(ctx); err != nil {
			t.Fatal(err)
		}
	}
	if d.retired != pools[2] {
		t.Fatal("the last pool replaced is not the one retired")
	}
	for i, p := range pools[:2] {
		for deadline := time.Now().Add(5 * time.Second); ; time.Sleep(10 * time.Millisecond) {
			err := p.PingContext(ctx)
			if err != nil && strings.Contains(err.Error(), "database is closed") {
				break
			}
			if time.Now().After(deadline) {
				t.Fatalf("pool %d is still open: ping = %v", i, err)
			}
		}
	}
}

func TestTheSchema(t *testing.T) {
	d := newDB(t)
	if _, err := d.Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	p := pool(t, d)
	var mode string
	var version, foreignKeys, synchronous, fullfsync, busy int
	for _, q := range []struct {
		query string
		dest  any
	}{
		{"PRAGMA journal_mode", &mode},
		{"SELECT schema_version FROM meta", &version},
		{"PRAGMA foreign_keys", &foreignKeys},
		{"PRAGMA synchronous", &synchronous},
		{"PRAGMA fullfsync", &fullfsync},
		{"PRAGMA busy_timeout", &busy},
	} {
		if err := p.QueryRowContext(t.Context(), q.query).Scan(q.dest); err != nil {
			t.Fatalf("%s: %v", q.query, err)
		}
	}
	if mode != "wal" || version != 1 || foreignKeys != 1 || synchronous != 2 || fullfsync != 1 || busy != busyTimeout {
		t.Errorf("journal_mode %s, schema_version %d, foreign_keys %d, synchronous %d, fullfsync %d, busy_timeout %d", mode, version, foreignKeys, synchronous, fullfsync, busy)
	}
	// Foreign keys hold: an event needs its case.
	if _, err := p.ExecContext(t.Context(), `INSERT INTO events (case_id, seq, author, event, at, data) VALUES ('nope', 1, 'agent', 'open', '', x'7b7d')`); err == nil || !strings.Contains(err.Error(), "FOREIGN KEY") {
		t.Errorf("event without a case: %v, want a foreign key error", err)
	}
	// One number per case.
	ids, _ := d.IDs(t.Context())
	if _, err := p.ExecContext(t.Context(), `INSERT INTO events (case_id, seq, author, event, at, data) VALUES (?, 1, 'agent', 'note', '', x'7b7d')`, ids[0]); err == nil || !strings.Contains(err.Error(), "UNIQUE") {
		t.Errorf("a second event 0001: %v, want a unique constraint error", err)
	}
}

func TestOpenRefusesWhatIsNotThisStore(t *testing.T) {
	ctx := t.Context()
	t.Run("newer schema", func(t *testing.T) {
		d := newDB(t)
		if _, err := d.Create(ctx, openOf(KindFYI)); err != nil {
			t.Fatal(err)
		}
		if _, err := pool(t, d).ExecContext(ctx, "UPDATE meta SET schema_version = 2"); err != nil {
			t.Fatal(err)
		}
		// A new DB on the file, as the next command would open.
		for _, err := range []error{
			func() error { _, _, err := openDB(t, d.Path).List(ctx); return err }(),
			func() error { _, err := openDB(t, d.Path).Create(ctx, openOf(KindFYI)); return err }(),
		} {
			if err == nil || !strings.Contains(err.Error(), "has schema version 2, newer than this cases reads (1): update cases") {
				t.Errorf("err = %v", err)
			}
		}
	})
	t.Run("a directory", func(t *testing.T) {
		dir := t.TempDir()
		_, _, err := openDB(t, dir).List(ctx)
		if err == nil || !strings.Contains(err.Error(), "is a directory") || !strings.Contains(err.Error(), "directory stores are not read") {
			t.Errorf("List: %v", err)
		}
		if _, err := openDB(t, dir).Create(ctx, openOf(KindFYI)); err == nil || !strings.Contains(err.Error(), "is a directory") {
			t.Errorf("Create: %v", err)
		}
	})
	t.Run("another database", func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "other.db")
		other, err := sql.Open("sqlite", path)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := other.ExecContext(ctx, "CREATE TABLE notes (body TEXT)"); err != nil {
			t.Fatal(err)
		}
		other.Close()
		if _, err := openDB(t, path).Create(ctx, openOf(KindFYI)); err == nil || !strings.Contains(err.Error(), "is not a cases store") {
			t.Errorf("Create: %v", err)
		}
	})
	t.Run("not a database", func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "cases.db")
		if err := os.WriteFile(path, []byte("these are notes, not a database\n"), 0o644); err != nil {
			t.Fatal(err)
		}
		if _, _, err := openDB(t, path).List(ctx); err == nil || errors.Is(err, fs.ErrNotExist) {
			t.Errorf("List: %v, want an error that is not a missing store", err)
		}
	})
}

// A serve or wait that outlives its store follows the file at the path: a
// store thrown away reads as missing, and one started again in its place is
// read and written, not the old file it had open.
func TestDBFollowsTheFileAtThePath(t *testing.T) {
	ctx := t.Context()
	path := filepath.Join(t.TempDir(), "cases.db")
	d := openDB(t, path)
	old, err := d.Create(ctx, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	poller := d.NewPoller()
	if cases, _, err := poller.Poll(); err != nil || len(cases) != 1 {
		t.Fatalf("Poll = %d cases, %v", len(cases), err)
	}

	for _, suffix := range []string{"", "-wal", "-shm"} {
		if err := os.Remove(path + suffix); err != nil && !errors.Is(err, fs.ErrNotExist) {
			t.Fatal(err)
		}
	}
	if _, err := d.Get(ctx, old.ID); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Get from a removed store: %v, want fs.ErrNotExist", err)
	}
	if _, _, err := poller.Poll(); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Poll of a removed store: %v, want fs.ErrNotExist", err)
	}
	if _, err := d.Answer(ctx, old.ID, AnswerRecord{Ack: true}); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("Answer in a removed store: %v, want fs.ErrNotExist", err)
	}

	// Another process starts the store again.
	other := openDB(t, path)
	fresh, err := other.Create(ctx, OpenRecord{Kind: KindStuck, Urgency: UrgencyToday, Title: "Started again"})
	if err != nil {
		t.Fatal(err)
	}
	if other.Disconnect() != nil {
		t.Fatal("disconnect")
	}
	cases, _, err := poller.Poll()
	if err != nil || len(cases) != 1 || cases[0].ID != fresh.ID {
		t.Fatalf("Poll after the store started again = %v, %v; want only %s", cases, err, fresh.ID)
	}
	if _, err := d.Park(ctx, fresh.ID, ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	if got, err := openDB(t, path).Get(ctx, fresh.ID); err != nil || got.State != StateParked {
		t.Errorf("the park did not reach the file at the path: %+v, %v", got, err)
	}
}
