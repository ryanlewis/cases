package main

import (
	"bufio"
	"errors"
	"fmt"
	"os"
	"strings"

	"github.com/ryanlewis/cases/internal/skill"
)

// SkillCmd manages the bundled agent skill. The agent names in the help text
// are written out; TestSkillHelpNamesEveryAgent keeps them in step with the
// registry.
type SkillCmd struct {
	Install   SkillInstallCmd   `cmd:"" help:"Install the bundled agent skill for an AI coding agent."`
	Uninstall SkillUninstallCmd `cmd:"" help:"Remove the bundled agent skill for an AI coding agent."`
	Show      SkillShowCmd      `cmd:"" help:"Print the skill's SKILL.md, or the files rendered for an agent."`
	List      SkillListCmd      `cmd:"" help:"List supported agents, where the skill goes and whether it is installed."`
}

type SkillInstallCmd struct {
	Agent string `arg:"" help:"Target agent: claude, codex or pi."`
	Path  string `help:"Install into this directory instead of the agent's default." placeholder:"DIR"`
	Yes   bool   `help:"Overwrite an installed skill without asking." short:"y"`
}

func (c *SkillInstallCmd) Run(d *Deps) error {
	agent, dir, err := resolveSkill(c.Agent, c.Path)
	if err != nil {
		return err
	}
	if skill.Exists(agent, dir) && !c.Yes {
		if !d.interactive() {
			return fmt.Errorf("skill already installed at %s; pass -y to overwrite", dir)
		}
		if !d.confirm(fmt.Sprintf("Skill already installed at %s. Overwrite?", dir)) {
			return errors.New("cancelled")
		}
	}
	if err := skill.Install(agent, dir); err != nil {
		return err
	}
	fmt.Fprintf(d.Stdout, "Installed %s skill to %s\n", agent.Name(), dir)
	return nil
}

type SkillUninstallCmd struct {
	Agent string `arg:"" help:"Target agent: claude, codex or pi."`
	Path  string `help:"Uninstall from this directory instead of the agent's default." placeholder:"DIR"`
	Yes   bool   `help:"Uninstall without asking." short:"y"`
}

func (c *SkillUninstallCmd) Run(d *Deps) error {
	agent, dir, err := resolveSkill(c.Agent, c.Path)
	if err != nil {
		return err
	}
	present := skill.InstalledFiles(agent, dir)
	if len(present) == 0 {
		return fmt.Errorf("no %s skill installed at %s", agent.Name(), dir)
	}
	fmt.Fprintf(d.Stderr, "Will remove %d file(s) from %s:\n", len(present), dir)
	for _, f := range present {
		fmt.Fprintf(d.Stderr, "  - %s\n", f)
	}
	if !c.Yes {
		if !d.interactive() {
			return errors.New("refusing to uninstall non-interactively; pass -y to confirm")
		}
		if !d.confirm(fmt.Sprintf("Remove %s skill at %s?", agent.Name(), dir)) {
			return errors.New("cancelled")
		}
	}
	if err := skill.Uninstall(agent, dir); err != nil {
		return err
	}
	fmt.Fprintf(d.Stdout, "Removed %s skill from %s\n", agent.Name(), dir)
	return nil
}

type SkillShowCmd struct {
	Agent string `arg:"" optional:"" help:"Render for this agent (claude, codex or pi); by default print SKILL.md with its frontmatter."`
}

func (c *SkillShowCmd) Run(d *Deps) error {
	if c.Agent == "" {
		fmt.Fprint(d.Stdout, skill.SkillMD())
		return nil
	}
	agent, err := skill.Lookup(c.Agent)
	if err != nil {
		return err
	}
	for fname, content := range agent.Files() {
		fmt.Fprintf(d.Stdout, "# %s\n%s", fname, content)
	}
	return nil
}

type SkillListCmd struct{}

func (c *SkillListCmd) Run(d *Deps) error {
	for _, a := range skill.Agents() {
		dir, err := a.DefaultDir()
		if err != nil {
			fmt.Fprintf(d.Stdout, "%-10s (path unresolved: %v)\n", a.Name(), err)
			continue
		}
		status := "not installed"
		if skill.Exists(a, dir) {
			status = "installed"
		}
		fmt.Fprintf(d.Stdout, "%-10s %s  (%s)\n", a.Name(), dir, status)
	}
	fmt.Fprintf(d.Stdout, "\nUse `cases skill install <agent>` (agents: %s)\n", skill.AgentNames())
	return nil
}

// resolveSkill looks up the agent and the directory to act on.
func resolveSkill(name, override string) (skill.Agent, string, error) {
	agent, err := skill.Lookup(name)
	if err != nil {
		return nil, "", err
	}
	if override != "" {
		return agent, override, nil
	}
	dir, err := agent.DefaultDir()
	return agent, dir, err
}

// interactive reports whether stdin is a terminal, so a prompt can be
// answered.
func (d *Deps) interactive() bool {
	f, ok := d.Stdin.(*os.File)
	if !ok {
		return false
	}
	fi, err := f.Stat()
	return err == nil && fi.Mode()&os.ModeCharDevice != 0
}

// confirm asks a yes/no question on stderr and reads the reply from stdin.
func (d *Deps) confirm(msg string) bool {
	fmt.Fprintf(d.Stderr, "%s [y/N]: ", msg)
	scanner := bufio.NewScanner(d.Stdin)
	if !scanner.Scan() {
		return false
	}
	answer := strings.ToLower(strings.TrimSpace(scanner.Text()))
	return answer == "y" || answer == "yes"
}
