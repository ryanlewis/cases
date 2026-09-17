package store

import "context"

// Store is a case store, as the CLI and the web inbox read and write it. Dir,
// a directory on this machine, is the only implementation.
//
// Every implementation keeps the directory store's errors, so callers need no
// branching: a missing case or store wraps fs.ErrNotExist, a write refused by
// AtRevision wraps ErrStale, and an event the case does not allow is a
// *TransitionError or the record's validation error.
type Store interface {
	// List folds every case in the store, as List does.
	List(ctx context.Context) ([]*Case, []*LoadError, error)
	// IDs lists the case ids without folding the cases, as CaseIDs does.
	IDs(ctx context.Context) ([]string, error)
	// Get folds the case with exactly this id.
	Get(ctx context.Context, id string) (*Case, error)
	// NewPoller returns a CasePoller over the store.
	NewPoller() CasePoller

	Create(ctx context.Context, rec OpenRecord) (*Case, error)
	Amend(ctx context.Context, id string, rec AmendRecord, pre ...Precondition) (*Case, error)
	Answer(ctx context.Context, id string, rec AnswerRecord, pre ...Precondition) (*Case, error)
	Pickup(ctx context.Context, id string, rec PickupRecord, pre ...Precondition) (*Case, error)
	Note(ctx context.Context, id string, rec NoteRecord, pre ...Precondition) (*Case, error)
	Close(ctx context.Context, id string, rec CloseRecord, pre ...Precondition) (*Case, error)
	Withdraw(ctx context.Context, id string, rec WithdrawRecord, pre ...Precondition) (*Case, error)
	Park(ctx context.Context, id string, rec ParkRecord, pre ...Precondition) (*Case, error)
	Resume(ctx context.Context, id string, author Author, rec ResumeRecord, pre ...Precondition) (*Case, error)
}

// CasePoller lists a store repeatedly; Poll returns the same as Store.List.
// *Poller is one.
type CasePoller interface {
	Poll() ([]*Case, []*LoadError, error)
}

// Dir is the store in the directory Root. Its methods resolve an id with
// CaseDir and call the package functions of the same name; ctx is unused.
type Dir struct {
	Root string
}

var _ Store = (*Dir)(nil)

// NewDir returns the store in the directory root.
func NewDir(root string) *Dir {
	return &Dir{Root: root}
}

func (d *Dir) List(context.Context) ([]*Case, []*LoadError, error) {
	return List(d.Root)
}

func (d *Dir) IDs(context.Context) ([]string, error) {
	return CaseIDs(d.Root)
}

func (d *Dir) Get(_ context.Context, id string) (*Case, error) {
	dir, err := CaseDir(d.Root, id)
	if err != nil {
		return nil, err
	}
	return Load(dir)
}

func (d *Dir) NewPoller() CasePoller {
	return NewPoller(d.Root)
}

func (d *Dir) Create(_ context.Context, rec OpenRecord) (*Case, error) {
	return Create(d.Root, rec)
}

func (d *Dir) Amend(_ context.Context, id string, rec AmendRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Amend(dir, rec, pre...) })
}

func (d *Dir) Answer(_ context.Context, id string, rec AnswerRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Answer(dir, rec, pre...) })
}

func (d *Dir) Pickup(_ context.Context, id string, rec PickupRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Pickup(dir, rec, pre...) })
}

func (d *Dir) Note(_ context.Context, id string, rec NoteRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Note(dir, rec, pre...) })
}

func (d *Dir) Close(_ context.Context, id string, rec CloseRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Close(dir, rec, pre...) })
}

func (d *Dir) Withdraw(_ context.Context, id string, rec WithdrawRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Withdraw(dir, rec, pre...) })
}

func (d *Dir) Park(_ context.Context, id string, rec ParkRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Park(dir, rec, pre...) })
}

func (d *Dir) Resume(_ context.Context, id string, author Author, rec ResumeRecord, pre ...Precondition) (*Case, error) {
	return d.write(id, func(dir string) (*Case, error) { return Resume(dir, author, rec, pre...) })
}

// write resolves id to its directory and runs the append on it.
func (d *Dir) write(id string, appendTo func(dir string) (*Case, error)) (*Case, error) {
	dir, err := CaseDir(d.Root, id)
	if err != nil {
		return nil, err
	}
	return appendTo(dir)
}
