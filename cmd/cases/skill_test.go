package main

import (
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

	// Stdin is not a terminal, so a second install needs -y.
	r := runCases(t, "", "skill", "install", "claude", "--path", dir)
	if r.err == nil || !strings.Contains(r.err.Error(), "-y") {
		t.Errorf("reinstall without -y: err = %v, want a refusal naming -y", r.err)
	}
	mustRun(t, "skill", "install", "claude", "--path", dir, "-y")

	if out := mustRun(t, "skill", "list"); !strings.Contains(out, "not installed") {
		t.Errorf("list = %q, want claude not installed at its default dir", out)
	}
	claudeDir := filepath.Join(os.Getenv("CLAUDE_CONFIG_DIR"), "skills", "cases")
	mustRun(t, "skill", "install", "claude")
	out = mustRun(t, "skill", "list")
	if !strings.Contains(out, claudeDir+"  (installed)") {
		t.Errorf("list = %q, want %s installed", out, claudeDir)
	}

	r = runCases(t, "", "skill", "uninstall", "claude")
	if r.err == nil || !strings.Contains(r.err.Error(), "-y") {
		t.Errorf("uninstall without -y: err = %v, want a refusal naming -y", r.err)
	}
	mustRun(t, "skill", "uninstall", "claude", "-y")
	if _, err := os.Stat(filepath.Join(claudeDir, "SKILL.md")); !os.IsNotExist(err) {
		t.Errorf("SKILL.md still there after uninstall: %v", err)
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
	if checked != 3 {
		t.Errorf("checked %d agent arguments, want 3 (install, uninstall, show)", checked)
	}
}
