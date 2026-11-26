// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Build script of the help crate.
//!
//! It lists the command pages (`docs/commands`) and the guides (`docs/guides`)
//! of the repository and writes `embedded_docs.rs`, which embeds each file
//! with `include_str!`, so that `help` can show examples and guides without
//! reading files at run time. Files whose name starts with `_` and
//! `README.md` are not documentation pages.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Directories of the documentation, relative to the root of the repository,
/// with the name of the constant that holds their files.
const SOURCES: [(&str, &str); 2] = [("docs/commands", "PAGES"), ("docs/guides", "GUIDES")];

/// Name of the generated file.
const OUTPUT: &str = "embedded_docs.rs";

fn main() -> Result<(), String> {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?);
    let root = manifest.join("../..");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    let mut stdout = std::io::stdout();
    let mut generated = String::new();
    writeln!(stdout, "cargo:rerun-if-changed=build.rs").map_err(|error| error.to_string())?;
    for (directory, constant) in SOURCES {
        let dir = root.join(directory);
        writeln!(stdout, "cargo:rerun-if-changed={}", dir.display())
            .map_err(|error| error.to_string())?;
        let files = markdown_files(&dir)?;
        writeln!(
            generated,
            "pub(crate) const {constant}: &[(&str, &str)] = &["
        )
        .map_err(|error| error.to_string())?;
        for (name, path) in files {
            writeln!(stdout, "cargo:rerun-if-changed={}", path.display())
                .map_err(|error| error.to_string())?;
            writeln!(
                generated,
                "    ({name:?}, include_str!({:?})),",
                path.to_string_lossy()
            )
            .map_err(|error| error.to_string())?;
        }
        writeln!(generated, "];").map_err(|error| error.to_string())?;
    }
    fs::write(out_dir.join(OUTPUT), generated).map_err(|error| error.to_string())
}

/// Names and paths of the documentation pages of a directory, by name.
fn markdown_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".md") && !name.starts_with('_') && name != "README.md" {
            let path = fs::canonicalize(entry.path())
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            files.push((name, path));
        }
    }
    files.sort();
    Ok(files)
}
