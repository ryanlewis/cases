#!/usr/bin/env bash
# End-to-end run of `cases login`, `cases login status` and `cases logout`,
# from the built binary, against a real hosted server. It is interactive:
# you approve each sign-in in your own browser, signed in to the account.
#
# It signs in, makes an agent token for one store, and revokes every token
# it made. It writes no cases. It is opt-in: it never runs in CI, and it
# needs a terminal.
#
# Usage:
#   CASES_E2E_SERVER=https://host [CASES_E2E_STORE=<store name or id>] \
#     [CASES_BIN=path/to/cases] [CASES_E2E_KEYCHAIN=1] scripts/e2e-login.sh
#
# CASES_E2E_SERVER    the server, https://host[:port]
# CASES_E2E_STORE     the store the agent token opens; the account's default
#                     when unset
# CASES_BIN           the binary; target/debug/cases when unset (cargo build --features remote)
# CASES_E2E_KEYCHAIN  1 keeps the login in the macOS keychain, to test that
#                     path. Unset, the keychain is never touched, and the
#                     login goes to a credentials file in a private
#                     temporary directory. With 1 it refuses to run while
#                     the keychain holds a login for the server already,
#                     since that login would be replaced and revoked.
#
# The steps:
#   1. cases login (loopback): the browser opens, you approve
#   2. cases stores and cases list work with the stored login
#   3. cases login --device: you type the code it shows; the old token is
#      revoked
#   4. cases login --file agent.token --store <store>: a token for one store
#      beside the login, which is left alone
#   5. cases logout --file agent.token: the file goes, its token is revoked
#   6. cases logout: signed out, the login forgotten, its token revoked
#   7. cases logout --forget with no login exits 1
#
# Needs curl and jq. It unsets CASES_TOKEN and CASES_TOKEN_FILE and uses a
# config and XDG directories of its own, so nothing of yours is read. A
# token goes to curl only through a header file in the private directory.
# No token, code or verifier is printed, and the output of every command is
# checked for anything shaped like a token. When a step fails it logs out
# what it signed in, as best it can, and says what it could not revoke.
set -euo pipefail

if [[ -n "${CI:-}" ]]; then
  echo "e2e-login.sh talks to a real server and does not run in CI" >&2
  exit 1
