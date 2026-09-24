# cases — ask a human and act on the answer

Use the `cases` CLI when you cannot go on without a person: a choice between options, approval to run scripts, sign-off on finished work, guidance when you are stuck, a reply to an open question, or something they should know. You open a case, wait for the answer in the background, pick it up, act on it, and close the case with what happened. The human answers from the terminal or from the web inbox (`cases serve`); you never answer.

## Safety

- **Safe to run freely**: `list`, `show`, `wait` without `--pickup`, `status`, `inbox --print`, `config path`, `config show`, `skill list`, `skill show`. They only read.
- **Agent writes**: `open`, `amend`, `pickup` (and `wait --pickup`, which prints a case it could not pick up as `answered`), `note`, `close`, `withdraw`. Each one adds an event to the case, and nothing can undo it: closed and withdrawn cases stay as the decision log. A write the case's state does not allow is refused with `Error: cannot <event> a case that is <state>`, writes nothing and exits 3. Exit 3 does not mean the write was already done: read the case with `show --json` to see its state. `Error: database is locked` means other writes, such as a long `cases sweep`, kept the store busy for more than 5 seconds; nothing was written, so run the command again. An error saying a `-wal` or `-shm` file `from an earlier store is still there` means the store file was removed while something still had it open: do not delete anything, and tell the human. Other errors exit 1.
- **Human writes — never run them**: `answer` and `resume`. Answering your own case, or resuming it with `resume --agent`, fakes the human's decision. If you think you know the answer, you do not need a case.
- **Never open or edit the database file; use the commands.** The state is worked out from the events stored in it, so a change made any other way corrupts the record. Do not run `sqlite3` on it, and do not move, copy over or delete it or the `-wal` and `-shm` files beside it.
- **Never put secrets or sensitive information in a case**: tokens, passwords, keys, private personal data or customer data. That covers the title, body, options, rows, context, notes and outcome. The store is a file on disk that keeps every event, and is shown in a browser. Name the secret or say where it lives instead.
- **One question per case.** Two questions in one case get one answer. Open a second case instead.
- **Do not open duplicates.** Before opening, read the table from `cases list --state open,answered,pickedup,parked` for a case of yours on the same question; each row shows the case's labels and title. Name the states: a bare `cases list` shows only open and parked cases. If your case is still open and needs changing, amend it; if it is `pickedup`, act on its answer and close it.
- `serve`, `service install` / `service uninstall` (which set serve up as a login service), `config init` and `skill install` / `skill uninstall` / `skill check` are for the human. Do not run them unasked. Bare `cases service` only reads, and you may run it.
- `cases inbox` without `--print` opens a browser on the human's machine: do not run it unless asked. `cases inbox --print` only prints a URL, and you may run it freely.
- **Never run `sweep` or `prune`.** `sweep` withdraws every open case that matches, other agents' included; `prune` moves closed and withdrawn cases out of the store, or deletes them. They are for the human. A closed or withdrawn case you still need may be pruned; `show` then fails with `no case "<id>"`.

## The store

Every command reads and writes one store: a SQLite database file. Leave it alone unless told otherwise: the default comes from `CASES_STORE`, the config file (`cases config show` prints what is in use) or `~/.local/share/cases/cases.db`. If you were told to use a store, pass `--store FILE` to every command, because the human and you must be looking at the same one.

A case id is the time the case was opened and a slug of its title, such as `2026-09-15T09-12-03Z-pin-bun-or-float`. `cases open` prints it on stdout. Keep it and pass it whole; every other command takes it.

## Lifecycle

```
open --answer--> answered --pickup--> pickedup --close--> closed
open --withdraw--> withdrawn
answered or pickedup --note--> open        (a follow-up question)
open --park--> parked --resume--> open     (stuck cases only)
open --amend--> open                       (a change before the answer)
```

- `answer`, `park` and `resume` are written by the human. Everything else is yours.
- `pickup` is only allowed on an answered case, and `close` only on a picked-up one.
- `note` on an open case adds to the thread and leaves it open. On an answered or picked-up case it reopens the case for another answer and clears the current one.
- `amend` and `withdraw` are only allowed on an open case.

