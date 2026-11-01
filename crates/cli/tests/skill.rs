//! `cases skill`: the agent skill this binary carries, installed for
//! Claude Code, Codex or Pi (SK-01). Every agent's directory is in the
//! test's own home.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Env, write_file};

/// The skill as `skill show` prints it, which is what is installed.
fn skill_md(env: &Env) -> String {
    let out = env.run(["skill", "show"]);
    assert_eq!(out.code, 0, "{out:?}");
    out.stdout
}

/// `dir/skills/cases`, where an agent whose config directory is `dir`
/// finds the skill.
fn skills(dir: &Path) -> PathBuf {
    dir.join("skills").join("cases")
}

#[test]
fn skill_show_prints_the_skill_with_its_frontmatter() {
    let env = Env::new();
    let text = skill_md(&env);
    assert!(
        text.starts_with("---\nname: cases\ndescription: "),
        "{text}"
    );
    assert!(text.contains("cases wait"));
    for agent in ["claude", "codex", "pi"] {
        let out = env.run(["skill", "show", agent]);
        assert_eq!(out.stdout, format!("# SKILL.md\n{text}"), "{agent}");
    }
}

/// The reference holds what an agent needs only now and then; it is printed,
/// never installed, so an installed skill is the one file (case sHP8j).
#[test]
fn skill_show_prints_the_reference_or_one_part_of_it() {
    let env = Env::new();
    let all = env.run(["skill", "show", "--reference"]);
    assert_eq!(all.code, 0, "{all:?}");
    assert!(
        all.stdout.starts_with("# cases — reference\n"),
        "{}",
        all.stdout
    );
    let amend = env.run(["skill", "show", "--reference", "amend"]);
    assert_eq!(amend.code, 0, "{amend:?}");
    assert!(amend.stdout.starts_with("## amend — "), "{}", amend.stdout);
    assert!(amend.stdout.contains("cases amend ID"));
    assert!(!amend.stdout.contains("\n## "), "{}", amend.stdout);
    assert!(all.stdout.contains(&amend.stdout));
    let out = env.run(["skill", "show", "--reference", "nope"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("kinds, open, amend"), "{out:?}");
    let out = env.run(["skill", "show", "claude", "--reference"]);
    assert_ne!(out.code, 0, "{out:?}");
}

#[test]
fn skill_install_list_and_uninstall() {
    let mut env = Env::new();
    let claude = env.path("claude");
    env.set("CLAUDE_CONFIG_DIR", claude.to_str().unwrap());
    let skill = skill_md(&env);
    let dir = env.path("elsewhere").join("cases");
    let dir_arg = dir.to_str().unwrap();

    let out = env.run(["skill", "install", "claude", "--path", dir_arg]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("Installed claude skill to {dir_arg}\n"));
    assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), skill);

    // The same skill again is a no-op, and needs no -y.
    let out = env.run(["skill", "install", "claude", "--path", dir_arg]);
    assert_eq!(
        out.stdout,
        format!("claude skill at {dir_arg} is already up to date\n")
    );

    // A different one is replaced only with -y: stdin is not a terminal to
    // ask on.
    write_file(&dir.join("SKILL.md"), "old\n");
    let out = env.run(["skill", "install", "claude", "--path", dir_arg]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("-y"), "{out:?}");
    assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), "old\n");
    for flag in ["-y", "--yes"] {
        write_file(&dir.join("SKILL.md"), "old\n");
        let out = env.run(["skill", "install", "claude", "--path", dir_arg, flag]);
        assert_eq!(out.code, 0, "{out:?}");
        assert_eq!(fs::read_to_string(dir.join("SKILL.md")).unwrap(), skill);
    }

    // The agent's own directory.
    let own = skills(&claude);
    let row = |status: &str| format!("claude     {}  ({status})\n", own.display());
    assert!(
        env.run(["skill", "list"])
            .stdout
            .contains(&row("not installed"))
    );
    assert_eq!(env.run(["skill", "install", "claude"]).code, 0);
    assert!(
        env.run(["skill", "list"])
            .stdout
            .contains(&row("installed"))
    );
    write_file(&own.join("SKILL.md"), "old\n");
    let list = env.run(["skill", "list"]).stdout;
    assert!(list.contains(&row("stale")), "{list}");
    assert!(list.ends_with("(agents: claude, codex, pi)\n"), "{list}");
    assert_eq!(env.run(["skill", "install", "claude", "-y"]).code, 0);

    // Uninstall lists what it removes, and needs -y without a terminal.
    let out = env.run(["skill", "uninstall", "claude"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("  - SKILL.md\n"), "{out:?}");
    assert!(out.stderr.contains("-y"), "{out:?}");
    assert!(own.join("SKILL.md").exists());
    write_file(&own.join("notes.md"), "mine\n");
    let out = env.run(["skill", "uninstall", "claude", "-y"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!("Removed claude skill from {}\n", own.display())
    );
    // A file that is not the skill's stays, and so does its directory.
    assert!(!own.join("SKILL.md").exists());
    assert_eq!(fs::read_to_string(own.join("notes.md")).unwrap(), "mine\n");
    fs::remove_file(own.join("notes.md")).unwrap();

    // Nothing to remove is an error; an empty directory is removed.
    let out = env.run(["skill", "uninstall", "claude", "-y"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("no claude skill installed"), "{out:?}");
    assert_eq!(env.run(["skill", "install", "claude"]).code, 0);
    assert_eq!(env.run(["skill", "uninstall", "claude", "-y"]).code, 0);
    assert!(!own.exists());
}

#[test]
fn each_agent_has_its_own_directory() {
    let mut env = Env::new();
    let home = env.path("home");
    for (agent, under) in [
        ("claude", ".claude"),
        ("codex", ".codex"),
        ("pi", ".pi/agent"),
    ] {
        assert_eq!(env.run(["skill", "install", agent]).code, 0);
        assert!(
            skills(&home.join(under)).join("SKILL.md").exists(),
            "{agent}"
        );
    }

    // Each moves with its variable. Pi expands a leading ~ in its own, as
    // Pi does; Claude Code and Codex take theirs as written.
    for (agent, var) in [
        ("claude", "CLAUDE_CONFIG_DIR"),
        ("codex", "CODEX_HOME"),
        ("pi", "PI_CODING_AGENT_DIR"),
    ] {
        env.set(var, "~/moved");
        assert_eq!(env.run(["skill", "install", agent]).code, 0);
        let want = if agent == "pi" {
            skills(&home.join("moved"))
        } else {
            // Relative to where cases ran: the test's directory.
            skills(&env.path("~/moved"))
        };
        assert!(
            want.join("SKILL.md").exists(),
            "{agent}: {}",
            want.display()
        );
        env.unset(var);
    }
}

#[test]
fn skill_check_fails_only_on_a_stale_or_unreadable_skill() {
    let mut env = Env::new();
    for var in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "PI_CODING_AGENT_DIR"] {
        let dir = env.path(var);
        env.set(var, dir.to_str().unwrap());
    }
    let claude = skills(&env.path("CLAUDE_CONFIG_DIR"));

    // Nothing installed is not a failure.
    let out = env.run(["skill", "check"]);
    assert_eq!((out.code, out.stdout.as_str()), (0, ""), "{out:?}");
    assert_eq!(env.run(["skill", "install", "claude"]).code, 0);
    let out = env.run(["skill", "check"]);
    assert_eq!((out.code, out.stdout.as_str()), (0, ""), "{out:?}");

    write_file(&claude.join("SKILL.md"), "old\n");
    for args in [&["skill", "check"][..], &["skill", "check", "claude"]] {
        let out = env.run(args);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        assert_eq!(
            out.stdout,
            format!(
                "claude skill at {} differs from this binary; run `cases skill install claude`\n",
                claude.display()
            )
        );
    }
    // Checking another agent leaves claude's out.
    let out = env.run(["skill", "check", "codex"]);
    assert_eq!((out.code, out.stdout.as_str()), (0, ""), "{out:?}");

    // An agent whose directory cannot be found does not stop the others
    // being checked, unless it is the one named.
    env.unset("HOME");
    env.unset("CODEX_HOME");
    let out = env.run(["skill", "check"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("codex: path unresolved: "), "{out:?}");
    let out = env.run(["skill", "check", "codex"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.starts_with("Error: cannot find"), "{out:?}");
    assert!(
        env.run(["skill", "list"])
            .stdout
            .contains("codex      (path unresolved: ")
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_skill_is_reported_and_replaced_only_with_yes() {
    use std::os::unix::fs::PermissionsExt;
    let mut env = Env::new();
    let claude = env.path("claude");
    env.set("CLAUDE_CONFIG_DIR", claude.to_str().unwrap());
    let dir = skills(&claude);
    let file = dir.join("SKILL.md");
    write_file(&file, "mine\n");
    let mode = |mode| fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
    mode(0o200);
    if fs::read(&file).is_ok() {
        // Running as root: the mode does not stop the read.
        return;
    }

    let out = env.run(["skill", "install", "claude"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("cannot read"), "{out:?}");
    assert!(out.stderr.contains("-y"), "{out:?}");

    let list = env.run(["skill", "list"]).stdout;
    assert!(
        list.contains(&format!("{}  (unreadable: ", dir.display())),
        "{list}"
    );
    let out = env.run(["skill", "check", "claude"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(
        out.stderr.contains(&format!(
            "claude: cannot read the skill at {}",
            dir.display()
        )),
        "{out:?}"
    );
    mode(0o600);
    assert_eq!(fs::read_to_string(&file).unwrap(), "mine\n");
    mode(0o200);

    let out = env.run(["skill", "install", "claude", "-y"]);
    assert_eq!(out.code, 0, "{out:?}");
    mode(0o600);
    assert_eq!(fs::read_to_string(&file).unwrap(), skill_md(&env));
}

#[test]
fn an_unknown_agent_is_refused_and_every_agent_is_named() {
    let env = Env::new();
    let dir = env.path("dir");
    for args in [
        &["skill", "install", "bogus", "--path", dir.to_str().unwrap()][..],
        &["skill", "check", "bogus"],
        &["skill", "show", "bogus"],
    ] {
        let out = env.run(args);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        for agent in ["claude", "codex", "pi"] {
            assert!(out.stderr.contains(agent), "{args:?}: {out:?}");
        }
    }
    assert!(!dir.exists());
    let help = env.run(["skill", "install", "--help"]).stdout;
    assert!(help.contains("claude, codex, pi"), "{help}");
}
