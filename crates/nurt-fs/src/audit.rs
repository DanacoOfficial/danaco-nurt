// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Wrapper that detects mutations made outside an active journal entry.
//!
//! [`AuditFs`] wraps any [`FsWrite`] and classifies every mutating call
//! before forwarding it. A call is admitted in one of four classes:
//!
//! | Class | Condition |
//! |---|---|
//! | [`WriteClass::Internal`] | every path lies inside a registered internal directory (state directory of the account, `.nurt-journal*` directories); any scope purpose |
//! | [`WriteClass::Unjournaled`] | every path lies inside a registered directory of a volume without a journal; scope purpose [`ScopePurpose::Operation`] |
//! | [`WriteClass::Transaction`] | scope purpose [`ScopePurpose::Operation`] and entry state `prepared` or `rolling_back` |
//! | [`WriteClass::Recovery`] | scope purpose [`ScopePurpose::Recovery`] or [`ScopePurpose::Reclaim`] and entry state `prepared`, `rolling_back` or `expiring` |
//!
//! Any other call panics with the method, the paths, the operation, the
//! scope purpose and the entry state, which fails the test that made it.
//! Admitted calls are recorded in order ([`AuditFs::recorded`]).
//!
//! Write handles are checked when they are opened; writes through an open
//! handle and [`FsWrite::lock_exclusive`], which takes no scope, are
//! forwarded without a check. [`AuditProvider`] applies the transaction and
//! recovery classes to a [`ProviderWrite`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::Duration;

use nurt_contracts::fs::{
    AbsPath, ContentHash, DirEntry, ExtendedMeta, FileLock, FileMeta, Follow, Fs, FsResult,
    FsWrite, Holder, LinkKind, MetaChange, MetaTransferReport, PathKey, Provider, ProviderCaps,
    ProviderChild, ProviderKey, ProviderScheme, ProviderSnapshot, ProviderValue, ProviderWrite,
    ReadHandle, RemoveStats, RenameMode, ScopePurpose, SpaceInfo, TxScope, VolumeId, VolumeInfo,
    WriteHandle,
};
use nurt_contracts::journal::{EntryState, OpId};

/// Source of the state of journal entries, consulted by [`AuditFs`] and
/// [`AuditProvider`] on every mutating call.
///
/// The journal implementation provides it by reading the persisted state of
/// the entry of `op` (the `STATE` file of the entry directory). The state
/// must reflect the latest durable transition: an entry whose `prepared`
/// state is not yet durable reports `None`, so that mutations of user data
/// before the durable `prepared` state are detected.
///
/// Closures `Fn(OpId) -> Option<EntryState>` implement the trait.
///
/// # Examples
///
/// ```
/// use nurt_contracts::journal::{EntryState, OpId};
/// use nurt_fs::audit::StateOracle;
///
/// let oracle = |op: OpId| (op == OpId(7)).then_some(EntryState::Prepared);
/// assert_eq!(oracle.state(OpId(7)), Some(EntryState::Prepared));
/// assert_eq!(oracle.state(OpId(8)), None);
/// ```
pub trait StateOracle: Send + Sync {
    /// Durable state of the entry of operation `op`; `None` when the entry
    /// does not exist or its state is not durable yet.
    fn state(&self, op: OpId) -> Option<EntryState>;
}

impl<F> StateOracle for F
where
    F: Fn(OpId) -> Option<EntryState> + Send + Sync,
{
    fn state(&self, op: OpId) -> Option<EntryState> {
        self(op)
    }
}

/// Class in which a mutating call was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteClass {
    /// Mutation of an operation whose entry is prepared or rolling back.
    Transaction,
    /// Recovery or reclaim of an entry that is prepared, rolling back or
    /// expiring.
    Recovery,
    /// Mutation of user data on a volume that keeps no journal, so no entry
    /// can exist.
    Unjournaled,
    /// Mutation confined to registered internal directories.
    Internal,
}

