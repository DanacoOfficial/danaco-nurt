// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes and directories given to the report builder.

use nurt_contracts::report::Change;

use crate::rules::is_fs_path;

/// Kind of the entry that a change concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum EntryKind {
    /// A regular file.
    #[default]
    File,
    /// A directory.
    Directory,
    /// A symbolic link or a junction.
    Link,
    /// A target outside the file system, for example `service:nginx`.
    Other,
}

impl EntryKind {
    /// The kind assumed for a path when the caller does not know it: a file
    /// for a file system path, [`EntryKind::Other`] for an identifier of a
    /// provider.
    #[must_use]
    pub fn infer(path: &str) -> Self {
        if is_fs_path(path) {
            Self::File
        } else {
            Self::Other
        }
    }

    /// Whether the entry counts as a file in targets and summaries: a regular
    /// file or a link.
    #[must_use]
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File | Self::Link)
    }
}

/// A change of one target together with the facts that the change record
/// does not carry.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct TargetChange {
    /// The change as listed in the report.
    pub change: Change,
    /// Kind of the entry concerned.
    pub entry: EntryKind,
    /// The change replaced an existing file: `write mode: overwrite`, or a
    /// move or copy with `on_conflict: overwrite`.
    pub replaced_existing: bool,
    /// Number of entries that a deleted directory held, when the entries are
    /// not listed as changes of their own.
    pub entries_inside: Option<u64>,
}

impl TargetChange {
    /// A change of an entry of the given kind.
    #[must_use]
    pub fn new(change: Change, entry: EntryKind) -> Self {
        Self {
            change,
            entry,
            replaced_existing: false,
            entries_inside: None,
        }
    }

    /// A change of a regular file.
    #[must_use]
    pub fn file(change: Change) -> Self {
        Self::new(change, EntryKind::File)
    }

    /// A change of a directory.
    #[must_use]
    pub fn directory(change: Change) -> Self {
        Self::new(change, EntryKind::Directory)
    }

    /// Marks the change as one that replaced an existing file.
    #[must_use]
    pub fn replacing_existing(mut self) -> Self {
        self.replaced_existing = true;
        self
    }

    /// Sets the number of entries held by a deleted directory.
    #[must_use]
    pub fn with_entries_inside(mut self, entries: u64) -> Self {
        self.entries_inside = Some(entries);
        self
    }
}

impl From<Change> for TargetChange {
    /// Wraps a change whose entry kind is inferred from its path.
    fn from(change: Change) -> Self {
        let entry = EntryKind::infer(change.path.as_str());
        Self::new(change, entry)
    }
}

/// A directory whose share of deleted files is measured for `mass_delete`,
/// for example the root searched by `find` or a directory removed
/// recursively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDirectory {
    /// Directory in canonical form.
    pub path: String,
    /// Number of files below the directory before the operation.
    pub files_before: u64,
}

impl ScopeDirectory {
    /// A directory that held `files_before` files before the operation.
    #[must_use]
    pub fn new(path: impl Into<String>, files_before: u64) -> Self {
        Self {
            path: path.into(),
            files_before,
        }
    }
}
