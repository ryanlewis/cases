package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// frameLines splits one draw's output into the lines it wrote, checking each
// clears its row first.
func frameLines(t *testing.T, out string) []string {
	t.Helper()
	body, ok := strings.CutSuffix(out, "\x1b[J")
	if !ok {
		t.Fatalf("frame does not clear below itself: %q", out)
	}
	var lines []string
	for _, l := range strings.SplitAfter(body, "\n") {
		if l == "" {
			continue
		}
		rest, ok := strings.CutPrefix(l, "\r\x1b[2K")
		if !ok || !strings.HasSuffix(rest, "\n") {
			t.Fatalf("line not cleared and ended: %q", l)
		}
		lines = append(lines, strings.TrimSuffix(rest, "\n"))
	}
	return lines
}

func TestScreenDrawRedrawsInPlace(t *testing.T) {
	root := t.TempDir()
	var out bytes.Buffer
	s := newStatusScreen(root, store.NewDir(root), "http://127.0.0.1:8765/", &out, false)

	s.draw(false, 0)
	first := out.String()
	if !strings.HasPrefix(first, "\r\x1b[2K") {
		t.Errorf("first frame moves the cursor up: %q", first)
	}
	lines := frameLines(t, first)
	got := strings.Join(lines, "\n")
	for _, want := range []string{
		"inbox       http://127.0.0.1:8765/",
		"store       " + root,
		"open          0   0 blocking · 0 today · 0 whenever",
		"since start 0 requests · 0 answers · 0 parks · 0 resumes · 0 notifications",
		"last event  none",
		"  nothing yet",
		"Ctrl-C quit",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("frame lacks %q:\n%s", want, got)
		}
	}
	if strings.Contains(got, "q quit") || strings.Contains(got, "\x1b[") {
		t.Errorf("frame offers q without keys, or has colour:\n%s", got)
	}
	if s.drawn != len(lines) {
		t.Errorf("drawn = %d, want %d", s.drawn, len(lines))
	}

	// Nothing changed: nothing written.
	out.Reset()
	s.draw(false, 0)
	if out.Len() != 0 {
		t.Errorf("identical frame redrawn: %q", out.String())
	}

	// A new case and a request: move up over the last frame and draw again.
	if _, err := store.Create(root, store.OpenRecord{Kind: store.KindFYI, Urgency: store.UrgencyBlocking, Title: "Look"}); err != nil {
		t.Fatal(err)
	}
	out.Reset()
	s.draw(true, 1)
	up, body, ok := strings.Cut(out.String(), "A")
	if !ok || up != "\x1b["+strconv.Itoa(len(lines)) {
		t.Fatalf("redraw does not move up %d lines: %q", len(lines), out.String())
	}
	got = strings.Join(frameLines(t, body), "\n")
	for _, want := range []string{
		"open          1   1 blocking · 0 today · 0 whenever",
		"since start 1 request · 0 answers",
		"last event  just now",
		"q quit   Ctrl-C quit",
	} {
		if !strings.Contains(got, want) {
			t.Errorf("redraw lacks %q:\n%s", want, got)
		}
	}
}

func TestScreenDrawStoreStates(t *testing.T) {
	t.Run("missing store is empty", func(t *testing.T) {
		var out bytes.Buffer
		absent := filepath.Join(t.TempDir(), "absent")
		newStatusScreen(absent, store.NewDir(absent), "u", &out, false).draw(false, 0)
		if got := out.String(); strings.Contains(got, "cannot read the store") || !strings.Contains(got, "open          0") {
			t.Errorf("frame = %q, want an empty store and no error", got)
		}
	})
	t.Run("unreadable store is reported", func(t *testing.T) {
		file := filepath.Join(t.TempDir(), "store")
		if err := os.WriteFile(file, nil, 0o600); err != nil {
			t.Fatal(err)
		}
		var out bytes.Buffer
		newStatusScreen(file, store.NewDir(file), "u", &out, true).draw(false, 0)
		if got := out.String(); !strings.Contains(got, "\x1b[1;31mcannot read the store: ") {
			t.Errorf("frame = %q, want the error in red", got)
		}
	})
}

func TestScreenLogWriter(t *testing.T) {
	var stderr bytes.Buffer
	s := newScreenAt(t, "u", &bytes.Buffer{}, false)
	w := s.logWriter(&stderr, true)

	// A line split across writes is kept once it ends.
	for _, p := range []string{"GET / 2", "00 1ms\nGET /a 200", " 1ms\n"} {
		if n, err := w.Write([]byte(p)); n != len(p) || err != nil {
			t.Fatalf("Write(%q) = %d, %v", p, n, err)
		}
	}
	if want := []string{"GET / 200 1ms", "GET /a 200 1ms"}; strings.Join(s.log, "|") != strings.Join(want, "|") {
		t.Errorf("log = %q, want %q", s.log, want)
	}
	if stderr.String() != "GET / 200 1ms\nGET /a 200 1ms\n" {
		t.Errorf("stderr = %q, want the writes passed through", stderr.String())
	}

	// Only the last logLines lines are kept, and a partial line waits.
	if _, err := w.Write([]byte("1\n2\n3\n4\n5\n6\n7")); err != nil {
		t.Fatal(err)
	}
	if want := []string{"2", "3", "4", "5", "6"}; strings.Join(s.log, "|") != strings.Join(want, "|") || string(s.part) != "7" {
		t.Errorf("log = %q, part = %q; want %q and 7", s.log, s.part, want)
	}
}

