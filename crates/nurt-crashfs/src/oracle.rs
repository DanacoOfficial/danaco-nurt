// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Oracle checking that previous content is never lost.
//!
//! The oracle compares snapshots of a directory tree taken through [`Fs`]
//! with an expected model computed independently of the journal: the tree
//! before an operation and the tree after it, derived by applying
//! [`Change`] values. After a crash and after recovery it checks that
//!
//! * the previous content of every target exists in at least one place (the
//!   tree or a preserved copy anywhere on the volume),
//! * the tree equals the state before or after the operation, never a mixture,
//! * no temporary entries remain after recovery,
//! * a second recovery changes nothing,
//! * no injected error ended silently,
//! * operation numbers in the history are unique.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use nurt_contracts::fs::{AbsPath, ContentHash, FileKind, Follow, Fs, FsResult};
use nurt_contracts::journal::{OpId, OperationRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Name prefix of temporary entries that recovery removes.
pub const TEMP_PREFIX: &str = ".nurt-tmp-";

/// State of one entry in a [`TreeSnapshot`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntrySnapshot {
    /// Regular file.
    File {
        /// Content digest.
        hash: ContentHash,
        /// Size in bytes.
        size: u64,
        /// Permission bits.
        mode: u32,
        /// Extended attributes as name and value pairs, sorted by name.
        xattrs: Vec<(OsString, Vec<u8>)>,
    },
    /// Directory.
    Directory {
        /// Permission bits.
        mode: u32,
    },
    /// Symbolic link or junction.
    Link {
        /// Stored target.
        target: PathBuf,
    },
}

/// Content and metadata of a tree, keyed by path relative to its root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSnapshot {
    entries: BTreeMap<PathBuf, EntrySnapshot>,
}

impl TreeSnapshot {
    /// Captures the tree below `root`, skipping relative paths listed in
    /// `exclude` and everything below them. Modification times are not
    /// recorded: recovery may legitimately change them.
    ///
    /// # Errors
    ///
    /// Fails with the error of the first [`Fs`] call that fails.
    pub fn capture(fs: &dyn Fs, root: &AbsPath, exclude: &[&Path]) -> FsResult<Self> {
        let mut entries = BTreeMap::new();
        let mut stack = vec![PathBuf::new()];
        while let Some(relative) = stack.pop() {
            let dir = Self::absolute(root, &relative)?;
            let listed: Vec<_> = fs.read_dir(&dir)?.collect::<FsResult<_>>()?;
            for entry in listed {
                let child = relative.join(&entry.name);
                if exclude.iter().any(|skip| child.starts_with(skip)) {
                    continue;
                }
                let path = Self::absolute(root, &child)?;
                let snapshot = Self::entry(fs, &path)?;
                if matches!(snapshot, EntrySnapshot::Directory { .. }) {
                    stack.push(child.clone());
                }
                entries.insert(child, snapshot);
            }
        }
        Ok(Self { entries })
    }

    /// Captures several trees, each keyed below its position in `roots`
    /// (`0`, `1`, and so on); used for snapshots of every volume.
    ///
    /// # Errors
    ///
    /// Fails with the error of the first [`Fs`] call that fails.
    pub fn capture_all(fs: &dyn Fs, roots: &[AbsPath]) -> FsResult<Self> {
        let mut entries = BTreeMap::new();
        for (index, root) in roots.iter().enumerate() {
            let prefix = PathBuf::from(index.to_string());
            let tree = Self::capture(fs, root, &[])?;
            entries.extend(
                tree.entries
                    .into_iter()
                    .map(|(path, entry)| (prefix.join(path), entry)),
            );
        }
        Ok(Self { entries })
    }

    fn absolute(root: &AbsPath, relative: &Path) -> FsResult<AbsPath> {
        AbsPath::new(root.as_path().join(relative))
    }

