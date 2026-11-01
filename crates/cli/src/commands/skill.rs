//! `cases skill`: installs, shows and lists the agent skill this binary
//! carries (SK-01, SK-02).

use std::env;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use clap::builder::PossibleValuesParser;

use crate::ctx::Ctx;
use crate::error::Error;

/// The skill's name, in its frontmatter and as its directory.
const NAME: &str = "cases";

/// When an agent should load the skill, in its frontmatter.
const DESCRIPTION: &str = "Use when the agent needs a human decision, approval, sign-off or \
answer before it can go on, is stuck, has something the human should know, or has finished a \
piece of work to brief the human on, when a `cases wait` line brings a message from the human, or when the user mentions cases or the cases inbox. \
Provides the `cases` CLI for opening a case, waiting for the answer, picking it up and closing it \
with the outcome, and for taking the messages the human sends.";

/// The skill's text, without its frontmatter.
const BODY: &str = include_str!("../../skill/SKILL.md");

/// The reference: what an agent needs only now and then, in parts, each
/// headed `## TOPIC — …`, which `skill show --reference` prints.
const REFERENCE: &str = include_str!("../../skill/REFERENCE.md");

/// The file the skill is installed as.
const FILE: &str = "SKILL.md";

/// An agent the skill can be installed for.
struct Agent {
    /// Its name, as the commands take it.
    name: &'static str,
    /// The variable that moves the agent's config directory.
    var: &'static str,
    /// Whether the agent expands a leading `~` in `var` itself. Installing
    /// must do as the agent does, or the skill lands where it never looks.
    expands_tilde: bool,
    /// The config directory under `$HOME` when `var` is not set.
    under_home: &'static [&'static str],
}

/// The agents, by name. Claude Code and Codex use their variable as it is;
/// Pi expands a leading `~` in its.
const AGENTS: [Agent; 3] = [
    Agent {
        name: "claude",
        var: "CLAUDE_CONFIG_DIR",
        expands_tilde: false,
        under_home: &[".claude"],
    },
    Agent {
        name: "codex",
        var: "CODEX_HOME",
        expands_tilde: false,
        under_home: &[".codex"],
    },
    Agent {
        name: "pi",
        var: "PI_CODING_AGENT_DIR",
        expands_tilde: true,
        under_home: &[".pi", "agent"],
    },
];

/// The command's arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Command,
}

/// The subcommands. Each agent argument offers every agent.
#[derive(Debug, Subcommand)]
enum Command {
    /// Install the skill for an AI coding agent.
    Install {
        /// The agent.
        #[arg(value_parser = agents())]
        agent: String,
        /// Install into this directory instead of the agent's own.
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
        /// Overwrite an installed skill that differs without asking.
        #[arg(short, long)]
        yes: bool,
    },
    /// Remove the skill from an AI coding agent.
    Uninstall {
        /// The agent.
        #[arg(value_parser = agents())]
        agent: String,
        /// Remove from this directory instead of the agent's own.
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
        /// Remove without asking.
        #[arg(short, long)]
        yes: bool,
    },
    /// Print the skill's SKILL.md, the files installed for an agent, or
    /// the skill's reference.
    Show {
        /// Print the files installed for this agent.
        #[arg(value_parser = agents(), conflicts_with = "reference")]
        agent: Option<String>,
        /// Print the skill's reference instead: all of it, or its part on
        /// TOPIC, such as amend.
        #[arg(
            long,
            value_name = "TOPIC",
            num_args = 0..=1,
            default_missing_value = ""
        )]
        reference: Option<String>,
    },
    /// List the agents, where the skill goes for each, and whether it is
    /// installed, stale or not installed.
    List,
    /// Exit 1 when an installed skill differs from this binary's or cannot
    /// be read. One that is not installed does not count.
    Check {
        /// Check only this agent.
        #[arg(value_parser = agents())]
        agent: Option<String>,
    },
}

/// The agents' names, for clap to offer and check.
fn agents() -> PossibleValuesParser {
    PossibleValuesParser::new(AGENTS.map(|agent| agent.name))
}

/// How the skill installed in a directory compares with this binary's.
#[derive(Debug, PartialEq, Eq)]
enum Status {
    /// Nothing is installed.
    NotInstalled,
    /// The same skill is installed, byte for byte.
    Installed,
    /// Another skill is installed, as after an upgrade of cases.
    Stale,
}

