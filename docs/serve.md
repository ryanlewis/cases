# serve

`cases serve` runs a web inbox over the store and opens it in the browser
(`open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows,
`xdg-open` elsewhere). Pass `--no-open`, or set
`no-open = true` in the [config file](configuration.md), to skip that.

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

## Finding a running serve

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

## Running serve as a service

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
in the [config file](configuration.md), as they do for serve. It sets `HOME` and
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
1 otherwise. When launchd or systemd cannot be asked at all, for example
because `launchctl` or the user's systemd bus cannot be reached, it prints the
error instead of reporting the service not loaded. `--json` prints
`installed`, `path`, `args`, `store`, `listen`, `loaded`, `running`, `url`,
`pid` and `problems`.

`cases service uninstall` stops the service and removes its file. It fails
when there is none, and leaves the file in place when it cannot ask launchd or
systemd whether the service is loaded. Other systems are not supported; run
`cases serve` under your own supervisor there.

Removing cases does not remove the service, so run `cases service uninstall`
first. Otherwise launchd or systemd keeps trying to start a binary that is no
longer there. On macOS, `brew uninstall --zap cases` also removes the launchd
agent, along with `serve.log` and the instance files in `~/.local/state/cases`.
It leaves the store and the config file alone. A plain `brew uninstall`, and
`brew upgrade`, leave the service in place. On Linux, Homebrew cannot remove a
systemd user unit, so run `cases service uninstall` before
`brew uninstall cases`.

## The inbox

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
event is one of those three and the id names a case in the store. The line
slides in, stays six seconds (longer while the pointer is over it or it has
the focus), then fades and folds away, and the two parameters leave the
address with it, so a reload after that does not show it again. Under reduced
motion it only fades. If the form is refused, the same case is shown again
with the error.

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

### Keys

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

## Browser notifications

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
