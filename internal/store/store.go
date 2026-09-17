package store

import "context"

// Store is a case store, as the CLI and the web inbox read and write it. DB,
// a SQLite file on this machine, is the only implementation.
//
// Every implementation keeps these errors, so callers need no branching: a
// missing case or store wraps fs.ErrNotExist, a write refused by AtRevision
// wraps ErrStale, and an event the case does not allow is a *TransitionError
// or the record's validation error.
type Store interface {
	// List folds every case in the store, sorted by id. A case that fails
	// to fold is returned in the second result and does not stop the others.
	List(ctx context.Context) ([]*Case, []*LoadError, error)
	// IDs lists the case ids, sorted, without folding the cases.
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
