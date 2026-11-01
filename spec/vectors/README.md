# Core vectors

These files are the contract for the core of cases: how a case's stored events fold into its state, which events each state allows, what each record must hold, and how the writer numbers, stamps and checks a new event. They are JSON, so any implementation of the core can run them with a small runner of its own. `cases-core` must pass every vector.

Text for people, such as error messages, is not part of the contract: errors are compared by category, never by message.

The set began as a copy of the Go core's vectors in `ryanlewis/cases` at 508548b. It now states the rules of v1 of the Rust rewrite, which differ from Go's in places. [CHANGES.md](CHANGES.md) lists every vector changed, added or removed since 508548b, with the user story or decision behind it. Story ids such as AG-02 and ST-04 below refer to the user stories the rewrite is built against.

## Files

| File | Vectors | What they cover |
| --- | --- | --- |
| `transitions.json` | 197 | Every event from every state of a stuck case and of a message, by each author that may write it, parking for each kind, who writes each event, who opens and amends each kind, a note or an amend reopening a case and the `reopen` it says so with, each kind from open to closed and a message from sent to closed, a briefing sent back and judged again, what a note and a close must say, an expired close of a briefing, events before the open |
| `open.json` | 141 | The open record: kind, urgency, title, options and rows for each kind, a briefing's body, `landed` and `left_out`, who carries out a row, a decision's recommend, a message's worker and labels, row ids, labels, the free text fields, `for`, `re` and `actor`, the rules only a write applies, and the order of the open's checks |
| `answer.json` | 100 | The answer for each kind, a briefing's verdict and the left-out items it files, `drop`, fields at their zero value, and answers checked against the case as amended, with the amend seq their writer saw |
| `amend.json` | 112 | What an amend adds, drops and replaces, a briefing's `landed` and `left_out`, what it may not repeat, which amends move `amend_seq`, an amend that reopens a case, and the human's amend of a message |
| `fold.json` | 80 | Replay: skipped rows and their problems, reading a record, times, cases with no valid open event, fields from newer versions, actors, the order of the checks, file names, notes stored before `reopen` |
| `writer.json` | 51 | Append: numbering, stamping and `at_revision` |
| `freshness.json` | 38 | How a case changed since a form was drawn from it: notes, amends that change the question and amends that do not, the human's edit of a message, answers, pickups, withdraws, a park, reopens, a state passed twice, skipped rows, and a revision or seq that does not match the case's |
| `stats.json` | 36 | Cases opened and answered on each UTC day and the median time to answer: day edges, days outside the window, an answer on a later day, a reopen and the answer after it, the median of an odd and an even count, drops, withdraws, messages, each kind answered, and times missing or out of order |

`crates/core/tests/vectors_shape.rs` checks every file against the format below, and this table against the files.

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
- `events` are the case's stored rows, in order, and `[]` when there are none. Each has the `seq`, `author` and `event` columns and its record. `seq` is at least 1 and goes up from one row to the next, and may skip numbers. The record is either `data`, a JSON object, or `raw`, a string that holds the record's exact bytes. `raw` is used where the bytes matter: a record that is not a JSON object, a key given twice, a number such as `2.0` or `-0`, a time the JSON spells with an escape.
- `append`, when present, is one new event to write to the case: its `author`, `event`, record `data`, the writer's clock `now`, and optionally `at_revision`, the revision the writer read the case at. The event is one of the nine below. An `open` is appended only to a case with no `events`, and without `at_revision`. An appended record is one a writer could be handed: it names only the fields its event defines, each of its type, and none of them is `null`. Its times are spelt with an upper-case `T`, and `Z` or an offset: the lower-case spellings are tried only in stored rows.
- `expect` describes the case afterwards. `expect_error` gives the `category` of the error instead. A vector has one of the two. A replay fails only with `no-open`.

A file holds one list and nothing after it, and no object in it gives a key twice. The keys above, and those of an appended record, are spelt exactly. No key of a vector, a row or an append is `null`.

## Records

Each event's record is a JSON object with these fields:

| Event | Author | Fields |
| --- | --- | --- |
| `open` | agent, or human for a message | `kind`, `urgency`, `title`, `body`, `options`, `recommend`, `rows`, `landed`, `left_out`, `links`, `labels`, `worker`, `brief`, `context`, `for`, `re`, `actor`, `opened_at` |
| `amend` | agent, or human for a message | `title`, `body`, `options`, `recommend`, `rows`, `landed`, `left_out`, `links`, `drop_links`, `labels`, `context`, `reopen`, `actor`, `amended_at` |
| `answer` | human | `choice`, `other`, `rows`, `signoff`, `briefing`, `file`, `text`, `drop`, `ack`, `note`, `amend_seq`, `actor`, `answered_at` |
| `pickup` | agent | `by`, `actor`, `picked_up_at` |
| `note` | agent | `body`, `reopen`, `actor`, `noted_at` |
| `close` | agent | `outcome`, `links`, `expired`, `actor`, `closed_at` |
| `withdraw` | agent or human | `reason`, `actor`, `withdrawn_at` |
| `park` | human | `note`, `actor`, `parked_at` |
| `resume` | human | `actor`, `resumed_at` |

A row of an open or an amend has `id`, `label`, `script`, `link`, `note` and `by`. A row of an answer has `id`, `verdict` and `note`. An actor has an `id`.

`choice` and `recommend` are whole numbers, and `file` is a list of them. `amend_seq` is a count, a whole number of 0 or more: a negative one is of the wrong type, so `malformed`. `other`, `drop`, `ack`, `reopen` and `landed` are `true` or `false`. `options`, `left_out`, `links`, `drop_links` and `labels` are lists of strings, `rows` is a list of rows, and `actor` is an actor. A field ending in `_at` is a time. Every other field is a string.

An actor says which human or agent wrote an event, as an `id` and nothing more. The `id` is 22 characters of the base64url alphabet (`A`–`Z`, `a`–`z`, `0`–`9`, `-`, `_`): 128 random bits, never derived from an email address or an account. The event's `author` gives the role, human or agent, and the core checks the author, never the actor: the actor is a label, not a permission. Names and kinds live in the store's table of actors, in a local store and a hosted one alike, and a hosted store can delete a person's row, after which the person's events keep a bare `id`. The core checks only that an `id` is 22 characters of that alphabet: it never decodes one, and never looks one up.

A decision's `recommend` is the option the agent recommends, numbered from 1 as a `choice` is; left out or 0, the case recommends none. The open's is `invalid` on any other kind, and when it names no option: the number after the last is other, which is not an option. A replay applies these rules as a write does. An amend's `recommend` replaces the case's, and may name an option the same amend adds; it cannot take the case's away. It is `invalid` on the same terms as the open's, checked against the options as amended. The recommend is shown to the human, and an answer is never checked against it. It is advice about the question, not the question, so a new one does not move `amend_seq` (ST-04). That also keeps builds from before the field in step with this one: such a build skips an amend that sets only a recommend as changing nothing, and works out the same `amend_seq`.

A `message` is the one kind the human opens: a message from the human to an agent. Its open is the human's, and so is an amend of it; an open or an amend of any other kind is the agent's. Either side's open or amend of the other side's kind is refused as `author`. A message starts `sent`, not `open`: it waits on an agent, not on the human. The agent takes it with a `pickup`, which makes it `pickedup`, and closes it with the outcome. The human may amend it while it is sent, and withdraw it while it is sent or picked up. It takes no answer, note, park or resume. It goes to a worker or a label: an open of a message whose `worker` is blank and which has no labels is `invalid`, on a replay as on a write.

