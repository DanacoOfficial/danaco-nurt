// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Recording journal: mutation transactions on the real file system that keep
//! the previous state of every target, undo committed operations and expose
//! what was declared, so that writes outside a transaction are detected.
//!
//! The journal follows the protocol of
//! [`MutationTx`](nurt_contracts::journal::MutationTx): targets are declared
//! and preserved before the first change, every change names a declared
//! target, and the expected state is checked before the change. Misuse of the
//! protocol is recorded as a violation and fails the call with
//! `E_STAGE_FAILED`, so that a scenario reports it instead of aborting.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use nurt_contracts::error::{Error, ErrorCode, Result as ContractResult};
use nurt_contracts::fs::{
    AbsPath, ContentHash, Durability, ExtendedMeta, FileKind, FsError, FsErrorKind, FsResult,
    LinkKind, MetaTransferReport, WriteHandle,
};
use nurt_contracts::journal::{
    AfterState, Applied, BeforeState, ExemptWrite, Expect, ExternalChange, Mutation, MutationTx,
    OpHandle, OpId, OpKind, PreserveMode, Preserved, RollbackReason, RollbackReport, TargetAction,
    TargetIndex, TargetOutcome, TargetPath, TargetSpec,
};
use nurt_contracts::report::IrreversibleEntry;
use sha2::{Digest as _, Sha256};

use crate::fsutil::{self, Entry};

/// Previous state of one path, kept in the journal store.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Saved {
    Absent,
    File { copy: PathBuf, executable: bool },
    Link { target: PathBuf, directory: bool },
    Tree { copy: PathBuf },
}

/// One declared target with its previous states.
#[derive(Debug, Clone)]
struct TargetRecord {
    spec: TargetSpec,
    path: PathBuf,
    saved: Saved,
    before: BeforeState,
    destination: Option<(PathBuf, Saved)>,
    after: Option<AfterState>,
    changed: bool,
    bytes_held: u64,
}

/// A committed operation.
#[derive(Debug, Clone)]
struct EntryRecord {
    op: OpId,
    targets: Vec<TargetRecord>,
    created_dirs: Vec<PathBuf>,
    undone: bool,
}

/// Paths a transaction was allowed to change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    /// Declared targets and rename destinations.
    pub exact: Vec<PathBuf>,
    /// Trees changed as a whole: created trees, deleted or replaced
    /// directories and object stores of exempt writes.
    pub trees: Vec<PathBuf>,
}

impl Declared {
    /// Returns `true` when a change of `path` was declared.
    #[must_use]
    pub fn covers(&self, path: &Path) -> bool {
        self.exact.iter().any(|exact| exact == path)
            || self.trees.iter().any(|tree| path.starts_with(tree))
    }
}

#[derive(Debug, Default)]
struct JournalState {
    entries: Vec<EntryRecord>,
    declared: Declared,
    violations: Vec<String>,
    next_op: u64,
}

/// Journal of the Nurt side of a scenario.
///
/// Previous content is copied into `store`, a directory outside the
/// repository.
#[derive(Debug)]
pub struct RecordingJournal {
    store: PathBuf,
    state: Mutex<JournalState>,
}

/// Result of [`RecordingJournal::undo_last`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoResult {
    /// The newest operation was undone.
    Undone(OpId),
    /// No committed operation is left.
    Nothing,
}

impl RecordingJournal {
    /// Creates a journal keeping previous content in `store`.
    #[must_use]
    pub fn new(store: PathBuf) -> Self {
        Self {
            store,
            state: Mutex::new(JournalState::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, JournalState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Opens a transaction for an operation of `kind`.
    #[must_use]
    pub fn begin(&self, kind: OpKind) -> RecordingTx<'_> {
        let op = {
            let mut state = self.lock();
            state.next_op += 1;
            OpId(state.next_op)
        };
        RecordingTx {
            journal: self,
            op,
            kind,
            targets: Vec::new(),
            created_dirs: Vec::new(),
            staged: BTreeMap::new(),
            exempt: Vec::new(),
            irreversible: Vec::new(),
            changed_any: false,
            failed: false,
        }
    }

    /// Number of committed operations that were not undone.
    #[must_use]
    pub fn committed(&self) -> usize {
        self.lock()
            .entries
            .iter()
            .filter(|entry| !entry.undone)
            .count()
    }

    /// Protocol violations recorded so far.
    #[must_use]
    pub fn violations(&self) -> Vec<String> {
        self.lock().violations.clone()
    }

    /// Paths declared by all transactions so far.
    #[must_use]
    pub fn declared(&self) -> Declared {
        self.lock().declared.clone()
    }

    fn violation(&self, message: String) -> Error {
        let error = Error::new(
            ErrorCode::StageFailed,
            format!("journal protocol: {message}"),
        );
        self.lock().violations.push(message);
        error
    }

    /// Undoes the newest committed operation that was not undone yet,
    /// restoring every target to its previous state in reverse order.
    ///
    /// # Errors
    ///
    /// Fails with a message naming the first path that cannot be restored.
    pub fn undo_last(&self) -> Result<UndoResult, String> {
        let entry = {
            let mut state = self.lock();
            let Some(entry) = state.entries.iter_mut().rev().find(|entry| !entry.undone) else {
                return Ok(UndoResult::Nothing);
            };
            entry.undone = true;
            entry.clone()
        };
        restore_all(&entry.targets, &entry.created_dirs)
            .map_err(|(path, error)| format!("cannot restore `{}`: {error}", path.display()))?;
        Ok(UndoResult::Undone(entry.op))
    }
}

/// A transaction of the [`RecordingJournal`].
#[derive(Debug)]
pub struct RecordingTx<'j> {
    journal: &'j RecordingJournal,
    op: OpId,
    kind: OpKind,
    targets: Vec<TargetRecord>,
    created_dirs: Vec<PathBuf>,
    staged: BTreeMap<u32, PathBuf>,
    exempt: Vec<ExemptWrite>,
    irreversible: Vec<IrreversibleEntry>,
    changed_any: bool,
    failed: bool,
}

impl RecordingTx<'_> {
    fn ensure_open(&self) -> ContractResult<()> {
        if self.failed {
            return Err(Error::new(
                ErrorCode::RolledBack,
                "the transaction was rolled back after a failed call",
            ));
        }
        Ok(())
    }

