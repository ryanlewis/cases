# C5: cases login

The design for C5 of the hosted plan (§3): `cases login`, `cases logout` and `cases login status` for an account on a hosted server. They use the loopback PKCE sign-in and the device code, keep the token in the macOS keychain or a 0600 file, and mint scoped agent tokens into a file. It slots into what C4 built (`docs/design/c4-remote-store.md`). Code comes after review.

Sources: the plan §2 ("Tokens", "The CLI"), §3 (C5), §4 (token expiry, scoped agent tokens, keychain prompts) and decisions 3, 12 and 14, with the C5 follow-ups under "Where it stands"; the README's 28 Sep (`eba6l`, `k8yKT`), 29 Sep (H6), 30 Sep (v2 sign-in) and 3 Oct (`K6U5Q`, `hU2Bv`) entries; AC-03 to AC-05, SV-01 and ST-11; spec/api-agents.md, "Auth", "Signing in" and "Errors"; the identity research, "The CLI, agents and MCP"; hosted's H6 design with its "As built" sections, and the Worker's `/v1/cli` routes; the round-2 spike, `feat/cli-login`.

## What changes

- `cases-store-remote` gains the sign-in wire: an unauthenticated `Signin` for the PKCE exchange and the device code, and `Client::revoke`. It still reads no environment, config or file.
- The CLI gets three commands: `cases login`, `cases login status` and `cases logout`.
- The CLI gets a token store, `credentials.rs`: the macOS login keychain, or `credentials.toml` in the config directory, mode 0600. It holds one token per server.
- The token order of SV-01 gains the stored token (see "The token order", and open question 2 for where).
- The kept list of stores (`accounts/<host>.json`) is written at login and removed at logout.
- A remote store with no token, or one the server refuses, says to run `cases login <server>`.
- macOS only: one new dependency, for the keychain (see "Where the token is kept").

C5 does not include C7's listening wait, the notice case for a new token (H8b, on the server), several accounts on one server (plan decision 14), or the Linux Secret Service (open question 7).

## The commands

All three reach no store: they are `Reach::Nothing`, like `stores`, `config` and `skill`. So they need no token to start, and never look up a store.

**Which server.** Each command takes an optional `[SERVER]`, an `https://host[:port]` address checked as C4's `Server` is. With none given, it is `CASES_SERVER`, then the config's `server`. With none of those, the command exits 1: "name the server: cases login https://…". There is no built-in server until the domain is live (open question 5).

### `cases login [SERVER]`

It signs this computer in to the account that approves it, keeps the token, and replaces any token it kept for that server before.

| Flag | Does |
| --- | --- |
| (none) | Loopback PKCE: opens the server's approval page in the browser, and listens on `127.0.0.1` for the browser to come back |
| `--device` | The device code: prints a page and a code to type in any signed-in browser, for SSH and machines with no browser |
| `--no-browser` | Loopback, but prints the page's address rather than opening it |
| `--name TEXT` | The token's name. Default: `cases CLI on <host name>`, at most 100 characters |
| `--store NAMES` | The stores the token opens: one name, or several split by commas. Default: every store in the account |
| `--file PATH` | Mints a new token into PATH for an agent or a service, and leaves this computer's login alone (see below) |
| `--force` | With `--file`: replace a file that is already there |

`--store` is the global flag. For `login` only the flag counts, never `CASES_STORE` or the config's `store`, so a token's scope is never set by accident. Its value must be store names (`team-x` or `team-x,ops`). A path, a URL or an id is refused, because the approval page and the device start take names. The server turns them into ids when the token is minted (plan decision 3).

What it prints on stdout, after the check (see "Signed in"):

```
Signed in to https://cases.example as ryan (ac_…).
Token "cases CLI on mbp", for every store, kept in the macOS keychain.
Stores: me (default), team-x.
```

Then, as needed, on stderr:

- with no server set: "Commands use the local store until a server is set: add server = "https://cases.example" to <config path>, or set CASES_SERVER." Login writes no config (open question 4).
- with another server set: "Commands use <that server>, not this one."
- when `CASES_TOKEN` or a token file is set, it wins over the stored token: "CASES_TOKEN is set, and commands use it rather than this login."
- when the token went to the file on macOS: why (`CASES_NO_KEYCHAIN` is set, or the keychain could not be written).
- when the token it replaced could not be revoked: "revoke the old token at https://cases.example/tokens".

`--json` is not offered on `login`. `login status --json` gives the same facts.

### `cases login --file PATH`

This is for an agent, CI or a service: `cases login --device --file ~/.config/cases/ci.token --store team-x`. It runs the same sign-in (loopback or `--device`) and the same check, then:

- writes the token to PATH as one line, made with mode 0600 and `O_EXCL`. An existing file is refused unless `--force`, which writes a temporary file (0600) beside it and renames it over. A missing parent directory is refused, not made.
- leaves the stored login alone: it does not replace it, does not revoke anything, and does not write the kept list. The new token is a second token for the account, listed on `/tokens` under its name.
- prints where it went and what it opens, never the token:

  ```
  Wrote a token for ryan (ac_…) to /home/ci/.config/cases/ci.token.
  Token "cases CLI on ci, ci.token", for team-x only.
  Use it with token-file = "/home/ci/.config/cases/ci.token", or CASES_TOKEN_FILE.
  ```

The default name adds the file's name (`cases CLI on ci, ci.token`), so `/tokens` tells the tokens apart.