## Kinds

| Kind | Use it for | Open with | The answer (`answer` in `wait` lines and `show --answer`) |
| --- | --- | --- | --- |
| `decision` | Choosing between options | `--option TEXT`, one per option (at least one) | `choice`: the 1-based option number. Or `other: true` with the human's `note`. "Other, see note" is always offered, so do not add it. |
| `approval` | Running scripts or gated actions | `--row JSON`, one per row (at least one) | `rows`: one `{id, verdict, note}` per row. `verdict` is `approve`, `hold` or `reject`. |
| `signoff` | Finished work that needs accepting | | `signoff`: `accept`, or `changes` with a `note` |
| `stuck` | You are blocked and need guidance | | `text` (guidance). The human may park the case instead of answering. |
| `question` | A question the human answers in their own words, not by picking an option | | `text` (the reply) |
| `fyi` | Something the human should know; you are not blocked | | `ack: true` |

Any kind may instead come back `drop: true`, with nothing else set but the `note`: the human does not want the work done.

Any answer may carry a `note`. Read it: it often narrows or conditions the choice.

An approval row is a JSON object with four non-empty fields. The `id` may use letters, digits, `.`, `-` and `_`, and must be unique in the case. `script` is the text itself, so it can be read on a phone, and `link` points at where it lives. The web inbox makes a `link` clickable only when it starts with `http://` or `https://`; anything else, such as a file path, is shown as text. An optional `note` is shown under the label; use it for what the human should know before approving, such as a side effect:

```sh
--row '{"id":"deps","label":"Install deps","script":"npm ci","link":"https://github.com/o/r/blob/main/setup.sh"}'
```

## Urgency

`--urgency` orders the human's inbox. Choose honestly; if everything is `blocking`, nothing is.

- `blocking` — you cannot do anything useful until it is answered.
- `today` — you are blocked on this, but have other work to get on with.
- `whenever` — it can wait. Use it for most `fyi` cases.

## Commands

Text flags come in two forms. For one line, pass it inline: `--body TEXT`, `--outcome TEXT`. For markdown, or anything longer, use `--body-file FILE` or `--outcome-file FILE`, or `-` to read stdin. Give one form, not both. An inline value that names an existing file is refused.

### `cases open`

```sh
cases open --kind KIND --urgency blocking|today|whenever --title TEXT \
  [--body TEXT | --body-file FILE|-] [--option TEXT]... [--row JSON]... \
  [--link URL]... [--label TEXT]... [--worker NAME] [--brief TEXT] \
  [--context TEXT] [--for NAME]
```

- `--title` is one line; it also names the case id.
- The body is markdown, so it usually goes in `--body-file`; `-` reads stdin. Write it for someone reading on a phone with no other context: what you are doing, what the question is, what each option costs, and what you recommend and why.
- `--option` is for `decision` only and `--row` for `approval` only; they are refused on any other kind.
- `--link URL` (repeatable) for the PR, issue or file the human should look at.
- `--label TEXT` (repeatable) groups the case with others, such as every case one piece of work opens. `list` and `wait` filter by it. A blank label, or the same label twice, is refused.
- `CASES_WORKER` and `CASES_LABEL`, if exported, stand in for `--worker` and one `--label` on `open`. `CASES_LABEL` is one label, commas included; an empty one makes `open` fail. Any `--label` flag replaces it, not adds to it. `wait` and `list` do not read them, so still pass `--label` or `--worker` there.
- `--worker NAME` names your session, the one waiting on the case. `--brief TEXT` says where to restart from if the case is parked: a brief, a ledger or a note, as a path or a short line. `--context TEXT` is free text shown to the human with the case. `--for NAME` names the human the case is addressed to, when you were told who should answer.
- Your events on a case opened with a worker record it as their `actor` (`{"name": ..., "kind": "agent"}`); without one they record none. An answer may carry the human's `actor` too; it says who answered and changes nothing about what the answer means.
- Prints the new case id.

