#!/usr/bin/env bash
# Writes the Homebrew cask for a release to stdout, from its SHA256SUMS. The
# release job pushes it to ryanlewis/homebrew-tap as Casks/cases.rb, so
# `brew install ryanlewis/tap/cases` installs the release's binary.
#
#   cask.sh VERSION SHA256SUMS
set -euo pipefail

version=$1
sums=$2

sha() {
  local name="cases-$version-$1.tar.gz" sha
  sha=$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$sums")
  if [[ ! $sha =~ ^[0-9a-f]{64}$ ]]; then
    echo "cask.sh: no checksum for $name in $sums" >&2
    exit 1
  fi
  echo "$sha"
}

mac_arm=$(sha aarch64-apple-darwin)
mac_x64=$(sha x86_64-apple-darwin)
linux_arm=$(sha aarch64-unknown-linux-musl)
linux_x64=$(sha x86_64-unknown-linux-musl)

cat <<EOF
# Written by the release workflow of ryanlewis/cases. Do not edit.
cask "cases" do
  version "$version"

  on_macos do
    on_arm do
      sha256 "$mac_arm"
      url "https://github.com/ryanlewis/cases/releases/download/v#{version}/cases-#{version}-aarch64-apple-darwin.tar.gz"
    end
    on_intel do
      sha256 "$mac_x64"
      url "https://github.com/ryanlewis/cases/releases/download/v#{version}/cases-#{version}-x86_64-apple-darwin.tar.gz"
    end
  end
  on_linux do
    on_arm do
      sha256 "$linux_arm"
      url "https://github.com/ryanlewis/cases/releases/download/v#{version}/cases-#{version}-aarch64-unknown-linux-musl.tar.gz"
    end
    on_intel do
      sha256 "$linux_x64"
      url "https://github.com/ryanlewis/cases/releases/download/v#{version}/cases-#{version}-x86_64-unknown-linux-musl.tar.gz"
    end
  end

  name "cases"
  desc "Human-in-the-loop inbox for agents"
  homepage "https://github.com/ryanlewis/cases"

  livecheck do
    skip "Written on release."
  end

  binary "cases"
end
EOF
