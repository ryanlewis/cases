//! `cases stores`: the account's stores on the server, and which one
//! commands use (ST-11, AC-04, SV-01). It always reads the account afresh
//! and keeps the list, so it is also how to refresh the list by hand.

use std::io::Write;
use std::path::PathBuf;

use cases_store_remote::{Server, StoreInfo, Whoami};
use serde_json::{Value, json};

use crate::config;
use crate::ctx::Ctx;
use crate::error::Error;
use crate::store::{self, Selection, StoreRef, Which};

/// The command's arguments.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    /// Print the account, the token, the stores and the store in use as
    /// JSON.
    #[arg(short, long)]
    json: bool,
}

/// The store commands use now, and where that came from: `flag`, `env`,
/// `config`, `default-store`, `account` or `only`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Selected {
    pub(crate) store: StoreRef,
    pub(crate) from: &'static str,
}

/// Lists the account's stores on the server, marking the one in use.
///
/// # Errors
///
/// When no server is set, there is no token for it, or the server cannot be
/// read or refuses the token.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let flag = ctx.store_flag.as_deref();
    let config = &ctx.config;
    let (server, which, file) = match store::select(flag, config)? {
        Selection::Remote { server, which } => (server, Some(which), None),
        Selection::Forward(forward) => {
            return Err(Error::new(format!(
                "the store is forwarded to {forward}, which has no account's stores"
            )));
        }
        Selection::Local(path) => match store::server(config)? {
            // A path wins over the server (ST-11): the server is listed,
            // and the last line says the file is the store.
            Some(server) => (server, None, Some(path)),
            None => {
                return Err(Error::new(format!(
                    "no server is set ({} or the config's server); the store is the local file {}",
                    config::SERVER_ENV,
                    path.display()
                )));
            }
        },
    };
    let whoami = store::account(&server, config)?;
    let default_store = store::default_store(config);
    let selected = which.and_then(|which| {
        let source = config::store_text(flag, config).map_or("", |(_, source)| source);
        selected(&whoami.stores, &which, source, default_store.as_deref())
    });
    if args.json {
        let shown = shown(&server, &whoami, selected.as_ref());
        serde_json::to_writer_pretty(&mut ctx.out, &shown)?;
        writeln!(ctx.out)?;
    } else {
        let text = text(
            &server,
            &whoami,
            selected.as_ref(),
            default_store.as_deref(),
            file,
        );
        ctx.out.write_all(text.as_bytes())?;
    }
    Ok(())
}

/// The store `which` names in `stores`, as a command resolves it, with
/// where it came from. `source` is where the store's text came from, and
/// `default_store` the config's `default-store`. None when there is no
/// single store to use: a name the list does not have, or no store named,
/// no default and not one store.
pub(crate) fn selected(
    stores: &[StoreInfo],
    which: &Which,
    source: &'static str,
    default_store: Option<&str>,
) -> Option<Selected> {
    let of = |store: &StoreInfo| StoreRef {
        id: store.id.clone(),
        name: Some(store.name.clone()),
    };
    let (which, from) = match (which, default_store) {
        (Which::Default, Some(text)) => (Which::of(text), "default-store"),
        (which, _) => (which.clone(), source),
    };
    match which {
        // An id is used whether or not the list has it.
        Which::Id(id) => {
            let name = stores
                .iter()
                .find(|store| store.id == id)
                .map(|store| store.name.clone());
            Some(Selected {
                store: StoreRef { id, name },
                from,
            })
        }
        Which::Name(name) => stores
            .iter()
            .find(|store| store.name == name)
            .map(|store| Selected {
                store: of(store),
                from,
            }),
        Which::Default => {
            if let Some(store) = stores.iter().find(|store| store.default) {
                return Some(Selected {
                    store: of(store),
                    from: "account",
                });
            }
            match stores {
                [store] => Some(Selected {
                    store: of(store),
                    from: "only",
                }),
                _ => None,
            }
        }
    }
}

/// The JSON of `cases stores --json`.
fn shown(server: &Server, whoami: &Whoami, selected: Option<&Selected>) -> Value {
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
        "selected": selected.map(|selected| json!({
            "id": selected.store.id,
            "name": selected.store.name,
            "from": selected.from,
        })),
    })
}

/// The text of `cases stores`: the account, the token, and a table of the
/// stores, `*` on the one in use, with a last line when the store is a file
/// that wins over the server.
fn text(
    server: &Server,
    whoami: &Whoami,
    selected: Option<&Selected>,
    default_store: Option<&str>,
    file: Option<PathBuf>,
) -> String {
    let token = &whoami.token;
    let scope = match &token.stores {
        None => "all stores".to_owned(),
        Some(ids) if ids.is_empty() => "no stores".to_owned(),
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
    };
    let expires = match &token.expires {
        Some(time) => format!("expires {}", time.split('T').next().unwrap_or(time)),
        None => "does not expire".to_owned(),
    };
    let mut lines = vec![
        format!(
            "account {} ({}) on {server}",
            whoami.account.handle, whoami.account.id
        ),
        format!("token {} ({scope}), {expires}", token.name),
        String::new(),
    ];
    if whoami.stores.is_empty() {
        lines.push("The token opens no stores.".to_owned());
    } else {
        let mut rows = vec![("NAME", "ID", String::from("DEFAULT"), false)];
        for store in &whoami.stores {
            let mut default = Vec::new();
            if store.default {
                default.push("account");
            }
            if default_store.is_some_and(|text| text == store.name || text == store.id) {
                default.push("config");
            }
            let used = selected.is_some_and(|selected| selected.store.id == store.id);
            rows.push((
                store.name.as_str(),
                store.id.as_str(),
                default.join(", "),
                used,
            ));
        }
        let name = rows
            .iter()
            .map(|row| row.0.chars().count())
            .max()
            .unwrap_or(0);
        let id = rows
            .iter()
            .map(|row| row.1.chars().count())
            .max()
            .unwrap_or(0);
        for (n, i, default, used) in rows {
            let mark = if used { '*' } else { ' ' };
            let line = format!("{mark} {n:name$}  {i:id$}  {default}");
            lines.push(line.trim_end().to_owned());
        }
    }
    if let Some(file) = file {
        lines.push(String::new());
        lines.push(format!(
            "The store is the file {}, which wins over the server {server}.",
            file.display()
        ));
    }
    lines.join("\n") + "\n"
}
