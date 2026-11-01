//! `cases`: a human-in-the-loop inbox for agents.
//!
//! The exit status says what kind of failure a command hit (CL-01): 0
//! success, 1 any other failure, 2 `wait` timed out, 3 the case's state does
//! not allow the event, 4 the case changed since the revision given.

mod briefings;
mod browser;
mod cli;
mod commands;
mod config;
#[cfg(feature = "remote")]
mod credentials;
mod ctx;
mod describe;
mod dial;
mod duration;
mod error;
mod forward;
#[cfg(feature = "remote")]
mod hosted;
#[cfg(not(feature = "remote"))]
#[path = "hosted_off.rs"]
mod hosted;
mod instance;
mod listen;
mod output;
mod program;
mod service;
mod store;
mod text;
mod tokens;

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use cases_store_sqlite::Db;
use clap::Parser;
use clap::error::ErrorKind;

use crate::cli::{Cli, Reach};
use crate::ctx::Ctx;
use crate::error::Error;
use crate::store::{AnyStore, Selection, Target};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            // Help and the version are what was asked for. Any other parse
            // error is bad input, which exits 1 like every other failure:
            // clap's own 2 is `wait`'s timeout.
            return match error.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::SUCCESS,
                _ => ExitCode::FAILURE,
            };
        }
    };
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => error.report(),
    }
}

/// Loads the config, finds the store, and runs the command.
fn run(cli: &Cli) -> Result<(), Error> {
    let mut config = config::load(cli.config.as_deref());
    // A file that cannot be used stops every command but the ones that
    // report on it: they are how to find out what is wrong.
    if !cli.command.diagnoses_config()
        && let Some(error) = config.error.take()
    {
        return Err(error);
    }
    // A command that does not read the store is not stopped by a store
    // that cannot be named, and one that needs no more than the server
    // needs no token: the store is found, and the token read, only for a
    // command that reaches the store (SV-01).
    let reach = cli.command.reach();
    let (target, store) = match (store::select(cli.store.as_deref(), &config), reach) {
        (Err(_), Reach::Nothing) => unreached(Target::Local {
            path: PathBuf::new(),
        }),
        (_, Reach::Local) => store::connect(
            Selection::Local(store::local_path(cli.store.as_deref(), &config)?),
            &config,
        )?,
        (Err(error), _) => return Err(error),
        (Ok(Selection::Remote { server, .. }), Reach::Nothing | Reach::Server) => {
            unreached(Target::Server { server })
        }
        (Ok(selection), _) => store::connect(selection, &config)?,
    };
    let mut ctx = Ctx {
        target,
        store,
        config,
        store_flag: cli.store.clone(),
        out: io::stdout().lock(),
    };
    commands::run(&cli.command, &mut ctx)
}

/// `target` for a command that never reads the store, with a store that
/// is never opened.
fn unreached(target: Target) -> (Target, AnyStore) {
    (target, AnyStore::Local(Db::new(PathBuf::new())))
}
