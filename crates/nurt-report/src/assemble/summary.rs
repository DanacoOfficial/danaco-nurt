// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Totals of the changes of an operation.
//!
//! | Field | Changes counted |
//! |---|---|
//! | `files_created` | `create` and `link` of a file or link |
//! | `directories_created` | `create` of a directory |
//! | `files_modified` | `modify` and `truncate` of a file or link |
//! | `files_deleted` | `delete` of a file or link |
//! | `directories_deleted` | `delete` of a directory |
//! | `files_renamed` | every `rename` |
//! | `replacements`, `lines_added`, `lines_removed` | sum of `replacements`, `added`, `removed` |
//! | `bytes_before`, `bytes_after` | sum of `size_before`, `size_after` of changes other than `delete` |
//! | `bytes_freed` | sum of `size_before` of `delete` changes |
//!
//! A directory removed or restored with its content also counts the files
//! and directories below it, as far as the change reports them.
//!
//! A counter that is zero and a byte total without any known size are
//! absent from the summary.

use nurt_contracts::report::{ChangeAction, Summary, Targets};
use nurt_contracts::value::Size;

use super::input::{EntryKind, TargetChange};

/// Totals of the changes, or `None` when no field of the summary would be
/// present.
#[must_use]
pub fn summarize_changes(changes: &[TargetChange]) -> Option<Summary> {
    let mut totals = Totals::default();
    for target in changes {
        totals.add(target);
    }
    let summary = totals.into_summary();
    (summary != Summary::default()).then_some(summary)
}

/// Targets counted from the changes: their number, files, directories and
/// bytes. The bytes of a target are its size after the change, or its size
/// before the change when it was deleted.
#[must_use]
pub fn count_targets(changes: &[TargetChange]) -> Targets {
    let mut files = 0_u64;
    let mut directories = 0_u64;
    let mut bytes: Option<u64> = None;
    for target in changes {
        match target.entry {
            EntryKind::Directory => {
                directories += 1 + target.directories_inside.unwrap_or(0);
                files += target.files_inside.unwrap_or(0);
            }
            entry if entry.is_file() => files += 1,
            _ => {}
        }
        let size = target.change.size_after.or(target.change.size_before);
        add_size(&mut bytes, size);
    }
    Targets {
        count: changes.len() as u64,
        files: non_zero(files),
        directories: non_zero(directories),
        bytes,
        listed: None,
    }
}

#[derive(Debug, Default)]
struct Totals {
    files_created: u64,
    files_modified: u64,
    files_deleted: u64,
    files_renamed: u64,
    directories_created: u64,
    directories_deleted: u64,
    replacements: u64,
    lines_added: u64,
    lines_removed: u64,
    bytes_before: Option<u64>,
    bytes_after: Option<u64>,
    bytes_freed: Option<u64>,
}

impl Totals {
    fn add(&mut self, target: &TargetChange) {
        let change = &target.change;
        self.count_action(change.action, target.entry);
        self.count_content(target);
        self.replacements = self
            .replacements
            .saturating_add(change.replacements.unwrap_or(0));
        self.lines_added = self.lines_added.saturating_add(change.added.unwrap_or(0));
        self.lines_removed = self
            .lines_removed
            .saturating_add(change.removed.unwrap_or(0));
        if change.action == ChangeAction::Delete {
            add_size(&mut self.bytes_freed, change.size_before);
        } else {
            add_size(&mut self.bytes_before, change.size_before);
            add_size(&mut self.bytes_after, change.size_after);
        }
    }

    /// Counts the files and directories below a directory that was created
    /// or deleted with its content.
    fn count_content(&mut self, target: &TargetChange) {
        if target.entry != EntryKind::Directory {
            return;
        }
        let files = target.files_inside.unwrap_or(0);
        let directories = target.directories_inside.unwrap_or(0);
        match target.change.action {
            ChangeAction::Create => {
                self.files_created += files;
                self.directories_created += directories;
            }
            ChangeAction::Delete => {
                self.files_deleted += files;
                self.directories_deleted += directories;
            }
            _ => {}
        }
    }

    fn count_action(&mut self, action: ChangeAction, entry: EntryKind) {
        let directory = entry == EntryKind::Directory;
        let file = entry.is_file();
        let counter = match action {
            ChangeAction::Create if directory => &mut self.directories_created,
            ChangeAction::Create | ChangeAction::Link if file => &mut self.files_created,
            ChangeAction::Modify | ChangeAction::Truncate if file => &mut self.files_modified,
            ChangeAction::Delete if directory => &mut self.directories_deleted,
            ChangeAction::Delete if file => &mut self.files_deleted,
            ChangeAction::Rename => &mut self.files_renamed,
            _ => return,
        };
        *counter += 1;
    }

    fn into_summary(self) -> Summary {
        Summary {
            files_created: non_zero(self.files_created),
            files_modified: non_zero(self.files_modified),
            files_deleted: non_zero(self.files_deleted),
            files_renamed: non_zero(self.files_renamed),
            directories_created: non_zero(self.directories_created),
            directories_deleted: non_zero(self.directories_deleted),
            replacements: non_zero(self.replacements),
            lines_added: non_zero(self.lines_added),
            lines_removed: non_zero(self.lines_removed),
            bytes_before: self.bytes_before,
            bytes_after: self.bytes_after,
            bytes_freed: self.bytes_freed,
        }
    }
}

fn add_size(total: &mut Option<u64>, size: Option<Size>) {
    if let Some(size) = size {
        *total = Some(total.unwrap_or(0).saturating_add(size.bytes()));
    }
}

fn non_zero(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}
