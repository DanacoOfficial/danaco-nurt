// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Recording mutation transaction on the real file system for unit tests.
//!
//! The transaction validates declarations as the journal does, keeps a
//! snapshot of every declared path, applies mutations directly and restores
//! the snapshots on rollback or undo. [`tree_state`] and
//! [`assert_only_declared_changes`] check that a command changed nothing
//! outside its declared targets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, WriteHandle};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, Expect, ExternalChange, Mutation, MutationTx,
    OpHandle, OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;
use sha2::Digest;

/// Content of a path at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Snapshot {
    Absent,
    File(Vec<u8>),
    Dir(BTreeMap<PathBuf, Snapshot>),
}

impl Snapshot {
    pub(crate) fn take(path: &Path) -> Self {
        match std::fs::symlink_metadata(path) {
            Err(_) => Self::Absent,
            Ok(meta) if meta.is_dir() => {
                let mut entries = BTreeMap::new();
                for entry in std::fs::read_dir(path).unwrap() {
                    let entry = entry.unwrap();
                    entries.insert(PathBuf::from(entry.file_name()), Self::take(&entry.path()));
                }
                Self::Dir(entries)
            }
            Ok(_) => Self::File(std::fs::read(path).unwrap()),
        }
    }

    fn restore(&self, path: &Path) {
        remove(path);
        match self {
            Self::Absent => {}
            Self::File(bytes) => std::fs::write(path, bytes).unwrap(),
            Self::Dir(entries) => {
                std::fs::create_dir(path).unwrap();
                for (name, child) in entries {
                    child.restore(&path.join(name));
                }
            }
        }
    }
}

fn remove(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path).unwrap(),
        Ok(_) => std::fs::remove_file(path).unwrap(),
        Err(_) => {}
    }
}

fn hash(bytes: &[u8]) -> ContentHash {
    ContentHash(sha2::Sha256::digest(bytes).into())
}

fn error(code: ErrorCode, path: &Path) -> Error {
    Error::new(code, format!("{code} at {}", path.display())).with_detail(
        "path",
        serde_json::Value::String(path.display().to_string()),
    )
}

#[derive(Debug, Default)]
struct Shared {
    committed: Vec<Vec<(PathBuf, Snapshot)>>,
    next_op: u64,
}

/// Journal handing out recording transactions and undoing committed ones.
#[derive(Debug, Clone, Default)]
pub(crate) struct TestJournal {
    shared: Arc<Mutex<Shared>>,
}

impl TestJournal {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn begin(&self) -> Box<RecordingTx> {
        Box::new(RecordingTx {
            shared: Arc::clone(&self.shared),
            targets: Vec::new(),
            snapshots: Vec::new(),
            changed: false,
            failed: false,
            exempt: Vec::new(),
        })
    }

    /// Number of committed operations not undone.
    pub(crate) fn committed(&self) -> usize {
        self.shared.lock().unwrap().committed.len()
    }

    /// Restores the state before the newest committed operation.
    pub(crate) fn undo_last(&self) {
        let snapshots = self.shared.lock().unwrap().committed.pop().unwrap();
        for (path, snapshot) in snapshots.iter().rev() {
            snapshot.restore(path);
        }
    }
}

/// Recording transaction.
#[derive(Debug)]
pub(crate) struct RecordingTx {
    shared: Arc<Mutex<Shared>>,
    targets: Vec<TargetSpec>,
    snapshots: Vec<(PathBuf, Snapshot)>,
    changed: bool,
    failed: bool,
    exempt: Vec<ExemptWrite>,
}

impl RecordingTx {
    /// Declared targets, in order.
    pub(crate) fn declared(&self) -> &[TargetSpec] {
        &self.targets
    }

    fn fs_path(spec: &TargetSpec) -> PathBuf {
        match &spec.path {
            TargetPath::Fs(path) => path.as_path().to_path_buf(),
            TargetPath::Provider(_) => panic!("provider targets are not used by git commands"),
        }
    }

    fn validate(spec: &TargetSpec) -> Result<()> {
        let path = Self::fs_path(spec);
        let exists = std::fs::symlink_metadata(&path).is_ok();
        match spec.action {
            TargetAction::Create if exists => Err(error(ErrorCode::Exists, &path)),
            TargetAction::Delete | TargetAction::Append if !exists => {
                Err(error(ErrorCode::NotFound, &path))
            }
            _ => Ok(()),
        }
    }

    fn check_expect(spec: &TargetSpec) -> Result<()> {
        let path = Self::fs_path(spec);
        let ok = match &spec.expect {
            Expect::Unchecked => true,
            Expect::Absent => std::fs::symlink_metadata(&path).is_err(),
            Expect::Hash(want) => std::fs::read(&path).is_ok_and(|bytes| hash(&bytes) == *want),
            Expect::Etag(_) => panic!("entity tags are not used by git commands"),
        };
        if ok {
            Ok(())
        } else {
            Err(error(ErrorCode::Stale, &path))
        }
    }

    fn fail(&mut self, error: Error) -> Error {
        self.failed = true;
        self.restore();
        error
    }

    fn restore(&mut self) {
        for (path, snapshot) in self.snapshots.iter().rev() {
            snapshot.restore(path);
        }
    }