    fn entry(fs: &dyn Fs, path: &AbsPath) -> FsResult<EntrySnapshot> {
        let meta = fs.extended_metadata(path, Follow::NoFollow)?;
        Ok(match meta.basic.kind {
            FileKind::Directory => EntrySnapshot::Directory {
                mode: meta.basic.mode,
            },
            FileKind::Symlink | FileKind::Junction => EntrySnapshot::Link {
                target: fs.read_link(path)?,
            },
            _ => {
                let mut xattrs: Vec<_> = meta
                    .xattrs
                    .into_iter()
                    .map(|xattr| (xattr.name, xattr.value))
                    .collect();
                xattrs.sort();
                EntrySnapshot::File {
                    hash: fs.content_hash(path)?,
                    size: meta.basic.size,
                    mode: meta.basic.mode,
                    xattrs,
                }
            }
        })
    }

    /// Entries keyed by relative path.
    pub fn entries(&self) -> &BTreeMap<PathBuf, EntrySnapshot> {
        &self.entries
    }

    /// Entry at a relative path.
    pub fn get(&self, relative: &Path) -> Option<&EntrySnapshot> {
        self.entries.get(relative)
    }

    /// Digests of every file content in the tree.
    pub fn hashes(&self) -> HashSet<ContentHash> {
        self.entries
            .values()
            .filter_map(|entry| match entry {
                EntrySnapshot::File { hash, .. } => Some(*hash),
                _ => None,
            })
            .collect()
    }

    /// Relative paths whose entry differs between `self` and `other`,
    /// including paths present in only one of them.
    pub fn differences(&self, other: &TreeSnapshot) -> Vec<PathBuf> {
        let paths: BTreeSet<&PathBuf> = self.entries.keys().chain(other.entries.keys()).collect();
        paths
            .into_iter()
            .filter(|path| self.entries.get(*path) != other.entries.get(*path))
            .cloned()
            .collect()
    }

    /// Snapshot with `change` applied: the expected state after an operation,
    /// computed without the journal.
    #[must_use]
    pub fn with(&self, change: &Change) -> TreeSnapshot {
        let mut next = self.clone();
        next.apply(change);
        next
    }

    fn apply(&mut self, change: &Change) {
        match change {
            Change::Write { path, content } => {
                let (mode, xattrs) = match self.entries.get(path) {
                    Some(EntrySnapshot::File { mode, xattrs, .. }) => (*mode, xattrs.clone()),
                    _ => (DEFAULT_FILE_MODE, Vec::new()),
                };
                self.entries.insert(
                    path.clone(),
                    EntrySnapshot::File {
                        hash: digest(content),
                        size: content.len() as u64,
                        mode,
                        xattrs,
                    },
                );
            }
            Change::Remove { path } => self.entries.retain(|key, _| !key.starts_with(path)),
            Change::Rename { from, to } => {
                self.entries.retain(|key, _| !key.starts_with(to));
                let moved: Vec<_> = self
                    .entries
                    .iter()
                    .filter(|(key, _)| key.starts_with(from))
                    .map(|(key, entry)| (key.clone(), entry.clone()))
                    .collect();
                for (key, entry) in moved {
                    self.entries.remove(&key);
                    let suffix = key.strip_prefix(from).unwrap_or(Path::new(""));
                    self.entries.insert(to.join(suffix), entry);
                }
            }
            Change::CreateDir { path, mode } => {
                self.entries
                    .insert(path.clone(), EntrySnapshot::Directory { mode: *mode });
            }
            Change::SetMode {
                path,
                mode: new_mode,
            } => {
                if let Some(EntrySnapshot::File { mode, .. } | EntrySnapshot::Directory { mode }) =
                    self.entries.get_mut(path)
                {
                    *mode = *new_mode;
                }
            }
        }
    }
}

/// Permission bits of a file created by a [`Change::Write`] without a
/// previous file.
const DEFAULT_FILE_MODE: u32 = 0o644;

fn digest(content: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(content).into())
}

