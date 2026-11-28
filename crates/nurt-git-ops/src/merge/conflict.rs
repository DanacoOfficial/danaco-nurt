// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::merge::conflict
// Responsibility:   Conflicts left by a merge, derived from the stages of the result index.
// Dependencies:     bstr, gix_hash, gix_index, gix_merge, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conflicts left by a merge, derived from the stages of the result index.

use std::collections::HashMap;

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_index::entry::Stage;
use gix_merge::tree::{Conflict, ResolutionFailure, TreatAsUnresolved};
use nurt_contracts::record::{GitConflict, GitConflictKind};
use nurt_contracts::value::Bytes;

/// One conflicting path of a merge result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict {
    /// Path relative to the repository root.
    pub path: BString,
    /// Kind of the conflict.
    pub kind: GitConflictKind,
    /// The file in the result tree contains conflict markers.
    pub markers: bool,
    /// Object of stage 1 (base).
    pub base: Option<ObjectId>,
    /// Object of stage 2 (current side).
    pub ours: Option<ObjectId>,
    /// Object of stage 3 (merged side).
    pub theirs: Option<ObjectId>,
}

impl MergeConflict {
    /// The `git_conflict` record of the conflict.
    #[must_use]
    pub fn to_record(&self) -> GitConflict {
        GitConflict {
            path: self.path.to_str_lossy().into_owned(),
            raw_path: self
                .path
                .to_str()
                .is_err()
                .then(|| Bytes::new(self.path.to_vec())),
            kind: self.kind,
            markers: self.markers,
            base: self.base.map(|id| id.to_hex().to_string()),
            ours: self.ours.map(|id| id.to_hex().to_string()),
            theirs: self.theirs.map(|id| id.to_hex().to_string()),
        }
    }
}

/// Kind of a conflict from the stages present for its path, as reported by
/// `git status`.
#[must_use]
pub fn kind_from_stages(base: bool, ours: bool, theirs: bool) -> GitConflictKind {
    match (base, ours, theirs) {
        (true, true, true) => GitConflictKind::BothModified,
        (false, true, true) => GitConflictKind::BothAdded,
        (true, true, false) => GitConflictKind::DeletedByThem,
        (true, false, true) => GitConflictKind::DeletedByUs,
        (false, true, false) => GitConflictKind::AddedByUs,
        (false, false, true) => GitConflictKind::AddedByThem,
        _ => GitConflictKind::BothDeleted,
    }
}

#[derive(Default)]
struct Detail {
    kind: Option<GitConflictKind>,
    merged: Option<ObjectId>,
    markers: bool,
}

pub(super) fn collect(index: &gix_index::State, conflicts: &[Conflict]) -> Vec<MergeConflict> {
    let details = details(conflicts);
    let mut out: Vec<MergeConflict> = Vec::new();
    for entry in index.entries() {
        let stage = entry.stage();
        if stage == Stage::Unconflicted {
            continue;
        }
        let path = entry.path(index);
        if out.last().is_none_or(|last| last.path != path) {
            out.push(MergeConflict {
                path: path.to_owned(),
                kind: GitConflictKind::BothDeleted,
                markers: false,
                base: None,
                ours: None,
                theirs: None,
            });
        }
        if let Some(last) = out.last_mut() {
            match stage {
                Stage::Base => last.base = Some(entry.id),
                Stage::Ours => last.ours = Some(entry.id),
                _ => last.theirs = Some(entry.id),
            }
        }
    }
    for item in &mut out {
        refine(item, details.get(item.path.as_bstr()));
    }
    out
}

fn refine(item: &mut MergeConflict, detail: Option<&Detail>) {
    item.kind = kind_from_stages(
        item.base.is_some(),
        item.ours.is_some(),
        item.theirs.is_some(),
    );
    let Some(detail) = detail else {
        return;
    };
    if let Some(kind) = detail.kind {
        item.kind = kind;
    }
    if detail.markers {
        let binary = detail.merged.is_some() && detail.merged == item.ours;
        item.markers = !binary;
        if binary {
            item.kind = GitConflictKind::Binary;
        }
    }
}

