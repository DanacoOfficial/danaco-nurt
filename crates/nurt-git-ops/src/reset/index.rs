// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading, building and serializing the index.

use std::path::PathBuf;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::State;
use gix::index::entry::{Flags, Mode, Stat};
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{ErrorCode, Result};

use super::repo::{Repo, error, io_error};

/// The current index, empty when the file does not exist.
pub(crate) fn load(repo: &Repo) -> Result<State> {
    let path = repo.index_path();
    if !path.exists() {
        return Ok(State::new(repo.repo.object_hash()));
    }
    let file = gix::index::File::at(&path, repo.repo.object_hash(), false, Default::default())
        .map_err(|cause| io_error("cannot read the index", cause))?;
    let state = file.into_parts().0;
    if state.is_sparse() {
        return Err(error(
            ErrorCode::GitUnsupported,
            "the index is sparse; expand it with `git sparse-checkout disable` first",
        )
        .with_problem("sparse_index"));
    }
    Ok(state)
}

/// Index with the entries of a tree and empty stat data.
pub(crate) fn from_tree(repo: &Repo, tree: ObjectId) -> Result<State> {
    repo.repo
        .index_from_tree(&tree)
        .map(|file| file.into_parts().0)
        .map_err(|cause| io_error("cannot read tree", cause))
}

/// Whether the index has entries in a conflict stage.
pub(crate) fn has_conflicts(state: &State) -> bool {
    state
        .entries()
        .iter()
        .any(|entry| entry.stage() != gix::index::entry::Stage::Unconflicted)
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

/// Index entry mode of a tree entry kind.
pub(crate) fn entry_mode(kind: EntryKind) -> Mode {
    match kind {
        EntryKind::BlobExecutable => Mode::FILE_EXECUTABLE,
        EntryKind::Link => Mode::SYMLINK,
        EntryKind::Commit => Mode::COMMIT,
        EntryKind::Blob | EntryKind::Tree => Mode::FILE,
    }
}

/// Writes the tree of the merged entries of `state`, like `git write-tree`;
/// entries added with intent to add are left out.
pub(crate) fn write_tree(repo: &Repo, state: &State) -> Result<ObjectId> {
    let empty = ObjectId::empty_tree(repo.repo.object_hash());
    let mut editor = repo
        .repo
        .edit_tree(empty)
        .map_err(|cause| io_error("cannot edit tree", cause))?;
    for entry in state.entries() {
        if entry.stage() != gix::index::entry::Stage::Unconflicted
            || entry.flags.contains(Flags::INTENT_TO_ADD)
        {
            continue;
        }
        editor
            .upsert(entry.path(state), entry_kind(entry.mode), entry.id)
            .map_err(|cause| io_error("cannot edit tree", cause))?;
    }
    editor
        .write()
        .map(gix::Id::detach)
        .map_err(|cause| io_error("cannot write tree", cause))
}

/// Merged entry of `path`: object, mode and stat.
pub(crate) fn merged_entry<'a>(state: &'a State, path: &BStr) -> Option<&'a gix::index::Entry> {
    state.entry_by_path_and_stage(path, gix::index::entry::Stage::Unconflicted)
}

/// New index content and the entries whose stat data is read from the
/// working tree when the index is written.
pub(crate) struct IndexUpdate {
    /// Entries of the new index.
    pub(crate) state: State,
    /// Paths whose stat data is taken from the working tree at write time.
    pub(crate) needs_stat: Vec<BString>,
    /// Path of the index file.
    pub(crate) path: PathBuf,
    /// Root of the working tree.
    pub(crate) workdir: PathBuf,
}

impl IndexUpdate {
    /// Index for `state` with stat data copied from `old` for entries with
    /// the same path, object and mode; the remaining merged entries are
    /// listed in `needs_stat` when `fresh` accepts them.
    pub(crate) fn new(
        repo: &Repo,
        mut state: State,
        old: &State,
        fresh: &mut dyn FnMut(&BStr, &gix::index::Entry) -> Result<bool>,
    ) -> Result<Self> {
        let mut needs_stat = Vec::new();
        let paths: Vec<BString> = state
            .entries()
            .iter()
            .map(|entry| entry.path(&state).to_owned())
            .collect();
        for (entry, path) in state.entries_mut().iter_mut().zip(&paths) {
            if entry.stage() != gix::index::entry::Stage::Unconflicted {
                continue;
            }
            let same = merged_entry(old, path.as_bstr())
                .filter(|previous| previous.id == entry.id && previous.mode == entry.mode);
            match same {
                Some(previous) if previous.stat != Stat::default() => {
                    entry.stat = previous.stat;
                    entry.flags |= previous.flags & Flags::SKIP_WORKTREE;
                }
                _ => {
                    if fresh(path.as_bstr(), entry)? {
                        needs_stat.push(path.clone());
                    }
                }
            }
        }
        Ok(Self {
            state,
            needs_stat,
            path: repo.index_path(),
            workdir: repo.workdir.clone(),
        })
    }

    /// Serialized index; stat data of `needs_stat` entries is read now.
    pub(crate) fn serialize(mut self) -> Result<Vec<u8>> {
        for path in &self.needs_stat {
            let file = super::repo::join_slash(&self.workdir, &path.to_str_lossy());
            let Ok(meta) = gix::index::fs::Metadata::from_path_no_follow(&file) else {
                continue;
            };
            let Ok(stat) = Stat::from_fs(&meta) else {
                continue;
            };
            if let Some(index) = self
                .state
                .entry_index_by_path_and_stage(path.as_bstr(), gix::index::entry::Stage::Unconflicted)
                && let Some(entry) = self.state.entries_mut().get_mut(index)
            {
                entry.stat = stat;
            }
        }
        self.state.remove_tree();
        let file = gix::index::File::from_state(self.state, self.path);
        let mut out = Vec::new();
        file.write_to(&mut out, gix::index::write::Options::default())
            .map_err(|cause| io_error("cannot serialize the index", cause))?;
        Ok(out)
    }
}
