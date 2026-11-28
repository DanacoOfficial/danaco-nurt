// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-catalog::registry
// Responsibility:   Origin of a command group.
// Dependencies:     nurt_contracts, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::collections::HashMap;
use std::fmt;

use nurt_contracts::error::Result;
use nurt_contracts::spec::{
    CATALOG_FORMAT, CORE_GROUP, CatalogFile, CatalogIndex, CatalogIssue, StageSpec, is_identifier,
};
use serde::Serialize;

use crate::error::{invalid, unknown_command, unknown_group};
use crate::stages::RegisteredImpl;

/// Origin of a command group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GroupOrigin {
    /// The core group, always present.
    Core,
    /// An optional group of the catalog.
    Optional,
    /// A group registered by an extension.
    Extension,
}

/// A command group: its description and its commands in catalog order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    /// Group name.
    pub name: String,
    /// One-sentence description of the group.
    pub summary: String,
    /// Where the group comes from.
    pub origin: GroupOrigin,
    /// Commands of the group, with the group name set.
    pub commands: Vec<StageSpec>,
}

/// Kind of a name in the catalog.
///
/// Commands and groups are separate namespaces: the name `verify` denotes the
/// core command and, in the other namespace, the group of test and analysis
/// commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameKind {
    /// A command name.
    Command,
    /// A group name.
    Group,
}

/// Result of a lookup by name and [`NameKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameEntry<'a> {
    /// The specification of a command.
    Command(&'a StageSpec),
    /// A group.
    Group(&'a Group),
}

/// Registry of commands and of their implementations.
///
/// The registry is filled in two steps. Loading ([`Registry::load_embedded`],
/// [`Registry::load_dir`], [`Registry::from_toml`]) builds the catalog table
/// from the data files. Registration ([`Registry::register_impl`],
/// [`Registry::register_extension`]) adds implementations and extension
/// commands. A registry that is no longer changed is shared by reference or
/// behind an `Arc`; every query takes `&self`.
#[derive(Clone)]
pub struct Registry {
    index: CatalogIndex,
    pub(crate) groups: Vec<Group>,
    /// Catalog files as written, before the group of a command is resolved,
    /// followed by one file per registered extension: the input of the
    /// catalog revision.
    pub(crate) listed: Vec<CatalogFile>,
    group_by_name: HashMap<String, usize>,
    command_by_name: HashMap<String, (usize, usize)>,
    pub(crate) impls: Vec<RegisteredImpl>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("groups", &self.groups.len())
            .field("commands", &self.command_by_name.len())
            .field("implementations", &self.impls.len())
            .finish()
    }
}

impl Registry {
    pub(crate) fn from_files(
        index: CatalogIndex,
        files: Vec<CatalogFile>,
        written: Vec<CatalogFile>,
    ) -> Self {
        let mut registry = Self {
            index,
            groups: Vec::new(),
            listed: written,
            group_by_name: HashMap::new(),
            command_by_name: HashMap::new(),
            impls: Vec::new(),
        };
        for file in files {
            let origin = if file.group == CORE_GROUP {
                GroupOrigin::Core
            } else {
                GroupOrigin::Optional
            };
            registry.push_group(Group {
                name: file.group,
                summary: file.summary,
                origin,
                commands: file.commands,
            });
        }
        registry
    }

    fn push_group(&mut self, group: Group) {
        let group_index = self.groups.len();
        self.group_by_name.insert(group.name.clone(), group_index);
        for (position, command) in group.commands.iter().enumerate() {
            self.command_by_name
                .insert(command.name.clone(), (group_index, position));
        }
        self.groups.push(group);
    }

    /// Returns the index the catalog was loaded with.
    #[must_use]
    pub fn index(&self) -> &CatalogIndex {
        &self.index
    }

