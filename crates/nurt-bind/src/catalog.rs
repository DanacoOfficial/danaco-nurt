// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bind::catalog
// Responsibility:   Command catalog used by the binder.
// Dependencies:     nurt_contracts, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Command catalog used by the binder.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use nurt_contracts::spec::{CatalogFile, CatalogIndex, CatalogIssue, StageSpec, check_catalog};
use sha2::{Digest, Sha256};

/// File name of the catalog index in a catalog directory.
pub const INDEX_FILE: &str = "_index.toml";

/// Prefix of a catalog revision.
pub const REVISION_PREFIX: &str = "sha256:";

/// Command catalog: the signatures of all commands, indexed by name.
///
/// The catalog is read from the files of format `nurt-catalog/1` and checked
/// for consistency when it is built; an inconsistent catalog is rejected.
/// Its revision, recorded as `catalog_rev` in every bound tree, is
/// `sha256:` followed by the lowercase hex SHA-256 digest of the canonical
/// JSON (RFC 8785) of the array of catalog files in index order.
///
/// # Examples
///
/// ```
/// use nurt_bind::Catalog;
///
/// let index = r#"
/// format = "nurt-catalog-index/1"
/// total = 1
///
/// [[file]]
/// name = "core"
/// file = "core.toml"
/// summary = "Core commands."
/// commands = 1
/// "#;
/// let core = r#"
/// format = "nurt-catalog/1"
/// group = "core"
/// summary = "Core commands."
///
/// [[command]]
/// name = "count"
/// category = "transform"
/// reversibility = "read"
/// effect = "read"
/// implementation = "builtin"
/// input = ["any"]
/// output = ["int"]
/// summary = "Count the input records."
/// "#;
/// let catalog = Catalog::from_toml(index, &[core])?;
/// assert_eq!(catalog.len(), 1);
/// assert!(catalog.command("count").is_some());
/// assert!(catalog.revision().starts_with("sha256:"));
/// # Ok::<(), nurt_bind::CatalogError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Catalog {
    commands: Vec<StageSpec>,
    by_name: HashMap<String, usize>,
    revision: String,
}

impl Catalog {
    /// Builds a catalog from its index and its files.
    ///
    /// The files are ordered by the index; every command carries the group of
    /// its file.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::Inconsistent`] with every problem found by
    /// [`check_catalog`], and [`CatalogError::Revision`] when the files cannot
    /// be serialized for the revision.
    pub fn new(index: &CatalogIndex, mut files: Vec<CatalogFile>) -> Result<Self, CatalogError> {
        let issues = check_catalog(index, &files);
        if !issues.is_empty() {
            return Err(CatalogError::Inconsistent { issues });
        }
        let position = |file: &CatalogFile| {
            index
                .files
                .iter()
                .position(|entry| entry.name == file.group)
                .unwrap_or(usize::MAX)
        };
        files.sort_by_key(position);
        let revision = revision(&files)?;
        let mut commands = Vec::new();
        for mut file in files {
            file.resolve_groups();
            commands.extend(file.commands);
        }
        let by_name = commands
            .iter()
            .enumerate()
            .map(|(position, command)| (command.name.clone(), position))
            .collect();
        Ok(Self {
            commands,
            by_name,
            revision,
        })
    }

    /// Builds a catalog from the text of its index and of its files.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::Parse`] when a text is not a valid catalog
    /// document, and the errors of [`Catalog::new`].
    pub fn from_toml(index: &str, files: &[&str]) -> Result<Self, CatalogError> {
        let index: CatalogIndex = parse(INDEX_FILE, index)?;
        let files = files
            .iter()
            .enumerate()
            .map(|(position, text)| parse(&format!("file {}", position + 1), text))
            .collect::<Result<Vec<CatalogFile>, CatalogError>>()?;
        Self::new(&index, files)
    }

    /// Reads the catalog from a directory with the index [`INDEX_FILE`] and
    /// the files it lists.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::Read`] when a file cannot be read,
    /// [`CatalogError::Parse`] when it is not a valid catalog document, and
    /// the errors of [`Catalog::new`].
    pub fn load_dir(dir: &Path) -> Result<Self, CatalogError> {
        let index: CatalogIndex = parse(INDEX_FILE, &read(dir, INDEX_FILE)?)?;
        let files = index
            .files
            .iter()
            .map(|entry| parse(&entry.file, &read(dir, &entry.file)?))
            .collect::<Result<Vec<CatalogFile>, CatalogError>>()?;
        Self::new(&index, files)
    }

    /// Returns the catalog revision, `sha256:` followed by 64 lowercase hex
    /// digits.
    #[must_use]
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Returns the command with the given canonical name.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<&StageSpec> {
        self.by_name
            .get(name)
            .and_then(|position| self.commands.get(*position))
    }

