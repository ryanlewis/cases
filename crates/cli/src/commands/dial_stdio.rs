//! `cases dial-stdio`: the far side of a forwarded store. A forwarding
//! cases runs it over ssh, `ssh host cases dial-stdio`, or by an `exec:`
//! forward's command, such as `docker exec -i box cases dial-stdio`, and
//! speaks the store
//! routes to it as HTTP/1.1 over its stdin and stdout (see [`crate::dial`]).
//! It serves that one connection against the local store, which it opens
//! itself, and exits when stdin ends: nothing else has to be running.
//! Several at once are several processes on one file, as every command is.
//!
//! It is hidden: a person never runs it.

use std::io;

use crate::ctx::Ctx;
use crate::dial;
use crate::error::Error;
use crate::store::AnyStore;

/// Its flags: none. The store is the local one, as `--store`,
/// `CASES_STORE` or the config file's `store` names it, never a forward or
/// a server.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {}

/// Serves the store routes over stdin and stdout until stdin ends.
pub(crate) fn run(_: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let AnyStore::Local(db) = &ctx.store else {
        return Err(Error::new("dial-stdio serves a local store file only"));
    };
    #[cfg(debug_assertions)]
    let db = &dial::test_clock(db);
    // A hangup, such as one sent to a process group left with a stopped
    // process in it, does not end it: its connection ends as its stdin
    // does, and the locks of a wait that lets them go itself are kept
    // until their lease runs out, not let go as the process dies.
    #[cfg(unix)]
    signal_hook::flag::register(
        signal_hook::consts::SIGHUP,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )?;
    let mut input = io::stdin().lock();
    dial::serve(db, dial::lease(), &mut input, &mut ctx.out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::forward::{Forward, Forwarded};
    use crate::store::Target;

    /// The command line never gives it another store (see
    /// `store::local_path`), so only a context made by hand reaches this.
    #[test]
    fn a_store_that_is_not_a_local_file_is_refused() {
        let forward: Forward = "ssh://devbox".parse().unwrap();
        let mut ctx = Ctx {
            store: AnyStore::Forward(Box::new(Forwarded::new(&forward, None))),
            target: Target::Forward { forward },
            config: Config::default(),
            store_flag: None,
            out: io::stdout().lock(),
        };
        let error = run(&Args {}, &mut ctx).unwrap_err();
        assert_eq!(
            error.to_string(),
            "dial-stdio serves a local store file only"
        );
    }
}
