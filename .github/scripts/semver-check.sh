#!/usr/bin/env bash
# Check the public API of each published crate, cases-core and cases-id,
# against BASE (a commit or ref, such as origin/main) with cargo-semver-checks.
#
# While a crate's version is the same as at BASE, the change must not break
# its API: it is checked as a minor release, so a removed or changed public
# item fails. cargo-semver-checks would otherwise take an unchanged version as
# a major release and skip every check. To make a breaking change, bump the
# crate's version in the same PR; the check then takes the bump as the
# acknowledgement. At 0.0.x any bump, such as 0.0.0 to 0.0.1, is major. A
# crate that is not at BASE yet has no API to break, and is skipped.
#
# Usage: semver-check.sh BASE
set -euo pipefail

base=${1:?usage: semver-check.sh BASE}

version() {
  sed -n 's/^version = "\(.*\)"$/\1/p' | head -n 1
}

# check PACKAGE MANIFEST
check() {
  local package=$1 manifest=$2
  if ! git cat-file -e "$base:$manifest" 2>/dev/null; then
    echo "$package is not at $base: there is no API to check it against."
    return
  fi
  local base_version head_version
  base_version=$(git show "$base:$manifest" | version)
  head_version=$(version < "$manifest")

  local args=(-p "$package" --baseline-rev "$base")
  if [[ "$base_version" == "$head_version" ]]; then
    echo "$package is $head_version, as at $base: a breaking change fails this check."
    echo "To make one, bump $package's version in this PR."
    args+=(--release-type minor)
  else
    echo "$package goes from $base_version to $head_version: checked against that bump."
  fi
  cargo semver-checks "${args[@]}"
}

check cases-core crates/core/Cargo.toml
check cases-id crates/id/Cargo.toml
