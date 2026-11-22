// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Loading of the command catalog (`catalog/*.toml`, format `nurt-catalog/1`).

use std::path::Path;

use nurt_contracts::spec::{CatalogFile, CatalogIndex, StageSpec, check_catalog};

use crate::error::{DocgenError, Result, read_text};

/// Name of the group of the core commands.
pub const CORE_GROUP: &str = "core";

/// Name of the call parameters that belong to the request envelope and not to a command.
pub const CALL_PARAMETERS: &[&str] = &[
    "preview",
    "base",
    "limit",
    "cursor",
    "fields",
    "format",
    "verbosity",
    "diff",
    "diff_context",
    "max_chars",
    "save_to",
    "timeout",
    "session",
    "language",
    "script",
];

/// The command catalog: the index and the groups with their commands.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// Index of the catalog files.
    pub index: CatalogIndex,
    /// Catalog files in the order of the index, core first.
    pub groups: Vec<CatalogFile>,
}

impl Catalog {
    /// Loads the catalog from a directory with `_index.toml` and the group files.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be read or parsed, or when the
    /// catalog fails the consistency checks of `nurt-contracts`.
    pub fn load(dir: &Path) -> Result<Self> {
        let index_path = dir.join("_index.toml");
        let index: CatalogIndex = parse_toml(&index_path)?;
        let mut groups = Vec::with_capacity(index.files.len());
        for entry in &index.files {
            let path = dir.join(&entry.file);
            let mut file: CatalogFile = parse_toml(&path)?;
            file.resolve_groups();
            groups.push(file);
        }
        let issues = check_catalog(&index, &groups);
        if let Some(first) = issues.first() {
            return Err(DocgenError::invalid(
                dir.display().to_string(),
                format!(
                    "catalog check failed ({} problems), first: {first}",
                    issues.len()
                ),
            ));
        }
        Ok(Self { index, groups })
    }

    /// Returns all commands in catalog order.
    pub fn commands(&self) -> impl Iterator<Item = &StageSpec> {
        self.groups.iter().flat_map(|group| group.commands.iter())
    }

    /// Returns the command with the given full name.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<&StageSpec> {
        self.commands().find(|command| command.name == name)
    }

    /// Returns the number of all commands.
    #[must_use]
    pub fn total(&self) -> usize {
        self.commands().count()
    }

    /// Returns the number of the commands of a group.
    #[must_use]
    pub fn group_size(&self, group: &str) -> usize {
        self.groups
            .iter()
            .filter(|file| file.group == group)
            .map(|file| file.commands.len())
            .sum()
    }

    /// Returns the number of the optional groups (all groups but the core).
    #[must_use]
    pub fn optional_groups(&self) -> usize {
        self.groups
            .iter()
            .filter(|file| file.group != CORE_GROUP)
            .count()
    }

    /// Returns the summary of a group.
    #[must_use]
    pub fn group_summary(&self, group: &str) -> Option<&str> {
        self.groups
            .iter()
            .find(|file| file.group == group)
            .map(|file| file.summary.as_str())
    }
}

fn parse_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = read_text(path)?;
    toml::from_str(&text)
        .map_err(|error| DocgenError::invalid(path.display().to_string(), error.to_string()))
}

/// Returns the lower-case keyword of an enumeration that serializes as a string.
#[must_use]
pub fn keyword<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => String::new(),
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::Catalog;
    use nurt_contracts::spec::{CatalogFile, CatalogIndex};

    const INDEX: &str = r#"
format = "nurt-catalog-index/1"
total = 5

[[file]]
name = "core"
file = "core.toml"
summary = "Core commands."
commands = 4

[[file]]
name = "git"
file = "git.toml"
summary = "Git repositories."
commands = 1
"#;

    const CORE: &str = r#"
format = "nurt-catalog/1"
group = "core"
summary = "Core commands."

[[command]]
name = "find"
category = "source"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["none"]
output = ["entry"]
summary = "Find entries under a directory."

[[command.param]]
name = "path"
type = "path"
position = 1
required = true
summary = "Directory to search."

[[command.param]]
name = "glob"
type = "glob | list<glob>"
position = 2
default = '"**"'
summary = "Name patterns."

[[command.param]]
name = "type"
type = "enum(file|directory|any)"
default = "file"
values = ["file", "directory", "any"]
summary = "Kind of entries."

[[command.param]]
name = "depth"
type = "int"
summary = "Number of levels."

[[command]]
name = "filter"
category = "transform"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["any"]
output = ["same"]
summary = "Keep the records that satisfy a condition."

[[command.param]]
name = "condition"
type = "expression"
position = 1
required = true
summary = "Condition evaluated on each record."

[[command]]
name = "remove"
category = "change"
reversibility = "reversible"
effect = "local"
implementation = "builtin"
input = ["none", "entry"]
output = ["effect"]
summary = "Remove files and directories."

[[command.param]]
name = "path"
type = "list<path>"
position = 1
required = true
repeated = true
summary = "Paths to remove."

[[command.param]]
name = "on_missing"
type = "enum(skip|error)"
default = "skip"
values = ["skip", "error"]
summary = "Behaviour for a missing path."

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

    const GIT: &str = r#"
format = "nurt-catalog/1"
group = "git"
summary = "Git repositories."

[[command]]
name = "git.status"
category = "source"
reversibility = "read"
effect = "read"
implementation = "builtin"
input = ["none"]
output = ["git_status"]
summary = "Show the working tree status."

[[command.param]]
name = "repo"
type = "list<path>"
position = 1
required = true
summary = "Repository path."
"#;

    pub(crate) fn catalog() -> Catalog {
        let index: CatalogIndex = toml::from_str(INDEX).expect("index fixture parses");
        let mut groups: Vec<CatalogFile> = [CORE, GIT]
            .iter()
            .map(|text| toml::from_str(text).expect("group fixture parses"))
            .collect();
        for group in &mut groups {
            group.resolve_groups();
        }
        Catalog { index, groups }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_commands_and_groups() {
        let catalog = fixtures::catalog();
        assert_eq!(catalog.total(), 5);
        assert_eq!(catalog.group_size(CORE_GROUP), 4);
        assert_eq!(catalog.optional_groups(), 1);
    }

    #[test]
    fn finds_command_by_full_name() {
        let catalog = fixtures::catalog();
        assert!(catalog.command("git.status").is_some());
        assert!(catalog.command("git.missing").is_none());
    }

    #[test]
    fn resolves_group_of_commands() {
        let catalog = fixtures::catalog();
        assert_eq!(
            catalog.command("find").map(|c| c.group.as_str()),
            Some("core")
        );
    }

    #[test]
    fn keyword_of_enumeration_is_snake_case() {
        let catalog = fixtures::catalog();
        let find = catalog.command("remove").expect("fixture command");
        assert_eq!(keyword(&find.reversibility), "reversible");
    }

    #[test]
    fn rejects_missing_index() {
        let dir = tempfile::tempdir().expect("temporary directory");
        assert!(Catalog::load(dir.path()).is_err());
    }
}
