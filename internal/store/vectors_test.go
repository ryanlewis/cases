package store

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"
	"time"
)

// vectorDir holds the core's contract as JSON vectors, for every
// implementation to run: see its README.md. TestVectors runs them against
// foldRows and apply.
const vectorDir = "../../spec/vectors"

// vectorID is the id every vector's case is folded under; the vectors do not
// check ids.
const vectorID = "vector"

// vector is one entry of a vector file.
type vector struct {
	Description string         `json:"description"`
	Events      []vectorEvent  `json:"events"`
	Append      *vectorAppend  `json:"append"`
	Expect      map[string]any `json:"expect"`
	ExpectError *struct {
		Category string `json:"category"`
	} `json:"expect_error"`
}

// vectorEvent is a stored event row. Its record is data, a JSON object, or
// raw, the exact bytes.
type vectorEvent struct {
	Seq    int             `json:"seq"`
	Author Author          `json:"author"`
	Event  EventType       `json:"event"`
	Data   json.RawMessage `json:"data"`
	Raw    *string         `json:"raw"`
}

// vectorAppend is an event written to the case the rows fold to.
type vectorAppend struct {
	Author     Author          `json:"author"`
	Event      EventType       `json:"event"`
	Data       json.RawMessage `json:"data"`
	Now        time.Time       `json:"now"`
	AtRevision *int            `json:"at_revision"`
}

func TestVectors(t *testing.T) {
	files, err := filepath.Glob(filepath.Join(vectorDir, "*.json"))
	if err != nil {
		t.Fatal(err)
	}
	if len(files) == 0 {
		t.Fatalf("no vector files in %s", vectorDir)
	}
	for _, file := range files {
		name := strings.TrimSuffix(filepath.Base(file), ".json")
		for _, v := range readVectors(t, file) {
			t.Run(name+"/"+v.Description, func(t *testing.T) { runVector(t, v) })
		}
	}
}

// readVectors reads a vector file and refuses a vector a runner in another
// language could read differently.
func readVectors(t *testing.T, file string) []vector {
	t.Helper()
	data, err := os.ReadFile(file)
	if err != nil {
		t.Fatal(err)
	}
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	var vectors []vector
	if err := dec.Decode(&vectors); err != nil {
		t.Fatalf("%s: %v", file, err)
	}
	seen := map[string]bool{}
	for i, v := range vectors {
		bad := func(format string, args ...any) {
			t.Helper()
			t.Fatalf("%s: vector %d (%q): "+format, append([]any{file, i, v.Description}, args...)...)
		}
		switch {
		case v.Description == "" || seen[v.Description]:
			bad("description is empty or not unique")
		case (v.Expect == nil) == (v.ExpectError == nil):
			bad("needs one of expect and expect_error")
		}
		seen[v.Description] = true
		for j, e := range v.Events {
			if j > 0 && e.Seq <= v.Events[j-1].Seq {
				bad("seq %d does not follow %d", e.Seq, v.Events[j-1].Seq)
			}
			if (e.Raw == nil) == (e.Data == nil) || (e.Data != nil && !isObject(e.Data)) {
				bad("event %d needs one of a data object and raw", j)
			}
		}
		for _, name := range []string{"events", "problems"} {
			if l, ok := v.Expect[name]; ok && !listOfObjects(l) {
				bad("expect %s is not a list of objects", name)
			}
		}
		if a := v.Append; a != nil {
			switch {
			case newRecord(a.Event) == nil:
				bad("append of an unknown event")
			case !isObject(a.Data):
				bad("append data is not an object")
			case a.Now.IsZero():
				bad("append has no now")
			case a.Event == EventOpen && (len(v.Events) > 0 || a.AtRevision != nil):
				bad("an open is appended only to no events, and with no at_revision")
			}
			// A writer takes a typed record, so an appended one has no
			// field its event does not define.
			dec := json.NewDecoder(bytes.NewReader(a.Data))
			dec.DisallowUnknownFields()
			if err := dec.Decode(newRecord(a.Event)); err != nil {
				bad("append data: %v", err)
			}
		}
	}
	return vectors
}

func isObject(data json.RawMessage) bool {
	return bytes.HasPrefix(bytes.TrimSpace(data), []byte("{"))
}

func listOfObjects(v any) bool {
	l, ok := v.([]any)
	for _, e := range l {
		if _, isMap := e.(map[string]any); !isMap {
			return false
		}
	}
	return ok
}

func runVector(t *testing.T, v vector) {
	rows := make([]row, len(v.Events))
	for i, e := range v.Events {
		data := []byte(e.Data)
		if e.Raw != nil {
			data = []byte(*e.Raw)
		}
		rows[i] = row{seq: e.Seq, author: e.Author, event: e.Event, data: data}
	}
	a := v.Append
	c := &Case{ID: vectorID}
	if a == nil || a.Event != EventOpen {
		var err error
		if c, err = foldRows(vectorID, rows); err != nil {
			checkError(t, err, v)
			return
		}
	}
	if a == nil {
		checkCase(t, "fold", c, rows, v)
		return
	}

	before := caseView(t, c, rows)
	written, err := appendVector(t, c, a)
	if err != nil {
		checkError(t, err, v)
		sameCase(t, "the refused append changed", before, caseView(t, c, rows))
		return
	}
	rows = append(rows, written)
	checkCase(t, "append", c, rows, v)
	// The writer and the reader use the same apply, so the rows with the
	// new one fold to the case the append left.
	replayed, err := foldRows(vectorID, rows)
	if err != nil {
		t.Fatalf("replay after the append: %v", err)
	}
	sameCase(t, "the replay after the append differs in", caseView(t, c, rows), caseView(t, replayed, rows))
}

