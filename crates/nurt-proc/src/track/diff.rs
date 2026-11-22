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

/// Whether `after` is the entry `before` under a new path. A rename keeps
/// the birth time of an entry, while an entry created after a deletion may
/// reuse the freed identity with a new birth time; without birth times on
/// both sides the identity alone decides. A file must also keep its size
/// and modification time: a file renamed and changed in the same run is
/// reported as deleted and created, so its new content is recorded.
fn renamed(before: &EntryState, after: &EntryState) -> bool {
    let same_birth = match (before.created_ns, after.created_ns) {
        (Some(old), Some(new)) => old == new,
        _ => true,
    };
    same_birth
        && (before.kind != Kind::File
            || (before.size == after.size && before.modified_ns == after.modified_ns))
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
        let from = identity(after)
            .and_then(|id| deleted.get(&id).map(|from| (id, *from)))
            .filter(|(_, from)| old.get(*from).is_some_and(|before| renamed(before, after)))
            .map(|(id, from)| {
                deleted.remove(&id);
                from
            });
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
                if change
                    .after
                    .as_ref()
                    .is_some_and(|state| state.kind == Kind::Directory) =>
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

#[cfg(test)]
mod tests {
    use super::*;

    /// File of `size` bytes with identity `inode`, modified at `modified_ns`
    /// and born at `created_ns`.
    fn file(inode: u64, size: u64, modified_ns: i64, created_ns: Option<i64>) -> EntryState {
        EntryState {
            kind: Kind::File,
            size,
            modified_ns,
            file_id: [1, inode, 0],
            created_ns,
            mode: 0o644,
            link: None,
            hash: None,
        }
    }

    fn actions(before: EntryState, after: EntryState) -> Vec<(String, Action)> {
        let old = Snapshot::from([(PathBuf::from("/w/old.txt"), before)]);
        let new = Snapshot::from([(PathBuf::from("/w/new.txt"), after)]);
        compare(&old, &new)
            .into_iter()
            .map(|change| (change.path.to_string_lossy().into_owned(), change.action))
            .collect()
    }

    fn rename() -> Vec<(String, Action)> {
        vec![(
            "/w/new.txt".to_owned(),
            Action::Rename {
                from: PathBuf::from("/w/old.txt"),
                implied: false,
            },
        )]
    }

    fn delete_and_create() -> Vec<(String, Action)> {
        vec![
            ("/w/new.txt".to_owned(), Action::Create),
            ("/w/old.txt".to_owned(), Action::Delete),
        ]
    }

    #[test]
    fn same_identity_and_birth_time_is_a_rename() {
        assert_eq!(
            actions(file(7, 5, 100, Some(50)), file(7, 5, 100, Some(50))),
            rename()
        );
    }

    #[test]
    fn reused_identity_with_a_new_birth_time_is_a_new_file() {
        assert_eq!(
            actions(file(7, 5, 100, Some(50)), file(7, 5, 100, Some(90))),
            delete_and_create()
        );
    }

    #[test]
    fn renamed_and_changed_file_is_deleted_and_created() {
        assert_eq!(
            actions(file(7, 5, 100, Some(50)), file(7, 9, 120, Some(50))),
            delete_and_create()
        );
    }

    #[test]
    fn without_birth_times_an_unchanged_file_with_the_same_identity_is_a_rename() {
        assert_eq!(
            actions(file(7, 5, 100, None), file(7, 5, 100, None)),
            rename()
        );
        assert_eq!(
            actions(file(7, 5, 100, None), file(7, 6, 100, None)),
            delete_and_create()
        );
    }
}