/// Change made by an operation, as seen in the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Change {
    /// File created or overwritten with `content`.
    Write {
        /// Relative path.
        path: PathBuf,
        /// New content.
        content: Vec<u8>,
    },
    /// File or tree removed.
    Remove {
        /// Relative path.
        path: PathBuf,
    },
    /// Entry moved, replacing the destination.
    Rename {
        /// Relative source path.
        from: PathBuf,
        /// Relative destination path.
        to: PathBuf,
    },
    /// Directory created.
    CreateDir {
        /// Relative path.
        path: PathBuf,
        /// Permission bits.
        mode: u32,
    },
    /// Permission bits changed.
    SetMode {
        /// Relative path.
        path: PathBuf,
        /// New permission bits.
        mode: u32,
    },
}

/// Which state a tree is expected to be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expected {
    /// The state before the operation: it did not commit durably.
    Before,
    /// The state after the operation: it committed durably.
    After,
    /// Either state; used before recovery and when the commit point is
    /// unknown.
    Either,
}

/// State a tree matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matched {
    /// The state before the operation.
    Before,
    /// The state after the operation.
    After,
}

/// Result of an operation as reported to its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpOutcome {
    /// Committed; `irreversible` is true when some previous content was not
    /// preserved and the result says so.
    Committed {
        /// Whether the result lists irreversible targets.
        irreversible: bool,
    },
    /// Rolled back with an error naming the failing target.
    RolledBack,
    /// Failed without rollback and without an irreversible marker.
    FailedSilently,
    /// Interrupted by a simulated crash.
    Crashed,
}

/// Property violated after a crash or an injected error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "violation", rename_all = "snake_case")]
pub enum Violation {
    /// Previous content of a target exists nowhere on the volume.
    PreviousContentLost {
        /// Relative path of the target before the operation.
        path: PathBuf,
    },
    /// The tree equals neither the state before nor the state after.
    IntermediateState {
        /// Paths that differ from the state before.
        from_before: Vec<PathBuf>,
        /// Paths that differ from the state after.
        from_after: Vec<PathBuf>,
    },
    /// The tree is a complete state, but not the expected one.
    WrongState {
        /// Expected state.
        expected: Expected,
        /// State found.
        found: Matched,
    },
    /// A temporary entry survived recovery.
    TemporaryLeftover {
        /// Relative path of the entry.
        path: PathBuf,
    },
    /// A second recovery changed the tree.
    NotIdempotent {
        /// Paths changed by the second recovery.
        changed: Vec<PathBuf>,
    },
    /// An injected error ended without rollback or irreversible marker.
    SilentError,
    /// Two history records share an operation number.
    DuplicateOperation {
        /// Number found twice.
        op: u64,
    },
    /// A file system call made by the check itself failed.
    Unreadable {
        /// Description of the failure.
        detail: String,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreviousContentLost { path } => {
                write!(f, "previous content of {} is lost", path.display())
            }
            Self::IntermediateState {
                from_before,
                from_after,
            } => write!(
                f,
                "intermediate state: {} paths differ from the state before, {} from the state after",
                from_before.len(),
                from_after.len()
            ),
            Self::WrongState { expected, found } => {
                write!(
                    f,
                    "expected the {expected:?} state, found the {found:?} state"
                )
            }
            Self::TemporaryLeftover { path } => {
                write!(f, "temporary entry {} survived recovery", path.display())
            }
            Self::NotIdempotent { changed } => {
                write!(f, "second recovery changed {} paths", changed.len())
            }
            Self::SilentError => {
                f.write_str("injected error ended without rollback or irreversible marker")
            }
            Self::DuplicateOperation { op } => {
                write!(f, "operation {op} appears twice in the history")
            }
            Self::Unreadable { detail } => write!(f, "tree cannot be read: {detail}"),
        }
    }
}

impl std::error::Error for Violation {}

