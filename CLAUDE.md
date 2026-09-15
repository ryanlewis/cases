# cases

CLI for a store of cases raised by an agent and answered by a human. The store is a directory with one directory per case and one JSON file per event (`NNNN-<author>-<event>.json`). State is the fold of those files in name order. The runtime is the Go standard library plus kong.

## Workflow

- Personal repo, no remote yet. Commit to `main` locally.
- **DO** use Conventional Commits.
- **NEVER** edit or delete event files in a store. Changes to the record format must keep reading files written by earlier versions.
- **DO** update README.md in the same change when a command, flag, event or record field changes.
- `serve` and the web views are a later phase. Do not add them alongside unrelated work.

## Commands

CI runs the same checks.

```
make build   # go build -o cases ./cmd/cases
make install # go install ./cmd/cases
make test    # go test -race ./...
make cover   # coverage summary
make lint    # golangci-lint run ./... (v2 config in .golangci.yml)
make fmt     # gofmt -w . && goimports -w .
```

## Architecture

- `internal/store/` — the store.
  - `types.go` — kinds, urgencies, states, authors, event types, and one record struct per event.
  - `fold.go` — `Load(dir)` folds a case directory into a `Case`. `(*Case).apply` is the only place transitions and answer shapes are checked.
  - `write.go` — `Create` and one append function per event. Each append locks the case directory, loads it, runs `apply` on the new event, and only then writes the file atomically (temp file plus rename).
  - `list.go` — `List(root)` and `Poller`, which reloads only case directories whose mtime changed. `cases wait` uses it.
  - `lock_unix.go` — `flock` on the case directory. It stops two local writers taking the same sequence number; it does nothing across machines.
- `cmd/cases/` — kong CLI. One file per subcommand, each with a `_test.go` sibling. `Deps` carries the store path and the streams.

## Conventions

- `apply` checks everything before changing the case, so a refused event leaves it unchanged. Load skips a refused or malformed file and adds it to `Case.Problems`; it fails only when there is no valid open event.
- The writer and the reader use the same `apply`, so anything the CLI writes will fold.
- Timestamps are UTC, set by the store when the record leaves them zero.
- `Event.Data` holds the file bytes as written, so unknown fields survive and appear in `show --json`.
- `[]string` flags whose values can contain commas need `sep:"none"`.
- `cases wait` exits 124 on timeout (`exitTimeout`).

## Testing

- Fold tests build events in memory with `fold(t, steps...)` in `internal/store/fold_test.go`. `TestTransitionMatrix` tries every event from every state against the lifecycle diagram. Add to it when the lifecycle changes.
- Store tests use `t.TempDir()`. `fixClock` pins the event clock; `rename` can be swapped to fail a write.
- CLI tests call `runCases(t, stdin, args...)` in `cmd/cases/main_test.go`. It parses with the real kong grammar and captures stdout and stderr. `Deps.Poll` is 10ms in tests.
- `e2e_test.go` runs a decision case through open, answer, wait, pickup and close.
