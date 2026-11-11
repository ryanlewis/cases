//! Longer text in the human's own editor (TU-06): `$VISUAL`, then
//! `$EDITOR`, then `vi`, on a file private to the user that is removed
//! afterwards.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::process::Command;

/// Opens `text` in the editor and gives what was saved, or `None` when the
/// editor exits non-zero or the text is as it was.
///
/// # Errors
///
/// When the file cannot be made or read, or the editor cannot be run.
pub(crate) fn edit(text: &str) -> Result<Option<String>, String> {
    let var = |name| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let editor = var("VISUAL")
        .or_else(|| var("EDITOR"))
        .unwrap_or_else(|| "vi".into());
    edit_with(&editor, text)
}

/// [`edit`] with the editor's command line given: a program and its
/// arguments, split at white space, as `code -w`.
pub(crate) fn edit_with(editor: &str, text: &str) -> Result<Option<String>, String> {
    let dir = tempfile::Builder::new()
        .prefix("cases-tui-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let path = dir.path().join("answer.md");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    drop(file);

    let mut words = editor.split_whitespace();
    let program = words.next().ok_or("no editor")?;
    let status = Command::new(program)
        .args(words)
        .arg(&path)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if !status.success() {
        return Ok(None);
    }
    let edited = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    // An editor adds a line break at the end of a file it saves.
    let edited = edited.strip_suffix('\n').unwrap_or(&edited).to_string();
    Ok((edited != text).then_some(edited))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An editor command that runs `body` as a shell script. `sh` reads the
    /// script rather than the kernel executing it: a child forked by another
    /// test can still hold the script open for writing, and executing a file
    /// in that state fails with "Text file busy".
    fn script(dir: &std::path::Path, body: &str) -> String {
        let path = dir.join("editor.sh");
        fs::write(&path, format!("{body}\n")).unwrap();
        format!("sh {}", path.display())
    }

    #[test]
    fn the_editor_gives_back_what_was_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mode = dir.path().join("mode");
        let editor = script(
            dir.path(),
            &format!(
                "stat -f %Lp \"$1\" > {0} 2>/dev/null || stat -c %a \"$1\" > {0}; printf 'more\\n' >> \"$1\"; echo \"$1\" > {1}",
                mode.display(),
                dir.path().join("path").display()
            ),
        );
        let edited = edit_with(&editor, "draft\n").unwrap();
        assert_eq!(edited.as_deref(), Some("draft\nmore"));
        assert_eq!(fs::read_to_string(&mode).unwrap().trim(), "600");
        let used = fs::read_to_string(dir.path().join("path")).unwrap();
        assert!(
            !std::path::Path::new(used.trim()).exists(),
            "the file was left"
        );
    }

    #[test]
    fn a_failed_or_unchanged_edit_keeps_the_text() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            edit_with(&script(dir.path(), "exit 1"), "draft").unwrap(),
            None
        );
        assert_eq!(
            edit_with(&script(dir.path(), "true"), "draft").unwrap(),
            None
        );
        assert!(edit_with("/no/such/editor", "draft").is_err());
    }
}