    fn store_dir(&self) -> PathBuf {
        self.journal.store.join(format!("op-{}", self.op.0))
    }

    fn fail_and_restore(&mut self, error: Error) -> Error {
        self.failed = true;
        let _ = self.restore_changed();
        error
    }

    fn restore_changed(&mut self) -> Result<Vec<PathBuf>, (PathBuf, io::Error)> {
        for path in std::mem::take(&mut self.staged).into_values() {
            let _ = fs::remove_file(path);
        }
        let changed: Vec<TargetRecord> = self
            .targets
            .iter()
            .filter(|target| target.changed)
            .cloned()
            .collect();
        restore_all(&changed, &self.created_dirs)?;
        for target in &mut self.targets {
            target.changed = false;
        }
        Ok(changed.into_iter().map(|target| target.path).collect())
    }

    fn preserve_one(&mut self, spec: &TargetSpec, index: u32) -> ContractResult<Preserved> {
        let TargetPath::Fs(abs) = &spec.path else {
            return Err(self.journal.violation(format!(
                "target {index} is a provider key, which git commands never change"
            )));
        };
        let path = abs.as_path().to_path_buf();
        let exists = fs::symlink_metadata(&path).is_ok();
        if exists
            && matches!(
                spec.action,
                TargetAction::Create | TargetAction::CreateTree | TargetAction::Link
            )
        {
            return Err(Error::new(
                ErrorCode::Exists,
                format!(
                    "`{}` exists, remove it or choose another name",
                    path.display()
                ),
            ));
        }
        if !exists
            && matches!(
                spec.action,
                TargetAction::Delete
                    | TargetAction::Replace
                    | TargetAction::Rename { .. }
                    | TargetAction::Attributes
                    | TargetAction::Truncate
            )
        {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!("`{}` does not exist, check the path", path.display()),
            ));
        }
        let store = self.store_dir().join(index.to_string());
        let (saved, before, bytes_held) = save(&path, &store).map_err(io_to_error(&path))?;
        let destination = match &spec.action {
            TargetAction::Rename { to, .. } => {
                let to = to.as_path().to_path_buf();
                let store = self.store_dir().join(format!("{index}-destination"));
                let (saved, _, _) = save(&to, &store).map_err(io_to_error(&to))?;
                Some((to, saved))
            }
            _ => None,
        };
        let mode = match (&spec.action, exists) {
            (TargetAction::Rename { .. }, _) => PreserveMode::Rename,
            (_, false) => PreserveMode::Created,
            (_, true) => PreserveMode::Copy,
        };
        self.declare(
            spec,
            &path,
            destination.as_ref().map(|(to, _)| to.as_path()),
        );
        self.targets.push(TargetRecord {
            spec: spec.clone(),
            path,
            saved,
            before: before.clone(),
            destination,
            after: None,
            changed: false,
            bytes_held,
        });
        Ok(Preserved {
            index: TargetIndex(index),
            volume: None,
            mode,
            bytes_held,
            before,
            irreversible: None,
            notable: Vec::new(),
        })
    }

    fn declare(&self, spec: &TargetSpec, path: &Path, destination: Option<&Path>) {
        let mut state = self.journal.lock();
        let declared = &mut state.declared;
        declared.exact.push(path.to_path_buf());
        if let Some(destination) = destination {
            declared.exact.push(destination.to_path_buf());
        }
        if matches!(
            spec.action,
            TargetAction::CreateTree | TargetAction::Delete | TargetAction::Replace
        ) {
            declared.trees.push(path.to_path_buf());
        }
    }

    fn target(&mut self, index: TargetIndex) -> ContractResult<usize> {
        let position = index.0 as usize;
        if position >= self.targets.len() {
            let error = self
                .journal
                .violation(format!("change of undeclared target {}", index.0));
            return Err(self.fail_and_restore(error));
        }
        Ok(position)
    }

    fn check_expect(&mut self, position: usize) -> ContractResult<()> {
        let target = &self.targets[position];
        if target.changed {
            return Ok(());
        }
        if let Err(message) = matches_expect(&target.path, &target.spec.expect) {
            let error = Error::new(
                ErrorCode::Stale,
                format!(
                    "`{}` changed since it was read: {message}; read the repository again",
                    target.path.display()
                ),
            );
            return Err(self.fail_and_restore(error));
        }
        Ok(())
    }

    fn perform(&mut self, position: usize, mutation: Mutation) -> ContractResult<()> {
        let path = self.targets[position].path.clone();
        if let Some(parent) = path.parent() {
            self.create_parents(parent).map_err(io_to_error(parent))?;
        }
        let staged = self.staged.remove(&(position as u32));
        let destination = self.targets[position]
            .destination
            .as_ref()
            .map(|(to, _)| to.clone());
        let before = &self.targets[position].before;
        let mode = before.mode.filter(|_| before.kind == Some(FileKind::File));
        perform_mutation(
            &path,
            mutation,
            staged.as_deref(),
            destination.as_deref(),
            mode,
        )
        .map_err(io_to_error(&path))
    }

    fn create_parents(&mut self, dir: &Path) -> io::Result<()> {
        let mut missing = Vec::new();
        let mut current = Some(dir);
        while let Some(path) = current {
            if fs::symlink_metadata(path).is_ok() {
                break;
            }
            missing.push(path.to_path_buf());
            current = path.parent();
        }
        for path in missing.into_iter().rev() {
            fs::create_dir(&path)?;
            self.journal.lock().declared.exact.push(path.clone());
            self.created_dirs.push(path);
        }
        Ok(())
    }
}

