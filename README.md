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

Each case is a directory of JSON files, one file per event, so the store can
live in a synced folder. No server is needed.

## Install

The repository is private, so install from a checkout with Go on the path:

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
`~/.local/share/cases`. In real use an agent runs the agent's commands; to try
it, run both sides in one shell.

```sh
# Agent: raise a decision and wait for the answer in the background.
# --since stops wait missing an answer that lands before it starts.
since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
echo "Pin bun to 1.2.3, or float it and fix the lockfile when it breaks?" > question.md
id=$(cases open --kind decision --urgency blocking --title "Pin bun or float?" \
  --body-file question.md --option "Pin to 1.2.3" --option "Float and fix the lockfile")
cases wait --id "$id" --since "$since" --timeout 2h > answered.jsonl &

# Human: answer it, from here or from the inbox that cases serve opens.
cases answer "$id" --option 1 --note "Revisit after 1.3"

# Agent: record that the answer was read, act on it, and close with the outcome.
cases pickup "$id" --by bun-pins
echo "Pinned in #12." | cases close "$id" --outcome-file -
cases show "$id"
```

## Store format

The store is a directory. By default it is `$XDG_DATA_HOME/cases`, which is
usually `~/.local/share/cases`. Name another with `--store`, `CASES_STORE` or
the [config file](#configuration). Each case is a directory named after the
time it was opened (UTC) and a slug of its title. That name is the case id.
Each write adds a new file:

```
cases/
  2026-09-15T09-12-03Z-pin-bun-or-float/
    0001-agent-open.json       # kind, urgency, title, body (markdown), options[], rows[], links[], labels[], worker, brief, context
    0002-agent-amend.json      # options[], rows[], links[], labels[] to add; body, context to replace; amended_at
    0003-human-answer.json     # choice / rows / signoff / text / drop / ack, note, answered_at
    0004-agent-pickup.json     # picked_up_at, by
    0005-agent-note.json       # follow-up question, reopens the case
    0006-human-answer.json
    0007-agent-pickup.json     # a reopened case is picked up again before it closes
    0008-agent-close.json      # outcome (markdown), links, closed_at
```

File names are `NNNN-<author>-<event>.json`. The author is `agent` or `human`.
The events are `open`, `amend`, `answer`, `pickup`, `note`, `close`,
`withdraw`, `park` and `resume`. Timestamps are RFC 3339 in UTC.

The `labels`, `worker`, `brief` and `context` fields on `open` are optional
and help the human act on a case. `labels` group cases, such as the ones one
piece of work opened; a label may not be blank or appear twice on a case.
`worker` names the agent session waiting on it. `brief` says where to restart
the work from if the case is parked: a brief, a ledger or a note, as a path or
a short line. `context` is free text shown with the case.

An `amend` changes a case that is still open. Its `options`, `rows`,
`links` and `labels` are added after the ones the case has, and its `body` or `context`
replaces the case's. A field it leaves out stays as it was. Nothing can be
removed, and options keep their numbers.

An amend is refused if it sets a blank `body`, `context`, option, link or
label, adds an option, link, label or row `id` the case already has, or changes nothing, such as
setting the `body` the case already has. So the same amend sent twice writes
nothing the second time.

The `open` file stays as it was written. The case shows the amended fields,
and the answer is checked against them: an approval answer needs a verdict on
the added rows too. An answer with the same sequence number as an amend was
written without seeing it, for example on a machine the amend had not synced
to yet. Unless the amend only added labels, the answer is refused and the case
stays open for another answer. An amend that sets a body or context counts
even when it is the one the case already has.

A `cases` from before `amend` skips amend files as unknown events: it shows an
amended case as it was opened and checks answers against that. A `cases` from
before the `question` kind refuses a question case's open event as an unknown
kind, so it reports the whole case as broken rather than skipping one file.
Update `cases` on every machine that uses the store before an agent amends a
case or opens a question.

A case's state is worked out by reading its files in name order. It is never
stored. Files are never edited or deleted, and closed cases are kept as the
decision log until `cases prune` moves them to the [archive](#archive).
Writes go to a temporary file (its name starts with a dot), which is then
hard-linked to the event's file name and removed, so a reader never sees half
a file. If a file already has that name, such as one a sync
client added during the write, the write fails and that file is kept. Where
hard links do not work (FAT, exFAT, some network mounts, or a sandbox that
blocks them), the temporary file is renamed into place instead, once a check
shows the name is still free. A file that arrives between that check and the
rename is replaced.

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
| `approval` | `rows`, each with `id`, `label`, `script` (the text itself), `link` and an optional `note` shown under the label | a verdict per row: `approve`, `hold` or `reject`, each with an optional note |
| `signoff` | | `accept`, or `changes` with a note |
| `stuck` | | guidance `text`, or `drop`; parking is a separate `park` event |
| `question` | | reply `text` |
| `fyi` | | `ack` |

Every answer may carry a `note`.

### Archive

`cases prune` moves whole case directories into `.archive` inside the store:

```
cases/
  .archive/
    2026-08-01T10-00-00Z-old-question/
  2026-09-15T09-12-03Z-pin-bun-or-float/
```

`list`, `wait` and `serve` skip any directory whose name starts with a dot,
and `show` looks for a case only at the top of the store, so an archived case
is out of sight. Its files are as they were. To restore one, move its
directory back:

```sh
mv ~/.local/share/cases/.archive/2026-08-01T10-00-00Z-old-question ~/.local/share/cases/
```

### Damaged files

A file that is not valid JSON, has an unexpected name, or records an event the
case could not accept at that point is skipped. The case still loads, and the
file is reported as a problem (`cases show` lists it; `list` and `wait` print
it as a warning on stderr). A case directory whose open event is damaged is
reported and the other cases are still listed. Fields the CLI does not know are
kept: `cases show --json` prints every event file as written.

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
| `store` | `--store` | `CASES_STORE` | `$XDG_DATA_HOME/cases`, or `~/.local/share/cases` |
| `listen` | `--listen` on `serve` | nothing | `127.0.0.1:8765` |
| `no-open` | `--no-open` on `serve` | nothing | `false` |
| `prune-age` | `--age` on `prune` | nothing | `720h` |

```toml
store = "~/Sync/cases"
```

A leading `~` in `store` is expanded. `CASES_STORE` set to an empty string
counts as unset.

An unknown key, a value that is not a string, or malformed TOML is an error
that names the file. It stops every command except `cases config path`,
`cases config show` and `cases config init`, which are how you find out which
file is at fault.

## CLI

Every command reads and writes the store directly.

Agent side:

```
cases open     --kind KIND --urgency blocking|today|whenever --title TEXT
               [--body-file FILE|-] [--option TEXT]... [--row JSON]...
               [--link URL]... [--label TEXT]... [--worker NAME]
               [--brief TEXT] [--context TEXT]
cases amend    ID [--body-file FILE|-] [--option TEXT]... [--row JSON]...
               [--link URL]... [--label TEXT]... [--context TEXT]
cases wait     [--since TIME] [--timeout DURATION] [--id ID]...
               [--label TEXT]... [--worker NAME]...
cases pickup   ID [--by NAME]
cases note     ID --body-file FILE|-
cases close    ID --outcome-file FILE|- [--link URL]...
cases withdraw ID [--reason TEXT]
```

Human side:

```
cases answer ID --option N | --other | --row ID=VERDICT[:NOTE]... |
                --accept | --changes | --text TEXT | --park | --drop | --ack
                [--note TEXT] [--revision N]
cases resume ID [--agent] [--revision N]
cases sweep  [--reason TEXT] [--older-than DURATION] [--label TEXT]...
             [--worker NAME]... [--yes]
cases prune  [--age DURATION] [--state closed,withdrawn] [--delete] [--yes]
```

Both:

```
cases list   [--state STATE,...] [--label TEXT]... [--worker NAME]... [--json]
cases show   ID [--json]
cases serve  [--listen 127.0.0.1:8765] [--no-open]
cases status [--json]
```

Setup:

```
cases skill install   AGENT [--path DIR] [-y]
cases skill uninstall AGENT [--path DIR] [-y]
cases skill show      [AGENT]
cases skill list
```

`open` prints the new case id. The other write commands print the id and the
new state.

`cases show` prints the case's revision, the number of event files it has, and
`show --json` has it as `revision`. Pass it to `answer` or `resume` as
`--revision N` and the write is refused, with nothing written, if an event has
been added to the case since it was read. The error names the revision read and
the current one, and the command exits non-zero.

`--row` on `open` and `amend` takes one JSON object per row, for example
`--row '{"id":"deps","label":"Install deps","script":"npm ci","link":"https://…"}'`.
Add `"note":"…"` to show a line under the row's label.

`--label`, `--worker`, `--brief` and `--context` on `open` set the fields described in
[store format](#store-format). `--by` on `pickup` records who picked the case
up, such as the agent session name. `--reason` on `withdraw` records why the
case no longer needs an answer; `show` and the web thread print it.

`amend` changes an open case as described in [store format](#store-format):
`--option`, `--row`, `--link` and `--label` add to the case, and `--body-file` and
`--context` replace its body and context. It refuses an amend that changes
nothing, an empty body file and an empty `--context`. `show` prints the body
or context an amend replaced in full under the thread line that says so, and
the web thread shows it under a `previous body` or `previous context` toggle.

`list --label TEXT` and `list --worker NAME` (each repeatable) show only cases
that have any of the given labels, or come from any of the given workers.
Given together, a case must match both.

`wait` is for an agent to run in the background. It checks the store every
second and returns as soon as a human answers, parks or resumes a case. It then
prints every case still waiting on the agent, one JSON object per line, and
exits 0. A case is waiting on the agent when its last event is one of those
three human events. By default only events that land after `wait` starts can
wake it, so running it again does not wake on answers already reported. With
`--since TIME` it also wakes on human events recorded after that time.
`--id ID` (repeatable) waits on those cases only.
`--label TEXT` and `--worker NAME` (each repeatable) wait on cases that have
any of the given labels, or come from any of the given workers; given
together, a case must match both, and with `--id` as well, all three. A filter
that matches no case, like an `--id` that is never answered, waits until the
timeout. If `--timeout` passes first,
it prints one line to stderr and exits 2; other errors exit 1. `wait` also
starts if the store directory does not exist yet.

### Clearing the inbox and pruning

`sweep` and `prune` change many cases at once, so both only print what they
would do unless given `--yes` (`-y`).

`sweep` withdraws every open case that matches, with `--reason` recorded on
each withdraw (default `swept`). `--label` and `--worker` filter as on `list`,
and `--older-than DURATION` takes only cases opened longer ago than that.
Withdraw is only allowed on an open case, so a matching case that is answered
or parked is listed as left and not changed. Each withdraw is the same `agent`
withdraw event `cases withdraw` writes, and goes through the same check. If one is refused, for example because the case
changed state in the meantime, sweep carries on with the rest, then names the
cases it could not withdraw and exits 1.

`prune` takes closed and withdrawn cases whose last event is older than
`--age` (a Go duration; default `720h`, or the `prune-age` config key; `0`
means any age). `--state closed` or `--state withdrawn` narrows it to one of
them; no other state is accepted. With `--yes` it moves each case directory to
`.archive/<id>` in the store, see [Archive](#archive). If a directory of that
name is already in the archive the case is left where it is, the others are
still moved, and prune exits 1. `--delete` removes the case directories
instead, and cannot be undone. A case directory that cannot be loaded is
reported on stderr and left.

`list` reads every case in the store on each run, so pruning keeps it fast as
the history grows. To prune daily, run `cases prune --yes` from cron or a
launchd job.

## serve

`cases serve` runs a web inbox over the store and opens it in the browser
(`open` on macOS, `xdg-open` elsewhere). Pass `--no-open`, or set
`no-open = "true"` in the [config file](#configuration), to skip that.

The default address is `127.0.0.1:8765`; change it with `--listen` or the
`listen` key in the config file. Only loopback addresses (`127.0.0.1`, `::1`,
`localhost`) are accepted.

On a terminal, serve shows a status screen that refreshes every second:

- the inbox URL and the store
- open cases by urgency (blocking in red), parked cases, cases with the agent
  (answered or picked up), and cases closed today and in all
- since start: requests, and answers, parks and resumes recorded by a human
  from the inbox or the CLI; and how long ago the last event was
- the last five request log lines
- how to set up an agent with the bundled skill

Press `q` or Ctrl-C to stop. `NO_COLOR` turns colour off. If stderr is not the
terminal, the request log is also written there.

When stdout is not a terminal (a pipe, a file, a service), serve prints one
line instead, logs each request to stderr and does not open the browser:

```sh
cases serve > serve.log
# Serving /Users/you/.local/share/cases at http://127.0.0.1:8765/
```

Page refreshes that run every two seconds are not logged or counted.

### Finding a running serve

While it runs, serve records itself in a JSON file (`pid`, `url`, `addr`,
`store`, `started_at`, `version`) in this machine's state directory:
`$XDG_STATE_HOME/cases`, or `~/.local/state/cases`. The file is named
`serve-<slug>-<hash>.json` from the store's absolute path, so serves on
different stores do not clash, and it is never put in the store, which may be
synced. Serve removes the file when it stops on `q`, Ctrl-C or SIGTERM.
`cases status` reads it for the current store:

```sh
cases status
# Serving /Users/you/.local/share/cases at http://127.0.0.1:8765/ (pid 4242, since 2026-09-16T09:12:03Z)
cases status --json   # the file's fields
```

It exits 1 and prints `not running` when there is no live serve. A file left
by a crash or `kill -9` counts as not running when its process is gone or
nothing accepts connections on its address, and the next serve replaces it.
Serve refuses to start while a live serve holds the file for the same store,
and names that serve's URL and pid. If the file cannot be read or parsed,
`cases status` prints an `Error:` line naming it and exits 1.

The check has three limits. It cannot tell a hung serve from a healthy one. A
store reached by two different paths (a symlink) gets two files. Two serves
started at the same moment on one store can both pass the check.

### The inbox

The inbox is laid out like a mail client. The left column lists open and
parked cases, blocking first, then oldest first. The right side shows the
selected case, and its card is highlighted in the list. The two columns scroll
separately. On a window narrower than 56rem there is one column: `/` shows the
list and `/cases/ID` shows the case, with Inbox in the header to go back.

- `/` selects the first case in the inbox, or says there are no open cases and
  reloads when one arrives.
- `/cases/ID` selects that case: the body rendered as markdown (GFM tables
  included), its links, a response form that fits the kind, and the thread.
  An amended case is shown as amended, and the thread lists what each amend
  changed.
  Sending the form writes one answer. A stuck case has a separate `park`
  button, which parks it instead, with the note if one is written. A parked
  case has a `resume` button, and shows in the list with an ochre edge.
- `/done` lists answered, picked-up, closed and withdrawn cases, newest first,
  with the outcome of closed ones.

The `options` button in the header opens an overlay that sets the theme
(system, light or dark), the face for case bodies, notes and outcomes (mono,
sans or serif), and whether links to other sites open in a new tab. The choices
are kept in this browser's local storage, not on the server, and apply at once;
`reset` goes back to the defaults. Close it with `close`, Escape, or a click
outside it.

After a successful answer, park or resume, the page moves to the case after it
in the inbox, or to `/` if it was the last one. A parked case stays in the
inbox. If the form is refused, the same case is shown again with the error.

Each form carries the case's revision from when the page was loaded: the
number of event files the case had. If an event has been added to the case
since, including one a sync brought in from another machine, the form is
refused and nothing is written. The case is shown again as it is now, so the
thread can be read before sending again; if the case is still open, the form
keeps what was typed. This stops a tab left open from answering a case that was
answered somewhere else and then reopened by a note, or a case that has been
amended since the page was loaded. `cases answer` and `cases resume` do the same when given
`--revision N`.

The tab title is the selected case's title, with the number of open blocking
cases in front; the header shows the same count beside the `inbox` and `done`
links. The list, the count and the thread refresh every two seconds. The page
reloads itself if the case changes state while it is open; on `/` it reloads
`/`, which selects whichever case is first. A change that leaves the state as
it was, such as a note or an amend on an open case, only updates the thread, so
what has been typed is kept; sending the form is then refused as described
above.

The app only answers requests addressed to its own host and port, refuses form
posts from other sites (checked with `Sec-Fetch-Site` and `Origin`), and sends
`Content-Security-Policy: default-src 'self'`. Raw HTML in a markdown body is
dropped. Links to other sites open in a new tab, with
`rel="noopener noreferrer"`. A case link or row link that does not start with
`http://` or `https://`, such as a file path, is shown as text, not a link. htmx is included in the binary; nothing is fetched
from the network.

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
```

| Agent | Directory | Relocated by |
| --- | --- | --- |
| `claude` | `~/.claude/skills/cases` | `$CLAUDE_CONFIG_DIR` |
| `codex` | `~/.codex/skills/cases` | `$CODEX_HOME` |
| `pi` | `~/.pi/agent/skills/cases` | `$PI_CODING_AGENT_DIR` (a leading `~` is expanded) |

`--path DIR` installs to or uninstalls from another directory. `install`
overwrites an installed skill only after asking, or with `-y`; `uninstall`
lists the files and asks, or needs `-y`. When stdin is not a terminal
neither command asks: without `-y` they refuse.

## Development

```sh
make build   # ./cases
make test    # go test -race ./...
make lint    # golangci-lint run ./...
```

## License

MIT
