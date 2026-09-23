// Package config loads defaults for the cases CLI from a TOML file. The
// file only seeds kong's flag resolution, so precedence is flag >
// environment > config file > built-in default with no second code path.
package config

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/alecthomas/kong"
	toml "github.com/pelletier/go-toml/v2"
)

// EnvVar names the environment variable that overrides the default path.
const EnvVar = "CASES_CONFIG"

// Where the path in use came from, reported by `cases config path`.
const (
	SourceFlag    = "flag"
	SourceEnv     = "env"
	SourceDefault = "default"
)

const (
	dirName   = "cases"
	fileName  = "config.toml"
	storeName = "cases.db"
)

// Error is a problem with the config file itself: unreadable, malformed, or
// carrying a key or value the CLI cannot use.
type Error struct {
	Path string
	Err  error
}

func (e *Error) Error() string { return fmt.Sprintf("config file %s: %v", e.Path, e.Err) }
func (e *Error) Unwrap() error { return e.Err }

func configErr(path string, format string, args ...any) *Error {
	return &Error{Path: path, Err: fmt.Errorf(format, args...)}
}

// Key describes one setting: the TOML key, the flag it seeds, the
// environment variable that beats it, and the value that applies when
// nothing sets it. Every value is held as a string; a Bool key also takes
// an unquoted true or false.
type Key struct {
	Name     string        // TOML key
	Flag     string        // flag it seeds, when not the same as Name
	Env      string        // environment variable that wins over the file, if any
	Default  func() string // built-in default
	Bool     bool          // also accepts a TOML boolean
	Commands []string      // commands whose flags this key may seed; empty means all
	Comment  []string      // template comment, one line per entry
	Example  string        // template assignment, written commented out
}

// Keys is the full set of settings the config file may carry. Every entry
// names a flag that exists on the CLI; TestConfigKeysMatchFlags enforces
// that.
var Keys = []Key{
	{
		Name:    "store",
		Env:     "CASES_STORE",
		Default: DefaultStore,
		Comment: []string{
			"Case store: one SQLite file, made on the first write. Same as --store",
			"or $CASES_STORE. A leading ~ is expanded. Keep it on a local disk,",
			"not in a synced folder or on a network share. Default:",
			"$XDG_DATA_HOME/cases/cases.db, or ~/.local/share/cases/cases.db.",
		},
		Example: `store = "~/cases/work.db"`,
	},
	{
		Name:     "listen",
		Default:  func() string { return "127.0.0.1:8765" },
		Commands: []string{"serve", "service install"},
		Comment: []string{
			"Address cases serve listens on. Same as --listen on serve and",
			"service install. Loopback only.",
		},
		Example: `listen = "127.0.0.1:8765"`,
	},
	{
		Name:     "no-open",
		Default:  func() string { return "false" },
		Bool:     true,
		Commands: []string{"serve"},
		Comment: []string{
			"Set to true to stop cases serve opening the inbox in the browser.",
			"Same as --no-open.",
		},
		Example: `no-open = true`,
	},
	{
		Name:     "name",
		Flag:     "as",
		Default:  func() string { return "" },
		Commands: []string{"answer", "resume", "serve", "service install"},
		Comment: []string{
			"Your name, recorded as the actor on answers, parks and resumes from",
			"cases answer, cases resume and the inbox. Same as --as on those",
			"commands. Default: none is recorded.",
		},
		Example: `name = "Ryan"`,
	},
	{
		Name:     "prune-age",
		Flag:     "age",
		Default:  func() string { return "720h" },
		Commands: []string{"prune"},
		Comment: []string{
			"How long after its last event a closed or withdrawn case is pruned,",
			"as a Go duration (h, m, s). Same as --age on cases prune.",
		},
		Example: `prune-age = "2160h"`,
	},
}

// FlagName is the flag the key seeds.
func (k Key) FlagName() string {
	if k.Flag != "" {
		return k.Flag
	}
	return k.Name
}

