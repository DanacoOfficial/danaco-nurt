// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading the working tree: comparison with index entries, the form in
//! which files are stored and the list of untracked files.
//!
//! Changes of the working tree are planned by `nurt_git::apply`; this module
//! only observes it. Configured filter drivers follow the policy of
//! [`GitRepo::filters`].

use std::path::PathBuf;

use gix::bstr::{BStr, BString};
use gix::hash::ObjectId;
use gix::index::State;
use gix::index::entry::{Mode, Stat};
use nurt_contracts::error::Result;
use nurt_git::repo::GitRepo;
use nurt_git::repo::filter::Filters;

use super::repo::{io_error, worktree_path};

/// What the working tree holds at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Probe {
    /// Nothing, or a parent component is not a directory.
    Absent,
    /// A regular file.
    File {
        /// Executable bit.
        exec: bool,
        /// Content.
        data: Vec<u8>,
    },
    /// A symbolic link.
    Link(PathBuf),
    /// A directory or another kind of entry.
    Other,
}

/// Reads the working tree and converts file content to the form stored in
/// the repository.
pub(crate) struct Worktree<'r> {
    repo: &'r GitRepo,
    filters: Filters<'r>,
    file_mode: bool,
    symlinks: bool,
    index_mtime: Option<u32>,
}

impl<'r> Worktree<'r> {
    /// Working tree of `repo`.
    pub(crate) fn new(repo: &'r GitRepo) -> Result<Self> {
        let caps = repo
            .gix()
            .filesystem_options()
            .map_err(|cause| io_error("cannot read the core settings", cause))?;
        let index_mtime = std::fs::metadata(repo.index_path())
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|elapsed| u32::try_from(elapsed.as_secs()).ok());
        Ok(Self {
            repo,
            filters: repo.filters()?,
            file_mode: caps.executable_bit,
            symlinks: caps.symlink,
            index_mtime,
        })
    }

    fn probe(&self, rel: &BStr) -> Result<Probe> {
        let path = worktree_path(self.repo, rel);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(cause)
                if matches!(
                    cause.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(Probe::Absent);
            }
            Err(cause) => return Err(io_error("cannot read the working tree", cause)),
        };
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target =
                std::fs::read_link(&path).map_err(|cause| io_error("cannot read link", cause))?;
            Ok(Probe::Link(target))
        } else if kind.is_file() {
            let data = std::fs::read(&path).map_err(|cause| io_error("cannot read file", cause))?;
            Ok(Probe::File {
                exec: is_executable(&meta),
                data,
            })
        } else {
            Ok(Probe::Other)
        }
    }

    fn hash_blob(&self, data: &[u8]) -> Result<ObjectId> {
        gix::objs::compute_hash(self.repo.gix().object_hash(), gix::objs::Kind::Blob, data)
            .map_err(|cause| io_error("cannot hash content", cause))
    }

    fn stored(&mut self, rel: &BStr, probe: &Probe) -> Result<Option<(Vec<u8>, Mode)>> {
        Ok(match probe {
            Probe::File { exec, data } => {
                let mode = if *exec && self.file_mode {
                    Mode::FILE_EXECUTABLE
                } else {
                    Mode::FILE
                };
                Some((self.filters.to_git(rel, data)?, mode))
            }
            Probe::Link(target) => Some((
                gix::path::into_bstr(target.as_path())
                    .map_err(|cause| io_error("invalid link target", cause))?
                    .into_owned()
                    .into(),
                Mode::SYMLINK,
            )),
            Probe::Absent | Probe::Other => None,
        })
    }

    /// Blob of the working tree entry at `rel` as it would be stored, with
    /// its mode; `None` when nothing that git tracks is there.
    pub(crate) fn stored_form(&mut self, rel: &BStr) -> Result<Option<(Vec<u8>, Mode)>> {
        let probe = self.probe(rel)?;
        self.stored(rel, &probe)
    }

    /// Whether the working tree holds the content of `entry`.
    pub(crate) fn matches(&mut self, rel: &BStr, entry: &gix::index::Entry) -> Result<bool> {
        if entry.mode == Mode::COMMIT {
            return Ok(true);
        }
        let probe = self.probe(rel)?;
        if let Probe::File { exec, .. } = &probe
            && self.file_mode
            && entry.mode != Mode::SYMLINK
            && *exec != (entry.mode == Mode::FILE_EXECUTABLE)
        {
            return Ok(false);
        }
        if self.stat_matches(rel, &entry.stat) {
            return Ok(true);
        }
        let stored = match (&probe, entry.mode == Mode::SYMLINK) {
            (Probe::Link(_), true) | (Probe::File { .. }, false) => self.stored(rel, &probe)?,
            (Probe::File { data, .. }, true) if !self.symlinks => {
                Some((data.clone(), Mode::SYMLINK))
            }
            _ => None,
        };
        match stored {
            Some((data, _)) => Ok(self.hash_blob(&data)? == entry.id),
            None => Ok(false),
        }
    }

    fn stat_matches(&self, rel: &BStr, stat: &Stat) -> bool {
        if *stat == Stat::default() {
            return false;
        }
        let racy = self
            .index_mtime
            .is_none_or(|index_mtime| stat.mtime.secs >= index_mtime);
        if racy {
            return false;
        }
        let path = worktree_path(self.repo, rel);
        gix::index::fs::Metadata::from_path_no_follow(&path)
            .ok()
            .and_then(|meta| Stat::from_fs(&meta).ok())
            .is_some_and(|current| current == *stat)
    }

    /// Untracked files that are not ignored, one path per file, sorted.
    pub(crate) fn untracked(&self, index: &State) -> Result<Vec<BString>> {
        let interrupt = std::sync::atomic::AtomicBool::new(false);
        let options = self
            .repo
            .gix()
            .dirwalk_options()
            .map_err(|cause| io_error("cannot list untracked files", cause))?
            .emit_untracked(gix::dir::walk::EmissionMode::Matching);
        let mut collect = gix::dir::walk::delegate::Collect::default();
        self.repo
            .gix()
            .dirwalk(index, None::<&str>, &interrupt, options, &mut collect)
            .map_err(|cause| io_error("cannot list untracked files", cause))?;
        Ok(collect
            .into_entries_by_path()
            .into_iter()
            .filter(|(entry, _)| matches!(entry.status, gix::dir::entry::Status::Untracked))
            .filter(|(entry, _)| !matches!(entry.disk_kind, Some(gix::dir::entry::Kind::Directory)))
            .map(|(entry, _)| entry.rela_path)
            .collect())
    }
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}
