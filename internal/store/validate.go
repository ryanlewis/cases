// Package store's record validators: one method per record type that
// (*Case).apply calls before it changes the case, plus the helpers only
// they use.
package store

import (
	"errors"
	"fmt"
	"slices"
	"strings"
)

func (r *OpenRecord) validate() error {
	if !slices.Contains(Kinds, r.Kind) {
		return fmt.Errorf("unknown kind %q", r.Kind)
	}
	if !slices.Contains(Urgencies, r.Urgency) {
		return fmt.Errorf("unknown urgency %q", r.Urgency)
	}
	if strings.TrimSpace(r.Title) == "" {
		return errors.New("title is empty")
	}
	if err := checkKindFields(r.Kind, r.Options, r.Rows); err != nil {
		return err
	}
	switch r.Kind {
	case KindDecision:
		if len(r.Options) == 0 {
			return errors.New("a decision case needs at least one option")
		}
		if err := checkOptions(r.Options); err != nil {
			return err
		}
	case KindApproval:
		if len(r.Rows) == 0 {
			return errors.New("an approval case needs at least one row")
		}
		if err := checkRows(r.Rows, nil); err != nil {
			return err
		}
	}
	return checkLabels(r.Labels, nil)
}

// validate checks the amend against cur, the case as it stands: the open
// event and any earlier amends.
func (r *AmendRecord) validate(cur *OpenRecord) error {
	if r.Body == "" && len(r.Options) == 0 && len(r.Rows) == 0 && len(r.Links) == 0 && len(r.Labels) == 0 && r.Context == "" {
		return errors.New("the amend has no body, options, rows, links, labels or context")
	}
	if err := checkKindFields(cur.Kind, r.Options, r.Rows); err != nil {
		return err
	}
	// An empty body or context leaves the case's as it was, so one that is
	// only whitespace is a mistake rather than a way to clear it.
	if r.Body != "" && strings.TrimSpace(r.Body) == "" {
		return errors.New("amend body is empty")
	}
	if r.Context != "" && strings.TrimSpace(r.Context) == "" {
		return errors.New("amend context is empty")
	}
	if err := checkOptions(r.Options); err != nil {
		return err
	}
	if err := checkRows(r.Rows, cur.Rows); err != nil {
		return err
	}
	for i, l := range r.Links {
		if strings.TrimSpace(l) == "" {
			return fmt.Errorf("link %d is empty", i+1)
		}
	}
	// An option or link the case already has adds nothing, as when an amend
	// is sent again.
	if err := checkNew("option", r.Options, cur.Options); err != nil {
		return err
	}
	if err := checkNew("link", r.Links, cur.Links); err != nil {
		return err
	}
	if err := checkLabels(r.Labels, cur.Labels); err != nil {
		return err
	}
	if len(r.Options) == 0 && len(r.Rows) == 0 && len(r.Links) == 0 && len(r.Labels) == 0 &&
		(r.Body == "" || r.Body == cur.Body) && (r.Context == "" || r.Context == cur.Context) {
		return errors.New("the amend changes nothing")
	}
	return nil
}

// checkKindFields checks that options are only on a decision case and rows
// only on an approval case.
func checkKindFields(kind Kind, options []string, rows []Row) error {
	if kind != KindDecision && len(options) > 0 {
		return fmt.Errorf("options are for decision cases, not %s", kind)
	}
	if kind != KindApproval && len(rows) > 0 {
		return fmt.Errorf("rows are for approval cases, not %s", kind)
	}
	return nil
}

// checkNew checks that each option or link an amend adds is not on the case
// already, in have, and is not added twice.
func checkNew(what string, added, have []string) error {
	for i, s := range added {
		switch {
		case slices.Contains(added[:i], s):
			return fmt.Errorf("%s %q is used twice", what, s)
		case slices.Contains(have, s):
			return fmt.Errorf("%s %q is already on the case", what, s)
		}
	}
	return nil
}

// checkLabels checks labels added to a case that already has the labels in
// have: none is blank, on the case already, or added twice.
func checkLabels(added, have []string) error {
	for i, l := range added {
		if strings.TrimSpace(l) == "" {
			return fmt.Errorf("label %d is empty", i+1)
		}
	}
	return checkNew("label", added, have)
}

// checkOptions checks that no option is blank.
func checkOptions(options []string) error {
	for i, o := range options {
		if strings.TrimSpace(o) == "" {
			return fmt.Errorf("option %d is empty", i+1)
		}
	}
	return nil
}

