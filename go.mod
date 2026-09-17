module github.com/ryanlewis/cases

go 1.26.0

toolchain go1.26.8

require (
	github.com/alecthomas/kong v1.16.1
	github.com/yuin/goldmark v1.8.6
)

require github.com/pelletier/go-toml/v2 v2.2.4

require (
	golang.org/x/mod v0.41.0 // indirect
	golang.org/x/sync v0.23.0 // indirect
	golang.org/x/sys v0.48.0 // indirect
	golang.org/x/telemetry v0.0.0-20260908163034-4bcc4b2ee518 // indirect
	golang.org/x/tools v0.50.0 // indirect
	golang.org/x/vuln v1.8.0 // indirect
)

tool golang.org/x/vuln/cmd/govulncheck