impl MutationTx for RecordingTx<'_> {
    fn op(&self) -> Option<OpId> {
        Some(self.op)
    }

    fn preserve(&mut self, targets: &[TargetSpec]) -> ContractResult<Vec<Preserved>> {
        self.ensure_open()?;
        if self.changed_any {
            let error = self
                .journal
                .violation("targets declared after the first change".to_owned());
            return Err(self.fail_and_restore(error));
        }
        let mut preserved = Vec::with_capacity(targets.len());
        for spec in targets {
            let index = u32::try_from(self.targets.len()).unwrap_or(u32::MAX);
            match self.preserve_one(spec, index) {
                Ok(item) => preserved.push(item),
                Err(error) => {
                    self.failed = true;
                    return Err(error);
                }
            }
        }
        Ok(preserved)
    }

    fn stage(&mut self, target: TargetIndex) -> ContractResult<Box<dyn WriteHandle>> {
        self.ensure_open()?;
        let position = self.target(target)?;
        if !matches!(
            self.targets[position].spec.action,
            TargetAction::Create | TargetAction::Modify | TargetAction::Replace
        ) {
            let error = self.journal.violation(format!(
                "target {}: stage for a target that does not write content",
                target.0
            ));
            return Err(self.fail_and_restore(error));
        }
        let path = self.targets[position].path.clone();
        let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        self.create_parents(&dir).map_err(io_to_error(&dir))?;
        let staged = dir.join(format!(".nurt-stage-{}-{}", self.op.0, target.0));
        let file = fs::File::create(&staged).map_err(io_to_error(&staged))?;
        let abs = AbsPath::new(&staged).map_err(Error::from)?;
        self.journal.lock().declared.exact.push(staged.clone());
        self.staged.insert(target.0, staged);
        Ok(Box::new(StagedFile { file, path: abs }))
    }

    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> ContractResult<Applied> {
        self.ensure_open()?;
        let position = self.target(target)?;
        if let Err(message) = check_mutation(&self.targets[position].spec.action, &mutation) {
            let error = self
                .journal
                .violation(format!("target {}: {message}", target.0));
            return Err(self.fail_and_restore(error));
        }
        self.check_expect(position)?;
        self.targets[position].changed = true;
        self.changed_any = true;
        if let Err(error) = self.perform(position, mutation) {
            return Err(self.fail_and_restore(error));
        }
        let after = after_state(&self.targets[position].path);
        self.targets[position].after = Some(after.clone());
        Ok(Applied {
            index: target,
            after,
        })
    }

    fn observe_external(&mut self, changes: &[ExternalChange]) -> ContractResult<Vec<Applied>> {
        self.ensure_open()?;
        let mut applied = Vec::with_capacity(changes.len());
        for change in changes {
            let Some(index) = change.declared else {
                let error = self.journal.violation(format!(
                    "undeclared external change of `{}`",
                    change.path.as_path().display()
                ));
                return Err(self.fail_and_restore(error));
            };
            let position = self.target(index)?;
            if self.targets[position].spec.action != TargetAction::ExternalWrite {
                let error = self
                    .journal
                    .violation(format!("target {} is not an external write", index.0));
                return Err(self.fail_and_restore(error));
            }
            let after = after_state(&self.targets[position].path);
            self.targets[position].changed = true;
            self.targets[position].after = Some(after.clone());
            self.changed_any = true;
            applied.push(Applied { index, after });
        }
        Ok(applied)
    }

    fn mark_irreversible(&mut self, marker: IrreversibleEntry) {
        self.irreversible.push(marker);
    }

    fn record_exempt(&mut self, write: ExemptWrite) {
        if let ExemptWrite::ObjectStore { repo } = &write {
            let mut state = self.journal.lock();
            let root = repo.as_path();
            state.declared.trees.push(root.join("objects"));
            state.declared.trees.push(root.join(".git").join("objects"));
        }
        self.exempt.push(write);
    }

    fn commit(mut self: Box<Self>) -> ContractResult<OpHandle> {
        self.ensure_open()?;
        for path in std::mem::take(&mut self.staged).into_values() {
            let _ = fs::remove_file(path);
        }
        let changed: Vec<TargetRecord> = self
            .targets
            .iter()
            .filter(|target| target.changed)
            .cloned()
            .collect();
        let has_entry = !changed.is_empty() || !self.irreversible.is_empty();
        let outcomes = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.changed)
            .map(|(position, target)| outcome(position, target))
            .collect();
        if has_entry {
            self.journal.lock().entries.push(EntryRecord {
                op: self.op,
                targets: changed.clone(),
                created_dirs: self.created_dirs.clone(),
                undone: false,
            });
        }
        Ok(OpHandle {
            op: has_entry.then_some(self.op),
            kind: self.kind.clone(),
            bulk: changed.len() > 1,
            targets: outcomes,
            irreversible: self.irreversible.clone(),
            notable: Vec::new(),
            exempt: self.exempt.clone(),
        })
    }

    fn rollback(mut self: Box<Self>, _reason: RollbackReason) -> ContractResult<RollbackReport> {
        let op = self.changed_any.then_some(self.op);
        match self.restore_changed() {
            Ok(restored) => Ok(RollbackReport {
                op,
                restored: restored.into_iter().filter_map(fs_target).collect(),
                failed: Vec::new(),
            }),
            Err((path, _)) => Ok(RollbackReport {
                op,
                restored: Vec::new(),
                failed: fs_target(path).into_iter().collect(),
            }),
        }
    }
}

