//! The config file, and where the store is (SV-01).
//!
//! The config file supplies defaults for flags. Precedence is flag, then
//! environment variable, then config file, then the built-in default. The file
//! is optional: one that is not there supplies nothing.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// The environment variable that names the config file.
const CONFIG_ENV: &str = "CASES_CONFIG";

/// The environment variable that names the store.
const STORE_ENV: &str = "CASES_STORE";

/// The environment variable that names the server of remote stores.
pub(crate) const SERVER_ENV: &str = "CASES_SERVER";

/// The environment variable that forwards every command to a store on
/// another machine, `ssh://host` or `exec:<command>`, before the config
/// file's `forward`.
pub(crate) const FORWARD_ENV: &str = "CASES_FORWARD";

/// The environment variable that names the file holding the token for the
/// server, before the config file's `token-file`.
pub(crate) const TOKEN_FILE_ENV: &str = "CASES_TOKEN_FILE";

/// The environment variable that names the file holding the token an
/// `http://` or `https://` forward sends, before the config file's
/// `forward-token-file`.
pub(crate) const FORWARD_TOKEN_FILE_ENV: &str = "CASES_FORWARD_TOKEN_FILE";

/// The keys the file may hold. `store` is the store, `forward` the machine
/// whose store every command is forwarded to over ssh, a command or HTTP,
/// `forward-token-file` the file of the token an HTTP forward sends, and
/// `server`,
/// `default-store` and `token-file` the server of remote stores, the store
/// to use there when none is named, and the file that holds the token for
/// it (SV-01); the others seed flags of single commands: `listen` and
/// `no-open` serve's, `name` the `--as` of the human's commands and serve,
/// `prune-age` prune's `--age`, and `mood` wait's `--mood`; `tui-stats`
/// whether `cases tui` shows its stats strip, `auto`, `text` or `off`;
/// `briefing-close-after` is how long a briefing stays picked up before serve
/// and `sweep --stale-briefings` close it as expired, or `off`. `allow-hosts`
/// and `tailscale-logins`, lists, are serve's alone: the hosts it answers
/// through a proxy such as `tailscale serve`, and the Tailscale logins a
/// request through one of them must come from. `store-tokens-file` is
/// serve's too: the file of the tokens its store routes take through a
/// proxy.
pub(crate) const KEYS: [&str; 16] = [
    "store",
    "forward",
    "forward-token-file",
    "server",
    "default-store",
    "token-file",
    "listen",
    "no-open",
    "allow-hosts",
    "tailscale-logins",
    "store-tokens-file",
    "name",
    "prune-age",
    "briefing-close-after",
    "mood",
    "tui-stats",
];

/// The key that also takes a TOML boolean.
const BOOL_KEY: &str = "no-open";

/// The keys that take a list of strings.
const LIST_KEYS: [&str; 2] = ["allow-hosts", "tailscale-logins"];

/// The config file in use, and what it holds.
#[derive(Debug, Default)]
pub(crate) struct Config {
    /// Where the file is, or would be. Empty when it cannot be found.
    pub(crate) path: PathBuf,
    /// Why the file cannot be used, when it cannot. It then supplies nothing,
    /// and every command but `cases config` stops with this error.
    pub(crate) error: Option<Error>,
    /// Each key the file sets, with its value as text. A boolean is `true`
    /// or `false`, and a list its items joined by `, `.
    values: BTreeMap<&'static str, String>,
    /// Each list key the file sets, with its items.
    lists: BTreeMap<&'static str, Vec<String>>,
}

impl Config {
    /// The value the file gives `key`, one of [`KEYS`], if it gives one.
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The items the file gives `key`, one of the list keys: none when it
    /// gives none.
    pub(crate) fn list(&self, key: &str) -> &[String] {
        self.lists.get(key).map_or(&[], Vec::as_slice)
    }
}

