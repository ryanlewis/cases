package store

import (
	"context"
	"database/sql"
	"slices"
	"strings"
)

// LoadError is a case that could not be folded.
type LoadError struct {
	ID  string
	Err error
}

func (e *LoadError) Error() string { return e.ID + ": " + e.Err.Error() }

func (e *LoadError) Unwrap() error { return e.Err }

// List folds every case in the store, sorted by id (which is by open time). A
// case that fails to fold is returned in bad and does not stop the others.
// err is set only when the store itself cannot be read.
func (d *DB) List(ctx context.Context) (cases []*Case, bad []*LoadError, err error) {
	pool, _, release, err := d.open(ctx, false)
	if err != nil {
		return nil, nil, err
	}
	defer release()
	folded, err := loadCases(ctx, pool, nil)
	if err != nil {
		return nil, nil, err
	}
	for _, f := range folded {
		if f.err != nil {
			bad = append(bad, &LoadError{ID: f.id, Err: f.err})
			continue
		}
		cases = append(cases, f.c)
	}
	return cases, bad, nil
}

// IDs lists the case ids in the store, sorted, without folding the cases.
func (d *DB) IDs(ctx context.Context) ([]string, error) {
	pool, _, release, err := d.open(ctx, false)
	if err != nil {
		return nil, err
	}
	defer release()
	return queryStrings(ctx, pool, `SELECT id FROM cases ORDER BY id`)
}

// Get folds the case with exactly this id.
func (d *DB) Get(ctx context.Context, id string) (*Case, error) {
	if err := ValidID(id); err != nil {
		return nil, err
	}
	pool, _, release, err := d.open(ctx, false)
	if err != nil {
		return nil, err
	}
	defer release()
	return loadCase(ctx, pool, id)
}

// SortInbox orders cases the way the inbox shows them: most urgent first, then
// oldest first. The id starts with the open time, so it sorts by age.
func SortInbox(cases []*Case) {
	slices.SortStableFunc(cases, func(a, b *Case) int {
		if r := a.Urgency.Rank() - b.Urgency.Rank(); r != 0 {
			return r
		}
		return strings.Compare(a.ID, b.ID)
	})
}

// loadCase folds the case id, reading its events in one statement.
func loadCase(ctx context.Context, q querier, id string) (*Case, error) {
	folded, err := loadCases(ctx, q, []string{id})
	if err != nil {
		return nil, err
	}
	if len(folded) == 0 {
		return nil, noCase(id)
	}
	return folded[0].c, folded[0].err
}

// foldedCase is one case as loadCases folded it, or the error that stopped
// the fold.
type foldedCase struct {
	id   string
	rows []row
	c    *Case
	err  error
}

// loadCases folds the cases with the given ids, or every case when ids is
// nil, in id order. Each case's events are read with it in one statement, so
// a case is never folded from part of a write. A case with no events is
// returned with the fold's error.
func loadCases(ctx context.Context, q querier, ids []string) ([]foldedCase, error) {
	if ids != nil && len(ids) == 0 {
		return nil, nil
	}
	query := `SELECT c.id, e.seq, e.author, e.event, e.data
		FROM cases c LEFT JOIN events e ON e.case_id = c.id`
	var args []any
	if ids != nil {
		query += ` WHERE c.id IN (` + strings.TrimSuffix(strings.Repeat("?,", len(ids)), ",") + `)`
		for _, id := range ids {
			args = append(args, id)
		}
	}
	query += ` ORDER BY c.id, e.seq`
	res, err := q.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer func() { _ = res.Close() }()

	var folded []foldedCase
	for res.Next() {
		var id string
		var seq sql.NullInt64
		var author, event sql.NullString
		var data []byte
		if err := res.Scan(&id, &seq, &author, &event, &data); err != nil {
			return nil, err
		}
		if n := len(folded); n == 0 || folded[n-1].id != id {
			folded = append(folded, foldedCase{id: id})
		}
		// A case with no events comes back as one row of nulls.
		if seq.Valid {
			last := &folded[len(folded)-1]
			last.rows = append(last.rows, row{seq: int(seq.Int64), author: Author(author.String), event: EventType(event.String), data: data})
		}
	}
	if err := res.Err(); err != nil {
		return nil, err
	}
	for i := range folded {
		folded[i].c, folded[i].err = foldRows(folded[i].id, folded[i].rows)
		folded[i].rows = nil
	}
	return folded, nil
}

