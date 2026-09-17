// Package skill bundles the cases agent skill and manages its
// installation into supported AI coding agents (e.g. Claude Code).
//
// The skill body is authored in a neutral Markdown source (SKILL.md),
// embedded into the binary. Each Agent adapter renders that source into
// on-disk files appropriate for its target (e.g. Claude Code's SKILL.md with
// YAML frontmatter).
package skill

import (
	"bytes"
	_ "embed"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

//go:embed SKILL.md
var body string

// Skill identity shared across agents. Individual agents may override these
// when their target requires it, but today every agent uses the defaults.
const (
	Name        = "cases"
	Description = "Use when the agent needs a human decision, approval, sign-off or answer before it can go on, is stuck, or has something the human should know, or when the user mentions cases or the cases inbox. Provides the `cases` CLI for opening a case, waiting for the answer, picking it up and closing it with the outcome."
)

// Body returns the neutral skill source body (no frontmatter).
func Body() string { return body }

// SkillMD returns a self-contained SKILL.md: shared frontmatter + body.
func SkillMD() string {
	return fmt.Sprintf("---\nname: %s\ndescription: %s\n---\n\n%s", Name, Description, body)
}

// sharedFiles is the SKILL.md payload every registered agent installs today.
// Agents that need extra files should compose this with their own additions.
var sharedFiles = map[string][]byte{
	"SKILL.md": []byte(SkillMD()),
}

// tildeRule says whether an agent expands a leading tilde in its own config
// directory variable. We have to match the agent exactly: expanding where the
// agent does not — or leaving it alone where the agent expands — installs the
// skill somewhere the agent never looks.
type tildeRule bool

const (
	// verbatimTilde hands the value to the filesystem as given, which is what
	// Claude Code and Codex do. Claude Code goes further and refuses to start
	// unless $CLAUDE_CONFIG_DIR is absolute.
	verbatimTilde tildeRule = false
	// expandsTilde rewrites a leading "~" or "~/" to $HOME first, which is
	// what Pi does to $PI_CODING_AGENT_DIR before reading from it.
	expandsTilde tildeRule = true
)

// resolveAgentDir returns the skill directory for an agent whose config
// directory can be relocated with envVar, falling back to $HOME joined with
// fallback when the variable is unset or empty. Beyond the agent's own tilde
// rule the value is used exactly as given — no relative-path rewriting.
func resolveAgentDir(envVar string, tilde tildeRule, fallback ...string) (string, error) {
	home := os.Getenv("HOME")
	base := os.Getenv(envVar)
	switch {
	case base == "":
		if home == "" {
			return "", fmt.Errorf("cannot locate the skill directory: neither $%s nor $HOME is set", envVar)
		}
		base = filepath.Join(append([]string{home}, fallback...)...)
	case tilde == expandsTilde && home != "":
		base = expandHomeTilde(base, home)
	}
	return filepath.Join(base, "skills", Name), nil
}

// expandHomeTilde rewrites a leading "~" or "~/" to home. Anything else — a
// "~user" form, a bare relative path — is left alone, matching the expansion
// Pi applies to $PI_CODING_AGENT_DIR.
func expandHomeTilde(path, home string) string {
	if path == "~" {
		return home
	}
	if strings.HasPrefix(path, "~/") {
		return filepath.Join(home, path[2:])
	}
	return path
}

// Agent renders and locates the skill for a particular AI coding agent.
type Agent interface {
	Name() string
	DefaultDir() (string, error)
	Files() map[string][]byte
}

var registry = map[string]Agent{}

func register(a Agent) { registry[a.Name()] = a }

// Lookup returns the agent adapter with the given name.
func Lookup(name string) (Agent, error) {
	a, ok := registry[name]
	if !ok {
		return nil, fmt.Errorf("unknown agent %q (supported: %s)", name, AgentNames())
	}
	return a, nil
}

// Agents returns the registered agents, sorted by name.
func Agents() []Agent {
	out := make([]Agent, 0, len(registry))
	for _, a := range registry {
		out = append(out, a)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name() < out[j].Name() })
	return out
}

// AgentNames returns a comma-separated list of registered agent names.
func AgentNames() string {
	agents := Agents()
	names := make([]string, len(agents))
	for i, a := range agents {
		names[i] = a.Name()
	}
	return strings.Join(names, ", ")
}

// Exists reports whether any of the agent's files are present in dir.
func Exists(a Agent, dir string) bool {
	for name := range a.Files() {
		if _, err := os.Stat(filepath.Join(dir, name)); err == nil {
			return true
		}
	}
	return false
}

// InstalledFiles returns the agent's files present on disk under dir,
// sorted for stable output.
func InstalledFiles(a Agent, dir string) []string {
	var found []string
	for name := range a.Files() {
		if _, err := os.Stat(filepath.Join(dir, name)); err == nil {
			found = append(found, name)
		}
	}
	sort.Strings(found)
	return found
}

// Status says how the skill under a directory compares with what this binary
// renders.
type Status int

const (
	// NotInstalled means none of the agent's files are present.
	NotInstalled Status = iota
	// Installed means every file is present and matches byte for byte.
	Installed
	// Stale means some file is present but at least one is missing or
	// differs, as after upgrading cases without reinstalling the skill.
	Stale
)

func (s Status) String() string {
	switch s {
	case Installed:
		return "installed"
	case Stale:
		return "stale"
	default:
		return "not installed"
	}
}

// Check compares the agent's files under dir with the bundled rendering.
func Check(a Agent, dir string) Status {
	present, same := 0, 0
	for name, content := range a.Files() {
		got, err := os.ReadFile(filepath.Join(dir, name))
		if err != nil {
			continue
		}
		present++
		if bytes.Equal(got, content) {
			same++
		}
	}
	switch {
	case present == 0:
		return NotInstalled
	case same == len(a.Files()):
		return Installed
	default:
		return Stale
	}
}

// Install writes the agent's rendered files to dir, creating it if needed.
// A file that already matches is left alone; any other is overwritten.
func Install(a Agent, dir string) error {
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	for name, content := range a.Files() {
		path := filepath.Join(dir, name)
		if got, err := os.ReadFile(path); err == nil && bytes.Equal(got, content) {
			continue
		}
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return err
		}
		if err := os.WriteFile(path, content, 0o644); err != nil {
			return err
		}
	}
	return nil
}

// Uninstall removes the agent's installed files from dir. If the directory is
// empty afterwards, it is also removed.
func Uninstall(a Agent, dir string) error {
	for name := range a.Files() {
		path := filepath.Join(dir, name)
		if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	entries, err := os.ReadDir(dir)
	if err == nil && len(entries) == 0 {
		_ = os.Remove(dir)
	}
	return nil
}
