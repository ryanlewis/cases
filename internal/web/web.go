// Package web serves the case store as a small local web app: an inbox, a
// case page with a response form shaped by kind, and a done list. Pages are
// rendered on the server; htmx only refreshes the inbox and the thread.
package web

import (
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// maxForm caps the size of a posted form.
const maxForm = 1 << 20

// CheckLoopback refuses a listen address that is not on the loopback
// interface. Binding anything wider waits for the edge identity check.
func CheckLoopback(addr string) error {
	host, port, err := net.SplitHostPort(addr)
	if err != nil {
		return fmt.Errorf("--listen %q: %w", addr, err)
	}
	if port == "" {
		return fmt.Errorf("--listen %q: no port", addr)
	}
	if host == "localhost" {
		return nil
	}
	if ip := net.ParseIP(host); ip != nil && ip.IsLoopback() {
		return nil
	}
	return fmt.Errorf("--listen %q: only a loopback address (127.0.0.1, ::1 or localhost) is allowed", addr)
}

// Server is the web app over one store.
type Server struct {
	// Actor is recorded on the answers, parks and resumes the inbox writes.
	// nil records none. Set it before serving.
	Actor *store.Actor

	store store.Store
	log   io.Writer
	// hosts are the Host header values requests may carry: the listen
	// address and its localhost spelling, on the listen port.
	hosts map[string]bool

	mu     sync.Mutex // guards poller and warned
	poller store.CasePoller
	warned map[string]bool

	logMu sync.Mutex
}

// logf writes one line to the log; handlers run concurrently.
func (s *Server) logf(format string, args ...any) {
	s.logMu.Lock()
	defer s.logMu.Unlock()
	fmt.Fprintf(s.log, format+"\n", args...)
}

// New returns a Server for cases, answering requests addressed to listen
// (host:port, as bound). Request logs and store warnings go to log.
func New(cases store.Store, listen string, log io.Writer) (*Server, error) {
	if err := CheckLoopback(listen); err != nil {
		return nil, err
	}
	host, port, _ := net.SplitHostPort(listen)
	s := &Server{
		store:  cases,
		log:    log,
		hosts:  map[string]bool{},
		poller: cases.NewPoller(),
		warned: map[string]bool{},
	}
	names := []string{host, "localhost"}
	if ip := net.ParseIP(host); ip != nil {
		// Browsers send the canonical spelling ([::1], not [0:0:...:1]).
		names[0] = ip.String()
	}
	if host == "localhost" {
		names = append(names, "127.0.0.1", "::1")
	}
	for _, name := range names {
		s.hosts[net.JoinHostPort(name, port)] = true
		if port == "80" {
			// Browsers leave the default port out of Host.
			if strings.Contains(name, ":") {
				name = "[" + name + "]"
			}
			s.hosts[name] = true
		}
	}
	return s, nil
}

// Handler returns the app's routes wrapped in the request guard and logger.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /{$}", s.inbox)
	mux.HandleFunc("GET /fragments/inbox", s.inboxFragment)
	mux.HandleFunc("GET /done", s.done)
	mux.HandleFunc("GET /cases/{id}", s.casePage)
	mux.HandleFunc("GET /cases/{id}/thread", s.threadFragment)
	mux.HandleFunc("POST /cases/{id}/answer", s.answer)
	mux.HandleFunc("POST /cases/{id}/resume", s.resume)
	mux.Handle("GET /static/", http.StripPrefix("/static/", http.FileServerFS(staticFS)))
	return s.logRequests(s.guard(mux))
}

// Serve runs handler on ln until ctx is cancelled, then shuts down.
func Serve(ctx context.Context, ln net.Listener, handler http.Handler) error {
	srv := &http.Server{Handler: handler, ReadHeaderTimeout: 10 * time.Second}
	errc := make(chan error, 1)
	go func() { errc <- srv.Serve(ln) }()
	select {
	case err := <-errc:
		return err
	case <-ctx.Done():
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := srv.Shutdown(shutdown); err != nil {
			return err
		}
		if err := <-errc; !errors.Is(err, http.ErrServerClosed) {
			return err
		}
		return nil
	}
}

// guard sets the security headers and refuses requests that could come from
// another site: a Host that is not this server (DNS rebinding), or a form post
// whose Sec-Fetch-Site or Origin says it came from elsewhere.
func (s *Server) guard(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Content-Security-Policy", "default-src 'self'; frame-ancestors 'none'; form-action 'self'")
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("Referrer-Policy", "same-origin")

		if !s.hosts[strings.ToLower(r.Host)] {
			http.Error(w, "forbidden: unexpected Host", http.StatusForbidden)
			return
		}
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			if site := r.Header.Get("Sec-Fetch-Site"); site != "" && site != "same-origin" && site != "none" {
				http.Error(w, "forbidden: cross-site request", http.StatusForbidden)
				return
			}
			if origin := r.Header.Get("Origin"); origin != "" && !strings.EqualFold(origin, "http://"+r.Host) {
				http.Error(w, "forbidden: foreign Origin", http.StatusForbidden)
				return
			}
		}
		next.ServeHTTP(w, r)
	})
}

type statusWriter struct {
	http.ResponseWriter
	status int
}

func (w *statusWriter) WriteHeader(code int) {
	w.status = code
	w.ResponseWriter.WriteHeader(code)
}

// logRequests writes one line per request. Successful htmx polls are left out:
// an open tab makes one every two seconds and they would bury everything else.
func (s *Server) logRequests(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		sw := &statusWriter{ResponseWriter: w, status: http.StatusOK}
		next.ServeHTTP(sw, r)
		if r.Method == http.MethodGet && r.Header.Get("HX-Request") == "true" && sw.status < 400 {
			return
		}
		s.logf("%s %s %s %d %s", start.Format(time.RFC3339), r.Method, r.URL.RequestURI(), sw.status, time.Since(start).Round(time.Millisecond))
	})
}

// cases polls the store. A store that does not exist yet reads as empty.
func (s *Server) cases() ([]*store.Case, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	cases, bad, err := s.poller.Poll()
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	for _, b := range bad {
		if msg := b.Error(); !s.warned[msg] {
			s.warned[msg] = true
			s.logf("warning: %s", msg)
		}
	}
	return cases, nil
}

// find returns the case with id from the poller, or nil.
func (s *Server) find(id string) (*store.Case, []*store.Case, error) {
	cases, err := s.cases()
	if err != nil {
		return nil, nil, err
	}
	for _, c := range cases {
		if c.ID == id {
			return c, cases, nil
		}
	}
	return nil, cases, nil
}

// Age renders how long ago t was, coarsely.
func Age(t, now time.Time) string {
	if t.IsZero() {
		return "-"
	}
	d := now.Sub(t)
	switch {
	case d < time.Minute:
		return "now"
	case d < time.Hour:
		return fmt.Sprintf("%dm", int(d.Minutes()))
	case d < 48*time.Hour:
		return fmt.Sprintf("%dh", int(d.Hours()))
	default:
		return fmt.Sprintf("%dd", int(d.Hours()/24))
	}
}
