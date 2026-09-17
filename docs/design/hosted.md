# cases beyond one laptop

Status: proposal, 2026-09-17. Direction set on case
`2026-09-17T01-14-47Z-explore-a-hosted-cases-for-teams` (option 3).

The same `cases` CLI should work three ways:

1. **Local**, as today: the store is a directory.
2. **Self-hosted**: you run a cases server and point the CLI at it.
3. **Hosted**: a paid service with accounts and teams. You `cases login`, the
   CLI talks to its API, and the web inbox is the same inbox with login and
   team switching.

This document says what the code already gives us, what is missing, the
seams to cut, and an order of work where each step is useful on its own. It
also answers two questions raised on other cases: whether to write a schema
version, and whether to leave htmx.

## 1. What we have today

These hold up in a server and should not change.

| Part | Where | Why it matters remotely |
| --- | --- | --- |
| Append-only log, one JSON file per event, state is the fold | `internal/store/fold.go` (`Load`) | Nothing is updated in place, so a server can store, replicate, back up and export events as they are. |
| One write path: every append runs `apply` before writing | `internal/store/write.go` (`appendEvent`) | A server that calls the same functions refuses the same events as the CLI. An old client cannot write an event the server does not accept. |
| Revision guard | `Case.Revision()`, `store.AtRevision`, `ErrStale` | This is already optimistic concurrency. It maps onto HTTP `ETag`/`If-Match`. |
| `Poller` reloads only changed case directories | `internal/store/list.go` | Keeps `wait` and `serve` cheap on a directory. A server needs a push feed instead (section 6). |
| Unknown fields survive a read | `Event.Data` keeps the bytes | New fields, such as identity, can be added without breaking old readers. |
| Request guard and strict CSP on serve | `internal/web/web.go` (`guard`, `CheckLoopback`) | Host check, cross-site POST refusal and `default-src 'self'` carry over. Only the loopback rule has to become configurable. |
| Bundled skill | `internal/skill/SKILL.md` | Agents learn the CLI, not the store. If the CLI surface stays the same against a URL, the skill barely changes. |

## 2. What modes 2 and 3 need

- **Identity.** File names say `agent` or `human` (`eventFile` in
  `fold.go`), nothing more. There is no record of which human answered,
  which agent token wrote, or who a case is for. `worker` on open and `by` on
  pickup are free text.
- **Auth.** Serve refuses any non-loopback address (`CheckLoopback`), and
  its only defence is that nobody else can reach it.
- **A store that is not a local directory.** Every command calls
  `store.Load`, `List`, `Create`, `Answer` and friends on a path.
  `sweep` and `prune` also use `Case.Dir` directly
  (`cmd/cases/sweep.go`, `cmd/cases/prune.go`).
- **Remote `wait`.** `wait` polls the filesystem every second
  (`cmd/cases/wait.go`).
- **Id collisions.** An id is the open time to the second plus a title slug
  (`Create` in `write.go`). The `-2` suffix only helps on one machine. Two
  machines syncing one folder can create the same id in the same second, and
  the lock (`lock_unix.go`, `flock`) does not stop two machines taking the
  same sequence number. With a server as the only writer, both problems go
  away: the server mints ids and takes the lock.
- **Backups and retention.** A laptop directory has none beyond what the
  user does. `cases prune` archives or deletes; that is the retention tool.
- **Multi-tenancy.** Nothing today knows about more than one store per
  process.
- **Deleting content.** An amend cannot erase a secret (`SKILL.md` says so).
  On a hosted service someone will paste a secret. Deleting a whole case works
  with an append-only log; editing one event does not.

## 3. The store interface

Add one interface that the CLI and the web app talk to. The directory code
stays as it is, behind it.

```go
type Store interface {
    List(ctx, Filter) ([]*Case, []*LoadError, error)
    Get(ctx, id string) (*Case, error)
    Open(ctx, OpenRecord, ...WriteOption) (*Case, error)
    Append(ctx, id string, ev EventType, rec record, ...WriteOption) (*Case, error)
    Watch(ctx, after Cursor) (<-chan Change, error)
}
```

- `WriteOption` carries `AtRevision(rev)` and, later, an idempotency key.
- `Append` replaces the nine functions at the seam. Inside the directory
  implementation they stay; `Append` dispatches to them.
- `Watch` is the `Poller` diff locally and the event feed remotely.
- **Directory implementation**: today's `internal/store`, wrapped.
- **HTTP implementation**: a client for section 4.
- `--store` takes a path or an `https://` URL; the CLI picks the
  implementation. `Deps.Store` becomes the interface.