func TestScreenLogWriterWithoutTee(t *testing.T) {
	var stderr bytes.Buffer
	s := newScreenAt(t, "u", &bytes.Buffer{}, false)
	if _, err := s.logWriter(&stderr, false).Write([]byte("GET / 200\n")); err != nil {
		t.Fatal(err)
	}
	if stderr.Len() != 0 || len(s.log) != 1 {
		t.Errorf("stderr = %q, log = %q; want nothing on stderr, one line kept", stderr.String(), s.log)
	}
}

func TestScreenLogWriterStderrFails(t *testing.T) {
	s := newScreenAt(t, "u", &bytes.Buffer{}, false)
	broken := errors.New("broken pipe")
	n, err := s.logWriter(writerFunc(func([]byte) (int, error) { return 0, broken }), true).Write([]byte("GET /\n"))
	if n != 0 || !errors.Is(err, broken) || len(s.log) != 0 {
		t.Errorf("Write = %d, %v, log %q; want 0, the stderr error, nothing kept", n, err, s.log)
	}
}

// TestScreenRunWithoutTerminal runs the screen with stdin that is not a
// terminal. Reading keys from a real terminal, raw mode and clipping to the
// terminal's height need a terminal and are not tested.
func TestScreenRunWithoutTerminal(t *testing.T) {
	out := &syncBuffer{}
	s := newScreenAt(t, "u", out, false)
	var requests atomic.Int64
	requests.Store(3)
	quit := func() { t.Error("quit called without a key") }
	restore := s.run(context.Background(), quit, strings.NewReader("q"), &requests)
	for deadline := time.Now().Add(5 * time.Second); !strings.Contains(out.String(), "3 requests") && time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
	}
	// The ticker redraws only after a second, so the frame showing 4 is the
	// one restore draws on the way out.
	requests.Store(4)
	restore()

	got := out.String()
	body, ok := strings.CutPrefix(got, "\x1b[?25l\x1b[?7l")
	if !ok {
		t.Fatalf("cursor not hidden and wrapping not turned off first: %q", got)
	}
	body, ok = strings.CutSuffix(body, "\x1b[?7h\x1b[?25h")
	if !ok {
		t.Fatalf("cursor and wrapping not restored last: %q", got)
	}
	first, last, ok := strings.Cut(body, "\x1b[J")
	if !ok || !strings.Contains(first, "3 requests") || !strings.Contains(last, "4 requests") {
		t.Errorf("frames = %q, want 3 requests then a last frame with 4", body)
	}
	if strings.Contains(body, "q quit") {
		t.Errorf("q offered with stdin not a terminal: %q", body)
	}
}

func TestReadKeys(t *testing.T) {
	tests := []struct {
		name   string
		reads  []string
		quits  int
		unread int // reads left when readKeys returns
	}{
		{"q", []string{"q", "x"}, 1, 1},
		{"Q among other keys", []string{"abQc"}, 1, 0},
		{"q in a later read", []string{"a", "b", "q", "x"}, 1, 1},
		{"input ends without q", []string{"a", "x"}, 0, 0},
		{"no input", nil, 0, 0},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			quits := 0
			r := &chunks{parts: tt.reads}
			readKeys(r, func() { quits++ })
			if quits != tt.quits || len(r.parts) != tt.unread {
				t.Errorf("quit called %d times with %d reads left, want %d and %d", quits, len(r.parts), tt.quits, tt.unread)
			}
		})
	}
}

// chunks returns one part per Read, then io.EOF.
type chunks struct{ parts []string }

func (c *chunks) Read(p []byte) (int, error) {
	if len(c.parts) == 0 {
		return 0, io.EOF
	}
	n := copy(p, c.parts[0])
	c.parts = c.parts[1:]
	return n, nil
}

func TestUptimeAndAgo(t *testing.T) {
	for d, want := range map[time.Duration]string{
		0:                             "0m",
		59 * time.Minute:              "59m",
		time.Hour + 5*time.Minute:     "1h 05m",
		23*time.Hour + 59*time.Minute: "23h 59m",
		24 * time.Hour:                "1d 0h",
		50 * time.Hour:                "2d 2h",
	} {
		if got := uptime(d); got != want {
			t.Errorf("uptime(%s) = %q, want %q", d, got, want)
		}
	}
	now := time.Date(2026, 9, 17, 12, 0, 0, 0, time.UTC)
	for _, tt := range []struct {
		t    time.Time
		want string
	}{
		{time.Time{}, "none"},
		{now.Add(-30 * time.Second), "just now"},
		{now.Add(-5 * time.Minute), "5m ago"},
		{now.Add(-47 * time.Hour), "47h ago"},
		{now.Add(-72 * time.Hour), "3d ago"},
	} {
		if got := ago(tt.t, now); got != tt.want {
			t.Errorf("ago(%s) = %q, want %q", now.Sub(tt.t), got, tt.want)
		}
	}
}

// newScreenAt is a status screen over an empty store in a scratch directory.
func newScreenAt(t *testing.T, url string, out io.Writer, color bool) *statusScreen {
	t.Helper()
	root := t.TempDir()
	return newStatusScreen(root, store.NewDir(root), url, out, color)
}