/// Loads the config file: the one `flag` names, else the one `CASES_CONFIG`
/// names, else `$XDG_CONFIG_HOME/cases/config.toml` or
/// `~/.config/cases/config.toml`. An empty flag or variable counts as not
/// given. The file is optional; one that cannot be used gives a `Config` with
/// its [`error`](Config::error) set and no values.
pub(crate) fn load(flag: Option<&str>) -> Config {
    let named = flag
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .or_else(|| env::var(CONFIG_ENV).ok().filter(|path| !path.is_empty()));
    let path = match named {
        Some(path) => expand_home(&path),
        None => match config_dir() {
            Some(dir) => dir.join("config.toml"),
            None => {
                return Config {
                    error: Some(Error::new(
                        "cannot find the config file: neither $XDG_CONFIG_HOME nor $HOME is set",
                    )),
                    ..Config::default()
                };
            }
        },
    };
    match read(&path) {
        Ok((values, lists)) => Config {
            path,
            error: None,
            values,
            lists,
        },
        Err(message) => Config {
            error: Some(Error::new(format!(
                "config file {}: {message}",
                path.display()
            ))),
            path,
            ..Config::default()
        },
    }
}

/// What the file `read` gives: each key as text, and each list key's items.
type Read = (
    BTreeMap<&'static str, String>,
    BTreeMap<&'static str, Vec<String>>,
);

/// The keys the file at `path` sets, or why it cannot be used. A file that is
/// not there sets none.
fn read(path: &Path) -> Result<Read, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Read::default()),
        Err(error) => return Err(format!("cannot read: {error}")),
    };
    let table: toml::Table = text
        .parse()
        .map_err(|error: toml::de::Error| format!("invalid TOML: {}", toml_error(&text, &error)))?;
    let mut values = BTreeMap::new();
    let mut lists = BTreeMap::new();
    for (name, value) in table {
        let Some(key) = KEYS.into_iter().find(|key| *key == name) else {
            return Err(format!(
                "unknown key {name:?} (valid keys: {})",
                KEYS.join(", ")
            ));
        };
        if LIST_KEYS.contains(&key) {
            let items = list(key, value)?;
            if !items.is_empty() {
                values.insert(key, items.join(", "));
                lists.insert(key, items);
            }
            continue;
        }
        let value = match value {
            toml::Value::String(text) if key == BOOL_KEY && text != "true" && text != "false" => {
                return Err(format!("key {key:?} must be true or false, got {text:?}"));
            }
            toml::Value::String(text) => text,
            toml::Value::Boolean(value) if key == BOOL_KEY => value.to_string(),
            other if key == BOOL_KEY => {
                return Err(format!(
                    "key {key:?} must be a boolean or a string, got {}",
                    type_name(&other)
                ));
            }
            other => {
                return Err(format!(
                    "key {key:?} must be a string, got {}",
                    type_name(&other)
                ));
            }
        };
        values.insert(key, value);
    }
    Ok((values, lists))
}

/// The items of the list key `key`: an array of strings, each trimmed and
/// not empty. A host is lowercased, as `Host` is compared, and must be a
/// bare `host[:port]`, without a scheme or a path.
fn list(key: &str, value: toml::Value) -> Result<Vec<String>, String> {
    let items = match value {
        toml::Value::Array(items) => items,
        other => {
            return Err(format!(
                "key {key:?} must be an array of strings, got {}",
                type_name(&other)
            ));
        }
    };
    items
        .into_iter()
        .map(|item| {
            let text = match item {
                toml::Value::String(text) => text,
                other => {
                    return Err(format!(
                        "key {key:?} must be an array of strings, got an array holding a {}",
                        type_name(&other)
                    ));
                }
            };
            let text = text.trim();
            if text.is_empty() {
                return Err(format!("key {key:?} holds an empty string"));
            }
            if key != "allow-hosts" {
                return Ok(text.to_owned());
            }
            if text.contains(|c: char| matches!(c, '/' | '@' | '?' | '#') || c.is_whitespace()) {
                return Err(format!(
                    "key {key:?} takes hosts as the browser sends them, such as \
                     \"devbox.example.ts.net:7337\", got {text:?}"
                ));
            }
            Ok(text.to_ascii_lowercase())
        })
        .collect()
}

/// A TOML error on one line, with the line and column it is at.
fn toml_error(text: &str, error: &toml::de::Error) -> String {
    let message = error.message().trim();
    let Some(span) = error.span() else {
        return message.to_owned();
    };
    let before = text.get(..span.start).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |line| line.chars().count())
        + 1;
    format!("line {line}, column {column}: {message}")
}

/// The TOML type of `value`, as the errors name it.
fn type_name(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) | toml::Value::Float(_) => "number",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}

/// The address serve listens on when nothing else names one.
pub(crate) const DEFAULT_LISTEN: &str = "127.0.0.1:7337";