fn fs_target(path: PathBuf) -> Option<TargetPath> {
    AbsPath::new(path).ok().map(TargetPath::Fs)
}

fn outcome(position: usize, target: &TargetRecord) -> TargetOutcome {
    let mode = match (&target.spec.action, target.before.exists) {
        (TargetAction::Rename { .. }, _) => PreserveMode::Rename,
        (_, false) => PreserveMode::Created,
        (_, true) => PreserveMode::Copy,
    };
    TargetOutcome {
        index: TargetIndex(u32::try_from(position).unwrap_or(u32::MAX)),
        path: target.spec.path.clone(),
        action: target.spec.action.clone(),
        mode,
        bytes_held: target.bytes_held,
        before: target.before.clone(),
        after: target.after.clone(),
        irreversible: None,
        notable: Vec::new(),
    }
}

/// Restores `targets` in reverse order, then removes directories created
/// for them when they are empty.
fn restore_all(
    targets: &[TargetRecord],
    created_dirs: &[PathBuf],
) -> Result<(), (PathBuf, io::Error)> {
    for target in targets.iter().rev() {
        if let Some((to, saved)) = &target.destination {
            restore(to, saved).map_err(|error| (to.clone(), error))?;
        }
        restore(&target.path, &target.saved).map_err(|error| (target.path.clone(), error))?;
    }
    for dir in created_dirs.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

fn save(path: &Path, store: &Path) -> io::Result<(Saved, BeforeState, u64)> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok((Saved::Absent, BeforeState::default(), 0));
        }
        Err(error) => return Err(error),
    };
    if let Some(parent) = store.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut before = BeforeState {
        exists: true,
        kind: None,
        size: Some(meta.len()),
        hash: None,
        mode: Some(fsutil::mode_bits(&meta)),
    };
    let kind = meta.file_type();
    if kind.is_symlink() {
        before.kind = Some(FileKind::Symlink);
        let target = fs::read_link(path)?;
        let directory = path.is_dir();
        return Ok((Saved::Link { target, directory }, before, 0));
    }
    if kind.is_dir() {
        before.kind = Some(FileKind::Directory);
        fsutil::copy_tree(path, store).map_err(|error| io::Error::other(error.to_string()))?;
        return Ok((
            Saved::Tree {
                copy: store.to_path_buf(),
            },
            before,
            meta.len(),
        ));
    }
    before.kind = Some(FileKind::File);
    let bytes = fs::read(path)?;
    before.hash = Some(content_hash(&bytes));
    fs::write(store, &bytes)?;
    let saved = Saved::File {
        copy: store.to_path_buf(),
        executable: fsutil::is_executable(&meta),
    };
    Ok((saved, before, meta.len()))
}