/// One admitted mutating call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    /// Method name of [`FsWrite`] or [`ProviderWrite`] (for example
    /// `"rename"`).
    pub method: &'static str,
    /// Paths passed to the method, in argument order.
    pub paths: Vec<AbsPath>,
    /// Provider key, for calls of [`AuditProvider`].
    pub key: Option<ProviderKey>,
    /// Operation of the scope.
    pub op: OpId,
    /// Purpose of the scope.
    pub purpose: ScopePurpose,
    /// Admission class.
    pub class: WriteClass,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn classify_user_write(purpose: ScopePurpose, state: Option<EntryState>) -> Option<WriteClass> {
    match (purpose, state?) {
        (ScopePurpose::Operation, EntryState::Prepared | EntryState::RollingBack) => {
            Some(WriteClass::Transaction)
        }
        (
            ScopePurpose::Recovery | ScopePurpose::Reclaim,
            EntryState::Prepared | EntryState::RollingBack | EntryState::Expiring,
        ) => Some(WriteClass::Recovery),
        _ => None,
    }
}

// Panicking is the purpose of the audit: it makes the test that issued the
// mutation fail at the offending call.
#[allow(clippy::panic, reason = "the audit fails the offending test by design")]
fn reject(method: &str, targets: &str, scope: &TxScope, state: Option<EntryState>) -> ! {
    panic!(
        "mutation outside a journal transaction: {method} on {targets} \
         (operation {}, purpose {:?}, entry state {state:?})",
        scope.op().0,
        scope.purpose()
    );
}

/// File system wrapper that verifies the write class of every mutation.
///
/// See the [module documentation](self) for the classes.
///
/// # Panics
///
/// Every mutating method of [`FsWrite`] panics when the call belongs to none
/// of the admitted classes.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::fs::{AbsPath, FsWrite};
/// use nurt_contracts::journal::{EntryState, OpId};
/// use nurt_fs::audit::AuditFs;
/// use nurt_fs::mem::MemFs;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let oracle = |_: OpId| Some(EntryState::Committed);
/// let audited = AuditFs::new(Arc::new(MemFs::new()?), Arc::new(oracle));
/// audited.add_internal_root(AbsPath::new("/state")?);
/// assert!(audited.recorded().is_empty());
/// # Ok(())
/// # }
/// ```
pub struct AuditFs<F: ?Sized> {
    inner: Arc<F>,
    oracle: Arc<dyn StateOracle>,
    internal: RwLock<Vec<AbsPath>>,
    unjournaled: RwLock<Vec<AbsPath>>,
    records: Mutex<Vec<AuditRecord>>,
}

impl<F: ?Sized> std::fmt::Debug for AuditFs<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditFs")
            .field("records", &lock(&self.records).len())
            .finish_non_exhaustive()
    }
}

impl<F: ?Sized + FsWrite> AuditFs<F> {
    /// Wraps `inner`, reading entry states from `oracle`.
    #[must_use]
    pub fn new(inner: Arc<F>, oracle: Arc<dyn StateOracle>) -> Self {
        Self {
            inner,
            oracle,
            internal: RwLock::new(Vec::new()),
            unjournaled: RwLock::new(Vec::new()),
            records: Mutex::new(Vec::new()),
        }
    }

    /// Registers a directory whose subtree holds internal state.
    pub fn add_internal_root(&self, root: AbsPath) {
        self.internal
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(root);
    }

    /// Registers a directory of a volume that keeps no journal; operations
    /// below it may change user data without an entry.
    pub fn add_unjournaled_root(&self, root: AbsPath) {
        self.unjournaled
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(root);
    }

    /// Admitted mutating calls, in call order.
    #[must_use]
    pub fn recorded(&self) -> Vec<AuditRecord> {
        lock(&self.records).clone()
    }

    /// Forgets the recorded calls.
    pub fn clear_records(&self) {
        lock(&self.records).clear();
    }

    /// Wrapped file system.
    #[must_use]
    pub fn inner(&self) -> &Arc<F> {
        &self.inner
    }

