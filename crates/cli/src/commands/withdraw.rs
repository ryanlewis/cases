//! `cases withdraw`: withdraws an open case that is no longer needed
//! (AG-11), or with `--as`, the human's withdraw, which may also end a case
//! an agent has, answered or picked up (ST-14).

use cases_core::{Author, Record, Withdraw};

use crate::commands::write::{self, WriteArgs};
use crate::ctx::{self, Ctx};
use crate::error::Error;

/// The arguments of `cases withdraw`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id. With --as, any part of it that names one case.
    id: String,

    /// Why the case no longer needs an answer.
    #[arg(long, value_name = "TEXT")]
    reason: Option<String>,

    /// Withdraw as the human, written by NAME: an open case, a message no
    /// agent has picked up, or a case an agent has, answered or picked up,
    /// such as one whose session died. Without it, the agent withdraws an
    /// open case. NAME may not be blank, and the config file's name does not
    /// stand in for it.
    #[arg(long = "as", value_name = "NAME", value_parser = ctx::human_name("withdraw"))]
    as_: Option<String>,

    #[command(flatten)]
    write: WriteArgs,
}

/// Withdraws the case, as the agent, or as the human with `--as`.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let (key, author, writer) = if let Some(name) = args.as_.as_deref() {
        let key = ctx.find_case(&args.id)?;
        (key, Author::Human, ctx.human(Some(name)))
    } else {
        let key = ctx::exact(&args.id)?;
        (key, Author::Agent, write::agent(ctx, key)?)
    };
    let mut withdraw = Withdraw::default();
    withdraw.reason = args.reason.clone().unwrap_or_default();
    let record = Record::Withdraw(withdraw);
    let case = write::append(ctx, key, author, &record, &writer, args.write.revision)?;
    write::print(ctx, key, &case, args.write.json)
}
