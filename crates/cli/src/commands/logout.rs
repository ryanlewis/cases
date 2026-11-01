//! `cases logout`: revokes the login `cases login` kept for a server, then
//! forgets it here (AC-03).
//!
//! The server is the argument, else `CASES_SERVER`, else the config file's
//! `server`. The token is forgotten only once the server has revoked it, or
//! says it already let it go (401 or 404); otherwise it is kept, so that a
//! live token is never left that nobody holds. `--forget` forgets it with
//! no request, for a server or account that is gone. `--file PATH` revokes
//! the token in PATH and removes the file instead.
//!
//! Logout touches only the stored login, or the file named: never
//! `CASES_TOKEN`, a token file the config names, or the config. It never
//! prints a token.

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use cases_store_remote::{Client, Error as RemoteError, Server, Token};

use crate::commands::login::{self, VERSION};
use crate::config::{self, Config};
use crate::credentials::{self, Login};
use crate::ctx::{self, Ctx};
use crate::error::Error;
use crate::store::{self, TOKEN_ENV};

/// The most a token file holds: a token is 40 characters, and a line.
const FILE_MAX: u64 = 1024;

/// The arguments of `cases logout`.
#[derive(Debug, clap::Args)]
pub(crate) struct Args {
    #[arg(
        value_name = "SERVER",
        help = "The server, https://host[:port]. Default: $CASES_SERVER, then the config file's \
                server."
    )]
    server: Option<String>,

    /// Forget the stored login, and the account's kept list of stores,
    /// with no request: for a server or an account that is gone. The token
    /// stays live on the server until it is revoked on its token page, or
    /// goes 90 days unused.
    #[arg(long)]
    forget: bool,

    /// Revoke the token in PATH, then remove the file, and leave the
    /// stored login alone. The file stays when the revoke fails. With
    /// --forget, only remove the file.
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
}

/// Revokes and forgets the stored login for the server, or the token in
/// the file.
///
/// # Errors
///
/// When no server is named or set, there is no stored login (or the file is
/// not a token's), it cannot be read or forgotten, or the server cannot
/// revoke it: then nothing is forgotten.
pub(crate) fn run(args: &Args, ctx: &mut Ctx) -> Result<(), Error> {
    let server = login::server(args.server.as_deref(), &ctx.config, "cases logout")?;
    match &args.file {
        Some(path) => file(&server, path, args.forget, ctx),
        None => stored(&server, args.forget, ctx),
    }
}

/// `cases logout [--forget]`: the stored login.
fn stored(server: &Server, forget: bool, ctx: &mut Ctx) -> Result<(), Error> {
    let Some(login) = credentials::read(server)? else {
        let also = other_token(server, &ctx.config).map_or(String::new(), |other| {
            format!(
                ". Commands use {other}, which logout never touches: revoke it at {server}/tokens"
            )
        });
        return Err(Error::new(format!(
            "not signed in to {server}: there is no stored login{also}"
        )));
    };
    if forget {
        forget_login(server, &login)?;
        writeln!(
            ctx.out,
            "Forgot the login for {server}, kept in {}. Its token stays live on the server until \
             it is revoked at {server}/tokens, or goes 90 days unused.",
            login.place
        )?;
    } else {
        let client = Client::new(server.clone(), login.token.clone()).with_version(VERSION);
        let revoked = revoke(&client, &login.token, login.account.as_deref()).map_err(|error| {
            Error::new(format!(
                "{server}: the token is not revoked: {error}. Run cases logout again, or cases \
                 logout --forget to forget it here only"
            ))
        })?;
        forget_login(server, &login)?;
        match revoked {
            Revoked::Now(Some(handle)) => {
                writeln!(ctx.out, "Signed out of {server} ({handle}).")?;
            }
            Revoked::Now(None) => writeln!(ctx.out, "Signed out of {server}.")?,
            Revoked::Already => writeln!(
                ctx.out,
                "Signed out of {server}: the server had already let the token go."
            )?,
        }
    }
    if let Some(other) = other_token(server, &ctx.config) {
        ctx::warn(format_args!(
            "commands use {other} for {server}, which logout leaves alone"
        ));
    }
    Ok(())
}

