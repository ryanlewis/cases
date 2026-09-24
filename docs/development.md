# Development

```sh
make build   # ./cases
make test    # go test -race ./...
make lint    # golangci-lint run ./...
make fmt     # golangci-lint fmt ./...
```

The store uses `modernc.org/sqlite`, SQLite translated to Go, so a build needs
no C compiler and cross-compiles like any Go program. Its go.mod pins the
`modernc.org/libc` it was built against, and this module must pin the same
version, so update the two together.

`govulncheck` is pinned as a `tool` directive in go.mod, so its dependency
graph (`golang.org/x/vuln` and its own dependencies) shows up in go.mod and
go.sum alongside the runtime dependencies.

To cut a release, push a semver tag: `git tag vX.Y.Z && git push origin
vX.Y.Z`. `.github/workflows/release.yml` runs the tests, then goreleaser
(`.goreleaser.yaml`) builds binaries for linux, darwin and windows, signs and
notarizes the darwin ones, publishes them with a `checksums.txt` file as a
GitHub release, attests their build provenance and updates the cask in
[ryanlewis/homebrew-tap](https://github.com/ryanlewis/homebrew-tap). A
prerelease tag such as `vX.Y.Z-rc.1` is published as a GitHub prerelease and
leaves the cask alone. The release job takes the signing, notarization and
tap secrets from the `release` environment and stops before building if any
is missing.

Set up the `release` environment before pushing the first tag: a run that
names a missing environment creates it with no protection rules. Give it a
deployment rule that allows only `v*` tags, and these secrets:

- `MACOS_SIGN_P12`: the Developer ID Application certificate and its private
  key as a `.p12` file, base64-encoded
- `MACOS_SIGN_PASSWORD`: the password of the `.p12`
- `MACOS_NOTARY_ISSUER_ID` and `MACOS_NOTARY_KEY_ID`: the issuer ID and key
  ID of an App Store Connect API key (the Developer role is enough)
- `MACOS_NOTARY_KEY`: that key's `.p8` file, base64-encoded; its PEM text
  pasted as is fails at notarization
- `HOMEBREW_TAP_GITHUB_TOKEN`: a fine-grained token with read and write
  access to the contents of ryanlewis/homebrew-tap only

```sh
base64 -i cert.p12 | gh secret set MACOS_SIGN_P12 --env release -R ryanlewis/cases
base64 -i AuthKey_<key-id>.p8 | gh secret set MACOS_NOTARY_KEY --env release -R ryanlewis/cases
```

`goreleaser check` validates the config. `goreleaser release --snapshot
--clean` builds everything into `dist/` without signing or publishing.

## Screenshots

The README's screenshots come from a demo store. `scripts/screenshots/seed.sh`
fills a new store with made-up cases, and `scripts/screenshots/capture.cjs`
takes the pictures from a running `cases serve` with Playwright, light and
dark:

```sh
make build
dir=$(mktemp -d)
sh scripts/screenshots/seed.sh ./cases "$dir/demo.db"
: > "$dir/config.toml"
XDG_STATE_HOME="$dir/state" ./cases --store "$dir/demo.db" --config "$dir/config.toml" \
  serve --no-open --listen 127.0.0.1:8799 > "$dir/serve.log" 2>&1 &
until curl -sf -o /dev/null http://127.0.0.1:8799/; do sleep 0.2; done
NODE_PATH=/path/to/node_modules node scripts/screenshots/capture.cjs http://127.0.0.1:8799 docs/images
kill %1
```

`NODE_PATH` names a `node_modules` that has the `playwright` package. Set
`CHROMIUM` to a Chromium binary if Playwright has not downloaded its own.
Reduce the images to 256 colours, with `pngquant` or Pillow's `quantize`,
which keeps each under 100 KB.
