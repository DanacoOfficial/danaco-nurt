// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal test double that applies mutations to the real file system.
//!
//! [`RecordingJournal`] follows the protocol of the journal contract and
//! checks its use strictly: preserving after a change, applying to an
//! undeclared target and a mutation that does not fit the declared action
//! panic. The previous state of every declared target is kept in memory, so
//! [`RecordingJournal::undo_last`] restores it byte for byte. Together with
//! [`tree_state`] and [`assert_declared`] a test proves that every change of
//! a directory went through a transaction.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::WriteHandle;
use nurt_contracts::journal::Expect;
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle,
    OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

use super::worktree::{content_hash, expect_of, observe_native};

/// Recorded state of a file system entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    /// No entry.
    Missing,
    /// Regular file with its content and permission bits.
    File {
        /// Content.
        bytes: Vec<u8>,
        /// Permission bits; zero where the system has none.
        mode: u32,
    },
    /// Symbolic link with its target.
    Symlink(PathBuf),
    /// Directory with its entries.
    Dir(BTreeMap<OsString, Snapshot>),
}

impl Snapshot {
    /// Records the entry at `path` recursively, without following links.
    ///
    /// # Panics
    ///
    /// When the entry cannot be read.
    pub fn take(path: &Path) -> Self {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return Self::Missing;
        };
        if meta.file_type().is_symlink() {
            return Self::Symlink(std::fs::read_link(path).expect("read link"));
        }
        if meta.is_dir() {
            let mut children = BTreeMap::new();
            for item in std::fs::read_dir(path).expect("read directory") {
                let item = item.expect("directory entry");
                children.insert(item.file_name(), Self::take(&item.path()));
            }
            return Self::Dir(children);
        }
        Self::File {
            bytes: std::fs::read(path).expect("read file"),
            mode: mode_of(&meta),
        }
    }

    /// Replaces whatever is at `path` by this state.
    ///
    /// # Panics
    ///
    /// When the file system refuses a change.
    pub fn restore(&self, path: &Path) {
        remove_any(path);
        match self {
            Self::Missing => {}
            Self::File { bytes, mode } => {
                std::fs::write(path, bytes).expect("restore file");
                set_mode(path, *mode);
            }
            Self::Symlink(target) => make_symlink(target, path),
            Self::Dir(children) => {
                std::fs::create_dir(path).expect("restore directory");
                for (name, child) in children {
                    child.restore(&path.join(name));
                }
            }
        }
    }
}

#[cfg(unix)]
fn mode_of(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(_meta: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("set mode");
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

#[cfg(unix)]
fn make_symlink(target: &Path, path: &Path) {
    std::os::unix::fs::symlink(target, path).expect("create link");
}

#[cfg(windows)]
fn make_symlink(target: &Path, path: &Path) {
    std::os::windows::fs::symlink_file(target, path).expect("create link");
}

fn remove_any(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path).expect("remove directory"),
        Ok(_) => std::fs::remove_file(path).expect("remove file"),
        Err(_) => {}
    }
}

/// State of every file, link and empty directory below `root`, keyed by
/// relative path; the object database of a repository is skipped because
/// its additions are exempt from the journal.
pub fn tree_state(root: &Path) -> BTreeMap<PathBuf, Snapshot> {
    let mut out = BTreeMap::new();
    collect(root, Path::new(""), &mut out);
    out
}

fn collect(root: &Path, rela: &Path, out: &mut BTreeMap<PathBuf, Snapshot>) {
    let path = root.join(rela);
    if rela == Path::new(".git/objects") {
        return;
    }
    match Snapshot::take(&path) {
        Snapshot::Dir(children) if !children.is_empty() => {
            for name in children.keys() {
                collect(root, &rela.join(name), out);
            }
        }
        state => {
            out.insert(rela.to_path_buf(), state);
        }
    }
}

/// Asserts that every path that differs between `before` and `after` lies
/// at or below a target declared in the last committed entry of `journal`.
///
/// # Panics
///
/// When a change was made outside the transaction.
pub fn assert_declared(
    root: &Path,
    before: &BTreeMap<PathBuf, Snapshot>,
    after: &BTreeMap<PathBuf, Snapshot>,
    journal: &RecordingJournal,
) {
    let declared = journal.last_declared();
    let changed = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path));
    for path in changed {
        let absolute = root.join(path);
        let covered = declared
            .iter()
            .any(|target| absolute.starts_with(target) || target.starts_with(&absolute));
        assert!(
            covered,
            "{} changed outside the transaction",
            path.display()
        );
    }
}

/// Journal test double; see the module documentation.
#[derive(Debug, Default)]
pub struct RecordingJournal {
    entries: Mutex<Vec<Vec<(PathBuf, Snapshot)>>>,
}

