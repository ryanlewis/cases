//! The inbox's modules, `web/static/*.ts`, as the JavaScript the browser
//! runs: each with its types blanked (see `build/strip.rs`), written to
//! `$OUT_DIR/static/` for `commands::serve::page` to embed. No node and no
//! npm: `tsc --noEmit` checks the types in CI, and this only strips them.
//! A file that does not strip fails the build.
//!
//! It also gzips the vendored mermaid, `web/static/vendor/mermaid/`, into
//! `$OUT_DIR/mermaid/`, and lists the files it wrote in
//! `$OUT_DIR/mermaid.rs`, for the page to embed and serve gzipped: over a
//! megabyte of script that a page only loads when a case has a diagram.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use flate2::Compression;
use flate2::write::GzEncoder;

#[path = "build/strip.rs"]
mod strip;

/// The inbox's TypeScript.
const WEB: &str = "src/commands/serve/web/static";

/// The files loaded as plain scripts, not modules.
const SCRIPTS: &[&str] = &["theme.ts"];

/// The vendored mermaid, served gzipped.
const MERMAID: &str = "src/commands/serve/web/static/vendor/mermaid";

fn main() -> ExitCode {
    println!("cargo::rerun-if-changed={WEB}");
    println!("cargo::rerun-if-changed=build/strip.rs");
    let Some(out) = env::var_os("OUT_DIR") else {
        println!("cargo::error=OUT_DIR is not set");
        return ExitCode::FAILURE;
    };
    if let Err(error) = gzip(MERMAID, Path::new(&out)) {
        println!("cargo::error=gzipping {MERMAID}: {}", one_line(&error));
        return ExitCode::FAILURE;
    }
    let out = Path::new(&out).join("static");
    let mut failed = false;
    let mut names: Vec<String> = match fs::read_dir(WEB) {
        Ok(dir) => dir
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| {
                Path::new(name).extension().is_some_and(|ext| ext == "ts")
                    && !name.ends_with(".d.ts")
            })
            .collect(),
        Err(error) => {
            println!("cargo::error=reading {WEB}: {error}");
            return ExitCode::FAILURE;
        }
    };
    names.sort();
    if let Err(error) = fs::create_dir_all(&out) {
        println!("cargo::error=creating {}: {error}", out.display());
        return ExitCode::FAILURE;
    }
    for name in names {
        let path = Path::new(WEB).join(&name);
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                println!("cargo::error={}: {error}", path.display());
                failed = true;
                continue;
            }
        };
        match strip::strip(&source, SCRIPTS.contains(&name.as_str())) {
            Ok(js) => {
                let to = out.join(Path::new(&name).with_extension("js"));
                if let Err(error) = fs::write(&to, js) {
                    println!("cargo::error={}: {error}", to.display());
                    failed = true;
                }
            }
            Err(problems) => {
                for problem in problems {
                    println!("cargo::error={}: {}", path.display(), one_line(&problem));
                }
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Each file under `dir` gzipped into `out/mermaid/`, at its path under
/// `dir`, and `out/mermaid.rs`: the files as the page's `File`s, each at
/// `vendor/mermaid/<path>`. The gzip header records no time and no name,
/// so the same file always gzips to the same bytes, and the build is the
/// same wherever it runs.
fn gzip(dir: &str, out: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    walk(Path::new(dir), &mut files)?;
    files.sort();
    let mut list = String::from("&[\n");
    for file in files {
        let path = file
            .strip_prefix(dir)
            .map_err(|error| error.to_string())?
            .to_str()
            .ok_or_else(|| format!("{} is not UTF-8", file.display()))?
            .to_owned();
        let bytes = fs::read(&file).map_err(|error| format!("{}: {error}", file.display()))?;
        let mut gz = GzEncoder::new(Vec::new(), Compression::best());
        gz.write_all(&bytes).map_err(|error| error.to_string())?;
        let gz = gz.finish().map_err(|error| error.to_string())?;
        let to = out.join("mermaid").join(format!("{path}.gz"));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&to, gz).map_err(|error| format!("{}: {error}", to.display()))?;
        let _ = writeln!(
            list,
            "    File {{ path: {:?}, kind: JS, bytes: include_bytes!({:?}) }},",
            format!("vendor/mermaid/{path}"),
            to.to_str()
                .ok_or_else(|| format!("{} is not UTF-8", to.display()))?,
        );
    }
    list.push_str("]\n");
    let to = out.join("mermaid.rs");
    fs::write(&to, list).map_err(|error| format!("{}: {error}", to.display()))
}

/// The files under `dir`, at any depth, into `files`, but for hidden ones.
fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        // A hidden file, such as the .DS_Store macOS leaves, is no part of
        // the release.
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            walk(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

/// `text` on one line, as a `cargo::error` takes it.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