fn restore(path: &Path, saved: &Saved) -> io::Result<()> {
    let as_io = |error: crate::error::DifftestError| io::Error::other(error.to_string());
    fsutil::remove_entry(path).map_err(as_io)?;
    match saved {
        Saved::Absent => Ok(()),
        Saved::File { copy, executable } => {
            fs::copy(copy, path)?;
            fsutil::set_executable(path, *executable).map_err(as_io)
        }
        Saved::Link { target, directory } => {
            fsutil::make_symlink(target, path, *directory).map_err(as_io)
        }
        Saved::Tree { copy } => fsutil::copy_tree(copy, path).map_err(as_io),
    }
}

fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

fn modified_ns(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_nanos()).ok())
        .unwrap_or(0)
}

/// Checks the expected state of a target before its first change.
///
/// File identity is not compared: stages under test may compute entity tags
/// through other interfaces, and the copies of a scenario are fresh files.
fn matches_expect(path: &Path, expect: &Expect) -> Result<(), String> {
    let meta = fs::symlink_metadata(path);
    match expect {
        Expect::Unchecked => Ok(()),
        Expect::Absent => match meta {
            Ok(_) => Err("it exists".to_owned()),
            Err(_) => Ok(()),
        },
        Expect::Hash(hash) => {
            let bytes = fs::read(path).map_err(|_| "it cannot be read".to_owned())?;
            (content_hash(&bytes) == *hash)
                .then_some(())
                .ok_or_else(|| "its content differs".to_owned())
        }
        Expect::Etag(etag) => {
            let meta = meta.map_err(|_| "it does not exist".to_owned())?;
            if meta.len() != etag.size || modified_ns(&meta) != etag.modified_ns {
                return Err("its size or modification time differs".to_owned());
            }
            match &etag.hash {
                Some(hash)
                    if fs::read(path)
                        .map(|bytes| content_hash(&bytes))
                        .ok()
                        .as_ref()
                        != Some(hash) =>
                {
                    Err("its content differs".to_owned())
                }
                _ => Ok(()),
            }
        }
    }
}

/// Checks that a mutation matches the declared action, by the table of the
/// reference journal.
fn check_mutation(action: &TargetAction, mutation: &Mutation) -> Result<(), String> {
    let writes_content = matches!(
        mutation,
        Mutation::WriteBytes(_) | Mutation::CommitStaged | Mutation::CopyFrom { .. }
    );
    let creates_entry = matches!(
        mutation,
        Mutation::CreateDir { .. } | Mutation::CreateLink { .. } | Mutation::CreateHardLink { .. }
    );
    let allowed = match action {
        TargetAction::Create => writes_content || creates_entry,
        TargetAction::CreateTree => matches!(mutation, Mutation::CreateDir { .. }),
        TargetAction::Modify => {
            writes_content || matches!(mutation, Mutation::WriteAt { .. } | Mutation::SetLength(_))
        }
        TargetAction::Append => matches!(mutation, Mutation::AppendBytes(_)),
        TargetAction::Delete => matches!(mutation, Mutation::Remove),
        TargetAction::Replace => {
            writes_content || creates_entry || matches!(mutation, Mutation::Remove)
        }
        TargetAction::Rename { .. } => matches!(mutation, Mutation::Rename),
        TargetAction::Attributes => matches!(mutation, Mutation::SetMeta(_)),
        TargetAction::Link => matches!(
            mutation,
            Mutation::CreateLink { .. } | Mutation::CreateHardLink { .. }
        ),
        TargetAction::Truncate => matches!(mutation, Mutation::SetLength(_)),
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "mutation {mutation:?} does not match the declared action {action:?}"
        ))
    }
}

fn perform_mutation(
    path: &Path,
    mutation: Mutation,
    staged: Option<&Path>,
    destination: Option<&Path>,
    mode: Option<u32>,
) -> io::Result<()> {
    match mutation {
        Mutation::CommitStaged => {
            let staged = staged.ok_or_else(|| io::Error::other("no staged file for the target"))?;
            replace_with(path, staged, mode)
        }
        Mutation::WriteBytes(bytes) => write_atomically(path, &bytes, mode),
        Mutation::AppendBytes(bytes) => fs::OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(&bytes),
        Mutation::WriteAt { offset, bytes } => {
            let mut file = fs::OpenOptions::new().write(true).open(path)?;
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(&bytes)
        }
        Mutation::SetLength(length) => fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .set_len(length),
        Mutation::CopyFrom { source, .. } => {
            let bytes = fs::read(source.as_path())?;
            write_atomically(path, &bytes, mode)
        }
        Mutation::Remove => {
            fsutil::remove_entry(path).map_err(|error| io::Error::other(error.to_string()))
        }
        Mutation::Rename => {
            let to = destination.ok_or_else(|| io::Error::other("rename without a destination"))?;
            fs::rename(path, to)
        }
        Mutation::CreateDir { mode } => {
            fs::create_dir(path)?;
            set_mode(path, mode)
        }
        Mutation::CreateLink { target, kind } => {
            fsutil::make_symlink(&target, path, kind != LinkKind::SymlinkFile)
                .map_err(|error| io::Error::other(error.to_string()))
        }
        Mutation::CreateHardLink { existing } => fs::hard_link(existing.as_path(), path),
        Mutation::SetMeta(meta) => {
            if let Some(mode) = meta.mode {
                set_mode(path, mode)?;
            }
            if let Some(nanos) = meta.modified_ns {
                let time =
                    UNIX_EPOCH + std::time::Duration::from_nanos(u64::try_from(nanos).unwrap_or(0));
                fs::File::options()
                    .write(true)
                    .open(path)?
                    .set_modified(time)?;
            }
            Ok(())
        }
        other => Err(io::Error::other(format!(
            "mutation {other:?} is not supported for repository files"
        ))),
    }
}

