//! `cases service`: reports on, installs and removes `cases serve` as a
//! user service (SV-03). Plain `cases service` is `cases service status`,
//! which only reads.

use std::env;
use std::io::{self, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde::Serialize;

use crate::config::{self, Config};
use crate::ctx::Ctx;
use crate::error::Error;
use crate::service::{Manager, Spec, System, UNIT};
use crate::store::{self, Target};
use crate::{instance, listen, program};

/// The command's arguments.
#[derive(Debug, clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    status: StatusArgs,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Report whether the cases serve service is installed, loaded and
    /// answering; exits 1 unless all three. Plain cases service does the
    /// same.
    Status(StatusArgs),
    /// Run cases serve as a user service that starts at login and restarts
    /// when it stops (launchd on macOS, systemd on Linux).
    Install {
        /// Address the service's serve listens on. Only loopback addresses
        /// are allowed. Default: the config file's listen, then
        /// 127.0.0.1:7337.
        #[arg(long, value_name = "HOST:PORT")]
        listen: Option<String>,
        /// Record answers, parks, resumes and withdraws from the inbox as
        /// written by NAME. Default: the config file's name.
        #[arg(long = "as", value_name = "NAME")]
        name: Option<String>,
    },
    /// Stop the cases serve user service and remove its file.
    Uninstall,
}

/// The arguments of `cases service status`.
#[derive(Debug, clap::Args)]
struct StatusArgs {
    /// Print JSON.
    #[arg(short, long)]
    json: bool,
}

/// Runs the command.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    if let (
        Some(Command::Install { .. }),
        Target::Remote { server, .. } | Target::Server { server },
    ) = (&args.command, &ctx.target)
    {
        return Err(Error::new(format!(
            "a remote store needs no local service; its inbox is at {}",
            store::inbox_url(server)
        )));
    }
    if let (Some(Command::Install { .. }), Target::Forward { forward }) =
        (&args.command, &ctx.target)
    {
        return Err(Error::new(format!(
            "a forwarded store needs no local service: install it on {forward}, where the store is"
        )));
    }
    let manager = Manager::for_this_machine()?;
    match &args.command {
        None => status(&args.status, ctx, &manager),
        Some(Command::Status(status_args)) => status(status_args, ctx, &manager),
        Some(Command::Install { listen, name }) => {
            install(ctx, &manager, listen.as_deref(), name.as_deref())
        }
        Some(Command::Uninstall) => {
            let path = manager.uninstall()?;
            writeln!(
                ctx.out,
                "Stopped the cases serve service and removed {}",
                path.display()
            )?;
            Ok(())
        }
    }
}

/// Installs the service with serve's flags written in: the store's absolute
/// path, the config file, `--listen`, `--as`, and `--no-open`, since a
/// service has no terminal.
fn install(
    ctx: &mut Ctx,
    manager: &Manager,
    listen: Option<&str>,
    name: Option<&str>,
) -> Result<(), Error> {
    let Some(path) = ctx.target.path().map(Path::to_path_buf) else {
        return Err(Error::new("a remote store needs no local service"));
    };
    let listen = listen
        .or_else(|| ctx.config.get("listen"))
        .unwrap_or(config::DEFAULT_LISTEN);
    listen::check_loopback(listen)?;
    let name = name.or_else(|| ctx.config.get("name")).unwrap_or_default();
    let store = instance::clean(&std::path::absolute(&path)?);
    check_no_serve(manager, &store, listen)?;
    let spec = serve_spec(manager.system, &store, &ctx.config, listen, name)?;
    let path = manager.install(&spec)?;
    let out = &mut ctx.out;
    writeln!(out, "Installed the cases serve service: {}", path.display())?;
    writeln!(out, "It runs: {}", shell_join(&spec.args))?;
    writeln!(
        out,
        "It starts at login and restarts when it stops. Rerun install after changing the \
         store, config, --listen or --as."
    )?;
    writeln!(out, "Check it with: cases service")?;
    writeln!(out, "Service manager: {}", manager.check())?;
    match manager.system {
        System::Launchd => writeln!(out, "Log: {}", spec.log.display())?,
        System::Systemd => writeln!(out, "Log: journalctl --user -u {UNIT}")?,
    }
    Ok(())
}