// checkRows checks rows added to a case that already has the rows in have:
// each id is safe to type as id=verdict and names one row on the case, and
// the label, script and link are set.
func checkRows(rows, have []Row) error {
	seen := map[string]bool{}
	for i, row := range rows {
		switch {
		case !rowIDPattern.MatchString(row.ID):
			return fmt.Errorf("row %d: id %q must be letters, digits, dot, dash or underscore", i+1, row.ID)
		case seen[row.ID]:
			return fmt.Errorf("row %d: id %q is used twice", i+1, row.ID)
		case slices.ContainsFunc(have, func(h Row) bool { return h.ID == row.ID }):
			return fmt.Errorf("row %d: id %q is already on the case", i+1, row.ID)
		case strings.TrimSpace(row.Label) == "":
			return fmt.Errorf("row %q: label is empty", row.ID)
		case strings.TrimSpace(row.Script) == "":
			return fmt.Errorf("row %q: script is empty", row.ID)
		case strings.TrimSpace(row.Link) == "":
			return fmt.Errorf("row %q: link is empty", row.ID)
		}
		seen[row.ID] = true
	}
	return nil
}

// answerFields names the fields each kind's answer may set, besides the note.
var answerFields = map[Kind][]string{
	KindDecision: {"choice", "other"},
	KindApproval: {"rows"},
	KindSignoff:  {"signoff"},
	KindStuck:    {"text", "drop"},
	KindQuestion: {"text"},
	KindFYI:      {"ack"},
}

// set lists the response fields the answer sets.
func (r *AnswerRecord) set() []string {
	var f []string
	add := func(name string, on bool) {
		if on {
			f = append(f, name)
		}
	}
	add("choice", r.Choice != 0)
	add("other", r.Other)
	add("rows", len(r.Rows) > 0)
	add("signoff", r.Signoff != "")
	add("text", r.Text != "")
	add("drop", r.Drop)
	add("ack", r.Ack)
	return f
}

// validate checks the answer against the shape the case's kind asks for, as
// the case stands in cur after any amends.
func (r *AnswerRecord) validate(cur *OpenRecord) error {
	set := r.set()
	for _, name := range set {
		if !slices.Contains(answerFields[cur.Kind], name) {
			return fmt.Errorf("%s answers cannot set %s", cur.Kind, name)
		}
	}
	if len(set) == 0 {
		return fmt.Errorf("the answer has no %s response", cur.Kind)
	}
	hasNote := strings.TrimSpace(r.Note) != ""

	switch cur.Kind {
	case KindDecision:
		if len(set) > 1 {
			return errors.New("choose one option or other, not both")
		}
		if r.Other && !hasNote {
			return errors.New("other needs a note")
		}
		if !r.Other && (r.Choice < 1 || r.Choice > len(cur.Options)) {
			return fmt.Errorf("choice %d is not an option (1-%d)", r.Choice, len(cur.Options))
		}
	case KindApproval:
		got := map[string]bool{}
		for _, row := range r.Rows {
			if !slices.ContainsFunc(cur.Rows, func(o Row) bool { return o.ID == row.ID }) {
				return fmt.Errorf("row %q is not on this case", row.ID)
			}
			if got[row.ID] {
				return fmt.Errorf("row %q is answered twice", row.ID)
			}
			switch row.Verdict {
			case VerdictApprove, VerdictHold, VerdictReject:
			default:
				return fmt.Errorf("row %q: verdict %q is not approve, hold or reject", row.ID, row.Verdict)
			}
			got[row.ID] = true
		}
		for _, o := range cur.Rows {
			if !got[o.ID] {
				return fmt.Errorf("row %q has no verdict", o.ID)
			}
		}
	case KindSignoff:
		switch r.Signoff {
		case SignoffAccept:
		case SignoffChanges:
			if !hasNote {
				return errors.New("requesting changes needs a note")
			}
		default:
			return fmt.Errorf("signoff %q is not accept or changes", r.Signoff)
		}
	case KindStuck:
		if len(set) > 1 {
			return errors.New("give guidance text or drop, not both")
		}
		if !r.Drop && strings.TrimSpace(r.Text) == "" {
			return errors.New("guidance text is empty")
		}
	case KindQuestion:
		if strings.TrimSpace(r.Text) == "" {
			return errors.New("reply text is empty")
		}
	case KindFYI:
		// Ack is the only field allowed and at least one field is set.
	}
	return nil
}