/// The value `key`, one of [`KEYS`], has when nothing sets it. The store's
/// is empty when neither `$XDG_DATA_HOME` nor `$HOME` is set.
pub(crate) fn default(key: &str) -> String {
    match key {
        "store" => default_store().map_or_else(String::new, |path| path.display().to_string()),
        "listen" => DEFAULT_LISTEN.to_owned(),
        "no-open" => "false".to_owned(),
        "prune-age" => "720h".to_owned(),
        crate::briefings::KEY => crate::briefings::DEFAULT.to_owned(),
        "mood" => "eager".to_owned(),
        "tui-stats" => "auto".to_owned(),
        _ => String::new(),
    }
}

/// Each key's value once the environment and the file are read, and where it
/// came from: `env`, `config` or `default`. It is what a command uses when no
/// flag gives the value, and what `config show` prints.
///
/// With a server set, a store that is not a path is a store's name or id in
/// the account, shown as it is given, and with no store given, the store is
/// the account's default. The token itself is never among them.
pub(crate) fn settings(config: &Config) -> Vec<(&'static str, String, &'static str)> {
    let server = server_text(config);
    KEYS.into_iter()
        .map(|key| {
            let (value, source) = match key {
                "store" => match store_text(None, config) {
                    Some((text, source)) if server.is_none() || is_path(&text) => {
                        (expand_home(&text).display().to_string(), source)
                    }
                    Some((text, source)) => (text, source),
                    None if server.is_some() => ("the account's default".to_owned(), "default"),
                    None => (default(key), "default"),
                },
                "forward" => forward_text(config).unwrap_or_else(|| (String::new(), "default")),
                "server" => server.clone().unwrap_or_else(|| (String::new(), "default")),
                "token-file" => match token_file_text(config) {
                    Some((text, source)) => (expand_home(&text).display().to_string(), source),
                    None => (default(key), "default"),
                },
                "forward-token-file" => match forward_token_file_text(config) {
                    Some((text, source)) => (expand_home(&text).display().to_string(), source),
                    None => (default(key), "default"),
                },
                _ => match config.get(key) {
                    Some(value) => (value.to_owned(), "config"),
                    None => (default(key), "default"),
                },
            };
            (key, value, source)
        })
        .collect()
}

/// The text that names the store: `flag`, else `CASES_STORE`, else the
/// config file's `store`, with where it came from: `flag`, `env` or
/// `config`. A value that is empty or only white space counts as not given,
/// and the text is trimmed.
pub(crate) fn store_text(flag: Option<&str>, config: &Config) -> Option<(String, &'static str)> {
    [
        (flag.map(str::to_owned), "flag"),
        (env::var(STORE_ENV).ok(), "env"),
        (config.get("store").map(str::to_owned), "config"),
    ]
    .into_iter()
    .filter_map(|(text, source)| Some((text?.trim().to_owned(), source)))
    .find(|(text, _)| !text.is_empty())
}

/// The server of remote stores, as text: `CASES_SERVER`, else the config
/// file's `server`, with where it came from, `env` or `config`. Empty
/// counts as not set, which means the store is local.
pub(crate) fn server_text(config: &Config) -> Option<(String, &'static str)> {
    [
        (env::var(SERVER_ENV).ok(), "env"),
        (config.get("server").map(str::to_owned), "config"),
    ]
    .into_iter()
    .filter_map(|(text, source)| Some((text?.trim().to_owned(), source)))
    .find(|(text, _)| !text.is_empty())
}

/// The machine whose store every command is forwarded to, as text:
/// `CASES_FORWARD`, else the config file's `forward`, with where it came
/// from, `env` or `config`. Empty counts as not set, which means the store
/// is not forwarded.
pub(crate) fn forward_text(config: &Config) -> Option<(String, &'static str)> {
    [
        (env::var(FORWARD_ENV).ok(), "env"),
        (config.get("forward").map(str::to_owned), "config"),
    ]
    .into_iter()
    .filter_map(|(text, source)| Some((text?.trim().to_owned(), source)))
    .find(|(text, _)| !text.is_empty())
}

