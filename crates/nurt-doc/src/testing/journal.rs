// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{FileKind, WriteHandle};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, Expect, ExternalChange, MutationTx, OpHandle,
    OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetOutcome, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;

/// Operation committed to a [`MemoryJournal`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    op: OpId,
    kind: OpKind,
    before: Vec<(PathBuf, Option<Vec<u8>>)>,
}

#[derive(Debug, Default)]
struct State {
    files: BTreeMap<PathBuf, Vec<u8>>,
    entries: Vec<Entry>,
    next_op: u64,
}

/// In-memory files and journal for tests of commands that change
/// documents.
///
/// Transactions from [`MemoryJournal::begin`] implement
/// [`MutationTx`]: declared targets are preserved, `WriteBytes`,
/// `AppendBytes` and `Remove` change the in-memory files, commit records an
/// entry, and [`MemoryJournal::undo_last`] restores the previous content of
/// every target byte for byte.
///
/// ```
/// use nurt_contracts::fs::AbsPath;
/// use nurt_contracts::journal::{Expect, Mutation, OpKind, TargetAction, TargetIndex, TargetPath, TargetSpec};
/// use nurt_doc::testing::MemoryJournal;
///
/// let path = AbsPath::new("/srv/app/config.json")?;
/// let journal = MemoryJournal::new().with_file(path.as_path(), b"{}".to_vec());
/// let mut tx = journal.begin(OpKind::command("set"));
/// tx.preserve(&[TargetSpec {
///     path: TargetPath::Fs(path.clone()),
///     action: TargetAction::Modify,
///     expect: Expect::Unchecked,
///     size_hint: None,
/// }])?;
/// tx.apply(TargetIndex(0), Mutation::WriteBytes(b"{\"a\": 1}".to_vec()))?;
/// tx.commit()?;
/// assert_eq!(journal.file(path.as_path()).as_deref(), Some(&b"{\"a\": 1}"[..]));
/// journal.undo_last()?;
/// assert_eq!(journal.file(path.as_path()).as_deref(), Some(&b"{}"[..]));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct MemoryJournal {
    state: Arc<Mutex<State>>,
}

impl MemoryJournal {
    /// Journal without files.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The journal with a file added.
    #[must_use]
    pub fn with_file(self, path: &Path, content: Vec<u8>) -> Self {
        self.lock().files.insert(path.to_owned(), content);
        self
    }

    /// Content of a file.
    #[must_use]
    pub fn file(&self, path: &Path) -> Option<Vec<u8>> {
        self.lock().files.get(path).cloned()
    }

    /// Number of committed operations that changed something.
    #[must_use]
    pub fn operations(&self) -> usize {
        self.lock().entries.len()
    }

    /// Kind of the last committed operation.
    #[must_use]
    pub fn last_kind(&self) -> Option<OpKind> {
        self.lock().entries.last().map(|entry| entry.kind.clone())
    }

    /// Starts a transaction.
    #[must_use]
    pub fn begin(&self, kind: OpKind) -> Box<dyn MutationTx> {
        Box::new(MemoryTx {
            state: Arc::clone(&self.state),
            kind,
            op: None,
            targets: Vec::new(),
            before: Vec::new(),
            changed: Vec::new(),
            irreversible: Vec::new(),
            exempt: Vec::new(),
        })
    }

