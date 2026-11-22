// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::kinds::{Category, EffectKind, Implementation, Reversibility};
use nurt_contracts::spec::StageSpec;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::registry::{Group, GroupOrigin, Registry};

/// Value of the `format` member of every result of [`Registry::catalog_json`].
pub const CATALOG_RESULT_FORMAT: &str = "nurt-catalog-result/1";

/// Prefix of the catalog revision.
const REVISION_PREFIX: &str = "sha256:";

/// What a catalog result describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogQuery<'a> {
    /// The groups with the number of their commands.
    Summary,
    /// The commands of one group.
    Group(&'a str),
    /// The full specification of one command.
    Command(&'a str),
}

#[derive(Serialize)]
struct SummaryResult<'a> {
    format: &'static str,
    catalog_rev: &'a str,
    total: usize,
    groups: Vec<GroupBrief<'a>>,
}

#[derive(Serialize)]
struct GroupBrief<'a> {
    name: &'a str,
    summary: &'a str,
    origin: GroupOrigin,
    commands: usize,
}

#[derive(Serialize)]
struct GroupResult<'a> {
    format: &'static str,
    catalog_rev: &'a str,
    group: GroupBrief<'a>,
    commands: Vec<CommandBrief<'a>>,
}

#[derive(Serialize)]
struct CommandBrief<'a> {
    name: &'a str,
    summary: &'a str,
    category: Category,
    effect: EffectKind,
    reversibility: Reversibility,
    implementation: Implementation,
}

#[derive(Serialize)]
struct CommandResult<'a> {
    format: &'static str,
    catalog_rev: &'a str,
    command: &'a StageSpec,
}

#[derive(Serialize)]
struct RevisionInput<'a> {
    index: &'a nurt_contracts::spec::CatalogIndex,
    groups: &'a [Group],
}

fn brief(group: &Group) -> GroupBrief<'_> {
    GroupBrief {
        name: &group.name,
        summary: &group.summary,
        origin: group.origin,
        commands: group.commands.len(),
    }
}

fn command_brief(spec: &StageSpec) -> CommandBrief<'_> {
    CommandBrief {
        name: &spec.name,
        summary: &spec.summary,
        category: spec.category,
        effect: spec.effect,
        reversibility: spec.reversibility,
        implementation: spec.implementation,
    }
}

fn internal(message: &str, error: &dyn std::fmt::Display) -> Error {
    Error::new(ErrorCode::Internal, format!("{message}: {error}"))
}

fn to_value(result: &impl Serialize) -> Result<Value> {
    serde_json::to_value(result).map_err(|error| internal("cannot build catalog result", &error))
}

impl Registry {
    /// Returns the revision of the loaded catalog.
    ///
    /// The revision is `sha256:` followed by the hexadecimal SHA-256 digest of
    /// the canonical JSON (RFC 8785) of the index and of every group with its
    /// commands, extensions included. It does not depend on the order of the
    /// members in the data files, on white space, or on the process, and it
    /// changes with any change of a command, a parameter or a group.
    ///
    /// # Errors
    ///
    /// Returns `E_INTERNAL` when the catalog cannot be serialized.
    pub fn catalog_rev(&self) -> Result<String> {
        let input = RevisionInput {
            index: self.index(),
            groups: &self.groups,
        };
        let canonical = serde_json_canonicalizer::to_vec(&input)
            .map_err(|error| internal("cannot canonicalize the catalog", &error))?;
        let digest = Sha256::digest(&canonical);
        Ok(format!("{REVISION_PREFIX}{}", hex::encode(digest)))
    }

    /// Builds the JSON result of the catalog query.
    ///
    /// The result has the members `format` ([`CATALOG_RESULT_FORMAT`]),
    /// `catalog_rev` and the content of the query: `total` and `groups` for
    /// [`CatalogQuery::Summary`], `group` and `commands` for
    /// [`CatalogQuery::Group`], `command` for [`CatalogQuery::Command`]. The
    /// result is deterministic: the same catalog gives the same value.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_NAME` for an unknown group and `E_UNKNOWN_COMMAND`
    /// for an unknown command.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_catalog::{CatalogQuery, Registry};
    ///
    /// let registry = Registry::load_embedded()?;
    /// let result = registry.catalog_json(&CatalogQuery::Command("remove"))?;
    /// assert_eq!(result["command"]["name"], "remove");
    /// assert!(result["catalog_rev"].as_str().is_some_and(|rev| rev.starts_with("sha256:")));
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn catalog_json(&self, what: &CatalogQuery<'_>) -> Result<Value> {
        let rev = self.catalog_rev()?;
        match *what {
            CatalogQuery::Summary => to_value(&SummaryResult {
                format: CATALOG_RESULT_FORMAT,
                catalog_rev: &rev,
                total: self.len(),
                groups: self.groups().map(brief).collect(),
            }),
            CatalogQuery::Group(name) => {
                let group = self.require_group(name)?;
                to_value(&GroupResult {
                    format: CATALOG_RESULT_FORMAT,
                    catalog_rev: &rev,
                    group: brief(group),
                    commands: group.commands.iter().map(command_brief).collect(),
                })
            }
            CatalogQuery::Command(name) => to_value(&CommandResult {
                format: CATALOG_RESULT_FORMAT,
                catalog_rev: &rev,
                command: self.require_command(name)?,
            }),
        }
    }
}
