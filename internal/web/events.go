package web

import (
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// versionOf names the store as the pages show it: which cases there are and
// how many events each has. Every event written moves it, as does a case
// taken out or put back, and it is the same for the same store in every run
// of serve, so a page keeps it across a restart.
func versionOf(cases []*store.Case) string {
	var b strings.Builder
	for _, c := range cases {
		fmt.Fprintf(&b, "%s %d\n", c.ID, c.Revision())
	}
	return textID(b.String())
}

// eventsURL is the event stream for a page drawn from cases.
func eventsURL(cases []*store.Case) string {
	return "/events?after=" + versionOf(cases)
}

// see records the version of the cases a poll read and, when it has moved,
// wakes the event streams. The caller holds s.mu.
func (s *Server) see(cases []*store.Case) {
	if v := versionOf(cases); v != s.version {
		s.version = v
		close(s.moved)
		s.moved = make(chan struct{})
	}
}

// changes returns the version the last poll saw, and a channel that is
// closed when a later poll sees another.
func (s *Server) changes() (string, <-chan struct{}) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.version, s.moved
}

// streamGap is the least time between two messages on one stream: a second,
// as often as serve reads the store by itself. Each message makes the page
// refresh, and the refresh reads the store, which can find a newer version
// and so set off another message: during a burst of writes, such as a sweep
// or a prune, the pages would refresh as fast as they could. The gap holds
// the next message back, and it then carries the version as it is by then,
// so no change goes unsent.
const streamGap = time.Second

// events is an open page's event stream: GET /events?after=V, where V is the
// version the page was drawn from. It sends the store's version as a message
// whenever it is not the last one the page has, at once if the store has
// changed since, and then each time a poll finds it changed, at most once
// every streamGap; the page refreshes what it shows on each. The browser
// reconnects a dropped stream by itself, sending the last version it had as
// Last-Event-ID. The stream ends when the page goes away or EndStreams is
// called.
func (s *Server) events(w http.ResponseWriter, r *http.Request) {
	after := r.Header.Get("Last-Event-ID")
	if after == "" {
		after = r.URL.Query().Get("after")
	}
	// Bring the version up to the store. A store that cannot be read leaves
	// it as it was; the page's own requests report the error.
	_, _ = s.cases()

	// The headers go out even when the stream is about to end, so the
	// browser reconnects: it gives up on a response of any other type.
	h := w.Header()
	h.Set("Content-Type", "text/event-stream")
	h.Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	flush := http.NewResponseController(w).Flush
	if flush() != nil {
		return
	}
	for {
		v, moved := s.changes()
		if v != after {
			if _, err := fmt.Fprintf(w, "id: %s\ndata: %s\n\n", v, v); err != nil {
				return
			}
			if flush() != nil {
				return
			}
			after = v
			select {
			case <-time.After(streamGap):
				continue
			case <-r.Context().Done():
				return
			case <-s.stop:
				return
			}
		}
		select {
		case <-moved:
		case <-r.Context().Done():
			return
		case <-s.stop:
			return
		}
	}
}

// EndStreams ends the event streams open on the server and any opened after.
// A stream never goes idle, so an http.Server shutting down would wait for
// them; cases serve passes this to Serve, which calls it as the shutdown
// starts. The pages reconnect to the next serve.
func (s *Server) EndStreams() {
	s.stopOnce.Do(func() { close(s.stop) })
}