A `briefing` shows the human a piece of finished work, as a document: its title is the headline, and its `body`, in markdown, is required, so an open whose body is missing or blank is `invalid`. `landed` says the work has landed already, and nothing waits on the answer; left out, it is `false`. `left_out` lists what the work left out on purpose: at most 10 items, each at most 300 characters (Unicode scalar values, not bytes), none blank and none given twice. Both are a briefing's only, and options, a recommend and rows are not a briefing's: any of these on the wrong kind is `invalid`, on a replay as on a write. The answer is a verdict in `briefing`, `accept`, or `send_back` with a note that is not blank, and in `file` any of the left-out items to file, by number from 1, as a `choice` numbers the options: each names an item, and none is given twice. `file` goes only with a briefing's verdict: beside `drop`, without a verdict, or on any other kind, it is `invalid`. An amend's `left_out` replaces the case's whole list, and one other than the case's changes the question, so it moves `amend_seq`: an answer files by number, and one that numbered the old list is `stale`. An amend's `landed` replaces the case's and does not move `amend_seq`. It is the one field an amend reads at its zero value: `landed: false` sets the case's to `false`, and on an amend of any other kind it is `invalid`, as `true` is. A briefing does not park, and moves as a signoff does: a send back is answered by the agent's amend with `reopen`.

`re` is the case the open is about, such as the message an agent's question follows, by its id, as text. Like `for`, it is not blank when given, on a replay as on a write, and nothing else is checked: the core never reads it as an id (see [Not covered](#not-covered)).

A note or an amend that reopens a case says so with `reopen`: a note on an `answered` or `pickedup` case, and the agent's amend of one, reopen it. The case is `open` again, and its answer and pickup are cleared; the events keep them. An amend without `reopen` is a `transition` on such a case, as it was; a note without it is refused by the writer as a `transition` (see [Rules only a write applies](#rules-only-a-write-applies)), so that a follow-up never reopens a case by accident. `reopen` on a note or an amend of a case that is `open` or `sent` is `invalid`, on a replay as on a write: there is nothing to reopen. A message is never reopened: the human's amend of a picked-up one is a `transition`, with `reopen` or without. An amend that only reopens, and changes nothing else, is `invalid`, as any amend that changes nothing is: it asks the human again, so it says what changed.

An amend's `title` replaces the case's, and is `invalid` when it is only white space. `drop_links` takes links off the case: each must be one of the case's links, matched exactly, and given once, and none may be a link the same amend adds; any other is `invalid`. The links left keep their order, and the amend's own `links` go after them. Neither moves `amend_seq`, for the reason a recommend does not: a build from before the fields skips such an amend as changing nothing.

An answer's `amend_seq` is the `amend_seq` of the case as its writer read it (ST-04). Left out, it is 0, and below 0 it is `malformed`. One below the case's is `stale`: the answer was written without seeing the question as it now stands. One above the case's names a change no writer can have seen, and is `invalid`.

## Reading a record

A row's record reads as its event's record when:

- it is one JSON object, with nothing but white space around it;
- no object in it, at any depth, gives a key twice;
- each field its event defines, where given, has the field's type and is not `null`. A record of `null`, or a field of `null`, is malformed: `null` is not a field left out;
- each time reads as a time (see [Definitions](#definitions)).

Field names are matched exactly (ST-06): `Kind` is not `kind`. A field the event does not define, including one whose name differs from a field's only in case, is ignored when the record is read and kept in the event's `data`. So a record from a newer version, with fields this one does not know, still folds (ST-05).

A field at its zero value, an empty string, 0, `false` or an empty list, means the same as the field left out, but for an amend's `landed` (see the briefing above): `{"choice": 1, "other": false}` is a choice. A time has no zero value: `0001-01-01T00:00:00Z` is a time in year 1, like any other (ST-06).

A record without its time folds, with no time. The writer gives a record without one the time `now`.

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
5. Check the event against the case with the rules a replay uses, in the same order. An appended open, and an appended note, must also keep the [write rules](#rules-only-a-write-applies). A refused append has the category a replay would give the row, or the category the write rule gives, and leaves every property of the case as it was.
6. An accepted append changes the case. Check `expect` against it. Then fold the rows with the new row added, and check that this gives the same case.

The core never reads a clock during a replay. Each record carries its own time (`opened_at`, `answered_at` and so on), and the vectors fix it. The fold does not read the events table's `at` column, so the vectors do not give one.

### Rules only a write applies

The writer refuses, as `invalid`, an open that gives an option twice, a link of only white space or a link twice, or a `worker`, `brief` or `context` of only white space (AG-02). A stored open that does any of these still folds.

The writer refuses, as a `transition`, a note on an `answered` or `pickedup` case without `reopen` (cases 6M6gx and AV8s1). It is checked with the state, before the note's own rules. A stored note without it still reopens the case: every note did before the field, and stores hold such notes. No row the writer stores from now on lacks it, and a replay reads one that does as the reopen it was.

These are the only rules a replay and a write do not share: a stored open whose `recommend` names no option, for one, does not fold, and nor does a stored note with `reopen` on an open case.

## The case

`expect` lists properties of the case. A runner checks the ones given and ignores the rest.

| Property | Value |
| --- | --- |
| `state` | `open`, `answered`, `pickedup`, `closed`, `withdrawn`, `parked` or `sent` |
| `revision` | The number of stored rows, skipped ones included |
| `amend_seq` | The `seq` of the last amend that changes the question: one that adds options or rows, or sets a body or a `left_out` other than the case's. 0 when there is none. An amend of links, labels, context or the recommend, or of the body the case has, does not count (ST-04) |
| `updated_at` | The latest time of the events that folded, or `null` |
| `kind`, `urgency`, `title`, `body`, `worker`, `brief`, `context`, `for`, `re` | From the open event, with `title`, `body` and `context` as the last amend that set them left them. `""` when not set |
| `options`, `rows`, `links`, `labels` | The open event's, then each amend's, in order, less the links an amend dropped. `[]` when there are none |
| `landed`, `left_out` | The open event's, as the last amend that set them left them. `false` and `[]` when not set |
| `recommend` | The open event's, as the last amend that set one left it. 0 when the case recommends none |
| `actor`, `opened_at` | The open event's, or `null` |
| `answer` | The answer the case stands on, or `null`. A note or an amend that reopens the case clears it |
| `pickup` | The pickup record, or `null`. A note or an amend that reopens the case clears it |
| `park` | The park record while the case is parked, or `null` |
| `close` | The close record, or `null` |
| `events` | The events that folded, in order |
| `problems` | The rows that were skipped, in order |

Each element of `events` has:

- `seq`, `author` and `event`, as stored
- `file`, the name the event had as a file: `seq` as at least four digits, the author and the event, such as `0004-agent-pickup.json`. Readers name events and skipped rows by it (AG-13, ST-05)
- `from`, the state before the event, left out for the open event
- `at`, the event's time, left out when it has none
- `actor`, left out when it has none
- `data`, the record as stored, as a JSON value

Each element of `problems` has the row's `seq` and `file`, and the `category` of the reason it was skipped.

Records (`answer`, `pickup`, `park`, `close`, `actor` and each element of `rows`) are JSON objects with the field names a stored record uses, such as `answered_at`. Each is the record as its event reads it: a field the event does not define, such as a `name` beside an actor's `id`, is left out, though `events[].data` keeps it. A field at its zero value is left out: an empty string or list, 0, `false`, or no time. So `{"choice": 1, "answered_at": "2026-09-15T09:02:00Z"}` is an answer with no note, and a pickup record with nothing in it is `{}`.

Times are RFC 3339 in UTC, ending in `Z`, with only as many fractional digits as needed, such as `2026-09-16T22:15:41.605108Z`.

Each property is compared as a whole JSON value, except `events` and `problems`. For those, the list must have the same length, and each element is checked only on the properties the vector gives it, where `null` means the property is absent. An element names only the properties listed above.

## Freshness

A vector in `freshness.json` is a replay with a form drawn from the case at some point. It has one of:

- `drawn_after: N`: the form was drawn from the case the first `N` rows fold to, so it keeps that case's `revision` and the highest `seq` among those rows, skipped ones included (`last_seq`). `N` is from 1 to the number of rows.
- `drawn_at: {"revision": R, "seq": S}`: the form was drawn at these, as given.

It has no `append`, and its `expect` gives `since`: how the case the rows fold to changed since the form was drawn. `since` is a property only these vectors give. It is one of:

- `unchanged`: nothing changed, or only notes and amends that kept the case in its state and left the question as it was. A write drawn then may take the new revision.
- `moved`: the case moved to other states, each passed once, by rows that all folded, and with the question as it was. A form that fits the new state may take the new revision.
- `changed`: anything else. A write drawn then keeps its revision, and the writer refuses it as `stale`.

The events since are the events with a `seq` above the form's. The answer is the first of these that holds (IN-05, IN-06):

1. The form's revision is the case's: `unchanged`.
2. The form's revision is above the case's: `changed`.
3. The events since are not as many as the rows added since the form's revision: `changed`. Fewer means the fold skipped a row; more means the form's `seq` is below the one its revision had.
4. The case's `amend_seq` is above the form's `seq`, so an amend since changed the question: `changed`. A message is the exception: its amends are the human's own (IN-13).
5. Every event since is a note or an amend whose `from` is the case's state now: `unchanged`.
6. The states the case passed are each event's `from`, then its state now. An event since with no `from`, the open, gives `changed`: the form was drawn before the case had a state. If no state comes back once the case has left it: `moved`. Otherwise, as in an answer and a reopen, or a park and a resume: `changed`.

The rule is the core's; what an inbox does with the answer, such as closing a form that does not fit the new state, is not.

## Stats

A vector in `stats.json` gives the stats of some cases, as an inbox shows them: how many were opened and answered on each of the last few days, and the median time the human took to answer. It has its own shape:

```json
{
  "description": "a case answered the day it opened counts both on that day",
  "cases": [
    [
      {"seq": 1, "author": "agent", "event": "open", "data": {"kind": "stuck", "urgency": "today", "title": "Mirror unreachable", "body": "Body.", "opened_at": "2026-11-02T09:00:00Z"}},
      {"seq": 2, "author": "human", "event": "answer", "data": {"text": "Try the other mirror.", "answered_at": "2026-11-02T09:10:00Z"}}
    ]
  ],
  "now": "2026-11-02T12:00:00Z",
  "days": 3,
  "expect": {"opened": [0, 0, 1], "answered": [0, 0, 1], "median": 600}
}
```

- `cases` lists each case's stored rows, as a vector's `events`, and `[]` when there are none. Every case folds.
- `now` is the clock, a time in UTC, and `days` the number of days counted, 0 or more.
- `expect` gives `opened` and `answered`, one count for each day, the oldest first and the day of `now` last, and `median`, the median time to answer in whole seconds, or `null` when there is none.

A runner folds each case's rows, and works out the stats of the cases:

1. The days are UTC days, each from `00:00:00Z` to just before the next. The last is the day `now` falls on, the first `days - 1` days before it. A time on any other day, before the first or after the last, is not counted.
2. A message is left out: the human opens it, and does not answer it.
3. A case counts as opened on the day of its `opened_at`.
4. Each answer the case took counts on the day of its `answered_at`: a case answered, reopened and answered again counts twice. An answer with `drop` is not an answer.
5. An answer's time to answer runs from when the case was last asked: its `opened_at`, or the time of the note or amend that last reopened it, an event from `answered` or `pickedup` after which the case is `open`. A park and a resume do not ask again.
6. The median is over the times to answer of the answers counted, in full, with their fractions of a second. With an even number of them it is the mean of the middle two. It is cut to whole seconds.

An event without a time is not counted, and a reopen without one leaves the answers after it with no time to answer, as does an open without one. A time to answer below zero, as a clock behind gives, is left out of the median; the answer still counts.

## Error categories

| Category | Meaning |
| --- | --- |
| `unknown` | An event type, kind or urgency this version does not know, perhaps written by a newer cases |
| `author` | The author may not write this event. The agent writes `pickup`, `note` and `close`, the human `answer`, `park` and `resume`, and either writes `withdraw`. Either writes `open` and `amend`, which belong to the side that opens the kind: the human's on a message, the agent's on every other kind, and the other side's is refused. Any other author is refused |
| `malformed` | The record does not read as the event's record (see [Reading a record](#reading-a-record)): it is not one JSON object, it gives a key twice, a field is `null` or of the wrong type, or a time does not read |
| `invalid` | The record reads but breaks a rule of its event, such as a missing kind, a blank title, a choice that is not an option, or an amend that changes nothing |
| `transition` | The case's state does not allow the event, or its kind does not: `park` on a case that is not stuck, and `note` on a message. A `withdraw` of a sent message or an answered or picked-up case is the human's alone: the agent's is a transition. An amend of an answered or picked-up case without `reopen`, and a written note on one without `reopen`, are transitions. An event before the open, and a second open by either author, are transitions too |
| `stale` | An append whose `at_revision` is not the case's revision, or an answer whose `amend_seq` is below the case's: it was written without seeing the question as it now stands (ST-04) |
| `no-open` | The case does not fold: no row is a valid open event |

When a row is wrong in more than one way, its category comes from the first check it fails:

1. The event type is known (`unknown`).
2. The author may write the event, on some kind of case (`author`).
3. The record reads (`malformed`).
4. An `actor`, when given, has an `id` of 22 base64url characters (`invalid`).
5. A case with no open event takes only an open (`transition`).
6. An amend's author is the side that opens the case's kind (`author`).
7. An answer's `amend_seq` is not below the case's (`stale`).
8. The state allows the event (`transition`), then the record keeps its event's rules (`invalid`). For a note on an answered or picked-up case, the write rule of `reopen` is part of the state's check. `reopen` on a case that is open or sent is checked straight after the state: for a note, after its body; for an amend, before its other rules. An open's kind is checked first: one left out is `invalid`, one this version does not know is `unknown`. Then its author is the side that opens that kind (`author`), then its urgency is checked as its kind is. Then come the open's other rules, its recommend's among them, and for an appended open the write rules, all `invalid`.

An append checks `at_revision` before all of these, once the rows fold. An appended open is checked in the same order as a stored one.

`transitions.json` tries every event, by each author that may write it, from every state of a stuck case and of a message. It also tries an open and an amend of each kind by the side that does not open it, since that is checked once the kind is known. A runner can check that it still does both.

## Definitions

- **Blank** means empty once white space is taken off both ends. White space is what has the Unicode White_Space property: a no-break space (U+00A0) is white space, a zero-width no-break space (U+FEFF) is not.
- Authors, event types, kinds, urgencies, verdicts and signoff values are matched exactly, so `Decision` is not a kind.
- A row id must match `^[A-Za-z0-9._-]+$` as a whole, so an id that ends in a newline does not.
- A whole number has no fraction part: `2.0` and `-0.0` are not whole numbers. `-0` is one, and is 0.
- A **time** is an RFC 3339 `date-time` (section 5.6), read from the JSON string's value, so an escape such as `\u005a` is the letter it spells. That is a date `YYYY-MM-DD`, a `T` or `t`, a time `hh:mm:ss`, then optionally a `.` and one or more digits, then `Z`, `z` or an offset `+hh:mm` or `-hh:mm`. The date is a day of the calendar, the hour 00 to 23, the minute 00 to 59 and the second 00 to 59, and an offset's hour 00 to 23 and minute 00 to 59. Anything else is malformed, such as a space in place of the `T`, an hour of one digit, a comma before the fraction, or a leap second (`23:59:60`), which RFC 3339 allows at the end of some days but no clock the writer reads gives.

## Store vectors

The files in `store/` test what a store keeps beside its cases, through the core's `Store` trait: its version, the cases changed since one, its table of actors, and the sessions listening for messages (ST-16, ST-12). The core does no IO and keeps nothing, so the vectors run against a store: `crates/vectors` (`cases-vectors`) is the runner, and a store runs them by implementing its `Harness` in its own tests. The core's tests run them against a store kept in memory, the smallest the trait allows, and `cases-store-sqlite`'s against a SQLite file. Both must pass every one.

| File | Vectors | What they cover |
| --- | --- | --- |
| `store/version.json` | 6 | The version moves on an open, each event, an archive and a delete, and an open after a delete; it stays on a refused event, open, archive or delete, and on listening and doing lines |
| `store/changes.json` | 9 | `rows_since`: every case with no version, nothing since the version now, the cases changed since an earlier one, and those archived or deleted since as removed |
| `store/actors.json` | 4 | One actor for each writer, of each name and kind, in the order made, and none for a refused write |
| `store/listening.json` | 21 | The local listening record: labels, a heartbeat, the lapse after 45 s, a worker listening again, and the doing line and its rule (AG-17) |

A store vector is an object with a `description`, unique within its file; `local`, `true` for a vector only a store that keeps the local listening record runs; and `steps`, run in order against one new store, whose clock starts at `2026-09-15T09:00:00Z`. Each step is one action, with what it takes and what it expects:

| Action | Takes | Does |
| --- | --- | --- |
| `clock` | | Sets the store's clock to this time |
| `open` | `as`, `author` (default `agent`), `writer` | Creates a case of this open record, by the writer of this name (none for the unnamed writer), and names it `as` |
| `append` | `author`, `event`, `data`, `writer`, `at_revision` | Appends to the case named |
| `archive`, `delete` | `at_revision` | Archives or deletes the case named |
| `listen` | | `{"wait", "worker", "labels", "kinds", "doing"}`: records the wait as listening, a heartbeat |
| `set_doing` | | `{"worker", "text"}`: sets the worker's doing line |
| `version` | `same_as`, `differs_from` | Reads the version, names it, and checks it is the same as one named before, or differs from each of a list |
| `rows_since` | `expect` | Reads the cases changed since the version named, or with `null` since none |
| `actors` | | Checks the table of actors: each actor's `kind` and `name`, in order |
| `listeners` | | Checks the workers listening now: each one's `worker`, `labels` and `doing`, and, when given, `since`, `heard`, `doing_at` and `waits`, each wait's `wait`, `labels` and `kinds`, by id. It also checks that the waits listening now are the listeners' waits, and that each listener's labels, times and doing line agree with its waits |

A write, `listen` or `set_doing` may give `expect_error`, the category it is refused with: a core category, `no-case` for a case the store does not have, `missing` for a store not made yet, `invalid` for a blank worker or a doing line that is not one, or `not-listening` for a doing line of a worker not listening. Without it, the action must be taken.

A version is opaque, so a vector only compares versions. `rows_since`'s `expect` gives `live`, every case in the store, in the order opened; `changed`, the cases with a row stored since the version; `removed`, the cases archived or deleted since; and `removed_may`, cases opened and removed since, which an answer may name. The store may give any answer the trait allows, and the runner checks the one it gives:

- nothing changed, only when the version is the one asked about, and `changed` and `removed` are empty;
- what changed: exactly `changed`, and `removed` with perhaps some of `removed_may`;
- every case: exactly `live`.

With no version, the answer must be every case. Every case an answer gives must have the rows the store gives for it now, and its version must be the store's. An answer of what changed names no key twice.

Each event an `open` or `append` stores must carry an actor that the table of actors has, of the event's author and the writer's name (ST-12).

## Not covered

The vectors cover the fold and the writer's rules for one case. They leave out:

- text for people: error messages, the lines of a case's thread, CLI output;
- case ids and their slugs (ST-10), which belong to the store and the CLI;
- the store beyond [Store vectors](#store-vectors): SQLite, the archive's tables, the poller, the lock, and writers running at the same time;
- the writer's actor, which a store hands the writer for who writes: the record gets it in place of any actor of its own, once that one has been checked. The vectors write with none;
- the CLI and the web inbox, such as the exit code for each category (CL-01: `transition` 3, `stale` 4, the rest 1), the web status codes (IN-04: 409 stale, 422 refused, 404 unknown case, 400 bad form) and `cases park` as a command of its own (HU-03);
- details of decoding that no vector pins yet: invalid UTF-8, a number too large for its field, a number with an exponent, and a fraction of a second with more than nine digits.

The user stories hold the behaviour outside the core, with the Go tests each came from.

## The Go vectors

The Go runner, `TestVectors` in `internal/store/vectors_test.go` of `ryanlewis/cases`, checks the set as it was at 508548b against the Go core. It does not run this set.

## Changing a vector

A vector records a rule of the core. Change one only with the rule it records, in the same commit, and add the change to [CHANGES.md](CHANGES.md) with the story or decision behind it. Keep descriptions unique within a file, use `data` unless the bytes matter, and keep the table of files in step with the files.