fn details(conflicts: &[Conflict]) -> HashMap<&BStr, Detail> {
    let mut out: HashMap<&BStr, Detail> = HashMap::new();
    for conflict in conflicts
        .iter()
        .filter(|c| c.is_unresolved(TreatAsUnresolved::git()))
    {
        let (ours, theirs) = conflict.changes_in_resolution();
        let kind = failure_kind(conflict);
        let content = conflict
            .content_merge()
            .filter(|merge| merge.resolution == gix_merge::blob::Resolution::Conflict);
        let mut locations = vec![ours.location(), theirs.location()];
        locations.extend(extra_location(conflict));
        for location in locations {
            let detail = out.entry(location).or_default();
            if kind.is_some() {
                detail.kind = kind;
            }
            if let Some(merge) = content {
                detail.markers = true;
                detail.merged = Some(merge.merged_blob_id);
            }
        }
    }
    out
}

fn failure_kind(conflict: &Conflict) -> Option<GitConflictKind> {
    let failure = match &conflict.resolution {
        Err(failure) => failure,
        Ok(_) => return None,
    };
    match failure {
        ResolutionFailure::OursRenamedTheirsRenamedDifferently { .. }
        | ResolutionFailure::OursRenamedTheirsRenamedToSameLocation => {
            Some(GitConflictKind::RenameRename)
        }
        ResolutionFailure::OursDeletedTheirsRenamed => Some(GitConflictKind::RenameDelete),
        ResolutionFailure::OursModifiedTheirsDirectoryThenOursRenamed { .. }
        | ResolutionFailure::OursDirectoryTheirsNonDirectoryTheirsRenamed { .. } => {
            Some(GitConflictKind::DirectoryFile)
        }
        ResolutionFailure::SubmoduleMerge | ResolutionFailure::SubmoduleAddAdd => {
            Some(GitConflictKind::Submodule)
        }
        ResolutionFailure::OursAddedTheirsAddedTypeMismatch { .. }
        | ResolutionFailure::OursModifiedTheirsRenamedTypeMismatch
        | ResolutionFailure::OursModifiedTheirsDeleted
        | ResolutionFailure::ModeConflict { .. }
        | ResolutionFailure::DistinctTypes { .. }
        | ResolutionFailure::DirectoryRename { .. }
        | ResolutionFailure::DirectoryRenameSplit { .. }
        | ResolutionFailure::DirectoryRenameBlocked { .. }
        | ResolutionFailure::DirectoryRenameCollision { .. }
        | ResolutionFailure::Unknown => None,
    }
}

fn extra_location(conflict: &Conflict) -> Option<&BStr> {
    match &conflict.resolution {
        Err(
            ResolutionFailure::OursModifiedTheirsDirectoryThenOursRenamed {
                renamed_unique_path_to_modified_blob: path,
            }
            | ResolutionFailure::OursDirectoryTheirsNonDirectoryTheirsRenamed {
                renamed_unique_path_of_theirs: path,
            }
            | ResolutionFailure::OursAddedTheirsAddedTypeMismatch {
                their_unique_location: path,
            },
        ) => Some(path.as_bstr()),
        Ok(gix_merge::tree::Resolution::SourceLocationAffectedByRename { final_location }) => {
            Some(final_location.as_bstr())
        }
        Ok(gix_merge::tree::Resolution::OursModifiedTheirsRenamedAndChangedThenRename {
            final_location: Some(path),
            ..
        }) => Some(path.as_bstr()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_stage_sets_to_status_kinds() {
        assert_eq!(
            kind_from_stages(true, true, true),
            GitConflictKind::BothModified
        );
        assert_eq!(
            kind_from_stages(false, true, true),
            GitConflictKind::BothAdded
        );
        assert_eq!(
            kind_from_stages(true, true, false),
            GitConflictKind::DeletedByThem
        );
        assert_eq!(
            kind_from_stages(true, false, true),
            GitConflictKind::DeletedByUs
        );
        assert_eq!(
            kind_from_stages(false, true, false),
            GitConflictKind::AddedByUs
        );
        assert_eq!(
            kind_from_stages(false, false, true),
            GitConflictKind::AddedByThem
        );
        assert_eq!(
            kind_from_stages(true, false, false),
            GitConflictKind::BothDeleted
        );
    }

    #[test]
    fn converts_conflict_to_record_with_hex_identifiers() {
        let id = ObjectId::empty_blob(gix_hash::Kind::Sha1);
        let conflict = MergeConflict {
            path: "a/b.txt".into(),
            kind: GitConflictKind::DeletedByThem,
            markers: false,
            base: Some(id),
            ours: Some(id),
            theirs: None,
        };
        let record = conflict.to_record();
        assert_eq!(record.path, "a/b.txt");
        assert_eq!(record.kind, GitConflictKind::DeletedByThem);
        assert_eq!(
            record.base.as_deref(),
            Some("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391")
        );
        assert!(record.theirs.is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
