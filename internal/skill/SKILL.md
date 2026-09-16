# cases — ask a human and act on the answer

Use the `cases` CLI when you cannot go on without a person: a choice between options, approval to run scripts, sign-off on finished work, guidance when you are stuck, a reply to an open question, or something they should know. You open a case, wait for the answer in the background, pick it up, act on it, and close the case with what happened. The human answers from the terminal or from the web inbox (`cases serve`); you never answer.

## Safety

- **Safe to run freely**: `list`, `show`, `wait`, `status`, `config path`, `config show`, `skill list`, `skill show`. They only read.
- **Agent writes**: `open`, `pickup`, `note`, `close`, `withdraw`. Each one adds an event file to the case, and nothing can undo it: closed and withdrawn cases stay as the decision log. A write the case's state does not allow is refused with `Error: cannot <event> a case that is <state>` and writes nothing.
- **Human writes — never run them**: `answer` and `resume`. Answering your own case, or resuming it with `resume --agent`, fakes the human's decision. If you think you know the answer, you do not need a case.
- **Never edit, rename or delete files in the store.** The state is worked out from the files, so a hand edit corrupts the record. Use the commands.
- **Never put secrets or sensitive information in a case**: tokens, passwords, keys, private personal data or customer data. That covers the title, body, options, rows, context, notes and outcome. The store is plain JSON on disk, may be synced, and is shown in a browser. Name the secret or say where it lives instead.
- **One question per case.** Two questions in one case get one answer. Open a second case instead.
- **Do not open duplicates.** Before opening, check `cases list --state open,answered,parked --json` for a case of yours on the same question.
- `serve`, `config init` and `skill install` / `skill uninstall` are for the human. Do not run them unasked.

## The store

Every command reads and writes one store directory. Leave it alone unless told otherwise: the default comes from `CASES_STORE`, the config file (`cases config show` prints what is in use) or `~/.local/share/cases`. If you were told to use a store, pass `--store DIR` to every command, because the human and you must be looking at the same one.

A case id is the name of its directory, such as `2026-09-15T09-12-03Z-pin-bun-or-float`. `cases open` prints it on stdout. Keep it; every other command takes it.

## Lifecycle

```
open --answer--> answered --pickup--> pickedup --close--> closed
open --withdraw--> withdrawn
answered or pickedup --note--> open        (a follow-up question)
open --park--> parked --resume--> open     (stuck cases only)
```

- `answer`, `park` and `resume` are written by the human. Everything else is yours.
- `pickup` is only allowed on an answered case, and `close` only on a picked-up one.
- `note` on an open case adds to the thread and leaves it open. On an answered or picked-up case it reopens the case for another answer and clears the current one.
- `withdraw` is only allowed on an open case.

## Kinds

| Kind | Use it for | Open with | The answer (`answer` in `show --json`) |
| --- | --- | --- | --- |
| `decision` | Choosing between options | `--option TEXT`, one per option (at least one) | `choice`: the 1-based option number. Or `other: true` with the human's `note`. "Other, see note" is always offered, so do not add it. |
| `approval` | Running scripts or gated actions | `--row JSON`, one per row (at least one) | `rows`: one `{id, verdict, note}` per row. `verdict` is `approve`, `hold` or `reject`. |
| `signoff` | Finished work that needs accepting | | `signoff`: `accept`, or `changes` with a `note` |
| `stuck` | You are blocked and need guidance | | `text` (guidance) or `drop: true`. The human may park the case instead of answering. |
| `question` | A question the human answers in their own words, not by picking an option | | `text` (the reply) |
| `fyi` | Something the human should know; you are not blocked | | `ack: true` |

Any answer may carry a `note`. Read it: it often narrows or conditions the choice.

An approval row is a JSON object with four non-empty fields. The `id` may use letters, digits, `.`, `-` and `_`, and must be unique in the case. `script` is the text itself, so it can be read on a phone, and `link` points at where it lives. An optional `note` is shown under the label; use it for what the human should know before approving, such as a side effect:

```sh
--row '{"id":"deps","label":"Install deps","script":"npm ci","link":"https://github.com/o/r/blob/main/setup.sh"}'
```

## Urgency

`--urgency` orders the human's inbox. Choose honestly; if everything is `blocking`, nothing is.

- `blocking` — you cannot do anything useful until it is answered.
- `today` — you are blocked on this, but have other work to get on with.
- `whenever` — it can wait. Use it for most `fyi` cases.

## Commands

### `cases open`

```sh
cases open --kind KIND --urgency blocking|today|whenever --title TEXT \
  [--body-file FILE|-] [--option TEXT]... [--row JSON]... \
  [--link URL]... [--worker NAME] [--brief PATH] [--context TEXT]
```

- `--title` is one line; it also names the case directory.
- `--body-file` is markdown; `-` reads stdin. Write it for someone reading on a phone with no other context: what you are doing, what the question is, what each option costs, and what you recommend and why.
- `--option` is for `decision` only and `--row` for `approval` only; they are refused on any other kind.
- `--link URL` (repeatable) for the PR, issue or file the human should look at.
- `--worker NAME` names your session, the one waiting on the case. `--brief PATH` is the path to the instructions your session started from, so the work can be restarted if the case is parked. `--context TEXT` is free text shown to the human with the case.
- Prints the new case id.

### `cases wait`

```sh
cases wait [--since TIME] [--timeout DURATION] [--id ID]...
```

