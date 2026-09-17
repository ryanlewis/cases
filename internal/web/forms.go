// Package web's form parsing: turning a posted form into the record the
// store expects.
package web

import (
	"errors"
	"fmt"
	"net/url"
	"strconv"
	"strings"

	"github.com/ryanlewis/cases/internal/store"
)

// revisionParam is the revision of the case a page was rendered from, as the
// page's forms send it back. A form from a page served before forms carried
// one sends none and gets -1, which no case is at.
func revisionParam(v url.Values) int {
	rev, err := strconv.Atoi(v.Get("revision"))
	if err != nil {
		return -1
	}
	return rev
}

// answerFromForm turns the posted response form into an answer. park is true
// when a stuck case is being parked, which is a park event rather than an
// answer. The drop button dismisses a case of any kind, whatever else the form
// holds. The store validates the result against the kind; this only reports a
// response that was not chosen at all, in words that fit the form.
func answerFromForm(c *store.Case, f url.Values) (rec store.AnswerRecord, park bool, err error) {
	rec.Note = strings.TrimSpace(f.Get("note"))
	if f.Get("drop") != "" {
		rec.Drop = true
		return rec, false, nil
	}
	switch c.Kind {
	case store.KindDecision:
		switch choice := f.Get("choice"); choice {
		case "":
			return rec, false, errors.New("choose an option")
		case "other":
			rec.Other = true
		default:
			n, err := strconv.Atoi(choice)
			if err != nil {
				return rec, false, fmt.Errorf("choice %q is not an option", choice)
			}
			rec.Choice = n
		}
	case store.KindApproval:
		for _, row := range c.Rows {
			verdict := f.Get("verdict." + row.ID)
			if verdict == "" {
				return rec, false, fmt.Errorf("choose approve, hold or reject for %q", row.Label)
			}
			rec.Rows = append(rec.Rows, store.RowAnswer{ID: row.ID, Verdict: verdict, Note: strings.TrimSpace(f.Get("note." + row.ID))})
		}
	case store.KindSignoff:
		if rec.Signoff = f.Get("signoff"); rec.Signoff == "" {
			return rec, false, errors.New("choose accept or request changes")
		}
	case store.KindStuck:
		// Park is its own button, not an answer; stuck=park is still read.
		if f.Get("park") != "" || f.Get("stuck") == "park" {
			return rec, true, nil
		}
		choice := f.Get("stuck")
		if choice == "" && strings.TrimSpace(f.Get("text")) != "" {
			// Guidance typed without ticking its button still means guidance.
			choice = "text"
		}
		switch choice {
		case "text":
			if rec.Text = strings.TrimSpace(f.Get("text")); rec.Text == "" {
				return rec, false, errors.New("write the guidance, or choose drop")
			}
		case "drop":
			rec.Drop = true
		default:
			return rec, false, errors.New("choose guidance or drop, or park it")
		}
	case store.KindQuestion:
		if rec.Text = strings.TrimSpace(f.Get("text")); rec.Text == "" {
			return rec, false, errors.New("write the reply")
		}
	case store.KindFYI:
		rec.Ack = f.Get("ack") != ""
	}
	return rec, false, nil
}