/// Refuses an install while a serve runs for the store, or something holds
/// the address: the service would fail on every start and be restarted
/// forever. What the loaded service itself holds is the serve install
/// restarts, so that is allowed.
fn check_no_serve(manager: &Manager, store: &Path, listen: &str) -> Result<(), Error> {
    let (mut old_store, mut old_listen) = (None, None);
    if manager.installed()?
        && manager.active()?
        && let Ok(args) = manager.args()
    {
        old_store = flag_value(&args, "--store").map(PathBuf::from);
        old_listen = flag_value(&args, "--listen").map(str::to_owned);
    }
    // The same store by another path is still the service's own.
    if old_store.as_deref().map(instance::resolve) != Some(instance::resolve(store))
        && let Ok(Some(running)) = instance::running(store)
    {
        return Err(Error::new(format!(
            "cases serve is already running for {} at {} (pid {}); stop it before installing \
             the service",
            running.store, running.url, running.pid
        )));
    }
    if old_listen.as_deref() == Some(listen) {
        return Ok(());
    }
    TcpListener::bind(listen).map(drop).map_err(|error| {
        Error::new(format!(
            "{listen} is already in use ({error}); stop what holds it, or pass another \
             --listen, before installing the service"
        ))
    })
}

/// The service for serve on `store`. `HOME` and `XDG_STATE_HOME` are set,
/// so that serve records itself where `cases status` looks. Under launchd
/// serve writes and rotates its own log; systemd's journal keeps its own
/// in bounds.
fn serve_spec(
    system: System,
    store: &Path,
    config: &Config,
    listen: &str,
    name: &str,
) -> Result<Spec, Error> {
    let home = env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .ok_or_else(|| Error::new("$HOME is not set"))?;
    // The service does not start in this directory, so a relative
    // $XDG_STATE_HOME would put it somewhere cases status does not look.
    let state = std::path::absolute(instance::dir()?)?;
    let mut args = vec![text(&executable()?), "--store".to_owned(), text(store)];
    if !config.path.as_os_str().is_empty() {
        args.extend([
            "--config".to_owned(),
            text(&std::path::absolute(&config.path)?),
        ]);
    }
    args.extend(["serve", "--no-open", "--listen", listen].map(str::to_owned));
    if !name.trim().is_empty() {
        args.extend(["--as".to_owned(), name.to_owned()]);
    }
    let log = state.join("serve.log");
    if system == System::Launchd {
        args.extend(["--log".to_owned(), text(&log)]);
    }
    Ok(Spec {
        args,
        env: vec![
            ("HOME".to_owned(), home),
            (
                "XDG_STATE_HOME".to_owned(),
                text(state.parent().unwrap_or(&state)),
            ),
        ],
        log,
    })
}

/// A path as text for the service file.
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// This binary, by the path it was run as when that is the same file as the
/// one running. On Linux the running one is the file a link points at, and
/// a package manager's `bin/cases` points at a versioned file an upgrade
/// removes: the service keeps to the link.
pub(crate) fn executable() -> Result<PathBuf, Error> {
    let exe = env::current_exe()?;
    let run_as = env::args_os().next().map(PathBuf::from).and_then(|arg| {
        if arg.components().count() > 1 {
            Some(arg)
        } else {
            program::find(arg.to_str()?).ok()
        }
    });
    if let Some(path) = run_as
        && same_file(&path, &exe)
        && let Ok(path) = std::path::absolute(&path)
    {
        return Ok(path);
    }
    Ok(exe)
}

/// Whether `a` and `b` are both there and the same file.
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.metadata(), b.metadata()) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Whether `a` and `b` are both there and the same file.
#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// `args` joined for people, single-quoting any a shell would split or
/// expand, so that the line can be pasted and runs as the service does.
fn shell_join(args: &[String]) -> String {
    let quoted: Vec<String> = args
        .iter()
        .map(|arg| {
            if arg.is_empty()
                || arg
                    .chars()
                    .any(|c| " \t\n'\"\\$`;&|<>()*?[]#~!{}".contains(c))
            {
                format!("'{}'", arg.replace('\'', r"'\''"))
            } else {
                arg.clone()
            }
        })
        .collect();
    quoted.join(" ")
}

