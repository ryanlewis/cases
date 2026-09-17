# Telling the human a case needs them

Status: proposal, 2026-09-17. Nothing here is built.

Today nothing in cases reaches outside the store. A blocking case raised at
09:00 is still unanswered at 14:00 unless the human looks at the inbox. This
document proposes one notifier: it detects events in the store, matches them
against routing rules in the config file, and hands them to sinks. The sinks
are a command, the open browser tab, and an HTTP webhook.

It replaces three smaller proposals: a `notify-command` hook on `cases open`,
a terminal bell in `serve`, and a directory of notify scripts.

## What exists today

- **Events.** Nine event types, each written by a fixed author
  (`authors` in `internal/store/fold.go`): the agent writes `open`, `amend`,
  `pickup`, `note`, `close` and `withdraw`; the human writes `answer` and
  `park`; either side writes `resume`. State is derived, never stored
  (`internal/store/types.go`).
- **Change detection.** `store.Poller` (`internal/store/list.go`) reloads only
  case directories whose mtime changed. Nothing runs it on a timer except the
  terminal status screen (`cmd/cases/serve_screen.go`, once a second, and only
  when stdout is a terminal). The web server polls only when a request arrives
  (`Server.cases` in `internal/web/web.go`).
- **Agent side.** `cases wait` (`cmd/cases/wait.go`) wakes when a case's last
  event is a human `answer`, `park` or `resume` (`needsAgent`) and that file is
  new: absent on its first poll, or later than `--since`. It then prints every
  case waiting on the agent and exits. It judges "new" by the file appearing,
  not its timestamp, because synced files can arrive late.
- **Human side.** `cases wait --for human` is being added in parallel. This
  document assumes it mirrors `needsAgent`: it wakes when a case newly lands on
  the human (an `open`, a `note` that reopens an answered case, an agent
  `resume`) and exits. Check this against the merged code.
- **Browser.** The inbox polls `/fragments/inbox` every 2s and the case page
  polls `/cases/{id}/thread` every 2s (`internal/web/templates/`). The CSP is
  `default-src 'self'`, with no inline scripts; the only scripts are
  `htmx.min.js` and `prefs.js`.
- **Config.** `internal/config/config.go` accepts only keys in `Keys`, every
  value a string, each seeding one flag. A table or array is an error today.
- **State outside the store.** `internal/instance` writes
  `$XDG_STATE_HOME/cases/serve-<slug>-<hash>.json` per store. That directory
  is per machine and never synced.
- **Exec.** The only command cases runs is `openBrowser` in
  `cmd/cases/serve.go`.

`wait` and `wait --for human` are one-shot. They have no rules, no sinks and no
memory across runs beyond `--since`. They remain the right tool for an agent
blocking on a case. They are not a notifier.

## Events worth notifying on

The notifier sees event files, not states. It names a few derived events,
because the raw type alone is not enough (a `note` can mean two things).

| Event name | Detected as | For | In slice 1 |
| --- | --- | --- | --- |
| `open` | an `open` file | human | on |
| `reopen` | a `note` on an answered or picked-up case | human | on |
| `resume` | a `resume` by the agent (case back in the inbox) | human | on |
| `amend` | an `amend` on an open case | human | off |
| `note` | a `note` on an open case | human | off |
| `stale` | a case still `open` with no new event after a rule's `after` | human | off |
| `withdraw` | a `withdraw` | human | off (see browser sink) |
| `close` | a `close`, with its outcome | human, as an FYI | off |
| `answer`, `park` | human files | agent | off |
| `resume-human` | a `resume` by the human | agent | off |
| `pickup` | a `pickup` | neither | off |

Slice 1 has a fixed rule (the "on" rows). Once rules exist, nothing fires
unless a rule names the event.

Notes:

- `reopen` needs the state before the note. The fold does not keep it, so
  `apply` would record the prior state on the `Event`, the way it already
  records `replacedBody` for amends. Small change, no format change.
- `stale` is the only event that is not a file. It needs a ticker, and its
  dedup key is the case plus its latest event file, so a reminder fires once
  per quiet stretch and again only after something new happens.
- Agent-side events are routable, so a human could be told "the agent got your
  answer". The agent itself should keep using `cases wait`. The notifier does
  not wake agents.
- Urgency is not an event. It is a match field. The fold has no urgency change:
  `amend` cannot change it.
- New means the file appeared since the notifier last looked, as in `wait`.
  Timestamps are only used for `stale`.

## Routing rules

