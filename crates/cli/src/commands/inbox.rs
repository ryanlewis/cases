//! `cases inbox`: opens the running inbox, or a case's page in it, or
//! prints its URL (AG-15).

use std::io::Write;
use std::path::Path;

use cases_core::Case;
use cases_store_sqlite::Key;

use crate::browser;
use crate::commands::status::not_running;
use crate::ctx::Ctx;
use crate::error::Error;
use crate::forward::Forward;
use crate::instance;
use crate::output;
use crate::store::{self, AnyStore, Server, Target};

/// The command's arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Case id, or any part of it that names one case.
    id: Option<String>,
    /// Print the URL instead of opening it.
    #[arg(long)]
    print: bool,
}

/// Opens or prints the inbox's URL, or the case's page.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let url = match ctx.target.clone() {
        Target::Remote { server, .. } => remote_url(args, ctx, &server)?,
        Target::Local { path } => local_url(args, ctx, &path)?,
        Target::Server { .. } => return Err(Error::new("inbox reaches no store")),
        Target::Forward { forward } => return forwarded(args, ctx, &forward),
    };
    if args.print {
        writeln!(ctx.out, "{url}")?;
        return Ok(());
    }
    browser::open(&url)
}

/// A remote store's inbox is the hosted one (ST-11).
fn remote_url(args: &Args, ctx: &mut Ctx, server: &Server) -> Result<String, Error> {
    let Some(id) = &args.id else {
        return Ok(store::inbox_url(server));
    };
    let (key, case) = ctx.find(id)?;
    Ok(hosted_case_url(ctx, key, &case).unwrap_or_else(|| store::inbox_url(server)))
}

/// The running local inbox's URL, or the case's page in it.
fn local_url(args: &Args, ctx: &mut Ctx, path: &Path) -> Result<String, Error> {
    let Some(info) = instance::running(path)? else {
        return Err(not_running(
            "not running; start one with `cases serve`, or `cases service install` for \
             an always-on inbox",
        ));
    };
    // A printed URL is a link, an allowed host's when serve has one,
    // for another device; the browser opened here goes straight to
    // the loopback address, which needs no proxy.
    let base = if args.print { info.link() } else { &info.url };
    match &args.id {
        Some(id) => Ok(instance::case_url(base, &case_page(ctx, id)?)),
        None => Ok(base.to_owned()),
    }
}

/// The case's page name: its id and title.
fn case_page(ctx: &mut Ctx, id: &str) -> Result<String, Error> {
    let (key, case) = ctx.find(id)?;
    Ok(output::titled(key, case.title()))
}

/// A printed URL is the far side's link, when its serve gives one: its
/// loopback address is no use here.
fn forwarded(args: &Args, ctx: &mut Ctx, forward: &Forward) -> Result<(), Error> {
    if args.print {
        // The case first, so that one not there is said as such.
        let page = match &args.id {
            Some(id) => Some(case_page(ctx, id)?),
            None => None,
        };
        if let AnyStore::Forward(forwarded) = &ctx.store
            && let Some(link) = forwarded.inbox_link()?
        {
            let url = match page {
                Some(page) => instance::case_url(&link, &page),
                None => link,
            };
            writeln!(ctx.out, "{url}")?;
            return Ok(());
        }
    }
    Err(Error::new(format!(
        "a forwarded store's inbox is served on {forward}, where the store is: run cases \
         inbox there"
    )))
}

/// The page of the case `key` in the hosted inbox, when the store is
/// remote, with no request (ST-11, AG-13).
pub(crate) fn hosted_case_url(ctx: &Ctx, key: Key, case: &Case) -> Option<String> {
    let (server, store) = ctx.store.hosted()?;
    Some(store::case_url(
        &server,
        &store,
        &output::titled(key, case.title()),
    ))
}