/// The value after `flag` in `args`.
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
}

/// What `cases service` finds, and its `--json`.
#[derive(Debug, Default, Serialize)]
struct Report {
    installed: bool,
    path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    store: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    listen: String,
    /// The `allow-hosts` of the service's config file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allow_hosts: Vec<String>,
    /// The `tailscale-logins` of the service's config file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tailscale_logins: Vec<String>,
    loaded: bool,
    running: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<i32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    problems: Vec<String>,
}

/// Reports the service's file, whether the service manager has it and
/// whether its inbox answers, and names each disagreement between them. It
/// changes nothing, and exits 1 unless the service is installed, loaded and
/// answering.
fn status(args: &StatusArgs, ctx: &mut Ctx, manager: &Manager) -> Result<(), Error> {
    let report = report(ctx, manager)?;
    let out = &mut ctx.out;
    if args.json {
        serde_json::to_writer_pretty(&mut *out, &report)?;
        writeln!(out)?;
    } else {
        write_text(out, &report, &manager.check())?;
    }
    if report.installed && report.loaded && report.running {
        return Ok(());
    }
    Err(Error::new(""))
}

/// `report` for people. `check` is the command that shows the service
/// manager's view of the service.
fn write_text(out: &mut impl Write, report: &Report, check: &str) -> io::Result<()> {
    let yes = |value| if value { "yes" } else { "no" };
    if report.installed {
        writeln!(out, "Installed: {}", report.path)?;
    } else {
        writeln!(out, "Installed: no ({})", report.path)?;
    }
    if !report.args.is_empty() {
        writeln!(out, "Runs:      {}", shell_join(&report.args))?;
        writeln!(out, "Store:     {}", report.store)?;
        writeln!(out, "Listen:    {}", report.listen)?;
        if !report.allow_hosts.is_empty() {
            writeln!(out, "Hosts:     {}", report.allow_hosts.join(", "))?;
        }
        if !report.tailscale_logins.is_empty() {
            writeln!(out, "Logins:    {}", report.tailscale_logins.join(", "))?;
        }
    }
    writeln!(out, "Loaded:    {} ({check})", yes(report.loaded))?;
    match report.pid {
        Some(pid) if report.running => writeln!(out, "Inbox:     {} (pid {pid})", report.url)?,
        _ => writeln!(out, "Inbox:     not running")?,
    }
    for problem in &report.problems {
        writeln!(out, "Problem:   {problem}")?;
    }
    Ok(())
}

/// What the service manager, the service's file and the instance file say.
fn report(ctx: &Ctx, manager: &Manager) -> Result<Report, Error> {
    let mut report = Report {
        path: text(&manager.path()),
        installed: manager.installed()?,
        loaded: manager.active()?,
        ..Report::default()
    };
    // A remote store has no file: the installed service names its own,
    // and with none installed there is no inbox to look for.
    let store = match ctx.target.path() {
        Some(path) => text(&instance::clean(&std::path::absolute(path)?)),
        None => String::new(),
    };
    // The inbox to look for is the installed service's; this command's
    // store stands in when there is none.
    report.store.clone_from(&store);
    if report.installed {
        match manager.args() {
            Err(error) => report.problems.push(error.to_string()),
            Ok(args) => read_installed(&mut report, args, &store),
        }
    }
    let running = if report.store.is_empty() {
        Ok(None)
    } else {
        instance::running(Path::new(&report.store))
    };
    match running {
        Ok(Some(running)) => {
            report.running = true;
            report.url = running.url;
            report.pid = Some(running.pid);
        }
        Ok(None) => {}
        Err(error) => report.problems.push(error.to_string()),
    }
    report.problems.extend(problem(
        report.installed,
        report.loaded,
        report.running,
        manager.system,
        &manager.check(),
        report.pid,
    ));
    Ok(report)
}