Rules and sinks live in the existing config file as two arrays of tables.

```toml
[[notify.sink]]
name = "phone"
type = "exec"
command = "~/bin/cases-to-hubbub"   # run with sh -c; case data only on stdin and env
timeout = "10s"

[[notify.sink]]
name = "tab"
type = "browser"

[[notify.rule]]
events  = ["open", "reopen", "resume"]
urgency = ["blocking"]
sinks   = ["phone", "tab"]

[[notify.rule]]
events  = ["open", "reopen", "resume"]
urgency = ["today", "whenever"]
sinks   = ["tab"]

[[notify.rule]]
events  = ["stale"]
urgency = ["blocking"]
after   = "2h"
sinks   = ["phone"]
```

Matching:

- Fields: `events`, `kind`, `urgency`, `label`, `worker`, `author`. Each is a
  list; an item matches if it equals the case's value (for `label`, any of the
  case's labels). A field left out matches everything.
- Every rule that matches fires. There is no first-match or `stop`. A sink
  gets a case-event at most once, even when two rules route it there.
- No rules configured means no notifications. `cases config init` writes the
  example above, commented out.

What the config loader needs:

- `notify` becomes a reserved top-level table. `Load` decodes it into typed
  structs (`[]Rule`, `[]Sink`) with go-toml and skips it in the string-key
  loop. It seeds no flag, so `Keys`, `Resolver` and
  `TestConfigKeysMatchFlags` are untouched.
- Validation at load: unknown fields, unknown event names, a rule naming a
  sink that does not exist, a bad duration. Same `config.Error` as today, so a
  broken rule stops commands and `cases config show` still reports it.
- `cases config show` lists rules and sinks.
- `cases notify check` prints, for the current store, which rules would fire
  for each open case, without running anything. This is how a user tests a
  rule.

### Dedup and the ledger

A rule must fire once per case-event, across serve restarts.

- Key: case id + event file name + sink. For `stale`: case id + latest event
  file + `stale` + `after` + sink.
- Ledger file: `$XDG_STATE_HOME/cases/notify-<slug>-<hash>.json`, named like
  the instance file (`instance.Path`). Never in the store: the store is synced
  and append-only, and what this machine has told its human is not a case
  event.
- First run with no ledger: record everything already in the store as seen
  and send nothing. Otherwise the first start sends the whole history.
- Events that land while serve is down are sent on the next start. If more
  than 5 are due for one sink at start, send one summary instead.
- A key is written after the sink succeeds. A failed send is retried on the
  next two ticks, then recorded as failed and logged. A crash mid-send can
  repeat a notification. That is the better failure for this job.
- Keys for case directories that no longer exist (after `prune`) are dropped.
- The notifier holds an `flock` on the ledger, so two notifiers on one machine
  and store do not both send. Two machines sharing a synced store each have
  their own ledger and will both notify. That is not solved here; a hosted
  store solves it (see below).

## Sinks

Every sink receives the same payload:

```json
{
  "rule": 0,
  "sink": "phone",
  "event": {"name": "open", "seq": 1, "author": "agent", "file": "0001-agent-open.json", "at": "…"},
  "case": { "…": "the object `cases show --json` prints" },
  "url": "http://127.0.0.1:8765/cases/<id>"
}
```

### exec

Runs a command per notification. This is the sink that makes hubbub, ntfy,
Things, tmux or anything else the user's choice rather than code in cases.

- `command` runs with `sh -c`, so pipes work. Case data never goes into the
  command string: the payload is on stdin, and the main fields are in the
  environment: `CASES_ID`, `CASES_EVENT`, `CASES_AUTHOR`, `CASES_KIND`,
  `CASES_URGENCY`, `CASES_STATE`, `CASES_TITLE` (newlines removed),
  `CASES_WORKER`, `CASES_LABELS` (comma-joined), `CASES_URL`, `CASES_RULE`,
  `CASES_SINK`.
- Timeout (default 10s) kills the process group. Exit 0 is success. The first
  line of stderr goes to the serve log on failure. Stdout is discarded.
- Cannot: return anything to cases, or run on a machine without the notifier.
- Tests: a command that writes stdin and env to a file in `t.TempDir()`; a
  `sleep` for the timeout; a non-zero exit for the retry path.

### browser

A desktop notification from an open inbox tab, through the Notifications API.

