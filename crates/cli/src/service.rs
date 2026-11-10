//! `cases serve` as a user service that starts at login and is restarted
//! when it stops: a launchd agent on macOS, a systemd user unit on Linux
//! (SV-03). The service manager is driven through a [`Runner`], so that the
//! tests here never reach the real one.

use std::env;
use std::ffi::OsString;
use std::fmt::{self, Write as _};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::error::Error;
use crate::program;

/// The launchd job's label, and the plist's name. Installing looks for no
/// other label. A service under another label left running on the same
/// address holds it, and install refuses.
pub(crate) const LABEL: &str = "sh.cases.serve";

/// The systemd user unit. It names no repository, so on Linux installing
/// replaces any unit of that name, whatever installed it.
pub(crate) const UNIT: &str = "cases-serve.service";

/// The status `launchctl print` exits with for a label launchd does not
/// know.
const LAUNCHD_NO_SERVICE: i32 = 113;

/// How often, and how far apart, install tries to load the agent while
/// launchd is still tearing down a job booted out just before.
const BOOTSTRAP_TRIES: u32 = 10;
const BOOTSTRAP_RETRY: Duration = Duration::from_millis(300);

/// The service manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum System {
    /// launchd, on macOS.
    Launchd,
    /// systemd, on Linux.
    Systemd,
}

impl System {
    /// How people call it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            System::Launchd => "launchd",
            System::Systemd => "systemd",
        }
    }

    /// The program that drives it.
    fn program(self) -> &'static str {
        match self {
            System::Launchd => "launchctl",
            System::Systemd => "systemctl",
        }
    }
}

/// What the service runs.
#[derive(Debug)]
pub(crate) struct Spec {
    /// The program and its arguments.
    pub(crate) args: Vec<String>,
    /// The service's environment.
    pub(crate) env: Vec<(String, String)>,
    /// The file serve logs to under launchd, whose directory install
    /// makes. systemd sends the output to the journal.
    pub(crate) log: PathBuf,
}

/// What a service manager command did: its exit status, `None` when a
/// signal ended it, and its stdout and stderr together.
#[derive(Debug)]
pub(crate) struct Ran {
    pub(crate) code: Option<i32>,
    pub(crate) output: String,
}

/// Runs `launchctl` or `systemctl` with the arguments given. An error is a
/// command that could not be run at all.
pub(crate) type Runner = Box<dyn Fn(&str, &[&str]) -> Result<Ran, Error>>;

/// Installs, removes and asks about the service on one system.
pub(crate) struct Manager {
    /// launchd or systemd.
    pub(crate) system: System,
    /// Where the plist or unit file goes.
    pub(crate) dir: PathBuf,
    /// The user whose launchd domain, `gui/UID`, the agent is loaded in.
    pub(crate) uid: u32,
    run: Runner,
    retry: Duration,
}

impl fmt::Debug for Manager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Manager")
            .field("system", &self.system)
            .field("dir", &self.dir)
            .field("uid", &self.uid)
            .finish_non_exhaustive()
    }
}

/// The error for an uninstall with nothing to remove.
pub(crate) const NOT_INSTALLED: &str = "the cases serve service is not installed";

impl Manager {
    /// A manager for `system` that writes into `dir` and runs its commands
    /// with `run`.
    pub(crate) fn new(system: System, dir: PathBuf, uid: u32, run: Runner) -> Manager {
        Manager {
            system,
            dir,
            uid,
            run,
            retry: BOOTSTRAP_RETRY,
        }
    }

    /// The manager for this machine and user: the plist in
    /// `~/Library/LaunchAgents` on macOS, the unit in
    /// `$XDG_CONFIG_HOME/systemd/user` or `~/.config/systemd/user` on Linux,
    /// and the `launchctl` or `systemctl` on `$PATH`.
    ///
    /// # Errors
    ///
    /// On another system, when the directory cannot be found, or when the
    /// program is not on `$PATH`.
    pub(crate) fn for_this_machine() -> Result<Manager, Error> {
        let (system, dir) = system_dir(
            env::consts::OS,
            env::var_os("HOME"),
            env::var_os("XDG_CONFIG_HOME"),
        )?;
        let program = program::find(system.program())?;
        let run: Runner = Box::new(move |_, args| {
            let output = Command::new(&program)
                .args(args)
                .output()
                .map_err(|error| Error::new(format!("{}: {error}", program.display())))?;
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            Ok(Ran {
                code: output.status.code(),
                output: text,
            })
        });
        Ok(Manager::new(system, dir, uid(), run))
    }

    /// The plist or unit file.
    pub(crate) fn path(&self) -> PathBuf {
        match self.system {
            System::Launchd => self.dir.join(format!("{LABEL}.plist")),
            System::Systemd => self.dir.join(UNIT),
        }
    }

    /// The plist or unit file for `spec`.
    pub(crate) fn render(&self, spec: &Spec) -> String {
        match self.system {
            System::Launchd => plist(spec),
            System::Systemd => unit(spec),
        }
    }

