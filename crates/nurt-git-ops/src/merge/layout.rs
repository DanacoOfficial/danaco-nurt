// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conflict layouts of `merge-ort` that `gix-merge` records differently.
//!
//! Two layouts are adjusted. When one side replaces a file by a symbolic link
//! and the other side edits the file, Git keeps the link at the path and
//! moves the regular file to `<path>~<side>`, so that each entry has its own
//! path. When one side renames a file that the other side deletes, Git
//! records the original blob as stage 1 of the new path.

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_index::entry::{Flags, Stage};
use gix_merge::tree::{Conflict, ResolutionFailure, TreatAsUnresolved};
use gix_object::tree::EntryMode;
use nurt_contracts::error::{Error, ErrorCode, Result};

/// Stage entry of a path: mode and object.
type Staged = (EntryMode, ObjectId);

/// Adjustment to apply to the result tree and to the index stages.
#[derive(Debug)]
pub(super) enum Fix {
    /// A link and a regular file of the same path are recorded separately.
    TypeSplit {
        path: BString,
        moved_path: BString,
        stale_path: Option<BString>,
        base: Option<Staged>,
        link: Staged,
        link_stage: Stage,
        regular: Staged,
        regular_stage: Stage,
    },
    /// The original blob of a renamed and deleted file is recorded as stage 1.
    RenameBase { path: BString, base: Staged },
}

/// Finds the conflicts to adjust. `ours` and `theirs` are the labels of the
/// two sides, used to name the moved regular file.
pub(super) fn plan(
    conflicts: &[Conflict],
    editor: &gix_object::tree::Editor<'_>,
    ours: &str,
    theirs: &str,
) -> Vec<Fix> {
    let mut fixes = Vec::new();
    for conflict in conflicts
        .iter()
        .filter(|c| c.is_unresolved(TreatAsUnresolved::git()))
    {
        if let Some(fix) = type_split(conflict, editor, ours, theirs) {
            fixes.push(fix);
        } else if let Some(fix) = rename_base(conflict) {
            fixes.push(fix);
        }
    }
    fixes
}

fn type_split(
    conflict: &Conflict,
    editor: &gix_object::tree::Editor<'_>,
    ours: &str,
    theirs: &str,
) -> Option<Fix> {
    let [base, Some(our), Some(their)] = conflict.entries() else {
        return None;
    };
    let types_differ = our.mode.is_link() != their.mode.is_link();
    if !types_differ || !our.mode.is_blob_or_symlink() || !their.mode.is_blob_or_symlink() {
        return None;
    }
    let path = conflict.changes_in_resolution().0.location().to_owned();
    let regular_is_ours = !our.mode.is_link();
    let (link, regular, label) = if regular_is_ours {
        (their, our, ours)
    } else {
        (our, their, theirs)
    };
    let (link_stage, regular_stage) = if regular_is_ours {
        (Stage::Theirs, Stage::Ours)
    } else {
        (Stage::Ours, Stage::Theirs)
    };
    let placed = match &conflict.resolution {
        Err(ResolutionFailure::OursAddedTheirsAddedTypeMismatch {
            their_unique_location,
        }) => Some(their_unique_location.clone()),
        _ => None,
    };
    let (moved_path, stale_path) = match placed {
        Some(placed) if !regular_is_ours => (placed, None),
        Some(placed) => (
            unique_path(editor, path.as_bstr(), label, Some(placed.as_bstr())),
            Some(placed),
        ),
        None => (unique_path(editor, path.as_bstr(), label, None), None),
    };
    Some(Fix::TypeSplit {
        moved_path,
        stale_path,
        path,
        base: base.map(|entry| (entry.mode, entry.id)),
        link: (link.mode, link.id),
        link_stage,
        regular: (regular.mode, regular.id),
        regular_stage,
    })
}

fn rename_base(conflict: &Conflict) -> Option<Fix> {
    if !matches!(
        conflict.resolution,
        Err(ResolutionFailure::OursDeletedTheirsRenamed)
    ) {
        return None;
    }
    let (ours, theirs) = conflict.changes_in_resolution();
    [ours, theirs].into_iter().find_map(|change| match change {
        gix_diff::tree_with_rewrites::Change::Rewrite {
            location,
            source_id,
            source_entry_mode,
            ..
        } => Some(Fix::RenameBase {
            path: location.clone(),
            base: (*source_entry_mode, *source_id),
        }),
        _ => None,
    })
}

fn unique_path(
    editor: &gix_object::tree::Editor<'_>,
    path: &BStr,
    label: &str,
    vacated: Option<&BStr>,
) -> BString {
    let label = label.replace('/', "_");
    let mut suffix: Option<u32> = None;
    loop {
        let mut candidate = BString::from(path.as_bytes());
        candidate.extend_from_slice(format!("~{label}").as_bytes());
        if let Some(n) = suffix {
            candidate.extend_from_slice(format!("_{n}").as_bytes());
        }
        if vacated == Some(candidate.as_bstr())
            || editor.get(components(candidate.as_bstr())).is_none()
        {
            return candidate;
        }
        suffix = Some(suffix.map_or(0, |n| n + 1));
    }
}

fn components(path: &BStr) -> impl Iterator<Item = &BStr> {
    path.split(|byte| *byte == b'/').map(ByteSlice::as_bstr)
}

/// Applies the tree part of `fixes`.
pub(super) fn apply_to_tree(
    editor: &mut gix_object::tree::Editor<'_>,
    fixes: &[Fix],
) -> Result<()> {
    for fix in fixes {
        let Fix::TypeSplit {
            path,
            moved_path,
            stale_path,
            link,
            regular,
            ..
        } = fix
        else {
            continue;
        };
        if let Some(stale) = stale_path {
            editor
                .remove(components(stale.as_bstr()))
                .map_err(|error| {
                    Error::new(
                        ErrorCode::Io,
                        format!("cannot record the conflict of {stale}: {error}"),
                    )
                })?;
        }
        for (target, (mode, id)) in [(path, link), (moved_path, regular)] {
            editor
                .upsert(components(target.as_bstr()), mode.kind(), *id)
                .map_err(|error| {
                    Error::new(
                        ErrorCode::Io,
                        format!("cannot record the conflict of {target}: {error}"),
                    )
                })?;
        }
    }
    Ok(())
}

/// Applies the stage part of `fixes` to `index`.
pub(super) fn apply_to_index(index: &mut gix_index::State, fixes: &[Fix]) {
    for fix in fixes {
        match fix {
            Fix::TypeSplit {
                path,
                moved_path,
                stale_path,
                base,
                link,
                link_stage,
                regular,
                regular_stage,
            } => {
                index.remove_entries(|_, entry_path, _| {
                    entry_path == path.as_bstr()
                        || entry_path == moved_path.as_bstr()
                        || stale_path
                            .as_ref()
                            .is_some_and(|s| entry_path == s.as_bstr())
                });
                push(index, path, *link_stage, *link);
                if let Some(base) = base {
                    push(index, moved_path, Stage::Base, *base);
                }
                push(index, moved_path, *regular_stage, *regular);
            }
            Fix::RenameBase { path, base } => {
                let present = index
                    .entry_index_by_path_and_stage(path.as_bstr(), Stage::Base)
                    .is_some();
                if !present {
                    push(index, path, Stage::Base, *base);
                }
            }
        }
    }
    index.sort_entries();
}

fn push(index: &mut gix_index::State, path: &BString, stage: Stage, (mode, id): Staged) {
    index.dangerously_push_entry(
        Default::default(),
        id,
        Flags::from(stage),
        mode.into(),
        path.as_bstr(),
    );
}
