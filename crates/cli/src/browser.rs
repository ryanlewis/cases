//! Opening a page in the human's browser.

use std::process::{Command, Stdio};
use std::thread;

use crate::error::Error;
use crate::program;

/// The program that opens a URL on this system: `open` on macOS,
/// `xdg-open` elsewhere.
const OPENER: &str = if cfg!(target_os = "macos") {
    "open"
} else {
    "xdg-open"
};

/// Starts the browser on `url`, and does not wait for it. The URL is passed
/// as an argument, not through a shell. The opener is found on `$PATH`
/// ([`program::find`]), so a test that puts its own `open` or `xdg-open`
/// first sees the URL and no browser opens.
///
/// # Errors
///
/// When the opener cannot be found or started.
pub(crate) fn open(url: &str) -> Result<(), Error> {
    let opener = program::find(OPENER)?;
    let mut child = Command::new(&opener)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::new(format!("{}: {error}", opener.display())))?;
    // Reaped in the background, so that a serve that runs on is not left
    // with a zombie.
    thread::spawn(move || child.wait());
    Ok(())
}
