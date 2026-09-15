# cases

(Formerly agent-inbox.)

`cases` is a small Go CLI for passing questions between an agent and a
human. An agent opens a case: a decision to make, scripts to approve, work to
sign off, a blocker, or something to know about. The human answers it. The
agent picks the answer up and closes the case with what happened. Each case is
a directory of JSON files, one file per event, so the store can sit in a synced
folder such as an Obsidian vault. No server is needed.

## Store format

The store is a directory you choose with `--store` or `CASES_STORE`. There is
no default. Each case is a directory named after the time it was opened (UTC)
and a slug of its title. Each write adds a new file:

```
cases/
  2026-09-15T09-12-03Z-kristi-chair/
    0001-agent-open.json       # kind, urgency, title, body (markdown), options[], rows[], links[], worker, brief, context
    0002-human-answer.json     # choice / rows / signoff / text / drop / ack, note, answered_at
    0003-agent-pickup.json     # picked_up_at, by
    0004-agent-note.json       # follow-up question, reopens the case
    0005-human-answer.json
    0006-agent-close.json      # outcome (markdown), links, closed_at
```

File names are `NNNN-<author>-<event>.json`. The author is `agent` or `human`.
The events are `open`, `answer`, `pickup`, `note`, `close`, `withdraw`, `park`
and `resume`. Timestamps are RFC 3339 in UTC.

A case's state is worked out by reading its files in name order. It is never
stored. Files are never edited or deleted, and closed cases are kept as the
decision log. Writes go to a temporary file (its name starts with a dot) and
are renamed into place, so a reader never sees half a file.

### States

```
open --answer--> answered --pickup--> pickedup --close--> closed
open --withdraw--> withdrawn
answered or pickedup --note--> open        (a follow-up question)
open --park--> parked --resume--> open     (stuck cases only)
```

A note on an open case adds to the thread and leaves it open. Anything else,
such as closing a case that has not been picked up, is refused and nothing is
written.

### Kinds and answers

| Kind | Open with | Answer with |
| --- | --- | --- |
| `decision` | one or more `options` | `choice` (1-based option number), or `other` with a note |
| `approval` | `rows`, each with `id`, `label`, `script` (the text itself) and `link` | a verdict per row: `approve`, `hold` or `reject`, each with an optional note |
| `signoff` | | `accept`, or `changes` with a note |
| `stuck` | | guidance `text`, or `drop`; parking is a separate `park` event |
| `fyi` | | `ack` |

Every answer may carry a `note`.

### Damaged files

A file that is not valid JSON, has an unexpected name, or records an event the
case could not accept at that point is skipped. The case still loads, and the
file is reported as a problem (`cases show` lists it; `list` and `wait` print
it as a warning on stderr). A case directory with no valid open event is
reported and the other cases are still listed. Fields the CLI does not know are
kept: `cases show --json` prints every event file as written.

## Install

```sh
make install    # go install ./cmd/cases
```

## CLI

Every command reads and writes the store directly.

Agent side:

```
cases open     --kind KIND --urgency blocking|today|whenever --title TEXT
               [--body-file FILE|-] [--option TEXT]... [--row JSON]...
               [--link URL]... [--worker NAME] [--brief PATH] [--context TEXT]
cases wait     [--since TIME] [--timeout DURATION]
cases pickup   ID [--by NAME]
cases note     ID --body-file FILE|-
cases close    ID --outcome-file FILE|- [--link URL]...
cases withdraw ID
```

Human side:

```
cases answer ID --option N | --other | --row ID=VERDICT[:NOTE]... |
                --accept | --changes | --text TEXT | --park | --drop | --ack
                [--note TEXT]
cases resume ID [--agent]
```

Both:

```
cases list [--state STATE,...] [--json]
cases show ID [--json]
```

`open` prints the new case id. The other write commands print the id and the
new state.

`--row` on `open` takes one JSON object per row, for example
`--row '{"id":"deps","label":"Install deps","script":"npm ci","link":"https://…"}'`.

`wait` checks the store every second. When it finds answered cases it prints
each one as a line of JSON and exits 0. Without `--since` it reports every case
that is currently answered, so a case still waiting to be picked up is reported
again. With `--since` it reports only answers written after that time. If
`--timeout` passes first it exits 124.

## Example

```sh
export CASES_STORE=~/notes/work/assistant/cases

# Agent: raise a decision and wait for it in the background.
id=$(cases open --kind decision --urgency blocking --worker bun-pins \
  --brief ~/briefs/bun-pins.md --title "Pin bun or float?" \
  --body-file question.md --option "Pin to 1.2.3" --option "Float and fix the lockfile")
cases wait --timeout 2h > answered.jsonl &

# Human: answer it.
cases answer "$id" --option 1 --note "Revisit after 1.3"

# Agent: read the answer, act, and record the outcome.
cases pickup "$id" --by manager
echo "Pinned in #12." | cases close "$id" --outcome-file -
cases show "$id"
```

## Development

```sh
make build   # ./cases
make test    # go test -race ./...
make lint    # golangci-lint run ./...
```

## License

MIT
