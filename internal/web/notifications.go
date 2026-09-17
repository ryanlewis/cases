package web

import (
	"encoding/json"
	"net/http"
	"strconv"
)

// notifications serves the feed of cases that landed on the human, for the
// browser notifications prefs.js shows: GET /notifications?after=N returns
// the items after id N, the latest id and the boot id. It polls the store
// first, so a tab learns of a case as soon as it asks.
func (s *Server) notifications(w http.ResponseWriter, r *http.Request) {
	var after int64
	if v := r.URL.Query().Get("after"); v != "" {
		n, err := strconv.ParseInt(v, 10, 64)
		if err != nil || n < 0 {
			http.Error(w, "after must be a notification id", http.StatusBadRequest)
			return
		}
		after = n
	}
	if _, err := s.cases(); err != nil {
		s.fail(w, err)
		return
	}
	data, err := json.Marshal(s.feed.After(after))
	if err != nil {
		s.fail(w, err)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	_, _ = w.Write(append(data, '\n'))
}

// Poll reads the store through the server's poller, which also feeds the
// notifier. serve calls it on a ticker so notifications are queued while no
// tab is asking.
func (s *Server) Poll() error {
	_, err := s.cases()
	return err
}

// Notified is how many notifications have been queued since the server
// started.
func (s *Server) Notified() int64 {
	return s.feed.Latest()
}
