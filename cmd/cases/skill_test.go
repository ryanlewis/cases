package main

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/ryanlewis/cases/internal/skill"
)

func TestSkillShowPrintsFrontmatter(t *testing.T) {
	out := mustRun(t, "skill", "show")
	if !strings.HasPrefix(out, "---\nname: cases\ndescription: ") {
		t.Errorf("skill show = %q, want the frontmatter first", out[:min(len(out), 80)])
	}
	if !strings.Contains(out, "cases wait") {
		t.Error("skill show is missing the body")
	}

	out = mustRun(t, "skill", "show", "claude")
	if !strings.HasPrefix(out, "# SKILL.md\n---\nname: cases\n") {
		t.Errorf("skill show claude = %q, want the rendered SKILL.md", out[:min(len(out), 80)])
	}
}

func TestSkillInstallListUninstall(t *testing.T) {
	t.Setenv("CLAUDE_CONFIG_DIR", t.TempDir())
	dir := filepath.Join(t.TempDir(), "cases")

	out := mustRun(t, "skill", "install", "claude", "--path", dir)
	if !strings.Contains(out, "Installed claude skill to "+dir) {
		t.Errorf("install = %q", out)
	}
	got, err := os.ReadFile(filepath.Join(dir, "SKILL.md"))
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != skill.SkillMD() {
		t.Error("installed SKILL.md differs from the bundled one")
	}

	// An identical install is a no-op and needs no -y.
	out = mustRun(t, "skill", "install", "claude", "--path", dir)
	if !strings.Contains(out, "claude skill at "+dir+" is already up to date") {
		t.Errorf("reinstall = %q, want already up to date", out)
	}

	// Stdin is not a terminal, so overwriting a different install needs -y.
	if err := os.WriteFile(filepath.Join(dir, "SKILL.md"), []byte("old\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	r := runCases(t, "", "skill", "install", "claude", "--path", dir)
	if r.err == nil || !strings.Contains(r.err.Error(), "-y") {
		t.Errorf("reinstall over a different skill without -y: err = %v, want a refusal naming -y", r.err)
	}
	if got, _ := os.ReadFile(filepath.Join(dir, "SKILL.md")); string(got) != "old\n" {
		t.Error("refused install overwrote SKILL.md")
	}
	out = mustRun(t, "skill", "install", "claude", "--path", dir, "-y")
	if !strings.Contains(out, "Installed claude skill to "+dir) {
		t.Errorf("install -y = %q", out)
	}
	if got, _ := os.ReadFile(filepath.Join(dir, "SKILL.md")); string(got) != skill.SkillMD() {
		t.Error("install -y did not overwrite the different SKILL.md")
	}

	if out := mustRun(t, "skill", "list"); !strings.Contains(out, "not installed") {
		t.Errorf("list = %q, want claude not installed at its default dir", out)
	}
	claudeDir := filepath.Join(os.Getenv("CLAUDE_CONFIG_DIR"), "skills", "cases")
	mustRun(t, "skill", "install", "claude")
	out = mustRun(t, "skill", "list")
	if !strings.Contains(out, claudeDir+"  (installed)") {
		t.Errorf("list = %q, want %s installed", out, claudeDir)
	}

	if err := os.WriteFile(filepath.Join(claudeDir, "SKILL.md"), []byte("old\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	out = mustRun(t, "skill", "list")
	if !strings.Contains(out, claudeDir+"  (stale)") {
		t.Errorf("list = %q, want %s stale", out, claudeDir)
	}
	mustRun(t, "skill", "install", "claude", "-y")

	r = runCases(t, "", "skill", "uninstall", "claude")
	if r.err == nil || !strings.Contains(r.err.Error(), "-y") {
		t.Errorf("uninstall without -y: err = %v, want a refusal naming -y", r.err)
	}
	mustRun(t, "skill", "uninstall", "claude", "-y")
	if _, err := os.Stat(filepath.Join(claudeDir, "SKILL.md")); !os.IsNotExist(err) {
		t.Errorf("SKILL.md still there after uninstall: %v", err)
	}
}

func TestSkillCheck(t *testing.T) {
	for _, env := range []string{"CLAUDE_CONFIG_DIR", "CODEX_HOME", "PI_CODING_AGENT_DIR"} {
		t.Setenv(env, t.TempDir())
	}
	claudeDir := filepath.Join(os.Getenv("CLAUDE_CONFIG_DIR"), "skills", "cases")

	// Nothing installed is not a failure.
	if out := mustRun(t, "skill", "check"); out != "" {
		t.Errorf("check with nothing installed = %q, want no output", out)
	}

	mustRun(t, "skill", "install", "claude")
	if out := mustRun(t, "skill", "check"); out != "" {
		t.Errorf("check with an up-to-date skill = %q, want no output", out)
	}

	if err := os.WriteFile(filepath.Join(claudeDir, "SKILL.md"), []byte("old\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"skill", "check"}, {"skill", "check", "claude"}} {
		r := runCases(t, "", args...)
		var ee *exitError
		if !errors.As(r.err, &ee) || ee.code != 1 {
			t.Errorf("%v: err = %v, want exit 1", args, r.err)
		}
		if want := "claude skill at " + claudeDir + " differs from this binary"; !strings.Contains(r.stdout, want) {
			t.Errorf("%v: stdout = %q, want %q", args, r.stdout, want)
		}
	}

	// Checking another agent ignores the stale claude skill.
	if out := mustRun(t, "skill", "check", "codex"); out != "" {
		t.Errorf("check codex = %q, want no output", out)
	}

	r := runCases(t, "", "skill", "check", "bogus")
	if r.err == nil || !strings.Contains(r.err.Error(), "supported") {
		t.Errorf("check bogus: err = %v, want the supported agents listed", r.err)
	}
}

func TestSkillUnknownAgent(t *testing.T) {
	r := runCases(t, "", "skill", "install", "bogus", "--path", t.TempDir())
	if r.err == nil || !strings.Contains(r.err.Error(), "supported") {
		t.Errorf("err = %v, want the supported agents listed", r.err)
	}
}

// The help text writes the agent names out rather than reading the registry.
func TestSkillHelpNamesEveryAgent(t *testing.T) {
	var cli CLI
	parser, err := newParser(&cli, nil)
	if err != nil {
		t.Fatal(err)
	}
	var checked int
	for _, cmd := range parser.Model.Children {
		if cmd.Name != "skill" {
			continue
		}
		for _, sub := range cmd.Children {
			for _, arg := range sub.Positional {
				checked++
				for _, a := range skill.Agents() {
					if !strings.Contains(arg.Help, a.Name()) {
						t.Errorf("skill %s <%s> help %q does not name %s", sub.Name, arg.Name, arg.Help, a.Name())
					}
				}
			}
		}
	}
	if checked != 4 {
		t.Errorf("checked %d agent arguments, want 4 (install, uninstall, show, check)", checked)
	}
}