/// The file that holds the token for the server, as text:
/// `CASES_TOKEN_FILE`, else the config file's `token-file`, with where it
/// came from, `env` or `config`. Empty counts as not set.
pub(crate) fn token_file_text(config: &Config) -> Option<(String, &'static str)> {
    [
        (env::var(TOKEN_FILE_ENV).ok(), "env"),
        (config.get("token-file").map(str::to_owned), "config"),
    ]
    .into_iter()
    .filter_map(|(text, source)| Some((text?.trim().to_owned(), source)))
    .find(|(text, _)| !text.is_empty())
}

/// The file that holds the token an HTTP forward sends, as text:
/// `CASES_FORWARD_TOKEN_FILE`, else the config file's `forward-token-file`,
/// with where it came from, `env` or `config`. Empty counts as not set.
pub(crate) fn forward_token_file_text(config: &Config) -> Option<(String, &'static str)> {
    [
        (env::var(FORWARD_TOKEN_FILE_ENV).ok(), "env"),
        (
            config.get("forward-token-file").map(str::to_owned),
            "config",
        ),
    ]
    .into_iter()
    .filter_map(|(text, source)| Some((text?.trim().to_owned(), source)))
    .find(|(text, _)| !text.is_empty())
}

/// Whether the store's text is a path: it has a `/` or ends in `.db`
/// (ST-11). A path names a local store even with a server set.
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "ST-11 reads the text as written: `work.DB` is a store's name with a server set"
)]
pub(crate) fn is_path(text: &str) -> bool {
    text.contains('/') || text.ends_with(".db")
}

/// The default store's file, `$XDG_DATA_HOME/cases/cases.db` or
/// `~/.local/share/cases/cases.db`.
///
/// # Errors
///
/// When neither `$XDG_DATA_HOME` nor `$HOME` is set.
pub(crate) fn default_store_path() -> Result<PathBuf, Error> {
    default_store()
        .ok_or_else(|| Error::new("--store or CASES_STORE must name the case store file"))
}

/// `$XDG_DATA_HOME/cases/cases.db` or `~/.local/share/cases/cases.db`, or
/// `None` when neither variable is set.
fn default_store() -> Option<PathBuf> {
    base_dir("XDG_DATA_HOME", &[".local", "share"]).map(|base| base.join("cases").join("cases.db"))
}

/// The config directory of cases, `$XDG_CONFIG_HOME/cases` or
/// `~/.config/cases`, or `None` when neither variable is set. The config
/// file is there unless `--config` or `CASES_CONFIG` names another; the
/// credentials file always is.
pub(crate) fn config_dir() -> Option<PathBuf> {
    base_dir("XDG_CONFIG_HOME", &[".config"]).map(|base| base.join("cases"))
}

/// The directory `$var` names, else `$HOME` joined with `under`, or `None`
/// when neither is set.
fn base_dir(var: &str, under: &[&str]) -> Option<PathBuf> {
    if let Some(dir) = env::var_os(var).filter(|dir| !dir.is_empty()) {
        return Some(dir.into());
    }
    let mut dir = home()?;
    dir.extend(under);
    Some(dir)
}