/// What the installed service's `args` say into `report`: its store and
/// address, the hosts its config file allows, and whether it is for
/// another binary, or for another store than this command's `store`
/// (empty for a store with no file here).
fn read_installed(report: &mut Report, args: Vec<String>, store: &str) {
    flag_value(&args, "--store")
        .unwrap_or_default()
        .clone_into(&mut report.store);
    flag_value(&args, "--listen")
        .unwrap_or_default()
        .clone_into(&mut report.listen);
    // The hosts the service's serve answers through a proxy, from the
    // config file it reads.
    if let Some(path) = flag_value(&args, "--config") {
        let config = config::load(Some(path));
        if let Some(error) = config.error {
            report.problems.push(error.to_string());
        } else {
            report.allow_hosts = config.list("allow-hosts").to_vec();
            report.tailscale_logins = config.list("tailscale-logins").to_vec();
        }
    }
    if let Ok(exe) = executable()
        && !same_file(Path::new(&args[0]), &exe)
    {
        report.problems.push(format!(
            "installed for another binary: {} (this is {})",
            args[0],
            exe.display()
        ));
    }
    // The same store by another path is still this one, as install takes
    // it.
    if !store.is_empty()
        && instance::resolve(Path::new(&report.store)) != instance::resolve(Path::new(store))
    {
        report.problems.push(format!(
            "installed for another store: {} (this command uses {store})",
            report.store
        ));
    }
    report.args = args;
}

/// The problem with a service `installed` (its file is there), `loaded` in
/// `system` and `running` (its inbox answers, as `pid`), if there is one.
/// `check` is the command that shows the service manager's view.
fn problem(
    installed: bool,
    loaded: bool,
    running: bool,
    system: System,
    check: &str,
    pid: Option<i32>,
) -> Option<String> {
    let system = system.name();
    match (installed, loaded, running) {
        (true, false, _) => Some(format!(
            "installed but not loaded in {system}; cases service install loads it again"
        )),
        (false, true, _) => Some(format!(
            "loaded in {system} but its file is missing; cases service uninstall stops it"
        )),
        (true, true, false) => Some(format!("loaded but the inbox does not answer; see {check}")),
        (false, false, true) => Some(format!(
            "not installed, but a cases serve started by hand is running (pid {}); stop it \
             before cases service install",
            pid.unwrap_or_default()
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_join_quotes_what_a_shell_would_split() {
        let args = ["/bin/cases", "--as", "Ryan Lewis", "", "it's"].map(str::to_owned);
        assert_eq!(
            shell_join(&args),
            r"/bin/cases --as 'Ryan Lewis' '' 'it'\''s'"
        );
    }

    /// Every combination of installed, loaded and running, row by row: the
    /// four that disagree name their problem and what to do; the others
    /// have none.
    #[test]
    fn problem_names_each_disagreement_and_what_to_do() {
        let check = "systemctl --user status cases-serve.service";
        let not_loaded =
            "installed but not loaded in systemd; cases service install loads it again";
        let file_gone =
            "loaded in systemd but its file is missing; cases service uninstall stops it";
        let not_answering = format!("loaded but the inbox does not answer; see {check}");
        let by_hand = "not installed, but a cases serve started by hand is running (pid 42); \
                       stop it before cases service install";
        let rows: [(bool, bool, bool, Option<&str>); 8] = [
            (false, false, false, None),
            (false, false, true, Some(by_hand)),
            (false, true, false, Some(file_gone)),
            (false, true, true, Some(file_gone)),
            (true, false, false, Some(not_loaded)),
            (true, false, true, Some(not_loaded)),
            (true, true, false, Some(&not_answering)),
            (true, true, true, None),
        ];
        for (installed, loaded, running, want) in rows {
            assert_eq!(
                problem(installed, loaded, running, System::Systemd, check, Some(42)).as_deref(),
                want,
                "{installed} {loaded} {running}"
            );
        }
        assert_eq!(
            problem(true, false, false, System::Launchd, check, None).as_deref(),
            Some("installed but not loaded in launchd; cases service install loads it again")
        );
        assert_eq!(
            problem(false, true, false, System::Launchd, check, None).as_deref(),
            Some("loaded in launchd but its file is missing; cases service uninstall stops it")
        );
    }

    #[test]
    fn flag_value_is_the_argument_after_the_flag() {
        let args = ["cases", "--store", "/a.db", "--listen"].map(str::to_owned);
        assert_eq!(flag_value(&args, "--store"), Some("/a.db"));
        assert_eq!(flag_value(&args, "--listen"), None);
        assert_eq!(flag_value(&args, "--as"), None);
    }
}
