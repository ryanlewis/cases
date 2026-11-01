//! The command tree: global flags and one subcommand per command.

use clap::{ArgAction, Parser, Subcommand};

use crate::commands::{
    amend, answer, close, config, dial_stdio, doing, inbox, list, message, note, open, park,
    pickup, prune, resume, serve, service, show, skill, status, sweep, tui, wait, withdraw,
};
#[cfg(feature = "remote")]
use crate::commands::{login, logout, stores};

/// Raise, answer, pick up and close cases in a SQLite store of events.
#[derive(Debug, Parser)]
#[command(
    name = "cases",
    version,
    disable_version_flag = true,
    args_override_self = true
)]
pub(crate) struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "STORE",
        help = "Case store: a SQLite file; the store on another machine over ssh, \
                ssh://[user@]host[:port], or over a command of yours, exec:<command>; with \
                a server set ($CASES_SERVER or the config \
                file's server), a store's name or id in the account; or a store's address, \
                https://host/<name or id>. Default: $CASES_FORWARD, then $CASES_STORE, then \
                the config file's forward (which also takes http://host[:port], a cases \
                serve there), then its store, then with a server the account's \
                default store, else $XDG_DATA_HOME/cases/cases.db or \
                ~/.local/share/cases/cases.db."
    )]
    pub(crate) store: Option<String>,

    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "TOML config file that supplies flag defaults. Default: $CASES_CONFIG, then \
                $XDG_CONFIG_HOME/cases/config.toml or ~/.config/cases/config.toml."
    )]
    pub(crate) config: Option<String>,

    /// Print version and exit.
    #[arg(short = 'v', long = "version", action = ArgAction::Version)]
    _version: Option<bool>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The commands.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Open a case (agent).
    Open(open::Args),
    /// Add options, rows or links to an open case, or replace its body,
    /// context, recommended option or a briefing's landed and left-out
    /// items (agent); with --as, edit a message not yet picked up (human).
    Amend(amend::Args),
    /// List cases.
    List(list::Args),
    /// Show one case and its thread.
    Show(show::Args),
    /// Block until a human answers, parks, resumes or withdraws a case, or
    /// sends a message to a wait that listens, then print the cases waiting
    /// on the agent as JSON lines (agent).
    Wait(wait::Args),
    /// Say what you are working on, beside your worker in the human's inbox
    /// while your wait listens (agent).
    Doing(doing::Args),
    /// Record that the answer has been read (agent).
    Pickup(pickup::Args),
    /// Add a follow-up to the thread of an open case; with --reopen, reopen
    /// an answered or picked-up case (agent).
    Note(note::Args),
    /// Record the outcome of a picked-up case (agent).
    Close(close::Args),
    /// Withdraw an open case that is no longer needed (agent); with --as, a
    /// sent message, or an answered or picked-up case, too (human).
    Withdraw(withdraw::Args),
    /// Answer an open case (human).
    Answer(answer::Args),
    /// Send a message to an agent: a worker or a label (human).
    Message(message::Args),
    /// Park an open stuck case until someone resumes it (human).
    Park(park::Args),
    /// Reopen a parked case (human).
    Resume(resume::Args),
    /// Withdraw the open cases that match, to clear the inbox; with
    /// --in-flight, the answered and picked-up ones too (human).
    Sweep(sweep::Args),
    /// Move closed and withdrawn cases older than --age into the store's
    /// archive (human).
    Prune(prune::Args),
    /// Read the inbox in the terminal (human).
    Tui(tui::Args),
    /// Serve the local web inbox on a loopback address.
    ///
    /// It also answers the store routes, for a cases on another machine
    /// whose forward is http://host:port, through a proxy such as
    /// tailscale serve --http=PORT. Straight to the loopback address they
    /// need nothing; through the proxy, a token of the config file's
    /// store-tokens-file or a login of its tailscale-logins. A request
    /// tailscale funnel passes on, or a browser sends, is refused.
    Serve(serve::Args),
    /// Print where cases serve is running for the store; exits 1 when it is
    /// not.
    Status(status::Args),
    /// List the account's stores on the server, and which one commands use.
    #[cfg(feature = "remote")]
    Stores(stores::Args),
    /// Sign this computer in to an account on a server, and keep its token;
    /// with --file, write a token for an agent to a file instead.
    #[cfg(feature = "remote")]
    #[command(
        long_about = "Sign this computer in to an account on a server, and keep its \
                            token; with --file, write a token for an agent to a file instead.\n\n\
                            The browser opens the server's approval page, and comes back to \
                            127.0.0.1; --device prints a page and a code to type in a browser \
                            anywhere. --store NAMES, split by commas, limits the stores the \
                            token opens; only the flag counts, never CASES_STORE or the config \
                            file. Default: every store in the account. Login writes no config.\n\n\
                            cases login status shows the token commands use, and what the \
                            server says about it."
    )]
    Login(login::Args),
    /// Revoke the login cases login kept for a server, then forget it; with
    /// --file, revoke the token in a file and remove the file.
    #[cfg(feature = "remote")]
    Logout(logout::Args),
    /// Open the running web inbox in the browser, or a case's page in it.
    Inbox(inbox::Args),
    /// Report on, install and remove cases serve as a user service.
    Service(service::Args),
    /// Inspect and create the config file that supplies flag defaults.
    Config(config::Args),
    /// Install, show and list the agent skill this binary carries.
    Skill(skill::Args),
    /// Serve the local store to a forwarding cases over stdin and stdout,
    /// as `ssh host cases dial-stdio`, or an `exec:` forward's command,
    /// runs it.
    #[command(hide = true)]
    DialStdio(dial_stdio::Args),
}

/// How far a command reaches into the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Not at all: it never reads the store.
    Nothing,
    /// To the server, on a remote store, but not the store: it needs no
    /// token, and the store is not looked up.
    Server,
    /// To the store, which is found, and on a server read with the token.
    Store,
    /// To the local store alone: a forward and a server are not read, so
    /// `dial-stdio` on the far side never forwards again.
    Local,
}

impl Command {
    /// How far the command reaches into the store. `config` and `skill`
    /// never read it, and `stores` reads the account, finding the store
    /// itself. On a remote store `serve` opens the hosted inbox and
    /// `service` installs nothing (ST-11), so they need the server alone.
    pub(crate) fn reach(&self) -> Reach {
        match self {
            Command::Config(_) | Command::Skill(_) => Reach::Nothing,
            #[cfg(feature = "remote")]
            Command::Stores(_) | Command::Login(_) | Command::Logout(_) => Reach::Nothing,
            Command::Serve(_) | Command::Service(_) => Reach::Server,
            Command::DialStdio(_) => Reach::Local,
            _ => Reach::Store,
        }
    }

    /// Whether the command is one that reports on the config file. Those
    /// run when the file cannot be used, since they are how to find out why.
    pub(crate) fn diagnoses_config(&self) -> bool {
        matches!(self, Command::Config(_))
    }
}