    fn is_internal(&self, path: &AbsPath) -> bool {
        self.internal
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|root| path.starts_with(root))
    }

    fn is_unjournaled(&self, path: &AbsPath) -> bool {
        self.unjournaled
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|root| path.starts_with(root))
    }

    fn admit(&self, scope: &TxScope, method: &'static str, paths: &[&AbsPath]) {
        let class = if paths.iter().all(|path| self.is_internal(path)) {
            WriteClass::Internal
        } else if scope.purpose() == ScopePurpose::Operation
            && paths.iter().all(|path| self.is_unjournaled(path))
        {
            WriteClass::Unjournaled
        } else {
            let state = self.oracle.state(scope.op());
            classify_user_write(scope.purpose(), state).unwrap_or_else(|| {
                let targets: Vec<String> = paths.iter().map(ToString::to_string).collect();
                reject(method, &targets.join(", "), scope, state)
            })
        };
        lock(&self.records).push(AuditRecord {
            method,
            paths: paths.iter().map(|path| (*path).clone()).collect(),
            key: None,
            op: scope.op(),
            purpose: scope.purpose(),
            class,
        });
    }
}

impl<F: ?Sized + FsWrite> Fs for AuditFs<F> {
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta> {
        self.inner.metadata(path, follow)
    }

    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta> {
        self.inner.extended_metadata(path, follow)
    }

    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>> {
        self.inner.open_read(path)
    }

    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>> {
        self.inner.open_stream_read(path, stream)
    }

    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>> {
        self.inner.read_dir(path)
    }

    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf> {
        self.inner.read_link(path)
    }

    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath> {
        self.inner.real_path(path)
    }

    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash> {
        self.inner.content_hash(path)
    }

    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo> {
        self.inner.volume(path)
    }

    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo> {
        self.inner.space(volume)
    }

    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey> {
        self.inner.path_key(path)
    }

    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>> {
        self.inner.holders(path)
    }

    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool> {
        self.inner.can_write_dir(dir)
    }
}

impl<F: ?Sized + FsWrite> FsWrite for AuditFs<F> {
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.admit(scope, "create_temp_in", &[dir]);
        self.inner.create_temp_in(scope, dir, name_hint)
    }

    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.admit(scope, "create_new", &[path]);
        self.inner.create_new(scope, path)
    }

    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>> {
        self.admit(scope, "open_in_place", &[path]);
        self.inner.open_in_place(scope, path)
    }

    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>> {
        self.admit(scope, "open_stream_write", &[path]);
        self.inner.open_stream_write(scope, path, stream)
    }

    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()> {
        self.admit(scope, "rename", &[from, to]);
        self.inner.rename(scope, from, to, mode)
    }

    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()> {
        self.admit(scope, "exchange", &[a, b]);
        self.inner.exchange(scope, a, b)
    }

    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()> {
        self.admit(scope, "replace_with_backup", &[target, replacement, backup]);
        self.inner
            .replace_with_backup(scope, target, replacement, backup)
    }

    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()> {
        self.admit(scope, "hard_link", &[existing, new]);
        self.inner.hard_link(scope, existing, new)
    }

    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()> {
        self.admit(scope, "clone_file", &[dst]);
        self.inner.clone_file(scope, src, dst)
    }

    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64> {
        self.admit(scope, "copy_file", &[dst]);
        self.inner.copy_file(scope, src, dst)
    }

    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.admit(scope, "remove_file", &[path]);
        self.inner.remove_file(scope, path)
    }

    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.admit(scope, "remove_dir", &[path]);
        self.inner.remove_dir(scope, path)
    }

    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats> {
        self.admit(scope, "remove_tree", &[path]);
        self.inner.remove_tree(scope, path, parallelism)
    }

    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()> {
        self.admit(scope, "create_dir", &[path]);
        self.inner.create_dir(scope, path, mode)
    }

    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()> {
        self.admit(scope, "create_link", &[link]);
        self.inner.create_link(scope, target, link, kind)
    }

    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()> {
        self.admit(scope, "set_meta", &[path]);
        self.inner.set_meta(scope, path, change)
    }

    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport> {
        self.admit(scope, "restore_meta", &[path]);
        self.inner.restore_meta(scope, path, meta)
    }

    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()> {
        self.admit(scope, "truncate", &[path]);
        self.inner.truncate(scope, path, length)
    }

    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()> {
        self.inner.sync_dir(scope, dir)
    }

    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>> {
        self.inner.lock_exclusive(path, timeout)
    }

    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()> {
        self.admit(scope, "mark_hidden", &[path]);
        self.inner.mark_hidden(scope, path)
    }
}