    /// The command that shows the service manager's view of the service.
    pub(crate) fn check(&self) -> String {
        match self.system {
            System::Launchd => format!("launchctl print {}", self.target()),
            System::Systemd => format!("systemctl --user status {UNIT}"),
        }
    }

    /// Whether the plist or unit file is there.
    ///
    /// # Errors
    ///
    /// When whether it is there cannot be found out.
    pub(crate) fn installed(&self) -> Result<bool, Error> {
        let path = self.path();
        match fs::metadata(&path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(Error::new(format!("{}: {error}", path.display()))),
        }
    }

    /// Writes the file for `spec` and has the service manager start it,
    /// replacing a service already installed. Gives the file's path.
    ///
    /// # Errors
    ///
    /// When the file cannot be written or a service manager command fails.
    /// launchd is asked whether it has the agent before the file is
    /// replaced, so that a query that fails leaves the file matching the
    /// agent that runs.
    pub(crate) fn install(&self, spec: &Spec) -> Result<PathBuf, Error> {
        let path = self.path();
        let body = self.render(spec);
        if self.system == System::Systemd {
            write_file(&self.dir, &path, &body)?;
            for args in [
                &["--user", "daemon-reload"][..],
                &["--user", "enable", UNIT],
                // restart starts a stopped unit and picks up a changed one.
                &["--user", "restart", UNIT],
            ] {
                self.run(args)?;
            }
            return Ok(path);
        }
        // The log's directory, made here as well as by serve, so that it is
        // there to look in from the start.
        if let Some(dir) = spec.log.parent() {
            fs::create_dir_all(dir).map_err(|error| in_path(dir, &error))?;
        }
        let loaded = self.loaded()?;
        write_file(&self.dir, &path, &body)?;
        // A loaded agent keeps the old plist until it is booted out, and
        // bootstrap refuses a label that is loaded.
        if loaded {
            self.run(&["bootout", &self.target()])?;
        }
        // bootout can return while launchd is still tearing the job down,
        // and a bootstrap then fails (5: Input/output error). The same
        // follows an uninstall run just before. Give the new job a few tries
        // rather than leave nothing running until the next login.
        let domain = self.domain();
        let file = path.to_string_lossy();
        let mut tries = 1;
        loop {
            match self.run(&["bootstrap", &domain, &file]) {
                Ok(()) => return Ok(path),
                Err(error) if tries == BOOTSTRAP_TRIES => return Err(error),
                Err(_) => {
                    tries += 1;
                    thread::sleep(self.retry);
                }
            }
        }
    }

    /// Stops the service and removes its file. Gives the file's path.
    ///
    /// # Errors
    ///
    /// When there is no file and nothing running ([`NOT_INSTALLED`]), when
    /// the service manager cannot be asked whether it has the service (the
    /// file is left), or when a command fails.
    pub(crate) fn uninstall(&self) -> Result<PathBuf, Error> {
        let path = self.path();
        let exists = self.installed()?;
        if self.system == System::Launchd {
            let loaded = self.loaded()?;
            if !exists && !loaded {
                return Err(Error::new(NOT_INSTALLED));
            }
            if loaded {
                self.run(&["bootout", &self.target()])?;
            }
            remove(&path)?;
            return Ok(path);
        }
        if exists {
            self.run(&["--user", "disable", "--now", UNIT])?;
            remove(&path)?;
        } else {
            // A unit whose file was removed runs until it is stopped; cases
            // service points here for it.
            if !self.active()? {
                return Err(Error::new(NOT_INSTALLED));
            }
            self.run(&["--user", "stop", UNIT])?;
        }
        self.run(&["--user", "daemon-reload"])?;
        Ok(path)
    }

    /// Whether the service manager has the service: loaded in launchd, or
    /// active in systemd, counting a unit waiting to be restarted
    /// (`activating`) as active, as launchd counts a job it will restart.
    /// It only reads.
    ///
    /// # Errors
    ///
    /// When the query fails without saying, as when the service manager
    /// cannot be reached, or is killed by a signal, whatever it printed
    /// first. That is not a service that is not there.
    pub(crate) fn active(&self) -> Result<bool, Error> {
        if self.system == System::Launchd {
            return self.loaded();
        }
        let args = ["--user", "is-active", UNIT];
        let ran = (self.run)(self.system.program(), &args)?;
        match ran.code {
            Some(0) => return Ok(true),
            None => return Err(self.failed(&args, &ran)),
            Some(_) => {}
        }
        match ran.output.trim() {
            "activating" => Ok(true),
            "inactive" | "failed" | "deactivating" | "maintenance" | "unknown" => Ok(false),
            _ => Err(self.failed(&args, &ran)),
        }
    }