/// `$HOME`, when it is set and not empty.
fn home() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// `path` with a leading `~` replaced by `$HOME`, so that a path can be
/// written that way in the config file. `~user` is left as it is, and so is
/// every path when `$HOME` is not set.
pub(crate) fn expand_home(path: &str) -> PathBuf {
    let rest = match path.strip_prefix('~') {
        Some("") => "",
        Some(rest) if rest.starts_with('/') => &rest[1..],
        _ => return PathBuf::from(path),
    };
    match home() {
        Some(home) if rest.is_empty() => home,
        Some(home) => home.join(rest),
        None => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Result<BTreeMap<&'static str, String>, String> {
        read_text(text).map(|(values, _)| values)
    }

    fn read_text(text: &str) -> Result<Read, String> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, text).unwrap();
        read(&path)
    }

    #[test]
    fn a_file_that_is_not_there_sets_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("none.toml")), Ok(Read::default()));
    }

    #[test]
    fn a_directory_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let error = read(dir.path()).unwrap_err();
        assert!(error.starts_with("cannot read: "), "{error}");
    }

    #[test]
    fn every_key_reads() {
        let values = config(
            "store = \"~/cases.db\"\nforward = \"ssh://devbox\"\nserver = \"https://cases.sh\"\n\
             forward-token-file = \"~/forward-token\"\n\
             store-tokens-file = \"~/store-tokens\"\n\
             default-store = \"me\"\ntoken-file = \"~/token\"\n\
             listen = \"127.0.0.1:1\"\nno-open = true\n\
             allow-hosts = [\"Devbox.example.ts.net:7337\"]\n\
             tailscale-logins = [\"ryan@github\", \"sam@github\"]\n\
             name = \"Ryan\"\nprune-age = \"2160h\"\nbriefing-close-after = \"off\"\n\
             mood = \"steady\"\ntui-stats = \"off\"\n",
        )
        .unwrap();
        assert_eq!(values.len(), KEYS.len());
        assert_eq!(values["store"], "~/cases.db");
        assert_eq!(values["forward"], "ssh://devbox");
        assert_eq!(values["no-open"], "true");
        assert_eq!(values["tailscale-logins"], "ryan@github, sam@github");
        assert_eq!(config("no-open = \"false\"\n").unwrap()["no-open"], "false");
    }

    #[test]
    fn a_key_of_the_wrong_type_is_refused() {
        for (text, want) in [
            (
                "stor = \"x\"\n",
                "unknown key \"stor\" (valid keys: store, forward, forward-token-file, server, \
                 default-store, token-file, listen, no-open, allow-hosts, tailscale-logins, \
                 store-tokens-file, name, prune-age, briefing-close-after, mood, tui-stats)",
            ),
            ("store = 1\n", "key \"store\" must be a string, got number"),
            (
                "name = true\n",
                "key \"name\" must be a string, got boolean",
            ),
            (
                "listen = [\"a\"]\n",
                "key \"listen\" must be a string, got array",
            ),
            (
                "no-open = 1\n",
                "key \"no-open\" must be a boolean or a string, got number",
            ),
            (
                "no-open = \"yes\"\n",
                "key \"no-open\" must be true or false, got \"yes\"",
            ),
        ] {
            assert_eq!(config(text).unwrap_err(), want, "{text}");
        }
    }

    /// A list key takes an array of strings, a host lowercased and bare,
    /// and an empty array sets nothing.
    #[test]
    fn a_list_key_takes_an_array_of_strings() {
        let (values, lists) = read_text(
            "allow-hosts = [\" Devbox.Example.ts.net:7337 \", \"[fd7a::1]:7337\"]\n\
             tailscale-logins = [\"ryan@github\"]\n",
        )
        .unwrap();
        assert_eq!(
            lists["allow-hosts"],
            ["devbox.example.ts.net:7337", "[fd7a::1]:7337"]
        );
        assert_eq!(
            values["allow-hosts"],
            "devbox.example.ts.net:7337, [fd7a::1]:7337"
        );
        assert_eq!(lists["tailscale-logins"], ["ryan@github"]);
        assert_eq!(read_text("allow-hosts = []\n").unwrap(), Read::default());
        for (text, want) in [
            (
                "allow-hosts = \"devbox\"\n",
                "key \"allow-hosts\" must be an array of strings, got string",
            ),
            (
                "tailscale-logins = [1]\n",
                "key \"tailscale-logins\" must be an array of strings, got an array \
                 holding a number",
            ),
            (
                "tailscale-logins = [\" \"]\n",
                "key \"tailscale-logins\" holds an empty string",
            ),
            (
                "allow-hosts = [\"https://devbox.example.ts.net\"]\n",
                "key \"allow-hosts\" takes hosts as the browser sends them, such as \
                 \"devbox.example.ts.net:7337\", got \"https://devbox.example.ts.net\"",
            ),
        ] {
            assert_eq!(config(text).unwrap_err(), want, "{text}");
        }
    }

    #[test]
    fn a_toml_error_names_its_line_and_column() {
        let error = config("store = \"a\"\nlisten = \n").unwrap_err();
        assert!(
            error.starts_with("invalid TOML: line 2, column "),
            "{error}"
        );
        assert!(!error.contains('\n'), "{error}");
    }

    #[test]
    fn a_leading_tilde_is_the_home_directory() {
        let Some(home) = home() else { return };
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/cases.db"), home.join("cases.db"));
        assert_eq!(
            expand_home("~ryan/cases.db"),
            PathBuf::from("~ryan/cases.db")
        );
        assert_eq!(expand_home("/tmp/~/x"), PathBuf::from("/tmp/~/x"));
    }
}
