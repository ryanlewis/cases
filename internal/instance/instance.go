// Package instance records a running `cases serve` in a small JSON file, so
// `cases status` can say where it is listening. The file lives in this
// machine's state directory, never in the store, which may be synced to other
// machines.
package instance

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net"
	"os"
	"path/filepath"
	"time"

	"github.com/ryanlewis/cases/internal/store"
)

// Info is what the file records.
type Info struct {
	PID       int       `json:"pid"`
	URL       string    `json:"url"`
	Addr      string    `json:"addr"`
	Store     string    `json:"store"`
	StartedAt time.Time `json:"started_at"`
	Version   string    `json:"version"`
}

// Dir is $XDG_STATE_HOME/cases, falling back to ~/.local/state/cases.
func Dir() (string, error) {
	base := os.Getenv("XDG_STATE_HOME")
	if base == "" {
		home := os.Getenv("HOME")
		if home == "" {
			return "", errors.New("cannot locate the state directory: neither $XDG_STATE_HOME nor $HOME is set")
		}
		base = filepath.Join(home, ".local", "state")
	}
	return filepath.Join(base, "cases"), nil
}

// Path is the file for one store: a slug of the store's directory name and a
// hash of its absolute path, so serves on two stores do not share a file.
func Path(storeDir string) (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	abs, err := filepath.Abs(storeDir)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256([]byte(abs))
	return filepath.Join(dir, "serve-"+store.Slug(filepath.Base(abs))+"-"+hex.EncodeToString(sum[:6])+".json"), nil
}

// Write records info for info.Store, replacing any file already there. The
// write goes to a temporary file that is renamed into place.
func Write(info Info) (err error) {
	path, err := Path(info.Store)
	if err != nil {
		return err
	}
	data, err := json.MarshalIndent(info, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".tmp-*")
	if err != nil {
		return err
	}
	tmp := f.Name()
	defer func() {
		if err != nil {
			_ = os.Remove(tmp)
		}
	}()
	if _, err = f.Write(append(data, '\n')); err != nil {
		_ = f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// Running returns the serve recorded for storeDir if it is still running, or
// nil. A file is stale when its process is gone or nothing accepts
// connections on its address; the second check covers a pid that was reused
// by an unrelated process after a crash.
func Running(storeDir string) (*Info, error) {
	path, err := Path(storeDir)
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var info Info
	if err := json.Unmarshal(data, &info); err != nil {
		return nil, &os.PathError{Op: "read", Path: path, Err: err}
	}
	if info.PID <= 0 || !alive(info.PID) {
		return nil, nil
	}
	conn, err := net.DialTimeout("tcp", info.Addr, 500*time.Millisecond)
	if err != nil {
		return nil, nil
	}
	_ = conn.Close()
	return &info, nil
}

// Remove deletes the file for storeDir if it still records pid, so a serve
// never removes a file another serve wrote.
func Remove(storeDir string, pid int) error {
	path, err := Path(storeDir)
	if err != nil {
		return err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil
		}
		return err
	}
	var info Info
	if json.Unmarshal(data, &info) == nil && info.PID != pid {
		return nil
	}
	return os.Remove(path)
}
