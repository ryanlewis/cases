//! `cases status`: where `cases serve` is running for the store (AG-15).

use std::io::Write;

use crate::ctx::Ctx;
use crate::error::Error;
use crate::instance;
use crate::store::{self, Target};

/// The command's arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Print the instance file's fields as JSON.
    #[arg(short, long)]
    json: bool,
}

/// Prints where serve is running, or fails with `not running` on stderr and
/// nothing on stdout.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let path = match &ctx.target {
        Target::Local { path } => path.clone(),
        // A remote store's inbox is the server's, and always there: this
        // says where, with no request (ST-11).
        Target::Remote { server, store } => {
            let url = store::inbox_url(server);
            if args.json {
                let shown = serde_json::json!({
                    "server": server.as_str(),
                    "store": {"id": store.id, "name": store.name},
                    "url": url,
                });
                serde_json::to_writer_pretty(&mut ctx.out, &shown)?;
                writeln!(ctx.out)?;
            } else {
                writeln!(ctx.out, "Remote store {store} on {server}, inbox at {url}")?;
            }
            return Ok(());
        }
        Target::Server { .. } => return Err(Error::new("status reaches no store")),
        // A forwarded store's inbox is served where the store is: this says
        // so, with nothing run there.
        Target::Forward { forward } => {
            if args.json {
                let shown = serde_json::json!({"forward": forward.to_string()});
                serde_json::to_writer_pretty(&mut ctx.out, &shown)?;
                writeln!(ctx.out)?;
            } else {
                writeln!(
                    ctx.out,
                    "Forwarded to {forward}: its inbox is served there, by cases serve"
                )?;
            }
            return Ok(());
        }
    };
    let Some(info) = instance::running(&path)? else {
        return Err(not_running("not running"));
    };
    if args.json {
        serde_json::to_writer_pretty(&mut ctx.out, &info)?;
        writeln!(ctx.out)?;
    } else {
        writeln!(
            ctx.out,
            "Serving {} at {} (pid {}, since {})",
            info.store, info.url, info.pid, info.started_at
        )?;
    }
    Ok(())
}

/// Exit 1 with `message` alone on stderr, not after `Error: `: no serve is
/// an answer, not a fault.
pub(crate) fn not_running(message: &str) -> Error {
    eprintln!("{message}");
    Error::new("")
}