impl RecordingJournal {
    /// Empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a transaction.
    pub fn begin(&self) -> RecordingTx<'_> {
        RecordingTx {
            journal: self,
            declared: Vec::new(),
            changed: false,
            failed: false,
            exempt: Vec::new(),
        }
    }

    /// Number of committed entries that changed something.
    ///
    /// # Panics
    ///
    /// When the journal lock is poisoned.
    pub fn len(&self) -> usize {
        self.entries.lock().expect("journal lock").len()
    }

    /// Whether no entry was committed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Targets declared by the last committed entry.
    ///
    /// # Panics
    ///
    /// When the journal lock is poisoned.
    pub fn last_declared(&self) -> Vec<PathBuf> {
        self.entries
            .lock()
            .expect("journal lock")
            .last()
            .map(|entry| entry.iter().map(|(path, _)| path.clone()).collect())
            .unwrap_or_default()
    }

    /// Restores every target of the last committed entry, in reverse order,
    /// and drops the entry.
    ///
    /// # Panics
    ///
    /// When no entry exists or a target cannot be restored.
    pub fn undo_last(&self) {
        let entry = self
            .entries
            .lock()
            .expect("journal lock")
            .pop()
            .expect("an entry to undo");
        for (path, before) in entry.iter().rev() {
            before.restore(path);
        }
    }
}

struct Declared {
    spec: TargetSpec,
    path: PathBuf,
    saved: Vec<(PathBuf, Snapshot)>,
    checked: bool,
    mutations: usize,
}

/// Transaction of a [`RecordingJournal`].
pub struct RecordingTx<'j> {
    journal: &'j RecordingJournal,
    declared: Vec<Declared>,
    changed: bool,
    failed: bool,
    exempt: Vec<ExemptWrite>,
}

fn io(err: &std::io::Error, path: &Path) -> Error {
    Error::new(ErrorCode::Io, format!("{}: {err}", path.display()))
}

impl RecordingTx<'_> {
    fn validate(spec: &TargetSpec, path: &Path) -> Result<()> {
        let exists = std::fs::symlink_metadata(path).is_ok();
        match &spec.action {
            TargetAction::Create | TargetAction::Link if exists => Err(Error::new(
                ErrorCode::Exists,
                format!("{} exists", path.display()),
            )),
            TargetAction::Delete
            | TargetAction::Replace
            | TargetAction::Attributes
            | TargetAction::Append
                if !exists =>
            {
                Err(Error::new(
                    ErrorCode::NotFound,
                    format!("{} does not exist", path.display()),
                ))
            }
            _ => Ok(()),
        }
    }

    fn check_expect(path: &Path, expect: &Expect) -> Result<()> {
        let observation = observe_native(path)?;
        let matches = match expect {
            Expect::Unchecked => true,
            Expect::Absent => !observation.node.exists(),
            Expect::Etag(_) => expect_of(path, &observation)? == *expect,
            Expect::Hash(hash) => {
                std::fs::read(path).is_ok_and(|bytes| content_hash(&bytes) == *hash)
            }
        };
        if matches {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::Stale,
                format!("{} changed", path.display()),
            ))
        }
    }

    fn fits(action: &TargetAction, mutation: &Mutation, count: usize) -> bool {
        let creating = matches!(
            mutation,
            Mutation::WriteBytes(_) | Mutation::CreateDir { .. } | Mutation::CreateLink { .. }
        );
        match action {
            TargetAction::Create => {
                matches!(
                    mutation,
                    Mutation::WriteBytes(_) | Mutation::CreateDir { .. }
                ) || matches!(mutation, Mutation::SetMeta(_)) && count > 0
            }
            TargetAction::Link => matches!(mutation, Mutation::CreateLink { .. }),
            TargetAction::Modify => {
                matches!(mutation, Mutation::WriteBytes(_) | Mutation::SetMeta(_))
            }
            TargetAction::Delete => matches!(mutation, Mutation::Remove),
            TargetAction::Replace => {
                if count == 0 {
                    matches!(mutation, Mutation::Remove)
                } else {
                    creating || matches!(mutation, Mutation::SetMeta(_))
                }
            }
            TargetAction::Attributes => matches!(mutation, Mutation::SetMeta(_)),
            TargetAction::Append => matches!(mutation, Mutation::AppendBytes(_)),
            TargetAction::Rename { .. } => matches!(mutation, Mutation::Rename),
            _ => false,
        }
    }

    fn perform(path: &Path, action: &TargetAction, mutation: Mutation) -> Result<()> {
        match mutation {
            Mutation::WriteBytes(bytes) => {
                let staged = path.with_extension("nurt-staged");
                std::fs::write(&staged, bytes).map_err(|err| io(&err, &staged))?;
                if let Ok(meta) = std::fs::symlink_metadata(path)
                    && meta.is_file()
                {
                    set_mode(&staged, mode_of(&meta));
                }
                std::fs::rename(&staged, path).map_err(|err| io(&err, path))
            }
            Mutation::AppendBytes(bytes) => std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(&bytes))
                .map_err(|err| io(&err, path)),
            Mutation::Remove => {
                remove_any(path);
                Ok(())
            }
            Mutation::CreateDir { .. } => std::fs::create_dir(path).map_err(|err| io(&err, path)),
            Mutation::CreateLink { target, .. } => {
                make_symlink(&target, path);
                Ok(())
            }
            Mutation::SetMeta(change) => {
                if let Some(mode) = change.mode {
                    set_mode(path, mode);
                }
                Ok(())
            }
            Mutation::Rename => match action {
                TargetAction::Rename { to, .. } => {
                    std::fs::rename(path, to.as_path()).map_err(|err| io(&err, path))
                }
                _ => Err(Error::new(
                    ErrorCode::StageFailed,
                    "rename without a rename target",
                )),
            },
            other => Err(Error::new(
                ErrorCode::StageFailed,
                format!("mutation {other:?} is not supported by the test journal"),
            )),
        }
    }
}

