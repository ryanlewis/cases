package store

import (
	"database/sql"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"slices"
	"strings"
	"testing"
	"time"

	"modernc.org/sqlite"
	sqlite3 "modernc.org/sqlite/lib"
)

func TestLoadToleratesMalformedRows(t *testing.T) {
	d := newDB(t)
	c, err := d.Create(t.Context(), openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	insertRow(t, d, c.ID, 2, "human", "answer", `{"ack": tru`)
	insertRow(t, d, c.ID, 3, "bob", "note", `{"body":"hi"}`)

	loaded, err := d.Get(t.Context(), c.ID)
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if loaded.State != StateOpen || loaded.Revision() != 3 || len(loaded.Events) != 1 {
		t.Errorf("state = %s, revision %d, %d events", loaded.State, loaded.Revision(), len(loaded.Events))
	}
	problems := strings.Join(loaded.Problems, "\n")
	for _, want := range []string{"0002-human-answer.json: malformed answer event", "0003-bob-note.json: note events are not written by the bob"} {
		if !strings.Contains(problems, want) {
			t.Errorf("problems %q missing %q", problems, want)
		}
	}

	// The next write goes after the malformed rows, not on top of them.
	after, err := d.Answer(t.Context(), c.ID, AnswerRecord{Ack: true})
	if err != nil {
		t.Fatal(err)
	}
	if last := after.Events[len(after.Events)-1]; last.File != "0004-human-answer.json" || after.Revision() != 4 {
		t.Errorf("next event = %s at revision %d", last.File, after.Revision())
	}
}

func TestLoadPreservesUnknownFields(t *testing.T) {
	d := newDB(t)
	if _, err := d.Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	const id = "2026-09-15T09-00-00Z-x"
	insertRow(t, d, id, 1, "agent", "open", `{"kind":"fyi","urgency":"today","title":"x","opened_at":"2026-09-15T09:00:00Z","colour":"teal","extra":{"n":1}}`)

	c, err := d.Get(t.Context(), id)
	if err != nil {
		t.Fatal(err)
	}
	if len(c.Problems) != 0 {
		t.Errorf("problems = %v", c.Problems)
	}
	var data map[string]any
	if err := json.Unmarshal(c.Events[0].Data, &data); err != nil {
		t.Fatal(err)
	}
	if data["colour"] != "teal" || data["extra"] == nil {
		t.Errorf("data = %v, want unknown fields kept", data)
	}
	out, err := json.Marshal(c)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(out), `"colour":"teal"`) {
		t.Errorf("case JSON dropped unknown field: %s", out)
	}
}

func TestLoadWithoutOpenFails(t *testing.T) {
	d := newDB(t)
	if _, err := d.Create(t.Context(), openOf(KindFYI)); err != nil {
		t.Fatal(err)
	}
	const empty = "2026-01-01T00-00-00Z-empty"
	if _, err := pool(t, d).ExecContext(t.Context(), `INSERT INTO cases (id, opened_at) VALUES (?, '')`, empty); err != nil {
		t.Fatal(err)
	}
	if _, err := d.Get(t.Context(), empty); err == nil || err.Error() != "no open event" {
		t.Errorf("case with no events: err = %v", err)
	}
	const broken = "2026-01-01T00-00-01Z-broken"
	insertRow(t, d, broken, 1, "agent", "open", `not json`)
	if _, err := d.Get(t.Context(), broken); err == nil || !strings.Contains(err.Error(), "no valid open event") {
		t.Errorf("malformed open: err = %v", err)
	}
}

