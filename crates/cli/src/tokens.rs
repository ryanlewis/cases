//! The token files of an HTTP forward: `store-tokens-file`, the tokens
//! `cases serve` takes on its store routes through a proxy, and
//! `forward-token-file`, the token an `http://` or `https://` forward
//! sends. Each holds tokens one to a line; blank lines and lines starting
//! `#` are skipped. A token is at least 32 letters, digits and `-._~+/=`,
//! as `openssl rand -hex 32` gives (see [`Token::forward`]).
//!
//! Nothing here prints a token, and no error holds one.

use std::fs;
use std::path::Path;

use cases_store_remote::Token;

use crate::config;
use crate::ctx;
use crate::error::Error;

/// The tokens of the file at `path`, named by `key` in its errors, such as
/// `the config file's store-tokens-file`. It warns when others than its
/// owner can read the file.
///
/// # Errors
///
/// When the file cannot be read, holds no token, or holds a line that is
/// not one.
pub(crate) fn read(path: &str, key: &str) -> Result<Vec<Token>, Error> {
    let path = config::expand_home(path);
    let text = fs::read_to_string(&path)
        .map_err(|error| Error::new(format!("{key}: cannot read {}: {error}", path.display())))?;
    warn_if_open(&path);
    let mut tokens = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let token = Token::forward(line).map_err(|error| {
            Error::new(format!(
                "{key}: line {} of {}: {error}",
                n + 1,
                path.display()
            ))
        })?;
        tokens.push(token);
    }
    if tokens.is_empty() {
        return Err(Error::new(format!(
            "{key}: {} holds no token; put one on a line of its own, such as openssl rand -hex \
             32 gives",
            path.display()
        )));
    }
    Ok(tokens)
}

/// Warns when others than the owner can read or write the file at `path`.
#[cfg(unix)]
fn warn_if_open(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = fs::metadata(path)
        && meta.permissions().mode() & 0o077 != 0
    {
        ctx::warn(format_args!(
            "{} can be read by others: make it private with chmod 600 {}",
            path.display(),
            path.display()
        ));
    }
}

/// Off unix, a file's mode says nothing to warn about.
#[cfg(not(unix))]
fn warn_if_open(_path: &Path) {}

/// The tokens serve's store routes take, kept so that none is shown: their
/// `Debug` gives only how many there are.
#[derive(Clone, Default)]
pub(crate) struct Tokens(Vec<Token>);

impl Tokens {
    pub(crate) fn new(tokens: Vec<Token>) -> Tokens {
        Tokens(tokens)
    }

    /// Whether `sent` is one of them. Each is compared in a time that
    /// depends only on the lengths, and all are compared, so that the time
    /// says nothing of which one, or how much of one, matched.
    pub(crate) fn has(&self, sent: &[u8]) -> bool {
        self.0.iter().fold(false, |found, token| {
            same(sent, token.expose().as_bytes()) | found
        })
    }
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tokens({} kept)", self.0.len())
    }
}

/// Whether `a` and `b` are the same, in a time that depends only on their
/// lengths.
pub(crate) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str) -> Token {
        Token::forward(text).unwrap()
    }

    #[test]
    fn a_file_s_tokens_are_its_lines_but_blanks_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens");
        let one = "a".repeat(32);
        let two = "0f".repeat(32);
        fs::write(&path, format!("# serve's\n\n  {one}  \n{two}\n")).unwrap();
        let got = read(path.to_str().unwrap(), "key").unwrap();
        let got: Vec<&str> = got.iter().map(Token::expose).collect();
        assert_eq!(got, [one.as_str(), two.as_str()]);
    }

    #[test]
    fn a_file_with_no_token_or_a_bad_one_is_refused_without_showing_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens");
        fs::write(&path, "# none yet\n").unwrap();
        let error = read(path.to_str().unwrap(), "key").unwrap_err().to_string();
        assert!(error.contains("holds no token"), "{error}");
        fs::write(&path, "secretsecret\n").unwrap();
        let error = read(path.to_str().unwrap(), "key").unwrap_err().to_string();
        assert!(
            error.contains("line 1") && !error.contains("secretsecret"),
            "{error}"
        );
        let error = read(dir.path().join("none").to_str().unwrap(), "key")
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("key: cannot read"), "{error}");
    }

    #[test]
    fn a_token_is_had_only_whole_and_never_shown() {
        let kept = "b".repeat(40);
        let tokens = Tokens::new(vec![token(&"a".repeat(32)), token(&kept)]);
        assert!(tokens.has(kept.as_bytes()));
        assert!(!tokens.has(&kept.as_bytes()[..39]));
        assert!(!tokens.has(format!("{kept}b").as_bytes()));
        assert!(!tokens.has(b""));
        assert!(!Tokens::default().has(kept.as_bytes()));
        assert_eq!(format!("{tokens:?}"), "Tokens(2 kept)");
    }
}