/// Forgets `login`: the keychain's item and the file's entry, and the kept
/// list of stores when it was read with its token.
fn forget_login(server: &Server, login: &Login) -> Result<(), Error> {
    credentials::remove(server)?;
    store::forget_account(server, &login.token)?;
    Ok(())
}

/// `cases logout --file PATH [--forget]`: the token in a file.
fn file(server: &Server, path: &Path, forget: bool, ctx: &mut Ctx) -> Result<(), Error> {
    let (path, token) = token_file(path)?;
    let shown = path.display();
    if forget {
        remove(&path)?;
        writeln!(
            ctx.out,
            "Removed {shown}. Its token stays live on the server until it is revoked at \
             {server}/tokens, or goes 90 days unused."
        )?;
    } else {
        let client = Client::new(server.clone(), token.clone()).with_version(VERSION);
        let revoked = revoke(&client, &token, None).map_err(|error| {
            Error::new(format!(
                "{server}: the token in {shown} is not revoked: {error}. The file is kept: run \
                 cases logout --file again, or add --forget to remove the file only"
            ))
        })?;
        remove(&path)?;
        // A kept list read with this token is no use now.
        let _ = store::forget_account(server, &token);
        match revoked {
            Revoked::Now(Some(handle)) => writeln!(
                ctx.out,
                "Revoked the token in {shown} for {handle} on {server}, and removed the file."
            )?,
            Revoked::Now(None) => writeln!(
                ctx.out,
                "Revoked the token in {shown} on {server}, and removed the file."
            )?,
            Revoked::Already => writeln!(
                ctx.out,
                "Removed {shown}: {server} had already let its token go."
            )?,
        }
    }
    if let Some((named, from)) = config::token_file_text(&ctx.config)
        && std::path::absolute(config::expand_home(&named)).is_ok_and(|named| named == path)
    {
        let from = if from == "env" {
            config::TOKEN_FILE_ENV
        } else {
            "the config file's token-file"
        };
        ctx::warn(format_args!(
            "{from} still names {shown}, so commands cannot read their token: point it at \
             another file, or remove it"
        ));
    }
    Ok(())
}

/// The token in the file at `path`, and the file as an absolute path. Only
/// a regular file that holds a token, one line, is taken: a link, a
/// directory, or a file that holds anything else is refused, so that logout
/// never removes a file that is not a token's.
fn token_file(path: &Path) -> Result<(PathBuf, Token), Error> {
    let path = std::path::absolute(path)
        .map_err(|error| Error::new(format!("--file {} is not a path: {error}", path.display())))?;
    let shown = path.display();
    let refuse = |why: &str| Error::new(format!("{shown} {why}: logout removes only a token file"));
    let meta = fs::symlink_metadata(&path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => Error::new(format!("{shown} is not there")),
        _ => Error::new(format!("{shown} cannot be read: {error}")),
    })?;
    if meta.file_type().is_symlink() {
        return Err(refuse("is a symbolic link"));
    }
    if !meta.is_file() {
        return Err(refuse("is not a regular file"));
    }
    if meta.len() > FILE_MAX {
        return Err(refuse("does not hold a token"));
    }
    let mut text = String::new();
    fs::File::open(&path)
        .and_then(|file| file.take(FILE_MAX).read_to_string(&mut text))
        .map_err(|error| match error.kind() {
            io::ErrorKind::InvalidData => refuse("does not hold a token"),
            _ => Error::new(format!("{shown} cannot be read: {error}")),
        })?;
    let token = text
        .trim()
        .parse::<Token>()
        .map_err(|_| refuse("does not hold a token"))?;
    Ok((path, token))
}

