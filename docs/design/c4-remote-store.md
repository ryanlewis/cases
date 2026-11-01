# C4: the remote store

The design for C4 of the hosted plan (§3): `cases-store-remote`, a store over the hosted Worker's `/v1/stores/{id}/…` routes, the `--store` names that resolve through the account, and `cases stores`. Code comes after review.

Sources: the plan §2 ("The CLI"), §3 (C4, C5, C7) and §4 (deltas, licence); the README's decisions of 28 Sep (`eba6l`, `k8yKT`), 1 Oct (local and remote modes) and 3 Oct (`K6U5Q`); ST-11, ST-16, AC-01, AC-04 to AC-06, SV-01, HU-06 and TU-01 to TU-04; spec/api-agents.md; the hosted Worker's H5 and H6 designs and its store routes; the round-1 spike on the branch `feat/remote-store`.

## What changes

- A new crate, `crates/store-remote` (`cases-store-remote`). It implements the core's `Store` trait over HTTP and has no rusqlite.
- `cases-core` gains the poller, moved from `cases-store-sqlite` and made generic over any `Store`. Nothing in the trait changes (see "The trait").
- The CLI holds a store that is either local or remote, chosen once per command from `--store`, `CASES_STORE`, the config and `CASES_SERVER`.
- Config gets the keys SV-01 already names: `server`, `default-store` and `token-file`. The token is `CASES_TOKEN` or the token file. C5 adds the stored token from `cases login`.
- A new command, `cases stores`.
- `serve`, `status`, `inbox` and `service` each do something defined on a remote store, rather than failing (ST-11, as the 1 Oct entry asks).
- `deny.toml` allows ISC and CDLA-Permissive-2.0, for rustls and its root certificates (plan §4, decision 10).

C4 does not include `cases login` (C5), the listening wait with 300 s polls (C7), or the TUI across several stores (open question 6).

## The crate

**Name:** `cases-store-remote`, `publish = false` until v1, as `cases-store-sqlite` is. Hosted can take it as a git dependency to run the store vectors over HTTP (see "Testing").

**Dependencies:**

| Crate | Why |
| --- | --- |
| `cases-core` | the trait, the types, `Case::fold`, `Category::from_name`, `Error::new` |
| `ureq` 3, features `rustls` and nothing else that is optional | the HTTP client: blocking, small, TLS by rustls with the ring provider |
| `serde`, `serde_json` | the wire shapes |
| `webpki-roots` (through ureq) | the root certificates. CDLA-Permissive-2.0 (open question 3) |

Dev-dependencies: `cases-store-sqlite` and `cases-vectors` for the fake server and the vectors, `libtest-mimic` and `tempfile`, as the other store crates have them.

**Sync, not async.** The CLI is synchronous, and so is the trait: a client "can block on HTTP", as its docs say. The only tokio in the CLI is `cases serve`'s, and that leaves with the web split. ureq runs a request on the calling thread with no runtime. reqwest's blocking client starts a tokio runtime underneath, and hyper and tokio would come back into the lean CLI for no gain. The spike used ureq 3.4.2 and it worked against the real Worker.

**No rusqlite, checked by a machine.** A CI step fails if `cargo tree -p cases-store-remote -e normal` lists `rusqlite` or `tokio`. Dev-dependencies are left out of that tree, so the fake server can use store-sqlite.

**Its types:**

- `Server`: a checked base URL. It must be `https://host[:port]` with no path, query, user or fragment. Plain `http` is allowed only for `127.0.0.1`, `localhost` and `[::1]`, for tests. The scheme is matched in any case, so `HTTPS://…` is never taken for a file (spike lesson).
- `Token`: the bearer. It checks the shape before anything is sent: `csu_` or `csa_`, then 36 base62 characters. The server checks the CRC. `Debug` and `Display` never show it.
- `Client`: one ureq agent, with the server and the token. It reads the account: `whoami()` gives `GET /v1/cli/whoami` as `Whoami {account, token, stores}`.
- `RemoteStore`: one store, by its id, on a `Client`. It implements `Store<Key = Key, Writer = Writer, Error = Error>`. Beside the trait it has `changes(after, hold)`, the held poll (see "Polling").
- `Error`: what went wrong outside the core's refusals (see "Errors").