/// Provider wrapper that verifies the write class of every change.
///
/// Changes are admitted in the classes [`WriteClass::Transaction`] and
/// [`WriteClass::Recovery`]; providers have no internal directories.
///
/// # Panics
///
/// Every method of [`ProviderWrite`] panics when the call belongs to neither
/// class.
pub struct AuditProvider<P: ?Sized> {
    inner: Arc<P>,
    oracle: Arc<dyn StateOracle>,
    records: Mutex<Vec<AuditRecord>>,
}

impl<P: ?Sized> std::fmt::Debug for AuditProvider<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditProvider")
            .field("records", &lock(&self.records).len())
            .finish_non_exhaustive()
    }
}

impl<P: ?Sized + ProviderWrite> AuditProvider<P> {
    /// Wraps `inner`, reading entry states from `oracle`.
    #[must_use]
    pub fn new(inner: Arc<P>, oracle: Arc<dyn StateOracle>) -> Self {
        Self {
            inner,
            oracle,
            records: Mutex::new(Vec::new()),
        }
    }

    /// Admitted changes, in call order.
    #[must_use]
    pub fn recorded(&self) -> Vec<AuditRecord> {
        lock(&self.records).clone()
    }

    fn admit(&self, scope: &TxScope, method: &'static str, key: &ProviderKey) {
        let state = self.oracle.state(scope.op());
        let class = classify_user_write(scope.purpose(), state).unwrap_or_else(|| {
            let target = format!("{}:{}", key.scheme.0, key.segments.join("/"));
            reject(method, &target, scope, state)
        });
        lock(&self.records).push(AuditRecord {
            method,
            paths: Vec::new(),
            key: Some(key.clone()),
            op: scope.op(),
            purpose: scope.purpose(),
            class,
        });
    }
}

impl<P: ?Sized + ProviderWrite> Provider for AuditProvider<P> {
    fn scheme(&self) -> ProviderScheme {
        self.inner.scheme()
    }

    fn caps(&self) -> ProviderCaps {
        self.inner.caps()
    }

    fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>> {
        self.inner.get(key)
    }

    fn list(&self, key: &ProviderKey) -> FsResult<Vec<ProviderChild>> {
        self.inner.list(key)
    }

    fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot> {
        self.inner.snapshot(key)
    }
}

impl<P: ?Sized + ProviderWrite> ProviderWrite for AuditProvider<P> {
    fn set(&self, scope: &TxScope, key: &ProviderKey, value: &ProviderValue) -> FsResult<()> {
        self.admit(scope, "set", key);
        self.inner.set(scope, key, value)
    }

    fn remove(&self, scope: &TxScope, key: &ProviderKey) -> FsResult<()> {
        self.admit(scope, "remove", key);
        self.inner.remove(scope, key)
    }