/// Removes the token file at `path`.
fn remove(path: &Path) -> Result<(), Error> {
    fs::remove_file(path)
        .map_err(|error| Error::new(format!("{} cannot be removed: {error}", path.display())))
}

/// What the revoke came to.
enum Revoked {
    /// The server revoked the token: of the account with this handle, when
    /// it is known.
    Now(Option<String>),
    /// The server had already let it go: it refused the token (401:
    /// revoked, expired, or not of the account), or does not know the
    /// account (404).
    Already,
}

/// Revokes `token`, the one `client` sends, in its account: `account`
/// when the stored login has it, else the one the kept list names when it
/// was read with the token, else the one the server gives for it. The
/// revoke is safe to send twice, so the client retries it as a read.
///
/// # Errors
///
/// When the server cannot be reached, or fails, or limits requests past
/// the client's one wait: the token may still be live.
fn revoke(client: &Client, token: &Token, account: Option<&str>) -> Result<Revoked, RemoteError> {
    let kept = store::kept_account(client.server(), token);
    let (account, handle) = match (account, kept) {
        (Some(account), kept) => {
            let handle = kept
                .filter(|(id, _)| id == account)
                .map(|(_, handle)| handle);
            (account.to_owned(), handle)
        }
        (None, Some((id, handle))) => (id, Some(handle)),
        (None, None) => match client.whoami() {
            Ok(whoami) => (whoami.account.id, Some(whoami.account.handle)),
            Err(RemoteError::Auth { .. } | RemoteError::NoToken) => return Ok(Revoked::Already),
            Err(error) => return Err(error),
        },
    };
    match client.revoke(&account) {
        Ok(()) => Ok(Revoked::Now(handle)),
        Err(RemoteError::Auth { .. } | RemoteError::NoToken) => Ok(Revoked::Already),
        Err(error) => Err(error),
    }
}

/// The token commands use for `server` other than the stored login, which
/// logout never touches: `CASES_TOKEN`, or a token file. They go only to
/// the configured server, or to any when none is configured (SV-01).
fn other_token(server: &Server, config: &Config) -> Option<String> {
    let configured = store::server(config).ok().flatten();
    if configured.is_some_and(|configured| configured != *server) {
        return None;
    }
    if std::env::var(TOKEN_ENV).is_ok_and(|token| !token.trim().is_empty()) {
        return Some(TOKEN_ENV.to_owned());
    }
    config::token_file_text(config).map(|(file, from)| {
        let from = if from == "env" {
            config::TOKEN_FILE_ENV
        } else {
            "the config file's token-file"
        };
        format!(
            "the token file {} ({from})",
            config::expand_home(&file).display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_regular_file_that_holds_a_token_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let token = "csu_000000000000000000000000000000000000";
        let good = dir.path().join("agent.token");
        fs::write(&good, format!("{token}\n")).unwrap();
        let (path, read) = token_file(&good).unwrap();
        assert_eq!((path, read.expose()), (good.clone(), token));
        let other = dir.path().join("notes.txt");
        fs::write(&other, "not a token\n").unwrap();
        let big = dir.path().join("big.token");
        fs::write(&big, format!("{token}\n{}", "x".repeat(2000))).unwrap();
        let binary = dir.path().join("binary.token");
        fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        let link = dir.path().join("link.token");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&good, &link).unwrap();
        for (path, says) in [
            (&other, "does not hold a token"),
            (&big, "does not hold a token"),
            (&binary, "does not hold a token"),
            (&dir.path().to_path_buf(), "is not a regular file"),
            (&dir.path().join("none"), "is not there"),
            #[cfg(unix)]
            (&link, "is a symbolic link"),
        ] {
            let error = token_file(path).unwrap_err().to_string();
            assert!(error.contains(says), "{}: {error}", path.display());
        }
        // Nothing refused was removed.
        assert!(other.exists() && good.exists());
    }
}