func TestListReportsBrokenCases(t *testing.T) {
	d := newDB(t)
	good, err := d.Create(t.Context(), openOf(KindDecision))
	if err != nil {
		t.Fatal(err)
	}
	insertRow(t, d, "2026-01-01T00-00-00Z-broken", 1, "agent", "open", `{"kind":`)

	cases, bad, err := d.List(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if len(cases) != 1 || cases[0].ID != good.ID {
		t.Errorf("cases = %v", cases)
	}
	if len(bad) != 1 || bad[0].ID != "2026-01-01T00-00-00Z-broken" || !strings.HasPrefix(bad[0].Error(), "2026-01-01T00-00-00Z-broken: no valid open event") {
		t.Errorf("bad = %v", bad)
	}
	if pcases, pbad, err := d.NewPoller().Poll(); err != nil || len(pcases) != 1 || len(pbad) != 1 || pbad[0].Error() != bad[0].Error() {
		t.Errorf("poller cases %v, bad = %v, err = %v", pcases, pbad, err)
	}
}

func TestIDs(t *testing.T) {
	d := newDB(t)
	var want []string
	for _, title := range []string{"B case", "A case"} {
		c, err := d.Create(t.Context(), OpenRecord{Kind: KindFYI, Urgency: UrgencyToday, Title: title, OpenedAt: time.Date(2026, 9, 15, 9, 0, 0, 0, time.UTC)})
		if err != nil {
			t.Fatal(err)
		}
		want = append(want, c.ID)
	}
	slices.Sort(want)
	if ids, err := d.IDs(t.Context()); err != nil || !slices.Equal(ids, want) {
		t.Errorf("IDs = %v, %v; want %v", ids, err, want)
	}
}

// poller returns a Poller over d.
func poller(d *DB) *Poller {
	return d.NewPoller().(*Poller)
}

// states polls and maps each case id to its state.
func states(t *testing.T, p *Poller) map[string]State {
	t.Helper()
	cases, bad, err := p.Poll()
	if err != nil || len(bad) > 0 {
		t.Fatalf("Poll: %v, bad %v", err, bad)
	}
	out := map[string]State{}
	for _, c := range cases {
		out[c.ID] = c.State
	}
	return out
}

func TestPollerSeesNewEventsAndCases(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	first, err := d.Create(ctx, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	p := poller(d)
	if got := states(t, p); len(got) != 1 || got[first.ID] != StateOpen {
		t.Fatalf("first poll: %v", got)
	}

	if _, err := d.Answer(ctx, first.ID, AnswerRecord{Ack: true}); err != nil {
		t.Fatal(err)
	}
	second, err := openDB(t, d.Path).Create(ctx, openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	if got := states(t, p); got[first.ID] != StateAnswered || got[second.ID] != StateOpen {
		t.Errorf("states = %v", got)
	}

	// With nothing written, the cases come from cache.
	cached := p.cache[first.ID].c
	states(t, p)
	if p.cache[first.ID].c != cached {
		t.Error("unchanged case was folded again")
	}
	// A write to one case folds only that case again.
	if _, err := d.Park(ctx, second.ID, ParkRecord{}); err != nil {
		t.Fatal(err)
	}
	if got := states(t, p); got[second.ID] != StateParked {
		t.Errorf("states = %v", got)
	}
	if p.cache[first.ID].c != cached {
		t.Error("a write to another case folded this one again")
	}

	if err := d.Delete(ctx, second.ID); err != nil {
		t.Fatal(err)
	}
	if got := states(t, p); len(got) != 1 || got[first.ID] != StateAnswered {
		t.Errorf("after delete: %v", got)
	}
}

// Taking out the case that holds the highest change and putting another in
// changes the case ids, which the poller's first check reads. The poller then
// folds again only the cases with an event above the highest change it had,
// so an event written to a case that stays must still get a change above it:
// a change is never used twice, even once the case that held it is gone. The
// write comes before the new case, which would otherwise take that change.
func TestPollerSeesACaseReplacedByAnother(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	older, err := d.Create(ctx, openOf(KindStuck))
	if err != nil {
		t.Fatal(err)
	}
	newest, err := d.Create(ctx, openOf(KindQuestion))
	if err != nil {
		t.Fatal(err)
	}
	p := poller(d)
	if got := states(t, p); len(got) != 2 {
		t.Fatalf("first poll: %v", got)
	}
	for _, step := range []struct {
		remove func(id string) error
		write  func() error
		want   State
	}{
		{
			func(id string) error { return d.Delete(ctx, id) },
			func() error { _, err := d.Park(ctx, older.ID, ParkRecord{}); return err },
			StateParked,
		},
		{
			func(id string) error { return d.Archive(ctx, id) },
			func() error { _, err := d.Resume(ctx, older.ID, AuthorHuman, ResumeRecord{}); return err },
			StateOpen,
		},
	} {
		if err := step.remove(newest.ID); err != nil {
			t.Fatal(err)
		}
		if err := step.write(); err != nil {
			t.Fatal(err)
		}
		replaced, err := d.Create(ctx, OpenRecord{Kind: KindStuck, Urgency: UrgencyToday, Title: "Replacement " + newest.ID})
		if err != nil {
			t.Fatal(err)
		}
		got := states(t, p)
		if len(got) != 2 || got[older.ID] != step.want || got[replaced.ID] != StateOpen {
			t.Fatalf("after replacing %s: %v, want %s %s", newest.ID, got, older.ID, step.want)
		}
		newest = replaced
	}
}

// A case put back from the archive by hand keeps the change its events had,
// which is below the highest the poller has seen. The poller still finds it,
// as a case it has not seen.
func TestPollerSeesACasePutBackFromTheArchive(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	c, err := d.Create(ctx, openOf(KindFYI))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := d.Create(ctx, openOf(KindStuck)); err != nil {
		t.Fatal(err)
	}
	if err := d.Archive(ctx, c.ID); err != nil {
		t.Fatal(err)
	}
	p := poller(d)
	if got := states(t, p); len(got) != 1 {
		t.Fatalf("after archive: %v", got)
	}
	putBack(t, d, c.ID)
	if got := states(t, p); len(got) != 2 || got[c.ID] != StateOpen {
		t.Errorf("after putting it back: %v", got)
	}
}

// Between two polls, one case is moved to the archive, as prune does, and
// another is put back from it. The number of cases is as it was, and so is the
// highest change: the case that holds it stays, and the case put back keeps
// the changes its events had. The case taken out is the newest row, so the
// case put back gets its rowid. The poller must still see the swap.
func TestPollerSeesACaseArchivedAndAnotherPutBack(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	var ids []string
	for _, kind := range []Kind{KindFYI, KindStuck, KindQuestion} {
		c, err := d.Create(ctx, openOf(kind))
		if err != nil {
			t.Fatal(err)
		}
		ids = append(ids, c.ID)
	}
	restored, holder, removed := ids[0], ids[1], ids[2]
	if _, err := d.Note(ctx, holder, NoteRecord{Body: "Still stuck."}); err != nil {
		t.Fatal(err)
	}
	if err := d.Archive(ctx, restored); err != nil {
		t.Fatal(err)
	}
	p := poller(d)
	if got := states(t, p); len(got) != 2 || got[holder] != StateOpen || got[removed] != StateOpen {
		t.Fatalf("first poll: %v", got)
	}

	if err := d.Archive(ctx, removed); err != nil {
		t.Fatal(err)
	}
	putBack(t, d, restored)
	if got := states(t, p); len(got) != 2 || got[holder] != StateOpen || got[restored] != StateOpen {
		t.Errorf("after the swap: %v, want %s and %s", got, holder, restored)
	}
}

// A poll whose refresh fails part-way may already have dropped cases from
// its cache. The next poll must read every case again, even when the store is
// back as the poller last read it, so that the ids and the highest change
// alone say nothing changed. The failure is a real SQLite error: with one
// connection in the pool and its limit on bound parameters lowered to 1, only
// the fold of the two new cases by name fails.
func TestPollerReadsEveryCaseAfterAFailedRefresh(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	create := func(kind Kind) string {
		t.Helper()
		c, err := d.Create(ctx, openOf(kind))
		if err != nil {
			t.Fatal(err)
		}
		return c.ID
	}
	create(KindFYI)
	create(KindStuck)
	archived := create(KindQuestion)
	p := poller(d)
	if got := states(t, p); len(got) != 3 {
		t.Fatalf("first poll: %v", got)
	}

	if err := d.Archive(ctx, archived); err != nil {
		t.Fatal(err)
	}
	added := []string{create(KindFYI), create(KindStuck)}
	db := pool(t, d)
	db.SetMaxOpenConns(1)
	was := limitParams(t, db, 1)
	if _, _, err := p.Poll(); err == nil {
		t.Fatal("poll with one parameter allowed: no error")
	}
	limitParams(t, db, was)

	for _, id := range added {
		if err := d.Delete(ctx, id); err != nil {
			t.Fatal(err)
		}
	}
	putBack(t, d, archived)
	cases, bad, err := p.Poll()
	if err != nil || len(bad) > 0 || len(cases) != 3 {
		t.Fatalf("poll after the failed one: %d cases, bad %v, err %v", len(cases), bad, err)
	}
	for i, c := range cases {
		if c == nil {
			t.Errorf("poll after the failed one: case %d is nil", i)
		}
	}
}

// limitParams sets the most parameters a statement may bind on the pool's
// one connection to n, and returns the limit it had.
func limitParams(t *testing.T, db *sql.DB, n int) int {
	t.Helper()
	conn, err := db.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = conn.Close() }()
	was, err := sqlite.Limit(conn, sqlite3.SQLITE_LIMIT_VARIABLE_NUMBER, n)
	if err != nil {
		t.Fatal(err)
	}
	return was
}

// putBack moves the case id from the archive back into the store in one
// transaction, as the restore in docs/store.md does.
func putBack(t *testing.T, d *DB, id string) {
	t.Helper()
	tx, err := pool(t, d).BeginTx(t.Context(), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = tx.Rollback() }()
	for _, stmt := range []string{
		`INSERT INTO cases (id, opened_at) SELECT id, opened_at FROM archive_cases WHERE id = ?1`,
		`INSERT INTO events (change, case_id, seq, author, event, at, data) SELECT change, case_id, seq, author, event, at, data FROM archive_events WHERE case_id = ?1`,
		`DELETE FROM archive_events WHERE case_id = ?1`,
		`DELETE FROM archive_cases WHERE id = ?1`,
	} {
		if _, err := tx.ExecContext(t.Context(), stmt, id); err != nil {
			t.Fatal(err)
		}
	}
	if err := tx.Commit(); err != nil {
		t.Fatal(err)
	}
}

// A store thrown away and started again holds a case with the same id and the
// same change as the case the poller has: the poller must fold the new file,
// whether or not a poll saw the store missing in between.
func TestPollerStartsAgainOnANewFile(t *testing.T) {
	d := newDB(t)
	ctx := t.Context()
	opened := time.Date(2026, 9, 15, 9, 0, 0, 0, time.UTC)
	c, err := d.Create(ctx, OpenRecord{Kind: KindFYI, Urgency: UrgencyToday, Title: "Same id", OpenedAt: opened})
	if err != nil {
		t.Fatal(err)
	}
	p := poller(d)
	states(t, p)

	throwAway := func() {
		t.Helper()
		for _, suffix := range []string{"", "-wal", "-shm"} {
			if err := os.Remove(d.Path + suffix); err != nil && !errors.Is(err, fs.ErrNotExist) {
				t.Fatal(err)
			}
		}
	}
	startAgain := func(kind Kind) {
		t.Helper()
		again := openDB(t, d.Path)
		if _, err := again.Create(ctx, OpenRecord{Kind: kind, Urgency: UrgencyToday, Title: "Same id", OpenedAt: opened}); err != nil {
			t.Fatal(err)
		}
		if err := again.Disconnect(); err != nil {
			t.Fatal(err)
		}
	}
	kindOf := func() Kind {
		t.Helper()
		cases, _, err := p.Poll()
		if err != nil || len(cases) != 1 || cases[0].ID != c.ID {
			t.Fatalf("Poll = %+v, %v", cases, err)
		}
		return cases[0].Kind
	}

	throwAway()
	if _, _, err := p.Poll(); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("Poll of a removed store: %v", err)
	}
	startAgain(KindStuck)
	if got := kindOf(); got != KindStuck {
		t.Errorf("after a poll saw the store missing: kind %s, want stuck", got)
	}

	throwAway()
	startAgain(KindQuestion)
	if got := kindOf(); got != KindQuestion {
		t.Errorf("with no poll in between: kind %s, want question", got)
	}
}