- Blocks until a human answers, parks or resumes a case, then prints every case waiting on the agent as JSON, one object per line, and exits 0. Each line is the same case object as `show --json`.
- Run it in the background; it can take hours.
- `--id ID` (repeatable) waits on those cases only. **Always pass it for your own cases.** Without it, `wait` wakes on any case in the store, including other agents' cases.
- By default only human events written after `wait` starts can wake it. An answer that lands between `cases open` and `cases wait` would be missed, so pass `--since` with a time from before you opened the case: an RFC 3339 time such as `2026-09-16T09:12:03Z`, or the case's `opened_at` from `show --json`.
- `--timeout` takes a Go duration (`30m`, `2h`). When it passes with nothing to report, `wait` prints one line to stderr and **exits 2**. Other errors exit 1. The default, 0, waits forever.

### `cases show` and `cases list`

```sh
cases show ID [--json]
cases list [--state STATE,...] [--json]
```

- `show --json` is the case: `state`, `kind`, `urgency`, `title`, `options`, `rows`, the current `answer`, `pickup`, `close`, and `events`, which holds every event file as written. Read the answer from here, not from the plain-text output.
- `list --state` takes `open`, `answered`, `pickedup`, `closed`, `withdrawn` or `parked`, comma-separated or repeated.
- A damaged event file is skipped and the rest of the case still loads. `show` lists it as a problem; `list`, `show` and `wait` also warn about it on stderr. Report it to the human; do not fix the file.

### `cases status`

```sh
cases status [--json]
```

- Prints where the human's web inbox (`cases serve`) is running for the store: its URL and pid. `--json` prints `pid`, `url`, `addr`, `store`, `started_at` and `version`.
- Read the result:
  - Exit 0 with the URL: the inbox is running. Give the human a link to the case: the URL followed by `cases/ID`.
  - Exit 1 with `not running` on stderr: no inbox is running, including one that crashed. Do not start one; tell the human they can answer from `cases serve` or the terminal.
  - Exit 1 with an `Error:` line: the check failed, usually because the inbox's state file is damaged or unreadable (the message names it). Report it to the human; do not fix or delete the file.
- Running means the process is alive and accepts connections. It cannot tell a hung inbox from a healthy one.
- Only if the `cases` CLI cannot run at all: the state file is `$XDG_STATE_HOME/cases/serve-<slug>-<hash>.json` (default `~/.local/state/cases`), with the same fields as `--json`. A crash can leave it behind, so it may be stale; `cases status` is the authority.

### `cases pickup`, `cases note`, `cases close`, `cases withdraw`

```sh
cases pickup   ID [--by NAME]
cases note     ID --body-file FILE|-
cases close    ID --outcome-file FILE|- [--link URL]...
cases withdraw ID [--reason TEXT]
```

- `pickup` records that you have read the answer. Do it before you act, so the human can see the answer was received.
- `note` adds a follow-up in markdown. Use it to ask a clarifying question about the answer; the case goes back to `open` for another answer.
- `close` records the outcome in markdown: what you did, and anything that did not go as planned. The outcome must not be empty. `--link` (repeatable) points at the evidence: a commit, PR or log.
- `withdraw` an open case that no longer needs an answer, for example because you found the answer yourself. `--reason` tells the human why. You cannot withdraw a case once it is answered; pick it up and close it instead.
- Each prints the case id and its new state.

## Workflow

```sh
since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
id=$(cases open --kind decision --urgency today --worker bun-pins \
  --title "Pin bun or float?" --body-file question.md \
  --option "Pin to 1.2.3" --option "Float and fix the lockfile")

# In the background. Exit 2 means the timeout passed: run it again.
cases wait --id "$id" --since "$since" --timeout 2h

cases show "$id" --json            # read .state and .answer
cases pickup "$id" --by bun-pins
# ... act on the answer ...
echo "Pinned bun to 1.2.3 in abc123." | cases close "$id" --outcome-file - --link https://github.com/o/r/pull/12
```

Check the inbox with `cases status` at two points:

- Right after `cases open`: if it is running, give the human the link to the case; if not, tell them the case id and that they can answer from `cases serve` or the terminal.
- When `wait` times out twice in a row: if the inbox is not running, tell the human, so they can start it or answer from the terminal, then wait again.

When `wait` returns, read the case's `state` and act on it:

- `answered` — pick it up and follow the answer:
  - `decision`: do option `choice` (options are numbered from 1). For `other`, do what the `note` says.
  - `approval`: run only the rows with `approve`. Do not run `hold` or `reject` rows. Say in the outcome which rows ran and what they did.
  - `signoff`: on `accept`, close. On `changes`, make the changes the note asks for, then `note` the case to ask for another look, and close once it is accepted.
  - `stuck`: follow the `text`, or on `drop` stop that work. Close with what you did.
  - `question`: use the `text`. Close with what you did with it.
  - `fyi`: close with a short outcome, for example "Acknowledged".
- `parked` — the human has set the work aside. Stop the work, do not pick up, and wait again with `--since` set to the case's `updated_at` from the line `wait` printed, not the old time: the park is later than the old time, so `wait` would return at once, again and again, and with no `--since` a resume that lands before `wait` starts is missed. The next event will be a `resume`.
- `open` after a resume — re-read your instructions and the thread, then wait for the answer, again with `--since` set to the case's new `updated_at`. If you are no longer stuck, withdraw the case.

If the answer is unclear, `pickup` and then `note` with the question, rather than guessing. Close every case you pick up: an unclosed case looks to the human like work still in progress.
