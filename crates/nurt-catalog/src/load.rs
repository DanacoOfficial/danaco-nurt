// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::fs;
use std::path::Path;

use nurt_contracts::error::{Error, Result};
use nurt_contracts::spec::{CatalogFile, CatalogIndex, CatalogIssue, check_catalog};

use crate::error::{catalog_message, invalid};
use crate::registry::Registry;

/// File name of the catalog index.
const INDEX_FILE: &str = "_index.toml";

macro_rules! embedded_catalog_file {
    ($file:literal) => {
        (
            $file,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../catalog/",
                $file
            )),
        )
    };
}

const EMBEDDED_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../catalog/_index.toml"
));

const EMBEDDED_FILES: &[(&str, &str)] = &[
    embedded_catalog_file!("core.toml"),
    embedded_catalog_file!("documents.toml"),
    embedded_catalog_file!("text.toml"),
    embedded_catalog_file!("code.toml"),
    embedded_catalog_file!("notebook.toml"),
    embedded_catalog_file!("git.toml"),
    embedded_catalog_file!("package.toml"),
    embedded_catalog_file!("archive.toml"),
    embedded_catalog_file!("network.toml"),
    embedded_catalog_file!("system.toml"),
    embedded_catalog_file!("administration.toml"),
    embedded_catalog_file!("schedule.toml"),
    embedded_catalog_file!("program.toml"),
    embedded_catalog_file!("journal.toml"),
    embedded_catalog_file!("verify.toml"),
];

fn parse_index(text: &str) -> Result<CatalogIndex> {
    let index: CatalogIndex = toml::from_str(text)
        .map_err(|error| catalog_message(format!("{INDEX_FILE}: {}", error.message())))?;
    let issues = index.check();
    if issues.is_empty() {
        Ok(index)
    } else {
        Err(invalid("catalog index", &issues))
    }
}

fn assemble(index: CatalogIndex, sources: &[(&str, &str)]) -> Result<Registry> {
    let mut issues: Vec<CatalogIssue> = Vec::new();
    let mut files: Vec<CatalogFile> = Vec::new();
    let mut written: Vec<CatalogFile> = Vec::new();
    for entry in &index.files {
        let Some((_, text)) = sources.iter().find(|(name, _)| *name == entry.file) else {
            issues.push(CatalogIssue::new(
                None,
                None,
                format!("file {} is listed in the index but missing", entry.file),
            ));
            continue;
        };
        match toml::from_str::<CatalogFile>(text) {
            Ok(mut file) => {
                if file.group != entry.name {
                    issues.push(CatalogIssue::new(
                        None,
                        None,
                        format!(
                            "file {} has group {}, the index lists {}",
                            entry.file, file.group, entry.name
                        ),
                    ));
                }
                written.push(file.clone());
                file.resolve_groups();
                files.push(file);
            }
            Err(error) => issues.push(CatalogIssue::new(
                None,
                None,
                format!("{}: {}", entry.file, error.message()),
            )),
        }
    }
    issues.extend(check_catalog(&index, &files));
    if issues.is_empty() {
        Ok(Registry::from_files(index, files, written))
    } else {
        Err(invalid("catalog", &issues))
    }
}

/// The embedded catalog data: the text of `_index.toml` and the pairs of
/// file name and text of the catalog files, in the order of the index.
///
/// Used by components that read the catalog in their own form.
#[must_use]
pub const fn embedded_sources() -> (&'static str, &'static [(&'static str, &'static str)]) {
    (EMBEDDED_INDEX, EMBEDDED_FILES)
}

impl Registry {
    /// Loads the catalog compiled into the executable.
    ///
    /// # Errors
    ///
    /// Returns `E_CATALOG` when the embedded catalog is inconsistent. The
    /// catalog data files are checked by the tests of this crate, so the
    /// error indicates a damaged build.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_catalog::Registry;
    ///
    /// let registry = Registry::load_embedded()?;
    /// assert_eq!(registry.len(), usize::try_from(registry.index().total)?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn load_embedded() -> Result<Self> {
        Self::from_toml(EMBEDDED_INDEX, EMBEDDED_FILES)
    }

    /// Loads a catalog from a directory with the layout of the catalog data:
    /// the index `_index.toml` and the files that it lists.
    ///
    /// # Errors
    ///
    /// Returns `E_CATALOG` when a file cannot be read or parsed, when the
    /// index or a file is inconsistent, or when the command counts differ from
    /// the index. The `issues` detail lists every problem found.
    pub fn load_dir(path: impl AsRef<Path>) -> Result<Self> {
        let dir = path.as_ref();
        let index_text = read(dir, INDEX_FILE)?;
        let index = parse_index(&index_text)?;
        let mut texts: Vec<(String, String)> = Vec::new();
        for entry in &index.files {
            texts.push((entry.file.clone(), read(dir, &entry.file)?));
        }
        let unlisted = unlisted_files(dir, &index)?;
        if !unlisted.is_empty() {
            return Err(invalid("catalog", &unlisted));
        }
        let sources: Vec<(&str, &str)> = texts
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str()))
            .collect();
        assemble(index, &sources)
    }

    /// Builds a catalog from the text of the index and of the catalog files.
    ///
    /// `files` pairs a file name as listed in the index with its text. The
    /// constructor serves loaders that do not read a directory, such as tests
    /// of other crates with a small synthetic catalog.
    ///
    /// # Errors
    ///
    /// Returns `E_CATALOG` under the conditions of [`Registry::load_dir`].
    pub fn from_toml(index: &str, files: &[(&str, &str)]) -> Result<Self> {
        assemble(parse_index(index)?, files)
    }
}

fn unlisted_files(dir: &Path, index: &CatalogIndex) -> Result<Vec<CatalogIssue>> {
    let entries = fs::read_dir(dir)
        .map_err(|error| catalog_message(format!("cannot list the catalog directory: {error}")))?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let listed = index.files.iter().any(|known| known.file == name);
        if name.ends_with(".toml") && name != INDEX_FILE && !listed {
            names.push(name);
        }
    }
    names.sort();
    Ok(names
        .into_iter()
        .map(|name| {
            CatalogIssue::new(
                None,
                None,
                format!("file {name} is not listed in the index"),
            )
        })
        .collect())
}

fn read(dir: &Path, name: &str) -> Result<String> {
    fs::read_to_string(dir.join(name))
        .map_err(|error| -> Error { catalog_message(format!("cannot read {name}: {error}")) })
}
