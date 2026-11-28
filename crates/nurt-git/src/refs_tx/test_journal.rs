// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::refs_tx::test_journal
// Responsibility:   Recording mutation transaction on the real file system for unit tests.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Recording mutation transaction on the real file system for unit tests.
//!
//! [`TestJournal`] wraps the journal of [`crate::apply::testjournal`], which
//! applies the rules of the reference journal, and records the targets a
//! transaction declared. [`tree_state`] and [`assert_only_declared_changes`]
//! check that a command changed nothing outside its declared targets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, WriteHandle};
use nurt_contracts::journal::{
    Applied, ExemptWrite, ExternalChange, Mutation, MutationTx, OpHandle, OpId, Preserved,
    RollbackReason, RollbackReport, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

use crate::apply::testjournal;

/// Journal handing out recording transactions and undoing committed ones.
#[derive(Debug, Default)]
pub(crate) struct TestJournal {
    inner: testjournal::RecordingJournal,
}

impl TestJournal {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn begin(&self) -> Box<RecordingTx<'_>> {
        Box::new(RecordingTx {
            inner: self.inner.begin(),
            targets: Vec::new(),
        })
    }

    /// Number of committed operations that changed something and were not
    /// undone.
    pub(crate) fn committed(&self) -> usize {
        self.inner.len()
    }

    /// Restores the state before the newest committed operation.
    pub(crate) fn undo_last(&self) {
        self.inner.undo_last();
    }
}

/// Recording transaction.
pub(crate) struct RecordingTx<'j> {
    inner: testjournal::RecordingTx<'j>,
    targets: Vec<TargetSpec>,
}

impl RecordingTx<'_> {
    /// Declared targets, in order.
    pub(crate) fn declared(&self) -> &[TargetSpec] {
        &self.targets
    }
}

impl MutationTx for RecordingTx<'_> {
    fn op(&self) -> Option<OpId> {
        self.inner.op()
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        let preserved = self.inner.preserve(targets)?;
        self.targets.extend_from_slice(targets);
        Ok(preserved)
    }

    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        self.inner.stage(target)
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied> {
        self.inner.apply(target, mutation)
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        self.inner.observe_external(changes)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.inner.mark_irreversible(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.inner.record_exempt(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        Box::new(self.inner).commit()
    }

    fn rollback(self: Box<Self>, reason: RollbackReason) -> Result<RollbackReport> {
        Box::new(self.inner).rollback(reason)
    }
}

fn fs_path(spec: &TargetSpec) -> PathBuf {
    match &spec.path {
        TargetPath::Fs(path) => path.as_path().to_path_buf(),
        TargetPath::Provider(_) => panic!("provider targets are not used by git commands"),
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
        allowed.push(fs_path(spec));
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