// KeyNames lists the key names in declaration order.
func KeyNames() []string {
	names := make([]string, len(Keys))
	for i, k := range Keys {
		names[i] = k.Name
	}
	return names
}

func lookup(name string) (Key, bool) {
	for _, k := range Keys {
		if name == k.Name {
			return k, true
		}
	}
	return Key{}, false
}

// DefaultPath is $XDG_CONFIG_HOME/cases/config.toml, falling back to
// ~/.config/cases/config.toml.
func DefaultPath() (string, error) {
	base := os.Getenv("XDG_CONFIG_HOME")
	if base == "" {
		home := os.Getenv("HOME")
		if home == "" {
			return "", errors.New("cannot locate the config file: neither $XDG_CONFIG_HOME nor $HOME is set")
		}
		base = filepath.Join(home, ".config")
	}
	return filepath.Join(base, dirName, fileName), nil
}

// DefaultStore is the store used when nothing names one:
// $XDG_DATA_HOME/cases/cases.db, falling back to
// ~/.local/share/cases/cases.db. It is empty when neither variable is set.
func DefaultStore() string {
	base := os.Getenv("XDG_DATA_HOME")
	if base == "" {
		home := os.Getenv("HOME")
		if home == "" {
			return ""
		}
		base = filepath.Join(home, ".local", "share")
	}
	return filepath.Join(base, dirName, storeName)
}

// ExpandHome replaces a leading ~ with $HOME, so a store path can be
// written that way in the file.
func ExpandHome(path string) string {
	if path != "~" && !strings.HasPrefix(path, "~/") {
		return path
	}
	home := os.Getenv("HOME")
	if home == "" {
		return path
	}
	return filepath.Join(home, path[1:])
}

// ResolvePath picks the config file to use: an explicit --config path
// wins, then $CASES_CONFIG, then the default location.
func ResolvePath(explicit string) (path, source string, err error) {
	if explicit != "" {
		return ExpandHome(explicit), SourceFlag, nil
	}
	if env := os.Getenv(EnvVar); env != "" {
		return ExpandHome(env), SourceEnv, nil
	}
	p, err := DefaultPath()
	if err != nil {
		return "", "", err
	}
	return p, SourceDefault, nil
}

// File is a loaded config file. The file is optional: one that is not
// there loads fine, with Exists false and no values.
type File struct {
	Path   string
	Source string
	Exists bool

	// Err is why the file could not be used, when it could not be. The
	// File is still returned so `cases config` can say which file is at
	// fault; it supplies no values in that state.
	Err error

	values map[string]string
}

// Load reads and checks the config file at path. The returned File is never
// nil: alongside an error it still names the file that failed.
func Load(path string) (*File, error) {
	f := &File{Path: path, Source: SourceDefault, values: map[string]string{}}
	fail := func(err error) (*File, error) {
		f.Err = err
		f.values = map[string]string{}
		return f, err
	}

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return f, nil
		}
		// Something is at the path that cannot be read: a directory, or a
		// file without read permission. Reporting it as absent would let
		// `config init` overwrite it.
		if _, statErr := os.Lstat(path); statErr == nil {
			f.Exists = true
		}
		return fail(configErr(path, "cannot read: %w", err))
	}
	f.Exists = true

	var raw map[string]any
	if err := toml.Unmarshal(data, &raw); err != nil {
		return fail(configErr(path, "invalid TOML: %s", tomlErrorText(err)))
	}
	for name, value := range raw {
		key, ok := lookup(name)
		if !ok {
			return fail(configErr(path, "unknown key %q (valid keys: %s)", name, strings.Join(KeyNames(), ", ")))
		}
		switch v := value.(type) {
		case string:
			f.values[key.Name] = v
		case bool:
			if !key.Bool {
				return fail(configErr(path, "key %q must be a string, got boolean", key.Name))
			}
			f.values[key.Name] = strconv.FormatBool(v)
		default:
			if key.Bool {
				return fail(configErr(path, "key %q must be a boolean or a string, got %s", key.Name, typeName(value)))
			}
			return fail(configErr(path, "key %q must be a string, got %s", key.Name, typeName(value)))
		}
	}
	return f, nil
}

