//! `cases config`: reports on and creates the config file (SV-01). Each
//! subcommand runs when the file cannot be used, since they are how to find
//! out why.

use std::fs;
use std::io::Write;

use clap::Subcommand;

use crate::config::{self, Config};
use crate::ctx::Ctx;
use crate::error::Error;

/// The command's arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Command,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Print the config file in use and whether it exists.
    Path,
    /// Print the defaults the environment and the config file establish.
    Show,
    /// Write a commented config file template.
    Init {
        /// Overwrite an existing config file.
        #[arg(short, long)]
        force: bool,
    },
}

/// The file `config init` writes: every key, commented out, with what it
/// does. It is a valid config file that sets nothing.
pub(crate) const TEMPLATE: &str = r#"# cases configuration
#
# Defaults for the flags below. Precedence is flag > environment >
# this file > built-in default.
#
# Read from $XDG_CONFIG_HOME/cases/config.toml, or
# ~/.config/cases/config.toml. Override with --config PATH or
# $CASES_CONFIG. The file is optional.
#
# Uncomment a line to change the default.

# Case store: one SQLite file, made on the first write. Same as --store
# or $CASES_STORE. A leading ~ is expanded. Keep it on a local disk,
# not in a synced folder or on a network share. Default:
# $XDG_DATA_HOME/cases/cases.db, or ~/.local/share/cases/cases.db.
# store = "~/cases/work.db"

# Forward every command to the store on another machine, over ssh:
# each connection runs `ssh host cases dial-stdio` there, with your ssh
# config and keys. Same as $CASES_FORWARD, which comes before
# $CASES_STORE; this comes before the store above. Default: none, so
# the store is a local file.
# forward = "ssh://devbox"
# Or forward over any command that gives a pipe to cases dial-stdio
# there; the whole command, run by sh -c:
# forward = "exec:docker exec -i box cases dial-stdio"
# Or over HTTP to cases serve there, through tailscale serve on the
# tailnet (tailscale serve --bg --http=7338 http://127.0.0.1:7337 there).
# Plain http only over a tailnet, whose WireGuard encrypts it; https
# needs a build with the remote feature.
# forward = "http://devbox.example.ts.net:7338"

# A file that holds the token an http:// forward sends, on a line of its
# own: one of the store-tokens-file of the cases serve there. Keep it
# private (chmod 600). Without it, the serve there lets in only a
# Tailscale login in its tailscale-logins; a tagged device has none, so
# needs a token. Same as $CASES_FORWARD_TOKEN_FILE. Default: none.
# forward-token-file = "~/.config/cases/forward-token"

# Server of remote stores. With it set, a store that is not a path is
# a store's name or id in your account there, and with no store named,
# the store is the account's default. Same as $CASES_SERVER. Default:
# none, so the store is a local file.
# server = "https://cases.sh"

# The store on the server to use when none is named: its name or id.
# Default: the account's default store, else the only store the token
# opens.
# default-store = "me"

# A file that holds the token for the server, on one line. Keep it
# private (chmod 600). Same as $CASES_TOKEN_FILE. $CASES_TOKEN comes
# before it, and it comes before the token cases login keeps.
# token-file = "~/.config/cases/token"

# Address cases serve listens on. Same as --listen on serve and
# service install. Loopback only.
# listen = "127.0.0.1:7337"

# Set to true to stop cases serve opening the inbox in the browser.
# Same as --no-open.
# no-open = true

# Other hosts cases serve answers to, as the browser sends them in Host
# (host:port, the port left out for 443), for a proxy that serves the
# inbox over HTTPS, such as tailscale serve:
#   tailscale serve --bg --https=7337 http://127.0.0.1:7337
# A write through one must come from https://<host>. serve still
# listens on loopback only, and refuses any request tailscale funnel
# passes on. The links cases gives, such as a case's url, name the
# first of them over https. Default: none.
# allow-hosts = ["devbox.example.ts.net:7337"]

# Tailscale logins that may use the inbox through allow-hosts: a
# request through one of them is refused unless tailscale serve's
# Tailscale-User-Login header names one of these, as is one it passes
# on with another Host. Requests made straight to the loopback address
# need none. Default: none, so any request through allow-hosts is let in.
# The store routes of an http:// forward on another machine are let in
# through the proxy for one of these logins too.
# tailscale-logins = ["ryan@github"]

# A file of the tokens cases serve takes on its store routes, the ones
# an http:// forward on another machine sends, one to a line, such as
# openssl rand -hex 32 gives. A store route through a proxy, such as
# tailscale serve --http=7338, needs one of these as its bearer token,
# or a login of tailscale-logins; straight to the loopback address it
# needs neither. Keep it private (chmod 600). Default: none, so a store
# route through a proxy needs a login.
# store-tokens-file = "~/.config/cases/store-tokens"

# Your name, recorded as the actor on answers, parks, resumes and
# withdraws from cases answer, park, resume and sweep, and the inbox.
# Same as --as on those commands and on serve and service install;
# cases withdraw is the human's only with its own --as NAME. Default:
# the login name, from $USER or $LOGNAME.
# name = "Ryan"

