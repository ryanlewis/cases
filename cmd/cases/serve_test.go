package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/instance"
	"github.com/ryanlewis/cases/internal/store"
)

func TestServeRefusesNonLoopback(t *testing.T) {
	for _, addr := range []string{"0.0.0.0:8765", ":8765", "192.168.1.10:8765"} {
		r := runCases(t, "", "--store", t.TempDir(), "serve", "--listen", addr)
		if r.err == nil || !strings.Contains(r.err.Error(), "loopback") {
			t.Errorf("--listen %s: err = %v", addr, r.err)
		}
	}
}

// syncBuffer lets the test read what serve prints while it is running.
type syncBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

func TestServeAnswersAndStops(t *testing.T) {
	root := t.TempDir()
	id := openDecision(t, root)

	ctx, cancel := context.WithCancel(context.Background())
	stdout, stderr := &syncBuffer{}, &syncBuffer{}
	done := make(chan error, 1)
	go func() {
		cmd := &ServeCmd{Listen: "127.0.0.1:0"}
		done <- cmd.Run(&Deps{Store: root, Stdout: stdout, Stderr: stderr, Context: ctx})
	}()

	urlPattern := regexp.MustCompile(`http://127\.0\.0\.1:\d+/`)
	var base string
	for deadline := time.Now().Add(5 * time.Second); base == "" && time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
		base = urlPattern.FindString(stdout.String())
	}
	if base == "" {
		t.Fatalf("serve printed no URL: %q", stdout.String())
	}

	resp, err := http.Get(base + "cases/" + id)
	if err != nil {
		t.Fatal(err)
	}
	body, _ := io.ReadAll(resp.Body)
	resp.Body.Close()
	if resp.StatusCode != http.StatusOK || !strings.Contains(string(body), "Pin bun?") {
		t.Errorf("GET case: %d", resp.StatusCode)
	}
	if !strings.Contains(stderr.String(), "GET /cases/"+id+" 200") {
		t.Errorf("request not logged: %q", stderr.String())
	}

	cancel()
	select {
	case err := <-done:
		if err != nil {
			t.Errorf("serve returned %v", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("serve did not stop")
	}
}

func TestServeWithoutTerminalPrintsURLAndOpensNothing(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	stdout := &syncBuffer{}
	var opened atomic.Int32
	done := make(chan error, 1)
	go func() {
		cmd := &ServeCmd{Listen: "127.0.0.1:0"}
		done <- cmd.Run(&Deps{Store: t.TempDir(), Stdout: stdout, Stderr: io.Discard, Context: ctx,
			OpenURL: func(string) error { opened.Add(1); return nil }})
	}()

	urlPattern := regexp.MustCompile(`^Serving .* at http://127\.0\.0\.1:\d+/\n$`)
	for deadline := time.Now().Add(5 * time.Second); !urlPattern.MatchString(stdout.String()) && time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
	}
	cancel()
	if err := <-done; err != nil {
		t.Errorf("serve returned %v", err)
	}
	if !urlPattern.MatchString(stdout.String()) {
		t.Errorf("stdout = %q, want only the Serving line", stdout.String())
	}
	if n := opened.Load(); n != 0 {
		t.Errorf("browser opened %d times with stdout not a terminal", n)
	}
}