- `sweep` and `prune` stop touching `Case.Dir`. `sweep` becomes a loop of
  `Append(withdraw)`. `prune` is storage housekeeping, so it stays
  directory-only; a server does retention itself.

The user has chosen to add `--revision` to all agent write commands. That
lines the two contracts up: every write can carry the revision it was based
on, locally as `AtRevision`, remotely as `If-Match`.

**Must not change:**

- The event file name `NNNN-<author>-<event>.json` and the record fields.
  Old installs must keep reading stores written by new ones.
- `apply` as the only place transitions are checked. The server calls it; it
  does not get its own rules.
- A server's store can be dumped as the same directory layout. That is the
  backup, the export and the way off the hosted product.

## 4. The HTTP API

Plain JSON over HTTPS, versioned by path.

| Method and path | Does | Preconditions and errors |
| --- | --- | --- |
| `GET /v1` | server version, known kinds and events | none |
| `GET /v1/cases?state=&label=&worker=` | list | none |
| `POST /v1/cases` | open; server mints the id | `Idempotency-Key` header, so a retried open does not make two cases |
| `GET /v1/cases/{id}` | the case, as `show --json` | response has `ETag: "<revision>"` |
| `POST /v1/cases/{id}/{event}` | append `answer`, `pickup`, `note`, ... | optional `If-Match: "<revision>"`; `412` when stale, `409` when `apply` refuses, `422` when the record is invalid |
| `GET /v1/events?after=CURSOR` | the feed (section 6) | none |

- **Revision as `If-Match`, not a body field.** It is a precondition on the
  resource, not part of the event, and `ETag` from `GET` gives it for free.
  Browser forms cannot set headers, so the web inbox keeps its hidden
  `revision` field and its `409` (`internal/web/views.go`). Both end at
  `store.AtRevision`.
- **Retries.** A write sent with `If-Match` is safe to retry: the second try
  gets `412`, and the client can read the case and see its event there.
  Without a revision, a retried `note` after a timeout could land twice. This
  is a second reason for `--revision` on agent writes.
- **Error text.** Refusals carry the message `apply` produced, so the CLI
  prints the same words for a local and a remote store.
- The server stamps identity and timestamps. Client-sent values for those are
  ignored.

## 5. Identity and auth

### Recording identity

Add optional fields to the records; do not change file names.

- On every event: `actor`, an object such as
  `{"id": "u_123", "name": "Ryan", "kind": "human"}`, set by the server from
  the credential. Locally the CLI can fill it from the config file, or leave
  it out. Not `by`: `PickupRecord` already has `by` as a string
  (`internal/store/types.go`), and an object there would make old binaries
  refuse every new pickup as malformed.
- On `open`: `for`, a user or team a case is addressed to.
- Old binaries ignore both fields when folding and keep them in
  `show --json`. No fold rule depends on them.

Authorisation (who may answer a case that is `for` someone) belongs in the
server, not in `apply`. `apply` stays about the lifecycle, so a local store
never has to know about users.

### Mode 2: self-hosted

`cases serve` grows two settings: a public host name, and a trusted identity
header. With both set it may listen beyond loopback.

- **Browser:** a proxy in front authenticates the user and sets a header.
  On the user's exe.dev VMs this exists already: the edge proxy asserts
  `X-ExeDev-Email` and `X-ExeDev-UserID` for browser traffic and strips any
  copy the client sends. The same setting works behind oauth2-proxy or a
  similar forward-auth proxy.
- **CLI and agents:** `Authorization: Bearer <token>`. The exe.dev edge
  passes a plain `Authorization` header through. Tokens are made on the
  server (`cases token create --name laptop-agent`), stored hashed, named
  (the name is the agent identity) and revocable.
- **The trap:** identity headers are only true for traffic that came through
  the proxy. A peer on a tailnet, or anything that reaches the port directly,
  can forge them. So the server must accept the header only on the listener
  the proxy uses, and bearer tokens everywhere else.

exe.dev is one deployment target for mode 2, not the design. The design is
"trusted header from a proxy, or bearer token".

### Mode 3: hosted

- Accounts with sign-in through an identity provider (GitHub or Google
  first) or an emailed link. A session cookie: `HttpOnly`, `Secure`,
  `SameSite=Lax`. The existing `Sec-Fetch-Site` and `Origin` checks stay as
  the CSRF defence.
- `cases login` uses the OAuth device flow and writes a token to the config
  directory with mode `0600`.
- Agent tokens are scoped to one team and named, as in mode 2.
- Orgs have members; a team is a store. Switching team in the web app, or
  `cases login --team`, switches store.

## 6. `wait` against a remote store

