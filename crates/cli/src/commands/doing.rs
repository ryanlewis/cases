//! `cases doing`: the agent says what it is working on, beside its name in
//! the human's inbox while it listens for messages (AG-17).

use cases_core::Store as _;
use cases_store_sqlite::doing_line;
use clap::builder::{StringValueParser, TypedValueParser};

use crate::commands::open::given;
use crate::ctx::Ctx;
use crate::error::Error;

/// The arguments of `cases doing`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// What you are working on now: one line of at most 80 characters.
    /// Blank clears it.
    #[arg(value_name = "TEXT", value_parser = line())]
    text: String,

    #[arg(
        long,
        value_name = "NAME",
        help = "The worker that is listening, as its cases wait --worker gives it. Default: \
                $CASES_WORKER."
    )]
    worker: Option<String>,
}

/// TEXT, which must be a doing line: checked before the store is read.
pub(crate) fn line() -> impl TypedValueParser<Value = String> {
    StringValueParser::new().try_map(|text: String| {
        doing_line(&text)
            .map(|_| text)
            .map_err(|error| error.to_string())
    })
}

/// Sets the worker's doing line.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let worker = args
        .worker
        .clone()
        .or_else(|| given("CASES_WORKER"))
        .ok_or_else(|| Error::new("doing needs --worker, or CASES_WORKER set"))?;
    match ctx.store.set_doing(&worker, &args.text) {
        Ok(()) => Ok(()),
        // No store yet: no one listens to it. On a remote store no wait of
        // this cases listens until the listening wait comes (ST-11, ST-16),
        // so it says the same.
        Err(error) if error.is_missing() || error.not_listening() => {
            Err(cases_store_sqlite::Error::NotListening { worker }.into())
        }
        Err(error) => Err(error.into()),
    }
}