func TestServeScreenStats(t *testing.T) {
	root := t.TempDir()
	start := time.Date(2026, 9, 16, 12, 0, 0, 0, time.Local)
	before, after := start.Add(-time.Hour), start.Add(time.Minute)
	open := func(kind store.Kind, urgency store.Urgency, options ...string) *store.Case {
		t.Helper()
		c, err := store.Create(root, store.OpenRecord{Kind: kind, Urgency: urgency, Title: string(kind) + " " + string(urgency), Options: options, OpenedAt: before})
		if err != nil {
			t.Fatal(err)
		}
		return c
	}
	must := func(_ *store.Case, err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}

	open(store.KindFYI, store.UrgencyBlocking)
	open(store.KindFYI, store.UrgencyBlocking)
	open(store.KindFYI, store.UrgencyToday)
	parked := open(store.KindStuck, store.UrgencyWhenever)
	must(store.Park(parked.Dir, store.ParkRecord{ParkedAt: after}))
	answered := open(store.KindDecision, store.UrgencyToday, "a", "b")
	must(store.Answer(answered.Dir, store.AnswerRecord{Choice: 1, AnsweredAt: before}))
	closed := open(store.KindFYI, store.UrgencyWhenever)
	must(store.Answer(closed.Dir, store.AnswerRecord{Ack: true, AnsweredAt: after}))
	must(store.Pickup(closed.Dir, store.PickupRecord{PickedUpAt: after}))
	must(store.Close(closed.Dir, store.CloseRecord{Outcome: "done", ClosedAt: after.Add(time.Minute)}))

	cases, bad, err := store.List(root)
	if err != nil || len(bad) > 0 {
		t.Fatalf("list: %v %v", err, bad)
	}
	now := after.Add(5 * time.Minute)
	lines := render(screenView{
		Store:    root,
		URL:      "http://127.0.0.1:8765/",
		Stats:    countCases(cases, start, now),
		Uptime:   now.Sub(start),
		Requests: 1,
		Log:      []string{"GET / 200 1ms"},
		Keys:     true,
		Now:      now,
	})
	got := strings.Join(lines, "\n")
	for _, want := range []string{
		"cases serve  ·  up 6m",
		"inbox       http://127.0.0.1:8765/",
		"open          3   2 blocking · 1 today · 0 whenever",
		"parked        1",
		"with agent    1   1 answered · 0 picked up",
		"closed        1   today · 1 in all",
		"since start 1 request · 1 answer · 1 park · 0 resumes",
		"last event  4m ago",
		"  GET / 200 1ms",
		agentHint[0],
		"q quit   Ctrl-C quit",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("screen lacks %q:\n%s", want, got)
		}
	}
	if strings.Contains(got, "\x1b[") {
		t.Errorf("colour codes without Color:\n%s", got)
	}
	if colored := strings.Join(render(screenView{Stats: countCases(cases, start, now), Color: true, Now: now}), "\n"); !strings.Contains(colored, "\x1b[1;31m2 blocking\x1b[0m") {
		t.Errorf("blocking count not highlighted:\n%q", colored)
	}
}

func TestServeFailsWhenAddressIsTaken(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	root := t.TempDir()
	r := runCases(t, "", "--store", root, "serve", "--listen", ln.Addr().String())
	if r.err == nil || !strings.Contains(r.err.Error(), "address already in use") {
		t.Errorf("err = %v, want address already in use", r.err)
	}
	if r.stdout != "" {
		t.Errorf("stdout = %q, want nothing", r.stdout)
	}
	// Nothing was recorded for cases status.
	path, _ := instance.Path(root)
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("state file after a failed listen: %v", err)
	}
}

func TestServeWithoutStateDirectoryStillServes(t *testing.T) {
	file := filepath.Join(t.TempDir(), "state")
	if err := os.WriteFile(file, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("XDG_STATE_HOME", file)

	ctx, cancel := context.WithCancel(context.Background())
	stdout, stderr := &syncBuffer{}, &syncBuffer{}
	done := make(chan error, 1)
	go func() {
		cmd := &ServeCmd{Listen: "127.0.0.1:0"}
		done <- cmd.Run(&Deps{Store: t.TempDir(), Stdout: stdout, Stderr: stderr, Context: ctx})
	}()
	base := waitForURL(t, stdout)
	resp, err := http.Get(base)
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	cancel()
	if err := <-done; err != nil {
		t.Errorf("serve returned %v", err)
	}
	if resp.StatusCode != http.StatusOK {
		t.Errorf("GET / = %d", resp.StatusCode)
	}
	if !strings.Contains(stderr.String(), "could not record the instance for cases status: ") {
		t.Errorf("stderr = %q, want the failed record logged", stderr.String())
	}
}

// waitForURL waits for serve to print its URL and returns it.
func waitForURL(t *testing.T, stdout *syncBuffer) string {
	t.Helper()
	urlPattern := regexp.MustCompile(`http://127\.0\.0\.1:\d+/`)
	for deadline := time.Now().Add(5 * time.Second); time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
		if url := urlPattern.FindString(stdout.String()); url != "" {
			return url
		}
	}
	t.Fatalf("serve printed no URL: %q", stdout.String())
	return ""
}