    fn restore(&self, scope: &TxScope, snapshot: &ProviderSnapshot) -> FsResult<()> {
        self.admit(scope, "restore", &snapshot.key);
        self.inner.restore(scope, snapshot)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::io::Write;

    use super::*;
    use crate::mem::MemFs;

    fn p(text: &str) -> AbsPath {
        AbsPath::for_test(text)
    }

    // Tests stand in for the journal, the only producer of scopes.
    #[allow(clippy::disallowed_methods, reason = "tests stand in for the journal")]
    fn scope(op: u64, purpose: ScopePurpose) -> TxScope {
        TxScope::new_for_journal(OpId(op), purpose)
    }

    fn audited(states: &[(u64, EntryState)]) -> AuditFs<MemFs> {
        let table: HashMap<OpId, EntryState> = states
            .iter()
            .map(|(op, state)| (OpId(*op), *state))
            .collect();
        let oracle = move |op: OpId| table.get(&op).copied();
        let fs = MemFs::new().expect("build");
        fs.seed_dir(&p("/home/user/project")).expect("seed");
        fs.seed_dir(&p("/state")).expect("seed");
        let audited = AuditFs::new(Arc::new(fs), Arc::new(oracle));
        audited.add_internal_root(p("/state"));
        audited
    }

    #[test]
    #[should_panic(expected = "mutation outside a journal transaction: create_dir")]
    fn mutation_of_committed_operation_panics() {
        let fs = audited(&[(1, EntryState::Committed)]);
        let _ = fs.create_dir(
            &scope(1, ScopePurpose::Operation),
            &p("/home/user/project/d"),
            0o755,
        );
    }

    #[test]
    #[should_panic(expected = "entry state None")]
    fn mutation_without_entry_panics() {
        let fs = audited(&[]);
        let _ = fs.remove_tree(
            &scope(9, ScopePurpose::Operation),
            &p("/home/user/project"),
            1,
        );
    }

    #[test]
    fn maintenance_inside_internal_roots_is_admitted() {
        let fs = audited(&[]);
        let maintenance = scope(3, ScopePurpose::Maintenance);
        fs.create_dir(&maintenance, &p("/state/journal"), 0o755)
            .expect("internal write");
        fs.mark_hidden(&maintenance, &p("/state/journal"))
            .expect("hide");
        fs.sync_dir(&maintenance, &p("/state")).expect("sync");
        let records = fs.recorded();
        assert_eq!(records.len(), 2);
        assert!(
            records
                .iter()
                .all(|record| record.class == WriteClass::Internal)
        );
    }

    #[test]
    fn operation_on_a_volume_without_journal_is_admitted_as_unjournaled() {
        let fs = audited(&[]);
        fs.add_unjournaled_root(p("/home/user"));
        fs.create_dir(
            &scope(2, ScopePurpose::Operation),
            &p("/home/user/project/d"),
            0o755,
        )
        .expect("unjournaled write");
        let classes: Vec<WriteClass> = fs.recorded().iter().map(|record| record.class).collect();
        assert_eq!(classes, [WriteClass::Unjournaled]);
    }

    #[test]
    #[should_panic(expected = "purpose Recovery")]
    fn recovery_on_a_volume_without_journal_panics() {
        let fs = audited(&[]);
        fs.add_unjournaled_root(p("/home/user"));
        let _ = fs.create_dir(
            &scope(2, ScopePurpose::Recovery),
            &p("/home/user/project/d"),
            0o755,
        );
    }

    #[test]
    #[should_panic(expected = "purpose Maintenance")]
    fn maintenance_outside_internal_roots_panics() {
        let fs = audited(&[(3, EntryState::Prepared)]);
        let _ = fs.create_dir(
            &scope(3, ScopePurpose::Maintenance),
            &p("/home/user/x"),
            0o755,
        );
    }

    #[test]
    #[should_panic(expected = "/home/user/project/a (operation 4, purpose Reclaim")]
    fn move_from_internal_to_user_data_needs_a_transaction() {
        let fs = audited(&[]);
        let _ = fs.rename(
            &scope(4, ScopePurpose::Reclaim),
            &p("/state/a"),
            &p("/home/user/project/a"),
            RenameMode::Replace,
        );
    }

    #[test]
    fn recovery_of_expiring_entry_touching_user_data_is_admitted() {
        let fs = audited(&[(5, EntryState::Expiring), (6, EntryState::RollingBack)]);
        fs.create_dir(&scope(5, ScopePurpose::Reclaim), &p("/home/user/r"), 0o755)
            .expect("reclaim");
        fs.remove_dir(&scope(6, ScopePurpose::Recovery), &p("/home/user/r"))
            .expect("recovery");
        let classes: Vec<WriteClass> = fs.recorded().iter().map(|record| record.class).collect();
        assert_eq!(classes, [WriteClass::Recovery, WriteClass::Recovery]);
    }

    #[test]
    fn admitted_mutations_are_recorded_completely_in_order() {
        let fs = audited(&[(1, EntryState::Prepared), (2, EntryState::RollingBack)]);
        let op = scope(1, ScopePurpose::Operation);
        let dir = p("/home/user/project");
        let file = p("/home/user/project/file");
        let other = p("/home/user/project/other");
        let mut handle = fs.create_new(&op, &file).expect("create");
        handle.write_all(b"data").expect("write");
        drop(handle);
        let temp = fs
            .create_temp_in(&op, &dir, "tmp")
            .expect("temp")
            .path()
            .clone();
        fs.open_in_place(&op, &file).expect("open");
        fs.rename(&op, &temp, &other, RenameMode::Replace)
            .expect("rename");
        fs.exchange(&op, &file, &other).expect("exchange");
        fs.hard_link(&op, &file, &p("/home/user/project/link"))
            .expect("link");
        fs.clone_file(&op, &file, &p("/home/user/project/clone"))
            .expect("clone");
        fs.copy_file(&op, &file, &p("/home/user/project/copy"))
            .expect("copy");
        fs.set_meta(&op, &file, &MetaChange::default())
            .expect("meta");
        let meta = fs
            .extended_metadata(&file, Follow::NoFollow)
            .expect("read meta");
        fs.restore_meta(&op, &file, &meta).expect("restore");
        fs.truncate(&op, &file, 1).expect("truncate");
        fs.create_link(
            &op,
            Path::new("file"),
            &p("/home/user/project/sym"),
            LinkKind::SymlinkFile,
        )
        .expect("symlink");
        fs.remove_file(&op, &p("/home/user/project/sym"))
            .expect("remove");
        let back = scope(2, ScopePurpose::Operation);
        fs.create_dir(&back, &p("/home/user/project/d"), 0o755)
            .expect("dir");
        fs.remove_dir(&back, &p("/home/user/project/d"))
            .expect("rmdir");
        fs.remove_tree(&back, &p("/home/user/project/clone"), 1)
            .expect("tree");
        let methods: Vec<&str> = fs.recorded().iter().map(|record| record.method).collect();
        assert_eq!(
            methods,
            [
                "create_new",
                "create_temp_in",
                "open_in_place",
                "rename",
                "exchange",
                "hard_link",
                "clone_file",
                "copy_file",
                "set_meta",
                "restore_meta",
                "truncate",
                "create_link",
                "remove_file",
                "create_dir",
                "remove_dir",
                "remove_tree",
            ]
        );
        let rename = &fs.recorded()[3];
        assert_eq!(rename.paths, [temp, other]);
        assert_eq!(rename.op, OpId(1));
        assert_eq!(rename.class, WriteClass::Transaction);
        fs.clear_records();
        assert!(fs.recorded().is_empty());
    }

    #[test]
    fn reads_and_locks_pass_through_without_records() {
        let fs = audited(&[]);
        let dir = p("/home/user/project");
        fs.metadata(&dir, Follow::Follow).expect("metadata");
        fs.read_dir(&dir).expect("list").for_each(drop);
        fs.volume(&dir).expect("volume");
        fs.path_key(&dir).expect("key");
        fs.holders(&dir).expect("holders");
        fs.can_write_dir(&dir).expect("writable");
        fs.real_path(&dir).expect("real");
        let id = fs.volume(&dir).expect("volume").id;
        fs.space(&id).expect("space");
        drop(
            fs.lock_exclusive(&p("/state/lock"), Duration::ZERO)
                .expect("lock"),
        );
        assert!(fs.open_read(&dir).is_err());
        assert!(fs.open_stream_read(&dir, "s").is_err());
        assert!(fs.read_link(&dir).is_err());
        assert!(fs.content_hash(&dir).is_err());
        assert!(fs.recorded().is_empty());
        assert!(format!("{fs:?}").starts_with("AuditFs"));
        assert!(Arc::strong_count(fs.inner()) >= 1);
    }

    #[test]
    fn stream_writes_and_replacement_are_audited() {
        let fs = audited(&[(1, EntryState::Prepared)]);
        let op = scope(1, ScopePurpose::Operation);
        let file = p("/home/user/project/f");
        fs.create_new(&op, &file).expect("create");
        assert!(fs.open_stream_write(&op, &file, "s").is_err());
        assert!(
            fs.replace_with_backup(&op, &file, &file, &p("/home/user/project/b"))
                .is_err()
        );
        let methods: Vec<&str> = fs.recorded().iter().map(|record| record.method).collect();
        assert_eq!(
            methods,
            ["create_new", "open_stream_write", "replace_with_backup"]
        );
    }

    #[derive(Default)]
    struct TableProvider {
        values: Mutex<BTreeMap<Vec<String>, ProviderValue>>,
    }

    impl Provider for TableProvider {
        fn scheme(&self) -> ProviderScheme {
            ProviderScheme("table".to_owned())
        }

        fn caps(&self) -> ProviderCaps {
            ProviderCaps {
                writable: true,
                hierarchical: false,
                persistent: false,
            }
        }

        fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>> {
            Ok(lock(&self.values).get(&key.segments).cloned())
        }

        fn list(&self, _key: &ProviderKey) -> FsResult<Vec<ProviderChild>> {
            Ok(Vec::new())
        }

        fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot> {
            let value = self.get(key)?;
            Ok(ProviderSnapshot {
                key: key.clone(),
                existed: value.is_some(),
                value,
                subtree: None,
            })
        }
    }

    impl ProviderWrite for TableProvider {
        fn set(&self, _scope: &TxScope, key: &ProviderKey, value: &ProviderValue) -> FsResult<()> {
            lock(&self.values).insert(key.segments.clone(), value.clone());
            Ok(())
        }

        fn remove(&self, _scope: &TxScope, key: &ProviderKey) -> FsResult<()> {
            lock(&self.values).remove(&key.segments);
            Ok(())
        }

        fn restore(&self, scope: &TxScope, snapshot: &ProviderSnapshot) -> FsResult<()> {
            match &snapshot.value {
                Some(value) => self.set(scope, &snapshot.key, value),
                None => self.remove(scope, &snapshot.key),
            }
        }
    }

    fn table_key() -> ProviderKey {
        ProviderKey {
            scheme: ProviderScheme("table".to_owned()),
            segments: vec!["a".to_owned()],
            value: None,
        }
    }

    #[test]
    fn provider_changes_inside_transactions_are_recorded() {
        let oracle = |op: OpId| (op == OpId(1)).then_some(EntryState::Prepared);
        let provider = AuditProvider::new(Arc::new(TableProvider::default()), Arc::new(oracle));
        let op = scope(1, ScopePurpose::Operation);
        let key = table_key();
        let before = provider.snapshot(&key).expect("snapshot");
        provider
            .set(&op, &key, &ProviderValue::U32(1))
            .expect("set");
        assert_eq!(
            provider.get(&key).expect("get"),
            Some(ProviderValue::U32(1))
        );
        provider.remove(&op, &key).expect("remove");
        provider.restore(&op, &before).expect("restore");
        assert_eq!(provider.scheme().0, "table");
        assert!(provider.caps().writable);
        assert!(provider.list(&key).expect("list").is_empty());
        let methods: Vec<&str> = provider
            .recorded()
            .iter()
            .map(|record| record.method)
            .collect();
        assert_eq!(methods, ["set", "remove", "restore"]);
        assert!(format!("{provider:?}").starts_with("AuditProvider"));
    }

    #[test]
    #[should_panic(expected = "set on table:a")]
    fn provider_change_outside_transaction_panics() {
        let oracle = |_: OpId| Some(EntryState::Undone);
        let provider = AuditProvider::new(Arc::new(TableProvider::default()), Arc::new(oracle));
        let _ = provider.set(
            &scope(1, ScopePurpose::Operation),
            &table_key(),
            &ProviderValue::U32(1),
        );
    }
}