impl Status {
    fn as_str(&self) -> &'static str {
        match self {
            Status::NotInstalled => "not installed",
            Status::Installed => "installed",
            Status::Stale => "stale",
        }
    }
}

/// The skill as installed: frontmatter that names it, then its text.
pub(crate) fn skill_md() -> String {
    format!("---\nname: {NAME}\ndescription: {DESCRIPTION}\n---\n\n{BODY}")
}

/// Runs the command.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let out = &mut ctx.out;
    match &args.command {
        Command::Install { agent, path, yes } => install(out, agent, path.as_deref(), *yes),
        Command::Uninstall { agent, path, yes } => uninstall(out, agent, path.as_deref(), *yes),
        Command::Show {
            reference: Some(topic),
            ..
        } => {
            write!(out, "{}", reference(topic)?)?;
            Ok(())
        }
        Command::Show { agent, .. } => {
            match agent {
                None => write!(out, "{}", skill_md())?,
                Some(_) => write!(out, "# {FILE}\n{}", skill_md())?,
            }
            Ok(())
        }
        Command::List => list(out),
        Command::Check { agent } => check(out, agent.as_deref()),
    }
}

/// The reference, or its part on `topic` when it names one: the part whose
/// heading's first word it is, from that heading to the next.
fn reference(topic: &str) -> Result<&'static str, Error> {
    if topic.is_empty() {
        return Ok(REFERENCE);
    }
    let parts = parts();
    parts
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(topic))
        .map(|(_, part)| *part)
        .ok_or_else(|| {
            let names: Vec<&str> = parts.iter().map(|(name, _)| *name).collect();
            Error::new(format!(
                "the reference has no topic {topic:?}; its topics are {}",
                names.join(", ")
            ))
        })
}

/// The reference's parts, each as its topic and its text.
fn parts() -> Vec<(&'static str, &'static str)> {
    let starts: Vec<usize> = REFERENCE
        .match_indices("\n## ")
        .map(|(at, _)| at + 1)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let end = starts.get(i + 1).copied().unwrap_or(REFERENCE.len());
            let part = &REFERENCE[start..end];
            let name = part[3..].split_whitespace().next().unwrap_or_default();
            (name, part)
        })
        .collect()
}

/// Installs the skill, asking before it replaces a different one.
fn install(out: &mut impl Write, name: &str, path: Option<&Path>, yes: bool) -> Result<(), Error> {
    let dir = dir(name, path)?;
    let status = match status(&dir) {
        Ok(status) => status,
        Err(_) if yes => Status::Stale,
        Err(error) => {
            return Err(Error::new(format!(
                "cannot read the skill at {}: {error}; pass -y to overwrite it",
                dir.display()
            )));
        }
    };
    match status {
        Status::Installed => {
            writeln!(
                out,
                "{name} skill at {} is already up to date",
                dir.display()
            )?;
            return Ok(());
        }
        Status::Stale if !yes => {
            if !io::stdin().is_terminal() {
                return Err(Error::new(format!(
                    "a different skill is installed at {}; pass -y to overwrite",
                    dir.display()
                )));
            }
            if !confirm(&format!(
                "A different skill is installed at {}. Overwrite?",
                dir.display()
            ))? {
                return Err(Error::new("cancelled"));
            }
        }
        Status::Stale | Status::NotInstalled => {}
    }
    fs::create_dir_all(&dir).map_err(|error| in_dir(&dir, &error))?;
    fs::write(dir.join(FILE), skill_md()).map_err(|error| in_dir(&dir, &error))?;
    writeln!(out, "Installed {name} skill to {}", dir.display())?;
    Ok(())
}