- **How the tab learns.** Serve keeps the last 50 browser-sink notifications in
  memory, each with an increasing id, and serves them as JSON at
  `GET /notifications?after=N`. `prefs.js` fetches it every 5 seconds on every
  page. It does not use the htmx fragments: `/fragments/inbox` polls only on
  the inbox and the thread only on a case page, and reading side effects out
  of swapped HTML ties notifications to markup.
- **Cursor.** The last shown id is kept in `localStorage`. A tab with no
  cursor starts at the latest id and shows nothing, so opening the inbox in
  the morning does not replay the night. The response also carries a `boot`
  id that changes when serve restarts; a tab that sees a new `boot` resets its
  cursor, since the in-memory ids start again.
- **Permission.** A "Desktop notifications" fieldset in the options dialog in
  `layout.html`. `Notification.requestPermission()` must run from a click, so
  it is a button, not a stored radio value. The fieldset shows granted,
  denied (with "change it in the browser's site settings") or not asked.
- **CSP.** Nothing to change. `fetch` to the same origin is allowed by
  `default-src 'self'`; the Notifications API is not governed by CSP; the icon
  comes from `/static/`. All code stays in `prefs.js`, no inline script.
  `http://127.0.0.1` counts as a secure context, which the API requires.
- **Several tabs.** Each tab may show the same item. The notification `tag` is
  the case id plus event, so the OS replaces rather than stacks. A `withdraw`
  routed to the browser closes the notification with that case's tag instead
  of showing a new one.
- **Click.** Focuses the tab and opens the case page.
- **Cannot.** Nothing is shown when no tab is open. Browsers slow timers in
  hidden tabs, so a background tab can lag by up to about a minute. Web Push
  would cover a closed tab but needs a service worker, a push subscription and
  a server that reaches the browser vendor's push service. That belongs in the
  hosted design, not here.
- **Dedup.** The ledger marks a browser item done when it is queued, not when
  a tab shows it. Serve cannot know whether a tab is open.
- **Tests.** Web tests with `newApp(t)` on `/notifications` (ids, `after`,
  cap, Host guard). The JavaScript has no test harness in the repo; it gets a
  manual check list in the PR.

### webhook

`POST` the payload as JSON to a URL.

- Fields: `url`, `timeout` (default 10s), `bearer-env` (the name of an
  environment variable holding a token, sent as `Authorization: Bearer`; the
  token itself never goes in the config file).
- 2xx is success; other responses and timeouts follow the ledger retry rule.
- Cannot: reshape the body. hubbub's `POST /v1/notify` wants
  `{title, message, priority}`, so hubbub is reached through `exec` with curl,
  not through this sink.
- This is the sink a hosted cases server would run, unchanged.
- Tests: `httptest.Server`, checking body, header, timeout and retry.

### terminal (not proposed)

A bell or OSC 9 written by serve to its terminal.

- When serve runs as a daemon, stdout is a log file or `/dev/null`, and
  nothing rings.
- In a herdr pane, OSC 0/2 titles are swallowed and OSC 9 may not pass
  through the pane emulator.
- Anyone who wants a terminal signal can get one from `exec`:
  `herdr notification show` or `tmux display-message`.

Recommendation: do not build it. See decision 3.

## Where the notifier runs

| Option | For | Against |
| --- | --- | --- |
| Inside `cases serve` | Already long-running and already has a `Poller`. The browser sink needs serve anyway. The user runs serve as a daemon. | Serve must be running. Needs a ticker goroutine; today serve only polls on requests. |
| A separate `cases notify` | Works without the web inbox. | A second daemon to install. Cannot feed the browser tab. |
| Exec hook in `cases open` | Smallest change. Works with nothing running. | Runs inside the agent's process, often sandboxed with no network or credentials. Sees only `open`: no reopen, resume or stale. A second routing path to keep in step. Does not exist on a remote store. |

Proposal:

- The engine lives in a new `internal/notify` package: detector (a `Poller`
  diff plus the ledger), rules, sinks. It takes a clock and a sink runner, so
  it is tested without serve.
- `cases serve` runs it in one goroutine on a 2s ticker, sharing the server's
  mutex-guarded `Poller` so the store is not read twice. It logs to the serve
  log and adds a "notifications sent / failed" line to the status screen.
- `cases notify` is the same engine in the foreground for people who do not
  run serve. It is cheap once the engine exists, and it refuses to start when
  serve is already notifying for that store (the ledger lock).
- No hook in `cases open`. The user chose "open plus serve", but the open hook
  is the weakest part of that: sandboxed, open-only, and a second path. This is
  decision 2.

## A remote store

Agreed with `hosted-vision` (writing `docs/design/hosted.md`):

