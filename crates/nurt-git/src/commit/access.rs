// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Working tree access of `git.commit`: files in their stored form, untracked
//! files, index entry updates, tree contents and changed line counts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};

use crate::repo::GitRepo;
use crate::repo::error as repo_error;
use crate::repo::filter::Filters;
use crate::repo::pathspec::PathFilter;

/// Bytes inspected for a NUL byte when deciding whether content is binary,
/// as `git` does.
pub(crate) const BINARY_PROBE_BYTES: usize = 8000;

/// Maps a failure of the repository library to `E_IO`.
pub(crate) fn lib_error(what: &str, error: impl std::fmt::Display) -> Error {
    Error::new(ErrorCode::Io, format!("cannot {what}: {error}"))
}

/// Content of a working tree file, converted for storage.
#[derive(Debug, Clone)]
pub(crate) struct WorkFile {
    /// Content in the repository form.
    pub(crate) data: Vec<u8>,
    /// Mode.
    pub(crate) kind: EntryKind,
    /// File status for the index entry.
    pub(crate) stat: gix::index::entry::Stat,
}

/// Reader of working tree files with the content filters of the repository.
pub(crate) struct Worktree<'r> {
    repo: &'r GitRepo,
    root: PathBuf,
    filters: Filters<'r>,
    filemode: bool,
    symlinks: bool,
}

impl<'r> Worktree<'r> {
    /// Prepares the reader.
    pub(crate) fn new(repo: &'r GitRepo) -> Result<Self> {
        let root = require_workdir(repo)?.to_path_buf();
        let filters = repo.filters()?;
        Ok(Self {
            repo,
            root,
            filters,
            filemode: repo.config_bool("core.filemode").unwrap_or(cfg!(unix)),
            symlinks: repo.config_bool("core.symlinks").unwrap_or(cfg!(unix)),
        })
    }

    /// Absolute path of a repository path.
    pub(crate) fn absolute(&self, path: &BStr) -> PathBuf {
        path.split_str("/").fold(self.root.clone(), |out, part| {
            out.join(part.to_str_lossy().as_ref())
        })
    }

