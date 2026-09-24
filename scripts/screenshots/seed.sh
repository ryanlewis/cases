#!/bin/sh
# Seed a demo store for the README screenshots. Usage: seed.sh CASES STORE
set -eu

bin=$1
store=$2

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
: >"$tmp/config.toml"
c() { env -u CASES_STORE -u CASES_WORKER -u CASES_LABEL "$bin" --store "$store" --config "$tmp/config.toml" "$@"; }

# Cases that are done, so the done list and the counts have something in them.
id=$(c open --kind question --urgency today --title "Which log level for the worker in staging?" \
  --body "The worker logs at debug in staging, which is most of the log volume." \
  --label log-cleanup --worker session-2)
c answer "$id" --text "info, and debug only behind the flag." >/dev/null
c pickup "$id" --by session-2 >/dev/null
c close "$id" --outcome "Set to info in staging. Debug stays behind \`LOG_DEBUG\`." >/dev/null

id=$(c open --kind fyi --urgency whenever --title "Nightly backup took 40 minutes" \
  --body "Up from 12 minutes last week. The new attachments table is most of it." \
  --worker session-5)
c answer "$id" --ack >/dev/null

# Open cases, one of each kind.
c open --kind signoff --urgency whenever --title "Review the rewritten onboarding guide" \
  --body "The guide now starts from a clean clone and ends with a passing test run. Sections on the old build script are gone." \
  --link https://example.com/pr/218 --label docs --worker session-4 >/dev/null

c open --kind question --urgency today --title "Keep the v1 export endpoint?" \
  --body "Nothing has called \`/v1/export\` in 30 days. Remove it in this release, or keep it one more?" \
  --label api-cleanup --worker session-2 >/dev/null

c open --kind approval --urgency today --title "Run the data migration on staging" \
  --body "The migration adds the \`archived_at\` column and backfills it. Each step is safe to run again." \
  --row '{"id":"schema","label":"Add the column","script":"make migrate STEP=0042","link":"https://example.com/migrations/0042"}' \
  --row '{"id":"backfill","label":"Backfill archived rows","script":"make backfill TABLE=projects","link":"https://example.com/migrations/0042","note":"About 20 minutes on staging data."}' \
  --label migrations --worker session-3 >/dev/null

c open --kind stuck --urgency blocking --title "Integration tests time out in CI" \
  --body "The integration suite passes locally in 3 minutes and times out after 20 in CI. The runner has half the memory." \
  --brief "notes/ci-timeouts.md" --label ci --worker session-1 >/dev/null

cat >"$tmp/body.md" <<'EOF'
The upload handler holds each file in memory until it is written. Files over
about 200 MB now fail on the smaller instances.

| Option | Change | Risk |
| --- | --- | --- |
| Stream to disk | handler only | low |
| Raise the limit | config | memory use grows with it |

Streaming is about a day of work. Raising the limit is a one-line change but
only moves the problem.
EOF
c open --kind decision --urgency blocking --title "Stream uploads or raise the size limit?" \
  --body-file "$tmp/body.md" --option "Stream uploads to disk" --option "Raise the limit to 1 GB" \
  --link https://example.com/issues/131 --label uploads --worker session-1 \
  --context "Two users hit this today." >/dev/null
