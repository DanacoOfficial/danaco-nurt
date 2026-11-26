// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Catalog adapter of the help crate.
//!
//! The adapter loads the catalog files into the contract types, resolves the
//! group of every command and pairs the catalog with the documentation
//! pages. It performs no file access: the caller supplies the text of the
//! index and of the files, which a build embeds or a test reads.

use std::fmt;
use std::sync::{Arc, OnceLock};

use nurt_contracts::spec::{
    CORE_GROUP, CatalogFile, CatalogIndex, CatalogIssue, StageSpec, check_catalog,
};

use crate::docs::Docs;

/// Error of loading a catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogError {
    /// A file is not valid TOML of the catalog format.
    Parse {
        /// Name of the file.
        source: String,
        /// What is wrong.
        message: String,
    },
    /// The files do not agree with the index or with the catalog contract.
    Invalid(Vec<CatalogIssue>),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { source, message } => write!(f, "{source}: {message}"),
            Self::Invalid(issues) => {
                write!(f, "the catalog is inconsistent")?;
                for issue in issues {
                    write!(f, "; {issue}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// The command catalog with its documentation.
#[derive(Debug, Clone)]
pub struct Catalog {
    index: CatalogIndex,
    files: Vec<CatalogFile>,
    docs: Docs,
}

impl Catalog {
    /// Builds a catalog from the text of the index and of the group files.
    ///
    /// The set of files may be incomplete; every file present must agree
    /// with the index.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::Parse`] for a file that does not parse and
    /// [`CatalogError::Invalid`] when the files violate the catalog contract.
    pub fn from_sources(
        index: &str,
        files: &[(&str, &str)],
        docs: Docs,
    ) -> Result<Self, CatalogError> {
        let index: CatalogIndex = toml::from_str(index).map_err(|error| CatalogError::Parse {
            source: "_index.toml".to_owned(),
            message: error.to_string(),
        })?;
        let mut parsed = Vec::with_capacity(files.len());
        for (source, text) in files {
            let mut file: CatalogFile =
                toml::from_str(text).map_err(|error| CatalogError::Parse {
                    source: (*source).to_owned(),
                    message: error.to_string(),
                })?;
            file.resolve_groups();
            parsed.push(file);
        }
        parsed.sort_by_key(|file| {
            index
                .files
                .iter()
                .position(|entry| entry.name == file.group)
                .unwrap_or(usize::MAX)
        });
        let issues = check_catalog(&index, &parsed);
        if !issues.is_empty() {
            return Err(CatalogError::Invalid(issues));
        }
        Ok(Self {
            index,
            files: parsed,
            docs,
        })
    }

    /// The catalog and the documentation compiled into the executable,
    /// built on first use and shared afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError`] when the embedded catalog or documentation
    /// is inconsistent, which indicates a damaged build.
    pub fn embedded() -> Result<Arc<Self>, CatalogError> {
        static EMBEDDED: OnceLock<Result<Arc<Catalog>, CatalogError>> = OnceLock::new();
        EMBEDDED
            .get_or_init(|| {
                let docs = Docs::embedded().map_err(|error| CatalogError::Parse {
                    source: error.source,
                    message: error.message,
                })?;
                let (index, files) = nurt_catalog::embedded_sources();
                Self::from_sources(index, files, docs).map(Arc::new)
            })
            .clone()
    }

    /// Documentation pages and guides.
    #[must_use]
    pub fn docs(&self) -> &Docs {
        &self.docs
    }

    /// Group files in the order of the index, core first.
    pub fn groups(&self) -> impl Iterator<Item = &CatalogFile> {
        self.files.iter()
    }

    /// Group file by name.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&CatalogFile> {
        self.files.iter().find(|file| file.group == name)
    }

    /// Commands of the core group.
    #[must_use]
    pub fn core_commands(&self) -> &[StageSpec] {
        self.group(CORE_GROUP)
            .map_or(&[][..], |file| file.commands.as_slice())
    }

    /// All commands, in the order of the index.
    pub fn commands(&self) -> impl Iterator<Item = &StageSpec> {
        self.files.iter().flat_map(|file| file.commands.iter())
    }

    /// Command by full name.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<&StageSpec> {
        self.commands().find(|command| command.name == name)
    }

    /// Number of commands in the loaded files.
    #[must_use]
    pub fn command_count(&self) -> usize {
        self.files.iter().map(|file| file.commands.len()).sum()
    }

    /// Index of the catalog files.
    #[must_use]
    pub fn index(&self) -> &CatalogIndex {
        &self.index
    }
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
name = "extra"
file = "extra.toml"
summary = "Extra commands."
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

    const EXTRA: &str = r#"
format = "nurt-catalog/1"
group = "extra"
summary = "Extra commands."

[[command]]
name = "extra.show"
category = "source"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["none"]
output = ["text"]
summary = "Show the extra data."
"#;

    #[test]
    fn groups_follow_the_order_of_the_index() {
        let catalog = Catalog::from_sources(
            INDEX,
            &[("extra.toml", EXTRA), ("core.toml", CORE)],
            Docs::empty(),
        )
        .unwrap();
        let names: Vec<&str> = catalog.groups().map(|file| file.group.as_str()).collect();
        assert_eq!(names, ["core", "extra"]);
        assert_eq!(catalog.command_count(), 2);
        assert_eq!(catalog.core_commands().len(), 1);
    }

    #[test]
    fn command_group_is_resolved_from_the_file() {
        let catalog = Catalog::from_sources(INDEX, &[("core.toml", CORE)], Docs::empty()).unwrap();
        assert_eq!(catalog.command("count").unwrap().group, "core");
        assert!(catalog.command("missing").is_none());
    }

    #[test]
    fn invalid_toml_is_reported_with_the_file_name() {
        let error =
            Catalog::from_sources(INDEX, &[("core.toml", "x = ")], Docs::empty()).unwrap_err();
        assert!(matches!(&error, CatalogError::Parse { source, .. } if source == "core.toml"));
    }

    #[test]
    fn count_that_differs_from_the_index_is_rejected() {
        let wrong = INDEX.replace("commands = 1\n\n[[file]]", "commands = 2\n\n[[file]]");
        let error =
            Catalog::from_sources(&wrong, &[("core.toml", CORE)], Docs::empty()).unwrap_err();
        assert!(matches!(error, CatalogError::Invalid(_)));
    }
}
