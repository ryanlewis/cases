#!/usr/bin/env bash
# End-to-end run of the built `cases` binary against a real hosted server,
# with a stand-in human who answers through the server's inbox route.
#
# It writes cases to the store it uses, so point it at a test store, never
# at one with real cases in it. It is opt-in: it never runs in CI.
#
# Usage:
#   CASES_E2E_SERVER=https://host CASES_E2E_TOKEN_FILE=path/to/token \
#     [CASES_E2E_STORE=<store name or id>] [CASES_BIN=path/to/cases] \
#     scripts/e2e-remote.sh
#
# CASES_E2E_SERVER   the server, https://host[:port]
# CASES_E2E_TOKEN_FILE  a file holding the token, one line; keep it mode 600
# CASES_E2E_STORE    the store to use; the account's default when unset
# CASES_BIN          the binary; target/debug/cases when unset (cargo build --features remote)
#
# Needs curl and jq. The token goes to the binary through a config file's
# token-file and to curl through a header file in a private temporary
# directory. It is never printed or put in a command line.
set -euo pipefail

if [[ -n "${CI:-}" ]]; then
  echo "e2e-remote.sh talks to a real server and does not run in CI" >&2
  exit 1
fi
SERVER=${CASES_E2E_SERVER:?set CASES_E2E_SERVER to the server, https://host}
TOKEN_FILE=${CASES_E2E_TOKEN_FILE:?set CASES_E2E_TOKEN_FILE to a file holding the token}
STORE=${CASES_E2E_STORE:-}
SERVER=${SERVER%/}
BIN=${CASES_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/debug/cases}
for tool in curl jq; do
  command -v "$tool" > /dev/null || { echo "e2e-remote.sh needs $tool" >&2; exit 1; }
done
[[ -x "$BIN" ]] || { echo "no binary at $BIN: run cargo build --features remote, or set CASES_BIN" >&2; exit 1; }
[[ -r "$TOKEN_FILE" ]] || { echo "cannot read the token file $TOKEN_FILE" >&2; exit 1; }
TOKEN_FILE=$(cd "$(dirname "$TOKEN_FILE")" && pwd)/$(basename "$TOKEN_FILE")

umask 077
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# A home of its own, so that nothing of the user's config, state or
# environment is read, and the account's list is kept in $WORK.
unset CASES_TOKEN CASES_TOKEN_FILE CASES_SERVER CASES_STORE CASES_WORKER
# Never the keychain: a login it reads would be the user's.
export CASES_NO_KEYCHAIN=1
export XDG_CONFIG_HOME="$WORK/config" XDG_STATE_HOME="$WORK/state" XDG_DATA_HOME="$WORK/data"
export CASES_CONFIG="$WORK/config.toml"
{
  printf 'server = "%s"\n' "$SERVER"
  printf 'token-file = "%s"\n' "$TOKEN_FILE"
  if [[ -n "$STORE" ]]; then
    printf 'default-store = "%s"\n' "$STORE"
  fi
} > "$CASES_CONFIG"

# curl's header, read from the token file without printing it.
{
  printf 'Authorization: Bearer '
  tr -d '[:space:]' < "$TOKEN_FILE"
  printf '\n'
} > "$WORK/auth"

cases() { "$BIN" "$@"; }
step() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAILED: %s\n' "$*" >&2; exit 1; }

# expect CODE COMMAND...: runs the command, which must exit CODE.
expect() {
  local want=$1 got=0
  shift
  "$@" || got=$?
  echo "exit $got"
  [[ "$got" == "$want" ]] || fail "$* exited $got, not $want"
}

step "stores"
if cases stores --help > /dev/null 2>&1; then
  cases stores
  STORE_ID=$(cases stores --json | jq -r '.selected.id // empty')
else
  echo "skipped: this cases has no stores command"
  STORE_ID=$(curl -sS --fail-with-body -H @"$WORK/auth" "$SERVER/v1/cli/whoami" \
    | jq -r --arg s "$STORE" \
      '.stores | (map(select(.id == $s or .name == $s)) + map(select(.default)) + .)[0].id // empty')
fi
[[ -n "$STORE_ID" ]] || fail "no store to use: set CASES_E2E_STORE"
echo "store $STORE_ID"
B="$SERVER/v1/stores/$STORE_ID"

# answer ID BODY: the stand-in human writes through the inbox route.
answer() {
  curl -sS --fail-with-body -H @"$WORK/auth" -H 'content-type: application/json' \
    -d "$2" "$B/cases/$1/answer" | jq -c '{id, state, revision}'
}

step "open"
ID=$(cases open --kind decision --urgency today --title "e2e $(date -u +%H:%M:%S)" \
  --option "Pin" --option "Float" --worker e2e-agent)
echo "id $ID"
cases show "$ID" --json | jq -c '{id, state, revision, worker}'

step "wait --pickup, while the stand-in human answers through the inbox route"
cases wait --id "$ID" --pickup --by e2e-agent --timeout 90s > "$WORK/wait.out" 2> "$WORK/wait.err" &
WAIT=$!
sleep 4
REV=$(cases show "$ID" --json | jq .revision)
SENT=$(date +%s)
answer "$ID" "{\"revision\":$REV,\"answer\":{\"choice\":2,\"note\":\"e2e stand-in\"}}"
WCODE=0
wait "$WAIT" || WCODE=$?
echo "wait exit $WCODE, woke within $(($(date +%s) - SENT)) s of the answer"
cat "$WORK/wait.err"
jq -c '{id, state, revision, choice: .answer.choice, fresh}' "$WORK/wait.out"
[[ "$WCODE" == 0 ]] || fail "wait exited $WCODE"
[[ "$(jq -r .state "$WORK/wait.out")" == pickedup ]] || fail "wait did not pick the case up"

step "an answer that lands before the wait is caught with --since <id>"
EARLY=$(cases open --kind fyi --urgency today --title "e2e early answer" --worker e2e-agent)
REV=$(cases show "$EARLY" --json | jq .revision)
answer "$EARLY" "{\"revision\":$REV,\"answer\":{\"ack\":true}}"
cases wait --id "$EARLY" --since "$EARLY" --pickup --by e2e-agent --timeout 20s \
  | jq -c '{id, state, fresh}'
cases close "$EARLY" --outcome "Seen" > /dev/null

step "close"
cases close "$ID" --outcome "Floated, per the e2e answer" --json

step "stale: a note at revision 1 exits 4"
expect 4 cases note "$ID" --body "late" --revision 1

step "transition: closing again exits 3"
expect 3 cases close "$ID" --outcome "again"

step "a wait that times out exits 2, at its timeout"
START=$(date +%s)
expect 2 cases wait --id "$ID" --timeout 3s
TOOK=$(($(date +%s) - START))
echo "took $TOOK s"
((TOOK <= 6)) || fail "a 3 s wait took $TOOK s"

step "a wrong token exits 1 and is not printed"
WRONG="csu_$(printf 'e2e%.0s' {1..12})"
CODE=0
CASES_TOKEN=$WRONG cases list 2> "$WORK/bad.err" || CODE=$?
cat "$WORK/bad.err"
echo "exit $CODE"
[[ "$CODE" == 1 ]] || fail "a wrong token exited $CODE, not 1"
if grep -qF -e "$WRONG" -f <(tr -d '[:space:]' < "$TOKEN_FILE"; echo) "$WORK/bad.err"; then
  fail "a token was printed"
fi

printf '\nE2E PASSED\n'
