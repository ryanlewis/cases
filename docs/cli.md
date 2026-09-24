# CLI

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
[store format](store.md). `open` takes the worker from `CASES_WORKER` and
one label from `CASES_LABEL` when the flags are not given, so a session can
export them once. `CASES_LABEL` is always exactly one label: the whole value,
commas and spaces included. A `--label` flag replaces it rather than adding to
it, and `CASES_LABEL` set to an empty string is a blank label, which `open`
refuses. Only `open` reads them; `list`, `wait` and `sweep` do not. `--by` on `pickup` records who picked the case
up, such as the agent session name. `--reason` on `withdraw` records why the
case no longer needs an answer; `show` and the web thread print it.

`amend` changes an open case as described in [store format](store.md):
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

## Clearing the inbox and pruning

`sweep` and `prune` change many cases at once, so both only print what they
would do unless given `--yes` (`-y`).

`sweep` withdraws every open case that matches, with `--reason` recorded on
each withdraw (default `swept`). `--kind`, `--label` and `--worker` filter as on `list`,
and `--older-than DURATION` takes only cases opened longer ago than that (on
`list` it reads the last event instead).
Withdraw is only allowed on an open case, so a matching case that is answered
or parked is listed as left and not changed. Each withdraw is the same `agent`
withdraw event `cases withdraw` writes, and goes through the same check. It is
made at the revision `sweep --yes` read when it listed the cases, as with
`--revision`, so a case that has an event written while sweep runs, such as an
answer or an amend, is refused and left as it is, even if it is open again.
`--yes` lists the cases again rather than using what a dry run printed, so it
also withdraws a matching case that changed or was opened after the dry run.
To withdraw only a case as you read it, use `cases withdraw ID --revision N`.
If one is refused, sweep carries on with the rest, then names the cases it
could not withdraw, with the state a changed case is now in, and exits 1.

`prune` takes closed and withdrawn cases whose last event is older than
`--age` (a Go duration; default `720h`, or the `prune-age` config key; `0`
means any age). `--state closed` or `--state withdrawn` narrows it to one of
them; no other state is accepted. With `--yes` it moves each case to the
store's archive, see [Archive](store.md#archive). If the archive already has a case
with that id, the case is left where it is, the others are still moved, and
prune exits 1. So is a case that has had an event written since prune read it.
`--delete` removes the cases instead, and cannot be undone. A case that cannot
be loaded is reported on stderr and left.

`list` reads every case in the store on each run, so pruning keeps it fast as
the history grows. To prune daily, run `cases prune --yes` from cron or a
launchd job.