    /// The program and arguments in the installed file, as install wrote
    /// them.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, or names no program.
    pub(crate) fn args(&self) -> Result<Vec<String>, Error> {
        let path = self.path();
        let text = fs::read_to_string(&path).map_err(|error| in_path(&path, &error))?;
        let args = match self.system {
            System::Launchd => plist_args(&text),
            System::Systemd => unit_args(&text),
        };
        match args {
            Ok(args) if !args.is_empty() => Ok(args),
            Ok(_) => Err(Error::new(format!(
                "read {}: it names no program",
                path.display()
            ))),
            Err(error) => Err(Error::new(format!("read {}: {error}", path.display()))),
        }
    }

    /// `gui/UID`, the user's launchd domain.
    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    /// The agent in the user's launchd domain.
    fn target(&self) -> String {
        format!("{}/{LABEL}", self.domain())
    }

    /// Whether launchd has the agent. Only `launchctl print` exiting
    /// [`LAUNCHD_NO_SERVICE`] means it does not; any other failure is an
    /// error.
    fn loaded(&self) -> Result<bool, Error> {
        let target = self.target();
        let args = ["print", target.as_str()];
        let ran = (self.run)(self.system.program(), &args)?;
        match ran.code {
            Some(0) => Ok(true),
            Some(LAUNCHD_NO_SERVICE) => Ok(false),
            _ => Err(self.failed(&args, &ran)),
        }
    }

    /// Runs the system's program with `args`, which must succeed.
    fn run(&self, args: &[&str]) -> Result<(), Error> {
        let ran = (self.run)(self.system.program(), args)?;
        if ran.code == Some(0) {
            return Ok(());
        }
        Err(self.failed(args, &ran))
    }

    /// The error for a command that failed: the command, its status and its
    /// output.
    fn failed(&self, args: &[&str], ran: &Ran) -> Error {
        let mut message = format!("{} {}: ", self.system.program(), args.join(" "));
        match ran.code {
            Some(code) => {
                let _ = write!(message, "exit status {code}");
            }
            None => message.push_str("killed by a signal"),
        }
        let output = ran.output.trim();
        if !output.is_empty() {
            let _ = write!(message, ": {output}");
        }
        Error::new(message)
    }
}

/// The service manager of the system `os` names, as in
/// [`env::consts::OS`], and where its plist or unit file goes, given
/// `$HOME` and `$XDG_CONFIG_HOME`. Empty counts as unset.
///
/// # Errors
///
/// On another system, or when the directory cannot be found.
fn system_dir(
    os: &str,
    home: Option<OsString>,
    xdg_config: Option<OsString>,
) -> Result<(System, PathBuf), Error> {
    let home = home.filter(|home| !home.is_empty()).map(PathBuf::from);
    if os == "macos" {
        let home =
            home.ok_or_else(|| Error::new("cannot find ~/Library/LaunchAgents: $HOME is not set"))?;
        Ok((System::Launchd, home.join("Library").join("LaunchAgents")))
    } else if os == "linux" {
        let base = match xdg_config.filter(|base| !base.is_empty()) {
            Some(base) => PathBuf::from(base),
            None => home
                .ok_or_else(|| {
                    Error::new(
                        "cannot find the systemd user directory: neither $XDG_CONFIG_HOME \
                         nor $HOME is set",
                    )
                })?
                .join(".config"),
        };
        Ok((System::Systemd, base.join("systemd").join("user")))
    } else {
        Err(Error::new(format!(
            "a cases serve service is not supported on {os}; run cases serve under your own \
             supervisor"
        )))
    }
}

/// This user's id.
#[cfg(unix)]
fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Off unix there is no launchd domain to name.
#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

/// Replaces `path`, in `dir`, with `body`, through a file renamed into place
/// so that the service manager never reads half of one.
fn write_file(dir: &Path, path: &Path, body: &str) -> Result<(), Error> {
    fs::create_dir_all(dir).map_err(|error| in_path(dir, &error))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = fs::write(&temp, body).and_then(|()| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o644))?;
        }
        fs::rename(&temp, path)
    });
    written.map_err(|error| {
        let _ = fs::remove_file(&temp);
        in_path(path, &error)
    })
}

/// Removes `path`, which may already be gone.
fn remove(path: &Path) -> Result<(), Error> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(in_path(path, &error)),
        _ => Ok(()),
    }
}

/// An error on `path`, naming it.
fn in_path(path: &Path, error: &io::Error) -> Error {
    Error::new(format!("{}: {error}", path.display()))
}