    /// Returns every command, in index order and file order.
    #[must_use]
    pub fn commands(&self) -> &[StageSpec] {
        &self.commands
    }

    /// Returns the number of commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns `true` when the catalog has no command.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

/// Error of reading or checking a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogError {
    /// A catalog file cannot be read.
    Read {
        /// File name.
        file: String,
        /// Reason reported by the system.
        message: String,
    },
    /// A catalog file is not a valid `nurt-catalog/1` document.
    Parse {
        /// File name.
        file: String,
        /// Reason reported by the parser.
        message: String,
    },
    /// The catalog is not consistent.
    Inconsistent {
        /// Every problem found.
        issues: Vec<CatalogIssue>,
    },
    /// The catalog cannot be serialized for its revision.
    Revision {
        /// Reason reported by the serializer.
        message: String,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { file, message } => write!(f, "cannot read catalog file {file}: {message}"),
            Self::Parse { file, message } => {
                write!(f, "catalog file {file} is not valid: {message}")
            }
            Self::Inconsistent { issues } => {
                write!(f, "the catalog is not consistent: ")?;
                for (position, issue) in issues.iter().enumerate() {
                    if position > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{issue}")?;
                }
                Ok(())
            }
            Self::Revision { message } => {
                write!(f, "cannot compute the catalog revision: {message}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

fn read(dir: &Path, file: &str) -> Result<String, CatalogError> {
    std::fs::read_to_string(dir.join(file)).map_err(|error| CatalogError::Read {
        file: file.to_owned(),
        message: error.to_string(),
    })
}

fn parse<T: serde::de::DeserializeOwned>(file: &str, text: &str) -> Result<T, CatalogError> {
    toml::from_str(text).map_err(|error| CatalogError::Parse {
        file: file.to_owned(),
        message: error.to_string(),
    })
}

fn revision(files: &[CatalogFile]) -> Result<String, CatalogError> {
    let canonical =
        serde_json_canonicalizer::to_string(&files).map_err(|error| CatalogError::Revision {
            message: error.to_string(),
        })?;
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(format!("{REVISION_PREFIX}{}", hex::encode(digest)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"
format = "nurt-catalog-index/1"
total = 2

[[file]]
name = "core"
file = "core.toml"
summary = "Core commands."
commands = 1

[[file]]
name = "text"
file = "text.toml"
summary = "Text commands."
commands = 1
"#;

    const CORE: &str = r#"
format = "nurt-catalog/1"
group = "core"
summary = "Core commands."

[[command]]
name = "count"
category = "transform"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["any"]
output = ["int"]
summary = "Count the input records."
"#;

    const TEXT: &str = r#"
format = "nurt-catalog/1"
group = "text"
summary = "Text commands."

[[command]]
name = "upper"
category = "transform"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["any"]
output = ["same"]
summary = "Upper-case the input text."
"#;

    #[test]
    fn orders_files_by_the_index_and_resolves_groups() {
        let catalog = Catalog::from_toml(INDEX, &[TEXT, CORE]).unwrap();
        let names: Vec<&str> = catalog.commands().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["count", "upper"]);
        assert_eq!(catalog.command("upper").unwrap().group, "text");
        assert!(!catalog.is_empty());
    }

    #[test]
    fn revision_does_not_depend_on_the_order_of_files() {
        let first = Catalog::from_toml(INDEX, &[CORE, TEXT]).unwrap();
        let second = Catalog::from_toml(INDEX, &[TEXT, CORE]).unwrap();
        assert_eq!(first.revision(), second.revision());
        assert_eq!(first.revision().len(), REVISION_PREFIX.len() + 64);
    }

    #[test]
    fn revision_changes_with_a_description() {
        let first = Catalog::from_toml(INDEX, &[CORE, TEXT]).unwrap();
        let changed = TEXT.replace("Upper-case the input text.", "Upper-case text.");
        let second = Catalog::from_toml(INDEX, &[CORE, &changed]).unwrap();
        assert_ne!(first.revision(), second.revision());
    }

    #[test]
    fn rejects_an_incomplete_catalog() {
        let error = Catalog::from_toml(INDEX, &[CORE, CORE]).unwrap_err();
        assert!(matches!(error, CatalogError::Inconsistent { .. }));
        assert!(
            error
                .to_string()
                .starts_with("the catalog is not consistent")
        );
    }

    #[test]
    fn rejects_an_unknown_key() {
        let broken = CORE.replace("summary = \"Count", "sumary = \"Count");
        let error = Catalog::from_toml(INDEX, &[&broken, TEXT]).unwrap_err();
        assert!(matches!(error, CatalogError::Parse { .. }));
    }

    #[test]
    fn reports_a_missing_directory() {
        let dir = std::env::temp_dir().join("nurt-bind-missing-catalog-directory");
        let error = Catalog::load_dir(&dir).unwrap_err();
        assert!(matches!(error, CatalogError::Read { .. }));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