/// Writes `bytes` to a temporary file beside `path` and renames it over
/// `path`, keeping the permission bits of the previous file.
fn write_atomically(path: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.nurt-write"));
    fs::write(&temporary, bytes)?;
    replace_with(path, &temporary, mode)
}

fn replace_with(path: &Path, source: &Path, mode: Option<u32>) -> io::Result<()> {
    if let Some(mode) = mode {
        set_mode(source, mode)?;
    }
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
        fs::remove_dir_all(path)?;
    }
    fs::rename(source, path)
}

fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

fn after_state(path: &Path) -> AfterState {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return AfterState::default();
    };
    let kind = if meta.file_type().is_symlink() {
        FileKind::Symlink
    } else if meta.is_dir() {
        FileKind::Directory
    } else {
        FileKind::File
    };
    let hash = (kind == FileKind::File)
        .then(|| fs::read(path).ok().map(|bytes| content_hash(&bytes)))
        .flatten();
    AfterState {
        exists: true,
        kind: Some(kind),
        size: Some(meta.len()),
        hash,
        mode: Some(fsutil::mode_bits(&meta)),
        meta: None,
    }
}

fn io_to_error(path: &Path) -> impl FnOnce(io::Error) -> Error {
    let path = path.to_path_buf();
    move |error| {
        let code = match error.kind() {
            io::ErrorKind::NotFound => ErrorCode::NotFound,
            io::ErrorKind::AlreadyExists => ErrorCode::Exists,
            io::ErrorKind::PermissionDenied => ErrorCode::Permission,
            _ => ErrorCode::Io,
        };
        Error::new(code, format!("cannot change `{}`: {error}", path.display()))
    }
}

/// Staged file opened by [`MutationTx::stage`].
#[derive(Debug)]
struct StagedFile {
    file: fs::File,
    path: AbsPath,
}

impl Write for StagedFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for StagedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl WriteHandle for StagedFile {
    fn path(&self) -> &AbsPath {
        &self.path
    }

    fn sync(&mut self, _durability: Durability) -> FsResult<()> {
        self.file.sync_all().map_err(|_| self.fs_error("sync"))
    }

    fn set_length(&mut self, length: u64) -> FsResult<()> {
        self.file
            .set_len(length)
            .map_err(|_| self.fs_error("set_length"))
    }

    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        set_mode(self.path.as_path(), meta.basic.mode).map_err(|_| self.fs_error("apply_meta"))?;
        let mut not_transferred = Vec::new();
        if !meta.xattrs.is_empty() {
            not_transferred.push("xattr".to_owned());
        }
        if meta.acl.is_some() {
            not_transferred.push("acl".to_owned());
        }
        Ok(MetaTransferReport { not_transferred })
    }
}

impl StagedFile {
    fn fs_error(&self, op: &str) -> FsError {
        FsError::new(FsErrorKind::Io, op, Some(self.path.as_path()))
    }
}

/// Fingerprint of one entry, for detecting changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    kind: &'static str,
    executable: bool,
    digest: Option<[u8; 32]>,
    link: Option<String>,
}

/// Fingerprints of every entry below a directory, keyed by absolute path.
pub type Snapshot = BTreeMap<PathBuf, Fingerprint>;

/// Fingerprints every entry below `root`, including the repository
/// directory.
///
/// # Errors
///
/// Fails with [`crate::error::DifftestError`] when an entry cannot be read.
pub fn snapshot(root: &Path) -> crate::error::Result<Snapshot> {
    let mut snapshot = Snapshot::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for item in fs::read_dir(&dir).map_err(crate::error::io_error("read directory", &dir))? {
            let path = item
                .map_err(crate::error::io_error("read directory", &dir))?
                .path();
            let fingerprint = match fsutil::read_entry(&path)? {
                Entry::File { executable, bytes } => Fingerprint {
                    kind: "file",
                    executable,
                    digest: Some(Sha256::digest(&bytes).into()),
                    link: None,
                },
                Entry::Dir => {
                    pending.push(path.clone());
                    Fingerprint {
                        kind: "dir",
                        executable: false,
                        digest: None,
                        link: None,
                    }
                }
                Entry::Link { target } => Fingerprint {
                    kind: "link",
                    executable: false,
                    digest: None,
                    link: Some(target),
                },
                Entry::Other => Fingerprint {
                    kind: "special",
                    executable: false,
                    digest: None,
                    link: None,
                },
            };
            snapshot.insert(path, fingerprint);
        }
    }
    Ok(snapshot)
}