AC-03 reads `--file` as "keep this login in `credentials.toml`, not the keychain". That reading breaks the case the plan names it for. In v1 a server has one stored login. On the human's own laptop, `cases login --file --store team-x` would replace the human's all-stores login with an agent's scoped one, and revoke the old one. So `--file` takes a path and makes a second token. `CASES_NO_KEYCHAIN` keeps a login out of the keychain, as AC-03 already allows (open question 1).

### `cases login status [SERVER]`

It shows the token commands will use for that server, and what the server says about it. It always calls `GET /v1/cli/whoami`, which also refreshes the kept list.

```
https://cases.example
  account   ryan (ac_…)
  token     "cases CLI on mbp", for every store, expires 2026-12-31
  from      the macOS keychain (cases login)
  default   me (the account's)
```

- `from` is `CASES_TOKEN`, `CASES_TOKEN_FILE <path>`, `token-file <path>`, `the macOS keychain (cases login)` or `<credentials file> (cases login)`. When a stored login is there but another token wins, a last line says so.
- `default` is the config's `default-store`, else the account's default, else the one store the token opens, else "none: name a store with --store". These are C4's rules.
- An expiry shows as its date. The server moves it with each use (90 days after last use).
- `--json`:

  ```json
  {"server": "https://cases.example",
   "account": {"id": "ac_…", "handle": "ryan"},
   "token": {"name": "cases CLI on mbp", "stores": null, "expires": "2026-12-31T10:00:00Z"},
   "from": "keychain",
   "default": {"id": "st_…", "name": "me", "from": "account"}}
  ```

  `from` is `env`, `env-file`, `token-file`, `keychain` or `credentials`. `default.from` is `default-store`, `account` or `only`, and `default` is `null` when there is none.

Exit codes: 0 when the server takes the token. 1 when there is no token ("not signed in to <server>: run cases login <server>"), when the server refuses it (its message, and which source the token came from), or when it cannot be reached (what is known locally, then why).

### `cases logout [SERVER]`

It revokes the stored token on the server, then forgets it here. It touches only the stored login, never `CASES_TOKEN` or a token file.

1. Read the stored token for the server. None: exit 1, "not signed in to <server>".
2. Find the account id: from the kept list when its token hash matches, else from `whoami`.
3. `DELETE /v1/accounts/{account}/cli/token` with the token.
4. Then:
   - **200:** forget it: the keychain item or the file's entry, and the kept list when its token hash matches. "Signed out of https://cases.example (ryan)."
   - **401** (already revoked or expired) or **404** (the server does not know the token or the account): forget it as above, and say the server had already let it go.
   - **No answer, 5xx, or 429 after its one wait:** forget nothing, and exit 1: "the token is not revoked: <why>. Run cases logout again, or cases logout --forget to forget it here only."

The revoke is safe to send twice, since revoking a revoked token answers 200. So it is retried like a read.

| Flag | Does |
| --- | --- |
| `--forget` | Forgets the stored token and the kept list, with no request. For a server or account that is gone. It says the token stays live on the server until it is revoked at `/tokens` or goes 90 days unused |
| `--file PATH` | Revokes the token in PATH, then removes the file. The same rules: the file stays when the revoke fails. With `--forget`, it only removes the file |

Neither form changes the config (`server`, `default-store`, `token-file`).

## Signing in: the loopback

1. **Listen.** Bind `127.0.0.1:0`: IPv4 loopback only, never `0.0.0.0` or `localhost`, on a port the system picks. The redirect is exactly `http://127.0.0.1:<port>/callback`, which is all the server takes.
2. **Make the secrets.** From `getrandom`, which the CLI already has:
   - the verifier: 32 bytes as base64url, 43 characters (RFC 7636 allows 43 to 128);
   - the state: 32 bytes as base64url;
   - the challenge: the verifier's SHA-256 as base64url (S256).
3. **Open the approval page:** `{server}/cli/authorize?code_challenge=…&code_challenge_method=S256&state=…&redirect_uri=…&name=…&stores=…`, with `stores` left out for every store. It opens through `browser::open`, unless `--no-browser` is given or the opener fails. Then the address goes to stderr with "Open this page to approve the sign-in". With no display (`SSH_CONNECTION` set, and no `DISPLAY` off macOS), it suggests `--device`. A signed-out browser is sent to `/login` and back by the server.
4. **Wait for one callback,** for up to 5 minutes. Each connection is answered on a thread of its own, reading at most 16 KiB with a 10 s read timeout, so a browser's preconnect never holds up the real request. Anything other than `GET /callback?…` with a `Host` of `127.0.0.1:<port>` gets 404 and is waited past.
   - **The state is checked first.** A callback whose state is not this sign-in's gets 400, "This is not the sign-in the terminal started". Nothing in it is used, not a code and not an error. The CLI warns once and goes on waiting, so a stray page cannot end the sign-in (open question 10).
   - With the right state and `error=access_denied`: the page says the sign-in was denied, and the CLI exits 1. The error's text is printed with its control characters removed.
   - With the right state and a `code`: the listener stops taking connections. That browser request is held open while the CLI exchanges the code and checks the token (below), so the page can tell the truth.
5. **What the browser is shown.** A small page with no script and no outside resources. It is sent with `Content-Security-Policy: default-src 'none'`, `Cache-Control: no-store`, `Referrer-Policy: no-referrer` and `Connection: close`. It says one of three things: "Signed in as ryan. You can close this tab.", "The sign-in failed: see the terminal.", or, after 20 s, "Finishing in the terminal." It never holds the token, the code, or the server's text.
6. **Exchange the code:** `POST /v1/cli/token` `{code, code_verifier, redirect_uri}` (next section).