The crate never reads the environment, the config or a file. Finding the server, the token and the store id is the CLI's job, so the crate stays easy to test and to reuse.

## The trait

**C4 does not change the `Store` trait.** C1 already widened it for this step, and each method has a route (next section). What a remote store needs beyond the trait stays in the remote crate:

- **The held poll.** `GET B/store/changes?after=V&timeout=S` blocks until the version moves. A local store has nothing like it: `cases wait` reads it again every second. The parameters that make a poll listen (`wait`, `worker`, `label`, `kind`, `since`, `doing`) are hosted-only (ST-16). So the poll is an inherent method of `RemoteStore`, and the CLI calls it where it holds a remote store. C7 can decide whether a provided trait method, such as `follow(after, hold)` that a local store answers at once, is worth adding. A provided method is an addition, so it would not break `cases-store-sqlite` or `cases-web`.
- **`listen`.** On a remote store the held poll is the listening record. `RemoteStore::listen` records nothing and gives `Ok(())`, as the trait's docs allow.

**One addition to `cases-core`: the poller.** Today `Poller` lives in `cases-store-sqlite` and holds a `&Db`. Its logic is pure: ask `rows_since` for what changed since the last version, fold those cases again, and drop the removed ones. It moves into the core as `Poller<'a, S: Store<Key = Key>>`, and `cases-store-sqlite` re-exports it, so `Db::poller()` keeps its name. `wait` and the TUI then work on either store. This is a new public item, so `cases-core` goes from 0.1.0 to 0.1.1 and the semver check passes.

## Each method, as a request

B is `{server}/v1/stores/{store id}`. Every request sends `Authorization: Bearer <token>` and `User-Agent: cases/<version>`. Writes send `Content-Type: application/json`.

| Method | Request | Answer read as |
| --- | --- | --- |
| `create(open, writer)` | `POST B/store/cases` `{"writer", "append"}` | `{"key", "rows"}`, folded with `Case::fold` to give `(key, case)` |
| `append(key, append, writer, at_revision)` | `POST B/store/cases/{key}/events` `{"writer", "append", "revision"}` | `{"key", "rows"}`, folded |
| `rows(key)` | `GET B/store/cases/{key}` | `{"key", "rows"}`, or `None` on 404 `nocase` |
| `list()` | `GET B/store/cases` | `cases`, in key order |
| `archive(key, at_revision)` | `POST B/store/cases/{key}/archive` `{"revision"}` | `{}` |
| `delete(key, at_revision)` | `POST B/store/cases/{key}/delete` `{"revision"}` | `{}` |
| `version()` | `GET B/store/changes`, with no `after` | `{"version"}`, at once |
| `rows_since(since)` | `GET B/store/cases?since=V`, or no `since` | see below |
| `actors()` | `GET B/store/actors` | `ActorRow`s, in the order made |
| `listen(…)` | none | `Ok(())` |
| `set_doing(worker, text)` | `PUT B/store/doing/{worker}` `{"text"}` | `{}`, or `not-listening` |
| `waits()` | `GET B/store/waits` | `Wait`s (see open question 2 for `heard`) |
| `listeners()` | `GET B/store/listeners`, a new route (open question 2) | `Listening`s, with their waits |

Notes:

- **`writer`** is the writer's name, or `null` for the unnamed writer of that author, as the core's `Writer` gives it. On a remote store the human's events go as the unnamed human, which is the account's actor (open question 1).
- **`append`** is the core's `Append` as JSON: `author`, `event` and `data`, where `data` is the record's JSON text, sent byte for byte.
- **Revisions and stale writes.** `revision` is the `at_revision` the caller passed, or `null`. The server's object runs the core's writer under its own lock, so a stale write comes back as 409 `stale` and nothing is stored (AG-12). The client never checks a revision itself. It does not need to, and a check on the client would race.
- **`rows_since`.** The answer has `version` and `cases`, and `removed` only when the server knows what changed. The client reads it as:
  - `removed` present, `cases` and `removed` empty, and `version` equal to `since`: `Changes::Unchanged`;
  - `removed` present otherwise: `Changes::Since`;
  - no `removed`: `Changes::All`.

  This is how the hosted Worker writes them today. It is the deltas item of plan §4 (H8d's CLI side): a `wait` or a TUI that follows a store reads only the cases that changed.
- **The doing line** is checked with the core's `doing_line` before anything is sent, so a line that is not one is refused as the trait says: before whether the store exists or the worker listens. That also keeps the vectors' `invalid` category on the client, not in a 400 `request`.
- **Keys** go on the wire as numbers (`key.get()`). Case ids stay the CLI's: the `cases-id` encoding of the key.
- **A 1 MiB request cap.** The server refuses a body over 1 MiB. The client checks the size first and fails with a clear message, without sending.

## Errors

The client reads the status first, then the body's `error.category`. It never reads `error.message` to decide anything: the message is for people and is only ever printed.

| Status | Category | The client gives |
| --- | --- | --- |
| 200 | | the answer |
| 400 | `malformed` | `StoreError::Refused`, `Malformed` |
| 400 | `request`, or any other | `Error::Request`: the server refused the request itself, which is a bug in one of the two |
| 401 | any | `Error::Auth`, with the server's message to print |
| 404 | `nocase` | `StoreError::NoCase` for a write, `Ok(None)` for `rows` |
| 404 | `not-listening` | `Error::NotListening` |
| 404 | `request` | `Error::NoStore`: no store with this id |
| 409 | `stale` | `StoreError::Refused`, `Stale` |
| 410 | any | `Error::Gone`: the store was removed |
| 422 | `unknown`, `author`, `invalid`, `transition` or `no-open` | `StoreError::Refused`, that category |
| 429 | any | `Error::Rate`, with `Retry-After` when sent. Read by status, not category (N26) |
| 3xx | | `Error::Redirect`. No redirect is followed |
| 5xx | any | `Error::Server` |
| anything else, or a body that is not the API's | | `Error::Protocol` |

- A refusal becomes `cases_core::Error::new(category, message)`, so the CLI's exit codes work as they do on a local store: `stale` exits 4, `transition` exits 3, and everything else exits 1 (CL-01). Errors are compared by category, never by message, as the vectors' README says.
- `Error::Transport` covers no answer at all: DNS, connect, TLS, a timeout, or a body cut short.
- `Error::is_transient()` is true for `Transport`, `Rate` and 502, 503 and 504. A wait and a TUI use it (see "Retries").
- No error, `Debug` or log line holds the token. The URL in an error holds the server and the store id, never a query that could carry a secret.

## How `--store` resolves

The CLI picks the store once, before the command runs, in `crates/cli/src/store.rs`. The rules are ST-11's and SV-01's, with the plan's resolution of names:

1. **The text:** `--store`, else `CASES_STORE`, else the config's `store`. An empty value counts as unset, as today.
2. **A path** when the text has a `/` or ends in `.db`: a local store, as today, with a leading `~` expanded.
3. **A URL** when the text starts with `https://` (any case), or `http://` on loopback: a remote store for this one call. It is `https://host/<name or id>`. The server is the URL's, and the token is the one for that server (see "The token").
4. **Otherwise, with a server set** (`CASES_SERVER`, else the config's `server`): a store in the account. The text is a store id when it is `st_` and 16 base62 characters, and a name otherwise.
5. **Otherwise, with no server:** a local path, as today. So `--store work` never moves to a server unless a server is set.
6. **No text at all:**
   - with a server set: `default-store` from the config, else the account's default (from `whoami`). If the token opens exactly one store, that store. Else the command fails with exit 1 and lists the stores (AC-04). Nothing is guessed.
   - with no server: the default local path, as today.

A config `store` that is a path wins over a server, because it is an explicit choice (open question 5). `cases stores` and `config show` say so when both are set.

**Names to ids.** The server never looks a name up: the CLI turns it into an id from the account's list of stores.

- The list comes from `GET /v1/cli/whoami`. The CLI keeps it in the state directory as `accounts/<host>.json`: the account, the stores (`id`, `name`, `default`), when it was read, and the SHA-256 of the token it was read with. It holds no secret. A different token, such as a new `CASES_TOKEN`, reads the list again.
- A name the list doesn't have, or a 401 or 410 from a store named from the list, makes the CLI read the list once more. If the name now has a different id, it sends the request again: a 401 or 410 means nothing was read or written, so this is safe for writes too. If the name is still missing, it fails: "no store named X in account Y" (AC-04).
- A store given by id is never looked up. A 401 or 410 on it fails at once. This lets a script pin a store across renames (plan decision 3).
- C5 writes the same file at login, from the token answer, so the first command after login needs no `whoami`.

**What the CLI holds.** `Ctx` gets a `Target`, either `Local { path }` or `Remote { server, store: {id, name} }`, and a store that is `Local(Db)` or `Remote(RemoteStore)`. That enum implements `Store` by calling the one inside, with an error type that is either store's error. Commands use the trait, as they mostly already do. Code that needs the path (serve, status, service, the instance file) matches on `Target`.

### The token

C4 needs a token without `cases login`, for tests and for staging:

1. `CASES_TOKEN`, when set and not empty;
2. the file named by the config's `token-file` (SV-01). One line, trimmed. A file that others can read gets a warning to `chmod 600` it, as in the spike.

C5 puts the stored token (keychain or `credentials.toml`) between the two, as SV-01 orders them.

- The token is read only when a command reaches the store. `cases skill`, `cases config` and `--help` run with a server set and no token (spike lesson).
- A remote store with no token fails with exit 1: "no token for <server>: set CASES_TOKEN or the config's token-file". After C5 it says "run cases login <server>".
- For a test server, `token-file` points at a mode-600 file holding a minted token, and the CLI never prints it.

## `cases stores`

It lists the account's stores. It always reads `GET /v1/cli/whoami` afresh and saves the list for name resolution, so it is also how to refresh the list by hand.

```
account ryan (ac_…) on https://staging.cases.sh
token mbp (all stores), expires 2026-12-31

  NAME     ID                     DEFAULT
* me       st_4kQ9mZr2pX7wLc0t    account
  team-x   st_hV3kQ9mZr2pX7wLa
```

- `*` marks the store commands use now. The DEFAULT column says `account` for the account's default and `config` for `default-store`.
- A token scoped to some stores lists only those, and the token line names its scope (`team-x only`).
- When a config `store` path overrides the server, a last line says so, with the path.
- `--json` gives one object:

  ```json
  {"server": "https://staging.cases.sh",
   "account": {"id": "ac_…", "handle": "ryan"},
   "token": {"name": "mbp", "stores": null, "expires": "2026-12-31T10:00:00Z"},
   "stores": [{"id": "st_…", "name": "me", "default": true}, …],
   "selected": {"id": "st_…", "name": "me", "from": "account"}}
  ```

  `stores` is `whoami`'s list as the server gives it. `selected.from` is `flag`, `env`, `config`, `default-store`, `account` or `only`. `selected` is `null` when there is no single store to use.
- With no server set, it exits 1: "no server is set (CASES_SERVER or the config's server); the store is the local file <path>" (open question 9).
- A 401 exits 1 with the server's message. The token is never printed.

## The local-only commands on a remote store

ST-11 lists what needs a local store. On a remote store each does this instead:

| Command | On a remote store |
| --- | --- |
| `serve` | Starts nothing. On a terminal, it opens the hosted inbox (`{server}/`) in the browser, unless `--no-open`. Off a terminal, it prints the address. Exit 0. (`cases-web` stays local only.) |
| `status` | Prints the store (name and id), the server and the hosted inbox's address, with no request. Exit 0. |
| `inbox` | Opens the hosted inbox, or a case's page in it, `{server}/cases/{store id}-{store name}/{case id}-{title slug}` (IN-02). `--print` prints the address. |
| `service install` | Refuses with exit 1: "a remote store needs no local service; its inbox is at <address>". |
| `service status`, `service uninstall` | As today. They are about the installed unit, which names its own local store. |
| `prune`, `prune --delete` | Work, through `archive` and `delete`, with the revision guard. |
| `sweep` | Works: human withdraws at the revision it read. |
| `tui` | Works on the one store resolved (open question 6). |
| `wait`, `doing` | Work. Before C7 a wait is not listed as listening (open question 7). |
| `show --json`, `wait --full` | The case's `url` is the hosted case address above, not a local serve's. |
| `config path`, `show`, `init` | As today. `show` adds `server`, `default-store` and `token-file`, never the token. |

Every other command works the same on both stores.

## Polling

- **`version()`** is a poll with no `after`, which the server answers at once.
- **`changes(after, hold)`** is the held poll with no listening parameters. `hold` is in whole seconds, at most 25 in C4. A hold under a second asks for the version at once, so a deadline is not overshot (spike lesson). It answers the version as soon as it differs from `after`.
- **`wait`** on a remote store replaces its one-second sleep with `changes(version, min(25 s, time left))`, then reads `rows_since` when the version moved. A local wait is unchanged.
- **The TUI** on a remote store runs the held poll on a thread of its own and wakes the screen when the version moves, rather than reading once a second. TU-04's "about once a second" is about how fast changes show, and a held poll shows them within about a second. Reading every second would cost about 86,000 requests a day for each open TUI, which is most of the included Durable Object requests on Workers Paid.
- **C7** adds the listening parameters, the 300 s hold capped by `--timeout`, and `doing` on a wait's first poll.

## Timeouts, retries, redirects and limits

- **Timeouts.** Connect: 10 s. Any other request, start to end: 30 s. A held poll: its hold plus 15 s, which covers the front's 5 s grace and the network.
- **Redirects.** None are followed, and a 3xx is an error. ureq would drop the token on a redirect and turn a write into a GET, so a command could report a write that never happened (spike lesson).
- **Answer size.** At most 256 MiB, well above ureq's 10 MB default, since `list` sends every live case in one answer (spike lesson). Over that, the command fails with a message naming the limit. Deltas keep most reads small.
- **Retries of reads.** A GET that fails with a transient error is tried again up to twice, after 0.5 s and 2 s, within the command's time.
- **Retries of writes.** None, except after a 429. A write that got no answer may or may not have been stored. The CLI says so and suggests `cases show`, then exits 1. An open has no revision, so a retried open could make two cases. Writes can retry once the `Idempotency-Key` header (a pre-public item) exists.
- **429.** Nothing was done, so any request may be sent again. The client waits for `Retry-After`, at most 10 s and never past the command's deadline, then tries once more. Otherwise it fails: "the server is limiting requests; try again in N s".
- **Offline.** A server that can't be reached fails the command with exit 1, naming the server and the reason. There is no local copy of a remote store and no queue of writes (ST-11: no sync). A wait that has read the store once rides out transient errors until its deadline and says so once on stderr (spike lesson; this lands with C4's wait change). A wait that fails its first read fails at once, so a wrong store isn't waited on. The TUI shows a line and tries again (TU-01).
- **Proxies.** ureq's default proxy settings from the environment (`HTTPS_PROXY`, `NO_PROXY`) are kept. The build checks this.

## What the spike taught, and what is kept

The spike (`feat/remote-store`, 4 commits) predates the trait, store ids and accounts. Its code is not carried over. What it learned is kept:

| Spike lesson | In C4 |
| --- | --- |
| Refuse redirects | kept |
| Lift the body limit to 256 MiB | kept |
| Match the URL scheme in any case | kept |
| Retry a wait's failed read after its first one, and fail at once before | kept, for wait and the TUI |
| Read the token only when a command needs the store | kept |
| Warn on a token file that others can read | kept |
| A hold under 1 s asks at once | kept |
| A fake of the store routes over a real SQLite store, with plain HTTP/1.1 on loopback | kept, and widened to run the vectors |
| An end-to-end script against a deployed Worker | kept, opt-in, against staging |
| `--store https://host/<store name>` as the way to name a store | changed: names go through the account; a URL is a one-off |
| 10 s polls so a heartbeat could beat between them | dropped: a remote wait never heartbeats (ST-16) |
| `PUT waits/{id}` to listen | dropped: the held poll is the record |
| A `Backend` enum with its own poller | replaced by the enum over `Store` and the core's generic poller |

## Testing

**Unit tests in the crate:** the server URL check; the token's shape and that `Debug` hides it; each row of the error table; `rows_since`'s three answers; the request-size check; that a 3xx is refused.

**The fake server** (`crates/store-remote/tests/fake/`): plain HTTP/1.1 on `127.0.0.1:0`, one thread per connection, with no new dependencies. Behind it is a `cases-store-sqlite` `Db` for each store id, with a clock the test sets. It speaks api-agents.md: the store routes, `whoami`, the token check, the held poll (a condition variable on the version), and the error shapes. It can also be told to answer 503, 429 with `Retry-After`, 302, a slow answer, an oversized body, a revoked token (401) or a removed store (410). It keeps every request line, so a test can check what went on the wire, and that the token was only ever in the header.

**The store vectors over HTTP.** `crates/store-remote/tests/store_vectors.rs` implements `cases_vectors::Harness` for `RemoteStore` against the fake:

- `store()` is a new store id on the fake, with an empty `Db`;
- `set_clock` sets the fake's clock;
- `category` maps `Error::Doing` to `invalid` and `Error::NotListening` to `not-listening`;
- `LOCAL` is `false`, so the heartbeat vectors show as ignored. The version, changes, actors and doing-line vectors run.

**The core vectors through the store.** As `cases-store-sqlite` does in `tests/vectors.rs`: each append goes through `RemoteStore`, so every refusal's category is checked after a trip over HTTP. Each replay writes its rows into the fake's `Db` directly and reads them back over HTTP, so a record's `data` must come back byte for byte, escapes and all.

**CLI tests** (`crates/cli/tests/remote.rs`, the binary against the fake):

- exit codes 4, 3 and 1;
- each step of the resolution: path, URL, name, id, `default-store`, the account's default, only one store, and several with no default;
- a renamed store found again after a 401;
- `cases stores` as text and JSON;
- each local-only command's remote behaviour;
- `skill` and `config` with no token;
- that no output or error holds the token.

**Mutants.** `cases-store-remote` joins the `cargo mutants` run. Survivors get a test, or an exclusion in `.cargo/mutants.toml` with the reason.

**End to end, opt-in.** `scripts/e2e-remote.sh`, from the spike's script, runs the built binary against a real server: open, wait with `--pickup` while a stand-in human answers through the inbox route, close, the exit codes for stale, transition and a wrong token, and `cases stores`. It needs `CASES_E2E_SERVER` and a token file. It never runs in CI. Ryan runs it against staging. Agents don't.

**Later, in hosted:** the same harness, pointed at `wrangler dev` with a minted token, would close hosted's follow-up "the store vectors could run through `cases_vectors::Harness` over HTTP". That needs a test-only clock route in the test build. It isn't part of C4.

## Spec and vector changes

In the vault, as one spec PR before the CLI wiring lands:

- **ST-11:** the list of local-only commands becomes the table above, saying what each does on a remote store, as the 1 Oct entry asks. The resolution steps gain the store id and the one-off URL with a name or id.
- **SV-01:** the token order for C4 (`CASES_TOKEN`, then `token-file`), with C5's stored token between them later. The list of stores kept in the state directory.
- **AC-04:** a store given by id is never looked up; a 401 or 410 on a name reads the list once and sends again.
- **HU-06:** on a remote store the CLI writes the human's events as the unnamed human, the account's actor (if open question 1 is accepted).
- **TU-04:** on a remote store the TUI follows by the held poll.
- **api-agents.md:**
  - `serve`, `status`, `service` and `inbox` "need a local store" becomes a pointer to ST-11's table;
  - `GET B/store/waits` gains `heard`, and `GET B/store/listeners` is added (if open question 2 is accepted);
  - the three shapes of `GET B/store/cases?since=V` are written out as the client reads them.

**Vectors:** none change. The core and the store vectors already cover what a remote store must do, and the remote store runs them. So `spec/vectors/CHANGES.md` needs no line. If review adds a vector, it gets its line then.

## Semver

- **`cases-core`: 0.1.0 to 0.1.1.** The poller is a new public item and nothing is removed or changed, so `.github/scripts/semver-check.sh` passes.
- **`cases-id`:** unchanged.
- **`cases-store-sqlite`:** its `Poller` becomes the core's, re-exported under the same name. It isn't published, so there is no semver check, but `cases-web` will link it after the split. The poller's methods keep their names and signatures.

## Size and the PRs

About 3,500 lines with tests: the crate about 900, the fake about 500, the CLI about 900, and tests about 1,200. In order:

| PR | What | Size |
| --- | --- | --- |
| S | The spec lines above, in the vault | small |
| H | If open question 2 is accepted: `heard` on `store/waits` and `GET B/store/listeners` in hosted | small |
| C4a | `cases-core`: the generic poller, moved from store-sqlite and re-exported. No change in behaviour | small |
| C4b | `cases-store-remote`: client, `Store`, errors, the fake, the store and core vectors over HTTP; the licences in `deny.toml`; the CI check for rusqlite and tokio | large, but one crate with no CLI change |
| C4c | The CLI: config keys, `CASES_SERVER` and `CASES_TOKEN`, resolution and the list of stores, the store enum in `Ctx`, every store command on a remote store, `url`, and the local-only commands' remote behaviour | large |
| C4d | `cases stores`, text and `--json` | small |
| C4e | Following a remote store: wait's held poll and retries, the TUI's poll thread; `scripts/e2e-remote.sh` | medium |

C4c could split in two if it reviews badly on a phone: resolution and config first, then the commands. C5 can start once C4c is in.

## Open questions

1. **Who the human is on a remote store.** The hosted inbox writes the human's events as the unnamed human, and HU-06 says on a server the human is the account. Recommend: on a remote store the CLI does the same and ignores `--as`, the config's `name` and `$USER` for the actor. `withdraw --as NAME` still needs its name, to mark the withdraw as the human's. The alternative, names as on a local store, would make one person into several actors in a hosted store.
2. **The listeners and waits routes.** `GET B/store/waits` has no `heard`, which the core's `Wait` needs, and no store route gives the core's `Listening` with its waits. Recommend a small hosted PR: add `heard` (when the poll began or last reconnected) to `store/waits`, and add `GET B/store/listeners` in the core's shape. The alternative is a client that sets `heard` to `since` and builds each listener's waits from a second read. That costs a request and can disagree with itself.
3. **Root certificates.** Recommend `webpki-roots`, bundled, as the licence decision assumed. A machine behind a proxy that intercepts TLS would need the OS's trust store instead (`rustls-platform-verifier`). Add that only if a machine needs it.
4. **Where the list of stores is kept.** Recommend the state directory, `accounts/<host>.json`, keyed by the token's SHA-256. C5 writes it at login. The alternative, `whoami` on every command, adds a request to every command an agent runs.
5. **A config `store` path with a server set.** Recommend the path wins, as ST-11's order gives it, and `cases stores` and `config show` say so. The alternative, letting the server win, would quietly move a store that was a file.
6. **The TUI and several stores.** The 1 Oct entry says the TUI in remote mode reads the account's stores, and herdr's count is across them. TU-01 says the TUI takes the store as every command does. Recommend: C4 gives the TUI one store, the one resolved. A TUI across stores comes later, with the same store filter and badge as the SPA in C6. The "waiting on you" count across stores comes with it.
7. **A wait on a remote store before C7.** Recommend: C4's wait uses held polls of up to 25 s without listening. It takes messages as it does locally, but it isn't shown as listening and `cases doing` is refused until C7. The alternative is to bring C7's listening into C4. That makes C4e larger, and C7 is already its own step.
8. **The one-off URL.** Recommend `https://host/<name or id>`, using the token for that host. The alternative is to drop URLs, now that names and ids cover a set server. A URL is still useful for one call to staging while the config points at production.
9. **`cases stores` with no server.** Recommend exit 1 with the reason and the local path. The alternative is exit 0, with the local file as the only "store".
10. **The trait.** Recommend no change in C4: the poller moves into the core as an addition, and the held poll stays in the remote crate. If C7 wants a `follow` method, it can be a provided method, which breaks nothing.
11. **Publishing the crate.** Recommend `publish = false` until v1, as store-sqlite is. Hosted takes it as a git dependency if it runs the vectors over HTTP.

## As built

C4 landed in five pull requests: #106 (the poller in the core), #107 (`cases-store-remote`), #108 (the CLI on a remote store), #109 (`cases stores`) and #110 (following a remote store, and the end-to-end script). On the hosted side, hosted#24 added `heard` to `store/waits` and the `store/listeners` route (open question 2). The spec lines went into the vault before the CLI wiring. Where the build differs from the text above:

- **The poller has a second constructor.** `Poller::with_read(store, read)` takes the read to use in place of `rows_since`. A local store uses it with its own read, which gives an error for a store that isn't made yet, where `rows_since` gives no cases. Without it, `serve`'s notifications, the TUI and `wait` would change how they treat a missing store. `Poller::version()` gives the version a read was taken at, which the held poll starts from. Both are additions; `cases-core` is at 0.1.1, not yet released.
- **The crate's errors.** A bad doing line or a blank worker is `Error::Invalid`, not `Error::Doing`, so a blank worker never reaches `PUT …/doing/` and comes back as a missing store. Only status 200 is an answer: the Worker answers every success with 200, so 201, 204, a 409 that isn't `stale` and a 422 outside the five categories are `Protocol`.
- **429.** A `Retry-After` over 10 s fails at once with `Rate`, rather than waiting 10 s. With no `Retry-After` the client waits 1 s. On a read, 429 gets its one wait and the other transient failures keep their two retries.
- **Deadlines.** The crate can't see a command's deadline, so `Client::with_deadline` gives it one: a retry or a 429 wait that would end past it isn't made, and each request is cut to the time left, with a 1 s floor. The CLI sets it from `--timeout`.
- **Waits listed twice.** A client that reconnects before its old socket closes can hold one wait id twice, so the server can list it twice. The client keeps one wait per worker and id: the one heard last.
- **`listen`** records nothing and checks nothing on a remote store.
- **`doing` on a remote store** is refused until C7, as the spec says, rather than working as the table above had it. The server answers `not-listening`, so the message is the local one.
- **Resolution.** A URL is checked for before a path, since a URL has a `/`; a bad address is refused rather than taken for a file. A `401` on a stale default is treated as on a name: the list is read once more. The list is read again at most once per command. Before C5, a one-off URL uses the same `CASES_TOKEN` or token file whatever its host.
- **Commands that never reach a store** (`config`, `skill`, `serve`, `service`, `stores`) need no token and no lookup. `serve` on a remote store prints the hosted inbox's address on a terminal too, then opens it unless `--no-open`. `status` calls `whoami` once when it has neither a kept list nor an id.
- **The fake server** lives in `crates/store-remote/tests/fake/` and `crates/cli/tests` include it by path, so there is no test-only crate or feature.
- **`cases stores`.** An expiry shows as its date; a token with none shows "does not expire". When a path wins over the server, `selected` is `null` in the JSON.
- **Following.** `wait` holds polls of up to 25 s, sleeping out the last second rather than polling it. After a first read that worked, it rides out transient errors with a pause from 1 s to 15 s, said once on stderr. The TUI's poll runs on a thread it never joins, so quitting doesn't wait on a held poll, and a pty test pins that `q` quits within 2 s. A local store is still read once a second.
- **End to end.** `scripts/e2e-remote.sh` runs against a server given by `CASES_E2E_SERVER` and `CASES_E2E_TOKEN_FILE`, refuses to run when `CI` is set, and prints no token. On 3 Oct `cases stores`, `open` and `withdraw` from main worked against staging, and `store/listeners` and `store/waits` answered as the spec says.