/// Removes the skill's files, after listing them and asking, and the
/// directory when nothing else is in it.
fn uninstall(
    out: &mut impl Write,
    name: &str,
    path: Option<&Path>,
    yes: bool,
) -> Result<(), Error> {
    let dir = dir(name, path)?;
    let file = dir.join(FILE);
    if fs::symlink_metadata(&file).is_err() {
        return Err(Error::new(format!(
            "no {name} skill installed at {}",
            dir.display()
        )));
    }
    eprintln!("Will remove 1 file from {}:\n  - {FILE}", dir.display());
    if !yes {
        if !io::stdin().is_terminal() {
            return Err(Error::new(
                "refusing to uninstall without a terminal to ask on; pass -y to confirm",
            ));
        }
        if !confirm(&format!("Remove {name} skill at {}?", dir.display()))? {
            return Err(Error::new("cancelled"));
        }
    }
    fs::remove_file(&file).map_err(|error| in_dir(&dir, &error))?;
    // Only an empty directory is removed; one with other files stays.
    let _ = fs::remove_dir(&dir);
    writeln!(out, "Removed {name} skill from {}", dir.display())?;
    Ok(())
}

/// Prints each agent, its skill directory and the skill's status there.
fn list(out: &mut impl Write) -> Result<(), Error> {
    for agent in &AGENTS {
        let name = agent.name;
        match agent.dir() {
            Err(error) => writeln!(out, "{name:<10} (path unresolved: {error})")?,
            Ok(dir) => match status(&dir) {
                Ok(status) => writeln!(out, "{name:<10} {}  ({})", dir.display(), status.as_str())?,
                Err(error) => writeln!(out, "{name:<10} {}  (unreadable: {error})", dir.display())?,
            },
        }
    }
    let names: Vec<&str> = AGENTS.iter().map(|agent| agent.name).collect();
    writeln!(
        out,
        "\nUse `cases skill install <agent>` (agents: {})",
        names.join(", ")
    )?;
    Ok(())
}

/// Prints each stale skill, and fails when one is stale or cannot be read.
fn check(out: &mut impl Write, only: Option<&str>) -> Result<(), Error> {
    let mut failed = false;
    for agent in AGENTS
        .iter()
        .filter(|agent| only.is_none_or(|name| name == agent.name))
    {
        let name = agent.name;
        let dir = match agent.dir() {
            Ok(dir) => dir,
            Err(error) if only.is_some() => return Err(error),
            // Nothing can be installed where it cannot be found, as `list`
            // says too.
            Err(error) => {
                eprintln!("{name}: path unresolved: {error}");
                continue;
            }
        };
        match status(&dir) {
            Ok(Status::Stale) => {
                failed = true;
                writeln!(
                    out,
                    "{name} skill at {} differs from this binary; run `cases skill install {name}`",
                    dir.display()
                )?;
            }
            Ok(_) => {}
            Err(error) => {
                failed = true;
                eprintln!(
                    "{name}: cannot read the skill at {}: {error}",
                    dir.display()
                );
            }
        }
    }
    if failed {
        return Err(Error::new(""));
    }
    Ok(())
}

/// The skill directory to act on: `path`, else the agent's own.
fn dir(name: &str, path: Option<&Path>) -> Result<PathBuf, Error> {
    if let Some(path) = path {
        return Ok(path.to_owned());
    }
    AGENTS
        .iter()
        .find(|agent| agent.name == name)
        .ok_or_else(|| Error::new(format!("unknown agent {name:?}")))?
        .dir()
}

impl Agent {
    /// The agent's skill directory: `skills/cases` in its config directory,
    /// which `$var` names, else the one under `$HOME`.
    fn dir(&self) -> Result<PathBuf, Error> {
        let home = env::var_os("HOME").filter(|home| !home.is_empty());
        let Some(base) = env::var_os(self.var).filter(|base| !base.is_empty()) else {
            let mut base = PathBuf::from(home.ok_or_else(|| {
                Error::new(format!(
                    "cannot find the skill directory: neither ${} nor $HOME is set",
                    self.var
                ))
            })?);
            base.extend(self.under_home);
            return Ok(base.join("skills").join(NAME));
        };
        let base = match (&home, base.to_str()) {
            (Some(home), Some("~")) if self.expands_tilde => PathBuf::from(home),
            (Some(home), Some(base)) if self.expands_tilde && base.starts_with("~/") => {
                Path::new(home).join(&base[2..])
            }
            _ => PathBuf::from(base),
        };
        Ok(base.join("skills").join(NAME))
    }
}

/// How the skill in `dir` compares with this binary's. Only a file that is
/// not there counts as absent: one that cannot be read is an error, since
/// what it holds is not known.
fn status(dir: &Path) -> io::Result<Status> {
    match fs::read(dir.join(FILE)) {
        Ok(text) if text == skill_md().as_bytes() => Ok(Status::Installed),
        Ok(_) => Ok(Status::Stale),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Status::NotInstalled),
        Err(error) => Err(error),
    }
}