**Recommendation: Server-Sent Events, with long-poll on the same URL.**

- The server gives each accepted event a store-wide cursor once its file is
  written, and publishes it to subscribers in-process.
- `GET /v1/events?after=CURSOR` streams the case id and the event's `seq`,
  `author`, `event`, `file` and `at`, plus kind, urgency and title for an
  open. `Last-Event-ID` resumes.
- With `Accept: application/json` the same URL holds the request until there
  is an event or 30 seconds pass, then returns. That covers proxies that
  buffer streams.
- If the server restarts and the cursor is from before, it says `reset`. The
  client re-lists, which is what `wait` already does on its first poll with
  `seen` and `--since`.
- `wait` keeps its exact output and exit codes. Only the source changes.
- The web inbox can use the same feed in place of its 2-second poll later.
  It does not have to.

Not websockets: traffic is one way, and SSE is plain HTTP that the Go
standard library serves with `http.Flusher`, with no new dependency.

Notifications (`docs/design/notifications.md`, written alongside this one)
agree on this split:

- Routing runs where events are detected: the `Poller` diff in `serve`
  locally, the after-write hook on a server. A sink is an in-process
  subscriber, not an API client polling.
- The webhook sink posts the same JSON locally and remotely (the event plus
  the case as `show --json`), so it survives the move unchanged. The exec
  sink is local only.
- A sink remembers what it sent by case id, event file and sink. Event file
  names are stable in both modes, so the key works for both.
- The browser tab's notification endpoint serves the same items the feed
  carries; only the transport changes. Notifying with no tab open (Web Push)
  is left to the hosted product.

## 7. Record format versioning and releases

The question, from case
`2026-09-17T00-34-48Z-reword-unknown-event-as-version-skew-not`: is it worth
writing a schema version at all, once there is a release strategy?

**Today's rules:** unknown fields are kept and ignored; an unknown event file
is skipped and listed as a problem; an unknown kind or urgency fails the whole
case (`OpenRecord.validate` in `validate.go`). Changes so far have been
additive: `amend` and `question` came in this way, with README notes on skew.

**What a per-event version would buy.** Very little. It only helps when an
existing field changes meaning, and the rule "never edit event files, keep
reading old ones" already forbids that. New meaning gets a new field or a new
event. An old binary reading a new version number can do no more than it does
with an unknown event: skip it and say so.

**What a per-store version would buy.** One real thing, on shared local
stores: an old binary could refuse to *write* to a store it cannot fully
read. Today an old binary can answer an amended case against the options it
knows. That is the actual skew hazard. But it only matters for mode 1 with a
synced folder, and it can be added later as a marker file
(`.cases.json` with `"min_version"`) without touching any event.

**Client and server of different versions.** In modes 2 and 3 the server is
the only thing that folds and writes. An old CLI cannot write a bad event,
because the server runs `apply`. What the CLI needs is to know what the
server speaks: `GET /v1` returns the server version and the kinds and events
it knows. The CLI says "update cases" when the server knows an event or kind
it does not. The path version (`/v1`) changes only for a breaking API change,
not for new events.

**Release strategy.** Today there are no tags; `version` is set by
`-ldflags` and defaults to `dev` (`cmd/cases/main.go`). Before mode 2:
semver tags, release binaries from CI, and a skew message that names the
version to install.

**Recommendation.** Do not write a schema version into events. Build the
reworded skew messages already chosen. Add `GET /v1` with capabilities when
the HTTP store lands. Keep the store marker as a tool for a future breaking
change, and do not build it until one is needed.

## 8. The web UI stack

The question, from case
`2026-09-17T00-34-49Z-confirm-what-was-recorded-after-an-answer`: move from
htmx to something more malleable for transitions and richer UX?

**What we have.** Server templates, six `hx-` attributes across
`inbox.html` and `case.html`, a 99-line `prefs.js`, and one stylesheet. The
CSP is `default-src 'self'` with no inline script or style.

**Transitions do not need a client app.** The View Transitions API animates
between full page loads with CSS alone
(`@view-transition { navigation: auto; }` plus `view-transition-name` on the
case card and the thread). htmx swaps can opt in too. It needs no script, so
the CSP is untouched. Browsers without it change page as they do now.

**What htmx and templates cost in a hosted, multi-user product:**

- Login, team switching and permissions are server concerns either way.
  Templates handle them well.
- Anything that must survive navigation on the client (a draft answer,
  keyboard triage across the list, optimistic updates) is awkward. Today the
  poll compares state so it does not throw away typing; that kind of care
  grows with every feature.
- Live updates from other people are fine: SSE into an htmx swap works.
- Offline answering on a phone is not realistic.