// sameCase reports each property of the case in which after differs from
// before.
func sameCase(t *testing.T, what string, before, after map[string]any) {
	t.Helper()
	for _, name := range slices.Sorted(maps.Keys(after)) {
		if !reflect.DeepEqual(after[name], before[name]) {
			t.Errorf("%s %s\n from %s\n   to %s", what, name, jsonText(before[name]), jsonText(after[name]))
		}
	}
}

// appendVector writes a.Event to c as appendEvent does inside its
// transaction, which the runner has no store for: check the revision, stamp
// the record, number it after the case's highest seq and apply it. It
// returns the row it wrote.
func appendVector(t *testing.T, c *Case, a *vectorAppend) (row, error) {
	if a.AtRevision != nil {
		if err := checkPreconditions([]Precondition{AtRevision(*a.AtRevision)}, c.Revision()); err != nil {
			return row{}, err
		}
	}
	rec := newRecord(a.Event)
	if err := json.Unmarshal(a.Data, rec); err != nil {
		t.Fatal(err)
	}
	rec.stamp(a.Now)
	data, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	data = append(data, '\n')
	seq := c.lastSeq + 1
	if err := c.apply(Event{Seq: seq, Author: a.Author, Type: a.Event, File: fileName(seq, a.Author, a.Event), Data: data}); err != nil {
		return row{}, err
	}
	c.lastSeq = seq
	c.events++
	return row{seq: seq, author: a.Author, event: a.Event, data: data}, nil
}

func checkError(t *testing.T, err error, v vector) {
	t.Helper()
	if v.ExpectError == nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if got := errorCategory(err); got != v.ExpectError.Category {
		t.Errorf("error category %q, want %q: %v", got, v.ExpectError.Category, err)
	}
}

// errorCategory sorts an error from the fold or the writer into the vectors'
// categories.
func errorCategory(err error) string {
	var te *TransitionError
	switch {
	case errors.As(err, &te):
		return "transition"
	case errors.Is(err, ErrStale):
		return "stale"
	}
	return messageCategory(err.Error())
}

// messageCategory sorts an error by its message. Apart from TransitionError
// and ErrStale the core's errors are plain text, and a skipped event reaches
// Problems only as text, so they are told apart by how the message starts.
// An error the core gains later reads as invalid until this learns it.
func messageCategory(msg string) string {
	switch {
	case strings.HasPrefix(msg, "no valid open event"), msg == "no open event":
		return "no-open"
	case strings.HasPrefix(msg, "unknown "):
		return "unknown"
	case strings.HasPrefix(msg, "malformed "):
		return "malformed"
	case strings.HasPrefix(msg, "cannot "), msg == "case is already open":
		return "transition"
	case strings.Contains(msg, " events are not written by the "):
		return "author"
	}
	return "invalid"
}

func checkCase(t *testing.T, stage string, c *Case, rows []row, v vector) {
	t.Helper()
	if v.Expect == nil {
		t.Fatalf("%s: accepted, want an error of category %q", stage, v.ExpectError.Category)
	}
	got := caseView(t, c, rows)
	for _, name := range slices.Sorted(maps.Keys(v.Expect)) {
		want := v.Expect[name]
		have, ok := got[name]
		switch {
		case !ok:
			t.Errorf("%s: expect names %q, which is not a property of the case", stage, name)
		case name == "events" || name == "problems":
			for _, d := range listDiff(have, want) {
				t.Errorf("%s: %s%s", stage, name, d)
			}
		case !reflect.DeepEqual(have, want):
			t.Errorf("%s: %s\n want %s\n  got %s", stage, name, jsonText(want), jsonText(have))
		}
	}
}

// listDiff compares have with want element by element, checking each element
// only on the properties want gives it; a property given as null must be
// absent. It describes each difference.
func listDiff(have, want any) []string {
	h, _ := have.([]any)
	w, ok := want.([]any)
	if !ok {
		return []string{": want is not a list"}
	}
	if len(h) != len(w) {
		// Name the elements by the properties want uses, or by their file.
		keys := map[string]bool{}
		for _, we := range w {
			for k := range we.(map[string]any) {
				keys[k] = true
			}
		}
		if len(keys) == 0 {
			keys = map[string]bool{"file": true, "category": true}
		}
		got := make([]string, len(h))
		for i, he := range h {
			p := map[string]any{}
			for k := range keys {
				if x, ok := he.(map[string]any)[k]; ok {
					p[k] = x
				}
			}
			got[i] = jsonText(p)
		}
		return []string{fmt.Sprintf(": %d elements, want %d\n want %s\n  got [%s]", len(h), len(w), jsonText(want), strings.Join(got, ", "))}
	}
	var diffs []string
	for i := range w {
		we, _ := w[i].(map[string]any)
		he, _ := h[i].(map[string]any)
		got := map[string]any{}
		same := true
		for k, wv := range we {
			got[k] = he[k]
			same = same && reflect.DeepEqual(he[k], wv)
		}
		if !same {
			diffs = append(diffs, fmt.Sprintf("[%d]\n want %s\n  got %s", i, jsonText(we), jsonText(got)))
		}
	}
	return diffs
}