// Setting is one key's effective value and where it came from, as reported
// by `cases config show`.
type Setting struct {
	Key    string
	Value  string
	Source string // "env", "config" or "default"
}

// Settings reports the value each key has once the environment and the file
// are taken into account: what a command sees when no flag overrides it.
func (f *File) Settings() []Setting {
	out := make([]Setting, 0, len(Keys))
	for _, k := range Keys {
		s := Setting{Key: k.Name, Value: k.Default(), Source: "default"}
		if f != nil {
			if v, ok := f.values[k.Name]; ok {
				s.Value, s.Source = v, "config"
			}
		}
		if k.Env != "" {
			if v := strings.TrimSpace(os.Getenv(k.Env)); v != "" {
				s.Value, s.Source = v, "env"
			}
		}
		if k.Name == "store" {
			s.Value = ExpandHome(s.Value)
		}
		out = append(out, s)
	}
	return out
}

// Resolver seeds kong's flag resolution from the file. It returns nil for a
// flag the file does not mention, or whose environment variable is set,
// leaving kong's own value in place.
func (f *File) Resolver() kong.Resolver {
	values := map[string]string{}
	if f != nil && f.Err == nil {
		for k, v := range f.values {
			values[k] = v
		}
	}
	return kong.ResolverFunc(func(_ *kong.Context, parent *kong.Path, flag *kong.Flag) (any, error) {
		i := slices.IndexFunc(Keys, func(k Key) bool {
			return k.FlagName() == flag.Name && (len(k.Commands) == 0 || slices.Contains(k.Commands, parent.Node().Path()))
		})
		if i < 0 {
			return nil, nil
		}
		v, ok := values[Keys[i].Name]
		if !ok {
			return nil, nil
		}
		// kong applies environment variables before resolvers run, and a
		// resolved value would replace them; the environment wins.
		for _, env := range flag.Envs {
			if strings.TrimSpace(os.Getenv(env)) != "" {
				return nil, nil
			}
		}
		return v, nil
	})
}

// Template is the commented file `cases config init` writes.
func Template() string {
	var b strings.Builder
	b.WriteString("# cases configuration\n")
	b.WriteString("#\n")
	b.WriteString("# Defaults for the flags below. Precedence is flag > environment >\n")
	b.WriteString("# this file > built-in default.\n")
	b.WriteString("#\n")
	b.WriteString("# Read from $XDG_CONFIG_HOME/cases/config.toml, or\n")
	b.WriteString("# ~/.config/cases/config.toml. Override with --config PATH or\n")
	b.WriteString("# $" + EnvVar + ". The file is optional.\n")
	b.WriteString("#\n")
	b.WriteString("# Uncomment a line to change the default.\n")
	for _, k := range Keys {
		b.WriteString("\n")
		for _, line := range k.Comment {
			b.WriteString("# " + line + "\n")
		}
		b.WriteString("# " + k.Example + "\n")
	}
	return b.String()
}

func typeName(v any) string {
	switch v.(type) {
	case bool:
		return "boolean"
	case string:
		return "string"
	case int64, float64:
		return "number"
	case map[string]any:
		return "table"
	case []any:
		return "array"
	default:
		return fmt.Sprintf("%T", v)
	}
}

// tomlErrorText renders a decode error on one line with the row and column
// the parser stopped at, instead of go-toml's multi-line excerpt.
func tomlErrorText(err error) string {
	var de *toml.DecodeError
	if errors.As(err, &de) {
		row, col := de.Position()
		return fmt.Sprintf("line %d, column %d: %s", row, col, strings.TrimPrefix(de.Error(), "toml: "))
	}
	return err.Error()
}
