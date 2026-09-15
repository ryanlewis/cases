package main

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"
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
