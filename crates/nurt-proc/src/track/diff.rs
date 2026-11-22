// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Comparison of the state before and after a run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::scan::{EntryState, Kind, Snapshot};

/// What happened to a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    /// The entry appeared.
    Create,
    /// The content of a file changed.
    Modify,
    /// The entry disappeared.
    Delete,
    /// The entry was renamed from `from` (same file identity).
    Rename {
        /// Previous path.
        from: PathBuf,
        /// The rename follows from the rename of a parent directory and is
        /// not reported separately.
        implied: bool,
    },
    /// Only the permission bits changed.
    Attributes,
    /// The target of a link changed.
    Link,
    /// The entry was replaced by an entry of another kind.
    Replace,
}

/// One detected change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Detected {
    /// Changed path; the new path of a rename.
    pub(super) path: PathBuf,
    /// Change.
    pub(super) action: Action,
    /// State before; for a rename, of the previous path.
    pub(super) before: Option<EntryState>,
    /// State after.
    pub(super) after: Option<EntryState>,
}

/// Change of an entry present before and after.
fn change(before: &EntryState, after: &EntryState) -> Option<Action> {
    if before.kind != after.kind {
        return Some(Action::Replace);
    }
    let content_changed = before.size != after.size
        || before.modified_ns != after.modified_ns
        || before.file_id != after.file_id
        || matches!((&before.hash, &after.hash), (Some(old), Some(new)) if old != new);
    match before.kind {
        Kind::File | Kind::Special if content_changed => Some(Action::Modify),
        Kind::Link if before.link != after.link => Some(Action::Link),
        _ if before.mode != after.mode => Some(Action::Attributes),
        _ => None,
    }
}

/// Identity usable for rename detection; zero identities are unknown.
fn identity(state: &EntryState) -> Option<([u64; 3], Kind)> {
    (state.file_id[1] != 0 || state.file_id[2] != 0).then_some((state.file_id, state.kind))
}

/// Changes between `old` and `new`, ordered by path.
pub(super) fn compare(old: &Snapshot, new: &Snapshot) -> Vec<Detected> {
    let mut detected = Vec::new();
    let mut deleted: HashMap<([u64; 3], Kind), &Path> = HashMap::new();
    let mut deleted_order = Vec::new();
    for (path, before) in old {
        match new.get(path) {
            Some(after) => {
                if let Some(action) = change(before, after) {
                    detected.push(Detected {
                        path: path.clone(),
                        action,
                        before: Some(before.clone()),
                        after: Some(after.clone()),
                    });
                }
            }
            None => {
                if let Some(id) = identity(before) {
                    deleted.insert(id, path);
                }
                deleted_order.push(path.as_path());
            }
        }
    }
    let mut renamed_from = Vec::new();
    for (path, after) in new.iter().filter(|(path, _)| !old.contains_key(*path)) {
        let from = identity(after).and_then(|id| deleted.remove(&id));
        let action = match from {
            Some(from) => {
                renamed_from.push(from);
                Action::Rename {
                    from: from.to_path_buf(),
                    implied: false,
                }
            }
            None => Action::Create,
        };
        let before = from.and_then(|from| old.get(from)).cloned();
        detected.push(Detected {
            path: path.clone(),
            action,
            before,
            after: Some(after.clone()),
        });
    }
    for path in deleted_order {
        if !renamed_from.contains(&path) {
            detected.push(Detected {
                path: path.to_path_buf(),
                action: Action::Delete,
                before: old.get(path).cloned(),
                after: None,
            });
        }
    }
    mark_implied(&mut detected);
    detected.sort_by(|left, right| left.path.cmp(&right.path));
    detected
}

/// Marks renames that follow from the rename of a parent directory.
fn mark_implied(detected: &mut [Detected]) {
    let directories: Vec<(PathBuf, PathBuf)> = detected
        .iter()
        .filter_map(|change| match &change.action {
            Action::Rename { from, .. }
                if change.after.as_ref().is_some_and(|state| state.kind == Kind::Directory) =>
            {
                Some((from.clone(), change.path.clone()))
            }
            _ => None,
        })
        .collect();
    for change in detected.iter_mut() {
        if let Action::Rename { from, implied } = &mut change.action {
            *implied = directories.iter().any(|(old_dir, new_dir)| {
                old_dir.as_path() != from.as_path()
                    && from
                        .strip_prefix(old_dir)
                        .is_ok_and(|relative| change.path.strip_prefix(new_dir) == Ok(relative))
            });
        }
    }
}
