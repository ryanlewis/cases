#!/usr/bin/env bash
# Runs a built `cases` and checks that `cases --version` prints VERSION.
#
#   check-version.sh BINARY VERSION
set -euo pipefail

bin=$1
want=$2

got=$("$bin" --version)
echo "$got"
if [[ "$got" != "cases $want" ]]; then
  echo "::error::$bin --version printed '$got', not 'cases $want'."
  exit 1
fi
