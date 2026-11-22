// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Options of rendering: description, language, groups and catalog version.

use std::collections::BTreeMap;
use std::str::FromStr;

use nurt_contracts::envelope::Language;

use crate::error::ToolDefError;

/// Name of the group that is always described.
pub const CORE_GROUP: &str = "core";

/// Name that selects every group with a description.
const ALL_GROUPS: &str = "all";

/// Separator of group names in the text form of [`GroupSelection`].
const GROUP_SEPARATOR: char = ',';

/// Separator of the parts of the assembled description.
const PART_SEPARATOR: &str = "\n\n";

/// Groups of commands whose descriptions are included in the tool
/// description, in addition to the core group.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum GroupSelection {
    /// The core group only.
    #[default]
    Core,
    /// Every group for which [`RenderOptions::with_group_description`]
    /// supplied a description, in the order of group names.
    All,
    /// The listed groups, in the given order; the core group is always
    /// included and is not listed.
    Only(Vec<String>),
}

impl FromStr for GroupSelection {
    type Err = ToolDefError;

    /// Parses `core`, `all` or a comma-separated list of group names.
    ///
    /// # Errors
    ///
    /// [`ToolDefError::InvalidGroupName`] when a listed name is empty or
    /// contains characters other than lowercase ASCII letters, digits and
    /// `-`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_tooldef::GroupSelection;
    ///
    /// assert_eq!("all".parse(), Ok(GroupSelection::All));
    /// assert_eq!(
    ///     "git,archive".parse(),
    ///     Ok(GroupSelection::Only(vec!["git".into(), "archive".into()]))
    /// );
    /// ```
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            CORE_GROUP => Ok(Self::Core),
            ALL_GROUPS => Ok(Self::All),
            _ => {
                let mut names: Vec<String> = Vec::new();
                for name in text.split(GROUP_SEPARATOR).map(str::trim) {
                    check_group_name(name)?;
                    if name != CORE_GROUP && !names.iter().any(|known| known == name) {
                        names.push(name.to_owned());
                    }
                }
                Ok(if names.is_empty() {
                    Self::Core
                } else {
                    Self::Only(names)
                })
            }
        }
    }
}

fn check_group_name(name: &str) -> Result<(), ToolDefError> {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(ToolDefError::InvalidGroupName {
            name: name.to_owned(),
        })
    }
}

/// Inputs of [`render`](crate::render) that do not depend on the format.
///
/// The description texts are supplied by the caller: the text of the core
/// group and, for every optional group that may be selected, the text of
/// that group. The engine produces them with `describe`; the generator only
/// joins the selected texts, so the output depends on nothing but these
/// options and the format.
///
/// # Examples
///
/// ```
/// use nurt_tooldef::{GroupSelection, RenderOptions};
///
/// let options = RenderOptions::new("Runs programs of the Nurt language.")
///     .with_group_description("git", "Git repositories.")
///     .with_groups(GroupSelection::All)
///     .with_catalog_version("1.0");
/// assert_eq!(options.catalog_version(), Some("1.0"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    core_description: String,
    group_descriptions: BTreeMap<String, String>,
    groups: GroupSelection,
    language: Language,
    catalog_version: Option<String>,
}

impl RenderOptions {
    /// Creates options with the description of the core group, English,
    /// the core group only and no catalog version.
    #[must_use]
    pub fn new(core_description: impl Into<String>) -> Self {
        Self {
            core_description: core_description.into(),
            group_descriptions: BTreeMap::new(),
            groups: GroupSelection::Core,
            language: Language::En,
            catalog_version: None,
        }
    }

    /// Supplies the description of an optional group.
    #[must_use]
    pub fn with_group_description(
        mut self,
        group: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.group_descriptions
            .insert(group.into(), description.into());
        self
    }

    /// Selects the groups whose descriptions are included.
    #[must_use]
    pub fn with_groups(mut self, groups: GroupSelection) -> Self {
        self.groups = groups;
        self
    }

    /// Sets the language of the description texts. The language is also
    /// the default of the `language` argument of the tool.
    #[must_use]
    pub fn with_language(mut self, language: Language) -> Self {
        self.language = language;
        self
    }

    /// Sets the version of the command catalog that the description texts
    /// were produced from. The version is written in the formats that carry
    /// metadata (`mcp` and `json-schema`).
    #[must_use]
    pub fn with_catalog_version(mut self, version: impl Into<String>) -> Self {
        self.catalog_version = Some(version.into());
        self
    }

    /// Returns the selected groups.
    #[must_use]
    pub fn groups(&self) -> &GroupSelection {
        &self.groups
    }

    /// Returns the language of the description texts.
    #[must_use]
    pub fn language(&self) -> Language {
        self.language
    }

    /// Returns the catalog version, when given.
    #[must_use]
    pub fn catalog_version(&self) -> Option<&str> {
        self.catalog_version.as_deref()
    }