The listener closes when the command ends. A Ctrl-C at any point stores nothing.

## Signing in: the device code

1. `POST /v1/cli/device` `{name, stores?}`, where `stores` is the list of names. The answer is `{device_code, user_code, verification_uri, interval, expires_in}`.
2. Print to stderr:

   ```
   Open https://cases.example/device in a browser signed in to your account,
   and type the code:

       BCDF-GHJK

   The code lasts 10 minutes. Type it only if you started this sign-in.
   ```

   The minutes round up. The browser is never opened, and the code is never put in the address (the spec, and the identity research's advice on device-code phishing). The device code itself is a secret and is never printed.
3. Poll `POST /v1/cli/device/token` `{device_code}` every `interval` seconds (at least 1), until `expires_in` runs out:

   | Answer | Then |
   | --- | --- |
   | 200 | the token answer: go on to the check |
   | 400 `authorization_pending` | poll again |
   | 400 `slow_down` | add 5 s to the interval (RFC 8628), poll again |
   | 400 `access_denied` | exit 1: the sign-in was denied |
   | 400 `expired_token` | exit 1: the code ran out; start again |
   | 400 `invalid_grant` | exit 1, with the server's message |
   | 429, any body | wait for `Retry-After` (at most 10 s, 1 s when none is sent), then poll again. Read by status (N26) |
   | 503 `unavailable` | a store could not take the token. The code stays approved, so poll again |
   | no answer, other 5xx | warn once, poll again |

   Each sleep stops at the deadline. A 429 on the start (too many sign-ins waiting) fails at once: "too many sign-ins are waiting on the server; try again in a few minutes".

## The token answer, and "Signed in"

Both flows end with the token answer: `{token, name, account: {id, handle}, stores: [{id, name}], default_store}`. `default_store` is `{id, name}`, or `null` when the token does not open the account's default.

- **`token`** is parsed as C4's `Token`: `csu_` or `csa_`, then 36 base62 characters. Anything else is a protocol error, and nothing is kept.
- **The check before "Signed in"** (AC-03): `GET /v1/cli/whoami` with the new token. It must answer 200, and agree with the token answer:
  - `account.id` is the answer's;
  - `token.name` is the answer's name;
  - `token.stores` is `null` for an all-stores token, or the same set of ids as the answer's `stores`;
  - `stores` holds every store the answer named.

  The mint answers only after every store in scope has the token, so a 200 here also shows the pushes landed (plan §2).
- **When the check fails** (401, a mismatch, or no answer after the client's retries), nothing is kept. The CLI revokes the new token, as best it can, and exits 1. Its message says whether the revoke worked, or to revoke the token at `{server}/tokens`.
- **Then, for a login (not `--file`), in this order:**
  1. **The kept list:** `accounts/<host>.json` is written from the check's `whoami` answer. It holds the token answer's stores, with each store's `default` flag, keyed by the new token's SHA-256. `Accounts::whoami` already does this. So the first command after login needs no request to resolve a name or the default (C4).
  2. **The token** is kept (next section). It is written before the old one is revoked, so a failure part-way never leaves the computer with no token.
  3. **The old token,** if one was kept for this server and is not the new one, is revoked: `DELETE /v1/accounts/{its account}/cli/token`. 200, 401 and 404 are all fine. Anything else is a warning naming `/tokens`. This is H6's open question 7: the server never guesses which token is "old"; the CLI revokes it.
  4. Print "Signed in".

**`default-store`** is never asked for and never written. The account's default comes in the token answer, and the kept list carries it. C4's rules then pick the store: the config's `default-store`, else the account's default, else the only store. A scoped token that does not open the account's default has no default, unless it opens exactly one store or `default-store` names one it opens. `login status` shows which rule applied.

## Where the token is kept

**One token per server.** The key is the server's address as C4's `Server` writes it (`https://cases.example`, with a port when it has one). The account id is kept beside the token, so a second account per server (plan decision 14) can key on both later.

**macOS: the login keychain,** unless `CASES_NO_KEYCHAIN` is set and not empty. It is a generic password item, with service `cases`, account `<server>`, and the token as the password. The account id is not in the item: logout finds it in the kept list, or asks `whoami`.

- If the keychain cannot be written, the token goes to the file with a warning. This happens over SSH, where the login keychain is often locked.
- **A read never stalls an agent.** When stdin is not a terminal, the read runs with keychain interaction turned off (`SecKeychain::disable_user_interaction`). A read that would need the human's approval then fails at once rather than waiting on a dialog nobody sees. That happens to an unsigned binary after each upgrade. The message says: "the keychain needs your approval to give cases its token for <server>: run cases login status in a terminal and choose Always Allow, or give agents a token file (cases login --file)". The plan §4 item "keychain prompts stall agents" is then a clear failure, not a hang.
- **Writing it makes the file's entry for that server go,** so only one copy is ever read.

**Elsewhere, or with `CASES_NO_KEYCHAIN`: `credentials.toml`** in the config directory, `$XDG_CONFIG_HOME/cases/` or `~/.config/cases/`. It is there even when `--config` or `CASES_CONFIG` points elsewhere, since the config file is shared and the credentials are not.

```toml
# cases login's tokens, one per server. Keep this file private.
["https://cases.example"]
account = "ac_…"
token = "csu_…"
```

- It is written to a temporary file named with the process id, made with mode 0600 and `O_EXCL`, then renamed over. A failed write removes the temporary file, so no copy is left behind.
- A directory it makes is 0700. A file that others can read, or a directory that others can write (which lets them replace the file), gets a warning with the `chmod` to fix it. This is the spike's check.
- It is read only with the `toml` parser the CLI already has. It is written by hand, as the spike did, quoting each string as JSON, which TOML reads as a basic string. This keeps `toml`'s writing feature out.

**Reading the stored token:** the keychain (when it is on), then the file. When `CASES_NO_KEYCHAIN` is set the keychain is never touched, not even read. Every test sets it.

**The keychain crate.** `security-framework` 3.7.0, pinned (`=3.7.0`), MIT OR Apache-2.0, as a macOS-only dependency. It brings `security-framework-sys`, `core-foundation`, `core-foundation-sys`, `bitflags` and `libc`, all MIT or Apache-2.0, so `deny.toml` needs no new licence. It gives `get_generic_password`, `set_generic_password` and `delete_generic_password`, and the interaction switch above. The spike's `keyring-core` 1.0.0 with `apple-native-keyring-store` 1.0.2 (both MIT OR Apache-2.0) wraps this same crate. It adds a store registry and `log`, and does not expose the interaction switch (open question 6).

## The token order

C4's `store::token` becomes, for a server:

1. `CASES_TOKEN`, when set and not empty;
2. a token file: `CASES_TOKEN_FILE`, else the config's `token-file` (open question 3);
3. the stored token for that server (keychain, then `credentials.toml`).

This puts the stored token last, where SV-01 and the C4 design put it between `CASES_TOKEN` and `token-file` (open question 2). The reasons:

- It follows SV-01's own rule of flag, then environment, then config, then default. A login is the default; a `token-file` someone wrote is a choice.
- A scoped agent token in a token file must win over the human's all-stores login on the same user account. Otherwise `--file --store` gives an agent the human's token, without a word.
- An agent with a token file never reads the keychain, so it never meets a keychain dialog.

The cost: a `token-file` left in the config hides a new login. `cases login` warns about that, and `login status` shows which token is used.

**A one-off URL** (`--store https://other.example/team-x`) takes the stored token for its own host. `CASES_TOKEN` and a token file go to it only when its host is the configured server's, or no server is configured. So a token is never sent to a server it was not made for (open question 9; until now any host got them).

**With no token,** the message becomes "not signed in to <server>: run cases login <server>, or set CASES_TOKEN or a token file".

**A keychain or file that cannot be read** is an error when it is the last source. It is never skipped quietly, because a skipped source could send another token. Since the stored token is now last, the spike's "warn and use token-file" case no longer arises.

## A revoked or expired token, in every command after

The server answers 401 `auth`, with a message that says `revoked`, `expired` or `not for this store`. The CLI does not read the message to decide anything: it prints it. It adds where the token came from and what to do:

- the stored login: "<server>: the token was refused (revoked). Run cases login <server>."
- `CASES_TOKEN`, or a token file: "<server>: the token in CASES_TOKEN was refused (expired)", with no login hint, since a login would not change what is used.

A stored token that gets 401 is not forgotten by itself, since "not for this store" does not mean it is dead. `cases login` replaces it, and `cases logout` forgets it. C4's behaviour on a store named from the list stays: a 401 reads the list once more before it fails (AC-04).

## Each request

All sign-in requests go through `cases-store-remote`, with C4's agent settings: no redirects followed (3xx is an error), `User-Agent: cases/<version>`, connect timeout 10 s, the environment's proxy settings, and no token in any URL.

| Step | Request | Auth | Timeout | Retries |
| --- | --- | --- | --- | --- |
| Approval | browser to `GET /cli/authorize?…` | browser session | 5 min for the callback | none |
| Exchange | `POST /v1/cli/token` `{code, code_verifier, redirect_uri}` | none | 30 s | see below |
| Device start | `POST /v1/cli/device` `{name, stores?}` | none | 30 s | 429: none, fails |
| Device poll | `POST /v1/cli/device/token` `{device_code}` | none | 30 s | the table above, until `expires_in` |
| Check | `GET /v1/cli/whoami` | the new token | 30 s | C4's read retries (0.5 s, 2 s) |
| Status | `GET /v1/cli/whoami` | the token in use | 30 s | C4's read retries |
| Revoke | `DELETE /v1/accounts/{account}/cli/token` | the token | 30 s | as a read: it is idempotent |

- **The exchange waits on the server.** The Worker answers only once every store in scope has the token, and tries for up to about 10 s first (H6, "As built"). So the exchange's timeout is 30 s, C4's `REQUEST_TIMEOUT`, which leaves room. A unit test pins it at 20 s or more. The device poll that finds the code approved mints the same way, with the same timeout.
- **503 `unavailable` on the exchange:** a store could not take the token. The server deleted it and kept the code usable for its 2 minutes (S4). The CLI tries again after 2 s, 5 s and 10 s, then fails: "a store could not be reached; run cases login again".
- **No answer to the exchange.** The server may have minted the token and lost the answer, and the code may be spent. The CLI sends the exchange once more. If that gives `invalid_grant`, it fails with: "the sign-in may have made a token that never arrived: revoke any you don't recognise at <server>/tokens, then run cases login again".
- **429 is read by status, never by category or message (N26).** On every sign-in route, 429 is `Error::Rate`. The client waits for `Retry-After` when it is 10 s or less (1 s when none is sent) and tries once more. A longer `Retry-After` fails at once, as C4 does. `slow_down` is only the poll's 400 answer.
- **The other answers,** read by status, then by category:

  | Status | Category | The client gives |
  | --- | --- | --- |
  | 200 | | the answer |
  | 400 | `invalid_grant`, `authorization_pending`, `slow_down`, `access_denied`, `expired_token` | `SigninError` of that kind |
  | 400 | anything else | `Error::Request` |
  | 401 | any | `Error::Auth`, with the message |
  | 404 | any (revoke) | `Error::NoToken`: the server does not know this token or account |
  | 429 | any | `Error::Rate` |
  | 503 | `unavailable` | `Error::Server`, transient |
  | 3xx, other 5xx, a body that is not the API's | | as C4 |

**In the crate.** `Signin::new(server)` holds a ureq agent and the server, but no token. It has `authorize_url(&Pkce, redirect, name, stores)`, `exchange(code, &Pkce, redirect)`, `device_start(name, stores)` and `device_poll(&DeviceCode)`. `Pkce` makes the verifier, the state and the challenge, and its `Debug` hides the verifier. `DeviceCode` hides itself the same way. `TokenAnswer` holds a `Token`, whose `Debug` and `Display` already hide it. `Client::revoke(account)` sends the `DELETE`. The loopback listener, the browser, the keychain and the file stay in the CLI, since the crate never touches the system.

## Security

- **CSRF and the state.** The state is 256 random bits, checked before anything else in the callback, with a constant-time compare. A callback with another state is never used: not its code and not its error. The listener answers only `127.0.0.1`, only `GET /callback`, with the expected `Host`. A page that finds the port can at most send a request that is turned away.
- **The code is bound twice:** to the verifier, which never leaves the CLI until the exchange, and to the exact redirect. The server spends it on a bad verifier. A code that leaks from the browser's history is useless.
- **Never logged or printed:** the verifier, the state, the code, the device code and the token. The authorize address holds the state and the challenge, which are not secrets. It is printed only when the browser cannot be opened. Errors hold the server and the route, never a body that was sent.
- **The token is shown nowhere** except in the file `--file` writes, which is made 0600, refuses to replace a file unless `--force`, and does not follow a symlink at PATH (`O_NOFOLLOW`). The token goes only in the `Authorization` header (AC-05). The tests check the fake's record of every request for that.
- **The device flow** never opens the browser and never puts the code in the address. It tells the human to type the code only for a sign-in they started. The server's page shows where the start came from.
- **Scope.** A scoped token is minted by the server from names it resolves itself, and the check confirms the scope before the token is kept or written.
- **Revoke on replace.** A new login revokes the token it replaces, so no live token is left that nobody holds.

## What the spike taught, and what is kept

The spike (`feat/cli-login`, commits 08eb1b1, 30826d7, 65ae6e5, 870dc8d) predates accounts, the `Store` trait and C4. It signed in to one store by its URL. Its code is the base for `login.rs` and `credentials.rs`, reworked for accounts:

| In the spike | In C5 |
| --- | --- |
| Loopback on `127.0.0.1:0`, never `0.0.0.0` | kept |
| PKCE S256, verifier and state from `getrandom`, base64url by hand, RFC 7636's test vector | kept |
| `--no-browser` prints the page | kept |
| A thread per callback connection, 16 KiB, 10 s read timeout | kept |
| The state checked before anything; control characters stripped from an error | kept |
| A callback with another state ends the sign-in | changed: refused, and the wait goes on |
| The browser is told "Signed in" before the exchange | changed: told after the exchange and the check |
| `store` and `client` in the requests | changed: `name` and `stores`, as the spec has them |
| No check after the exchange | added: `whoami` before "Signed in" |
| Device: the server's interval, +5 s on `slow_down`, retry until the deadline, minutes rounded up | kept |
| Device errors read by category, 429 included | changed: 429 by status (N26) |
| Logout through the store's `DELETE …/token` | changed: `DELETE /v1/accounts/{account}/cli/token` |
| A token taken if it starts with `csu_` | changed: parsed as `Token` |
| `keyring-core` and `apple-native-keyring-store` | changed: `security-framework` directly, with reads that fail rather than prompt off a terminal |
| `credentials.toml` keyed by the store's URL | changed: keyed by the server, with the account id |
| `CASES_NO_KEYCHAIN`, set by every test | kept |
| A keychain that can't be written falls back to the file, with a warning | kept |
| A keychain or file error falls back to `token-file`, with a warning | dropped: the stored token comes last now |
| A new login revokes the token it replaces, and warns when it can't | kept |
| Temporary file named with the pid and removed on failure; directories 0700; a warning when the file or directory is open to others | kept |
| A warning at login when `CASES_TOKEN` is set | kept, and for a token file too |
| Login writes the store's URL into the config | dropped: login writes no config |
| Login shares the hosted store's agent and error helpers | changed: the sign-in wire lives in `cases-store-remote` |
| `scripts/e2e-login.mjs`: headless Chromium signs in with the Worker's secret | changed: an interactive script that Ryan approves (see "Testing") |

## Testing

**Unit tests in `cases-store-remote`:**

- PKCE against RFC 7636's appendix B;
- the verifier and the state are 43 base64url characters;
- `Debug` hides the verifier, the device code and the token;
- the authorize address: each parameter escaped, `stores` comma-joined or left out;
- each row of the sign-in error table, including a 429 whose category is `slow_down`, which is still `Rate`;
- the exchange's timeout is at least 20 s;
- the token answer: a bad token, or a missing account, is a protocol error.

**The fake server** (`crates/store-remote/tests/fake/`) grows the sign-in routes. It keeps tokens it mints, each with a scope and a revoked flag, beside the fixed `TOKEN`, and checks every store route and `whoami` against them. A 401's message says `revoked`, `expired` or `not for this store`. The new pieces:

- `approve(authorize_url)` plays the approval page and the human. It checks the query as the Worker does (S256, the redirect's shape, the name, the store names), keeps a grant, and gives the redirect address with `code` and `state`. `deny(authorize_url)` gives `error=access_denied`. A test then GETs that address on the CLI's loopback port, as the browser would.
- `POST /v1/cli/token`: spends the code on success, a bad verifier or another redirect; can be told to answer 503 and keep the code.
- `POST /v1/cli/device` and `POST /v1/cli/device/token`, with `interval` 1, and controls to approve, deny or expire a code. A poll inside the interval gets `slow_down`.
- `DELETE /v1/accounts/{account}/cli/token`: revokes, gives 200 for a token already revoked, and 404 for another account.
- Faults on these routes, as C4's: 429 with and without `Retry-After`, 503, a hang-up after the request (the lost answer), and a slow answer.

**CLI tests** (`crates/cli/tests/login.rs`, the binary against the fake). Each test has its own home, config and state directories, and `CASES_NO_KEYCHAIN=1`, set in `tests/common` for every test so none can reach a real keychain. The browser is a stand-in `open` (or `xdg-open`) first on `PATH`, which writes the address to a file, as `browser.rs` allows. The tests cover:

- loopback: signed in; the token in `credentials.toml` with mode 0600; the kept list written; the next command uses the token with no `whoami`;
- the browser page says signed in, or failed;
- a callback with another state gets 400 and the sign-in still finishes; a denial exits 1;
- device: pending, then approved; `slow_down` widens the interval; denied; expired; a 503 then approval;
- a failed check: nothing kept, and the new token revoked;
- the exchange's 503 then success; a lost answer then `invalid_grant` gives the `/tokens` message;
- logging in again revokes the old token, and keeps the new one if that revoke fails;
- `--store` scope reaches the approval as `stores=`; `CASES_STORE` does not;
- `--file`: written with mode 0600; an existing file refused; `--force` replaces it; a symlink refused; the stored login and kept list untouched;
- the token order: `CASES_TOKEN`, then `CASES_TOKEN_FILE` and `token-file`, then the stored token; and a one-off URL to another host gets none of the first two;
- `login status`, as text and JSON, for each `from`, and exit 1 for a revoked token, naming its source;
- `logout`: revoked and forgotten; the server down keeps the token and exits 1; `--forget` sends no request; `--file` removes the file only after a revoke;
- no output, error, file other than the credentials and `--file`, or request outside `Authorization` holds the token. The verifier appears only in the exchange's body.

**The listener on its own:** unit tests in the CLI for the callback with a short wait: the timeout; a preconnect that sends nothing; a request over 16 KiB; another path; another `Host`.

**No pty test is needed.** The flow reads no keys. Whether stdin is a terminal only switches keychain interaction, and the tests never reach the keychain.

**The keychain by hand.** One `#[ignore]` test on macOS stores, reads and deletes a token under a throwaway service, `cases-test-<pid>`. Ryan runs it with `cargo test -p cases -- --ignored keychain`. Agents and CI don't.

**Mutants.** The sign-in module is in `cases-store-remote`, which the nightly run already mutates. Survivors get a test, or an exclusion in `.cargo/mutants.toml` with the reason. The CLI, with the listener and the keychain, is not in the run, as today.

**End to end, opt-in.** `scripts/e2e-login.sh` runs the built binary against `CASES_E2E_SERVER`, in a home of its own with `CASES_NO_KEYCHAIN=1`. It refuses to run when `CI` is set, and prints no token. It runs:

1. `cases login --device`, which Ryan approves in his browser;
2. `login status`, and `cases stores`;
3. an `open` and a `withdraw`;
4. `login --file --store <name> --device`, then a command with that file, and one against another store, which must get 401;
5. `logout --file`, then `logout`, then the same commands, which must get 401 `revoked`.

A second run does the loopback with Ryan's browser. Ryan runs it against staging. Agents never do. A headless browser would need the server's bootstrap secret in the script, which a human approving does not.

## Spec and vector changes

In the vault, as one spec PR before the CLI wiring:

- **AC-03:**
  - `--file PATH` mints a second token into PATH (mode 0600, refused if present unless forced) and leaves the login alone (open question 1). `CASES_NO_KEYCHAIN` is how a login stays out of the keychain.
  - The check names what `whoami` must agree with.
  - Credentials are keyed by the server, with the account id kept beside the token.
  - Login writes no config (open question 4).
  - `logout` forgets only after the revoke works; `--forget` makes no request; `logout --file`.
  - A keychain read off a terminal fails rather than prompting.
- **AC-05:** `cases login --file PATH --store team-x`; the scope comes only from the flag.
- **SV-01:**
  - The token order becomes `CASES_TOKEN`, then a token file (`CASES_TOKEN_FILE`, then `token-file`), then the stored token (open questions 2 and 3).
  - `CASES_TOKEN` and a token file go to a one-off URL only on the configured server's host (open question 9).
  - The no-token message names `cases login`.
- **ST-11:** the one-off URL line: "with the token for that host" means the stored token, and the others only as SV-01 says.
- **api-agents.md:**
  - the errors table gains 503 `unavailable` (the exchange and the device poll), and the sign-in routes' 400 codes;
  - the revoke's 404 for a token the server does not know;
  - a line under "Signing in" that 429 is read by status.

**Vectors:** none change. Signing in is not the core's, and no store behaviour moves. `spec/vectors/CHANGES.md` needs no line.

## Semver

- **`cases-core`:** unchanged.
- **`cases-id`:** unchanged.
- **`cases-store-remote`:** gains `Signin`, `Pkce`, `DeviceCode`, `TokenAnswer` and `Client::revoke`, and error kinds. It is `publish = false`, so there is no semver check.

## Size and the PRs

About 2,600 lines with tests:

- the crate's sign-in wire: about 350, with 250 of tests;
- the fake's routes: about 300;
- the CLI: about 950, of which `login.rs` is 600 and `credentials.rs` with the keychain 350;
- the CLI tests: about 750.

In order:

| PR | What | Size |
| --- | --- | --- |
| S | The spec lines above, in the vault | small |
| C5a | `cases-store-remote`: `Signin`, PKCE, the exchange, the device start and poll, `Client::revoke`, their errors; the fake's sign-in routes and minted tokens | medium, no CLI change |
| C5b | The CLI's token store: `credentials.rs`, the keychain on macOS, `CASES_NO_KEYCHAIN`; the new token order with `CASES_TOKEN_FILE`; the one-off URL's host rule; the 401 and no-token messages | medium |
| C5c | `cases login`: loopback and `--device`, the check, the kept list, replace and revoke; `--file` | large |
| C5d | `cases login status` and `cases logout`, with `--forget` and `--file` | small |
| C5e | `scripts/e2e-login.sh`, and the design's "As built" | small |

C5c can split in two if it reviews badly on a phone: loopback first, then `--device` and `--file`. The web split (`K6U5Q`) can start once C5d is in.

## Open questions

1. **What `--file` means.** AC-03 reads it as "keep this login in `credentials.toml`". The plan and AC-05 use `--file --store` for scoped agent tokens. Recommend: `--file PATH` mints a second token into PATH and leaves the login alone. `CASES_NO_KEYCHAIN` keeps a login out of the keychain. The alternative, AC-03's reading, has one login per server. On a laptop, an agent's scoped login would then replace the human's, and revoke it.
2. **Where the stored token goes in the order.** SV-01 and the C4 design put it between `CASES_TOKEN` and `token-file`. Recommend: last, after the token file. That matches SV-01's flag, environment, config, default; a scoped agent token in a token file wins over the human's login; and an agent with a token file never reads the keychain. The alternative keeps SV-01 as written. A `token-file` would then work only on machines with no login, and agents on the laptop would need `CASES_TOKEN`.
3. **`CASES_TOKEN_FILE`.** Recommend adding it: the token file's environment variable, checked before the config's `token-file`. An agent can then use its own token without a config of its own, and without putting the secret in its environment, where other processes and logs can see it. The alternative is `CASES_TOKEN="$(cat file)"`, or a `CASES_CONFIG` for each agent.
4. **Does login set the server?** Recommend no: login writes no config. With no server set, it prints the line to add. The CLI's config handling only reads, and ST-11 says a store never moves to a server quietly: setting the server moves every command, the agents' too. The alternative writes `server` when none is set, as the spike wrote the store.
5. **A default server.** Recommend none in C5: `cases login` with no server, and none set, fails and asks for one. When `app.cases.sh` is live (H9), C8 can make it the default for `cases login` alone.
6. **The keychain crate.** Recommend `security-framework` =3.7.0 directly (MIT OR Apache-2.0). It is the crate the spike's pair wraps, with fewer pieces, and it has the switch that makes a read fail rather than prompt off a terminal. The alternative is the spike's `keyring-core` =1.0.0 and `apple-native-keyring-store` =1.0.2, which would make a Linux keyring a one-line addition later.
7. **The Linux Secret Service.** Recommend not in v1: Linux uses the 0600 file, as `gh` falls back to. Agents and servers on Linux rarely have a desktop session's keyring unlocked, and a D-Bus prompt would stall them as the keychain does.
8. **How the scope is named.** Recommend the global `--store`, read only from the flag, with names split by commas. That is what AC-05 writes. The alternative is a `--scope` flag of login's own, which avoids giving `--store` a second meaning.
9. **Tokens and one-off URLs.** Today any host given as a one-off URL gets `CASES_TOKEN` or the token file. Recommend: a one-off URL gets the stored token for its host, and the other two only on the configured server's host, or when no server is configured. The alternative keeps C4's behaviour, which can send a token to a server it was not made for.
10. **A callback with another state.** Recommend: refuse it with 400, warn, and keep waiting. The spike ended the sign-in. That lets any page that finds the port end someone's sign-in, and gains nothing, since the state check already keeps its code out.
11. **When the check after the exchange fails.** Recommend: keep nothing, revoke the new token as best we can, and say whether that worked. The alternative keeps the token with a warning. AC-03 says the CLI checks the token before it says it is signed in.
12. **Printing a token, `--file -`.** Recommend not in C5. A token is only ever written to a 0600 file, which the tests can check. For a CI secret: `gh secret set CASES_TOKEN < file`, then `rm file`.
13. **The end-to-end run.** Recommend an interactive script, in which Ryan approves the device code and the loopback in his own browser. The alternative is the spike's headless Chromium, which needs the server's bootstrap secret in the script.

## As built

Ryan signed this design off on 3 Oct with all thirteen recommendations. C5 landed in #114 (the sign-in wire in `cases-store-remote`, and the fake's sign-in routes), #115 (where the login is kept, and the token order), #116 (`cases login`), #117 (`cases login status` and `cases logout`) and the end-to-end script, with the spec lines in the vault first. Where the build differs from the text above:

- **Revoking a revoked token.** `DELETE /v1/accounts/{account}/cli/token` checks the token first, so a token already revoked or expired answers 401 `auth`, as does a token that isn't the account's. Only the `/tokens` page's revoke answers 200 again. 404 comes only for an account that isn't the server's. The CLI counts 200, 401 and 404 as done, as above, so logout is unaffected; the spec now says so.
- **The device poll.** The Worker answers `slow_down` to any poll inside its interval, even after approval. A bad device-start body is 400 `request`; a bad poll or exchange body is `invalid_grant`. The exchange retries any 503, since the Worker's is always `unavailable`.
- **Pure helpers in the crate.** `TokenAnswer::check` (the check before "Signed in"), `DeviceCode::wait_after` (the poll table) and `redirect_uri` live in `cases-store-remote` beside `Signin`, `Pkce`, `DeviceCode`, `TokenAnswer` and `Client::revoke`, so they are tested there.
- **The approval address is always printed,** not only when no browser opens. It holds the state and the challenge, which are not secrets on their own, and it helps when the browser opens on another screen.
- **The browser's page is held until the token is kept,** not only checked, so a save that fails is never shown as "Signed in".
- **The 5-minute wait** is tested on the listener alone, with a short wait, not end to end.
- **A stored login that can't be read** when login starts is a warning (its old token can't be revoked), and the sign-in goes on; a credentials file too broken to write then fails the save, and the new token is revoked.
- **An old login with no account beside it** (a keychain item has none) finds its account through `whoami` with the old token before the revoke; a 401 or 404 there counts as already gone.
- **Callbacks.** A callback with another state gets the 400 page whatever it carries. One with the right state and neither a code nor an error gets 400, and the wait goes on. A request over 16 KiB gets 431; another path, method or `Host` gets 404.
- **The "no display" hint** for `--device` appears when `SSH_CONNECTION` is set, and off macOS also needs `DISPLAY` unset.
- **Notes about the server and other tokens.** The "no server set" and "another server set" notes print for `--file` too, since an agent's token file needs a server set as well. The warnings that `CASES_TOKEN` or a token file wins over the new login print for a login only, and not when another server is configured.
- **The default token name** is `cases CLI on <host>`, with `, <file name>` added for `--file`, at most 100 characters. The host name comes from rustix's `system` feature; there is no new crate.
- **Store names in `--store`** must be 1 to 63 lower-case letters, digits or `-`, so an id, a path or a URL is refused before anything is sent.
- **The token store.** The keychain item is a generic password with service `cases` and the server as its account. `credentials.toml` is always in the config directory, even when `--config` names another config file. A corrupt file is an error naming the file and line, never its contents. A malformed entry for any server makes the whole file unreadable, so a write never drops entries it couldn't parse. Two logins writing the file at the same moment can lose one write, as in the spike.
- **The token order's host rule** compares the whole server address: scheme, host and port. A malformed configured server now fails a one-off URL too, where before the one-off URL ignored it.
- **Refusal messages** keep C4's sentence shape and add where the token came from ("the server refused the token cases login kept in <place> …"); only a refused stored login adds "Run cases login <server>".
- **`login status`** adds the stores the token opens, as text and as a `stores` array in `cases stores`'s shape. Its JSON `from` is the source's name only; the path is in the text. It ignores `--store` and `CASES_STORE`, and uses the config's `default-store` only for the configured server. When it gets no answer, the text output prints the server and `from` line before the error; `--json` prints nothing on stdout.
- **`logout` and `logout --forget` with no stored login** exit 1, naming `CASES_TOKEN` or the token file when one of those is what commands use. The kept list is removed only when it was read with the token being logged out. `--file` refuses a symlink, a directory, a missing file, a file over 1 KiB or one that isn't a token, and warns when the config or `CASES_TOKEN_FILE` still points at it.
- **Parsing.** `cases login [SERVER]` and `cases login status [SERVER]` share one command with `args_conflicts_with_subcommands`. A server can't be called "status", since a server must be an `https://` address, or `http://` on loopback.
- **Testing.** No test touches the real keychain: `CASES_NO_KEYCHAIN` is set for every test, the TUI's pty tests and the end-to-end scripts. The one keychain test, `credentials::tests::keychain_by_hand_stores_reads_and_deletes_a_token`, is `#[ignore]` and uses a throwaway item; run it with `cargo test -p cases -- --ignored keychain`. The tests play the browser: they read the approval address from a stand-in `open` on `PATH`, approve it on the fake, and send the redirect to the CLI's port themselves. CI runs on Linux only, so the keychain code was built and linted on macOS before each merge.
- **End to end.** `scripts/e2e-login.sh` runs against a server given by `CASES_E2E_SERVER`, refuses to run when `CI` is set, and pauses for Ryan to approve the loopback and the device code in his own browser. It uses a file, not the keychain, unless `CASES_E2E_KEYCHAIN=1`. It has not yet run against staging.
