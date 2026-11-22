// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Sections and entries of a repository profile.

use serde::{Deserialize, Serialize};

/// Section of a repository profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Section {
    /// Languages of the files, with their share.
    Languages,
    /// Directories holding a project manifest.
    Modules,
    /// Programs and libraries the project starts from.
    EntryPoints,
    /// Build, test and check commands.
    Commands,
    /// Files with instructions for contributors and tools.
    Instructions,
    /// Files changed most often, and uncommitted changes.
    Hotspots,
    /// Project manifests.
    Manifests,
}

impl Section {
    /// Every section, in output order.
    pub(crate) const ALL: [Self; 7] = [
        Self::Languages,
        Self::Modules,
        Self::EntryPoints,
        Self::Commands,
        Self::Instructions,
        Self::Hotspots,
        Self::Manifests,
    ];

    /// Name used by the `sections` parameter.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Languages => "languages",
            Self::Modules => "modules",
            Self::EntryPoints => "entry_points",
            Self::Commands => "commands",
            Self::Instructions => "instructions",
            Self::Hotspots => "hotspots",
            Self::Manifests => "manifests",
        }
    }

    /// Section named `name`.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.as_str() == name)
    }
}

/// One entry of a profile.
///
/// Paths are relative to the profiled root and use `/` as separator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// Section of the entry.
    pub(crate) section: Section,
    /// Name: command name, entry point, file or language.
    pub(crate) name: String,
    /// File or directory the entry refers to.
    pub(crate) path: Option<String>,
    /// Command line to run.
    pub(crate) command: Option<String>,
    /// File and key or line the entry was read from.
    pub(crate) origin: Option<String>,
    /// Number of commits that changed the file.
    pub(crate) changes: Option<u64>,
    /// Time of the last change, nanoseconds since the Unix epoch.
    pub(crate) last_change_ns: Option<i64>,
}

impl Entry {
    /// Entry with a name and no other fields.
    pub(crate) fn new(section: Section, name: impl Into<String>) -> Self {
        Self {
            section,
            name: name.into(),
            path: None,
            command: None,
            origin: None,
            changes: None,
            last_change_ns: None,
        }
    }

    /// Sets the path.
    #[must_use]
    pub(crate) fn path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Sets the command.
    #[must_use]
    pub(crate) fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    /// Sets the origin.
    #[must_use]
    pub(crate) fn origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}

/// Joins a directory and a name relative to the root; the root directory is
/// the empty string.
pub(crate) fn join(dir: &str, name: &str) -> String {
    let name = name.trim_start_matches("./");
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// Directory part of a relative path; the empty string for the root.
pub(crate) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// File name part of a relative path.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}
