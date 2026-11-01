#!/usr/bin/env bash
# Packs a release binary with LICENSE and README.md into a .tar.gz, with no
# directory inside, so `tar -xzf ARCHIVE -C ~/.local/bin cases` installs it.
#
#   pack.sh BINARY ARCHIVE
set -euo pipefail

bin=$1
archive=$2

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp "$bin" "$stage/cases"
cp LICENSE README.md "$stage/"
mkdir -p "$(dirname "$archive")"
# Owned by uid 0 with no names, so extracting as root does not hand the
# binary to whichever uid built it; and no xattrs or AppleDouble ._ files from
# macOS. bsdtar (macOS) and GNU tar (Linux) spell the owner flags differently.
if [[ $(tar --version) == *"GNU tar"* ]]; then
  owner=(--owner=0 --group=0 --numeric-owner)
else
  owner=(--uid 0 --gid 0 --numeric-owner)
fi
COPYFILE_DISABLE=1 tar --no-xattrs "${owner[@]}" -czf "$archive" -C "$stage" cases LICENSE README.md
tar -tzvf "$archive"
