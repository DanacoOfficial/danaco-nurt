// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Observation of the working tree: entry kinds, expected states, the
//! up-to-date check of tracked files and the classification of untracked
//! files.

use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice, ByteVec};
use gix_index::entry::{Mode, Stat};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::ContentHash;
use nurt_contracts::journal::Expect;
use sha2::{Digest, Sha256};

use super::error::{io_error, repo_error, unsafe_path};
use super::filter::{Direction, Filters};

/// Kind of a working tree entry found by `lstat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Node {
    /// No entry.
    Missing,
    /// Regular file.
    File {
        /// The executable bit is set.
        executable: bool,
    },
    /// Symbolic link.
    Symlink,
    /// Directory.
    Dir,
    /// Device, FIFO or socket.
    Other,
}

impl Node {
    pub(crate) fn exists(self) -> bool {
        self != Self::Missing
    }

    pub(crate) fn is_dir(self) -> bool {
        self == Self::Dir
    }
}

/// An entry of the working tree as seen when the plan was made.
#[derive(Debug, Clone)]
pub(crate) struct Observation {
    pub(crate) node: Node,
    pub(crate) meta: Option<std::fs::Metadata>,
}

/// Converts a path relative to the working tree into a native path below
/// `root`.
///
/// Fails with `E_GIT_UNSUPPORTED` when the path cannot be represented on
/// this system, such as a name that is not valid Unicode on Windows.
pub(crate) fn native_path(root: &Path, rela: &BStr) -> Result<PathBuf> {
    let native = gix_path::to_native_path_on_windows(rela)
        .map_err(|err| unsafe_path(rela, &err.to_string()))?;
    Ok(root.join(native))
}

/// Reads the entry at `path` without following a final symbolic link.
pub(crate) fn observe_native(path: &Path) -> Result<Observation> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(Observation {
            node: node_of(&meta),
            meta: Some(meta),
        }),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(Observation {
                node: Node::Missing,
                meta: None,
            })
        }
        Err(err) => Err(io_error(&err, path)),
    }
}

fn node_of(meta: &std::fs::Metadata) -> Node {
    let kind = meta.file_type();
    if kind.is_symlink() {
        Node::Symlink
    } else if kind.is_dir() {
        Node::Dir
    } else if kind.is_file() {
        Node::File {
            executable: is_executable(meta),
        }
    } else {
        Node::Other
    }
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const USER_EXECUTE: u32 = 0o100;
    meta.permissions().mode() & USER_EXECUTE != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Expected state of an observed entry for the journal check.
///
/// On Unix the entity tag (size, modification time, device and inode) is
/// used; elsewhere files are identified by the hash of their content, and
/// other entries are not checked.
pub(crate) fn expect_of(path: &Path, observation: &Observation) -> Result<Expect> {
    let Some(meta) = &observation.meta else {
        return Ok(Expect::Absent);
    };
    etag_expect(path, observation.node, meta)
}

#[cfg(unix)]
fn etag_expect(_path: &Path, _node: Node, meta: &std::fs::Metadata) -> Result<Expect> {
    use nurt_contracts::fs::{Etag, FileId};
    use std::os::unix::fs::MetadataExt;
    const NANOS_PER_SECOND: i64 = 1_000_000_000;
    Ok(Expect::Etag(Etag {
        size: meta.size(),
        modified_ns: meta
            .mtime()
            .saturating_mul(NANOS_PER_SECOND)
            .saturating_add(meta.mtime_nsec()),
        file_id: FileId {
            volume: meta.dev(),
            low: meta.ino(),
            high: 0,
        },
        hash: None,
    }))
}

#[cfg(not(unix))]
fn etag_expect(path: &Path, node: Node, _meta: &std::fs::Metadata) -> Result<Expect> {
    match node {
        Node::File { .. } => {
            let bytes = std::fs::read(path).map_err(|err| io_error(&err, path))?;
            Ok(Expect::Hash(content_hash(&bytes)))
        }
        _ => Ok(Expect::Unchecked),
    }
}

/// SHA-256 of content, as used by the journal.
pub(crate) fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

/// Checks that the entry at `path` still has the state `expect`; used for
/// files guarded by a lock that the journal cannot check atomically.
pub(crate) fn verify_expect(path: &Path, expect: &Expect) -> Result<()> {
    let observation = observe_native(path)?;
    let matches = match expect {
        Expect::Unchecked => true,
        Expect::Absent => !observation.node.exists(),
        Expect::Etag(_) => expect_of(path, &observation)? == *expect,
        Expect::Hash(hash) => match observation.node {
            Node::File { .. } => {
                let bytes = std::fs::read(path).map_err(|err| io_error(&err, path))?;
                content_hash(&bytes) == *hash
            }
            _ => false,
        },
    };
    if matches {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::Stale,
            format!(
                "'{}' changed after the plan was made; repeat the command",
                path.display()
            ),
        )
        .with_found(path.display().to_string()))
    }
}