fi
SERVER=${CASES_E2E_SERVER:?set CASES_E2E_SERVER to the server, https://host}
SERVER=${SERVER%/}
WANT_STORE=${CASES_E2E_STORE:-}
KEYCHAIN=${CASES_E2E_KEYCHAIN:-}
BIN=${CASES_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/cases}
[[ "$SERVER" == https://* ]] || { echo "CASES_E2E_SERVER must be https://host[:port]" >&2; exit 1; }
[[ -z "$KEYCHAIN" || "$KEYCHAIN" == 1 ]] || { echo "CASES_E2E_KEYCHAIN is 1 or unset" >&2; exit 1; }
[[ -t 0 && -t 1 ]] || { echo "e2e-login.sh is interactive: run it in a terminal" >&2; exit 1; }
for tool in curl jq; do
  command -v "$tool" > /dev/null || { echo "e2e-login.sh needs $tool" >&2; exit 1; }
done
[[ -x "$BIN" ]] || { echo "no binary at $BIN: run cargo build --features remote, or set CASES_BIN" >&2; exit 1; }
if [[ -n "$KEYCHAIN" ]]; then
  [[ "$(uname -s)" == Darwin ]] || { echo "CASES_E2E_KEYCHAIN=1 needs macOS" >&2; exit 1; }
  command -v security > /dev/null || { echo "CASES_E2E_KEYCHAIN=1 needs security" >&2; exit 1; }
fi

umask 077
WORK=$(mktemp -d)

# A home of its own, so that nothing of the user's config, state or
# environment is read, and the login and the account's list stay in $WORK.
unset CASES_TOKEN CASES_TOKEN_FILE CASES_SERVER CASES_STORE CASES_WORKER
if [[ -n "$KEYCHAIN" ]]; then
  unset CASES_NO_KEYCHAIN
else
  export CASES_NO_KEYCHAIN=1
fi
export XDG_CONFIG_HOME="$WORK/config" XDG_STATE_HOME="$WORK/state" XDG_DATA_HOME="$WORK/data"
export CASES_CONFIG="$WORK/config.toml"
printf 'server = "%s"\n' "$SERVER" > "$CASES_CONFIG"
CREDS="$XDG_CONFIG_HOME/cases/credentials.toml"
AGENT="$WORK/agent.token"
TOKEN_SHAPE='cs[ua]_[0-9A-Za-z]{36}'

STEP="setup"
PASSED=()
DONE=""

cases() { "$BIN" "$@"; }
step() {
  STEP=$1
  printf '\n== %s\n' "$STEP"
}
pass() {
  PASSED+=("$STEP")
  echo "ok"
}
fail() {
  printf 'FAILED at step "%s": %s\n' "$STEP" "$*" >&2
  exit 1
}
pause() {
  printf '\n%s\nPress Enter when ready. ' "$*"
  read -r < /dev/tty
}

# run NAME COMMAND...: runs the command with its stderr shown as it comes,
# for the sign-in's page and code, and kept in $WORK/NAME.err; its stdout
# is kept in $WORK/NAME.out and shown after. Sets CODE to its exit code.
# Fails when either holds anything shaped like a token.
run() {
  local name=$1
  shift
  set +e
  "$@" 2>&1 > "$WORK/$name.out" | tee "$WORK/$name.err" >&2
  CODE=${PIPESTATUS[0]}
  set -e
  cat "$WORK/$name.out"
  echo "exit $CODE"
  if grep -qE "$TOKEN_SHAPE" "$WORK/$name.out" "$WORK/$name.err"; then
    fail "the output of $* holds something shaped like a token"
  fi
}

# expect CODE: the last run exited CODE.
expect() {
  [[ "$CODE" == "$1" ]] || fail "exited $CODE, not $1"
}

# says NAME TEXT: the last run's stdout or stderr, kept as NAME, has TEXT.
says() {
  grep -qF -- "$2" "$WORK/$1.out" "$WORK/$1.err" || fail "the output has no \"$2\""
}

# The keychain item's account is the server as cases writes it; it is read
# from login status after the first sign-in.
KEY=$SERVER

# auth FILE: writes curl's header for the token of the stored login to FILE,
# without printing it.
auth() {
  local token
  if [[ -n "$KEYCHAIN" ]]; then
    token=$(security find-generic-password -s cases -a "$KEY" -w 2> /dev/null) || token=""
  else
    token=$(sed -n 's/^token = "\(.*\)"$/\1/p' "$CREDS" 2> /dev/null | head -n 1) || token=""
  fi
  [[ "$token" =~ ^$TOKEN_SHAPE$ ]] || fail "cannot read the stored login's token"
  printf 'Authorization: Bearer %s\n' "$token" > "$1"
}

# auth_file TOKENFILE FILE: curl's header for the token in TOKENFILE.
auth_file() {
  local token
  token=$(tr -d '[:space:]' < "$1")
  [[ "$token" =~ ^$TOKEN_SHAPE$ ]] || fail "$1 does not hold a token"
  printf 'Authorization: Bearer %s\n' "$token" > "$2"
}

# whoami FILE: the HTTP status of whoami with the header in FILE.
whoami() {
  curl -sS -o /dev/null -w '%{http_code}' -H @"$1" "$SERVER/v1/cli/whoami" || echo 000
}

# revoked FILE WHAT: whoami refuses the header in FILE with 401.
revoked() {
  local got
  got=$(whoami "$1")
  echo "whoami with $2: $got"
  [[ "$got" == 401 ]] || fail "$2 got $got from whoami, not 401: it is still live"
}

# stored: whether a login is kept for the server.
stored() {
  if [[ -n "$KEYCHAIN" ]]; then
    security find-generic-password -s cases -a "$KEY" > /dev/null 2>&1
  else
    [[ -f "$CREDS" ]] && grep -qF "[\"$SERVER\"]" "$CREDS"
  fi
}

# mode FILE: the file's permission bits, such as 600.
mode() {
  if [[ "$(uname -s)" == Darwin ]]; then
    stat -f %Lp "$1"
  else
    stat -c %a "$1"
  fi
}

# On the way out: when a step failed, log out what was signed in, as best it
# can, and say what is left live. Then the private directory goes.
finish() {
  local code=$? left=""
  set +e
  if [[ -z "$DONE" ]]; then
    printf '\nPassed: %s\n' "${PASSED[*]:-none}" >&2
    if [[ -f "$AGENT" ]]; then
      echo "cleaning up: cases logout --file agent.token" >&2
      if ! "$BIN" logout --file "$AGENT" > /dev/null 2> "$WORK/cleanup.err"; then
        left="the agent token in $AGENT"
      fi
    fi
    if stored; then
      echo "cleaning up: cases logout" >&2
      if ! "$BIN" logout > /dev/null 2> "$WORK/cleanup.err"; then
        left="${left:+$left, and }the stored login"
      fi
    fi
    if [[ -n "$left" ]]; then
      grep -vE "$TOKEN_SHAPE" "$WORK/cleanup.err" >&2
      echo "NOT REVOKED: $left. Revoke it at $SERVER/tokens." >&2
      if [[ -n "$KEYCHAIN" ]] && stored; then
        echo "The keychain still holds the login for $KEY: cases logout --forget $KEY removes it." >&2
      fi
    else
      echo "No token this run made is left live." >&2
    fi
  fi
  rm -rf "$WORK"
  exit "$code"
}
trap finish EXIT
trap 'exit 130' INT TERM

if [[ -n "$KEYCHAIN" ]]; then
  echo "This run keeps the login in the macOS keychain (CASES_E2E_KEYCHAIN=1)."
  for key in "$SERVER" "$(printf '%s' "$SERVER" | tr '[:upper:]' '[:lower:]')"; do
    if security find-generic-password -s cases -a "$key" > /dev/null 2>&1; then
      DONE=1
      fail "the keychain holds a login for $key already, which this run would replace and revoke"
    fi
  done
  echo "The keychain may ask you to let security read the item: allow it once."
else
  echo "This run never touches the keychain (CASES_NO_KEYCHAIN=1): the login goes to a"
  echo "credentials file in a private temporary directory, removed at the end."
fi
echo "Server: $SERVER"
echo "Binary: $BIN"

step "1. cases login, with the loopback"
pause "Your browser will open the approval page for $SERVER. Approve it there."
run login cases login
expect 0
says login "Signed in to $SERVER"
run status cases login status
expect 0
run status-json cases login status --json
expect 0
KEY=$(jq -r .server "$WORK/status-json.out")
[[ "$KEY" == "$SERVER" ]] || fail "cases writes the server as $KEY: set CASES_E2E_SERVER=$KEY"
HANDLE=$(jq -r .account.handle "$WORK/status-json.out")
ACCOUNT=$(jq -r .account.id "$WORK/status-json.out")
[[ -n "$HANDLE" && "$HANDLE" != null ]] || fail "login status --json shows no account"
says status "$HANDLE ($ACCOUNT)"
if [[ -n "$KEYCHAIN" ]]; then
  FROM=keychain
  says status "the macOS keychain (cases login)"
else
  FROM=credentials
  says status "$CREDS (cases login)"
  [[ "$(mode "$CREDS")" == 600 ]] || fail "$CREDS is mode $(mode "$CREDS"), not 600"
fi
[[ "$(jq -r .from "$WORK/status-json.out")" == "$FROM" ]] || fail "login status --json is not from $FROM"
stored || fail "no login is kept for $SERVER"
auth "$WORK/first.auth"
pass

step "2. a command reaches the store with the stored login"
run stores cases stores
expect 0
run list cases list
expect 0
pass

# The store the agent token opens: its name, which --store for login takes,
# and its id, which the token's scope lists.
if [[ -n "$WANT_STORE" ]]; then
  PICK=$(jq -c --arg s "$WANT_STORE" \
    '[.stores[] | select(.id == $s or .name == $s)][0] // empty' "$WORK/status-json.out")
else
  PICK=$(jq -c '.default // empty | {id, name}' "$WORK/status-json.out")
fi
[[ -n "$PICK" ]] || fail "no store for the agent token: set CASES_E2E_STORE to one of the account's"
STORE_ID=$(jq -r .id <<< "$PICK")
STORE_NAME=$(jq -r .name <<< "$PICK")
OTHER=$(jq -r --arg id "$STORE_ID" '[.stores[] | select(.id != $id)][0].name // empty' \
  "$WORK/status-json.out")

step "3. cases login --device, which replaces the login"
pause "It will show a page and a code. Open the page in your browser, signed in to the account, and type the code."
run device cases login --device
expect 0
says device "Signed in to $SERVER"
auth "$WORK/second.auth"
if cmp -s "$WORK/first.auth" "$WORK/second.auth"; then
  fail "the stored token is the one from step 1"
fi
run status cases login status --json
expect 0
[[ "$(jq -r .account.id "$WORK/status.out")" == "$ACCOUNT" ]] || fail "the login is for another account"
[[ "$(jq -r .from "$WORK/status.out")" == "$FROM" ]] || fail "login status --json is not from $FROM"
revoked "$WORK/first.auth" "the token from step 1"
pass

step "4. cases login --file agent.token --store $STORE_NAME"
SAME='{account, name: .token.name, stores: .token.stores, from}'
jq -S "$SAME" "$WORK/status.out" > "$WORK/before.json"
pause "Your browser will open the approval page again, for a token that opens $STORE_NAME alone. Approve it there."
run agent cases login --file "$AGENT" --store "$STORE_NAME"
expect 0
says agent "Wrote a token for $HANDLE ($ACCOUNT) to $AGENT"
[[ -f "$AGENT" ]] || fail "$AGENT is not there"
[[ "$(mode "$AGENT")" == 600 ]] || fail "$AGENT is mode $(mode "$AGENT"), not 600"
auth_file "$AGENT" "$WORK/agent.auth"
auth "$WORK/third.auth"
cmp -s "$WORK/second.auth" "$WORK/third.auth" || fail "the stored login changed"
run status cases login status --json
expect 0
jq -S "$SAME" "$WORK/status.out" > "$WORK/after.json"
cmp -s "$WORK/before.json" "$WORK/after.json" || fail "login status changed after --file"
run agent-status env CASES_TOKEN_FILE="$AGENT" "$BIN" login status --json
expect 0
[[ "$(jq -r .from "$WORK/agent-status.out")" == env-file ]] || fail "the agent token is not from CASES_TOKEN_FILE"
[[ "$(jq -c .token.stores "$WORK/agent-status.out")" == "[\"$STORE_ID\"]" ]] \
  || fail "the agent token's scope is not $STORE_NAME ($STORE_ID) alone"
[[ "$(jq -c '[.stores[].id]' "$WORK/agent-status.out")" == "[\"$STORE_ID\"]" ]] \
  || fail "whoami for the agent token lists stores other than $STORE_NAME"
run agent-list env CASES_TOKEN_FILE="$AGENT" "$BIN" list --store "$STORE_NAME"
expect 0
if [[ -n "$OTHER" ]]; then
  run agent-other env CASES_TOKEN_FILE="$AGENT" "$BIN" list --store "$OTHER"
  [[ "$CODE" != 0 ]] || fail "the agent token opened $OTHER"
else
  echo "the account has one store: no other store to try the agent token on"
fi
pass

step "5. cases logout --file agent.token"
run logout-file cases logout --file "$AGENT"
expect 0
says logout-file "and removed the file"
[[ ! -e "$AGENT" ]] || fail "$AGENT is still there"
revoked "$WORK/agent.auth" "the agent token"
pass

step "6. cases logout"
run logout cases logout
expect 0
says logout "Signed out of $SERVER"
run status cases login status
expect 1
if stored; then
  fail "a login is still kept for $SERVER"
fi
revoked "$WORK/second.auth" "the token from step 3"
pass

step "7. cases logout --forget, with no login"
run forget cases logout --forget
expect 1
says forget "not signed in to $SERVER"
pass

DONE=1
printf '\nPassed:\n'
printf '  %s\n' "${PASSED[@]}"
printf '\nE2E LOGIN PASSED\n'