// Poller lists a store repeatedly, folding again only the cases that have
// changed. A poll first reads the store's highest change and its number of
// cases. When both are as they were, it returns the cases it has without
// reading any events: every event written raises the highest change, since
// change is never used twice, and taking a case out, as prune does, lowers
// the number of cases. Otherwise it folds the cases with an event above the
// highest change it had, and any case it has not seen, and drops the cases no
// longer in the store.
type Poller struct {
	db *DB
	// gen is the DB's pool generation the cache was read from. A new file
	// at the path starts the cache again.
	gen   int
	head  pollHead
	ids   []string // nil until the first read
	cache map[string]foldedCase
}

// maxStale is the most cases a poll folds again by name. Past it the poll
// folds every case, which also keeps the statement under SQLite's limit on
// parameters.
const maxStale = 100

// pollHead is what a poll checks before reading any case.
type pollHead struct {
	change int64
	cases  int
}

// NewPoller returns a Poller over the store.
func (d *DB) NewPoller() CasePoller {
	return &Poller{db: d, cache: map[string]foldedCase{}}
}

// Poll returns the same as List, from cache where nothing has changed. The
// poll reads the store in one read transaction, so what it caches is one
// moment of the store.
func (p *Poller) Poll() (cases []*Case, bad []*LoadError, err error) {
	ctx := context.Background()
	pool, gen, release, err := p.db.open(ctx, false)
	if err != nil {
		p.reset(0)
		return nil, nil, err
	}
	defer release()
	if gen != p.gen {
		p.reset(gen)
	}
	tx, err := pool.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return nil, nil, err
	}
	defer func() { _ = tx.Rollback() }()
	if err := checkVersion(ctx, tx, p.db.Path); err != nil {
		p.reset(0)
		return nil, nil, err
	}

	var head pollHead
	err = tx.QueryRowContext(ctx, `SELECT (SELECT coalesce(max(change), 0) FROM events), (SELECT count(*) FROM cases)`).Scan(&head.change, &head.cases)
	if err != nil {
		return nil, nil, err
	}
	if p.ids == nil || head != p.head {
		if err := p.refresh(ctx, tx); err != nil {
			return nil, nil, err
		}
		p.head = head
	}
	for _, id := range p.ids {
		entry := p.cache[id]
		if entry.err != nil {
			bad = append(bad, &LoadError{ID: id, Err: entry.err})
			continue
		}
		cases = append(cases, entry.c)
	}
	return cases, bad, nil
}

// reset forgets every case, for a store that is gone or is another file.
func (p *Poller) reset(gen int) {
	p.gen, p.head, p.ids = gen, pollHead{}, nil
	clear(p.cache)
}

// refresh brings the cache up to the store: it drops the cases no longer
// there, and folds again the cases with an event stored since the last read
// and the cases it has not seen, such as one put back from the archive, whose
// events keep the change they had.
func (p *Poller) refresh(ctx context.Context, tx *sql.Tx) error {
	ids, err := queryStrings(ctx, tx, `SELECT id FROM cases ORDER BY id`)
	if err != nil {
		return err
	}
	stale := map[string]bool{}
	if p.ids == nil {
		clear(p.cache)
	} else {
		changed, err := queryStrings(ctx, tx, `SELECT DISTINCT case_id FROM events WHERE change > ?`, p.head.change)
		if err != nil {
			return err
		}
		for _, id := range changed {
			stale[id] = true
		}
	}
	present := map[string]bool{}
	for _, id := range ids {
		present[id] = true
		if _, ok := p.cache[id]; !ok {
			stale[id] = true
		}
	}
	for id := range p.cache {
		if !present[id] {
			delete(p.cache, id)
		}
	}
	if len(stale) > 0 {
		var names []string // nil folds every case
		if len(stale) < len(ids) && len(stale) <= maxStale {
			for id := range stale {
				names = append(names, id)
			}
		}
		folded, err := loadCases(ctx, tx, names)
		if err != nil {
			return err
		}
		for _, f := range folded {
			p.cache[f.id] = f
		}
	}
	p.ids = ids
	return nil
}

// queryStrings runs a query whose rows are one string each.
func queryStrings(ctx context.Context, q querier, query string, args ...any) ([]string, error) {
	rows, err := q.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer func() { _ = rows.Close() }()
	out := []string{}
	for rows.Next() {
		var s string
		if err := rows.Scan(&s); err != nil {
			return nil, err
		}
		out = append(out, s)
	}
	return out, rows.Err()
}