/// The working tree of a repository with the settings that govern how its
/// files compare to index entries.
pub(crate) struct Worktree<'r> {
    pub(crate) root: PathBuf,
    pub(crate) caps: gix_fs::Capabilities,
    repo: &'r gix::Repository,
    stat_options: gix_index::entry::stat::Options,
    index_timestamp: (i64, u32),
    filters: Filters<'r>,
    excludes: gix_worktree::Stack,
}

impl<'r> Worktree<'r> {
    /// Worktree view for the current `index` of `repo`.
    pub(crate) fn new(
        repo: &'r gix::Repository,
        root: PathBuf,
        index: &gix_index::State,
    ) -> Result<Self> {
        let caps = repo
            .filesystem_options()
            .map_err(|err| repo_error(err, "read the core settings"))?;
        let stat_options = repo
            .stat_options()
            .map_err(|err| repo_error(err, "read the core settings"))?;
        let excludes = repo
            .excludes(
                index,
                None,
                gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(|err| repo_error(err, "read the ignore rules"))?
            .detach();
        let timestamp = index.timestamp();
        let index_timestamp = (timestamp.unix_seconds(), timestamp.nanoseconds());
        Ok(Self {
            root,
            caps,
            repo,
            stat_options,
            index_timestamp,
            filters: Filters::new(repo, index, Direction::ToGit)?,
            excludes,
        })
    }

    pub(crate) fn path(&self, rela: &BStr) -> Result<PathBuf> {
        native_path(&self.root, rela)
    }

    pub(crate) fn observe(&self, rela: &BStr) -> Result<Observation> {
        observe_native(&self.path(rela)?)
    }

    /// Whether `rela` matches the ignore rules (`.gitignore`,
    /// `info/exclude`, `core.excludesFile`).
    pub(crate) fn is_ignored(&mut self, rela: &BStr, is_dir: bool) -> Result<bool> {
        let mode = is_dir.then_some(Mode::DIR);
        let platform = self
            .excludes
            .at_entry(rela, mode, &self.repo.objects)
            .map_err(|err| repo_error(err, "read the ignore rules"))?;
        Ok(platform.is_excluded())
    }

    /// Whether the working tree file of a tracked `entry` agrees with it, by
    /// the rules `git` applies before overwriting or removing it: a missing
    /// file agrees, a file of another kind or with another executable bit
    /// does not, and a file whose statistics differ from the entry, or are
    /// racy, is compared by content after the clean filters.
    pub(crate) fn is_uptodate(
        &mut self,
        entry: &gix_index::Entry,
        rela: &BStr,
        index: &gix_index::State,
    ) -> Result<bool> {
        if entry.flags.contains(gix_index::entry::Flags::INTENT_TO_ADD) {
            return Ok(false);
        }
        let observation = self.observe(rela)?;
        if !observation.node.exists() {
            return Ok(true);
        }
        if entry.mode == Mode::COMMIT {
            return Ok(observation.node.is_dir());
        }
        if !self.kind_matches(entry.mode, observation.node) {
            return Ok(false);
        }
        let stat = stat_of(&self.path(rela)?)?;
        if entry.stat.size != 0 && stat.size != entry.stat.size {
            return Ok(false);
        }
        if !self.is_racy(&entry.stat) && entry.stat.matches(&stat, self.stat_options) {
            return Ok(true);
        }
        let id = self.hash_worktree(rela, observation.node, index)?;
        Ok(id == entry.id)
    }

    /// Whether an entry with `stat` may have changed within the time
    /// granularity of the index write, so that only its content decides.
    fn is_racy(&self, stat: &Stat) -> bool {
        let (secs, nsecs) = self.index_timestamp;
        match secs.cmp(&i64::from(stat.mtime.secs)) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Equal => {
                !(self.stat_options.use_nsec && self.stat_options.check_stat)
                    || nsecs <= stat.mtime.nsecs
            }
            std::cmp::Ordering::Greater => false,
        }
    }