    fn mutate(spec: &TargetSpec, mutation: Mutation) -> Result<()> {
        let path = Self::fs_path(spec);
        let io = |_: std::io::Error| error(ErrorCode::Io, &path);
        match (&spec.action, mutation) {
            (TargetAction::Create | TargetAction::Modify, Mutation::WriteBytes(bytes)) => {
                std::fs::write(&path, bytes).map_err(io)
            }
            (TargetAction::Append, Mutation::AppendBytes(bytes)) => {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .map_err(io)?;
                file.write_all(&bytes).map_err(io)
            }
            (TargetAction::Create, Mutation::CreateDir { .. }) => {
                std::fs::create_dir(&path).map_err(io)
            }
            (TargetAction::Delete, Mutation::Remove) => {
                remove(&path);
                Ok(())
            }
            (TargetAction::Rename { to, replace }, Mutation::Rename) => {
                if !replace && std::fs::symlink_metadata(to.as_path()).is_ok() {
                    return Err(error(ErrorCode::Exists, to.as_path()));
                }
                if std::fs::symlink_metadata(&path).is_err() {
                    return Err(error(ErrorCode::NotFound, &path));
                }
                std::fs::rename(&path, to.as_path()).map_err(io)
            }
            (action, mutation) => panic!("mutation {mutation:?} does not match action {action:?}"),
        }
    }
}

impl MutationTx for RecordingTx {
    fn op(&self) -> Option<OpId> {
        None
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        assert!(!self.changed, "preserve after the first change");
        assert!(!self.failed, "call on a rolled back transaction");
        let mut out = Vec::new();
        for spec in targets {
            if let Err(error) = Self::validate(spec) {
                return Err(self.fail(error));
            }
            let path = Self::fs_path(spec);
            self.snapshots.push((path.clone(), Snapshot::take(&path)));
            if let TargetAction::Rename { to, .. } = &spec.action {
                let to = to.as_path().to_path_buf();
                self.snapshots.push((to.clone(), Snapshot::take(&to)));
            }
            let index = TargetIndex(u32::try_from(self.targets.len()).unwrap());
            self.targets.push(spec.clone());
            out.push(Preserved {
                index,
                volume: None,
                mode: PreserveMode::Copy,
                bytes_held: 0,
                before: BeforeState::default(),
                irreversible: None,
                notable: Vec::new(),
            });
        }
        Ok(out)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        panic!("staged writes are not used by git commands")
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        assert!(!self.failed, "call on a rolled back transaction");
        let spec = self
            .targets
            .get(target.0 as usize)
            .cloned()
            .unwrap_or_else(|| panic!("undeclared target {target:?}"));
        self.changed = true;
        let result = Self::check_expect(&spec).and_then(|()| Self::mutate(&spec, mutation));
        match result {
            Ok(()) => Ok(Applied {
                index: target,
                after: AfterState::default(),
            }),
            Err(error) => Err(self.fail(error)),
        }
    }

    fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        panic!("external writes are not used by git commands")
    }

    fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {
        panic!("git commands of this crate have no irreversible effects")
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.exempt.push(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        assert!(!self.failed, "commit of a rolled back transaction");
        let mut shared = self.shared.lock().unwrap();
        shared.next_op += 1;
        let op = OpId(shared.next_op);
        if self.changed {
            shared.committed.push(self.snapshots.clone());
        }
        Ok(OpHandle {
            op: self.changed.then_some(op),
            kind: OpKind::command("test"),
            bulk: self.targets.len() > 1,
            targets: Vec::new(),
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: self.exempt,
        })
    }

    fn rollback(mut self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        if !self.failed {
            self.restore();
        }
        Ok(RollbackReport {
            op: None,
            restored: Vec::new(),
            failed: Vec::new(),
        })
    }
}

/// Every file and directory below `root`, with file contents.
pub(crate) fn tree_state(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    collect(root, &mut out);
    out
}

fn collect(dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.insert(path.clone(), None);
            collect(&path, out);
        } else {
            out.insert(path.clone(), Some(std::fs::read(&path).unwrap()));
        }
    }
}

/// Panics when a path changed between `before` and `after` that is neither
/// a declared target nor below a declared directory target, ignoring
/// `exempt` subtrees (the object store).
pub(crate) fn assert_only_declared_changes(
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    after: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    declared: &[TargetSpec],
    exempt: &[&Path],
) {
    let mut allowed: Vec<PathBuf> = Vec::new();
    for spec in declared {
        allowed.push(RecordingTx::fs_path(spec));
        if let TargetAction::Rename { to, .. } = &spec.action {
            allowed.push(to.as_path().to_path_buf());
        }
    }
    let keys: std::collections::BTreeSet<&PathBuf> = before.keys().chain(after.keys()).collect();
    for path in keys {
        if before.get(path) == after.get(path) || exempt.iter().any(|e| path.starts_with(e)) {
            continue;
        }
        let covered = allowed.iter().any(|target| path.starts_with(target));
        assert!(
            covered,
            "{} changed outside the transaction",
            path.display()
        );
    }
}

/// Absolute path for tests.
pub(crate) fn abs(path: &Path) -> AbsPath {
    AbsPath::new(path).unwrap()
}
