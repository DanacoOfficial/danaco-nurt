// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Index states built from trees and the file status data of a new index.

use std::collections::HashSet;
use std::path::Path;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::State;
use gix::index::entry::{Flags, Mode, Stage, Stat};
use gix::objs::tree::EntryKind;
use nurt_contracts::error::Result;
use nurt_git::repo::GitRepo;

use super::repo::{io_error, join_slash};

/// Index with the entries of a tree and empty stat data.
pub(crate) fn from_tree(repo: &GitRepo, tree: ObjectId) -> Result<State> {
    repo.gix()
        .index_from_tree(&tree)
        .map(|file| file.into_parts().0)
        .map_err(|cause| io_error("cannot read tree", cause))
}

/// Whether the index has entries in a conflict stage.
pub(crate) fn has_conflicts(state: &State) -> bool {
    !nurt_git::repo::index::unmerged_paths(state).is_empty()
}

/// Tree kind of an index entry mode.
pub(crate) fn entry_kind(mode: Mode) -> EntryKind {
    if mode == Mode::FILE_EXECUTABLE {
        EntryKind::BlobExecutable
    } else if mode == Mode::SYMLINK {
        EntryKind::Link
    } else if mode == Mode::COMMIT {
        EntryKind::Commit
    } else {
        EntryKind::Blob
    }
}

/// Merged entry of `path`.
pub(crate) fn merged_entry<'a>(state: &'a State, path: &BStr) -> Option<&'a gix::index::Entry> {
    state.entry_by_path_and_stage(path, Stage::Unconflicted)
}

/// Paths whose file status data is set when the new index is written.
#[derive(Debug, Default)]
pub(crate) struct StatSources<'a> {
    /// Files the operation writes; their stat data is taken after the
    /// write when the index entry holds the written object, otherwise it
    /// stays empty so that the next status compares the content.
    pub(crate) written: Option<&'a HashSet<BString>>,
    /// Unchanged files that already hold the content of their new entry;
    /// their stat data is read now.
    pub(crate) read_now: Option<&'a HashSet<BString>>,
}

/// `state` with file status data: copied from `old` for entries with the
/// same path, object and mode, read from the working tree for the paths of
/// `sources.read_now`, empty otherwise.
///
/// The cached tree extension is dropped because the entries changed.
pub(crate) fn with_stats(
    root: &Path,
    mut state: State,
    old: &State,
    sources: &StatSources<'_>,
) -> State {
    let contains =
        |set: Option<&HashSet<BString>>, path: &BString| set.is_some_and(|set| set.contains(path));
    let paths: Vec<BString> = state
        .entries()
        .iter()
        .map(|entry| entry.path(&state).to_owned())
        .collect();
    for (entry, path) in state.entries_mut().iter_mut().zip(&paths) {
        if entry.stage() != Stage::Unconflicted {
            continue;
        }
        entry.stat = Stat::default();
        if contains(sources.written, path) {
            continue;
        }
        let previous = merged_entry(old, path.as_bstr())
            .filter(|previous| previous.id == entry.id && previous.mode == entry.mode);
        match previous {
            Some(previous) if previous.stat != Stat::default() => {
                entry.stat = previous.stat;
                entry.flags |= previous.flags & Flags::SKIP_WORKTREE;
            }
            _ if contains(sources.read_now, path) => {
                let file = join_slash(root, &path.to_str_lossy());
                if let Some(stat) = gix::index::fs::Metadata::from_path_no_follow(&file)
                    .ok()
                    .and_then(|meta| Stat::from_fs(&meta).ok())
                {
                    entry.stat = stat;
                }
            }
            _ => {}
        }
    }
    state.remove_tree();
    state
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
mod tests {
    use super::*;

    fn state_with(paths: &[&str]) -> State {
        let hash = gix::hash::Kind::Sha1;
        let mut state = State::new(hash);
        for path in paths {
            state.dangerously_push_entry(
                Stat {
                    size: 3,
                    ..Stat::default()
                },
                ObjectId::empty_blob(hash),
                Flags::empty(),
                Mode::FILE,
                path.as_bytes().as_bstr(),
            );
        }
        state.sort_entries();
        state
    }

    #[test]
    fn stat_data_of_unchanged_entries_is_kept_and_written_entries_are_cleared() {
        let old = state_with(&["a", "b"]);
        let new = old.clone();
        let written: HashSet<BString> = [BString::from("b")].into_iter().collect();
        let sources = StatSources {
            written: Some(&written),
            read_now: None,
        };
        let result = with_stats(Path::new("/nonexistent"), new, &old, &sources);
        let sizes: Vec<u32> = result
            .entries()
            .iter()
            .map(|entry| entry.stat.size)
            .collect();
        assert_eq!(sizes, vec![3, 0]);
    }

    #[test]
    fn entry_kinds_follow_the_modes() {
        assert_eq!(entry_kind(Mode::FILE), EntryKind::Blob);
        assert_eq!(entry_kind(Mode::FILE_EXECUTABLE), EntryKind::BlobExecutable);
        assert_eq!(entry_kind(Mode::SYMLINK), EntryKind::Link);
        assert_eq!(entry_kind(Mode::COMMIT), EntryKind::Commit);
        assert!(!has_conflicts(&state_with(&["a"])));
        assert!(merged_entry(&state_with(&["a"]), b"a".as_bstr()).is_some());
    }
}