/// Asks `question` on stderr, and reads yes or no from stdin. Anything but
/// `y` or `yes` is no.
fn confirm(question: &str) -> Result<bool, Error> {
    eprint!("{question} [y/N]: ");
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// An error writing in `dir`, naming it.
fn in_dir(dir: &Path, error: &io::Error) -> Error {
    Error::new(format!("{}: {error}", dir.display()))
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;
    use crate::cli::Cli;

    #[test]
    fn skill_md_starts_with_frontmatter_that_names_it() {
        let text = skill_md();
        assert!(text.starts_with("---\nname: cases\ndescription: Use when "));
        let end = text[4..].find("\n---\n").unwrap();
        assert!(!text[..end].contains("\n\n"));
        assert!(text.ends_with(BODY));
        assert!(!BODY.starts_with("---"));
    }

    /// The text inside code: `inline` spans, and the lines of fenced
    /// blocks, with a line that ends in `\` joined to the next, as a shell
    /// joins them.
    fn code(text: &str) -> Vec<String> {
        let mut code = Vec::new();
        let mut fenced = false;
        let mut joined = String::new();
        for line in text.lines() {
            if line.starts_with("```") {
                assert!(
                    joined.is_empty(),
                    "a code block ends in a line that ends in `\\`: {joined}"
                );
                fenced = !fenced;
            } else if fenced {
                if let Some(start) = line.strip_suffix('\\') {
                    joined.push_str(start);
                } else {
                    joined.push_str(line);
                    code.push(std::mem::take(&mut joined));
                }
            } else {
                code.extend(line.split('`').skip(1).step_by(2).map(str::to_owned));
            }
        }
        assert!(!fenced, "a code block is not closed");
        code
    }

    /// The long flags in `words`, the words of one command, up to the end of
    /// the command: `--kind` from `--kind KIND` or `[--json`, and `--since`
    /// from `--since=TIME`.
    fn flags<'a>(words: &[&'a str]) -> Vec<&'a str> {
        words
            .iter()
            .take_while(|word| !["&&", "||", "|", ";"].contains(word))
            .filter_map(|word| {
                let word = word.trim_start_matches(['[', '(']);
                let flag = word.strip_prefix("--")?;
                let flag = flag.split(['=', ']', ')', '.', ',', '|']).next()?;
                (!flag.is_empty()).then_some(flag)
            })
            .collect()
    }

    #[test]
    fn every_command_the_skill_names_exists() {
        let body = names(BODY);
        let reference = names(REFERENCE);
        assert!(
            body.len() > 20,
            "found only {} commands in the skill",
            body.len()
        );
        assert!(
            reference.len() > 15,
            "found only {} in the reference",
            reference.len()
        );
        // The ones an agent runs are in the skill, and amend, which it runs
        // now and then, in the reference.
        for name in [
            "open", "wait", "pickup", "note", "close", "withdraw", "show", "list", "status",
        ] {
            assert!(
                body.iter().any(|named| named == name),
                "the skill does not name cases {name}"
            );
        }
        assert!(
            reference.iter().any(|named| named == "amend"),
            "the reference does not name cases amend"
        );
    }

    /// The commands `text` names in its code, each checked to exist with
    /// every flag it is given.
    fn names(text: &str) -> Vec<String> {
        let cli = Cli::command();
        let mut names = Vec::new();
        let mut flagged = 0;
        for code in code(text) {
            let mut rest = code.as_str();
            while let Some(at) = rest.find("cases ") {
                let before = rest[..at].chars().next_back();
                let words: Vec<&str> = rest[at + 6..].split_whitespace().take(2).collect();
                rest = &rest[at + 6..];
                if before.is_some_and(|c| c.is_alphanumeric() || "-_./".contains(c)) {
                    continue;
                }
                let Some(name) = words
                    .first()
                    .filter(|word| word.chars().all(|c| c.is_ascii_lowercase()))
                else {
                    continue;
                };
                let command = cli.find_subcommand(name).unwrap_or_else(|| {
                    panic!("the skill names `cases {name}`, which is no command: {code}")
                });
                names.push(command.get_name().to_owned());
                let sub = words
                    .get(1)
                    .filter(|word| word.chars().all(|c| c.is_ascii_lowercase()));
                let mut command = command;
                if let Some(sub) = sub
                    && command.has_subcommands()
                {
                    command = command.find_subcommand(sub).unwrap_or_else(|| {
                        panic!("the skill names `cases {name} {sub}`, which is no command: {code}")
                    });
                }
                // The flags up to the next `cases`, each one the command,
                // or every command, takes.
                let end = rest.find("cases ").unwrap_or(rest.len());
                let words: Vec<&str> = rest[..end].split_whitespace().collect();
                for flag in flags(&words) {
                    let known = command
                        .get_arguments()
                        .chain(cli.get_arguments())
                        .any(|arg| arg.get_long() == Some(flag));
                    assert!(
                        known || flag == "help",
                        "the skill gives `cases {name}` the flag --{flag}, which it does not \
                         take: {code}"
                    );
                    flagged += 1;
                }
            }
        }
        assert!(flagged > 30, "found only {flagged} flags");
        names
    }

    #[test]
    fn the_reference_is_in_parts_by_topic() {
        let parts = parts();
        let topics: Vec<&str> = parts.iter().map(|(topic, _)| *topic).collect();
        assert_eq!(
            topics,
            [
                "kinds", "open", "amend", "briefing", "wait", "show", "list", "status", "doing",
                "drawing", "store", "mod"
            ]
        );
        // Its first lines name every topic, in order.
        let intro = &REFERENCE[..REFERENCE.find("\n## ").unwrap()];
        let named: Vec<&str> = intro.split('`').skip(1).step_by(2).skip(1).collect();
        assert_eq!(named, topics);
        for (topic, part) in &parts {
            assert!(part.starts_with(&format!("## {topic} — ")), "{part}");
            assert_eq!(reference(topic).unwrap(), *part);
            assert_eq!(reference(&topic.to_uppercase()).unwrap(), *part);
            assert_eq!(part.matches("\n## ").count(), 0, "{topic}");
        }
        assert_eq!(reference("").unwrap(), REFERENCE);
        let error = reference("nope").unwrap_err().to_string();
        assert!(error.contains("kinds, open, amend"), "{error}");
    }

    #[test]
    fn each_topic_the_skill_sends_an_agent_to_is_in_the_reference() {
        let mut sent = 0;
        for (at, _) in BODY.match_indices("--reference ") {
            let topic: String = BODY[at + 12..]
                .chars()
                .take_while(char::is_ascii_lowercase)
                .collect();
            if topic.is_empty() {
                continue;
            }
            assert!(
                reference(&topic).is_ok(),
                "the skill names the topic {topic}"
            );
            sent += 1;
        }
        assert!(sent >= 5, "the skill sends an agent to {sent} topics");
    }

    #[test]
    fn each_agent_argument_offers_every_agent() {
        let cli = Cli::command();
        let skill = cli.find_subcommand("skill").unwrap();
        let mut checked = 0;
        for sub in skill.get_subcommands() {
            for arg in sub.get_positionals() {
                let values: Vec<String> = arg
                    .get_possible_values()
                    .iter()
                    .map(|value| value.get_name().to_owned())
                    .collect();
                assert_eq!(
                    values,
                    ["claude", "codex", "pi"],
                    "skill {}",
                    sub.get_name()
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 4, "install, uninstall, show and check");
    }

    #[test]
    fn the_agents_are_in_order_of_name() {
        assert!(AGENTS.is_sorted_by_key(|agent| agent.name));
    }

    #[test]
    fn a_skill_is_installed_stale_or_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(status(dir.path()).unwrap(), Status::NotInstalled);
        fs::write(dir.path().join(FILE), skill_md()).unwrap();
        assert_eq!(status(dir.path()).unwrap(), Status::Installed);
        fs::write(dir.path().join(FILE), format!("{}\n", skill_md())).unwrap();
        assert_eq!(status(dir.path()).unwrap(), Status::Stale);
        fs::remove_file(dir.path().join(FILE)).unwrap();
        fs::create_dir(dir.path().join(FILE)).unwrap();
        assert!(status(dir.path()).is_err());
    }
}
