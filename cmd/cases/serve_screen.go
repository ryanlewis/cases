package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// agentHint is what the status screen tells the user about pointing an agent
// at the store.
var agentHint = []string{
	"Agents: install the bundled skill with `cases skill install claude` (or codex, pi)",
	"        and they can raise cases with `cases open` and act on answers with `cases wait`.",
}

// logLines is how many request log lines the screen keeps.
const logLines = 5

// serveStats is what the status screen reports about the store.
type serveStats struct {
	Open        map[store.Urgency]int
	OpenTotal   int
	Parked      int
	Answered    int
	PickedUp    int
	ClosedToday int
	Closed      int
	// Answers, Parks and Resumes count human events recorded since serve
	// started, whether they came from the web inbox or the CLI.
	Answers   int
	Parks     int
	Resumes   int
	LastEvent time.Time
	Err       error
}

// countCases folds the cases into serveStats. since is when serve started;
// now decides which closes count as today, in local time.
func countCases(cases []*store.Case, since, now time.Time) serveStats {
	s := serveStats{Open: map[store.Urgency]int{}}
	y, m, d := now.Local().Date()
	for _, c := range cases {
		switch c.State {
		case store.StateOpen:
			s.Open[c.Urgency]++
			s.OpenTotal++
		case store.StateParked:
			s.Parked++
		case store.StateAnswered:
			s.Answered++
		case store.StatePickedUp:
			s.PickedUp++
		case store.StateClosed:
			s.Closed++
			if c.Close != nil {
				if cy, cm, cd := c.Close.ClosedAt.Local().Date(); cy == y && cm == m && cd == d {
					s.ClosedToday++
				}
			}
		}
		if c.UpdatedAt.After(s.LastEvent) {
			s.LastEvent = c.UpdatedAt
		}
		for _, ev := range c.Events {
			if ev.Author != store.AuthorHuman || ev.At.Before(since) {
				continue
			}
			switch ev.Type {
			case store.EventAnswer:
				s.Answers++
			case store.EventPark:
				s.Parks++
			case store.EventResume:
				s.Resumes++
			}
		}
	}
	return s
}

// screenView is everything one frame of the status screen shows.
type screenView struct {
	Store    string
	URL      string
	Stats    serveStats
	Uptime   time.Duration
	Requests int64
	Log      []string
	Keys     bool // q is read from the keyboard
	Color    bool
	Now      time.Time
}

// render lays out one frame, a line per entry.
func render(v screenView) []string {
	st := func(code, s string) string {
		if !v.Color || code == "" {
			return s
		}
		return "\x1b[" + code + "m" + s + "\x1b[0m"
	}
	const dim, bold, red, cyan, green = "2", "1", "1;31", "36", "32"
	label := func(s string) string { return st(dim, fmt.Sprintf("%-12s", s)) }
	count := func(n int) string { return fmt.Sprintf("%3d", n) }
	s := v.Stats

	lines := []string{
		st(bold, "cases serve") + st(dim, "  ·  up "+uptime(v.Uptime)),
		"",
		label("inbox") + st(cyan, v.URL),
		label("store") + v.Store,
		"",
	}
	if s.Err != nil {
		lines = append(lines, st(red, "cannot read the store: "+s.Err.Error()), "")
	}

	blocking := fmt.Sprintf("%d blocking", s.Open[store.UrgencyBlocking])
	if s.Open[store.UrgencyBlocking] > 0 {
		blocking = st(red, blocking)
	}
	openCount := count(s.OpenTotal)
	if s.OpenTotal > 0 {
		openCount = st(bold, openCount)
	}
	lines = append(lines,
		label("open")+openCount+"   "+blocking+st(dim, " · ")+
			fmt.Sprintf("%d today", s.Open[store.UrgencyToday])+st(dim, " · ")+
			fmt.Sprintf("%d whenever", s.Open[store.UrgencyWhenever]),
		label("parked")+count(s.Parked),
		label("with agent")+count(s.Answered+s.PickedUp)+
			fmt.Sprintf("   %d answered", s.Answered)+st(dim, " · ")+fmt.Sprintf("%d picked up", s.PickedUp),
		label("closed")+count(s.ClosedToday)+"   today"+st(dim, " · ")+fmt.Sprintf("%d in all", s.Closed),
		"",
		label("since start")+fmt.Sprintf("%s · %s · %s · %s",
			plural(int(v.Requests), "request"), plural(s.Answers, "answer"),
			plural(s.Parks, "park"), plural(s.Resumes, "resume")),
		label("last event")+ago(s.LastEvent, v.Now),
		"",
		st(dim, "log"),
	)
	if len(v.Log) == 0 {
		lines = append(lines, st(dim, "  nothing yet"))
	}
	for _, l := range v.Log {
		lines = append(lines, "  "+l)
	}
	lines = append(lines, "")
	lines = append(lines, agentHint...)
	lines = append(lines, "")
	if v.Keys {
		lines = append(lines, st(green, "q")+" quit   "+st(green, "Ctrl-C")+" quit")
	} else {
		lines = append(lines, st(green, "Ctrl-C")+" quit")
	}
	return lines
}

func plural(n int, word string) string {
	if n == 1 {
		return "1 " + word
	}
	return fmt.Sprintf("%d %ss", n, word)
}

func uptime(d time.Duration) string {
	switch {
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	case d < 24*time.Hour:
		return fmt.Sprintf("%dh %02dm", int(d.Hours()), int(d.Minutes())%60)
	default:
		return fmt.Sprintf("%dd %dh", int(d.Hours()/24), int(d.Hours())%24)
	}
}

