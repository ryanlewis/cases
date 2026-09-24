# Store format

The store is one SQLite file. By default it is `$XDG_DATA_HOME/cases/cases.db`,
which is usually `~/.local/share/cases/cases.db`. Name another with `--store`,
`CASES_STORE` or the [config file](configuration.md). The first `cases open`
creates the file and its directory; until then every command reads the store
as empty and creates nothing.

While a command or `cases serve` has the store open, SQLite keeps two more
files beside it, `cases.db-wal` and `cases.db-shm`, and removes them when the
last one closes it. After a crash they can hold writes that are not in
`cases.db` yet; the next command that opens the store moves them in, so keep
the three files together.

A store written by an earlier `cases`, a directory with one JSON file per
event, is not read, and nothing is carried over from it. Named with `--store`,
`CASES_STORE` or the config file, the directory is refused; name a file
instead. The earlier default store was the directory `~/.local/share/cases`
itself, so the new default file sits inside it, beside the old case
directories, which are left as they were and not read. Update `cases`
everywhere that uses a store at once: an earlier `cases` keeps writing case
directories that this one does not see.

## Tables

| Table | One row per | Columns |
| --- | --- | --- |
| `cases` | case | `id`, `opened_at` |
| `events` | event | `change`, `case_id`, `seq`, `author`, `event`, `at`, `data` |
| `archive_cases` | case `cases prune` archived | `id`, `opened_at`, `archived_at` |
| `archive_events` | event of an archived case | as in `events` |
| `meta` | store (one row) | `schema_version`, now `1` |

A case's id is the time it was opened (UTC) and a slug of its title, such as
`2026-09-15T09-12-03Z-pin-bun-or-float`. If a case in the store or its archive
already has that id, `-2`, `-3` and so on are added.

Each event is one row in `events`:

- `seq` numbers the case's events from 1, in the order they were written. No
  two events of a case have the same number.
- `author` is `agent` or `human`.
- `event` is `open`, `amend`, `answer`, `pickup`, `note`, `close`,
  `withdraw`, `park` or `resume`.
- `at` is the time the record gives, RFC 3339 in UTC.
- `data` is the event's JSON record exactly as written, below.
- `change` numbers every event in the store and only goes up, even after
  `prune`. `serve` and `wait` use it to find the cases that changed since they
  last read the store.

Each event also has the name it would have as a file,
`NNNN-<author>-<event>.json`. `cases show --json` prints it as `file`, problems
name the event by it, and `wait` and the browser notifications use it to tell
events apart. A case's events, with what each record holds:

```
2026-09-15T09-12-03Z-pin-bun-or-float
  0001-agent-open.json       # kind, urgency, title, body (markdown), options[], rows[], links[], labels[], worker, brief, context, for
  0002-agent-amend.json      # options[], rows[], links[], labels[] to add; body, context to replace; amended_at
  0003-human-answer.json     # choice / rows / signoff / text / drop / ack, note, answered_at
  0004-agent-pickup.json     # picked_up_at, by
  0005-agent-note.json       # follow-up question, reopens the case
  0006-human-answer.json
  0007-agent-pickup.json     # a reopened case is picked up again before it closes
  0008-agent-close.json      # outcome (markdown), links, closed_at
```

Timestamps in records are RFC 3339 in UTC.

The `labels`, `worker`, `brief` and `context` fields on `open` are optional
and help the human act on a case. `labels` group cases, such as the ones one
piece of work opened; a label may not be blank or appear twice on a case.
`worker` names the agent session waiting on it. `brief` says where to restart
the work from if the case is parked: a brief, a ledger or a note, as a path or
a short line. `context` is free text shown with the case. `for` names who the
case is addressed to, such as the human expected to answer it; nothing checks
it against who answers. An amend cannot change it.

Every event may carry an optional `actor`, the name and kind of whoever wrote
it, such as `{"name": "Ryan", "kind": "human"}`. `show` and the web thread
print it as `by NAME`, and `show --json` has it on each event (the open
event's `actor` and `for` also appear at the top level with the other open
fields). The CLI records the human's name from `--as` or the `name` config key
on `answer`, `resume` and the inbox, and the case's `worker` on the agent's
events; when neither is set it records no actor. The store checks only that an
actor it is given has a name and a kind, and never who may write what.

An `amend` changes a case that is still open. Its `options`, `rows`,
`links` and `labels` are added after the ones the case has, and its `body` or `context`
replaces the case's. A field it leaves out stays as it was. Nothing can be
removed, and options keep their numbers.

An amend is refused if it sets a blank `body`, `context`, option, link or
label, adds an option, link, label or row `id` the case already has, or changes nothing, such as
setting the `body` the case already has. So the same amend sent twice writes
nothing the second time.

The `open` record stays as it was written. The case shows the amended fields,
and the answer is checked against them: an approval answer needs a verdict on
the added rows too. An answer is numbered after every event before it, so it
has seen every amend.