# How long after its last event a closed or withdrawn case is pruned,
# such as 30d, 720h or 90m. Same as --age on cases prune.
# prune-age = "2160h"

# How long a briefing stays picked up before cases serve closes it as
# expired, saying why in its outcome, such as 14d, or off. An agent
# closes its own briefings; this ends the ones it never closed. serve
# checks each hour; cases sweep --stale-briefings does it by hand.
# Default: 7d.
# briefing-close-after = "14d"

# How long cases wait keeps collecting after the first answer, so that
# answers given together wake the agent once: eager, steady or patient.
# Same as --mood on cases wait.
# mood = "steady"

# Whether cases tui shows its stats strip at first: auto, text or off.
# s shows or hides it either way. Default: auto.
# tui-stats = "off"
"#;

/// Runs the command.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    match args.command {
        Command::Path => path(ctx),
        Command::Show => show(ctx),
        Command::Init { force } => init(ctx, force),
    }
}

/// Prints the file's path and whether it is there, and warns when it cannot
/// be used.
fn path(ctx: &mut Ctx) -> Result<(), Error> {
    let config = &mut ctx.config;
    if config.path.as_os_str().is_empty() {
        return Err(config.error.take().unwrap_or_else(unfound));
    }
    writeln!(ctx.out, "{} ({})", config.path.display(), existence(config))?;
    if let Some(error) = &config.error {
        crate::ctx::warn(format_args!("this file cannot be used: {error}"));
    }
    Ok(())
}

/// Prints each key's value and where it came from, after the file.
fn show(ctx: &mut Ctx) -> Result<(), Error> {
    let config = &mut ctx.config;
    if !config.path.as_os_str().is_empty() {
        writeln!(
            ctx.out,
            "config: {} ({})",
            config.path.display(),
            existence(config)
        )?;
    }
    if let Some(error) = config.error.take() {
        // The file as it is, to see the problem in (SV-01, case
        // 2026-09-25T20-55-32Z-broken-config-what-do-config-show-and-init-do).
        if let Ok(text) = fs::read_to_string(&config.path) {
            writeln!(ctx.out, "\n{}", text.trim_end_matches('\n'))?;
        }
        return Err(error);
    }
    writeln!(ctx.out, "These apply when no flag overrides them.\n")?;
    let settings = config::settings(config);
    let key_width = settings
        .iter()
        .map(|(key, ..)| key.len())
        .max()
        .unwrap_or(0);
    let value_width = settings
        .iter()
        .map(|(_, value, _)| value.chars().count())
        .max()
        .unwrap_or(0);
    for (key, value, source) in settings {
        writeln!(
            ctx.out,
            "  {key:key_width$}  {value:value_width$}  {source}"
        )?;
    }
    // A value cases tui would refuse, said now rather than when it starts.
    if let Err(error) = crate::commands::tui::stats::Mode::of(config.get("tui-stats")) {
        crate::ctx::warn(error);
    }
    // A store that is a path wins over a server (ST-11), and says so.
    if let Some((server, _)) = config::server_text(config)
        && let Some((store, _)) = config::store_text(None, config)
        && config::is_path(&store)
    {
        writeln!(
            ctx.out,
            "\nThe store is the file {}, which wins over the server {server}.",
            config::expand_home(&store).display()
        )?;
    }
    Ok(())
}

/// Writes the template, unless a file is there and `force` is not set.
fn init(ctx: &mut Ctx, force: bool) -> Result<(), Error> {
    let config = &mut ctx.config;
    if config.path.as_os_str().is_empty() {
        return Err(config.error.take().unwrap_or_else(unfound));
    }
    let path = &config.path;
    if exists(config) && !force {
        return Err(Error::new(format!(
            "config file already exists: {} (pass --force to overwrite)",
            path.display()
        )));
    }
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir)
            .map_err(|error| Error::new(format!("{}: {error}", dir.display())))?;
    }
    fs::write(path, TEMPLATE)
        .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;
    writeln!(ctx.out, "Wrote config template to {}", path.display())?;
    Ok(())
}

/// Whether something is at the file's path. A file that cannot be read, or a
/// directory, is there: `config init` must not replace it unasked.
fn exists(config: &Config) -> bool {
    fs::metadata(&config.path).is_ok()
}

/// How `path` and `show` say whether the file is there.
fn existence(config: &Config) -> &'static str {
    if exists(config) {
        "exists"
    } else {
        "not found"
    }
}

/// The error for a file whose path could not be worked out, when the load
/// gave none.
fn unfound() -> Error {
    Error::new("cannot find the config file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_names_every_key_and_sets_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, TEMPLATE).unwrap();
        let loaded = config::load(path.to_str());
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        for key in config::KEYS {
            assert_eq!(loaded.get(key), None, "{key}");
            // Each key has a line to uncomment, which sets it.
            let line = TEMPLATE
                .lines()
                .find(|line| line.starts_with(&format!("# {key} = ")))
                .unwrap_or_else(|| panic!("no line for {key}"));
            fs::write(&path, &line[2..]).unwrap();
            let loaded = config::load(path.to_str());
            assert!(loaded.error.is_none(), "{line}: {:?}", loaded.error);
            assert!(loaded.get(key).is_some(), "{line}");
        }
    }
}