### `cases amend`

```sh
cases amend ID [--body TEXT | --body-file FILE|-] [--option TEXT]... [--row JSON]... \
  [--link URL]... [--label TEXT]... [--context TEXT] [--revision N]
```

- Changes an open case before the human answers it: another option, another script to approve, a link, or a body or context that is wrong or out of date. The answer is checked against the case as amended, so an approval answer covers the rows you add.
- `--option` adds options to a `decision` case, numbered after the ones it has. `--row` adds rows to an `approval` case; each `id` must be new to the case. `--link` adds links and `--label` adds labels. An option, link or label the case already has is refused, and so is an amend that changes nothing, so sending the same amend twice writes nothing the second time.
- `--body` or `--body-file` replaces the whole body, so write all of it, not only what changed. `--context` replaces the context.
- An amend erases nothing: the body and context it replaces stay in the store, and `show`, `show --json` and the web thread show them. If a case holds a secret, amending it out does not remove it; tell the human so they can rotate it.
- Nothing can be removed or reordered. If the question itself has changed, withdraw the case and open a new one; for a second question, open a second case.
- Put every change in one amend: each one can send the human back to read the case again. To add to the thread without changing the case, use `note`.
- An amend does not wake `wait`; leave your wait running.
- Prints the case id and its state.

### `cases wait`

```sh
cases wait [--pickup [--by NAME]] [--since TIME|ID] [--timeout DURATION] [--id ID]... \
  [--kind KIND]... [--label TEXT]... [--worker NAME]... [--for agent|human]
```

- Blocks until a human answers, parks or resumes a case, then prints every case waiting on the agent as JSON, one object per line, and exits 0. Each line is the same case object as `show --json`, `revision` included, without `url`. Each line also has `fresh` and `next_since`.
- `--pickup` picks up each `answered` case before printing it, as `cases pickup --by NAME` does, so its line says `pickedup` and has the `revision` after the pickup. It picks up every answered case that matches, so scope it to your own: `--id`, or `--label` with `--worker` set to your session name. No later `wait` prints a case it picked up.
- `fresh` is true for the cases that woke `wait`. A case with `fresh` false was usually already waiting on you, such as a parked case, which is printed on every wake until it is resumed. It can also be an answer that records an earlier time than the `--since` you passed, so do not skip a case on `fresh` alone: a case `wait` picked up still needs you.
- `next_since` is the same on every line: pass it as `--since` when you run `wait` again.
- Run it in the background; it can take hours.
- `--id ID` (repeatable) waits on those cases only. **Always pass `--id` or `--label` for your own cases.** Without either, `wait` wakes on any case in the store, including other agents' cases.
- `--kind KIND`, `--label TEXT` and `--worker NAME` (each repeatable) wait on cases with any of those kinds or labels, or from any of those workers. A case must match every filter given, `--id` included. `--kind` alone does not scope `wait` to your own cases; pair it with `--id` or `--label`. A filter that matches none of your cases waits until the timeout, as an `--id` that is never answered does, so check the label you pass is the one you opened with.
- By default only human events written after `wait` starts can wake it. An answer that lands between `cases open` and `cases wait` would be missed, so pass `--since` with the id `cases open` printed: `wait` then counts every event since that case was opened. `--since` also takes an RFC 3339 time such as `2026-09-16T09:12:03Z`. An id that is not in the store is an error (exit 1).
- `--timeout` takes a Go duration (`30m`, `2h`). When it passes with nothing to report, `wait` prints one line to stderr and **exits 2**: run it again with the same `--since`. Other errors exit 1. The default, 0, waits forever.
- `--for human` waits for cases waiting on the human instead. It is for the human's notifiers; you do not need it.

### `cases show` and `cases list`

```sh
cases show ID [--json | --answer]
cases list [--state STATE,...|--all] [--urgency URGENCY]... \
  [--older-than DURATION] [--kind KIND]... [--label TEXT]... \
  [--worker NAME]... [--count|--json]
```