## Writes

Each write is one transaction. It takes the store's write lock, reads the case,
checks `--revision` if one was given, checks the event against the case with
the same code that reads it, and adds the event numbered after the case's
latest. A refused write changes nothing. Writers take turns, whether they are
in one process or several, so two writers never take the same number. A writer
that waits more than 5 seconds for the lock fails with `database is locked` and
writes nothing; a long run of writes by another command can keep it waiting
that long, though `sweep` and `prune` pause briefly after each write to let
other writers in. Each write is synced to disk before the command returns.

A case's state is worked out by reading its events in order. It is never
stored. Events are never changed or removed, and closed cases are kept as the
decision log until `cases prune` moves them to the [archive](#archive).

Change the store only through `cases`. Keep the file on a local disk that only
this machine uses: not in a synced folder (Dropbox, iCloud Drive, Syncthing),
where a sync client can copy it halfway through a write, and not on a network
share (NFS, SMB) or a folder shared into a VM or container, where SQLite's
locks and shared memory do not work.

To back the store up while `cases` may be using it, run
`sqlite3 cases.db ".backup cases-backup.db"`. Stop `cases serve` and any
`cases wait` before removing, moving or replacing `cases.db`: a process that
still has the store open keeps using its `cases.db-wal` and `cases.db-shm`, and
a file put in its place would be read with them. To start again, stop them,
then delete `cases.db`, `cases.db-wal` and `cases.db-shm` together. If
`cases.db` is gone but either of the other two is still there, `cases` refuses
to make a new store until both are deleted or the store is put back. Putting
the store back is the safe choice: deleting them throws away any writes that
were only in `cases.db-wal`. To restore a
backup, stop them, delete `cases.db-wal` and `cases.db-shm`, and copy the
backup to `cases.db`.

## States

```
open --answer--> answered --pickup--> pickedup --close--> closed
open --withdraw--> withdrawn
answered or pickedup --note--> open        (a follow-up question)
open --park--> parked --resume--> open     (stuck cases only)
open --amend--> open                       (a change before the answer)
```

A note on an open case adds to the thread and leaves it open. An amend is only
allowed on an open case, and also leaves it open. Anything else, such as
closing a case that has not been picked up, is refused and nothing is written.

## Kinds and answers

| Kind | Open with | Answer with |
| --- | --- | --- |
| `decision` | one or more `options` | `choice` (1-based option number), or `other` with a note |
| `approval` | `rows`, each with `id`, `label`, `script` (the text itself), `link` and an optional `note` shown under the label | a verdict per row: `approve` (run it), `hold` (not now, ask again later) or `reject` (never run this row, shown as "don't run" in the CLI and the web inbox), each with an optional note |
| `signoff` | | `accept`, or `changes` with a note |
| `stuck` | | guidance `text`; parking is a separate `park` event |
| `question` | | reply `text` |
| `fyi` | | `ack` |

Any kind may be answered with `drop` instead, which dismisses the case: the
human does not want the work done. A `drop` answer sets nothing else but the
note. Every answer may carry a `note`.

## Archive

`cases prune` moves whole cases, every event with them, from `cases` and
`events` into `archive_cases` and `archive_events`, one transaction per case.
`list`, `show`, `wait` and `serve` read only `cases` and `events`, so an
archived case is out of sight. Its events are as they were written, and its id
is not given to a new case.

There is no command to restore a case. To put one back, back up the store and
move its rows back in one transaction. `.bail on` stops at the first error, so
a failed step leaves the archive as it was:

```sh
sqlite3 ~/.local/share/cases/cases.db <<'SQL'
.bail on
.timeout 5000
BEGIN IMMEDIATE;
INSERT INTO cases (id, opened_at)
  SELECT id, opened_at FROM archive_cases WHERE id = '2026-08-01T10-00-00Z-old-question';
INSERT INTO events (change, case_id, seq, author, event, at, data)
  SELECT change, case_id, seq, author, event, at, data FROM archive_events
  WHERE case_id = '2026-08-01T10-00-00Z-old-question';
DELETE FROM archive_events WHERE case_id = '2026-08-01T10-00-00Z-old-question';
DELETE FROM archive_cases WHERE id = '2026-08-01T10-00-00Z-old-question';
COMMIT;
SQL
```

## Damaged events

An event whose record is not valid JSON, or that the case could not accept at
that point, such as one written by hand or by a newer `cases`, is skipped. The
case still loads, and the event is reported as a problem under its file name
(`cases show` lists it; `list` and `wait` print it as a warning on stderr). A
case whose open event is damaged is reported and the other cases are still
listed. Fields the CLI does not know are kept: `cases show --json` prints every
event's record as written.

An unknown event, kind or urgency is reported as possibly written by a newer
`cases`, with the `go install` command that updates it. The same message comes
from an event that `cases` did not write, so check where it came from before
updating. A store whose schema version is newer than this `cases` reads is
refused with the same advice.
