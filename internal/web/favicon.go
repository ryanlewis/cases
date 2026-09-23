package web

import (
	"fmt"
	"net/http"
	"net/url"
	"strconv"
)

// The favicon is an SVG drawn by the server from its query, so the page's CSP
// needs no data: or blob: images and the page no script to draw it. The layout
// points <link rel=icon> at Icon's URL, and the tally fragment swaps that link
// whenever the counts change; the browser fetches the new URL.
//
// With nothing waiting it is the plain mark: a paper tile with a "c". With
// cases waiting the tile turns to ink and shows the count, 9+ above nine;
// with a blocking case among them it turns red, the one hue the page gives
// blocking.
const (
	iconMark = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">` +
		`<rect x="1" y="1" width="30" height="30" fill="#fbfbfa" stroke="#1a1a18" stroke-width="2"/>` +
		`<path d="M21 11A7.07 7.07 0 1 0 21 21" fill="none" stroke="#1a1a18" stroke-width="4"/>` +
		`</svg>`
	iconCount = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">` +
		`<rect x="0.5" y="0.5" width="31" height="31" fill="%s" stroke="#fbfbfa" stroke-width="1"/>` +
		`<text x="16" y="23.5" text-anchor="middle" font-family="ui-monospace, Menlo, Consolas, monospace" font-size="%d" font-weight="700" fill="#fbfbfa">%s</text>` +
		`</svg>`
	iconInk      = "#1a1a18"
	iconBlocking = "#a11b2b"
)

// Icon is the favicon's URL for these counts: the open cases, as in the
// title, and whether any of them is blocking. Parked cases are not counted.
func (t tally) Icon() string {
	n := t.Waiting()
	if n == 0 {
		return "/favicon.svg"
	}
	q := url.Values{"n": {strconv.Itoa(n)}}
	if t.Blocking > 0 {
		q.Set("blocking", "1")
	}
	return "/favicon.svg?" + q.Encode()
}

// favicon draws the icon Icon names. It reads only its query, not the store.
func (s *Server) favicon(w http.ResponseWriter, r *http.Request) {
	q := r.URL.Query()
	n := 0
	if v := q.Get("n"); v != "" {
		var err error
		if n, err = strconv.Atoi(v); err != nil || n < 0 {
			http.Error(w, "bad n", http.StatusBadRequest)
			return
		}
	}
	blocking := false
	switch q.Get("blocking") {
	case "":
	case "1":
		blocking = true
	default:
		http.Error(w, "bad blocking", http.StatusBadRequest)
		return
	}
	// The icon is a function of the query alone, so a browser may keep it: a
	// refresh that points back at a count already drawn needs no fetch.
	w.Header().Set("Content-Type", "image/svg+xml")
	w.Header().Set("Cache-Control", "max-age=3600")
	if n == 0 {
		fmt.Fprint(w, iconMark)
		return
	}
	fill, text, size := iconInk, strconv.Itoa(n), 24
	if blocking {
		fill = iconBlocking
	}
	if n > 9 {
		text, size = "9+", 20
	}
	fmt.Fprintf(w, iconCount, fill, size, text)
}
