//! `cases config`: where the config file is, what it sets, and a template
//! to start one (SV-01).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::{Env, write_file};

#[test]
fn config_path_names_the_file_and_whether_it_is_there() {
    let mut env = Env::new();
    let default = env.path("config").join("cases").join("config.toml");
    let out = env.run(["config", "path"]);
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (0, format!("{} (not found)\n", default.display()).as_str()),
        "{out:?}"
    );

    env.config("");
    assert_eq!(
        env.run(["config", "path"]).stdout,
        format!("{} (exists)\n", default.display())
    );

    // --config and CASES_CONFIG name it instead, there or not.
    let mine = env.path("mine.toml");
    write_file(&mine, "");
    let mine = mine.to_str().unwrap();
    assert_eq!(
        env.run(["--config", mine, "config", "path"]).stdout,
        format!("{mine} (exists)\n")
    );
    env.set("CASES_CONFIG", mine);
    assert_eq!(
        env.run(["config", "path"]).stdout,
        format!("{mine} (exists)\n")
    );
    let none = env.path("none.toml");
    let out = env.run(["--config", none.to_str().unwrap(), "config", "path"]);
    assert_eq!(out.stdout, format!("{} (not found)\n", none.display()));
    assert_eq!(out.code, 0);
}

#[test]
fn a_config_file_that_cannot_be_used_is_still_reported() {
    let env = Env::new();
    let path = env.config("stor = \"x\"\n");

    let out = env.run(["config", "path"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{} (exists)\n", path.display()));
    assert!(
        out.stderr
            .starts_with("warning: this file cannot be used: config file "),
        "{out:?}"
    );
    assert!(out.stderr.contains("unknown key \"stor\""), "{out:?}");

    // show prints the problem, and the file as it is (SV-01).
    let out = env.run(["config", "show"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.stdout,
        format!("config: {} (exists)\n\nstor = \"x\"\n", path.display())
    );
    assert!(out.stderr.contains("unknown key \"stor\""), "{out:?}");

    // init refuses to replace it without --force, and replaces it with it.
    let out = env.run(["config", "init"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("already exists"), "{out:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "stor = \"x\"\n");
    let out = env.run(["config", "init", "-f"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("# cases configuration\n")
    );
    assert_eq!(env.run(["config", "show"]).code, 0);

    // A directory at the path cannot be read, and is not replaced either.
    let env = Env::new();
    let path = env.path("config").join("cases").join("config.toml");
    std::fs::create_dir_all(&path).unwrap();
    assert!(env.run(["config", "path"]).stdout.ends_with("(exists)\n"));
    let out = env.run(["config", "init"]);
    assert!(out.stderr.contains("already exists"), "{out:?}");
    assert!(path.is_dir());
}

#[test]
fn config_show_prints_each_default_and_where_it_came_from() {
    let mut env = Env::new();
    let store = env.path("mine.db");
    let path = env.config(&format!(
        "store = {:?}\nname = \"Ryan\"\n",
        store.to_str().unwrap()
    ));
    let out = env.run(["config", "show"]);
    assert_eq!(out.code, 0, "{out:?}");
    let store = store.display().to_string();
    let width = store.len();
    let line =
        |key: &str, value: &str, source: &str| format!("  {key:20}  {value:width$}  {source}\n");
    assert_eq!(
        out.stdout,
        format!(
            "config: {} (exists)\nThese apply when no flag overrides them.\n\n{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}{}",
            path.display(),
            line("store", &store, "config"),
            line("forward", "", "default"),
            line("forward-token-file", "", "default"),
            line("server", "", "default"),
            line("default-store", "", "default"),
            line("token-file", "", "default"),
            line("listen", "127.0.0.1:7337", "default"),
            line("no-open", "false", "default"),
            line("allow-hosts", "", "default"),
            line("tailscale-logins", "", "default"),
            line("store-tokens-file", "", "default"),
            line("name", "Ryan", "config"),
            line("prune-age", "720h", "default"),
            line("briefing-close-after", "7d", "default"),
            line("mood", "eager", "default"),
            line("tui-stats", "auto", "default"),
        )
    );

    // The environment beats the file, and a leading ~ is expanded.
    env.set("CASES_STORE", "~/env.db");
    let out = env.run(["config", "show"]).stdout;
    let home = env.path("home").join("env.db");
    assert!(
        out.contains(&format!("  store                 {}", home.display())),
        "{out}"
    );
    assert!(out.lines().nth(3).unwrap().ends_with("  env"), "{out}");
}

#[test]
fn config_show_gives_the_store_under_the_data_home_by_default() {
    let mut env = Env::new();
    let out = env.run(["config", "show"]).stdout;
    let data = env.path("data").join("cases").join("cases.db");
    assert!(
        out.contains(&format!(
            "  store                 {}  default\n",
            data.display()
        )),
        "{out}"
    );

    env.unset("XDG_DATA_HOME");
    let out = env.run(["config", "show"]).stdout;
    let home = env.path("home").join(".local/share/cases/cases.db");
    assert!(out.contains(&home.display().to_string()), "{out}");

    // With nowhere to put a store, config still runs, and the store is
    // blank.
    env.unset("HOME");
    let out = env.run(["config", "show"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("  store                 "), "{out:?}");
}

#[test]
fn config_init_writes_a_template_and_refuses_to_overwrite_unless_forced() {
    let env = Env::new();
    let path = env.path("config").join("cases").join("config.toml");
    let out = env.run(["config", "init"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!("Wrote config template to {}\n", path.display())
    );
    let template = std::fs::read_to_string(&path).unwrap();
    assert!(
        template.starts_with("# cases configuration\n"),
        "{template}"
    );

    let out = env.run(["config", "init"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("--force"), "{out:?}");

    write_file(&path, "store = \"/x\"\n");
    for flag in ["--force", "-f"] {
        assert_eq!(env.run(["config", "init", flag]).code, 0);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), template);
        write_file(&path, "store = \"/x\"\n");
    }

    // The template sets nothing: every value is the default.
    env.run(["config", "init", "--force"]);
    let out = env.run(["config", "show"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout.matches("  default\n").count(), 16, "{out:?}");
}

#[test]
fn config_needs_a_subcommand() {
    let env = Env::new();
    let out = env.run(["config"]);
    assert_eq!(out.code, 1, "{out:?}");
}

#[test]
fn config_show_reads_the_store_as_the_commands_do() {
    let env = Env::new();
    let data = env.path("data").join("cases").join("cases.db");
    env.config("store = \"  \"\n");
    let out = env.run(["config", "show"]).stdout;
    assert!(
        out.contains(&format!(
            "  store                 {}  default\n",
            data.display()
        )),
        "{out}"
    );
    env.config("store = \" ~/x.db \"\n");
    let out = env.run(["config", "show"]).stdout;
    let home = env.path("home").join("x.db");
    assert!(
        out.contains(&format!(
            "  store                 {}  config\n",
            home.display()
        )),
        "{out}"
    );
}

/// Without `$XDG_CONFIG_HOME`, the config file is `~/.config/cases/config.toml`
/// (SV-01).
#[test]
fn the_config_file_falls_back_to_the_home_config_directory() {
    let mut env = Env::new();
    env.unset("XDG_CONFIG_HOME");
    let store = env.path("from-home-config.db");
    common::write_file(
        &env.path("home").join(".config/cases/config.toml"),
        &format!("store = {:?}\n", store.to_str().unwrap()),
    );
    let out = env.run([
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Home",
    ]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        store.exists(),
        "the case is not in the store the file names"
    );
}

/// A `tui-stats` that cases tui would refuse is warned of by config show,
/// which still shows the rest (TU-13).
#[test]
fn config_show_warns_of_a_tui_stats_that_is_not_one() {
    let env = Env::new();
    env.config("tui-stats = \"on\"\n");
    let out = env.run(["config", "show"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("  tui-stats "), "{out:?}");
    assert_eq!(
        out.stderr,
        "warning: the config file's tui-stats: \"on\" is not auto, text or off\n"
    );
    env.config("tui-stats = \"text\"\n");
    assert_eq!(env.run(["config", "show"]).stderr, "");
}
