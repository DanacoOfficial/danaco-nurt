// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Model of the journal coverage table `tables/journal-coverage.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::journal::PreserveMode;
use nurt_contracts::report::Irreversible;
use serde::Deserialize;

/// Format identifier of the table.
pub(crate) const FORMAT: &str = "nurt-journal-coverage/1";

/// Coverage value of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Coverage {
    /// Undo restores content and metadata exactly.
    Full,
    /// Part of the effect or part of the cases is not preserved.
    Partial,
    /// The effect is outside the journal.
    #[serde(rename = "none")]
    Uncovered,
    /// The operation changes nothing.
    NoEffect,
}

/// One row of the table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    /// Catalog command name.
    pub(crate) command: String,
    /// Action, mode or variant; `None` for the default form.
    #[serde(default)]
    pub(crate) action: Option<String>,
    /// Catalog group.
    pub(crate) group: String,
    /// What the operation changes.
    pub(crate) changes: String,
    /// Preservation modes of the entry.
    pub(crate) preserve: Vec<PreserveMode>,
    /// Coverage value.
    pub(crate) coverage: Coverage,
    /// Reasons of the row in addition to the generic ones.
    #[serde(default)]
    pub(crate) irreversible: Vec<Irreversible>,
    /// Description of the exception.
    #[serde(default)]
    pub(crate) exception: String,
    /// Clarification.
    #[serde(default)]
    #[expect(dead_code, reason = "read for completeness of the deserialized table")]
    pub(crate) note: String,
}

impl Row {
    /// Key of the row.
    pub(crate) fn key(&self) -> (&str, Option<&str>) {
        (self.command.as_str(), self.action.as_deref())
    }

    /// Label for messages.
    pub(crate) fn label(&self) -> String {
        match &self.action {
            Some(action) => format!("{} {action}", self.command),
            None => self.command.clone(),
        }
    }
}

/// The whole table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoverageTable {
    /// Format identifier.
    pub(crate) format: String,
    /// Reasons that may degrade any row.
    pub(crate) generic_irreversible: Vec<Irreversible>,
    /// Rows.
    pub(crate) row: Vec<Row>,
}

impl CoverageTable {
    /// Row with the given key.
    pub(crate) fn find(&self, command: &str, action: Option<&str>) -> Option<&Row> {
        self.row.iter().find(|row| row.key() == (command, action))
    }
}

/// Root of the repository.
pub(crate) fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Reads and parses the table.
pub(crate) fn load() -> CoverageTable {
    let path = repository_root().join("tables/journal-coverage.toml");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}
