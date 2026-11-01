//! Text from a flag: a line inline, or markdown from a file or stdin (CL-03).

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use crate::error::Error;

/// The text of a pair of flags such as `--body` and `--body-file`: `inline`,
/// or the contents of `file`, where `-` reads stdin. `None` when neither is
/// given. Parsing refuses both at once.
///
/// An inline value that names an existing file is refused: it is far more
/// likely a path meant for the file flag than the text to store, and a store
/// cannot take a write back.
///
/// # Errors
///
/// When the inline value names a file, or the file cannot be read or is not
/// UTF-8.
pub(crate) fn read(
    flag: &str,
    inline: Option<&str>,
    file: Option<&Path>,
) -> Result<Option<String>, Error> {
    if let Some(text) = inline {
        if fs::metadata(text).is_ok_and(|meta| !meta.is_dir()) {
            return Err(Error::new(format!(
                "--{flag} {text:?} names a file; pass it with --{flag}-file"
            )));
        }
        return Ok(Some(text.to_owned()));
    }
    let Some(file) = file else {
        return Ok(None);
    };
    let text = if file == Path::new("-") {
        let mut text = String::new();
        io::stdin().read_to_string(&mut text).map(|_| text)
    } else {
        fs::read_to_string(file)
    };
    text.map(Some)
        .map_err(|error| Error::new(format!("--{flag}-file {}: {error}", file.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_text_is_taken_as_it_is() {
        assert_eq!(
            read("body", Some("Bun is pinned."), None).unwrap(),
            Some("Bun is pinned.".into())
        );
        assert_eq!(read("body", None, None).unwrap(), None);
    }

    #[test]
    fn inline_text_that_names_a_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.md");
        fs::write(&file, "# Body\n").unwrap();
        let file = file.to_str().unwrap();
        let error = read("outcome", Some(file), None).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("--outcome {file:?} names a file; pass it with --outcome-file")
        );
        // A directory is not a file anyone meant to pass.
        let dir = dir.path().to_str().unwrap();
        assert_eq!(read("body", Some(dir), None).unwrap(), Some(dir.into()));
    }

    #[test]
    fn a_file_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("body.md");
        fs::write(&file, "# From a file\n").unwrap();
        assert_eq!(
            read("body", None, Some(&file)).unwrap(),
            Some("# From a file\n".into())
        );
        let missing = dir.path().join("missing.md");
        let error = read("body", None, Some(&missing)).unwrap_err().to_string();
        assert!(error.starts_with("--body-file "), "{error}");
    }
}
