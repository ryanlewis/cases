# skill

The agent skill is a `SKILL.md` embedded in the binary, so it always matches
the commands the binary has. It is written for an agent: the lifecycle, the
kinds and what each answer looks like, the agent-side commands, what not to do,
and a recipe for open, wait, pickup and close.

```sh
cases skill install claude   # write SKILL.md into the agent's skills directory
cases skill uninstall claude # remove it (and the directory, if empty)
cases skill show             # print the skill
cases skill list             # each agent, where the skill goes, and whether it is installed
cases skill check            # exit 1 if an installed skill differs from this binary
```

| Agent | Directory | Relocated by |
| --- | --- | --- |
| `claude` | `~/.claude/skills/cases` | `$CLAUDE_CONFIG_DIR` |
| `codex` | `~/.codex/skills/cases` | `$CODEX_HOME` |
| `pi` | `~/.pi/agent/skills/cases` | `$PI_CODING_AGENT_DIR` (a leading `~` is expanded) |

`skill list` compares the files in each agent's directory byte for byte with
what this binary renders and reports one of three states: `installed` (they
match), `stale` (a file is there but differs or is missing, as after
upgrading cases without reinstalling the skill) or `not installed`. A file
that is there but cannot be read shows as `unreadable`, with the error.

`--path DIR` installs to or uninstalls from another directory. When the
installed skill already matches, `install` says it is already up to date and
writes nothing. It overwrites a stale skill only after asking, or with `-y`;
`uninstall` lists the files and asks, or needs `-y`. When stdin is not a
terminal neither command asks: without `-y` they refuse. `install` also
refuses without `-y` when a file of the installed skill cannot be read.

`skill check [agent]` looks at each agent's default directory, or only the
named agent's, prints a line for each stale skill and exits 1 if there is
one, or if a skill cannot be read. A skill that is not installed does not
count. It is for scripts and Makefiles; it reports drift but does not fix
it.
