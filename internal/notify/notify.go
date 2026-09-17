// Package notify decides when a case has newly landed on the human and keeps
// the notifications that follow in a feed an open inbox tab reads.
//
// The Engine is given the store's cases after every poll. It compares their
// events, by case id and file name, with the ones it has already seen, so an
// event counts as new when it appears in the store, not by its timestamp: an
// event that records an earlier time still counts. The first poll only records
// what is there.
//
// The rule is fixed for now: a case that is open because the agent opened it,
// followed up on an answer (a note that reopens the case), resumed it after a
// park, or replied with a note after the human resumed it. Every urgency
// matches; the tab filters by urgency itself.
package notify

import (
	"crypto/rand"
	"encoding/hex"
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// Event names the notifier gives the events that put a case on the human.
const (
	EventOpen   = "open"
	EventReopen = "reopen"
	EventResume = "resume"
	// EventReply is the agent's first note after the human resumed a parked
	// case: the case was already open, but the human is waiting on it.
	EventReply = "reply"
)

// SinkBrowser is the only sink there is.
const SinkBrowser = "browser"

// FeedSize is how many notifications the feed keeps.
const FeedSize = 50

// EventRef is the event a notification is about.
type EventRef struct {
	Name   string          `json:"name"`
	Seq    int             `json:"seq"`
	Author store.Author    `json:"author"`
	Type   store.EventType `json:"event"`
	File   string          `json:"file"`
	At     time.Time       `json:"at"`
}

// Item is one notification. Title, Body and URL are what the tab shows and
// where a click goes; Tag is the same for every tab showing the item, so the
// desktop shows it once.
type Item struct {
	ID       int64       `json:"id"`
	Sink     string      `json:"sink"`
	Tag      string      `json:"tag"`
	Title    string      `json:"title"`
	Body     string      `json:"body"`
	URL      string      `json:"url"`
	QueuedAt time.Time   `json:"queued_at"`
	Event    EventRef    `json:"event"`
	Case     *store.Case `json:"case"`
}

// Page is what a reader gets from the feed: the items after the id it asked
// for, the latest id, and the boot id. Ids start again when serve restarts,
// and a new boot id tells the reader to drop the id it kept.
type Page struct {
	Boot   string `json:"boot"`
	Latest int64  `json:"latest"`
	Items  []Item `json:"items"`
}

// Feed keeps the last FeedSize notifications in memory, with increasing ids.
type Feed struct {
	mu     sync.Mutex
	boot   string
	latest int64
	items  []Item
}

// NewFeed returns an empty feed with a fresh boot id.
func NewFeed() *Feed {
	b := make([]byte, 8)
	_, _ = rand.Read(b)
	return &Feed{boot: hex.EncodeToString(b)}
}

func (f *Feed) add(it Item) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.latest++
	it.ID = f.latest
	f.items = append(f.items, it)
	if n := len(f.items) - FeedSize; n > 0 {
		f.items = append([]Item(nil), f.items[n:]...)
	}
}

// Latest is the id of the newest item, or 0 when there is none: how many
// items the feed has taken.
func (f *Feed) Latest() int64 {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.latest
}

// After returns the items with an id above after, oldest first.
func (f *Feed) After(after int64) Page {
	f.mu.Lock()
	defer f.mu.Unlock()
	p := Page{Boot: f.boot, Latest: f.latest, Items: []Item{}}
	for _, it := range f.items {
		if it.ID > after {
			p.Items = append(p.Items, it)
		}
	}
	return p
}

// Engine turns polls of the store into notifications on a feed. It is not
// safe for concurrent use; the caller polls from one place.
type Engine struct {
	feed *Feed
	now  func() time.Time
	// seen holds case id / event file name for every event already looked
	// at; nil until the first poll.
	seen map[string]bool
}

// New returns an Engine that adds to feed. now stamps the items; nil means
// time.Now.
func New(feed *Feed, now func() time.Time) *Engine {
	if now == nil {
		now = time.Now
	}
	return &Engine{feed: feed, now: now}
}

// Observe takes the cases from one poll and adds a notification for each case
// that has newly landed on the human. It returns how many it added. The first
// call records the events already in the store and adds none.
func (e *Engine) Observe(cases []*store.Case) int {
	first := e.seen == nil
	if first {
		e.seen = map[string]bool{}
	}
	added := 0
	for _, c := range cases {
		fresh := map[string]bool{}
		for _, ev := range c.Events {
			key := c.ID + "/" + ev.File
			if !e.seen[key] {
				e.seen[key] = true
				fresh[ev.File] = true
			}
		}
		if first || len(fresh) == 0 {
			continue
		}
		ev, name, ok := Landed(c)
		if !ok || !fresh[ev.File] {
			continue
		}
		e.feed.add(item(c, ev, name, e.now()))
		added++
	}
	return added
}

// Landed reports whether the case is on the human because of one of the
// fixed rule's events, and which: the case is open, and the latest event
// that put it there, looking past amends and notes on the open case, is the
// agent opening it, reopening it with a note, or resuming it. When that
// event is the human resuming it, the agent's first note after the resume
// is the one that put it on the human, as for `cases wait --for human`.
func Landed(c *store.Case) (store.Event, string, bool) {
	n := len(c.Events)
	if c.State != store.StateOpen || n == 0 {
		return store.Event{}, "", false
	}
	i, note := n-1, -1
	for i > 0 {
		ev := c.Events[i]
		if ev.Type == store.EventNote && ev.From() == store.StateOpen {
			note = i
		} else if ev.Type != store.EventAmend {
			break
		}
		i--
	}
	ev := c.Events[i]
	switch {
	case ev.Type == store.EventResume && ev.Author == store.AuthorHuman && note >= 0:
		return c.Events[note], EventReply, true
	case ev.Type == store.EventOpen:
		return ev, EventOpen, true
	case ev.Type == store.EventNote:
		return ev, EventReopen, true
	case ev.Type == store.EventResume && ev.Author == store.AuthorAgent:
		return ev, EventResume, true
	}
	return ev, "", false
}

func item(c *store.Case, ev store.Event, name string, now time.Time) Item {
	body := []string{string(c.Urgency) + " " + string(c.Kind)}
	switch name {
	case EventOpen:
		if c.Worker != "" {
			body = append(body, "from "+c.Worker)
		}
	case EventReopen:
		body = append(body, "the agent followed up")
	case EventResume:
		body = append(body, "back in the inbox")
	case EventReply:
		body = append(body, "the agent replied")
	}
	return Item{
		Sink:     SinkBrowser,
		Tag:      c.ID + "/" + ev.File,
		Title:    c.Title,
		Body:     strings.Join(body, " · "),
		URL:      "/cases/" + url.PathEscape(c.ID),
		QueuedAt: now.UTC(),
		Event:    EventRef{Name: name, Seq: ev.Seq, Author: ev.Author, Type: ev.Type, File: ev.File, At: ev.At},
		Case:     c,
	}
}