- `show --json` is the case: `state`, `kind`, `urgency`, `title`, `options`, `rows`, the current `answer`, `pickup`, `close`, `events`, which holds every event as written, `revision`, the number of events including any that were skipped, and `url`, the case's page in the human's web inbox, empty when no inbox is running.
- `show --answer` prints only `state`, `kind`, `revision` and `answer`, as JSON. `answer` is `null` when the case has none (open, parked, or reopened by a note); it exits 0 either way. Read the answer from here, not from the plain-text output.
- Without `--state`, `list` shows open and parked cases only. `--all` shows every state.
- `list --state` takes `open`, `answered`, `pickedup`, `closed`, `withdrawn` or `parked`, comma-separated or repeated, and wins over `--all`.
- `list --kind KIND`, `--urgency URGENCY`, `--label TEXT` and `--worker NAME` (each repeatable) show cases with any of those kinds, urgencies or labels, or from any of those workers. A case must match every filter given.
- `list --older-than DURATION` (`30m`, `2h`) shows cases whose last event is older than that, not their open time.
- `list --count` prints only the number of matching cases, `0` when none match.
- `list --json` prints one object, `{"list": [...]}`, the matching cases as an array under `list`, `[]` when none match. This differs from `wait`, which prints one object per line.
- A damaged event is skipped and the rest of the case still loads. `show` lists it as a problem; `list`, `show` and `wait` also warn about it on stderr. Report it to the human; do not try to fix the store.

### `cases status`

```sh
cases status [--json]
```

- Prints where the human's web inbox (`cases serve`) is running for the store: its URL and pid. `--json` prints `pid`, `url`, `addr`, `store`, `started_at` and `version`.
- Read the result:
  - Exit 0 with the URL: the inbox is running. For a link to a case, use `url` from `cases show ID --json`, or `cases inbox --print ID`.
  - Exit 1 with `not running` on stderr: no inbox is running, including one that crashed. Do not start one; tell the human they can answer from `cases serve` or the terminal.
  - Exit 1 with an `Error:` line: the check failed, usually because the inbox's state file is damaged or unreadable (the message names it). Report it to the human; do not fix or delete the file.
- Running means the process is alive and accepts connections. It cannot tell a hung inbox from a healthy one.
- `cases service [--json]` says whether the inbox is set up as a login service, whether launchd or systemd has it, and whether it answers, with a `Problem:` line for each disagreement. It only reads. Exit 1 means it is not installed, loaded and answering; report that to the human rather than installing or restarting anything.
- Only if the `cases` CLI cannot run at all: the state file is `$XDG_STATE_HOME/cases/serve-<slug>-<hash>.json` (default `~/.local/state/cases`), with the same fields as `--json`. A crash can leave it behind, so it may be stale; `cases status` is the authority.
- `cases inbox [ID] [--print]` opens the running inbox, or a case's page in it, in the browser. Opening a browser is for the human: run it only with `--print`, or when asked to open it. `cases inbox --print ID` prints the case's link without opening anything, the same link `cases show ID --json` has as `url`.

### `cases pickup`, `cases note`, `cases close`, `cases withdraw`

```sh
cases pickup   ID [--by NAME] [--revision N]
cases note     ID --body TEXT | --body-file FILE|- [--revision N]
cases close    ID --outcome TEXT | --outcome-file FILE|- [--link URL]... [--revision N]
cases withdraw ID [--reason TEXT] [--revision N]
```

- `--revision N` refuses the write, and writes nothing, if the case has changed since you read it at revision N. Take N from the `revision` in the `wait` line, `show --answer` or `show --json` you acted on. **Always pass it on `note` and `close`**: a note on a case the human has answered since you read it reopens the case and throws that answer away. If the write is refused as stale, read the case again with `show --answer` before deciding what to do. `amend`, `pickup` and `withdraw` take it too.