/// The launchd agent for `spec`: loaded at login (`RunAtLoad`) and started
/// again whenever it exits while its program is there (`KeepAlive`'s
/// `PathState`). A plist left behind when the program is removed, as by a
/// package manager's uninstall, then goes quiet rather than try to start a
/// missing program for ever.
fn plist(spec: &Spec) -> String {
    let mut plist = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    let p = &mut plist;
    element(p, "\t", "key", "Label");
    element(p, "\t", "string", LABEL);
    element(p, "\t", "key", "ProgramArguments");
    p.push_str("\t<array>\n");
    for arg in &spec.args {
        element(p, "\t\t", "string", arg);
    }
    p.push_str("\t</array>\n");
    if !spec.env.is_empty() {
        element(p, "\t", "key", "EnvironmentVariables");
        p.push_str("\t<dict>\n");
        for (key, value) in &spec.env {
            element(p, "\t\t", "key", key);
            element(p, "\t\t", "string", value);
        }
        p.push_str("\t</dict>\n");
    }
    element(p, "\t", "key", "RunAtLoad");
    p.push_str("\t<true/>\n");
    element(p, "\t", "key", "KeepAlive");
    p.push_str("\t<dict>\n");
    element(p, "\t\t", "key", "PathState");
    p.push_str("\t\t<dict>\n");
    element(
        p,
        "\t\t\t",
        "key",
        spec.args.first().map_or("", String::as_str),
    );
    p.push_str("\t\t\t<true/>\n\t\t</dict>\n\t</dict>\n");
    // serve writes its log to `spec.log` itself, and rotates it, pointing
    // its stderr at the file it writes as it starts. launchd's stderr goes
    // to the same file, for what comes before that, such as an argument
    // this binary does not know; stdout says only what the log says.
    let log = spec.log.to_string_lossy();
    element(p, "\t", "key", "StandardOutPath");
    element(p, "\t", "string", "/dev/null");
    element(p, "\t", "key", "StandardErrorPath");
    element(p, "\t", "string", &log);
    p.push_str("</dict>\n</plist>\n");
    plist
}

/// Adds `<tag>text</tag>` on a line of its own to `plist`.
fn element(plist: &mut String, indent: &str, tag: &str, text: &str) {
    let _ = writeln!(plist, "{indent}<{tag}>{}</{tag}>", xml_escape(text));
}

/// `text` with the characters XML gives meaning to written as entities,
/// along with tabs and line breaks.
fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&#34;"),
            '\'' => escaped.push_str("&#39;"),
            '\t' => escaped.push_str("&#x9;"),
            '\n' => escaped.push_str("&#xA;"),
            '\r' => escaped.push_str("&#xD;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// The systemd user unit for `spec`. Enabled, it starts with the user's
/// session (`default.target`), and it is restarted whenever it exits. Each
/// start is skipped when its program is not there
/// (`ConditionPathExists`), which also ends the restarts, so a unit left
/// behind when the program is removed goes quiet.
fn unit(spec: &Spec) -> String {
    let args: Vec<String> = spec.args.iter().map(|arg| unit_quote(arg, true)).collect();
    // A condition's value is not quoted: only its specifiers are special.
    let program = spec.args.first().map_or("", String::as_str);
    let mut unit = format!(
        "[Unit]\nDescription=cases web inbox (cases serve)\nConditionPathExists={}\n\n\
         [Service]\nExecStart={}\n",
        program.replace('%', "%%"),
        args.join(" ")
    );
    for (key, value) in &spec.env {
        let _ = writeln!(
            unit,
            "Environment={}",
            unit_quote(&format!("{key}={value}"), false)
        );
    }
    unit.push_str("Restart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n");
    unit
}

/// `value` in double quotes for a unit file. `%` starts a specifier
/// everywhere, and `$` a variable in a command line (`exec`), so each is
/// doubled where it is special.
fn unit_quote(value: &str, exec: bool) -> String {
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '%' => quoted.push_str("%%"),
            '$' if exec => quoted.push_str("$$"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

/// The `ProgramArguments` array of a plist as [`plist`] writes it.
fn plist_args(text: &str) -> Result<Vec<String>, String> {
    let Some(at) = text.find("<key>ProgramArguments</key>") else {
        return Ok(Vec::new());
    };
    let rest = text[at..]
        .split_once("<array>")
        .ok_or("no array after ProgramArguments")?
        .1;
    let array = rest
        .split_once("</array>")
        .ok_or("the ProgramArguments array is not closed")?
        .0;
    let mut args = Vec::new();
    let mut rest = array.trim_start();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<string/>") {
            args.push(String::new());
            rest = after.trim_start();
            continue;
        }
        let after = rest
            .strip_prefix("<string>")
            .ok_or_else(|| format!("not a string in ProgramArguments: {rest}"))?;
        let (value, after) = after
            .split_once("</string>")
            .ok_or("a string in ProgramArguments is not closed")?;
        args.push(xml_unescape(value)?);
        rest = after.trim_start();
    }
    Ok(args)
}

/// `text` with its XML entities read.
fn xml_unescape(text: &str) -> Result<String, String> {
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        plain.push_str(&rest[..at]);
        let (entity, after) = rest[at + 1..]
            .split_once(';')
            .ok_or_else(|| format!("an entity is not closed: {rest}"))?;
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => entity
                .strip_prefix("#x")
                .map(|hex| u32::from_str_radix(hex, 16))
                .or_else(|| entity.strip_prefix('#').map(str::parse))
                .and_then(Result::ok)
                .and_then(char::from_u32)
                .ok_or_else(|| format!("unknown entity &{entity};"))?,
        };
        plain.push(c);
        rest = after;
    }
    plain.push_str(rest);
    Ok(plain)
}