    fn kind_matches(&self, mode: Mode, node: Node) -> bool {
        match (mode, node) {
            (Mode::SYMLINK, Node::Symlink) => true,
            (Mode::SYMLINK, Node::File { .. }) => !self.caps.symlink,
            (Mode::FILE, Node::File { executable }) => !executable || !self.caps.executable_bit,
            (Mode::FILE_EXECUTABLE, Node::File { executable }) => {
                executable || !self.caps.executable_bit
            }
            _ => false,
        }
    }

    /// Object id of the working tree entry at `rela` after the clean
    /// filters, as `git hash-object` would compute it.
    pub(crate) fn hash_worktree(
        &mut self,
        rela: &BStr,
        node: Node,
        index: &gix_index::State,
    ) -> Result<gix::ObjectId> {
        let path = self.path(rela)?;
        let data = match node {
            Node::Symlink => {
                let target = std::fs::read_link(&path).map_err(|err| io_error(&err, &path))?;
                let target = gix_path::into_bstr(target)
                    .map_err(|err| repo_error(err, "read a link target"))?
                    .into_owned();
                gix_path::to_unix_separators_on_windows(target)
                    .into_owned()
                    .into()
            }
            _ => {
                let bytes = std::fs::read(&path).map_err(|err| io_error(&err, &path))?;
                self.filters.clean(rela, &bytes, index)?
            }
        };
        gix_object::compute_hash(self.repo.object_hash(), gix_object::Kind::Blob, &data)
            .map_err(|err| repo_error(err, "hash a working tree file"))
    }

    /// Untracked entries below the directory `rela` that are not ignored,
    /// skipping the paths for which `tracked` returns true. Ignored
    /// directories are not entered.
    pub(crate) fn untracked_below(
        &mut self,
        rela: &BStr,
        tracked: &dyn Fn(&BStr) -> bool,
    ) -> Result<Vec<BString>> {
        let mut found = Vec::new();
        let mut pending = vec![rela.to_owned()];
        while let Some(dir) = pending.pop() {
            let native = self.path(dir.as_bstr())?;
            let reader = std::fs::read_dir(&native).map_err(|err| io_error(&err, &native))?;
            for item in reader {
                let item = item.map_err(|err| io_error(&err, &native))?;
                let name = gix_path::os_str_into_bstr(&item.file_name())
                    .map_err(|_| repo_error("not valid unicode", "read a directory"))?
                    .to_owned();
                let mut child = dir.clone();
                child.push_byte(b'/');
                child.push_str(&name);
                if tracked(child.as_bstr()) {
                    continue;
                }
                let is_dir = item.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
                if self.is_ignored(child.as_bstr(), is_dir)? {
                    continue;
                }
                if is_dir {
                    pending.push(child);
                } else {
                    found.push(child);
                }
            }
        }
        found.sort();
        Ok(found)
    }
}

/// Index statistics of a working tree entry.
pub(crate) fn stat_of(path: &Path) -> Result<Stat> {
    let meta =
        gix_index::fs::Metadata::from_path_no_follow(path).map_err(|err| io_error(&err, path))?;
    Stat::from_fs(&meta).map_err(|err| repo_error(err, "read file times"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn modification_time_after_the_year_2262_saturates_like_the_file_system_back_ends() {
        let dir = tempfile::tempdir().expect("directory");
        let path = dir.path().join("late");
        let file = std::fs::File::create(&path).expect("file");
        let late = std::time::UNIX_EPOCH + std::time::Duration::from_secs(10_000_000_000);
        if file.set_modified(late).is_err() {
            return;
        }
        let meta = std::fs::symlink_metadata(&path).expect("metadata");
        let Ok(Expect::Etag(etag)) = etag_expect(&path, Node::File { executable: false }, &meta)
        else {
            panic!("an entity tag was expected");
        };
        assert_eq!(etag.modified_ns, i64::MAX);
    }
}
