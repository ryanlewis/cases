# cases

`cases` is a Go CLI for passing questions from a coding agent to a human and
getting the answer back. The question is called a case.

1. The agent opens a case: a decision to make, scripts to approve, work to
   sign off, a blocker, or something the human should know.
2. The agent waits in the background.
3. The human answers from the terminal or from a local web inbox
   (`cases serve`).
4. The agent picks up the answer, acts on it, and closes the case with what
   happened.

The cases live in one SQLite file on this machine, and every event is kept as
it was written. No server is needed.

## Install

With Homebrew, on macOS or Linux:

```sh
brew install ryanlewis/tap/cases
```

Or download the archive for your platform from the
[latest release](https://github.com/ryanlewis/cases/releases/latest) and
verify it against the published checksums:

```sh
gh release download vX.Y.Z -R ryanlewis/cases -p 'cases_X.Y.Z_<os>_<arch>.*' -p checksums.txt
sha256sum -c checksums.txt --ignore-missing
tar -xzf cases_X.Y.Z_<os>_<arch>.tar.gz cases   # unzip the .zip on windows
install cases /usr/local/bin/cases
```

GitHub also keeps a build provenance attestation for each archive, which
shows that this repository's release workflow built it from the tag:

```sh
gh attestation verify cases_X.Y.Z_<os>_<arch>.tar.gz -R ryanlewis/cases \
  --signer-workflow ryanlewis/cases/.github/workflows/release.yml \
  --source-ref refs/tags/vX.Y.Z
```

The macOS binaries are signed and notarized. The binary includes SQLite;
nothing else needs installing.

Or install from a checkout with Go on the path:

```sh
git clone git@github.com:ryanlewis/cases.git
cd cases
make install    # go install ./cmd/cases
```

Then install the agent skill. It teaches the agent when to open a case, how to
wait for the answer and how to act on it. It supports Claude Code, Codex and
Pi; see [skill](#skill).

```sh
cases skill install claude    # also: codex, pi
```

## A first case

This runs one case end to end in the default store,
`~/.local/share/cases/cases.db`. In real use an agent runs the agent's
commands; to try it, run both sides in one shell.

```sh
# Agent: raise a decision and wait for the answer in the background.
echo "Pin bun to 1.2.3, or float it and fix the lockfile when it breaks?" > question.md
id=$(cases open --kind decision --urgency blocking --title "Pin bun or float?" \
  --body-file question.md --option "Pin to 1.2.3" --option "Float and fix the lockfile")
# --since "$id" stops wait missing an answer that lands before it starts.
cases wait --id "$id" --since "$id" --timeout 2h > answered.jsonl &

# Human: answer it, from here or from the inbox that cases serve opens.
cases answer "$id" --option 1 --note "Revisit after 1.3"

# Agent: record that the answer was read, act on it, and close with the outcome.
cases pickup "$id" --by bun-pins
cases close "$id" --outcome "Pinned in #12."
cases show "$id"
```

## Store format

The store is one SQLite file. By default it is `$XDG_DATA_HOME/cases/cases.db`,
which is usually `~/.local/share/cases/cases.db`. Name another with `--store`,
`CASES_STORE` or the [config file](#configuration). The first `cases open`
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

### Tables

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

### Writes

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

### States

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

### Kinds and answers

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

### Archive

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

### Damaged events

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

## Configuration

A TOML file supplies defaults for `--store`, `--listen`, `--no-open` and
`prune --age`, so a store kept outside the default location needs naming only
once, and a scheduled `cases prune --yes` needs no arguments. Precedence is
flag > environment variable > config file > built-in default.

The file is read from `$XDG_CONFIG_HOME/cases/config.toml`, falling back to
`~/.config/cases/config.toml`. Override the location with `--config PATH` or
the `CASES_CONFIG` environment variable. The file is optional.

```sh
cases config init    # write a commented template (refuses to overwrite; --force to replace)
cases config path    # print the file in use and whether it exists
cases config show    # print the defaults the environment and the file establish
```

| Key | Sets | Beaten by | Default |
| --- | --- | --- | --- |
| `store` | `--store` | `CASES_STORE` | `$XDG_DATA_HOME/cases/cases.db`, or `~/.local/share/cases/cases.db` |
| `listen` | `--listen` on `serve` and `service install` | nothing | `127.0.0.1:8765` |
| `no-open` | `--no-open` on `serve` | nothing | `false` (`true` or `false`, quoted or not) |
| `name` | `--as` on `answer`, `resume`, `serve` and `service install` | nothing | none: no actor is recorded |
| `prune-age` | `--age` on `prune` | nothing | `720h` |

```toml
store = "~/cases/work.db"
```

A leading `~` in `store` is expanded. `CASES_STORE` set to an empty string
counts as unset.

Values are strings, except `no-open`, which also takes an unquoted `true` or
`false`. An unknown key, a value of the wrong type, or malformed TOML is an
error that names the file. It stops every command except `cases config path`,
`cases config show` and `cases config init`, which are how you find out which
file is at fault.

## CLI

Every command opens the store file itself; nothing needs to be running. Name
the store with `--store FILE` before the command, as in
`cases --store ~/cases/work.db list`, or with `CASES_STORE` or the `store`
config key.

Agent side:

```
cases open     --kind KIND --urgency blocking|today|whenever --title TEXT
               [--body TEXT | --body-file FILE|-] [--option TEXT]...
               [--row JSON]... [--link URL]... [--label TEXT]...
               [--worker NAME] [--brief TEXT] [--context TEXT]
               [--for NAME]
cases amend    ID [--body TEXT | --body-file FILE|-] [--option TEXT]...
               [--row JSON]... [--link URL]... [--label TEXT]...
               [--context TEXT] [--revision N]
cases wait     [--for agent|human] [--since TIME|ID] [--timeout DURATION]
               [--id ID]... [--kind KIND]... [--label TEXT]...
               [--worker NAME]...
cases pickup   ID [--by NAME] [--revision N]
cases note     ID --body TEXT | --body-file FILE|- [--revision N]
cases close    ID --outcome TEXT | --outcome-file FILE|- [--link URL]...
               [--revision N]
cases withdraw ID [--reason TEXT] [--revision N]
```

Human side:

```
cases answer ID --option N | --other | --row ID=VERDICT[:NOTE]... |
                --accept | --changes | --text TEXT | --text-file FILE|- |
                --park | --drop | --ack
                [--note TEXT] [--revision N] [--as NAME]
cases resume ID [--agent] [--revision N] [--as NAME]
cases sweep  [--reason TEXT] [--older-than DURATION] [--kind KIND]...
             [--label TEXT]... [--worker NAME]... [--yes]
cases prune  [--age DURATION] [--state closed,withdrawn] [--delete] [--yes]
```

Both:

```
cases list   [--state STATE,...|--all] [--urgency URGENCY]...
             [--older-than DURATION] [--kind KIND]... [--label TEXT]...
             [--worker NAME]... [--count|--json]
cases show   ID [--json | --answer]
cases serve  [--listen 127.0.0.1:8765] [--no-open] [--as NAME]
cases status [--json]
cases inbox  [ID] [--print]
```

`list --json` prints one object, `{"list": [...]}`, the matching cases as an
array under `list`, `[]` when none match. `wait` prints one object per line
instead; see below.

Setup:

```
cases skill install   AGENT [--path DIR] [-y]
cases skill uninstall AGENT [--path DIR] [-y]
cases skill show      [AGENT]
cases skill list
cases skill check     [AGENT]
cases service         [--json]
cases service install [--listen 127.0.0.1:8765] [--as NAME]
cases service uninstall
```

`open` prints the new case id. The other write commands print the id and the
new state.

`answer`, `resume` and `show` also take part of a case id, so
`cases show mirror` works when exactly one case id contains `mirror`. An exact
id always wins. If no id contains it the command fails, and if more than one
does it fails and prints the ids that match. A whole id, the timestamp and
slug, is taken exactly, so a case that has been pruned is not mistaken for a
later one such as `ID-2`.
The agent commands, `wait --id` and the web inbox take the exact id only.

A command on one case exits 3 when the case's state does not allow it, such as
`pickup` on a case that is not answered or `close` on one that is not picked
up; the `Error:` line says which. Exit 3 means the store refused the event, not
that it was already done, so read the case to see where it is. Other errors
exit 1, and `wait` exits 2 on timeout. `sweep` and `prune` exit 1 when any case
fails.

`cases show` prints the case's revision, the number of events it has, and
`show --json` and `show --answer` have it as `revision`. Pass it to `answer`, `resume`, `amend`,
`pickup`, `note`, `close` or `withdraw` as `--revision N` and the write is refused, with nothing written, if an event has
been added to the case since it was read. The error names the revision read and
the current one, and the command exits non-zero.

`--body` on `open`, `note` and `amend` and `--outcome` on `close` take the text
inline, for a line; `--body-file` and `--outcome-file` read markdown from a
file, or from stdin with `-`. Give one or the other, not both; `note` and
`close` need one. An inline value that names an existing file is refused, so
`--outcome outcome.md` is not stored as the text `outcome.md`.

`--text-file` on `answer` reads the guidance or reply from a file, or from stdin
with `-`, in place of `--text`. An empty file is refused.

`show --answer` prints only what an agent needs to act on the answer, as one
JSON object: `state`, `kind`, `revision` and `answer`. `answer` is `null` when
the case has none, such as an open, parked or reopened case, and the command
exits 0 either way.

`--row` on `open` and `amend` takes one JSON object per row, for example
`--row '{"id":"deps","label":"Install deps","script":"npm ci","link":"https://…"}'`.
Add `"note":"…"` to show a line under the row's label.

`--label`, `--worker`, `--brief`, `--context` and `--for` on `open` set the fields described in
[store format](#store-format). `open` takes the worker from `CASES_WORKER` and
one label from `CASES_LABEL` when the flags are not given, so a session can
export them once. `CASES_LABEL` is always exactly one label: the whole value,
commas and spaces included. A `--label` flag replaces it rather than adding to
it, and `CASES_LABEL` set to an empty string is a blank label, which `open`
refuses. Only `open` reads them; `list`, `wait` and `sweep` do not. `--by` on `pickup` records who picked the case
up, such as the agent session name. `--reason` on `withdraw` records why the
case no longer needs an answer; `show` and the web thread print it.

`amend` changes an open case as described in [store format](#store-format):
`--option`, `--row`, `--link` and `--label` add to the case, and `--body` or
`--body-file` and `--context` replace its body and context. It refuses an amend
that changes nothing, an empty `--body` or body file and an empty `--context`. `show` prints the body
or context an amend replaced in full under the thread line that says so, and
the web thread shows it under a `previous body` or `previous context` toggle.

`list` shows open and parked cases, the same ones as the web inbox, in the
same order. `--state` (comma-separated or repeated) shows the states you name
instead, and `--all` shows every state; `--state` wins if both are given.

`list --kind KIND`, `list --urgency URGENCY`, `list --label TEXT` and
`list --worker NAME` (each repeatable) show only cases that have any of the
given kinds, urgencies or labels, or come from any of the given workers. A case
must match every filter given. `--kind` and `--urgency` refuse a value that is
not a kind or urgency. `--older-than DURATION` (a Go duration such as `30m`)
shows only cases whose last event is older than that, so a case resumed a
minute ago does not count as waiting since it was opened. `--count` prints just
the number of matching cases, `0` when none match, in place of the table or
the JSON.

`wait` is for an agent to run in the background. It checks the store every
second and returns as soon as a human answers, parks or resumes a case. It then
prints every case still waiting on the agent, one JSON object per line, and
exits 0. A case is waiting on the agent when its last event is one of those
three human events. By default only events that land after `wait` starts can
wake it, so running it again does not wake on answers already reported. With
`--since TIME` it also wakes on human events recorded after that RFC 3339
time. `--since ID` takes a case id instead and uses the time that case was
opened, so an agent can pass the id `open` printed and not miss an answer that
lands before `wait` starts. An id that is not in the store is an error (exit
1).
`--id ID` (repeatable) waits on those cases only.
`--kind KIND`, `--label TEXT` and `--worker NAME` (each repeatable) wait on
cases that have any of the given kinds or labels, or come from any of the given
workers; a case must match every filter given, `--id` included. `--kind` does
not narrow `wait` to your own cases. A filter
that matches no case, like an `--id` that is never answered, waits until the
timeout. If `--timeout` passes first,
it prints one line to stderr and exits 2; other errors exit 1. `wait` also
starts if the store file does not exist yet.

Each line `wait` prints is the case as `show --json` prints it, without
`revision`, plus two fields:

- `fresh` is true when the event that put the case there is new to this
  `wait`: it was written while waiting, or it is later than `--since`. Those
  are the cases that woke it. The rest were already waiting, such as a parked
  case, which is printed on every wake until it is resumed.
- `next_since` is the value to pass as `--since` to the next `wait`, the same
  on every line: the time of the latest event that put a printed case there,
  or the `--since` given if that is later. Waiting again with it does not wake
  on the events just printed, but does wake on one recorded later that lands
  before the new `wait` starts. An event that records a time of its own
  earlier than `next_since`, such as one added by hand, does not wake the next
  `wait` if it is written before that `wait` starts, and is printed with
  `fresh` false on a later wake. It is left out when none of those events
  records a time.

`wait --for human` is the same wait from the human's side, for a notifier to
run: it returns when a case lands on the human and prints every case waiting
on the human. A case is waiting on the human when it is open and its last event
is the agent's: an open, a note, an amend or a resume by the agent. An amend
to a case that was already waiting does not wake it, and nor does a human
event. `--since`, `--timeout` and the filters work as they do for the agent;
on timeout the stderr line says `no case needed the human`. `--for agent` is
the default.

### Clearing the inbox and pruning

`sweep` and `prune` change many cases at once, so both only print what they
would do unless given `--yes` (`-y`).

`sweep` withdraws every open case that matches, with `--reason` recorded on
each withdraw (default `swept`). `--kind`, `--label` and `--worker` filter as on `list`,
and `--older-than DURATION` takes only cases opened longer ago than that (on
`list` it reads the last event instead).
Withdraw is only allowed on an open case, so a matching case that is answered
or parked is listed as left and not changed. Each withdraw is the same `agent`
withdraw event `cases withdraw` writes, and goes through the same check. If one is refused, for example because the case
changed state in the meantime, sweep carries on with the rest, then names the
cases it could not withdraw and exits 1.

`prune` takes closed and withdrawn cases whose last event is older than
`--age` (a Go duration; default `720h`, or the `prune-age` config key; `0`
means any age). `--state closed` or `--state withdrawn` narrows it to one of
them; no other state is accepted. With `--yes` it moves each case to the
store's archive, see [Archive](#archive). If the archive already has a case
with that id, the case is left where it is, the others are still moved, and
prune exits 1. So is a case that has had an event written since prune read it.
`--delete` removes the cases instead, and cannot be undone. A case that cannot
be loaded is reported on stderr and left.

`list` reads every case in the store on each run, so pruning keeps it fast as
the history grows. To prune daily, run `cases prune --yes` from cron or a
launchd job.

## serve

`cases serve` runs a web inbox over the store and opens it in the browser
(`open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows,
`xdg-open` elsewhere). Pass `--no-open`, or set
`no-open = true` in the [config file](#configuration), to skip that.

The default address is `127.0.0.1:8765`; change it with `--listen` or the
`listen` key in the config file. Only loopback addresses (`127.0.0.1`, `::1`,
`localhost`) are accepted.

On a terminal, serve shows a status screen that refreshes every second:

- the inbox URL and the store
- open cases by urgency (blocking in red), parked cases, cases with the agent
  (answered or picked up), and cases closed today and in all
- since start: requests, answers, parks and resumes recorded by a human
  from the inbox or the CLI, and [notifications](#browser-notifications)
  queued; and how long ago the last event was
- the last five request log lines
- how to set up an agent with the bundled skill

Press `q` or Ctrl-C to stop. `NO_COLOR` turns colour off. If stderr is not the
terminal, the request log is also written there.

When stdout is not a terminal (a pipe, a file, a service), serve prints one
line instead, logs each request to stderr and does not open the browser:

```sh
cases serve > serve.log
# Serving /Users/you/.local/share/cases/cases.db at http://127.0.0.1:8765/
```

A page's event stream and the refreshes it sets off, the tab icons the page
and its refreshes point at, and a tab's checks for notifications, are not
logged or counted.

### Finding a running serve

While it runs, serve records itself in a JSON file (`pid`, `url`, `addr`,
`store`, `started_at`, `version`) in this machine's state directory:
`$XDG_STATE_HOME/cases`, or `~/.local/state/cases`. The file is named
`serve-<slug>-<hash>.json` from the store's absolute path, so serves on
different stores do not clash, and it is never put beside the store. Serve removes the file when it stops on `q`, Ctrl-C or SIGTERM.
`cases status` reads it for the current store:

```sh
cases status
# Serving /Users/you/.local/share/cases/cases.db at http://127.0.0.1:8765/ (pid 4242, since 2026-09-16T09:12:03Z)
cases status --json   # the file's fields
```

It exits 1 and prints `not running` when there is no live serve. A file left
by a crash or `kill -9` counts as not running when its process is gone or
nothing accepts connections on its address, and the next serve replaces it.
Serve refuses to start while a live serve holds the file for the same store,
and names that serve's URL and pid. If the file cannot be read or parsed,
`cases status` prints an `Error:` line naming it and exits 1.

`cases show` uses the same file to print a `url` line, the case's page in the
running inbox, and `show --json` has it as `url`. With no serve running there
is no `url` line and the field is empty. A file that cannot be read leaves it
empty too, with a warning on stderr.

`cases inbox` opens the running inbox in the browser, or with an id, that
case's page in it:

```sh
cases inbox          # opens the inbox
cases inbox ID       # opens the case's page
cases inbox --print  # prints the URL instead of opening it
```

It exits 1 with a message naming `cases serve` or `cases service install`
when nothing is running; it never starts a server. `--print` is for scripts
and for agents: an agent should not open a browser on the human's machine
without being asked, but may print the link (`cases inbox --print ID`) to
give the human one, instead of building it by hand from `cases status`.

The check has three limits. It cannot tell a hung serve from a healthy one. A
store reached by two different paths (a symlink) gets two files. Two serves
started at the same moment on one store can both pass the check.

### Running serve as a service

`cases service install` sets serve up as a user service that starts at login
and is started again whenever it stops.

- On macOS it writes a launchd agent,
  `~/Library/LaunchAgents/com.github.ryanlewis.cases.serve.plist`
  (`RunAtLoad`, `KeepAlive`), and loads it with
  `launchctl bootstrap gui/$UID`. Its output goes to
  `$XDG_STATE_HOME/cases/serve.log`, or `~/.local/state/cases/serve.log`.
- On Linux it writes a systemd user unit,
  `$XDG_CONFIG_HOME/systemd/user/cases-serve.service` (or
  `~/.config/systemd/user`), with `Restart=always`, then runs
  `systemctl --user daemon-reload`, `enable` and `restart`. Its output goes to
  the journal: `journalctl --user -u cases-serve`. A user unit stops when you
  log out unless lingering is on (`loginctl enable-linger`).

The service runs this binary, found by its absolute path, as
`cases --store STORE --config CONFIG serve --no-open --listen ADDR [--as NAME]`,
with what install resolved: the store's absolute path, the config file,
`--listen` and `--as`. The last two default from the `listen` and `name` keys
in the [config file](#configuration), as they do for serve. It sets `HOME` and
`XDG_STATE_HOME`, so the service records itself where `cases status` looks.
Run install again after moving the binary or changing any of those; it
replaces the file and restarts the service. There is one service per user, so
installing for another store replaces it.

A crashed serve leaves a stale instance file, which the restarted one
replaces, as above. While the service holds the address, a `cases serve` you
start by hand fails. The other way round, an install refuses while a
serve is running for the store or something holds the address, and names what
to stop. A reinstall skips the part of that check the loaded service already
covers: a serve on the same store, or the same address, is the service itself.

`cases service` on its own (also `cases service status`) reports on the
service and changes nothing:

```
Installed: /Users/you/Library/LaunchAgents/com.github.ryanlewis.cases.serve.plist
Runs:      /Users/you/go/bin/cases --store /Users/you/.local/share/cases/cases.db --config /Users/you/.config/cases/config.toml serve --no-open --listen 127.0.0.1:8765
Store:     /Users/you/.local/share/cases/cases.db
Listen:    127.0.0.1:8765
Loaded:    yes (launchctl print gui/501/com.github.ryanlewis.cases.serve)
Inbox:     http://127.0.0.1:8765/ (pid 4242)
```

It reads the command back from the file, asks launchd (`launchctl print`) or
systemd (`systemctl --user is-active`) whether they have the service, and
checks the service's store for a running inbox as `cases status` does. A
`Problem:` line names anything that disagrees: installed but not loaded,
loaded but not answering, loaded with its file gone, a file for another store
or binary than this command's, or a serve started by hand with no service
installed. It exits 0 when the service is installed, loaded and answering, and
1 otherwise. `--json` prints `installed`, `path`, `args`, `store`, `listen`,
`loaded`, `running`, `url`, `pid` and `problems`.

`cases service uninstall` stops the service and removes its file. It fails
when there is none. Other systems are not supported; run `cases serve` under
your own supervisor there.

### The inbox

The inbox is laid out like a mail client. The left column lists open and
parked cases, blocking first, then oldest first. The right side shows the
selected case, and its card is highlighted in the list. The two columns scroll
separately. On a window narrower than 56rem there is one column: `/` shows the
list and `/cases/ID` shows the case, with Inbox in the header to go back.

Where the browser supports cross-document view transitions, moving between
pages cross-fades, with the header held still, and the list column too on a
wide window. With reduced motion set in the system, or in other browsers,
pages change at once.

The header counts open cases by urgency and parked cases, as in
`3 blocking · 5 today · 12 whenever · 1 parked`. A count of zero is left out,
and only the blocking count is red. The page title starts with the number of
open cases, of any urgency, as in `(4) cases`, so a browser tab shows
what is waiting; parked cases are not counted, and with none open there is no
number; the title is otherwise `cases` on every page. The tab icon shows the
same count: a plain `c` tile with none open, the count on a dark tile when cases
are open (`9+` above nine), and on a red tile when any of them is blocking. A
parked blocking case does not turn it red. The server draws the icon as SVG at
`/favicon.svg`, so the page's content security policy allows no `data:` images;
browsers that do not show SVG tab icons still have the title. The counts, the
title and the icon follow the store while the page is open, as described below,
except in Safari, which loads a page's icon once, when the page loads, so its
tab icon shows the count as of the last page load.

- `/` selects the first case in the inbox. With no open or parked cases it
  shows one inbox zero panel instead of the columns: the cases you answered,
  parked or resumed today and this week, the cases closed today, the answered
  ones still with an agent, and when you last answered. It reloads when a case
  arrives.
- `/cases/ID` selects that case: its context and brief, the body rendered as
  markdown (GFM tables included), its links, a response form that fits the
  kind, and the thread.
  An open or parked case sits beside the inbox list. An answered, picked-up,
  closed or withdrawn case sits beside the done list instead, with `done`
  current in the header, the case selected, and the list filtered to `in
  flight` for an answered or picked-up case and to `all` otherwise.
  An amended case is shown as amended, and the thread lists what each amend
  changed.
  Times on the case and in the thread are in the local time zone of the
  machine running serve, with how long ago each was; the store keeps UTC.
  Sending the form writes one answer. A stuck case has a separate `park`
  button, which parks it instead, with the note if one is written. Every other
  kind has a `drop` button, which answers with `drop` and the note, and a stuck
  case drops through its `drop it` choice. A parked
  case has a `resume` button, and shows in the list with an ochre edge.
- `/done` lists answered, picked-up, closed and withdrawn cases, newest first,
  with the outcome of closed ones. Chips at the top filter it and show each
  count: `in flight` (`?show=inflight`: answered or picked up, with who has
  each case and how long since the answer or pickup, newest first), `closed
  today` (`?show=closed-today`: closed since midnight in the local time zone)
  and `all` (the default). The inbox zero panel's figures link to the first
  two.

The `options` button in the header opens an overlay that sets the theme
(system, light or dark), the face for case bodies, notes and outcomes (mono,
sans or serif), the text size of the whole page (small, medium or large), and
whether links to other sites open in a new tab, and
[desktop notifications](#browser-notifications). The choices are kept in this
browser's local storage, not on the server, and apply at once;
`reset` goes back to the defaults. Close it with `close`, Escape, or a click
outside it.

After a successful answer, park or resume, the page moves to the case after it
in the inbox, or to `/` if it was the last one. A parked case stays in the
inbox. The redirect carries `event` (`answer`, `park` or `resume`) and
`recorded` (the case id), and the page it lands on starts with one line saying
what was recorded on which case, linked to it, with a `dismiss` link that
reloads the page without those two parameters. The line is shown only when the
event is one of those three and the id names a case in the store, and it stays
in the URL, so a reload shows it again. If the form is refused, the same case
is shown again with the error.

Each form carries the case's revision from when the case was drawn: the
number of events the case had. While the page is open, the case is drawn again
as events are added, and the form's revision with it, unless an amend has
changed the question (see below). If the form's revision is not the case's,
because an event landed just as the form was sent, the page had lost touch
with serve, or the question changed, the form is refused and nothing is
written. The case is shown again as it is now, so the
thread can be read before sending again; if the case is still open, the form
keeps what was typed. This stops a tab from answering a case that was answered
somewhere else and then reopened by a note before the tab has shown it, or a
case whose question has changed since the form was drawn. `cases answer`, `cases resume` and the agent's write commands do the same
when given `--revision N`.

The header counts sit beside the `inbox` and `done` links. A page that shows
the inbox or a case follows the store. Serve reads the store every second, and
the page holds an event stream (`/events`) on which serve says when the store
has changed, at most once a second. The list, the count and the case then
refresh; beside a done case the done list does not, and `/done` does not follow
the store at all. The list and the case also check once a minute, which keeps
the ages on the list right. The page reloads itself if the case changes state
while it is open; on `/` it reloads `/`, which selects whichever case is first.
A change that leaves the state as it was, such as a note or an amend on an open
case, draws the case again in place: its header, the form and the thread. The
form keeps what has been typed and chosen in it, and a line saying a form was
refused stays. The form takes the case's new revision, so a note that has come
in on the page does not stop it being sent, except after an amend that changes
the question (anything but labels): then the form keeps the revision it had,
so its next send is refused and the case shown again to be checked. If serve
restarts, the page reconnects by itself and catches up.

Only a tab that is shown holds its stream open: browsers allow six connections
to one address over plain HTTP, and six streams would leave none for the pages
themselves. It passes what it hears to the browser's other inbox tabs, so a
tab in the background follows the store too, and a tab shown again catches up
at once.

The app only answers requests addressed to its own host and port, refuses form
posts from other sites (checked with `Sec-Fetch-Site` and `Origin`), and sends
`Content-Security-Policy: default-src 'self'`. Raw HTML in a markdown body is
dropped. Links to other sites open in a new tab, with
`rel="noopener noreferrer"`. A case link or row link that does not start with
`http://` or `https://`, such as a file path, is shown as text, not a link. htmx is included in the binary; nothing is fetched
from the network.

#### Keys

| Key | Does |
|---|---|
| Cmd+Enter or Ctrl+Enter | sends the open case's answer from anywhere on its page, text fields included, except a focused link, which opens in a new tab |
| `1` to `9` | chooses that option on a decision, sign-off or stuck case: the options in the order shown, so on a decision the number after the last option is `other, see note` |
| `j`, `k` | opens the next or previous case in the inbox list |
| `?` | lists these keys |

A key sends the form as the send button does: the browser checks the required
fields first, and a form older than the case is refused in the same way. It
sends once; pressing it again while the post is on its way does nothing, for
up to 10 seconds. The other keys do nothing while the cursor is in a text
field, while a dialog is open, or with Cmd, Ctrl or Alt held. An approval has a
choice per row, even with one row, so the number keys leave it alone; the arrow
keys move between a row's verdicts.

### Browser notifications

An open inbox tab can show a desktop notification when a case lands on you.
Turn it on in `options` under desktop notifications: `blocking cases` or
`every case`. The browser asks for permission the first time; `allow
notifications` asks again while it has not been answered. If it is refused,
the choice goes back to `off`, and the overlay says to allow it in the
browser's site settings. `http://127.0.0.1` and `localhost` count as secure
origins, so no certificate is needed.

A notification fires when a case becomes open because the agent:

- opened it,
- followed up an answered or picked-up case with a note, which reopens it, or
- resumed it after it was parked, or
- wrote its first note after you resumed a parked case yourself.

It does not fire for any other note or amend on a case that is already open,
for your own resume, for a case that was answered or withdrawn before
serve saw it, or for cases already in the store when serve started. Clicking
it opens the case in that tab.

Serve reads the store every second and keeps the last 50 notifications in
memory, numbered from 1. Each page with notifications turned on asks
`/notifications?after=N` every five seconds and shows the new ones. The
number shown last is kept in the browser with serve's boot id. A tab with no
kept number starts at the latest and shows nothing. A number from before serve
restarted starts again from the first notification of the new run, so a case
that lands just after a restart is still shown, and a tab opened later shows
what came in since the restart, up to 50. Every tab shows the same
notification under one tag, so the desktop shows it once.

The limits: nothing is shown when no inbox tab is open; browsers slow timers
in background tabs, so a notification can come up to about a minute late; and
a case that arrives while serve is stopped is not notified when it starts.

## skill

The agent skill is a `SKILL.md` embedded in the binary, so it always matches
the commands the binary has. It is written for an agent: the lifecycle, the
kinds and what each answer looks like, the agent-side commands, what not to do,
and a recipe for open, wait, pickup and close.

```sh
cases skill install claude   # write SKILL.md into the agent's skills directory
cases skill uninstall claude # remove it (and the directory, if empty)
cases skill show             # print the skill
cases skill list             # each agent, where the skill goes, and whether it is installed
cases skill check            # exit 1 if an installed skill differs from this binary
```

| Agent | Directory | Relocated by |
| --- | --- | --- |
| `claude` | `~/.claude/skills/cases` | `$CLAUDE_CONFIG_DIR` |
| `codex` | `~/.codex/skills/cases` | `$CODEX_HOME` |
| `pi` | `~/.pi/agent/skills/cases` | `$PI_CODING_AGENT_DIR` (a leading `~` is expanded) |

`skill list` compares the files in each agent's directory byte for byte with
what this binary renders and reports one of three states: `installed` (they
match), `stale` (a file is there but differs or is missing, as after
upgrading cases without reinstalling the skill) or `not installed`. A file
that is there but cannot be read shows as `unreadable`, with the error.

`--path DIR` installs to or uninstalls from another directory. When the
installed skill already matches, `install` says it is already up to date and
writes nothing. It overwrites a stale skill only after asking, or with `-y`;
`uninstall` lists the files and asks, or needs `-y`. When stdin is not a
terminal neither command asks: without `-y` they refuse. `install` also
refuses without `-y` when a file of the installed skill cannot be read.

`skill check [agent]` looks at each agent's default directory, or only the
named agent's, prints a line for each stale skill and exits 1 if there is
one, or if a skill cannot be read. A skill that is not installed does not
count. It is for scripts and Makefiles; it reports drift but does not fix
it.

## Development

```sh
make build   # ./cases
make test    # go test -race ./...
make lint    # golangci-lint run ./...
make fmt     # golangci-lint fmt ./...
```

The store uses `modernc.org/sqlite`, SQLite translated to Go, so a build needs
no C compiler and cross-compiles like any Go program. Its go.mod pins the
`modernc.org/libc` it was built against, and this module must pin the same
version, so update the two together.

`govulncheck` is pinned as a `tool` directive in go.mod, so its dependency
graph (`golang.org/x/vuln` and its own dependencies) shows up in go.mod and
go.sum alongside the runtime dependencies.

To cut a release, push a semver tag: `git tag vX.Y.Z && git push origin
vX.Y.Z`. `.github/workflows/release.yml` runs the tests, then goreleaser
(`.goreleaser.yaml`) builds binaries for linux, darwin and windows, signs and
notarizes the darwin ones, publishes them with a `checksums.txt` file as a
GitHub release, attests their build provenance and updates the cask in
[ryanlewis/homebrew-tap](https://github.com/ryanlewis/homebrew-tap). A
prerelease tag such as `vX.Y.Z-rc.1` is published as a GitHub prerelease and
leaves the cask alone. The release job takes the signing, notarization and
tap secrets from the `release` environment and stops before building if any
is missing.

Set up the `release` environment before pushing the first tag: a run that
names a missing environment creates it with no protection rules. Give it a
deployment rule that allows only `v*` tags, and these secrets:

- `MACOS_SIGN_P12`: the Developer ID Application certificate and its private
  key as a `.p12` file, base64-encoded
- `MACOS_SIGN_PASSWORD`: the password of the `.p12`
- `MACOS_NOTARY_ISSUER_ID` and `MACOS_NOTARY_KEY_ID`: the issuer ID and key
  ID of an App Store Connect API key (the Developer role is enough)
- `MACOS_NOTARY_KEY`: that key's `.p8` file, base64-encoded; its PEM text
  pasted as is fails at notarization
- `HOMEBREW_TAP_GITHUB_TOKEN`: a fine-grained token with read and write
  access to the contents of ryanlewis/homebrew-tap only

```sh
base64 -i cert.p12 | gh secret set MACOS_SIGN_P12 --env release -R ryanlewis/cases
base64 -i AuthKey_<key-id>.p8 | gh secret set MACOS_NOTARY_KEY --env release -R ryanlewis/cases
```

`goreleaser check` validates the config. `goreleaser release --snapshot
--clean` builds everything into `dist/` without signing or publishing.

## License

MIT
