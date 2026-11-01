# cases (Rust)

cases, a human-in-the-loop inbox for agents, in Rust. The spec below is the reference for how it behaves. This repo is public.

## Scope

Work goes into the local CLI: the commands, the TUI, a queryable API and the built-in web inbox (`serve`), as they are usable today.

Self-hosting over ssh is in scope: `forward = "ssh://host"` or `CASES_FORWARD` sends every command's store calls over `ssh host cases dial-stdio` to the one store on that machine.

Hosted is parked. Keep `cases login` and the hosted half of `store-remote` building and passing their tests, but don't extend them.

## Workflow

- Commit to a branch, raise a PR, and merge on green. Conventional Commits.
- PRs are squash-merged. The PR title becomes the commit subject on main, so it must be a Conventional Commit subject (CI checks it with `.github/scripts/check-pr-title.sh`). The PR body becomes the commit body: write it as a commit message, saying what changed and why.
- Pin dependencies. Pin GitHub Actions to full SHAs with a `# vX.Y.Z` comment. Renovate bumps them.
- `permissions: contents: read` at the top of every workflow.

## Spec

- User stories: `~/dev/notes/projects/cases/spec/stories.md` (keep/change/drop, with acceptance criteria at the level of behaviour).
- Core vectors: `spec/vectors/` (format in its README.md). `core` must pass every vector. Errors are compared by category, never by message.
- A vector change needs a line in `spec/vectors/CHANGES.md` naming the vector by its description; CI refuses a pull request without one.
- Decisions: `~/dev/notes/projects/cases/README.md`, in the section on building cases in Rust.

## Layout

- `crates/core` (`cases-core`): pure. No IO, no clock, no SQLite. It is published as its own crate with its own version (`core-vX.Y.Z` tags). Keep it `no_std`-clean where that is cheap.
- `crates/id` (`cases-id`): case ids, the Sqids encoding of a case's numeric key. `no_std`, no dependencies, published like the core. Ids are a permanent contract, and its tests pin them.
- `crates/store-sqlite` (`cases-store-sqlite`): the local store in one SQLite file. It implements the core's `Store` trait, with the actors table, archive and delete, and the poller.
- `crates/store-remote` (`cases-store-remote`, not published): the store protocol, the store routes of the agent API over HTTP (ureq). It implements the core's `Store` trait as `RemoteStore`, with no rusqlite or tokio in its normal dependencies (CI checks). It reaches a hosted server over HTTPS, or a forwarded store over a pipe to a child process (`Dial`, `Client::dial`). Its `hosted` feature, on by default, adds the sign-in, whoami, revoke and TLS (rustls); the cli takes the crate without it, and turns it on with its own `remote` feature, off by default and in a release; CI tests both. Its tests run the store and core vectors over HTTP against a fake server backed by store-sqlite, which `crates/cli/tests` share by path.
- `crates/vectors` (`cases-vectors`, not published): the runner of the store vectors (`spec/vectors/store/`). A store implements its `Harness` in its tests, as core (a store in memory), store-sqlite and store-remote do.
- `crates/cli`: the `cases` binary and the web inbox (`serve`). `dial.rs` serves the store routes against the local store for the hidden `cases dial-stdio`, the far side of a forwarded store; `tests/forward*.rs` run every vector, and the commands end to end, through it. The inbox's own modules are TypeScript (`src/commands/serve/web/static/*.ts`). `build.rs` strips their types with oxc, so the build needs no node; `tsc` checks them in CI, with the types for preact, htm and marked from `web/package.json`, pinned to the vendored versions.
- The hosted server, a Cloudflare Worker, lives in the private `ryanlewis/cases-hosted`. It is parked.

## Commands

```
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo deny check
cargo mutants -p cases-core -p cases-id -p cases-store-sqlite -p cases-store-remote
.github/scripts/semver-check.sh origin/main
.github/scripts/check-vector-changes.py origin/main
(cd crates/cli/src/commands/serve/web && npm ci --ignore-scripts && npx tsc --noEmit -p tsconfig.json && npm test)
```

`cargo mutants` mutates the core, the ids and the stores to find gaps in its tests; add a test that kills a survivor, or exclude it in `.cargo/mutants.toml` with the reason. The semver check fails on a breaking change to the public API of `cases-core` or `cases-id` until that crate's version is bumped. Install the tools with `cargo install --locked cargo-deny cargo-mutants cargo-semver-checks`.