    /// Reads a file; `None` when it is missing or is a directory.
    ///
    /// `tracked` is the mode recorded in the index, kept for the executable
    /// bit when `core.filemode` is false and for links when `core.symlinks`
    /// is false.
    pub(crate) fn read(
        &mut self,
        path: &BStr,
        tracked: Option<EntryKind>,
    ) -> Result<Option<WorkFile>> {
        let absolute = self.absolute(path);
        let metadata = match std::fs::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(repo_error::io(&absolute, &error)),
        };
        if metadata.is_dir() {
            return Ok(None);
        }
        let (data, kind) = if metadata.file_type().is_symlink() && self.symlinks {
            let target =
                std::fs::read_link(&absolute).map_err(|error| repo_error::io(&absolute, &error))?;
            let target = target.to_string_lossy().replace('\\', "/");
            (target.into_bytes(), EntryKind::Link)
        } else {
            let raw =
                std::fs::read(&absolute).map_err(|error| repo_error::io(&absolute, &error))?;
            let kind = self.file_kind(&metadata, tracked);
            let data = if kind == EntryKind::Link {
                raw
            } else {
                self.filters.to_git(path, &raw)?
            };
            (data, kind)
        };
        let fs_metadata = gix::index::fs::Metadata::from_path_no_follow(&absolute)
            .map_err(|error| repo_error::io(&absolute, &error))?;
        let stat = gix::index::entry::Stat::from_fs(&fs_metadata)
            .map_err(|error| lib_error("read the file time", error))?;
        Ok(Some(WorkFile { data, kind, stat }))
    }

    fn file_kind(&self, metadata: &std::fs::Metadata, tracked: Option<EntryKind>) -> EntryKind {
        if !self.symlinks && tracked == Some(EntryKind::Link) {
            return EntryKind::Link;
        }
        if !self.filemode {
            return match tracked {
                Some(EntryKind::BlobExecutable) => EntryKind::BlobExecutable,
                _ => EntryKind::Blob,
            };
        }
        if is_executable(metadata) {
            EntryKind::BlobExecutable
        } else {
            EntryKind::Blob
        }
    }

    /// Untracked files that are not ignored and that `filter` selects, in
    /// path order. Directories holding another repository are skipped.
    pub(crate) fn untracked(
        &self,
        index: &gix::index::State,
        filter: &PathFilter,
    ) -> Result<Vec<BString>> {
        let mut excludes = self
            .repo
            .gix()
            .excludes(
                index,
                None,
                gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(|error| lib_error("read the ignore rules", error))?;
        let mut found = Vec::new();
        let mut pending = vec![BString::default()];
        while let Some(dir) = pending.pop() {
            let absolute = self.absolute(dir.as_ref());
            let mut names: Vec<(String, std::fs::FileType)> = std::fs::read_dir(&absolute)
                .map_err(|error| repo_error::io(&absolute, &error))?
                .filter_map(std::result::Result::ok)
                .filter_map(|entry| {
                    Some((
                        entry.file_name().into_string().ok()?,
                        entry.file_type().ok()?,
                    ))
                })
                .collect();
            names.sort_by(|left, right| left.0.cmp(&right.0));
            for (name, file_type) in names {
                if name == ".git" {
                    continue;
                }
                let path = join_path(dir.as_ref(), BStr::new(name.as_bytes()));
                let relative = PathBuf::from(path.to_str_lossy().as_ref());
                let is_dir = file_type.is_dir();
                let mode = if is_dir {
                    gix::index::entry::Mode::DIR
                } else {
                    gix::index::entry::Mode::FILE
                };
                let excluded = excludes
                    .at_path(&relative, Some(mode))
                    .map_err(|error| lib_error("read the ignore rules", error))?
                    .is_excluded();
                if excluded {
                    continue;
                }
                if is_dir {
                    let nested = self.absolute(path.as_ref()).join(".git").exists();
                    if !nested && filter.may_contain(path.as_ref()) {
                        pending.push(path);
                    }
                } else if index.entry_range(path.as_ref()).is_none()
                    && filter.matches(path.as_ref())
                {
                    found.push(path);
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

/// Root of the working tree, or `E_GIT_UNSUPPORTED` for a bare repository.
pub(crate) fn require_workdir(repo: &GitRepo) -> Result<&Path> {
    repo.workdir().ok_or_else(|| {
        Error::new(
            ErrorCode::GitUnsupported,
            format!(
                "{} has no working tree; commit the index with `staged: true`",
                repo.git_dir().display()
            ),
        )
        .with_problem("bare_repository")
    })
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const OWNER_EXECUTE: u32 = 0o100;
    metadata.permissions().mode() & OWNER_EXECUTE != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Joins a directory and a name with `/`.
pub(crate) fn join_path(prefix: &BStr, name: &BStr) -> BString {
    let mut path = BString::from(prefix.to_vec());
    if !path.is_empty() {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

/// Identifier of a blob with the given content.
pub(crate) fn blob_id(kind: gix::hash::Kind, data: &[u8]) -> Result<ObjectId> {
    gix::objs::compute_hash(kind, gix::objs::Kind::Blob, data)
        .map_err(|error| lib_error("hash a file", error))
}

/// Index mode of a tree entry kind.
pub(crate) fn index_mode_of(kind: EntryKind) -> gix::index::entry::Mode {
    match kind {
        EntryKind::BlobExecutable => gix::index::entry::Mode::FILE_EXECUTABLE,
        EntryKind::Link => gix::index::entry::Mode::SYMLINK,
        EntryKind::Commit => gix::index::entry::Mode::COMMIT,
        EntryKind::Tree => gix::index::entry::Mode::DIR,
        EntryKind::Blob => gix::index::entry::Mode::FILE,
    }
}

/// Tree entry kind of an index mode; `None` for directory entries.
pub(crate) fn kind_of_index_mode(mode: gix::index::entry::Mode) -> Option<EntryKind> {
    mode.to_tree_entry_mode().map(|mode| mode.kind())
}

/// Stage-0 entry of `path`: object and kind.
pub(crate) fn staged_entry(
    state: &gix::index::State,
    path: &BStr,
) -> Option<(ObjectId, EntryKind)> {
    let entry = state.entry_by_path_and_stage(path, gix::index::entry::Stage::Unconflicted)?;
    if entry
        .flags
        .contains(gix::index::entry::Flags::INTENT_TO_ADD)
    {
        return None;
    }
    Some((entry.id, kind_of_index_mode(entry.mode)?))
}

/// Replaces every entry of `path` (all stages) by one stage-0 entry.
pub(crate) fn upsert_entry(
    state: &mut gix::index::State,
    path: &BStr,
    id: ObjectId,
    kind: EntryKind,
    stat: gix::index::entry::Stat,
) {
    remove_entry(state, path);
    state.dangerously_push_entry(
        stat,
        id,
        gix::index::entry::Flags::empty(),
        index_mode_of(kind),
        path,
    );
    state.sort_entries();
}

/// Removes every entry of `path`, all stages.
pub(crate) fn remove_entry(state: &mut gix::index::State, path: &BStr) {
    state.remove_entries(|_, entry_path, _| entry_path == path);
}

/// Paths of the stage-0 and conflict entries of an index, each once.
pub(crate) fn index_paths(state: &gix::index::State) -> Vec<BString> {
    let mut paths: Vec<BString> = Vec::new();
    for entry in state.entries() {
        let path = entry.path(state);
        if entry.mode == gix::index::entry::Mode::DIR {
            continue;
        }
        if paths.last().is_none_or(|last| last.as_bstr() != path) {
            paths.push(path.to_owned());
        }
    }
    paths
}

/// File of a tree: object and kind.
pub(crate) type TreeFiles = BTreeMap<BString, (ObjectId, EntryKind)>;

/// Files of a tree by path; empty for `None`.
pub(crate) fn tree_files(repo: &GitRepo, tree: Option<ObjectId>) -> Result<TreeFiles> {
    let Some(tree) = tree else {
        return Ok(TreeFiles::new());
    };
    let index = repo
        .gix()
        .index_from_tree(&tree)
        .map_err(|error| lib_error(&format!("read tree {tree}"), error))?;
    let state: gix::index::State = index.into();
    Ok(state
        .entries()
        .iter()
        .filter_map(|entry| {
            Some((
                entry.path(&state).to_owned(),
                (entry.id, kind_of_index_mode(entry.mode)?),
            ))
        })
        .collect())
}

/// Index state with the files of a tree; an empty state for `None`.
pub(crate) fn state_of_tree(repo: &GitRepo, tree: Option<ObjectId>) -> Result<gix::index::State> {
    match tree {
        None => Ok(gix::index::State::new(repo.gix().object_hash())),
        Some(tree) => repo
            .gix()
            .index_from_tree(&tree)
            .map(Into::into)
            .map_err(|error| lib_error(&format!("read tree {tree}"), error)),
    }
}

/// Reads a blob.
pub(crate) fn blob(repo: &GitRepo, id: ObjectId) -> Result<Vec<u8>> {
    repo.gix()
        .find_object(id)
        .map(|object| object.data.clone())
        .map_err(|error| lib_error(&format!("read object {id}"), error))
}

/// Returns `true` when content is binary by the rule of `git`.
pub(crate) fn is_binary(data: &[u8]) -> bool {
    data[..data.len().min(BINARY_PROBE_BYTES)].contains(&0)
}

/// Lines added and removed between two contents, as `git diff --numstat`
/// counts them; `None` for binary content.
pub(crate) fn line_counts(old: &[u8], new: &[u8]) -> Option<(u64, u64)> {
    if is_binary(old) || is_binary(new) {
        return None;
    }
    let input = gix_imara_diff::InternedInput::new(
        gix_imara_diff::sources::byte_lines(old),
        gix_imara_diff::sources::byte_lines(new),
    );
    let mut diff = gix_imara_diff::Diff::compute(gix_imara_diff::Algorithm::Myers, &input);
    diff.postprocess_lines(&input);
    Some(diff.hunks().fold((0_u64, 0_u64), |(added, removed), hunk| {
        (
            added + u64::from(hunk.after.end - hunk.after.start),
            removed + u64::from(hunk.before.end - hunk.before.start),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_detection_and_line_counts() {
        assert!(is_binary(b"a\0b"));
        assert!(!is_binary(b"text\n"));
        assert_eq!(line_counts(b"a\nb\nc\n", b"a\nB\nc\nd\n"), Some((2, 1)));
        assert_eq!(line_counts(b"", b"\0"), None);
    }

    #[test]
    fn entries_are_replaced_and_removed_with_all_stages() {
        let mut state = gix::index::State::new(gix::hash::Kind::Sha1);
        let id = ObjectId::empty_blob(gix::hash::Kind::Sha1);
        let ours = gix::index::entry::Flags::from_stage(gix::index::entry::Stage::Ours);
        state.dangerously_push_entry(
            gix::index::entry::Stat::default(),
            id,
            ours,
            gix::index::entry::Mode::FILE,
            BStr::new("a.txt"),
        );
        upsert_entry(
            &mut state,
            BStr::new("a.txt"),
            id,
            EntryKind::BlobExecutable,
            Default::default(),
        );
        assert_eq!(state.entries().len(), 1);
        assert_eq!(
            staged_entry(&state, BStr::new("a.txt")),
            Some((id, EntryKind::BlobExecutable))
        );
        upsert_entry(
            &mut state,
            BStr::new("b/c.txt"),
            id,
            EntryKind::Blob,
            Default::default(),
        );
        assert_eq!(
            index_paths(&state),
            vec![BString::from("a.txt"), BString::from("b/c.txt")]
        );
        remove_entry(&mut state, BStr::new("a.txt"));
        assert_eq!(index_paths(&state), vec![BString::from("b/c.txt")]);
    }

    #[test]
    fn blob_ids_match_git() {
        assert_eq!(
            blob_id(gix::hash::Kind::Sha1, b"").unwrap(),
            ObjectId::empty_blob(gix::hash::Kind::Sha1)
        );
        assert_eq!(join_path(BStr::new(""), BStr::new("a")), BString::from("a"));
        assert_eq!(
            join_path(BStr::new("d"), BStr::new("a")),
            BString::from("d/a")
        );
    }
}