    /// Returns the specification of the command with the given name.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<&StageSpec> {
        let &(group, position) = self.command_by_name.get(name)?;
        self.groups.get(group)?.commands.get(position)
    }

    /// Returns the specification of the command with the given name.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_COMMAND` when the catalog has no such command.
    pub fn require_command(&self, name: &str) -> Result<&StageSpec> {
        self.command(name).ok_or_else(|| unknown_command(name))
    }

    /// Returns the group with the given name.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.get(*self.group_by_name.get(name)?)
    }

    /// Returns the group with the given name.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_NAME` when the catalog has no such group.
    pub fn require_group(&self, name: &str) -> Result<&Group> {
        self.group(name).ok_or_else(|| unknown_group(name))
    }

    /// Looks a name up in the namespace of the given kind.
    #[must_use]
    pub fn lookup(&self, name: &str, kind: NameKind) -> Option<NameEntry<'_>> {
        match kind {
            NameKind::Command => self.command(name).map(NameEntry::Command),
            NameKind::Group => self.group(name).map(NameEntry::Group),
        }
    }

    /// Returns the commands of the group, or `None` for an unknown group.
    #[must_use]
    pub fn commands_in(&self, group: &str) -> Option<&[StageSpec]> {
        self.group(group).map(|group| group.commands.as_slice())
    }

    /// Returns the groups in catalog order, core first, extensions last.
    pub fn groups(&self) -> impl Iterator<Item = &Group> {
        self.groups.iter()
    }

    /// Returns every command in catalog order.
    pub fn commands(&self) -> impl Iterator<Item = &StageSpec> {
        self.groups.iter().flat_map(|group| group.commands.iter())
    }

    /// Returns the number of commands, extensions included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.command_by_name.len()
    }

    /// Returns `true` when the registry has no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.command_by_name.is_empty()
    }

    /// Registers the commands of an extension under the extension name.
    ///
    /// Every command name is `<prefix>.<name>`. The prefix must be new: it
    /// must not equal a group or a command of the registry and must not be
    /// the first part of a command name, so an extension cannot replace or
    /// shadow a core name. Each specification passes the checks of the
    /// catalog contract. Nothing is registered when a check fails.
    ///
    /// # Errors
    ///
    /// Returns `E_CATALOG` with every problem found in the `issues` detail.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_catalog::Registry;
    /// use nurt_contracts::spec::StageSpec;
    ///
    /// let mut registry = Registry::load_embedded()?;
    /// let mut spec = registry.command("count").cloned().expect("core command");
    /// spec.name = "metrics.count".to_owned();
    /// spec.group.clear();
    /// registry.register_extension("metrics", "Metrics commands.", vec![spec])?;
    /// assert!(registry.command("metrics.count").is_some());
    ///
    /// let mut core = registry.command("count").cloned().expect("core command");
    /// core.name = "sort".to_owned();
    /// assert!(registry.register_extension("other", "Other commands.", vec![core]).is_err());
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn register_extension(
        &mut self,
        prefix: &str,
        summary: &str,
        commands: Vec<StageSpec>,
    ) -> Result<()> {
        let issues = self.check_extension(prefix, summary, &commands);
        if !issues.is_empty() {
            return Err(invalid(&format!("extension `{prefix}`"), &issues));
        }
        let commands: Vec<StageSpec> = commands
            .into_iter()
            .map(|mut command| {
                command.group = prefix.to_owned();
                command
            })
            .collect();
        self.listed.push(CatalogFile {
            format: CATALOG_FORMAT.to_owned(),
            group: prefix.to_owned(),
            summary: summary.to_owned(),
            commands: commands.clone(),
        });
        self.push_group(Group {
            name: prefix.to_owned(),
            summary: summary.to_owned(),
            origin: GroupOrigin::Extension,
            commands,
        });
        Ok(())
    }

    fn check_extension(
        &self,
        prefix: &str,
        summary: &str,
        commands: &[StageSpec],
    ) -> Vec<CatalogIssue> {
        let issue = |message: String| CatalogIssue::new(None, None, message);
        let mut issues = Vec::new();
        if !is_identifier(prefix) {
            issues.push(issue(format!("prefix `{prefix}` is not an identifier")));
        }
        if !summary.trim().ends_with('.') {
            issues.push(issue(
                "summary must be one sentence ending with '.'".to_owned(),
            ));
        }
        if self.group_by_name.contains_key(prefix) || self.command_by_name.contains_key(prefix) {
            issues.push(issue(format!("prefix `{prefix}` is a name of the catalog")));
        }
        let dotted = format!("{prefix}.");
        if self
            .command_by_name
            .keys()
            .any(|name| name.starts_with(&dotted))
        {
            issues.push(issue(format!(
                "prefix `{prefix}` is used by catalog commands"
            )));
        }
        let mut seen: Vec<&str> = Vec::new();
        for command in commands {
            if !command.name.starts_with(&dotted) {
                issues.push(issue(format!(
                    "command `{}` does not start with `{dotted}`",
                    command.name
                )));
            }
            if !command.group.is_empty() && command.group != prefix {
                issues.push(issue(format!(
                    "command `{}` names group `{}` instead of `{prefix}`",
                    command.name, command.group
                )));
            }
            if seen.contains(&command.name.as_str()) {
                issues.push(issue(format!("command `{}` is given twice", command.name)));
            }
            seen.push(&command.name);
            issues.extend(command.check());
        }
        issues
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
