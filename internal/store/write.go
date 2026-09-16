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

// link, rename and remove are os.Link, os.Rename and os.Remove, replaceable so
// tests can fail or race the last step of a write.
var (
	link   = os.Link
	rename = os.Rename
	remove = os.Remove
)

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

// ErrStale refuses a write made with AtRevision: an event has been written to
// the case since the caller read it.
var ErrStale = errors.New("the case has changed since it was read")

// Precondition is a check on the case as it is on disk, made while the case is
// locked for the write and before the event is checked. Make one with
// AtRevision.
type Precondition struct {
	revision int
}

// AtRevision is the precondition that the case is still at revision rev: the
// Revision of the case as the caller read it. Otherwise the write fails with
// ErrStale and nothing is written.
func AtRevision(rev int) Precondition {
	return Precondition{revision: rev}
}

// Answer records the human's answer.
func Answer(dir string, rec AnswerRecord, pre ...Precondition) (*Case, error) {
	return appendEvent(dir, AuthorHuman, EventAnswer, &rec, pre...)
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
func Park(dir string, rec ParkRecord, pre ...Precondition) (*Case, error) {
	return appendEvent(dir, AuthorHuman, EventPark, &rec, pre...)
}

// Resume reopens a parked case. Either side may resume.
func Resume(dir string, author Author, rec ResumeRecord, pre ...Precondition) (*Case, error) {
	return appendEvent(dir, author, EventResume, &rec, pre...)
}

// appendEvent folds the case, checks the preconditions and then the new event
// against it with the same code the fold uses, and only then writes the next
// event file. The case directory is locked for the whole sequence so two local
// writers cannot take the same sequence number, and a precondition cannot pass
// on a case that changes before the write.
func appendEvent(dir string, author Author, typ EventType, rec record, pre ...Precondition) (*Case, error) {
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
	for _, p := range pre {
		if rev := c.Revision(); rev != p.revision {
			return nil, fmt.Errorf("%w: read at revision %d, now at %d", ErrStale, p.revision, rev)
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
	c.files++
	return c, nil
}

// writeFileAtomic writes data to a temporary file in dir and then publishes it
// under name, so a reader sees either no file or the whole file. The temporary
// name starts with a dot, which the fold ignores. It refuses to replace a file
// that already exists.
func writeFileAtomic(dir, name string, data []byte) (err error) {
	final := filepath.Join(dir, name)
	// A cheap early refusal. It cannot see a file added after it runs;
	// publish refuses that one.
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
	if err = publish(tmp, final); err != nil {
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

// publish gives the complete temporary file tmp the name final. It refuses to
// replace a file that already has the name, such as one a sync client added
// after writeFileAtomic checked for it.
//
// It links rather than renames, because a link fails when the name is taken
// and a rename would replace the file. Where hard links do not work (FAT,
// exFAT, some network mounts, a sandbox that denies them) the link fails with
// another error, and publish renames once it has checked that the name is
// still free. A file added between that check and the rename is replaced.
func publish(tmp, final string) error {
	err := link(tmp, final)
	if err == nil {
		// The fold skips the temporary name if removing it fails.
		_ = remove(tmp)
		return nil
	}
	if !errors.Is(err, fs.ErrExist) {
		_, serr := os.Lstat(final)
		if errors.Is(serr, fs.ErrNotExist) {
			return rename(tmp, final)
		}
		if serr != nil {
			return serr
		}
	}
	// The name is taken, possibly by this link: on a network mount a link can
	// go through and still report an error, as when a retried request finds
	// the link the first one made.
	if sameFile(tmp, final) {
		_ = remove(tmp)
		return nil
	}
	return fmt.Errorf("%s already exists", final)
}

// sameFile reports whether the names a and b are links to one file.
func sameFile(a, b string) bool {
	ai, err := os.Lstat(a)
	if err != nil {
		return false
	}
	bi, err := os.Lstat(b)
	return err == nil && os.SameFile(ai, bi)
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
