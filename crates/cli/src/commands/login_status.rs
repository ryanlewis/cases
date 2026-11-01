//! `cases login status`: the token commands use for a server, and what the
//! server says about it (AC-03, SV-01).
//!
//! The server is the argument, else `CASES_SERVER`, else the config file's
//! `server`. The token is found in SV-01's order, and always checked with
//! `GET /v1/cli/whoami`, which also refreshes the account's kept list of
//! stores. It never prints the token.

use std::fmt::Write as _;
use std::io::Write as _;

use cases_store_remote::{Server, Whoami};
use serde_json::{Value, json};

use crate::commands::login;
use crate::commands::stores::{self, Selected};
use crate::config::Config;
use crate::credentials;
use crate::ctx::Ctx;
use crate::error::Error;
use crate::store::{self, Source, Which};

/// The arguments of `cases login status`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    #[arg(
        value_name = "SERVER",
        help = "The server, https://host[:port]. Default: $CASES_SERVER, then the config file's \
                server."
    )]
    server: Option<String>,

    /// Print the server, the account, the token, where it came from and
    /// the default store as JSON.
    #[arg(short, long)]
    json: bool,
}

/// Shows the token for the server and what the server says about it.
///
/// # Errors
///
/// When no server is named or set, there is no token for it, or the server
/// refuses the token or cannot be reached: what is known here is printed
/// first, then the error says why.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let config = &ctx.config;
    let server = login::server(args.server.as_deref(), config, "cases login status")?;
    let (source, answer) = store::status(&server, config)?;
    let whoami = match answer {
        Ok(whoami) => whoami,
        Err(error) => {
            if !args.json {
                writeln!(ctx.out, "{server}\n  from      {}", source.described())?;
            }
            return Err(error);
        }
    };
    let default_store = configured(&server, config)
        .then(|| store::default_store(config))
        .flatten();
    let default = stores::selected(
        &whoami.stores,
        &Which::Default,
        "",
        default_store.as_deref(),
    );
    if args.json {
        let shown = shown(&server, &whoami, &source, default.as_ref());
        serde_json::to_writer_pretty(&mut ctx.out, &shown)?;
        writeln!(ctx.out)?;
        return Ok(());
    }
    let unused = match &source {
        Source::Login(_) => None,
        // A login kept for the server that another token wins over.
        _ => credentials::read(&server).ok().flatten(),
    };
    let mut text = format!(
        "{server}\n  account   {} ({})\n  token     {:?}, for {}, {}\n  stores    {}\n  from      \
         {}\n  default   {}\n",
        whoami.account.handle,
        whoami.account.id,
        whoami.token.name,
        scope(&whoami),
        expires(&whoami),
        login::stores_text(&whoami),
        source.described(),
        default_text(default.as_ref(), default_store.as_deref()),
    );
    if let Some(login) = unused {
        let _ = writeln!(
            text,
            "  unused    the login cases login kept in {}: {} wins over it",
            login.place,
            source.described()
        );
    }
    ctx.out.write_all(text.as_bytes())?;
    Ok(())
}

/// Whether `server` is the configured one, whose `default-store` the config
/// gives.
fn configured(server: &Server, config: &Config) -> bool {
    store::server(config).ok().flatten().as_ref() == Some(server)
}

/// What the token opens: `every store`, or the stores' names and `only`.
fn scope(whoami: &Whoami) -> String {
    match &whoami.token.stores {
        None => "every store".to_owned(),
        Some(ids) if ids.is_empty() => "no store".to_owned(),
        Some(ids) => {
            let names: Vec<&str> = ids
                .iter()
                .map(|id| {
                    whoami
                        .stores
                        .iter()
                        .find(|store| store.id == *id)
                        .map_or(id.as_str(), |store| store.name.as_str())
                })
                .collect();
            format!("{} only", names.join(", "))
        }
    }
}

/// When the token expires, as its date: the server moves it with each use.
fn expires(whoami: &Whoami) -> String {
    match &whoami.token.expires {
        Some(time) => format!("expires {}", time.split('T').next().unwrap_or(time)),
        None => "does not expire".to_owned(),
    }
}

/// The default store, and which of C4's rules gave it.
fn default_text(default: Option<&Selected>, default_store: Option<&str>) -> String {
    match (default, default_store) {
        (Some(selected), _) => {
            let name = selected.store.name.as_deref().unwrap_or(&selected.store.id);
            let why = match selected.from {
                "default-store" => "the config file's default-store",
                "account" => "the account's",
                _ => "the only store",
            };
            format!("{name} ({why})")
        }
        (None, Some(text)) => {
            format!("none: the config file's default-store, {text}, is not a store the token opens")
        }
        (None, None) => "none: name a store with --store".to_owned(),
    }
}

/// The JSON of `cases login status --json`.
fn shown(server: &Server, whoami: &Whoami, source: &Source, default: Option<&Selected>) -> Value {
    let stores: Vec<Value> = whoami
        .stores
        .iter()
        .map(|store| json!({"id": store.id, "name": store.name, "default": store.default}))
        .collect();
    json!({
        "server": server.as_str(),
        "account": {"id": whoami.account.id, "handle": whoami.account.handle},
        "token": {
            "name": whoami.token.name,
            "stores": whoami.token.stores,
            "expires": whoami.token.expires,
        },
        "stores": stores,
        "from": source.name(),
        "default": default.map(|selected| json!({
            "id": selected.store.id,
            "name": selected.store.name,
            "from": selected.from,
        })),
    })
}