/// Checks that the previous content of every target exists somewhere in
/// `volume`, a snapshot of the whole volume including the journal.
///
/// Targets are relative paths in `before`; a directory target covers every
/// file and link below it. Files are matched by content digest, links by
/// target.
///
/// # Errors
///
/// Fails with [`Violation::PreviousContentLost`] naming the first target
/// whose content is missing.
pub fn check_preserved(
    before: &TreeSnapshot,
    targets: &[PathBuf],
    volume: &TreeSnapshot,
) -> Result<(), Violation> {
    let hashes = volume.hashes();
    let links: HashSet<&PathBuf> = volume
        .entries
        .values()
        .filter_map(|entry| match entry {
            EntrySnapshot::Link { target } => Some(target),
            _ => None,
        })
        .collect();
    for (path, entry) in &before.entries {
        if !targets.iter().any(|target| path.starts_with(target)) {
            continue;
        }
        let kept = match entry {
            EntrySnapshot::File { hash, .. } => hashes.contains(hash),
            EntrySnapshot::Link { target } => links.contains(target),
            EntrySnapshot::Directory { .. } => true,
        };
        if !kept {
            return Err(Violation::PreviousContentLost { path: path.clone() });
        }
    }
    Ok(())
}

/// Checks that `tree` equals `before` or `after` as `expected` requires.
///
/// # Errors
///
/// Fails with [`Violation::IntermediateState`] when `tree` equals neither
/// state, and with [`Violation::WrongState`] when it equals the state not
/// expected.
pub fn check_complete(
    tree: &TreeSnapshot,
    before: &TreeSnapshot,
    after: &TreeSnapshot,
    expected: Expected,
) -> Result<Matched, Violation> {
    let from_before = tree.differences(before);
    let from_after = tree.differences(after);
    let found = if from_before.is_empty() {
        Matched::Before
    } else if from_after.is_empty() {
        Matched::After
    } else {
        return Err(Violation::IntermediateState {
            from_before,
            from_after,
        });
    };
    let accepted = match expected {
        Expected::Either => true,
        Expected::Before => from_before.is_empty(),
        Expected::After => from_after.is_empty(),
    };
    if accepted {
        Ok(found)
    } else {
        Err(Violation::WrongState { expected, found })
    }
}

/// Checks that no entry name starts with [`TEMP_PREFIX`].
///
/// # Errors
///
/// Fails with [`Violation::TemporaryLeftover`] naming the first such entry.
pub fn check_no_leftovers(volume: &TreeSnapshot) -> Result<(), Violation> {
    let leftover = volume.entries.keys().find(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(TEMP_PREFIX))
    });
    match leftover {
        Some(path) => Err(Violation::TemporaryLeftover { path: path.clone() }),
        None => Ok(()),
    }
}

/// Checks that a second recovery left the volume unchanged.
///
/// # Errors
///
/// Fails with [`Violation::NotIdempotent`] listing the changed paths.
pub fn check_idempotent(first: &TreeSnapshot, second: &TreeSnapshot) -> Result<(), Violation> {
    let changed = first.differences(second);
    if changed.is_empty() {
        Ok(())
    } else {
        Err(Violation::NotIdempotent { changed })
    }
}

/// Checks that an operation did not end silently: it committed, rolled
/// back or crashed.
///
/// # Errors
///
/// Fails with [`Violation::SilentError`] for [`OpOutcome::FailedSilently`].
pub fn check_explicit(outcome: OpOutcome) -> Result<(), Violation> {
    match outcome {
        OpOutcome::FailedSilently => Err(Violation::SilentError),
        _ => Ok(()),
    }
}

/// Checks that no operation number appears twice in the history.
///
/// # Errors
///
/// Fails with [`Violation::DuplicateOperation`] naming the first repeated
/// number.
pub fn check_history(records: &[OperationRecord]) -> Result<(), Violation> {
    let mut seen: HashSet<OpId> = HashSet::new();
    for record in records {
        if !seen.insert(record.id) {
            return Err(Violation::DuplicateOperation { op: record.id.0 });
        }
    }
    Ok(())
}

impl From<nurt_contracts::fs::FsError> for Violation {
    fn from(err: nurt_contracts::fs::FsError) -> Self {
        Self::Unreadable {
            detail: err.to_string(),
        }
    }
}