- `pickup` records that you have read the answer. Do it before you act, so the human can see the answer was received. `wait --pickup` does it for you.
- `note` adds a follow-up in markdown. Use it to ask a clarifying question about the answer; the case goes back to `open` for another answer.
- `close` records the outcome in markdown: what you did, and anything that did not go as planned. The outcome must not be empty. `--link` (repeatable) points at the evidence: a commit, PR or log.
- `withdraw` an open case that no longer needs an answer, for example because you found the answer yourself. `--reason` tells the human why. You cannot withdraw a case once it is answered; pick it up and close it instead.
- Each prints the case id and its new state.

## Workflow

A case takes two turns. In the first, open the case and wait for its answer in one command, run in the background:

```sh
id=$(cases open --kind decision --urgency today --worker bun-pins \
  --title "Pin bun or float?" --body-file question.md \
  --option "Pin to 1.2.3" --option "Float and fix the lockfile") &&
  cases wait --id "$id" --since "$id" --pickup --by bun-pins
```

While it waits, `cases list --worker NAME` shows the case's id, to amend or withdraw it. If the command stops with no answer, run only its `cases wait` part again: all of it would open a second case.

In the same turn, run `cases status` on its own: if it exits 1, no inbox is running, so tell the human they can answer from `cases serve` or the terminal.

In the second turn, when the command finishes, read its output from the notice or the background output file: one line per case, an answered one already picked up. If that output is gone, find the case with `cases list --state pickedup --worker NAME` and read it with `cases show ID --answer`. Act on the answer, and chain the close onto your last command with the line's `id` and `revision`:

```sh
git commit -qam "Pin bun to 1.2.3" &&
  cases close 2026-09-24T09-12-03Z-pin-bun-or-float --revision 3 \
    --outcome "Pinned bun to 1.2.3 in abc123." --link https://github.com/o/r/pull/12
```

Read each line's `state` and act on it:

- `pickedup` — `wait` picked it up for you. Follow the answer:
  - `drop: true`, on any kind: stop that work and do not act on the case. Close with what you stopped, and the `note` if there is one.
  - `decision`: do option `choice` (options are numbered from 1). For `other`, do what the `note` says.
  - `approval`: run only the rows with `approve`. Do not run `hold` or `reject` rows. Say in the outcome which rows ran and what they did.
  - `signoff`: on `accept`, close. On `changes`, make the changes the note asks for, then `note` the case to ask for another look, and close once it is accepted.
  - `stuck`: follow the `text`. Close with what you did.
  - `question`: use the `text`. Close with what you did with it.
  - `fyi`: close with a short outcome, for example "Acknowledged".
- `answered` — `wait` could not pick it up, usually because the case changed after it read it; stderr says why. Read it again with `cases show ID --answer`. If it is still `answered` at revision N, pick it up with `--revision N`, follow the answer, and close with `--revision` N+1. If it is `pickedup`, another wait of yours took it: act on it only once. If it is `open`, wait again.
- `parked` — the human has set the work aside. Stop the work and wait again with `--since` set to the `next_since` that `wait` printed. With the old `--since`, `wait` returns at once on the same park; with none, a resume that lands before `wait` starts is missed. The next event will be a `resume`.
- `open` after a resume — re-read your instructions and the thread, then wait for the answer, again with `--since` set to the new `next_since`. If you are no longer stuck, withdraw the case.

A session that opens several cases gives them all the same `--label`, such as the name of its work, and its session name as `--worker`. It runs one `wait` with both, in place of the `cases wait` part of each command, so a case it opens later is covered without restarting `wait`; its first `--since` is the id of the first case it opened. Do not also run `wait --id` on those cases: of two `--pickup` waits on one case, the one that loses may never wake.

If the answer is unclear, `note` the case with your question rather than guessing, and wait again in one background command: `cases note ID --body-file q.md --revision N && cases wait --id ID --since NEXT_SINCE --pickup --by NAME`, or the note alone when a label wait already covers the case. Close every case you pick up: an unclosed case looks to the human like work still in progress.