// caseView is the case as the vectors describe it: every property, records
// as recordView writes them, times in UTC and event data as stored.
func caseView(t *testing.T, c *Case, rows []row) map[string]any {
	t.Helper()
	view := map[string]any{
		"state":      c.State,
		"revision":   c.Revision(),
		"amend_seq":  c.AmendSeq(),
		"updated_at": timeView(c.UpdatedAt),
		"kind":       c.Kind,
		"urgency":    c.Urgency,
		"title":      c.Title,
		"body":       c.Body,
		"options":    listView(c.Options),
		"rows":       recordView(t, listView(c.Rows)),
		"links":      listView(c.Links),
		"labels":     listView(c.Labels),
		"worker":     c.Worker,
		"brief":      c.Brief,
		"context":    c.Context,
		"for":        c.For,
		"actor":      recordView(t, c.Actor),
		"opened_at":  timeView(c.OpenedAt),
		"answer":     nil,
		"pickup":     nil,
		"park":       nil,
		"close":      nil,
	}
	if r := c.Answer; r != nil {
		utc := *r
		utc.AnsweredAt = r.AnsweredAt.UTC()
		view["answer"] = recordView(t, utc)
	}
	if r := c.Pickup; r != nil {
		utc := *r
		utc.PickedUpAt = r.PickedUpAt.UTC()
		view["pickup"] = recordView(t, utc)
	}
	if r := c.Park; r != nil {
		utc := *r
		utc.ParkedAt = r.ParkedAt.UTC()
		view["park"] = recordView(t, utc)
	}
	if r := c.Close; r != nil {
		utc := *r
		utc.ClosedAt = r.ClosedAt.UTC()
		view["close"] = recordView(t, utc)
	}
	events := []any{}
	for _, ev := range c.Events {
		e := map[string]any{"seq": ev.Seq, "author": ev.Author, "event": ev.Type, "file": ev.File, "data": ev.Data}
		if ev.From() != "" {
			e["from"] = ev.From()
		}
		if at := timeView(ev.At); at != nil {
			e["at"] = at
		}
		if ev.Actor != nil {
			e["actor"] = recordView(t, ev.Actor)
		}
		events = append(events, e)
	}
	view["events"] = events
	problems := []any{}
	for _, p := range c.Problems {
		i := slices.IndexFunc(rows, func(r row) bool { return strings.HasPrefix(p, fileName(r.seq, r.author, r.event)+": ") })
		if i < 0 {
			t.Fatalf("problem %q names no row", p)
		}
		r := rows[i]
		name := fileName(r.seq, r.author, r.event)
		problems = append(problems, map[string]any{"seq": r.seq, "file": name, "category": messageCategory(strings.TrimPrefix(p, name+": "))})
	}
	view["problems"] = problems
	var out map[string]any
	roundTrip(t, view, &out)
	return out
}

// listView keeps an empty list a list.
func listView[T any](l []T) []T {
	if l == nil {
		return []T{}
	}
	return l
}

func timeView(at time.Time) any {
	if at.IsZero() {
		return nil
	}
	return at.UTC().Format(time.RFC3339Nano)
}

// zeroTime is how encoding/json writes a time that is not set.
const zeroTime = "0001-01-01T00:00:00Z"

// recordView is the record v as a JSON value, with every object field at its
// zero value left out: an empty string or list, 0, false, null or a time
// not set.
func recordView(t *testing.T, v any) any {
	t.Helper()
	var out any
	roundTrip(t, v, &out)
	return dropZero(out)
}

// roundTrip writes v as JSON and reads it into out, so views compare with
// the values read from a vector file.
func roundTrip(t *testing.T, v, out any) {
	t.Helper()
	data, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, out); err != nil {
		t.Fatal(err)
	}
}

func dropZero(v any) any {
	switch v := v.(type) {
	case map[string]any:
		for k, x := range v {
			x = dropZero(x)
			switch x {
			case nil, "", zeroTime, 0.0, false:
				delete(v, k)
				continue
			}
			if l, ok := x.([]any); ok && len(l) == 0 {
				delete(v, k)
				continue
			}
			v[k] = x
		}
	case []any:
		for i := range v {
			v[i] = dropZero(v[i])
		}
	}
	return v
}

func jsonText(v any) string {
	data, err := json.Marshal(v)
	if err != nil {
		return err.Error()
	}
	return string(data)
}
