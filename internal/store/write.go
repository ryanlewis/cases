package store

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// idTimeLayout is the timestamp at the front of a case id. Colons are left out
// so the id is a safe directory name everywhere.
const idTimeLayout = "2006-01-02T15-04-05Z"

// maxSlug caps the title part of a case id.
const maxSlug = 48

// now is the clock for event timestamps. Tests replace it.
var now = func() time.Time { return time.Now().UTC() }

// rename is os.Rename, replaceable so tests can fail the last step of a write.
var rename = os.Rename

// CaseDir returns the directory for case id in the store root. It refuses an
// id that would name anything other than a direct child of root.
func CaseDir(root, id string) (string, error) {
	if id == "" || strings.HasPrefix(id, ".") || filepath.Base(id) != id || strings.ContainsAny(id, `/\`) {
		return "", fmt.Errorf("invalid case id %q", id)
	}
	return filepath.Join(root, id), nil
}

// Create opens a new case in root, creating root if it does not exist. The
// case id is the open time and a slug of the title; if that directory already
// exists a numeric suffix is added.
func Create(root string, rec OpenRecord) (*Case, error) {
	rec.stamp(now())
	if err := rec.validate(); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(root, 0o755); err != nil {
		return nil, err
	}
	base := rec.OpenedAt.Format(idTimeLayout) + "-" + Slug(rec.Title)
	var dir string
	for n := 1; ; n++ {
		id := base
		if n > 1 {
			id = fmt.Sprintf("%s-%d", base, n)
		}
		dir = filepath.Join(root, id)
		err := os.Mkdir(dir, 0o755)
		if err == nil {
			break
		}
		if !errors.Is(err, fs.ErrExist) || n >= 100 {
			return nil, err
		}
	}
	c, err := appendEvent(dir, AuthorAgent, EventOpen, &rec)
	if err != nil {
		// The directory is ours and empty; leaving it would show up as a
		// broken case in every listing.
		_ = os.Remove(dir)
		return nil, err
	}
	return c, nil
}

// Answer records the human's answer.
func Answer(dir string, rec AnswerRecord) (*Case, error) {
	return appendEvent(dir, AuthorHuman, EventAnswer, &rec)
}

// Pickup records that the agent has read the answer.
func Pickup(dir string, rec PickupRecord) (*Case, error) {
	return appendEvent(dir, AuthorAgent, EventPickup, &rec)
}

// Note records a follow-up from the agent. It reopens an answered or picked-up
// case.
func Note(dir string, rec NoteRecord) (*Case, error) {
	return appendEvent(dir, AuthorAgent, EventNote, &rec)
}

// Close records the outcome of a picked-up case.
func Close(dir string, rec CloseRecord) (*Case, error) {
	return appendEvent(dir, AuthorAgent, EventClose, &rec)
}

// Withdraw records that the agent no longer needs an open case answered.
func Withdraw(dir string, rec WithdrawRecord) (*Case, error) {
	return appendEvent(dir, AuthorAgent, EventWithdraw, &rec)
}

// Park records the human parking an open stuck case.
func Park(dir string, rec ParkRecord) (*Case, error) {
	return appendEvent(dir, AuthorHuman, EventPark, &rec)
}

// Resume reopens a parked case. Either side may resume.
func Resume(dir string, author Author, rec ResumeRecord) (*Case, error) {
	return appendEvent(dir, author, EventResume, &rec)
}

// appendEvent folds the case, checks the new event against it with the same
// code the fold uses, and only then writes the next event file. The case
// directory is locked for the whole sequence so two local writers cannot take
// the same sequence number.
func appendEvent(dir string, author Author, typ EventType, rec record) (*Case, error) {
	rec.stamp(now())
	data, err := json.MarshalIndent(rec, "", "  ")
	if err != nil {
		return nil, err
	}
	data = append(data, '\n')

	unlock, err := lockDir(dir)
	if err != nil {
		return nil, err
	}
	defer unlock()

	c := &Case{ID: filepath.Base(dir), Dir: dir}
	if typ != EventOpen {
		if c, err = Load(dir); err != nil {
			return nil, err
		}
	}
	seq := c.lastSeq + 1
	name := fmt.Sprintf("%04d-%s-%s.json", seq, author, typ)
	if err := c.apply(Event{Seq: seq, Author: author, Type: typ, File: name, Data: data}); err != nil {
		return nil, err
	}
	if err := writeFileAtomic(dir, name, data); err != nil {
		return nil, err
	}
	c.lastSeq = seq
	return c, nil
}

// writeFileAtomic writes data to a temporary file in dir and renames it into
// place, so a reader sees either no file or the whole file. The temporary name
// starts with a dot, which the fold ignores. It refuses to replace a file that
// already exists.
func writeFileAtomic(dir, name string, data []byte) (err error) {
	final := filepath.Join(dir, name)
	if _, err := os.Lstat(final); err == nil {
		return fmt.Errorf("%s already exists", final)
	}
	f, err := os.CreateTemp(dir, ".tmp-*")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer func() {
		if err != nil {
			_ = os.Remove(tmp)
		}
	}()
	if _, err = f.Write(data); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Sync(); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	if err = os.Chmod(tmp, 0o644); err != nil {
		return err
	}
	if err = rename(tmp, final); err != nil {
		return err
	}
	// Persist the new directory entry. A failure here does not undo the
	// write, which has already happened.
	if d, derr := os.Open(dir); derr == nil {
		_ = d.Sync()
		_ = d.Close()
	}
	return nil
}

// Slug turns a title into the lowercase ASCII words-and-dashes part of a case
// id.
func Slug(title string) string {
	var b strings.Builder
	dash := false
	for _, r := range strings.ToLower(title) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') {
			b.WriteRune(r)
			dash = false
			continue
		}
		if b.Len() > 0 && !dash {
			b.WriteByte('-')
			dash = true
		}
	}
	s := strings.TrimRight(b.String(), "-")
	if len(s) > maxSlug {
		s = s[:maxSlug]
		if i := strings.LastIndexByte(s, '-'); i > 0 {
			s = s[:i]
		}
	}
	if s == "" {
		return "case"
	}
	return s
}
