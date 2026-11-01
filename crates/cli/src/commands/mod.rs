//! The commands, one module each.

pub(crate) mod amend;
pub(crate) mod answer;
pub(crate) mod close;
pub(crate) mod config;
pub(crate) mod dial_stdio;
pub(crate) mod doing;
pub(crate) mod inbox;
pub(crate) mod list;
#[cfg(feature = "remote")]
pub(crate) mod login;
#[cfg(feature = "remote")]
pub(crate) mod login_status;
#[cfg(feature = "remote")]
pub(crate) mod logout;
pub(crate) mod message;
pub(crate) mod note;
pub(crate) mod open;
pub(crate) mod park;
pub(crate) mod pickup;
pub(crate) mod prune;
pub(crate) mod resume;
pub(crate) mod serve;
pub(crate) mod service;
pub(crate) mod show;
pub(crate) mod skill;
pub(crate) mod status;
#[cfg(feature = "remote")]
pub(crate) mod stores;
pub(crate) mod sweep;
pub(crate) mod tui;
pub(crate) mod wait;
pub(crate) mod withdraw;
pub(crate) mod write;

use crate::cli::Command;
use crate::ctx::Ctx;
use crate::error::Error;

/// Runs `command`.
pub(crate) fn run(command: &Command, ctx: &mut Ctx) -> Result<(), Error> {
    match command {
        Command::Open(args) => open::run(args, ctx),
        Command::List(args) => list::run(args, ctx),
        Command::Show(args) => show::run(args, ctx),
        Command::Tui(args) => tui::run(args, ctx),
        Command::Amend(args) => amend::run(args, ctx),
        Command::Wait(args) => wait::run(args, ctx),
        Command::Doing(args) => doing::run(args, ctx),
        Command::Pickup(args) => pickup::run(args, ctx),
        Command::Note(args) => note::run(args, ctx),
        Command::Close(args) => close::run(args, ctx),
        Command::Withdraw(args) => withdraw::run(args, ctx),
        Command::Answer(args) => answer::run(args, ctx),
        Command::Message(args) => message::run(args, ctx),
        Command::Park(args) => park::run(args, ctx),
        Command::Resume(args) => resume::run(args, ctx),
        Command::Sweep(args) => sweep::run(args, ctx),
        Command::Prune(args) => prune::run(args, ctx),
        Command::Serve(args) => serve::run(args, ctx),
        Command::Status(args) => status::run(args, ctx),
        #[cfg(feature = "remote")]
        Command::Stores(args) => stores::run(args, ctx),
        #[cfg(feature = "remote")]
        Command::Login(args) => login::run(args, ctx),
        #[cfg(feature = "remote")]
        Command::Logout(args) => logout::run(args, ctx),
        Command::Inbox(args) => inbox::run(args, ctx),
        Command::Service(args) => service::run(args, ctx),
        Command::Config(args) => config::run(args, ctx),
        Command::Skill(args) => skill::run(args, ctx),
        Command::DialStdio(args) => dial_stdio::run(args, ctx),
    }
}