    /// Restores the targets of the last committed operation.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` when no operation is left to undo.
    pub fn undo_last(&self) -> Result<OpId> {
        let mut state = self.lock();
        let entry = state
            .entries
            .pop()
            .ok_or_else(|| Error::new(ErrorCode::JournalNoop, "no operation is left to undo"))?;
        for (path, content) in entry.before.into_iter().rev() {
            match content {
                Some(content) => state.files.insert(path, content),
                None => state.files.remove(&path),
            };
        }
        Ok(entry.op)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Transaction of a [`MemoryJournal`].
#[derive(Debug)]
pub struct MemoryTx {
    state: Arc<Mutex<State>>,
    kind: OpKind,
    op: Option<OpId>,
    targets: Vec<(PathBuf, TargetSpec)>,
    before: Vec<Option<Vec<u8>>>,
    changed: Vec<bool>,
    irreversible: Vec<IrreversibleEntry>,
    exempt: Vec<ExemptWrite>,
}

fn misuse(message: &str) -> Error {
    Error::new(ErrorCode::StageFailed, message.to_owned())
}

fn before_state(content: Option<&Vec<u8>>) -> BeforeState {
    BeforeState {
        exists: content.is_some(),
        kind: content.map(|_| FileKind::File),
        size: content.map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
        hash: None,
        mode: None,
    }
}

fn after_state(content: Option<&Vec<u8>>) -> AfterState {
    AfterState {
        exists: content.is_some(),
        kind: content.map(|_| FileKind::File),
        size: content.map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
        hash: None,
        mode: None,
    }
}

impl MemoryTx {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn index(&self, target: TargetIndex) -> Result<usize> {
        let index = usize::try_from(target.0).unwrap_or(usize::MAX);
        if index < self.targets.len() {
            Ok(index)
        } else {
            Err(misuse("the target was not declared"))
        }
    }

    fn check(spec: &TargetSpec, existing: Option<&Vec<u8>>, path: &Path) -> Result<()> {
        let shown = path.display();
        match (&spec.action, &spec.expect, existing) {
            (TargetAction::Create, _, Some(_)) | (_, Expect::Absent, Some(_)) => Err(Error::new(
                ErrorCode::Exists,
                format!("{shown} exists"),
            )),
            (TargetAction::Modify | TargetAction::Create, _, _) | (_, _, Some(_)) => Ok(()),
            (_, _, None) => Err(Error::new(ErrorCode::NotFound, format!("{shown} does not exist"))),
        }
    }
}

impl MutationTx for MemoryTx {
    fn op(&self) -> Option<OpId> {
        self.op
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
        if self.changed.iter().any(|changed| *changed) {
            return Err(misuse("targets are declared after the first change"));
        }
        let mut preserved = Vec::with_capacity(targets.len());
        for spec in targets {
            let TargetPath::Fs(path) = &spec.path else {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "the memory journal holds files only; declare file targets",
                ));
            };
            let path = path.as_path().to_owned();
            let existing = self.lock().files.get(&path).cloned();
            Self::check(spec, existing.as_ref(), &path)?;
            let index = TargetIndex(u32::try_from(self.targets.len()).unwrap_or(u32::MAX));
            preserved.push(Preserved {
                index,
                volume: None,
                mode: if existing.is_some() { PreserveMode::Copy } else { PreserveMode::Created },
                bytes_held: existing
                    .as_ref()
                    .map_or(0, |bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
                before: before_state(existing.as_ref()),
                irreversible: None,
                notable: Vec::new(),
            });
            self.targets.push((path, spec.clone()));
            self.before.push(existing);
            self.changed.push(false);
        }
        if self.op.is_none() {
            let mut state = self.lock();
            state.next_op += 1;
            let op = OpId(state.next_op);
            drop(state);
            self.op = Some(op);
        }
        Ok(preserved)
    }

    fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
        Err(Error::new(
            ErrorCode::Unsupported,
            "the memory journal has no staged files; apply `WriteBytes` instead",
        ))
    }

    fn apply(&mut self, target: TargetIndex, mutation: nurt_contracts::journal::Mutation) -> Result<Applied> {
        use nurt_contracts::journal::Mutation;
        let index = self.index(target)?;
        let path = self.targets[index].0.clone();
        let mut state = self.lock();
        match mutation {
            Mutation::WriteBytes(bytes) => {
                state.files.insert(path.clone(), bytes);
            }
            Mutation::AppendBytes(bytes) => state.files.entry(path.clone()).or_default().extend(bytes),
            Mutation::Remove => {
                state.files.remove(&path);
            }
            _ => {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "the memory journal applies WriteBytes, AppendBytes and Remove only",
                ));
            }
        }
        let after = after_state(state.files.get(&path));
        drop(state);
        self.changed[index] = true;
        Ok(Applied { index: target, after })
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>> {
        let mut applied = Vec::new();
        for change in changes {
            if let Some(index) = change.declared {
                let position = self.index(index)?;
                self.changed[position] = true;
                let after = after_state(self.lock().files.get(change.path.as_path()));
                applied.push(Applied { index, after });
            }
        }
        Ok(applied)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        self.exempt.push(write);
    }

    fn commit(self: Box<Self>) -> Result<OpHandle> {
        let this = *self;
        let mut state = this.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut outcomes = Vec::new();
        let mut before = Vec::new();
        for (position, ((path, spec), previous)) in this.targets.iter().zip(&this.before).enumerate() {
            if !this.changed[position] {
                continue;
            }
            before.push((path.clone(), previous.clone()));
            outcomes.push(TargetOutcome {
                index: TargetIndex(u32::try_from(position).unwrap_or(u32::MAX)),
                path: spec.path.clone(),
                action: spec.action.clone(),
                mode: if previous.is_some() { PreserveMode::Copy } else { PreserveMode::Created },
                bytes_held: previous
                    .as_ref()
                    .map_or(0, |bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
                before: before_state(previous.as_ref()),
                after: Some(after_state(state.files.get(path))),
                irreversible: None,
                notable: Vec::new(),
            });
        }
        let op = if outcomes.is_empty() && this.irreversible.is_empty() {
            None
        } else {
            this.op
        };
        if let Some(op) = op {
            state.entries.push(Entry {
                op,
                kind: this.kind.clone(),
                before,
            });
        }
        Ok(OpHandle {
            op,
            kind: this.kind,
            bulk: false,
            targets: outcomes,
            irreversible: this.irreversible,
            notable: Vec::new(),
            exempt: this.exempt,
        })
    }

    fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
        let this = *self;
        let mut state = this.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut restored = Vec::new();
        for (position, ((path, spec), previous)) in this.targets.iter().zip(&this.before).enumerate().rev() {
            if !this.changed[position] {
                continue;
            }
            match previous {
                Some(content) => state.files.insert(path.clone(), content.clone()),
                None => state.files.remove(path),
            };
            restored.push(spec.path.clone());
        }
        Ok(RollbackReport {
            op: this.op,
            restored,
            failed: Vec::new(),
        })
    }
}
