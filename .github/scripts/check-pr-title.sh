#!/usr/bin/env bash
# Check that a pull request's title is a Conventional Commit subject. PRs are
# squash-merged, so the title becomes the commit's subject on main.
#
# Usage: check-pr-title.sh TITLE, or with the title in $TITLE.
set -euo pipefail

title=${1-${TITLE-}}
pattern='^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([a-z0-9-]+\))?!?: [^[:space:]]'
problems=()

if [[ ! "$title" =~ $pattern ]]; then
  problems+=("It is not a Conventional Commit subject, such as \"fix(core): read a negative amend_seq as malformed\".")
fi
if [[ "$title" == *. ]]; then
  problems+=("It ends with a period.")
fi
if (( ${#title} > 72 )); then
  problems+=("It is ${#title} characters long; the most is 72.")
fi

if (( ${#problems[@]} )); then
  printf 'The PR title "%s" cannot be the squash commit subject:\n' "$title" >&2
  printf -- '- %s\n' "${problems[@]}" >&2
  exit 1
fi
echo "The PR title is a Conventional Commit subject."