impl MutationTx for RecordingTx<'_> {
    fn op(&self) -> Option<OpId> {
        (!self.declared.is_empty()).then_some(OpId(1))
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        assert!(!self.changed, "preserve after the first change");
        let mut out = Vec::with_capacity(targets.len());
        for spec in targets {
            let TargetPath::Fs(path) = &spec.path else {
                panic!("provider targets are not supported by the test journal");
            };
            let path = path.as_path().to_path_buf();
            Self::validate(spec, &path)?;
            let mut saved = vec![(path.clone(), Snapshot::take(&path))];
            if let TargetAction::Rename { to, .. } = &spec.action {
                saved.push((to.as_path().to_path_buf(), Snapshot::take(to.as_path())));
            }
            let exists = !matches!(saved[0].1, Snapshot::Missing);
            let index = TargetIndex(u32::try_from(self.declared.len()).expect("target count"));
            out.push(Preserved {
                index,
                volume: None,
                mode: if exists {
                    PreserveMode::Copy
                } else {
                    PreserveMode::Created
                },
                bytes_held: 0,
                before: BeforeState {
                    exists,
                    ..BeforeState::default()
                },
                irreversible: None,
                notable: Vec::new(),
            });
            self.declared.push(Declared {
                spec: spec.clone(),
                path,
                saved,
                checked: false,
                mutations: 0,
            });
        }
        Ok(out)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(Error::new(
            ErrorCode::Io,
            "staged writes are not supported by the test journal",
        ))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        if self.failed {
            return Err(Error::new(
                ErrorCode::RolledBack,
                "the transaction was rolled back",
            ));
        }
        let position = usize::try_from(target.0).expect("target index");
        let declared = self
            .declared
            .get_mut(position)
            .unwrap_or_else(|| panic!("target {position} was not declared"));
        assert!(
            Self::fits(&declared.spec.action, &mutation, declared.mutations),
            "mutation {mutation:?} does not fit the action {:?} of {}",
            declared.spec.action,
            declared.path.display()
        );
        self.changed = true;
        let result = if declared.checked {
            Ok(())
        } else {
            declared.checked = true;
            Self::check_expect(&declared.path, &declared.spec.expect)
        }
        .and_then(|()| Self::perform(&declared.path, &declared.spec.action, mutation));
        declared.mutations += 1;
        if let Err(err) = result {
            self.failed = true;
            return Err(err);
        }
        Ok(Applied {
            index: target,
            after: AfterState {
                exists: std::fs::symlink_metadata(&declared.path).is_ok(),
                ..AfterState::default()
            },
        })
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        Ok(Vec::new())
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {}

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.exempt.push(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        assert!(!self.failed, "commit after a failed mutation");
        let changed = self.changed;
        if changed {
            let saved = self
                .declared
                .into_iter()
                .flat_map(|declared| declared.saved)
                .collect();
            self.journal
                .entries
                .lock()
                .expect("journal lock")
                .push(saved);
        }
        Ok(OpHandle {
            op: changed.then_some(OpId(1)),
            kind: OpKind::command("test"),
            bulk: true,
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: self.exempt,
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        let mut restored = Vec::new();
        for declared in self.declared.iter().rev() {
            if declared.mutations > 0 {
                for (path, before) in declared.saved.iter().rev() {
                    before.restore(path);
                }
                restored.push(declared.spec.path.clone());
            }
        }
        Ok(RollbackReport {
            op: None,
            restored,
            failed: Vec::new(),
        })
    }
}