    /// Returns the tool description: the core text followed by the texts
    /// of the selected groups, separated by blank lines.
    ///
    /// # Errors
    ///
    /// - [`ToolDefError::EmptyDescription`] when the core text is blank;
    /// - [`ToolDefError::MissingGroupDescription`] when a listed group has
    ///   no description or its description is blank;
    /// - [`ToolDefError::InvalidGroupName`] when a listed name is not a
    ///   valid group name;
    /// - [`ToolDefError::EmptyCatalogVersion`] when the catalog version is
    ///   blank.
    pub fn description(&self) -> Result<String, ToolDefError> {
        let core = self.core_description.trim();
        if core.is_empty() {
            return Err(ToolDefError::EmptyDescription);
        }
        if self
            .catalog_version
            .as_deref()
            .is_some_and(|version| version.trim().is_empty())
        {
            return Err(ToolDefError::EmptyCatalogVersion);
        }
        let mut parts = vec![core];
        for group in self.selected_groups()? {
            let text = self
                .group_descriptions
                .get(group)
                .map(|text| text.trim())
                .filter(|text| !text.is_empty())
                .ok_or_else(|| ToolDefError::MissingGroupDescription {
                    group: group.to_owned(),
                })?;
            parts.push(text);
        }
        Ok(parts.join(PART_SEPARATOR))
    }

    fn selected_groups(&self) -> Result<Vec<&str>, ToolDefError> {
        match &self.groups {
            GroupSelection::Core => Ok(Vec::new()),
            GroupSelection::All => Ok(self
                .group_descriptions
                .keys()
                .map(String::as_str)
                .filter(|group| *group != CORE_GROUP)
                .collect()),
            GroupSelection::Only(groups) => {
                let mut selected: Vec<&str> = Vec::new();
                for group in groups {
                    check_group_name(group)?;
                    if group != CORE_GROUP && !selected.contains(&group.as_str()) {
                        selected.push(group);
                    }
                }
                Ok(selected)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_selection_uses_the_core_text_only() {
        let options = RenderOptions::new("  Core.  ").with_group_description("git", "Git.");
        assert_eq!(options.description(), Ok("Core.".to_owned()));
        assert_eq!(options.groups(), &GroupSelection::Core);
        assert_eq!(options.language(), Language::En);
        assert_eq!(options.catalog_version(), None);
    }

    #[test]
    fn all_selection_joins_groups_in_name_order() {
        let options = RenderOptions::new("Core.")
            .with_group_description("text", "Text.")
            .with_group_description("archive", "Archives.")
            .with_group_description(CORE_GROUP, "Ignored.")
            .with_groups(GroupSelection::All);
        assert_eq!(
            options.description(),
            Ok("Core.\n\nArchives.\n\nText.".to_owned())
        );
    }

    #[test]
    fn listed_selection_keeps_order_and_skips_duplicates_and_core() {
        let options = RenderOptions::new("Core.")
            .with_group_description("text", "Text.")
            .with_group_description("git", "Git.")
            .with_groups(GroupSelection::Only(vec![
                "text".into(),
                CORE_GROUP.into(),
                "git".into(),
                "text".into(),
            ]));
        assert_eq!(
            options.description(),
            Ok("Core.\n\nText.\n\nGit.".to_owned())
        );
    }

    #[test]
    fn rejects_blank_core_description() {
        assert_eq!(
            RenderOptions::new(" \n").description(),
            Err(ToolDefError::EmptyDescription)
        );
    }

    #[test]
    fn rejects_listed_group_without_description() {
        let options = RenderOptions::new("Core.")
            .with_group_description("git", "  ")
            .with_groups(GroupSelection::Only(vec!["git".into()]));
        assert_eq!(
            options.description(),
            Err(ToolDefError::MissingGroupDescription {
                group: "git".into()
            })
        );
    }

    #[test]
    fn rejects_invalid_listed_group_name() {
        let options =
            RenderOptions::new("Core.").with_groups(GroupSelection::Only(vec!["Git".into()]));
        assert_eq!(
            options.description(),
            Err(ToolDefError::InvalidGroupName { name: "Git".into() })
        );
    }

    #[test]
    fn rejects_blank_catalog_version() {
        let options = RenderOptions::new("Core.").with_catalog_version(" ");
        assert_eq!(
            options.description(),
            Err(ToolDefError::EmptyCatalogVersion)
        );
    }

    #[test]
    fn parses_group_selection_text() {
        assert_eq!("core".parse(), Ok(GroupSelection::Core));
        assert_eq!("core, core".parse(), Ok(GroupSelection::Core));
        assert_eq!(
            "git, text,git".parse(),
            Ok(GroupSelection::Only(vec!["git".into(), "text".into()]))
        );
        assert_eq!(
            "git,,text".parse::<GroupSelection>(),
            Err(ToolDefError::InvalidGroupName {
                name: String::new()
            })
        );
    }
}
