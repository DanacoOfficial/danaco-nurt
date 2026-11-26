// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! State of the working tree as `git add -A` would record it in a temporary
//! index: tracked files with their working tree content and untracked files
//! that are not ignored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::entry::{Flags, Mode, Stage};
use gix::objs::tree::EntryKind;
use nurt_contracts::error::Result;

use super::support::{Filters, TreeEntry, repo_error, worktree_path};

/// Where blobs of working tree files go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Blobs {
    /// Written to the object database of the repository.
    Write,
    /// Only hashed; nothing is written.
    HashOnly,
}

/// Files of the working tree by path.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    /// Every file that `git add -A` would record.
    pub(crate) files: BTreeMap<BString, TreeEntry>,
    /// Paths of `files` that are not in the index.
    pub(crate) untracked: BTreeSet<BString>,
    /// Tracked paths whose index entries are in conflict.
    pub(crate) conflicted: bool,
    /// Blobs written to the object database.
    pub(crate) objects_written: u64,
}

/// Records the working tree of `repo`.
pub(crate) fn snapshot_worktree(
    repo: &gix::Repository,
    index: &gix::index::State,
    filters: &mut Filters<'_>,
    blobs: Blobs,
) -> Result<Snapshot> {
    let root = super::support::workdir(repo)?;
    let filemode = repo
        .config_snapshot()
        .boolean("core.filemode")
        .unwrap_or(cfg!(unix));
    let mut snapshot = Snapshot::default();
    let mut hasher = BlobHasher {
        repo,
        blobs,
        written: 0,
    };
    let stat_options = gix::index::entry::stat::Options::default();
    let mut previous: Option<BString> = None;
    for entry in index.entries() {
        let path = entry.path(index);
        if previous.as_ref().is_some_and(|seen| seen.as_bstr() == path) {
            continue;
        }
        previous = Some(path.to_owned());
        if entry.stage() != Stage::Unconflicted {
            snapshot.conflicted = true;
        }
        if let Some(recorded) = record_tracked(
            &root,
            index,
            entry,
            path,
            filters,
            &mut hasher,
            TrackedOptions {
                filemode,
                stat: stat_options,
            },
        )? {
            snapshot.files.insert(path.to_owned(), recorded);
        }
    }
    for path in untracked_files(repo, index)? {
        let file = worktree_path(&root, path.as_bstr())?;
        if let Some(recorded) = hash_file(
            &file,
            path.as_bstr(),
            index,
            filters,
            &mut hasher,
            None,
            filemode,
        )? {
            snapshot.files.insert(path.clone(), recorded);
            snapshot.untracked.insert(path);
        }
    }
    snapshot.objects_written = hasher.written;
    Ok(snapshot)
}

#[derive(Clone, Copy)]
struct TrackedOptions {
    filemode: bool,
    stat: gix::index::entry::stat::Options,
}

fn record_tracked(
    root: &Path,
    index: &gix::index::State,
    entry: &gix::index::Entry,
    path: &BStr,
    filters: &mut Filters<'_>,
    hasher: &mut BlobHasher<'_>,
    options: TrackedOptions,
) -> Result<Option<TreeEntry>> {
    if entry.flags.contains(Flags::SKIP_WORKTREE) || entry.mode == Mode::COMMIT {
        return Ok(entry
            .mode
            .to_tree_entry_mode()
            .map(|mode| (mode.kind(), entry.id)));
    }
    let file = worktree_path(root, path)?;
    let unchanged = entry.stage() == Stage::Unconflicted
        && !entry.flags.contains(Flags::INTENT_TO_ADD)
        && gix::index::fs::Metadata::from_path_no_follow(&file)
            .ok()
            .and_then(|metadata| gix::index::entry::Stat::from_fs(&metadata).ok())
            .is_some_and(|stat| {
                entry.stat.matches(&stat, options.stat)
                    && !entry.stat.is_racy(index.timestamp(), options.stat)
            });
    if unchanged {
        return Ok(entry
            .mode
            .to_tree_entry_mode()
            .map(|mode| (mode.kind(), entry.id)));
    }
    let index_kind = entry.mode.to_tree_entry_mode().map(|mode| mode.kind());
    hash_file(
        &file,
        path,
        index,
        filters,
        hasher,
        index_kind,
        options.filemode,
    )
}

fn hash_file(
    file: &Path,
    path: &BStr,
    index: &gix::index::State,
    filters: &mut Filters<'_>,
    hasher: &mut BlobHasher<'_>,
    index_kind: Option<EntryKind>,
    filemode: bool,
) -> Result<Option<TreeEntry>> {
    let metadata = match std::fs::symlink_metadata(file) {
        Ok(metadata) => metadata,
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(None);
        }
        Err(err) => return Err(repo_error(&format!("read `{}`", file.display()), err)),
    };
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(file)
            .map_err(|err| repo_error(&format!("read `{}`", file.display()), err))?;
        let bytes = gix::path::into_bstr(target)
            .map_err(|err| repo_error(&format!("read `{}`", file.display()), err))?;
        let id = hasher.blob(bytes.as_ref())?;
        return Ok(Some((EntryKind::Link, id)));
    }
    if !metadata.is_file() {
        return Ok(None);
    }
    let content = std::fs::read(file)
        .map_err(|err| repo_error(&format!("read `{}`", file.display()), err))?;
    let content = filters.convert_to_git(path, &content, index)?;
    let id = hasher.blob(&content)?;
    let executable = if filemode {
        is_executable(&metadata)
    } else {
        index_kind == Some(EntryKind::BlobExecutable)
    };
    let kind = if executable {
        EntryKind::BlobExecutable
    } else {
        EntryKind::Blob
    };
    Ok(Some((kind, id)))
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