**What a client-rendered app costs:**

- A JavaScript toolchain and `node_modules` in CI. Per the repo's rules that
  means `npm ci --ignore-scripts`, pinned deps and a vendored or embedded
  build.
- A second model of the case in the browser, and a JSON API for the web as
  well as the CLI. The API is coming anyway (section 4), which is the best
  argument for a client app.
- CSP: a bundle served from `'self'` is fine, but the bundler must not emit
  inline scripts, and CSS-in-JS libraries that inject styles at runtime need
  `'unsafe-inline'` or nonces. Plain CSS files avoid that.

**Recommendation.** Stay on htmx and templates through modes 2 and a hosted
beta. Add view transitions in CSS now; that answers the transitions wish.
Reconsider when one of these is true: we want client state that survives
navigation (keyboard-driven triage, drafts across cases), we want offline
use, or the JSON API is complete and a second client is planned. At that
point a small client app over the same API costs little extra.

## 9. Staging

Each step is useful alone. None is thrown away. Sizes: S is days, M is about
a week, L is weeks.

| # | Step | Size | Useful alone because |
| --- | --- | --- | --- |
| 1 | Skew wording (chosen) and `--revision` on agent writes (chosen) | S | Clearer errors on synced stores; safe retries later |
| 2 | Release tags, CI binaries, version in skew messages | S | Tells people what to install |
| 3 | `Store` interface; CLI and web talk to it; `sweep` off `Case.Dir` | M | Tests can use a fake; no behaviour change |
| 4 | Optional `actor` and `for` fields; CLI fills `actor` from config | S | A shared synced store shows who answered |
| 5 | Serve beyond loopback behind a trusted header (existing case, rank 59) | M | The inbox works from a phone on an exe.dev VM |
| 6 | After-write hook, cursor and `/v1/events`; web inbox may use it | M | Notifications can hang off it on one machine |
| 7 | JSON API, bearer tokens, HTTP `Store`, `--store https://`, remote `wait` | L | Mode 2 is complete |
| 8 | Accounts, sign-in, `cases login`, orgs, teams as stores | L | Hosted only |
| 9 | Tenant storage, backups, retention, whole-case deletion, export | L | Hosted only |
| 10 | Web Push, billing, limits, abuse handling, operations | L | Hosted only |

Steps 1 to 7 serve a single user with more than one machine. Steps 8 to 10
are the hosted product.

Storage for step 9: a directory per team on a disk is enough to start and
keeps the export trivial. A database table of event rows (case id, seq,
author, event, the JSON bytes) is the likely next step. Either way the bytes
are the ones the file format defines, and `apply` folds them.

## 10. Decisions

**1. Adopt the staged path, and what to build first.**

- a. Steps 1 to 4 now: the chosen S cases, releases, the `Store` interface
  and identity fields. Nothing visible changes except `actor` in the thread.
- b. Identity on serve first (step 5), as the earlier recommendation said,
  and the interface when the API work starts.
- c. Park the direction: build only step 1 and revisit the hosted idea later.

Recommendation: a. The interface is what makes steps 5 and 7 cheap and keeps
the web app and CLI on one write path. Step 5 is worth doing straight after
if the phone inbox on a VM is wanted soon.

**2. Schema version in the record format.**

- a. None. Keep additive changes, skew messages, and a `GET /v1`
  capabilities check when the API exists.
- b. A store marker with a minimum writer version, built now.
- c. A version field on every event.

Recommendation: a, as argued in section 7. b stays available for a future
breaking change and costs nothing to defer.

**3. Web UI stack.**

- a. Stay on htmx and templates; add CSS view transitions now; revisit on the
  triggers in section 8.
- b. Move to a client-rendered app now, over a JSON API built for it.
- c. Stay on htmx, and add small vanilla script modules from `/static` for
  the few interactions that need client state.

Recommendation: a. It gives the transitions asked for without a toolchain,
and c is the natural next move if one interaction outgrows htmx before the
triggers for b are met.

## Open questions

- **Where the hosted server code lives.** `internal/store` cannot be imported
  from another module. Either the hosted server lives in this repo, or the
  store package moves out of `internal`. This can wait until step 8.
- **Secrets in hosted cases.** Whole-case deletion fits the log; redacting one
  event does not. Is deleting the case enough?
- **Does the exe.dev edge buffer SSE?** The long-poll fallback covers it, but
  it should be checked on a VM before step 6.
- **Synced local stores.** Should ids get a short random suffix to stop
  cross-machine collisions in mode 1? It is only needed if synced folders
  stay a supported shape once mode 2 exists.
