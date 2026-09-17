.PHONY: build install lint test cover fmt tidy clean

BINARY := cases

VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo none)
DATE    ?= $(shell date -u +%Y-%m-%dT%H:%M:%SZ)
LDFLAGS := -X main.version=$(VERSION) -X main.commit=$(COMMIT) -X main.date=$(DATE)

build:
	go build -ldflags "$(LDFLAGS)" -o $(BINARY) ./cmd/cases

install:
	go install -ldflags "$(LDFLAGS)" ./cmd/cases

lint:
	golangci-lint run ./...

test:
	go test -race ./...

cover:
	go test -race -coverprofile=coverage.out ./...
	go tool cover -func=coverage.out

fmt:
	golangci-lint fmt ./...

tidy:
	go mod tidy

clean:
	rm -f $(BINARY) coverage.out
