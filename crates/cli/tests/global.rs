//! What every command shares: the store path and the config file (SV-01),
//! exit codes (CL-01) and the command tree.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test panics to fail; clippy exempts #[test] functions but not the helpers here"
)]

mod common;

use common::{Env, write_file};

/// Opens an fyi case with no `--store`, and gives the store it landed in.
fn open_without_store(env: &Env, args: &[&str]) -> (String, common::Output) {
    let mut all = args.to_vec();
    all.extend([
        "open",
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Where",
    ]);
    let out = env.run(&all);
    assert_eq!(out.code, 0, "{out:?}");
    (out.stdout.trim().to_owned(), out)
}

/// Whether the store at `path` has the case `id`.
fn has(path: &std::path::Path, id: &str) -> bool {
    cases_core::Store::rows(&cases_store_sqlite::Db::new(path), &common::key(id))
        .ok()
        .flatten()
        .is_some()
}

#[test]
fn the_store_defaults_to_the_data_home() {
    let mut env = Env::new();
    let want = env.path("data").join("cases").join("cases.db");
    // CASES_STORE set but blank counts as unset.
    for blank in ["", " "] {
        env.set("CASES_STORE", blank);
        let (id, _) = open_without_store(&env, &[]);
        assert!(has(&want, &id), "CASES_STORE={blank:?}");
    }
    assert!(env.run(["list"]).stdout.contains("Where"));

    // Without XDG_DATA_HOME, it is under ~/.local/share.
    env.unset("XDG_DATA_HOME");
    let (id, _) = open_without_store(&env, &[]);
    assert!(has(
        &env.path("home").join(".local/share/cases/cases.db"),
        &id
    ));

    // With neither, nothing names the store.
    env.unset("HOME");
    let out = env.run(["list"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("--store or CASES_STORE"), "{out:?}");
}

#[test]
fn the_store_comes_from_the_environment() {
    let mut env = Env::new();
    let store = env.path("env.db");
    env.set("CASES_STORE", store.to_str().unwrap());
    let (id, _) = open_without_store(&env, &[]);
    assert!(has(&store, &id));
    assert!(env.run(["list"]).stdout.contains(&id));
}

#[test]
fn the_config_file_names_the_store_below_the_flag_and_the_environment() {
    let mut env = Env::new();
    let from_file = env.path("file.db");
    env.config(&format!("store = {:?}\n", from_file.to_str().unwrap()));
    let (id, _) = open_without_store(&env, &[]);
    assert!(has(&from_file, &id));
    assert!(env.run(["list"]).stdout.contains(&id));

    let from_env = env.path("env.db");
    env.set("CASES_STORE", from_env.to_str().unwrap());
    let (id, _) = open_without_store(&env, &[]);
    assert!(has(&from_env, &id), "the environment beats the file");

    let from_flag = env.path("flag.db");
    let (id, _) = open_without_store(&env, &["--store", from_flag.to_str().unwrap()]);
    assert!(has(&from_flag, &id), "the flag wins");
    // The flag after the command too.
    let out = env.run(["list", "--store", from_flag.to_str().unwrap()]);
    assert!(out.stdout.contains(&id), "{out:?}");

    env.set("CASES_STORE", "");
    let (id, _) = open_without_store(&env, &[]);
    assert!(
        has(&from_file, &id),
        "a blank CASES_STORE falls back to the file"
    );
}

#[test]
fn a_leading_tilde_is_the_home_directory() {
    let env = Env::new();
    env.config("store = \"~/cases.db\"\n");
    let (id, _) = open_without_store(&env, &[]);
    assert!(has(&env.path("home").join("cases.db"), &id));
    let (id, _) = open_without_store(&env, &["--store", "~/flag.db"]);
    assert!(has(&env.path("home").join("flag.db"), &id));
}

/// With no server set, a store named without a `/` or `.db` is a file of
/// that name in the current directory, as a path would be (ST-11).
#[test]
fn a_bare_name_with_no_server_is_a_file_here() {
    let env = Env::new();
    let (id, _) = open_without_store(&env, &["--store", "inbox"]);
    assert!(has(&env.path("inbox"), &id));
    assert!(env.run(["--store", "inbox", "list"]).stdout.contains(&id));
}

#[test]
fn the_config_file_comes_from_the_flag_or_the_environment() {
    let mut env = Env::new();
    let store = env.path("mine.db");
    let config = env.path("mine.toml");
    write_file(&config, &format!("store = {:?}\n", store.to_str().unwrap()));
    let config = config.to_str().unwrap();
    let eq = format!("--config={config}");
    for args in [vec!["--config", config], vec![eq.as_str()]] {
        let (id, _) = open_without_store(&env, &args);
        assert!(has(&store, &id), "{args:?}");
    }
    let out = env.run(["list", "--config", config]);
    assert!(
        out.stdout.contains("UkLWZ"),
        "--config after the command: {out:?}"
    );

    env.set("CASES_CONFIG", config);
    let (id, _) = open_without_store(&env, &[]);
    assert!(has(&store, &id), "CASES_CONFIG");

    // A config file that is not there supplies nothing.
    let (id, _) = open_without_store(&env, &["--config", "none.toml"]);
    assert!(has(&env.path("data/cases/cases.db"), &id));
}

#[test]
fn the_last_config_flag_wins() {
    let mut env = Env::new();
    let file = |name: &str| {
        let config = env.path(&format!("{name}.toml"));
        let store = env.path(&format!("{name}.db"));
        write_file(&config, &format!("store = {:?}\n", store.to_str().unwrap()));
        (config.to_str().unwrap().to_owned(), store)
    };
    let (first, _) = file("first");
    let (second, second_store) = file("second");
    let (from_env, env_store) = file("env");
    env.set("CASES_CONFIG", &from_env);

    let (id, _) = open_without_store(&env, &["--config", &first, "--config", &second]);
    assert!(has(&second_store, &id), "two flags");
    let (id, _) = open_without_store(
        &env,
        &[&format!("--config={first}"), &format!("--config={second}")],
    );
    assert!(has(&second_store, &id), "two flags, = spelling");
    let (id, _) = open_without_store(&env, &["--config", &first, "--config="]);
    assert!(
        has(&env_store, &id),
        "an empty last flag falls through to the environment"
    );
    // Nothing after -- is a flag.
    let out = env.run([
        "--config",
        &second,
        "show",
        "--",
        &format!("--config={first}"),
    ]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("no case"), "{out:?}");
}

#[test]
fn a_config_file_that_cannot_be_used_stops_every_command_but_config() {
    let env = Env::new();
    let path = env.config("stor = \"x\"\n");
    for args in [
        &["list"][..],
        &["show", "x"],
        &[
            "open",
            "--kind",
            "fyi",
            "--urgency",
            "today",
            "--title",
            "x",
        ],
        &["wait"],
    ] {
        let out = env.cases(args);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        assert_eq!(
            out.stderr,
            format!(
                "Error: config file {}: unknown key \"stor\" (valid keys: store, forward, forward-token-file, server, default-store, token-file, listen, no-open, allow-hosts, tailscale-logins, store-tokens-file, name, prune-age, briefing-close-after, mood, tui-stats)\n",
                path.display()
            ),
            "{args:?}"
        );
    }
    assert!(!env.store.exists());
    // cases config is how to find out what is wrong, so the file does not
    // stop it.
    let out = env.run(["config", "path"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{} (exists)\n", path.display()));
    assert!(out.stderr.contains("cannot be used"), "{out:?}");
}

#[test]
fn a_config_file_of_the_wrong_shape_is_named_in_the_error() {
    let env = Env::new();
    for (text, want) in [
        ("store = 1\n", "key \"store\" must be a string, got number"),
        (
            "no-open = \"yes\"\n",
            "key \"no-open\" must be true or false",
        ),
        ("store = \n", "invalid TOML: line 1, column "),
        (
            "store = \"a\"\nstore = \"b\"\n",
            "invalid TOML: line 2, column ",
        ),
    ] {
        let path = env.config(text);
        let out = env.run(["list"]);
        assert_eq!(out.code, 1, "{text:?}: {out:?}");
        let prefix = format!("Error: config file {}: {want}", path.display());
        assert!(out.stderr.starts_with(&prefix), "{text:?}: {out:?}");
    }
    // Every key reads.
    env.config("store = \"~/c.db\"\nlisten = \"127.0.0.1:1\"\nno-open = true\nname = \"Ryan\"\nprune-age = \"720h\"\n");
    assert_eq!(env.run(["list"]).code, 0);
}

#[test]
fn version_prints_the_version() {
    let env = Env::new();
    for flag in ["-v", "--version"] {
        let out = env.run([flag]);
        assert_eq!(out.code, 0, "{out:?}");
        assert_eq!(out.stdout, format!("cases {}\n", env!("CARGO_PKG_VERSION")));
    }
    let out = env.run(["--help"]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("Usage: cases"), "{out:?}");
}

#[test]
fn bad_input_exits_1_not_clap_s_2() {
    let env = Env::new();
    for args in [
        &[][..],
        &["nope"],
        &["list", "--nope"],
        &["open", "--kind", "fyi"],
        &["show"],
    ] {
        let out = env.run(args);
        assert_eq!(out.code, 1, "{args:?}: {out:?}");
        assert_eq!(out.stdout, "", "{args:?}");
    }
}

#[test]
fn park_is_a_command_of_its_own() {
    let env = Env::new();
    let help = env.run(["--help"]).stdout;
    assert!(help.contains("\n  park "), "{help}");
}

/// Only open reads `CASES_WORKER` and `CASES_LABEL` (AG-01): list and wait
/// filter by what they are given, not by the environment.
#[test]
fn only_open_reads_the_worker_and_label_from_the_environment() {
    let mut env = Env::new();
    let other = env.open(&[
        "--kind",
        "fyi",
        "--urgency",
        "today",
        "--title",
        "Other",
        "--worker",
        "w1",
        "--label",
        "y",
    ]);
    env.set("CASES_WORKER", "w9");
    env.set("CASES_LABEL", "x");
    let mine = env.open(&["--kind", "fyi", "--urgency", "today", "--title", "Mine"]);
    assert_eq!(env.case(&mine).worker(), "w9");
    assert_eq!(env.case(&mine).labels(), ["x"]);
    let listed = env.ok(&["list"]);
    assert!(
        listed.contains(&other) && listed.contains(&mine),
        "{listed}"
    );
    // A wait scoped by neither wakes on the other worker's case too.
    env.write(&other, "human", "answer", r#"{"ack": true}"#);
    let out = env.cases(&["wait", "--since", &other, "--timeout", "5s"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(
        out.stdout.contains(&format!(r#""id":"{other}""#)),
        "{out:?}"
    );
}

/// There is no restore command: a case moved out by prune stays out (ST-13).
#[test]
fn there_is_no_restore_command() {
    let env = Env::new();
    let out = env.cases(&["restore", "UkLWZ"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(!env.ok(&["--help"]).contains("restore"));
}