func ago(t, now time.Time) string {
	d := now.Sub(t)
	switch {
	case t.IsZero():
		return "none"
	case d < time.Minute:
		return "just now"
	case d < time.Hour:
		return fmt.Sprintf("%dm ago", int(d.Minutes()))
	case d < 48*time.Hour:
		return fmt.Sprintf("%dh ago", int(d.Hours()))
	default:
		return fmt.Sprintf("%dd ago", int(d.Hours()/24))
	}
}

// statusScreen redraws the status block in place on a terminal.
type statusScreen struct {
	root  string
	url   string
	out   io.Writer
	color bool
	since time.Time

	logMu sync.Mutex
	log   []string
	part  []byte // a log write not yet ended by a newline

	poller store.CasePoller
	drawn  int    // lines on screen from the last frame
	frame  string // the last frame, to skip identical redraws
}

// newStatusScreen shows root, the store's path, and counts the cases in cases.
func newStatusScreen(root string, cases store.Store, url string, out io.Writer, color bool) *statusScreen {
	return &statusScreen{root: root, url: url, out: out, color: color, since: time.Now(), poller: cases.NewPoller()}
}

// logWriter returns the writer for the request log. Lines are kept for the
// screen, and also passed to stderr when tee is set (stderr is not the
// terminal the screen is on).
func (s *statusScreen) logWriter(stderr io.Writer, tee bool) io.Writer {
	return writerFunc(func(p []byte) (int, error) {
		if tee {
			if _, err := stderr.Write(p); err != nil {
				return 0, err
			}
		}
		s.logMu.Lock()
		defer s.logMu.Unlock()
		s.part = append(s.part, p...)
		for {
			i := bytes.IndexByte(s.part, '\n')
			if i < 0 {
				break
			}
			s.log = append(s.log, string(s.part[:i]))
			s.part = s.part[i+1:]
		}
		if len(s.log) > logLines {
			s.log = s.log[len(s.log)-logLines:]
		}
		return len(p), nil
	})
}

type writerFunc func([]byte) (int, error)

func (f writerFunc) Write(p []byte) (int, error) { return f(p) }

// run draws the screen, redraws it every second when something changed,
// and reads q from stdin to call quit. The returned function stops that and
// puts the terminal back; it must run before serve returns.
func (s *statusScreen) run(ctx context.Context, quit func(), stdin io.Reader, requests *atomic.Int64) (restore func()) {
	keys := false
	restoreKeys := func() {}
	// A background job (cases serve &) must not touch the terminal's input,
	// or SIGTTIN and SIGTTOU stop the whole server.
	if f, ok := stdin.(*os.File); ok && isTerminal(f) && isForeground(f) {
		keys = true
		// Without raw mode, q still works followed by Enter.
		if r, err := keysRaw(f); err == nil {
			restoreKeys = r
		}
		go readKeys(f, quit)
	}

	// Hide the cursor and turn off line wrapping, so a long line is clipped
	// and the line count used to move back up stays true.
	fmt.Fprint(s.out, "\x1b[?25l\x1b[?7l")
	stop := make(chan struct{})
	done := make(chan struct{})
	go func() {
		defer close(done)
		tick := time.NewTicker(time.Second)
		defer tick.Stop()
		for {
			s.draw(keys, requests.Load())
			select {
			case <-ctx.Done():
				return
			case <-stop:
				return
			case <-tick.C:
			}
		}
	}()
	return func() {
		close(stop)
		<-done
		s.draw(keys, requests.Load())
		fmt.Fprint(s.out, "\x1b[?7h\x1b[?25h")
		restoreKeys()
	}
}

// readKeys calls quit when q is typed. It runs until stdin ends; a read that
// is still blocked when serve exits ends with the process.
func readKeys(r io.Reader, quit func()) {
	buf := make([]byte, 16)
	for {
		n, err := r.Read(buf)
		if bytes.ContainsAny(buf[:n], "qQ") {
			quit()
			return
		}
		if err != nil {
			return
		}
	}
}

func (s *statusScreen) draw(keys bool, requests int64) {
	now := time.Now()
	cases, _, err := s.poller.Poll()
	if errors.Is(err, fs.ErrNotExist) {
		err = nil
	}
	s.logMu.Lock()
	log := append([]string(nil), s.log...)
	s.logMu.Unlock()
	lines := render(screenView{
		Store:    s.root,
		URL:      s.url,
		Stats:    withErr(countCases(cases, s.since, now), err),
		Uptime:   now.Sub(s.since),
		Requests: requests,
		Log:      log,
		Keys:     keys,
		Color:    s.color,
		Now:      now,
	})
	// A frame taller than the terminal scrolls, and moving back up by its
	// line count would then land short and stack copies. Leave a row free
	// for the cursor that the last newline moves to.
	if f, ok := s.out.(*os.File); ok {
		if rows := termRows(f); rows > 1 && len(lines) > rows-1 {
			lines = lines[:rows-1]
		}
	}
	frame := strings.Join(lines, "\n")
	if frame == s.frame {
		return
	}
	s.frame = frame
	var b strings.Builder
	if s.drawn > 0 {
		fmt.Fprintf(&b, "\x1b[%dA", s.drawn)
	}
	for _, l := range lines {
		b.WriteString("\r\x1b[2K" + l + "\n")
	}
	b.WriteString("\x1b[J")
	fmt.Fprint(s.out, b.String())
	s.drawn = len(lines)
}

func withErr(s serveStats, err error) serveStats {
	s.Err = err
	return s
}