/// The `ExecStart` line of a unit as [`unit()`] writes it, split into the
/// program and its arguments.
fn unit_args(text: &str) -> Result<Vec<String>, String> {
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))
        .ok_or("no ExecStart line")?;
    let mut args = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        let mut chars = rest
            .strip_prefix('"')
            .ok_or_else(|| format!("unquoted argument in ExecStart: {rest}"))?
            .char_indices();
        let mut arg = String::new();
        let end = loop {
            let Some((i, c)) = chars.next() else {
                return Err("unterminated quote in ExecStart".to_owned());
            };
            match c {
                '"' => break i,
                '\\' => match chars.next() {
                    Some((_, 'n')) => arg.push('\n'),
                    Some((_, c)) => arg.push(c),
                    None => return Err("unterminated quote in ExecStart".to_owned()),
                },
                '%' | '$' => {
                    // A doubled one is one; a single one is kept as it is.
                    let mut ahead = chars.clone();
                    if ahead.next().is_some_and(|(_, next)| next == c) {
                        chars = ahead;
                    }
                    arg.push(c);
                }
                c => arg.push(c),
            }
        };
        args.push(arg);
        rest = rest[1 + end + 1..].trim_start();
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// The commands a fake runner was given, one line each.
    type Calls = Rc<RefCell<Vec<String>>>;

    /// A runner that records its commands and runs none. A command whose
    /// line starts with a prefix in `fail` exits with that status and
    /// output; every other succeeds.
    fn fake(fail: &[(&str, i32, &str)]) -> (Runner, Calls) {
        let calls = Calls::default();
        let record = Rc::clone(&calls);
        let fail: Vec<(String, i32, String)> = fail
            .iter()
            .map(|(line, code, output)| ((*line).to_owned(), *code, (*output).to_owned()))
            .collect();
        let run: Runner = Box::new(move |name, args| {
            let line = format!("{name} {}", args.join(" "));
            record.borrow_mut().push(line.clone());
            let (code, output) = fail
                .iter()
                .find(|(prefix, ..)| line.starts_with(prefix))
                .map_or((0, String::new()), |(_, code, output)| {
                    (*code, output.clone())
                });
            Ok(Ran {
                code: Some(code),
                output,
            })
        });
        (run, calls)
    }

    /// `launchctl print` for a label launchd does not know.
    const NO_SERVICE: (&str, i32, &str) = (
        "launchctl print",
        113,
        "Bad request.\nCould not find service",
    );

    /// `systemctl is-active` for a unit that is not running.
    const INACTIVE: (&str, i32, &str) = ("systemctl --user is-active", 3, "inactive\n");

    fn fake_manager(
        system: System,
        fail: &[(&str, i32, &str)],
    ) -> (Manager, Calls, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (run, calls) = fake(fail);
        let mut manager = Manager::new(system, dir.path().join("agents"), 501, run);
        manager.retry = Duration::ZERO;
        (manager, calls, dir)
    }

    fn spec(dir: &Path) -> Spec {
        Spec {
            args: [
                "/opt/my tools/cases",
                "--store",
                "/data/a&b <1>.db",
                "serve",
                "--no-open",
                "--listen",
                "127.0.0.1:7337",
            ]
            .map(str::to_owned)
            .to_vec(),
            env: vec![
                ("HOME".to_owned(), "/Users/me".to_owned()),
                (
                    "XDG_STATE_HOME".to_owned(),
                    "/Users/me/.local/state".to_owned(),
                ),
            ],
            log: dir.join("state").join("cases").join("serve.log"),
        }
    }

    /// The calls made since the last look.
    fn calls(calls: &Calls) -> Vec<String> {
        calls.borrow_mut().drain(..).collect()
    }

    /// A variable of the environment [`system_dir`] is given.
    fn var(text: &str) -> OsString {
        OsString::from(text)
    }

    #[test]
    fn on_macos_the_plist_is_under_home_whatever_xdg_config_home_is() {
        let want = (System::Launchd, PathBuf::from("/h/Library/LaunchAgents"));
        for xdg in [None, Some(var("")), Some(var("/x"))] {
            assert_eq!(system_dir("macos", Some(var("/h")), xdg).unwrap(), want);
        }
    }

    /// `$HOME` unset or empty, and `$XDG_CONFIG_HOME` does not stand in.
    #[test]
    fn on_macos_without_home_there_is_no_plist_directory() {
        for home in [None, Some(var(""))] {
            let error = system_dir("macos", home, Some(var("/x"))).unwrap_err();
            assert_eq!(
                error.to_string(),
                "cannot find ~/Library/LaunchAgents: $HOME is not set"
            );
        }
    }

    #[test]
    fn on_linux_the_unit_is_under_xdg_config_home_then_home() {
        let xdg = (System::Systemd, PathBuf::from("/x/systemd/user"));
        let home = (System::Systemd, PathBuf::from("/h/.config/systemd/user"));
        assert_eq!(
            system_dir("linux", Some(var("/h")), Some(var("/x"))).unwrap(),
            xdg
        );
        assert_eq!(system_dir("linux", None, Some(var("/x"))).unwrap(), xdg);
        assert_eq!(
            system_dir("linux", Some(var("")), Some(var("/x"))).unwrap(),
            xdg
        );
        assert_eq!(system_dir("linux", Some(var("/h")), None).unwrap(), home);
        assert_eq!(
            system_dir("linux", Some(var("/h")), Some(var(""))).unwrap(),
            home
        );
    }

    #[test]
    fn on_linux_without_either_there_is_no_unit_directory() {
        for (home, xdg) in [(None, None), (Some(var("")), Some(var("")))] {
            let error = system_dir("linux", home, xdg).unwrap_err();
            assert_eq!(
                error.to_string(),
                "cannot find the systemd user directory: neither $XDG_CONFIG_HOME nor $HOME is set"
            );
        }
    }

    /// Any other system is refused by name, even with both directories.
    #[test]
    fn another_system_is_refused_by_name() {
        for os in ["freebsd", "windows"] {
            let error = system_dir(os, Some(var("/h")), Some(var("/x"))).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!(
                    "a cases serve service is not supported on {os}; run cases serve under your \
                     own supervisor"
                )
            );
        }
    }

    #[test]
    fn the_plist_runs_serve_at_load_and_keeps_it_alive() {
        let spec = spec(Path::new("/tmp"));
        let plist = plist(&spec);
        for want in [
            "<key>Label</key>\n\t<string>sh.cases.serve</string>",
            "\t\t<string>/opt/my tools/cases</string>\n\t\t<string>--store</string>\n\t\t\
             <string>/data/a&amp;b &lt;1&gt;.db</string>\n\t\t<string>serve</string>",
            "<key>HOME</key>\n\t\t<string>/Users/me</string>",
            "<key>XDG_STATE_HOME</key>\n\t\t<string>/Users/me/.local/state</string>",
            "<key>RunAtLoad</key>\n\t<true/>",
            "<key>KeepAlive</key>\n\t<dict>\n\t\t<key>PathState</key>\n\t\t<dict>\n\t\t\t\
             <key>/opt/my tools/cases</key>\n\t\t\t<true/>\n\t\t</dict>\n\t</dict>",
            "<key>StandardOutPath</key>\n\t<string>/dev/null</string>",
            "<key>StandardErrorPath</key>\n\t<string>/tmp/state/cases/serve.log</string>",
        ] {
            assert!(plist.contains(want), "{want}\n{plist}");
        }
        // plutil comes with macOS; elsewhere the checks above have to do.
        let plutil = Path::new("/usr/bin/plutil");
        if !plutil.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.plist");
        fs::write(&path, &plist).unwrap();
        let out = Command::new(plutil)
            .arg("-lint")
            .arg(&path)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }

    #[test]
    fn the_unit_restarts_serve_and_quotes_what_systemd_reads() {
        let spec = Spec {
            args: ["/opt/my tools/50% cases", "--as", r#"R "100%" $USER\x"#]
                .map(str::to_owned)
                .to_vec(),
            env: vec![
                ("HOME".to_owned(), "/home/me".to_owned()),
                ("XDG_STATE_HOME".to_owned(), "/home/me/50%$".to_owned()),
            ],
            log: PathBuf::new(),
        };
        assert_eq!(
            unit(&spec),
            r#"[Unit]
Description=cases web inbox (cases serve)
ConditionPathExists=/opt/my tools/50%% cases

[Service]
ExecStart="/opt/my tools/50%% cases" "--as" "R \"100%%\" $$USER\\x"
Environment="HOME=/home/me"
Environment="XDG_STATE_HOME=/home/me/50%%$"
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
"#
        );
    }

    #[test]
    fn install_writes_the_plist_and_bootstraps_it() {
        let (manager, log, dir) = fake_manager(System::Launchd, &[NO_SERVICE]);
        let spec = spec(dir.path());
        let path = manager.install(&spec).unwrap();
        assert_eq!(path, manager.dir.join(format!("{LABEL}.plist")));
        assert_eq!(fs::read_to_string(&path).unwrap(), plist(&spec));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }
        assert!(spec.log.parent().unwrap().is_dir());
        assert_eq!(
            calls(&log),
            [
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl bootstrap gui/501 {}", path.display()),
            ]
        );
        // Nothing but the plist is left in the directory.
        assert_eq!(fs::read_dir(&manager.dir).unwrap().count(), 1);
    }

    #[test]
    fn install_boots_out_a_loaded_agent_first() {
        let (manager, log, dir) = fake_manager(System::Launchd, &[]);
        let path = manager.install(&spec(dir.path())).unwrap();
        assert_eq!(
            calls(&log),
            [
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl bootout gui/501/{LABEL}"),
                format!("launchctl bootstrap gui/501 {}", path.display()),
            ]
        );
    }

    #[test]
    fn install_tries_bootstrap_again_while_launchd_tears_down() {
        let dir = tempfile::tempdir().unwrap();
        let (inner, log) = fake(&[]);
        let fails = Cell::new(2);
        let run: Runner = Box::new(move |name, args| {
            let ran = inner(name, args)?;
            if args[0] == "bootstrap" && fails.get() > 0 {
                fails.set(fails.get() - 1);
                return Ok(Ran {
                    code: Some(5),
                    output: "Bootstrap failed: 5: Input/output error".to_owned(),
                });
            }
            Ok(ran)
        });
        let mut manager = Manager::new(System::Launchd, dir.path().join("agents"), 501, run);
        manager.retry = Duration::ZERO;
        let path = manager.install(&spec(dir.path())).unwrap();
        let bootstrap = format!("launchctl bootstrap gui/501 {}", path.display());
        assert_eq!(
            calls(&log),
            [
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl bootout gui/501/{LABEL}"),
                bootstrap.clone(),
                bootstrap.clone(),
                bootstrap,
            ]
        );

        // It gives up after ten tries, with the last error.
        let (manager, log, dir) = fake_manager(
            System::Launchd,
            &[NO_SERVICE, ("launchctl bootstrap", 5, "no")],
        );
        let error = manager.install(&spec(dir.path())).unwrap_err().to_string();
        assert!(error.ends_with("exit status 5: no"), "{error}");
        assert_eq!(calls(&log).len(), 11);
    }

    #[test]
    fn install_writes_the_unit_then_reloads_enables_and_restarts_it() {
        let (manager, log, dir) = fake_manager(System::Systemd, &[]);
        let spec = spec(dir.path());
        let path = manager.install(&spec).unwrap();
        assert_eq!(path, manager.dir.join(UNIT));
        assert_eq!(fs::read_to_string(&path).unwrap(), unit(&spec));
        assert_eq!(
            calls(&log),
            [
                "systemctl --user daemon-reload".to_owned(),
                format!("systemctl --user enable {UNIT}"),
                format!("systemctl --user restart {UNIT}"),
            ]
        );
    }

    #[test]
    fn a_command_that_fails_is_named_with_its_output() {
        let (manager, _, dir) = fake_manager(
            System::Systemd,
            &[("systemctl --user restart", 1, "Failed to connect to bus\n")],
        );
        let error = manager.install(&spec(dir.path())).unwrap_err().to_string();
        assert_eq!(
            error,
            format!("systemctl --user restart {UNIT}: exit status 1: Failed to connect to bus")
        );
    }

    #[test]
    fn uninstall_boots_out_the_agent_and_removes_the_plist() {
        let (manager, log, dir) = fake_manager(System::Launchd, &[]);
        let path = manager.install(&spec(dir.path())).unwrap();
        calls(&log);
        assert_eq!(manager.uninstall().unwrap(), path);
        assert!(!path.exists());
        assert_eq!(
            calls(&log),
            [
                format!("launchctl print gui/501/{LABEL}"),
                format!("launchctl bootout gui/501/{LABEL}"),
            ]
        );
    }

    #[test]
    fn uninstall_removes_a_plist_never_loaded_and_refuses_nothing() {
        let (manager, log, dir) = fake_manager(System::Launchd, &[NO_SERVICE]);
        assert_eq!(manager.uninstall().unwrap_err().to_string(), NOT_INSTALLED);
        let path = manager.install(&spec(dir.path())).unwrap();
        calls(&log);
        manager.uninstall().unwrap();
        assert!(!path.exists());
        assert_eq!(calls(&log), [format!("launchctl print gui/501/{LABEL}")]);
    }

    #[test]
    fn uninstall_disables_the_unit_and_removes_it() {
        let (manager, log, dir) = fake_manager(System::Systemd, &[INACTIVE]);
        assert_eq!(manager.uninstall().unwrap_err().to_string(), NOT_INSTALLED);
        assert_eq!(calls(&log), [format!("systemctl --user is-active {UNIT}")]);
        let path = manager.install(&spec(dir.path())).unwrap();
        calls(&log);
        manager.uninstall().unwrap();
        assert!(!path.exists());
        assert_eq!(
            calls(&log),
            [
                format!("systemctl --user disable --now {UNIT}"),
                "systemctl --user daemon-reload".to_owned(),
            ]
        );
    }

    #[test]
    fn uninstall_stops_a_unit_whose_file_is_gone() {
        let (manager, log, _dir) = fake_manager(System::Systemd, &[]);
        assert_eq!(manager.uninstall().unwrap(), manager.dir.join(UNIT));
        assert_eq!(
            calls(&log),
            [
                format!("systemctl --user is-active {UNIT}"),
                format!("systemctl --user stop {UNIT}"),
                "systemctl --user daemon-reload".to_owned(),
            ]
        );
    }

    #[test]
    fn args_reads_back_what_install_wrote() {
        let args = [
            "/opt/my tools/cases",
            "--store",
            r#"/d/a&b <1> "q" 50% $HOME\x 'it'"#,
            "serve",
            "--as",
            "line\nbreak\ttab",
            "",
        ]
        .map(str::to_owned)
        .to_vec();
        for system in [System::Launchd, System::Systemd] {
            let (manager, _, dir) = fake_manager(system, &[NO_SERVICE]);
            assert!(manager.args().is_err(), "{system:?}");
            let spec = Spec {
                args: args.clone(),
                env: vec![("HOME".to_owned(), "/h".to_owned())],
                log: dir.path().join("log"),
            };
            manager.install(&spec).unwrap();
            assert_eq!(manager.args().unwrap(), args, "{system:?}");
            fs::write(manager.path(), "[Service]\nRestart=always\n").unwrap();
            assert!(manager.args().is_err(), "{system:?}");
        }
    }

    #[test]
    fn active_asks_the_service_manager() {
        for (system, query, absent) in [
            (
                System::Launchd,
                format!("launchctl print gui/501/{LABEL}"),
                NO_SERVICE,
            ),
            (
                System::Systemd,
                format!("systemctl --user is-active {UNIT}"),
                INACTIVE,
            ),
        ] {
            let (manager, log, _dir) = fake_manager(system, &[]);
            assert!(manager.active().unwrap());
            assert_eq!(calls(&log), std::slice::from_ref(&query));
            let (manager, log, _dir) = fake_manager(system, &[absent]);
            assert!(!manager.active().unwrap());
            assert_eq!(calls(&log), [query]);
        }
        // A unit waiting out RestartSec is still the service manager's.
        let (manager, _, _dir) = fake_manager(
            System::Systemd,
            &[("systemctl --user is-active", 3, "activating\n")],
        );
        assert!(manager.active().unwrap());
    }

    /// Ways the query can fail without saying whether the service is there:
    /// the program cannot be run, it cannot reach the service manager, or it
    /// is killed by a signal after it printed a state.
    fn query_failures() -> Vec<(System, Runner)> {
        /// A runner whose every command prints `output` and is then killed
        /// by a signal.
        fn killed(output: &'static str) -> Runner {
            Box::new(move |_, _| {
                Ok(Ran {
                    code: None,
                    output: output.to_owned(),
                })
            })
        }
        fn cannot_run(_: &str, _: &[&str]) -> Result<Ran, Error> {
            Err(Error::new("cannot run"))
        }
        vec![
            (System::Launchd, Box::new(cannot_run)),
            (System::Systemd, Box::new(cannot_run)),
            (
                System::Launchd,
                fake(&[("launchctl", 5, "Could not print: 5: Input/output error")]).0,
            ),
            (
                System::Systemd,
                fake(&[("systemctl", 1, "Failed to connect to bus: No medium found")]).0,
            ),
            (System::Systemd, killed("activating\n")),
            (System::Systemd, killed("inactive\n")),
        ]
    }

    #[test]
    fn a_query_that_fails_is_an_error_not_a_service_that_is_not_there() {
        for (system, run) in query_failures() {
            let dir = tempfile::tempdir().unwrap();
            let manager = Manager::new(system, dir.path().to_owned(), 501, run);
            assert!(manager.active().is_err(), "{system:?}");
        }
    }

    /// A query killed by a signal has not answered, whatever it printed
    /// before it was killed: its output is not read as the unit's state.
    #[test]
    fn a_query_killed_after_it_printed_is_an_error() {
        for state in ["activating\n", "inactive\n"] {
            let dir = tempfile::tempdir().unwrap();
            let run: Runner = Box::new(move |_, _| {
                Ok(Ran {
                    code: None,
                    output: state.to_owned(),
                })
            });
            let manager = Manager::new(System::Systemd, dir.path().to_owned(), 501, run);
            let error = manager.active().unwrap_err().to_string();
            assert!(error.contains("killed by a signal"), "{state:?}: {error}");
        }
    }

    #[test]
    fn a_query_that_fails_stops_install_and_uninstall_before_they_change_the_file() {
        for (system, run) in query_failures() {
            let dir = tempfile::tempdir().unwrap();
            let manager = Manager::new(system, dir.path().join("agents"), 501, run);
            let path = manager.path();
            write_file(&manager.dir, &path, "old\n").unwrap();
            assert!(manager.install(&spec(dir.path())).is_err(), "{system:?}");
            if system == System::Launchd {
                assert_eq!(fs::read_to_string(&path).unwrap(), "old\n");
            }
            assert!(manager.uninstall().is_err(), "{system:?}");
            assert!(path.exists(), "{system:?}");
            fs::remove_file(&path).unwrap();
            let error = manager.uninstall().unwrap_err().to_string();
            assert_ne!(error, NOT_INSTALLED, "{system:?}");
        }
    }
}
