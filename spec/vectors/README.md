# Core vectors

These files are the contract for the core of cases: how a case's stored events fold into its state, which events each state allows, what each record must hold, and how the writer numbers, stamps and checks a new event. They are JSON, so another implementation of the core can run them with a small runner of its own and be checked against the Go one. The Go runner is `internal/store/vectors_test.go`, and `make test` runs it.

A vector says what the Go core does today. Text for people, such as error messages, is not part of the contract: errors are compared by category. `go-quirks.json` holds Go behaviour that another implementation may decide to change; see [Current Go behaviour](#current-go-behaviour-a-rewrite-may-decide).

## Files

| File | Vectors | What they cover |
| --- | --- | --- |
| `transitions.json` | 105 | Every event from every state of a stuck case, parking for each kind, who writes each event, a note reopening a case, each kind from open to closed, what a note and a close must say, events before the open |
| `open.json` | 63 | The open record: kind, urgency, title, options and rows for each kind, row ids, labels, the free text fields, `for` and `actor` |
| `answer.json` | 61 | The answer for each kind, `drop`, and answers checked against the case as amended |
| `amend.json` | 59 | What an amend adds and replaces, what it may not repeat, and `amend_seq` |
| `fold.json` | 44 | Replay: skipped rows and their problems, cases with no valid open event, times, fields from newer builds, actors, records from earlier builds, the order of the checks, file names |
| `writer.json` | 50 | Append: numbering, stamping and `at_revision` |
| `go-quirks.json` | 39 | Current Go behaviour that another implementation may decide on |

`COVERAGE.md` lists the Go tests whose subject the vectors leave out.

## A vector

Each file is a JSON list of vectors, such as this one from `transitions.json`:

```json
{
  "description": "answered: pickup is accepted and the case is pickedup",
  "events": [
    {"seq": 1, "author": "agent", "event": "open", "data": {"kind": "stuck", "urgency": "today", "title": "Mirror unreachable", "body": "Body.", "opened_at": "2026-09-15T09:01:00Z"}},
    {"seq": 2, "author": "human", "event": "answer", "data": {"text": "Try the other mirror.", "answered_at": "2026-09-15T09:02:00Z"}}
  ],
  "append": {"author": "agent", "event": "pickup", "data": {"by": "bun-pins"}, "now": "2026-09-15T09:03:00Z"},
  "expect": {"state": "pickedup", "revision": 3}
}
```

- `description` names the vector. It is unique within its file.
- `events` are the case's stored rows, in order. Each has the `seq`, `author` and `event` columns and its record. `seq` is at least 1 and goes up from one row to the next, and may skip numbers. The record is either `data`, a JSON object, or `raw`, a string that holds the record's exact bytes. `raw` is used where the bytes matter: a record that is not valid JSON, a field given twice, a number such as `2.0`, and records copied from a store.
- `append`, when present, is one new event to write to the case: its `author`, `event`, record `data`, the writer's clock `now`, and optionally `at_revision`, the revision the writer read the case at. An `open` is appended only to a case with no `events`, and without `at_revision`. An appended record has only the fields its event defines, and none of them is `null`.
- `expect` describes the case afterwards. `expect_error` gives the `category` of the error instead.

A file holds one list and nothing after it, and no object in it gives a key twice. The keys above, and those of an appended record, are spelt exactly. Go's decoder reads a key in another case as the same key and merges a key given twice, where other parsers do not, so the Go runner refuses both.

## Running a vector

A vector without `append` is a **replay**:

1. Fold the rows in order.
2. A row the case cannot take is skipped. It is listed in `problems` with its category and still counts in `revision`, and the fold goes on with the next row.
3. If no row is a valid open event, the fold fails with `no-open`.

A vector with `append` does what the writer does:

1. For an `open`, start from an empty case. Otherwise fold the rows as in a replay. If that fails, the append fails with `no-open`.
2. If `at_revision` is given and is not the case's `revision`, refuse the append as `stale`.
3. If the record has no time, give it `now`. A time the record has is kept, as the same instant: the stored record gives it in UTC, so `11:45:00+02:00` is stored as `09:45:00Z`.
4. Number the new row one more than the highest `seq` among the rows, skipped rows included. An open is numbered 1.
5. Check the event against the case with the rules a replay uses. A refused append has the category a replay would give the row, except for an open that breaks more than one rule (see [Current Go behaviour](#current-go-behaviour-a-rewrite-may-decide)), and leaves every property of the case as it was.
6. An accepted append changes the case. Check `expect` against it. Then fold the rows with the new row added, and check that this gives the same case.

The core never reads a clock during a replay. Each record carries its own time (`opened_at`, `answered_at` and so on), and the vectors fix it. The fold does not read the events table's `at` column, so the vectors do not give one.

## The case

`expect` lists properties of the case. A runner checks the ones given and ignores the rest.

| Property | Value |
| --- | --- |
| `state` | `open`, `answered`, `pickedup`, `closed`, `withdrawn` or `parked` |
| `revision` | The number of stored rows, skipped ones included |
| `amend_seq` | The `seq` of the last amend that sets a body, context, option, row or link, or 0. A body or context counts even when it is the text the case already has. An amend that sets only labels does not count |
| `updated_at` | The latest time of the events that folded, or `null` |
| `kind`, `urgency`, `title`, `body`, `worker`, `brief`, `context`, `for` | From the open event, with `body` and `context` as the last amend that set them left them. `""` when not set |
| `options`, `rows`, `links`, `labels` | The open event's, then each amend's, in order. `[]` when there are none |
| `actor`, `opened_at` | The open event's, or `null` |
| `answer` | The answer the case stands on, or `null`. A note that reopens the case clears it |
| `pickup` | The pickup record, or `null`. A note that reopens the case clears it |
| `park` | The park record while the case is parked, or `null` |
| `close` | The close record, or `null` |
| `events` | The events that folded, in order |
| `problems` | The rows that were skipped, in order |

Each element of `events` has:

- `seq`, `author` and `event`, as stored
- `file`, the name the event had as a file: `seq` as at least four digits, the author and the event, such as `0004-agent-pickup.json`
- `from`, the state before the event, left out for the open event
- `at`, the event's time, left out when it has none
- `actor`, left out when it has none
- `data`, the record as stored, as a JSON value

Each element of `problems` has the row's `seq` and `file`, and the `category` of the reason it was skipped.

Records (`answer`, `pickup`, `park`, `close`, `actor` and each element of `rows`) are JSON objects with the field names a stored record uses, such as `answered_at`. Each is the record as its event reads it: a field the event does not define, such as an actor's `id` from a later build, is left out, though `events[].data` keeps it. A field at its zero value is left out: an empty string or list, 0, false, null, or no time. So `{"choice": 1, "answered_at": "2026-09-15T09:02:00Z"}` is an answer with no note, and a pickup record with nothing in it is `{}`.

Times are RFC 3339 in UTC, ending in `Z`, with only as many fractional digits as needed, such as `2026-09-16T22:15:41.605108Z`.

Each property is compared as a whole JSON value, except `events` and `problems`. For those, the list must have the same length, and each element is checked only on the properties the vector gives it, where `null` means the property is absent, or for `data`, a record of `null`. An element names only the properties listed above.

## Error categories

| Category | Meaning |
| --- | --- |
| `unknown` | An event type, kind or urgency this version does not know, perhaps written by a newer cases |
| `author` | The author may not write this event. The agent writes `open`, `amend`, `pickup`, `note`, `close` and `withdraw`, the human writes `answer` and `park`, and either writes `resume`. Any other author is refused |
| `malformed` | The record does not read as the event's record: it is not a JSON object, a field has the wrong type, or a time does not read (see [Definitions](#definitions)) |
| `invalid` | The record reads but breaks a rule of its event, such as a blank title, a choice that is not an option, or an amend that changes nothing |
| `transition` | The case's state does not allow the event, or its kind does not, for `park`. An event before the open, and a second open, are transitions too |
| `stale` | An append whose `at_revision` is not the case's revision |
| `no-open` | The case does not fold: no row is a valid open event |

When a row is wrong in more than one way, its category comes from the first check it fails:

1. The event type is known (`unknown`).
2. The author may write the event (`author`).
3. The record reads (`malformed`).
4. An `actor`, when given, has a name and a kind (`invalid`).
5. A case with no open event takes only an open (`transition`).
6. The state allows the event (`transition`), then the record keeps its event's rules (`invalid`). An open's kind and urgency are checked (`unknown`) before its other rules.

An append checks `at_revision` before all of these, once the rows fold. An appended open is the exception to the order: see [Current Go behaviour](#current-go-behaviour-a-rewrite-may-decide).

## Definitions

- **Blank** means empty once white space is taken off both ends. White space is what Go's `unicode.IsSpace` takes, the Unicode White_Space characters: a no-break space (U+00A0) is white space, a zero-width no-break space (U+FEFF) is not.
- Kinds, urgencies, verdicts and signoff values are matched exactly, so `Decision` is not a kind.
- A row id must match `^[A-Za-z0-9._-]+$` as a whole, so an id that ends in a newline does not.
- A time is read as Go's `time.Parse` reads RFC 3339, from the string's bytes as stored: an upper-case `T`, and `Z` or a numeric offset. That differs from RFC 3339 both ways; see [Current Go behaviour](#current-go-behaviour-a-rewrite-may-decide).

## Current Go behaviour, a rewrite may decide

`go-quirks.json` records what Go does today where another implementation could reasonably do something else. Changing one changes how existing stores fold or what a writer accepts, so decide each on purpose.

- **Field names match whatever their case.** Go reads `"Kind"` as `kind` and `"ACK"` as `ack`, by Unicode simple case folding, so a Kelvin sign (U+212A) reads as `k` too.
- **A field given twice is read twice, into the same value.** This holds when the two differ only in case, too. A later string, number, boolean or time replaces the earlier one, and a later `null` leaves it as it was. A later object is merged into the earlier one, field by field. A later list replaces the earlier one, except that an object in it is merged into the object at the same place in the earlier list. A later `null` clears a list or an object.
- **Null.** A record of `null` reads as an empty record, so a pickup of `null` folds. A field of `null` reads as a field not given, so an open with `"title": null` is `invalid`, not `malformed`.
- **Zero values.** A response field at its zero value reads as not set: `{"choice": 1, "other": false}` is a choice, and `{"ack": false}` alone is no response.
- **Zero times.** A record without its time folds, with no time. Go holds it as the zero time, which `cases show --json` prints as `0001-01-01T00:00:00Z`. A record that gives that time reads the same as one without, and an append gives it `now`. The vectors show no time as `null`, or leave it out.
- **RFC 3339 as Go reads it.** A lower-case `t` or `z`, a leap second (`23:59:60`) and a time written with a JSON escape, all of which RFC 3339 allows, are `malformed`. An hour of one digit, a comma before the fraction, and an offset of `+24:00` or with a minute of 60, none of which RFC 3339 allows, are read.
- **Missing kind or urgency.** An open without a kind or an urgency is refused as `unknown`, as if a newer cases had written it.
- **An appended open is checked for its own rules first.** `Create` checks an open's kind, urgency and other rules before it takes the store's lock, then appends it with the checks a replay makes. So an open with an unknown kind and a nameless actor is refused as `unknown` when written, while a stored row of it is skipped as `invalid`, for the actor.
- **Open is looser than amend.** An open may give an option twice, a link twice or a blank link, and an amend may not. The fold checks every open event again, so refusing these in the fold would stop stored cases from folding. An implementation that wants to refuse them should do it only when writing.
- **An answer is checked against amends only through `at_revision`.** The fold refuses an answer numbered at or below `amend_seq`, but the writer numbers each event after the last row, so that never happens: an answer written without seeing an amend is accepted unless it sends `at_revision`. With `at_revision`, an answer is stale after any amend, even one that only added labels, which `amend_seq` leaves out.

Two more details have no vector. The fold's check of an answer's number against `amend_seq` can only be reached with seqs out of order or used twice, which the store's unique `(case_id, seq)` prevents; `TestAnswerNeedNotSeeALabelAmend` checks it in Go. And Go returns a second open as a plain error rather than a `TransitionError`, which no command can reach, since `cases open` always makes a new case; the vectors count it as `transition`.

## Not covered

The vectors cover the fold and the writer's rules for one case. They leave out text for people (error messages, the thread lines from `Describe`, CLI output), case ids and slugs, the store itself (SQLite, the archive, the poller, writers running at the same time) and other decoding details, such as invalid UTF-8 or a number too large for an integer, which no test pins yet. `COVERAGE.md` lists the Go tests for the rest.

## The Go runner

`TestVectors` in `internal/store/vectors_test.go` reads every `.json` file in this directory and runs each vector as a subtest named after its file and description:

```sh
go test ./internal/store -run 'TestVectors/answer'
```

It replays rows with `foldRows`. It appends in the steps listed under [Running a vector](#running-a-vector), calling `apply` for the check, and then writes the same append with the store's own writer (`Create`, or the method behind `Answer`, `Park` and the others) to a case of its own in a scratch database. The two must agree: the same error category and nothing stored, or the same case, stored so that it folds the same way again. So the runner's steps stay the writer's steps. The writer has no way to write an open by anyone but the agent, so that append is checked with `apply` alone. `TestTransitionVectorsCoverEveryState` checks that `transitions.json` tries every event, by each author that may write it, from every state of a stuck case. Go's errors are plain text apart from `TransitionError` and `ErrStale`, so the runner sorts the others into categories by how their message starts. An error the core gains later reads as `invalid` until the runner learns it.

## Changing a vector

A vector records what the Go core does. When the core changes on purpose, change its vectors in the same commit and say why in the commit message. Keep descriptions unique within a file, and use `data` unless the bytes matter.