/// Paths that differ between two snapshots and are not covered by
/// `declared`, in path order.
#[must_use]
pub fn undeclared_changes(
    before: &Snapshot,
    after: &Snapshot,
    declared: &Declared,
) -> Vec<PathBuf> {
    let changed = before
        .iter()
        .filter(|(path, fingerprint)| after.get(*path) != Some(fingerprint))
        .map(|(path, _)| path)
        .chain(after.keys().filter(|path| !before.contains_key(*path)));
    let mut undeclared: Vec<PathBuf> = changed
        .filter(|path| !declared.covers(path))
        .filter(|path| !is_parent_of_declared(path, before, after, declared))
        .cloned()
        .collect();
    undeclared.sort();
    undeclared.dedup();
    undeclared
}

/// Directories whose only change is that a declared entry inside them was
/// created or removed do not count as changes; a directory fingerprint has
/// no content, so this applies only to created or removed directories that
/// contain declared entries.
fn is_parent_of_declared(
    path: &Path,
    before: &Snapshot,
    after: &Snapshot,
    declared: &Declared,
) -> bool {
    let is_dir = |snapshot: &Snapshot| snapshot.get(path).is_some_and(|f| f.kind == "dir");
    (is_dir(before) || is_dir(after))
        && declared
            .exact
            .iter()
            .chain(&declared.trees)
            .any(|item| item.starts_with(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::journal::OpKind;

    fn spec(path: &Path, action: TargetAction) -> TargetSpec {
        TargetSpec {
            path: TargetPath::Fs(AbsPath::new(path).unwrap()),
            action,
            expect: Expect::Unchecked,
            size_hint: None,
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, RecordingJournal) {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let journal = RecordingJournal::new(dir.path().join("store"));
        (dir, work, journal)
    }

    #[test]
    fn commits_changes_and_undoes_them_byte_for_byte() {
        let (_dir, work, journal) = setup();
        fs::write(work.join("a"), b"old").unwrap();
        fs::write(work.join("gone"), b"bye").unwrap();
        let before = snapshot(&work).unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        let preserved = tx
            .preserve(&[
                spec(&work.join("a"), TargetAction::Modify),
                spec(&work.join("new/b"), TargetAction::Create),
                spec(&work.join("gone"), TargetAction::Delete),
            ])
            .unwrap();
        assert_eq!(preserved[0].mode, PreserveMode::Copy);
        assert_eq!(preserved[1].mode, PreserveMode::Created);
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"new".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(1), Mutation::WriteBytes(b"b".to_vec()))
            .unwrap();
        tx.apply(TargetIndex(2), Mutation::Remove).unwrap();
        let handle = tx.commit().unwrap();
        assert!(handle.op.is_some());
        assert_eq!(handle.targets.len(), 3);
        let after = snapshot(&work).unwrap();
        assert!(undeclared_changes(&before, &after, &journal.declared()).is_empty());
        assert_eq!(fs::read(work.join("a")).unwrap(), b"new");
        assert_eq!(journal.committed(), 1);
        assert!(matches!(
            journal.undo_last().unwrap(),
            UndoResult::Undone(_)
        ));
        assert_eq!(snapshot(&work).unwrap(), before);
        assert_eq!(journal.undo_last().unwrap(), UndoResult::Nothing);
    }

    #[test]
    fn replaces_a_file_through_a_staged_write_and_sets_attributes() {
        let (_dir, work, journal) = setup();
        fs::write(work.join("index"), b"v1").unwrap();
        let before = snapshot(&work).unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.stage")));
        let mut modify = spec(&work.join("index"), TargetAction::Modify);
        modify.expect = Expect::Hash(content_hash(b"v1"));
        tx.preserve(&[modify, spec(&work.join("index"), TargetAction::Attributes)])
            .unwrap();
        let mut staged = tx.stage(TargetIndex(0)).unwrap();
        staged.write_all(b"v2").unwrap();
        drop(staged);
        tx.apply(TargetIndex(0), Mutation::CommitStaged).unwrap();
        let mode = nurt_contracts::fs::MetaChange {
            mode: Some(0o755),
            ..Default::default()
        };
        tx.apply(TargetIndex(1), Mutation::SetMeta(mode)).unwrap();
        tx.commit().unwrap();
        assert_eq!(fs::read(work.join("index")).unwrap(), b"v2");
        assert!(
            undeclared_changes(&before, &snapshot(&work).unwrap(), &journal.declared()).is_empty()
        );
        journal.undo_last().unwrap();
        assert_eq!(snapshot(&work).unwrap(), before);
    }

    #[test]
    fn existing_lock_file_fails_with_exists() {
        let (_dir, work, journal) = setup();
        fs::write(work.join("index.lock"), b"").unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.stage")));
        let error = tx
            .preserve(&[spec(&work.join("index.lock"), TargetAction::Create)])
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(
            tx.apply(TargetIndex(0), Mutation::Remove)
                .unwrap_err()
                .code(),
            ErrorCode::RolledBack
        );
    }

    #[test]
    fn stale_expectation_rolls_back_earlier_changes() {
        let (_dir, work, journal) = setup();
        fs::write(work.join("a"), b"1").unwrap();
        fs::write(work.join("b"), b"2").unwrap();
        let before = snapshot(&work).unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        let mut stale = spec(&work.join("b"), TargetAction::Modify);
        stale.expect = Expect::Hash(content_hash(b"other"));
        tx.preserve(&[spec(&work.join("a"), TargetAction::Modify), stale])
            .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(b"x".to_vec()))
            .unwrap();
        let error = tx
            .apply(TargetIndex(1), Mutation::WriteBytes(b"y".to_vec()))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(snapshot(&work).unwrap(), before);
        tx.rollback(RollbackReason::TargetFailed).unwrap();
        assert_eq!(journal.committed(), 0);
    }

    #[test]
    fn records_protocol_violations() {
        let (_dir, work, journal) = setup();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        tx.preserve(&[spec(&work.join("a"), TargetAction::Create)])
            .unwrap();
        let error = tx.apply(TargetIndex(0), Mutation::Rename).unwrap_err();
        assert_eq!(error.code(), ErrorCode::StageFailed);
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        assert!(tx.apply(TargetIndex(3), Mutation::Remove).is_err());
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        tx.preserve(&[spec(&work.join("c"), TargetAction::Create)])
            .unwrap();
        tx.apply(TargetIndex(0), Mutation::WriteBytes(Vec::new()))
            .unwrap();
        assert!(
            tx.preserve(&[spec(&work.join("d"), TargetAction::Create)])
                .is_err()
        );
        assert_eq!(journal.violations().len(), 3);
    }

    #[test]
    fn detects_writes_outside_the_transaction() {
        let (_dir, work, journal) = setup();
        fs::create_dir_all(work.join(".git/objects/ab")).unwrap();
        let before = snapshot(&work).unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.commit")));
        tx.record_exempt(ExemptWrite::ObjectStore {
            repo: AbsPath::new(&work).unwrap(),
        });
        tx.commit().unwrap();
        fs::write(work.join(".git/objects/ab/cdef"), b"object").unwrap();
        fs::write(work.join("stray"), b"x").unwrap();
        let undeclared =
            undeclared_changes(&before, &snapshot(&work).unwrap(), &journal.declared());
        assert_eq!(undeclared, vec![work.join("stray")]);
    }

    #[test]
    fn outcomes_keep_the_declared_index_of_changed_targets() {
        let (_dir, work, journal) = setup();
        fs::write(work.join("kept"), b"1").unwrap();
        fs::write(work.join("changed"), b"2").unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        tx.preserve(&[
            spec(&work.join("kept"), TargetAction::Modify),
            spec(&work.join("changed"), TargetAction::Modify),
        ])
        .unwrap();
        tx.apply(TargetIndex(1), Mutation::WriteBytes(b"3".to_vec()))
            .unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(handle.targets.len(), 1);
        assert_eq!(handle.targets[0].index.0, 1);
    }

    #[test]
    fn transaction_without_changes_has_no_entry() {
        let (_dir, work, journal) = setup();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        tx.preserve(&[spec(&work.join("a"), TargetAction::Create)])
            .unwrap();
        let handle = tx.commit().unwrap();
        assert_eq!(handle.op, None);
        assert_eq!(journal.committed(), 0);
    }

    #[test]
    fn deletes_and_restores_directory_trees_and_links() {
        let (_dir, work, journal) = setup();
        fs::create_dir_all(work.join("tree/sub")).unwrap();
        fs::write(work.join("tree/sub/f"), b"f").unwrap();
        let linked = fsutil::make_symlink(Path::new("tree"), &work.join("link"), true).is_ok();
        let before = snapshot(&work).unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        let mut targets = vec![spec(&work.join("tree"), TargetAction::Delete)];
        if linked {
            targets.push(spec(&work.join("link"), TargetAction::Delete));
        }
        tx.preserve(&targets).unwrap();
        for index in 0..targets.len() {
            tx.apply(TargetIndex(index as u32), Mutation::Remove)
                .unwrap();
        }
        tx.commit().unwrap();
        assert!(!work.join("tree").exists());
        journal.undo_last().unwrap();
        assert_eq!(snapshot(&work).unwrap(), before);
    }

    #[test]
    fn external_writes_are_restored_by_rollback() {
        let (_dir, work, journal) = setup();
        fs::write(work.join("index"), b"before").unwrap();
        let mut tx = Box::new(journal.begin(OpKind::command("git.test")));
        tx.preserve(&[spec(&work.join("index"), TargetAction::ExternalWrite)])
            .unwrap();
        fs::write(work.join("index"), b"after").unwrap();
        let change = ExternalChange {
            path: AbsPath::new(work.join("index")).unwrap(),
            kind: nurt_contracts::journal::ExternalChangeKind::Modified,
            declared: Some(TargetIndex(0)),
        };
        tx.observe_external(&[change]).unwrap();
        let report = tx.rollback(RollbackReason::Cancelled).unwrap();
        assert_eq!(report.restored.len(), 1);
        assert_eq!(fs::read(work.join("index")).unwrap(), b"before");
    }
}