struct BlobHasher<'repo> {
    repo: &'repo gix::Repository,
    blobs: Blobs,
    written: u64,
}

impl BlobHasher<'_> {
    fn blob(&mut self, content: &[u8]) -> Result<ObjectId> {
        match self.blobs {
            Blobs::Write => {
                let id = self
                    .repo
                    .write_blob(content)
                    .map_err(|err| repo_error("write a blob", err))?;
                self.written += 1;
                Ok(id.detach())
            }
            Blobs::HashOnly => {
                gix::objs::compute_hash(self.repo.object_hash(), gix::object::Kind::Blob, content)
                    .map_err(|err| repo_error("hash a blob", err))
            }
        }
    }
}

/// Untracked files that are not ignored, relative to the working tree root.
fn untracked_files(repo: &gix::Repository, index: &gix::index::State) -> Result<Vec<BString>> {
    let options = repo
        .dirwalk_options()
        .map_err(|err| repo_error("prepare the directory walk", err))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_ignored(None)
        .emit_tracked(false)
        .recurse_repositories(false)
        .emit_pruned(false)
        .emit_empty_directories(false);
    let mut collect = gix::dir::walk::delegate::Collect::default();
    let interrupt = AtomicBool::new(false);
    repo.dirwalk(index, None::<&str>, &interrupt, options, &mut collect)
        .map_err(|err| repo_error("walk the working tree", err))?;
    Ok(collect
        .into_entries_by_path()
        .into_iter()
        .filter(|(entry, _)| {
            entry.status == gix::dir::entry::Status::Untracked
                && matches!(
                    entry.disk_kind,
                    Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink)
                )
        })
        .map(|(entry, _)| entry.rela_path)
        .collect())
}

/// Files that the ignore rules exclude, relative to the working tree root.
/// Ignored directories are listed by the files below them.
pub(crate) fn ignored_paths(
    repo: &gix::Repository,
    index: &gix::index::State,
) -> Result<Vec<BString>> {
    let root = super::support::workdir(repo)?;
    let options = repo
        .dirwalk_options()
        .map_err(|err| repo_error("prepare the directory walk", err))?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_ignored(Some(gix::dir::walk::EmissionMode::Matching))
        .emit_tracked(false)
        .recurse_repositories(false)
        .emit_pruned(false)
        .emit_empty_directories(false);
    let mut collect = gix::dir::walk::delegate::Collect::default();
    let interrupt = AtomicBool::new(false);
    repo.dirwalk(index, None::<&str>, &interrupt, options, &mut collect)
        .map_err(|err| repo_error("walk the working tree", err))?;
    let mut out = Vec::new();
    for (entry, _) in collect.into_entries_by_path() {
        if !matches!(entry.status, gix::dir::entry::Status::Ignored(_)) {
            continue;
        }
        match entry.disk_kind {
            Some(gix::dir::entry::Kind::File | gix::dir::entry::Kind::Symlink) => {
                out.push(entry.rela_path);
            }
            Some(gix::dir::entry::Kind::Directory) => {
                let dir = worktree_path(&root, entry.rela_path.as_bstr())?;
                files_below(&dir, &entry.rela_path, &mut out)?;
            }
            _ => {}
        }
    }
    out.sort();
    Ok(out)
}

fn files_below(dir: &Path, rela: &BString, out: &mut Vec<BString>) -> Result<()> {
    let read_error = |err: std::io::Error| repo_error(&format!("read `{}`", dir.display()), err);
    for item in std::fs::read_dir(dir).map_err(read_error)? {
        let item = item.map_err(read_error)?;
        let mut child = rela.clone();
        child.push(b'/');
        child.extend_from_slice(&super::support::repo_path(Path::new(&item.file_name()))?);
        if item.file_type().map_err(read_error)?.is_dir() {
            files_below(&item.path(), &child, out)?;
        } else {
            out.push(child);
        }
    }
    Ok(())
}

/// Tree of the stage 0 entries of `index`, or `None` when the index has
/// conflicts.
pub(crate) fn index_entries(index: &gix::index::State) -> Option<BTreeMap<BString, TreeEntry>> {
    let mut out = BTreeMap::new();
    for entry in index.entries() {
        if entry.stage() != Stage::Unconflicted {
            return None;
        }
        if entry.flags.contains(Flags::INTENT_TO_ADD) {
            continue;
        }
        if let Some(mode) = entry.mode.to_tree_entry_mode() {
            out.insert(entry.path(index).to_owned(), (mode.kind(), entry.id));
        }
    }
    Some(out)
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[allow(clippy::unwrap_used, reason = "test helper")]
    fn executable_with_mode(mode: u32) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        is_executable(&std::fs::symlink_metadata(&file).unwrap())
    }

    #[test]
    fn only_the_owner_execute_bit_makes_a_file_executable() {
        assert!(executable_with_mode(0o755));
        assert!(executable_with_mode(0o700));
        assert!(!executable_with_mode(0o644));
        assert!(!executable_with_mode(0o641));
        assert!(!executable_with_mode(0o611));
    }
}