- A hosted server gives each accepted event a store-wide cursor and serves
  `GET /v1/events?after=CURSOR` as SSE or long-poll. Its notifier subscribes to
  that after-write hook instead of diffing a `Poller`. Rules, ledger keys and
  payload stay the same.
- `webhook` runs on the server unchanged. `exec` stays local: a shared server
  should not run user commands.
- The browser tab keeps its client code. Only the transport changes, from
  `GET /notifications?after=N` to the SSE feed.

## Recipes

For the README, under `exec`. Each is a sink `command`. They read the payload
from stdin and fields from the environment.

**ntfy**

```sh
curl -s -H "Title: $CASES_TITLE" -H "Priority: $([ "$CASES_URGENCY" = blocking ] && echo high || echo default)" \
  -H "Click: $CASES_URL" -d "$CASES_KIND case from ${CASES_WORKER:-an agent}" \
  https://ntfy.sh/my-cases-topic
```

**hubbub** (`POST /v1/notify` with a bearer key, per hubbub's README)

```sh
jq -c '{title: .case.title, message: "\(.case.kind) · \(.case.urgency)\n\(.url)",
        priority: (if .case.urgency == "blocking" then "high" else "default" end)}' |
  curl -s -H "Authorization: Bearer $HUBBUB_KEY" -H 'Content-Type: application/json' -d @- "$HUBBUB_URL/v1/notify"
```

**Things 3** (create only; completing it on close needs a case-to-todo map and
is left out)

```sh
things add "$CASES_TITLE" --notes "$CASES_URL" --tags cases --when today
```

**tmux**

```sh
tmux display-message "cases: $CASES_URGENCY $CASES_KIND: $CASES_TITLE"
```

hubbub has no CLI; it is an HTTP API, so the recipe is curl. The Things line
uses the `things` CLI (`things add` with `--notes`, `--tags`, `--when`).
Recipes live in the README only. There is no `examples/`
directory for CI to miss.

## First slice and what follows

**Slice 1 (M): the tab tells you.** Useful alone, and it is what the user asked
for when serve runs as a daemon.

- `internal/notify` engine with the detector and a fixed rule: `open`,
  `reopen` and agent `resume`, every urgency. No config yet.
- Record the prior state on `Event` for `reopen`.
- Browser sink: `/notifications`, the `prefs.js` fetch loop, the options
  fieldset with a "blocking only / every case" choice kept in `localStorage`.
- Serve runs the engine on a ticker. No ledger yet: the tab's cursor already
  stops repeats, and a restart starts from the current store.
- README section for the browser notification, and the web conventions in
  `CLAUDE.md` (what `prefs.js` now does).

**Slice 2 (M): rules and commands.**

- `[[notify.rule]]` and `[[notify.sink]]` in the config loader, with
  validation and `cases config show` output.
- `exec` sink, the ledger with its lock and first-run baseline,
  `cases notify check`.
- The README recipes.

**Slice 3 (S–M each, in any order).**

- `stale` rules with `after`.
- `webhook` sink.
- `cases notify` foreground command.
- `withdraw` closing browser notifications.

## Decisions for the user

**1. What the first slice delivers.**
Options: (a) browser notifications from the serve tab first, rules and exec
second; (b) rules, ledger and exec first, browser second; (c) both in one
slice (L).
Recommendation: (a). The tab is the path the user said they care about, it
needs no config to be useful, and the engine it forces is the one slice 2
plugs rules into. (b) gets hubbub working sooner, but hubbub can already be
reached today with `cases wait --for human` in a shell loop, without rules or
dedup.

**2. Whether `cases open` runs a hook.**
Options: (a) no hook; the notifier runs in serve, with `cases notify` for
people without serve; (b) keep the `notify-command` hook on `open` as well, as
originally chosen; (c) hook on `open` only, no serve notifier.
Recommendation: (a). The hook runs inside the agent's sandbox, sees only
opens, and would be a second routing path that a hosted store cannot have.
`cases notify` covers the "nothing else running" case without those costs.

**3. The terminal bell.**
Options: (a) do not build a terminal sink; point to `exec` with
`herdr notification show` or `tmux display-message`; (b) a bell on the status
screen when the blocking count rises, behind a flag; (c) a full terminal sink
with bell and OSC 9.
Recommendation: (a). Serve runs as a daemon, so the bell reaches no one, and
herdr swallows the title and possibly OSC 9. `exec` already reaches the
terminal multiplexer where it matters.
