# cases

CLI and local web inbox for a store of cases raised by an agent and answered by a human. The store is a directory with one directory per case and one JSON file per event (`NNNN-<author>-<event>.json`). State is the fold of those files in name order. The runtime is the Go standard library plus kong, goldmark and go-toml.

## Workflow

- Personal repo at github.com/ryanlewis/cases (private). Commit to `main` and push; CI runs on push and pull requests.
- **DO** use Conventional Commits.
- **NEVER** edit or delete event files in a store. Changes to the record format must keep reading files written by earlier versions.
- **DO** update README.md in the same change when a command, flag, event or record field changes.
- **DO** update `internal/skill/SKILL.md` when adding, removing or changing a subcommand's surface. The bundled agent skill ships in the binary and drifts silently otherwise.

## Commands

CI runs the same checks.

```
make build   # go build -o cases ./cmd/cases
make install # go install ./cmd/cases
make test    # go test -race ./...
make cover   # coverage summary
make lint    # golangci-lint run ./... (v2 config in .golangci.yml)
make fmt     # golangci-lint fmt ./...
```

## Architecture

- `internal/store/` — the store.
  - `types.go` — kinds, urgencies, states, authors, event types, and one record struct per event.
  - `fold.go` — `Load(dir)` folds a case directory into a `Case`. `apply` is the only place transitions are checked; the record validators it calls live in `validate.go`.
  - `write.go` — `Create` and one append function per event. Each append locks the case directory, loads it, runs `apply` on the new event, and only then writes the file atomically: the temp file is hard-linked to the event's name, which fails if the name is taken, and renamed only after a second check where hard links do not work. `Answer`, `Park` and `Resume` take an optional `AtRevision(rev)`, checked in the same locked section before `apply`; a case whose `Revision()` (its number of event files) has moved on fails with `ErrStale`.
  - `list.go` — `List(root)` and `Poller`, which reloads only case directories whose mtime changed. `cases wait` uses it.
  - `lock_unix.go` — `flock` on the case directory. It stops two local writers taking the same sequence number; it does nothing across machines.
  - `describe.go` — `(*Case).Describe(ev)`, plain-text lines for an event, shared by `show` and the web thread.
- `internal/web/` — `cases serve`. `web.go` holds the `Server`, routes, the request guard (Host must be the loopback listen address; POSTs with a foreign `Sec-Fetch-Site` or `Origin` get 403; CSP `default-src 'self'`) and the request log. `views.go` holds the handlers and view data, `forms.go` holds form parsing. Templates in `templates/` and `static/` are embedded. Reads go through one mutex-guarded `store.Poller`; POSTs load the case from disk and write through `store.Answer`, `store.Park` or `store.Resume`, then redirect.
- `internal/instance/` — the file a running `cases serve` records itself in (pid, url, store) for `cases status`: `$XDG_STATE_HOME/cases/serve-<slug>-<hash>.json`, outside the store. `Running` treats a dead pid or an address that refuses connections as no instance.
- `internal/skill/` — the bundled agent skill. `SKILL.md` is the neutral source, embedded; one adapter per agent (`claude.go`, `codex.go`, `pi.go`) renders it and knows the agent's skill directory. `cases skill install|uninstall|show|list` in `cmd/cases/skill.go`.
- `internal/config/` — the TOML config file (`$XDG_CONFIG_HOME/cases/config.toml`). `Keys` is the allow-list of settings, each naming the flag it seeds. `Load` returns a `File` that is never nil, so `cases config` can report a broken file. `(*File).Resolver()` is a kong resolver; it yields nothing for a flag whose env var is set, so precedence is flag > env > file > default.
- `cmd/cases/` — kong CLI. One file per subcommand, each with a `_test.go` sibling. `Deps` carries the store path, the streams and the loaded config. `main` and `runCases` both load the config off the argv before building the parser, and a config error stops every command except the `config` subcommands (marked by `diagnosesConfig`).

## Conventions

- `apply` checks everything before changing the case, so a refused event leaves it unchanged. Load skips a refused or malformed file and adds it to `Case.Problems`; it fails only when there is no valid open event.
- The writer and the reader use the same `apply`, so anything the CLI writes will fold.
- Timestamps are UTC, set by the store when the record leaves them zero.
- `Event.Data` holds the file bytes as written, so unknown fields survive and appear in `show --json`.
- `[]string` flags whose values can contain commas need `sep:"none"`.
- htmx is vendored at `internal/web/static/htmx.min.js` and embedded. Never load it from a CDN. To upgrade: take `dist/htmx.min.js` from the npm tarball, check the tarball against the registry's integrity hash, and update the version, integrity and `htmxSHA256` in `views.go` (`TestStaticHTMXIsTheRecordedRelease` checks the file).
- Web pages have no inline scripts or styles, so the CSP holds. The only script files are the vendored htmx and our own `internal/web/static/prefs.js`, which applies the browser-stored preferences (`data-theme`, `data-face`, `data-links` on `<html>`) before first paint; add an option by extending its `OPTIONS` table, the CSS keyed on the attribute, and a fieldset in the options dialog in `layout.html` (there is no `/options` page; the overlay is a native `<dialog>` opened by `prefs.js`). Everything is escaped by `html/template`. The only `template.HTML` is goldmark output, with raw HTML left off (never `html.WithUnsafe`).
- Answers from the web go through the same store functions as the CLI. Do not add another way to write events. A web form that writes carries the case's `Revision()` in a hidden `revision` field, and its handler passes it back with `store.AtRevision`, so a form from a page older than the case gets 409 and writes nothing. The thread poll compares only the state: reloading on every new event would throw away what the human has typed, and the 409 already stops a stale send. Web tests send it with `withRevision(form, rev)` or read it off a page with `pageRevision`.
- `cases wait` wakes on a new human answer, park or resume. New means the file was not there on its first poll, or is later than `--since`. It exits 2 on timeout (`exitTimeout`).

## Testing

- Fold tests build events in memory with `fold(t, steps...)` in `internal/store/fold_test.go`. `TestTransitionMatrix` tries every event from every state against the lifecycle diagram. Add to it when the lifecycle changes.
- Store tests use `t.TempDir()`. `fixClock` pins the event clock; `link`, `rename` and `remove` can be swapped to fail or race a write.
- CLI tests call `runCases(t, stdin, args...)` in `cmd/cases/main_test.go`. It parses with the real kong grammar and captures stdout and stderr. `Deps.Poll` is 10ms in tests. `TestMain` points `HOME` and the XDG directories at a scratch directory, so tests never see the developer's config file or default store; `writeConfig(t, body)` in `config_test.go` puts a file at the default location for one test.
- Web tests (`internal/web`) use `newApp(t)` and `a.do(method, path, form, headers)` with `httptest`. Requests carry `Host: 127.0.0.1:8765` unless the test sets another.
- `e2e_test.go` runs a decision case through open, answer, wait, pickup and close.
