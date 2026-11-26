// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Undo check: comparison of the current state of items with the journal.
//!
//! Before an undo or redo writes anything, every item of the plan is compared
//! with the state the journal recorded after the change: the target must have
//! the recorded hash, kind, size and permission bits, a deleted target must
//! still be absent, and the preserved content of an overwritten or deleted
//! file must still have the hash it had before the change. The module also
//! builds the language errors of `undo` and `redo`.

use std::collections::{BTreeMap, HashMap};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, ContentHash, FileKind, Follow, Fs, FsErrorKind, PathKey, ProviderKey,
};
use nurt_contracts::journal::{
    AfterState, BeforeState, Conflict, ConflictKind, EntryState, Expect, OpId, OpState,
    PreserveMode, TargetAction, TargetIndex, TargetOutcome, TargetPath,
};
use serde_json::Value;

use crate::history;
use crate::manifest::TargetRole;
use crate::store::JournalCtx;

/// Identity of an item address: the volume path key of a file system entry,
/// or the provider key. Spellings that denote the same entry have equal keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ItemKey {
    /// File system entry.
    Fs(PathKey),
    /// Provider key or value.
    Provider(ProviderKey),
}

/// Current state of an address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Observed {
    /// The entry exists.
    pub(crate) exists: bool,
    /// Kind of the entry.
    pub(crate) kind: Option<FileKind>,
    /// Size of a regular file.
    pub(crate) size: Option<u64>,
    /// Permission bits.
    pub(crate) mode: Option<u32>,
    /// Content hash, computed only when requested.
    pub(crate) hash: Option<ContentHash>,
}

/// Read access to the current state of items and to preserved content.
pub(crate) trait TargetProbe {
    /// Current state of an address; the content hash is computed only when
    /// `with_hash` is true and the address is a regular file or a value.
    fn observe(&self, path: &TargetPath, with_hash: bool) -> Result<Observed>;

    /// Identity key of an address.
    fn item_key(&self, path: &TargetPath) -> Result<ItemKey>;

    /// Hashes of the content preserved for the items `indexes` of entry
    /// `op`, read from the entry once for all of them; an item for which the
    /// entry keeps no preserved content is absent from the map.
    fn blob_hashes(
        &self,
        op: OpId,
        indexes: &[TargetIndex],
    ) -> Result<HashMap<TargetIndex, ContentHash>>;
}

/// Hashes of preserved content by entry and target index.
pub(crate) type BlobHashes = HashMap<(OpId, TargetIndex), ContentHash>;

/// State an address must have for the undo of an item to proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expected {
    /// Nothing exists at the address.
    Absent,
    /// An entry exists; every known field must match.
    Present {
        /// Kind.
        kind: Option<FileKind>,
        /// Size of a regular file.
        size: Option<u64>,
        /// Permission bits.
        mode: Option<u32>,
        /// Content hash.
        hash: Option<ContentHash>,
    },
}

impl Expected {
    /// State recorded after the change.
    pub(crate) fn after(after: &AfterState) -> Self {
        if after.exists {
            Self::Present {
                kind: after.kind,
                size: after.size,
                mode: after.mode,
                hash: after.hash,
            }
        } else {
            Self::Absent
        }
    }

    /// State recorded before the change; it is the state after an older
    /// change of the same address.
    pub(crate) fn before(before: &BeforeState) -> Self {
        if before.exists {
            Self::Present {
                kind: before.kind,
                size: before.size,
                mode: before.mode,
                hash: before.hash,
            }
        } else {
            Self::Absent
        }
    }

    /// Recorded content hash.
    pub(crate) fn hash(&self) -> Option<ContentHash> {
        match self {
            Self::Absent => None,
            Self::Present { hash, .. } => *hash,
        }
    }

    /// Expected state handed to the restoring transaction, which checks it
    /// atomically with the change.
    pub(crate) fn to_expect(&self) -> Expect {
        match self {
            Self::Absent => Expect::Absent,
            Self::Present {
                hash: Some(hash), ..
            } => Expect::Hash(*hash),
            Self::Present { hash: None, .. } => Expect::Unchecked,
        }
    }

    /// Difference between the expected and the observed state.
    pub(crate) fn classify(&self, observed: &Observed) -> Option<ConflictKind> {
        let Self::Present {
            kind,
            size,
            mode,
            hash,
        } = self
        else {
            return observed.exists.then_some(ConflictKind::Created);
        };
        if !observed.exists {
            return Some(ConflictKind::Deleted);
        }
        if differs(*kind, observed.kind) {
            return Some(ConflictKind::TypeChanged);
        }
        let changed = differs(*size, observed.size)
            || differs(*mode, observed.mode)
            || differs(*hash, observed.hash);
        changed.then_some(ConflictKind::Modified)
    }

    fn wants_hash(&self) -> bool {
        self.hash().is_some()
    }
}

fn differs<T: PartialEq>(expected: Option<T>, observed: Option<T>) -> bool {
    matches!((expected, observed), (Some(expected), Some(observed)) if expected != observed)
}

/// Current state of a file system entry, without following a final link.
///
/// A missing entry, or a missing directory on the way to it, is reported as
/// absent. Fails with the file system error otherwise.
pub(crate) fn observe_fs(fs: &dyn Fs, path: &AbsPath, with_hash: bool) -> Result<Observed> {
    let meta = match fs.metadata(path, Follow::NoFollow) {
        Ok(meta) => meta,
        Err(error)
            if matches!(
                error.kind,
                FsErrorKind::NotFound | FsErrorKind::NotADirectory
            ) =>
        {
            return Ok(Observed::default());
        }
        Err(error) => return Err(error.into()),
    };
    let is_file = meta.kind == FileKind::File;
    let hash = if with_hash && is_file {
        Some(fs.content_hash(path)?)
    } else {
        None
    };
    Ok(Observed {
        exists: true,
        kind: Some(meta.kind),
        size: is_file.then_some(meta.size),
        mode: Some(meta.mode),
        hash,
    })
}

/// Identity key of an address on the file system interface.
///
/// Fails when the volume of a file system path cannot be determined.
pub(crate) fn fs_item_key(fs: &dyn Fs, path: &TargetPath) -> Result<ItemKey> {
    match path {
        TargetPath::Fs(path) => Ok(ItemKey::Fs(fs.path_key(path)?)),
        TargetPath::Provider(key) => Ok(ItemKey::Provider(key.clone())),
    }
}

/// Destination of a rename within one volume, recorded by a single record.
/// A rename across volumes has one record per volume, each describing its
/// own address.
fn local_rename(target: &TargetOutcome, role: TargetRole) -> Option<&AbsPath> {
    match &target.action {
        TargetAction::Rename { to, .. } if role == TargetRole::Whole => Some(to),
        _ => None,
    }
}

/// Addresses changed by a recorded target: the target and, for a rename
/// within one volume, its destination.
pub(crate) fn addresses(target: &TargetOutcome, role: TargetRole) -> Vec<TargetPath> {
    match local_rename(target, role) {
        Some(to) => vec![target.path.clone(), TargetPath::Fs(to.clone())],
        None => vec![target.path.clone()],
    }
}

/// Address where the entry changed by a target is found after the change:
/// the destination of a rename within one volume, otherwise the target
/// itself.
pub(crate) fn primary(target: &TargetOutcome, role: TargetRole) -> TargetPath {
    match local_rename(target, role) {
        Some(to) => TargetPath::Fs(to.clone()),
        None => target.path.clone(),
    }
}

/// Expected current state of every address of a target: the state recorded
/// after the change. For a rename within one volume the source must be free
/// and the after-state describes the entry at the destination.
pub(crate) fn expected_after(
    target: &TargetOutcome,
    role: TargetRole,
) -> Vec<(TargetPath, Expected)> {
    let Some(after) = &target.after else {
        return Vec::new();
    };
    match local_rename(target, role) {
        Some(to) => vec![
            (target.path.clone(), Expected::Absent),
            (TargetPath::Fs(to.clone()), Expected::after(after)),
        ],
        None => vec![(target.path.clone(), Expected::after(after))],
    }
}

/// Address in the form shown to the language: a path, or a provider key as
/// `scheme:segment/segment` with `@value` for a value.
pub(crate) fn display_path(path: &TargetPath) -> String {
    match path {
        TargetPath::Fs(path) => path.to_language_string(),
        TargetPath::Provider(key) => {
            let mut text = format!("{}:{}", key.scheme.0, key.segments.join("/"));
            if let Some(value) = &key.value {
                text.push('@');
                text.push_str(value);
            }
            text
        }
    }
}

/// Compares one address with its expected state.
///
/// Fails when the address cannot be read.
pub(crate) fn check_address<P>(
    probe: &P,
    op: OpId,
    path: &TargetPath,
    expected: &Expected,
) -> Result<Option<Conflict>>
where
    P: TargetProbe + ?Sized,
{
    let observed = probe.observe(path, expected.wants_hash())?;
    Ok(expected.classify(&observed).map(|kind| Conflict {
        op,
        path: path.clone(),
        kind,
        expected: expected.hash(),
        actual: observed.hash,
    }))
}

/// Hash recorded before the change for a target whose preserved content is
/// compared with it.
///
/// Deleted and overwritten files are preserved by keeping the old file
/// itself; a process that kept it open can still write to it. Copies and
/// clones are private to the journal and are not checked.
fn checked_blob(target: &TargetOutcome) -> Option<ContentHash> {
    let keeps_old_file = matches!(
        target.mode,
        PreserveMode::Moved
            | PreserveMode::Exchange
            | PreserveMode::ReplaceBackup
            | PreserveMode::Hardlink
    );
    if keeps_old_file && target.before.kind == Some(FileKind::File) {
        target.before.hash
    } else {
        None
    }
}

/// Hashes of the preserved content of every target that [`check_blob`]
/// compares, with one call of [`TargetProbe::blob_hashes`] per entry.
///
/// Fails when the preserved content of an entry cannot be read.
pub(crate) fn preserved_hashes<'t, P>(
    probe: &P,
    targets: impl IntoIterator<Item = (OpId, &'t TargetOutcome)>,
) -> Result<BlobHashes>
where
    P: TargetProbe + ?Sized,
{
    let mut wanted: BTreeMap<OpId, Vec<TargetIndex>> = BTreeMap::new();
    for (op, target) in targets {
        if checked_blob(target).is_some() {
            wanted.entry(op).or_default().push(target.index);
        }
    }
    let mut hashes = BlobHashes::new();
    for (op, indexes) in wanted {
        for (index, hash) in probe.blob_hashes(op, &indexes)? {
            hashes.insert((op, index), hash);
        }
    }
    Ok(hashes)
}

/// Compares the preserved content of an item, hashed by
/// [`preserved_hashes`], with the hash recorded before the change.
///
/// Fails with `E_JOURNAL_CORRUPT` when the preserved content is missing.
pub(crate) fn check_blob(
    hashes: &BlobHashes,
    op: OpId,
    target: &TargetOutcome,
) -> Result<Option<Conflict>> {
    let Some(recorded) = checked_blob(target) else {
        return Ok(None);
    };
    match hashes.get(&(op, target.index)).copied() {
        None => Err(corrupt_error(op).with_problem("missing_preserved_content")),
        Some(found) if found != recorded => Ok(Some(Conflict {
            op,
            path: target.path.clone(),
            kind: ConflictKind::BlobModified,
            expected: Some(recorded),
            actual: Some(found),
        })),
        Some(_) => Ok(None),
    }
}

/// Current state of items on the file system and the providers of the
/// journal store, and the preserved content of its entries.
///
/// Provider values are compared by existence only: the journal records no
/// hash of a provider value after the change.
impl TargetProbe for JournalCtx {
    fn observe(&self, path: &TargetPath, with_hash: bool) -> Result<Observed> {
        match path {
            TargetPath::Fs(path) => observe_fs(self.fs.as_ref(), path, with_hash),
            TargetPath::Provider(key) => {
                let provider = self.provider(&key.scheme).ok_or_else(|| {
                    Error::new(
                        ErrorCode::NotFound,
                        format!(
                            "no provider serves the scheme {}; enable it to undo its values",
                            key.scheme.0
                        ),
                    )
                })?;
                Ok(Observed {
                    exists: provider.get(key)?.is_some(),
                    ..Observed::default()
                })
            }
        }
    }

    fn item_key(&self, path: &TargetPath) -> Result<ItemKey> {
        fs_item_key(self.fs.as_ref(), path)
    }

    fn blob_hashes(
        &self,
        op: OpId,
        indexes: &[TargetIndex],
    ) -> Result<HashMap<TargetIndex, ContentHash>> {
        let mut hashes = HashMap::with_capacity(indexes.len());
        for (index, data) in history::preserved_files(self, op, indexes)? {
            let data = match data {
                history::PreservedData::File(data) => data,
                history::PreservedData::Inline(hash) => {
                    hashes.insert(index, hash);
                    continue;
                }
            };
            match self.fs.content_hash(&data) {
                Ok(hash) => {
                    hashes.insert(index, hash);
                }
                Err(error) if error.kind == FsErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(hashes)
    }
}

/// `E_JOURNAL_NOOP` for an entry that cannot be undone or redone in its
/// state, or that does not exist (`state: None`).
pub(crate) fn noop_error(op: OpId, state: Option<EntryState>) -> Error {
    let message = match state.and_then(OpState::from_entry) {
        None if state.is_some() => {
            format!(
                "operation {} is still in progress; retry after it completes",
                op.0
            )
        }
        None => format!(
            "operation {} is not in the journal; list operations with history",
            op.0
        ),
        Some(OpState::Undone) => format!(
            "operation {} is already undone; list operations with history",
            op.0
        ),
        Some(_) => format!("operation {} has nothing left to restore", op.0),
    };
    let error = Error::new(ErrorCode::JournalNoop, message).with_detail("op", Value::from(op.0));
    match state {
        Some(state) => error.with_detail("state", state_value(state)),
        None => error,
    }
}

/// `E_JOURNAL_NOOP` when the selection is empty.
pub(crate) fn nothing_error(message: &str) -> Error {
    Error::new(ErrorCode::JournalNoop, message)
}

/// `E_JOURNAL_EXPIRED` listing the expired entries of the selection.
pub(crate) fn expired_error(ops: &[OpId]) -> Error {
    let numbers: Vec<u64> = ops.iter().map(|op| op.0).collect();
    let message = format!(
        "{} of the selection expired and can no longer be undone; nothing changed, select later operations",
        count_phrase(numbers.len(), "operation")
    );
    Error::new(ErrorCode::JournalExpired, message).with_detail("expired", Value::from(numbers))
}

/// `E_JOURNAL_CORRUPT` for an entry that failed its integrity check.
pub(crate) fn corrupt_error(op: OpId) -> Error {
    Error::new(
        ErrorCode::JournalCorrupt,
        format!(
            "operation {} failed its integrity check; its data is kept for salvage",
            op.0
        ),
    )
    .with_detail("op", Value::from(op.0))
}

/// `E_JOURNAL_CONFLICT` listing the differences, with ready programs that
/// resolve them.
pub(crate) fn conflict_error(conflicts: &[Conflict], fixes: &[String]) -> Error {
    let message = format!(
        "{} changed after the operation; nothing changed, choose on_conflict: force or include_later",
        count_phrase(conflicts.len(), "item")
    );
    let mut error = Error::new(ErrorCode::JournalConflict, message).with_detail(
        "conflicts",
        serde_json::to_value(conflicts).unwrap_or_default(),
    );
    for fix in fixes {
        error = error.with_fix(fix.clone());
    }
    error
}

/// `E_JOURNAL_CONFLICT` for `on_conflict: abort`: only the fact of the abort
/// and the number of differences are reported.
pub(crate) fn aborted_error(count: usize) -> Error {
    Error::new(
        ErrorCode::JournalConflict,
        format!(
            "the undo found {} and was aborted; nothing changed",
            count_phrase(count, "difference")
        ),
    )
    .with_problem("aborted")
    .with_detail("count", Value::from(count))
}

/// `E_JOURNAL_PARTIAL` for a selection with items released for space when
/// the call asked for whole entries.
pub(crate) fn partial_error(
    ops: &[OpId],
    available: &[TargetPath],
    unavailable: &[TargetPath],
) -> Error {
    let numbers: Vec<u64> = ops.iter().map(|op| op.0).collect();
    let released = match unavailable.len() {
        1 => "1 item was".to_owned(),
        count => format!("{count} items were"),
    };
    let message = format!(
        "{released} released for space; nothing changed, use on_conflict: partial to undo the remaining items"
    );
    let error = Error::new(ErrorCode::JournalPartial, message)
        .with_detail(
            "available",
            serde_json::to_value(available).unwrap_or_default(),
        )
        .with_detail(
            "unavailable",
            serde_json::to_value(unavailable).unwrap_or_default(),
        );
    match numbers.as_slice() {
        [single] => error.with_detail("op", Value::from(*single)),
        _ => error.with_detail("ops", Value::from(numbers)),
    }
}

/// `E_ROLLBACK_FAILED` listing the items that were not restored.
pub(crate) fn rollback_failed_error(failed: &[TargetPath]) -> Error {
    Error::new(
        ErrorCode::RollbackFailed,
        format!(
            "{} could not be restored; restore them from the listed paths",
            count_phrase(failed.len(), "item")
        ),
    )
    .with_detail("failed", serde_json::to_value(failed).unwrap_or_default())
}

fn state_value(state: EntryState) -> Value {
    match OpState::from_entry(state) {
        Some(listed) => Value::from(listed.as_str()),
        None => serde_json::to_value(state).unwrap_or_default(),
    }
}

fn count_phrase(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::fs::{ContentHash, FileKind};
    use nurt_contracts::journal::{
        AfterState, BeforeState, ConflictKind, EntryState, Expect, OpId, TargetPath,
    };

    use super::{
        Expected, Observed, aborted_error, noop_error, observe_fs, partial_error,
        rollback_failed_error,
    };
    use crate::undo::testkit::{TestJournal, abs, hash_of};

    fn file(hash: ContentHash) -> Observed {
        Observed {
            exists: true,
            kind: Some(FileKind::File),
            size: Some(1),
            mode: Some(0o644),
            hash: Some(hash),
        }
    }

    fn present(hash: ContentHash) -> Expected {
        Expected::Present {
            kind: Some(FileKind::File),
            size: Some(1),
            mode: Some(0o644),
            hash: Some(hash),
        }
    }

    #[test]
    fn classify_reports_each_kind_of_difference() {
        let a = hash_of(b"a");
        let b = hash_of(b"b");
        assert_eq!(present(a).classify(&file(a)), None);
        assert_eq!(present(a).classify(&file(b)), Some(ConflictKind::Modified));
        assert_eq!(
            present(a).classify(&Observed::default()),
            Some(ConflictKind::Deleted)
        );
        let directory = Observed {
            kind: Some(FileKind::Directory),
            hash: None,
            ..file(a)
        };
        assert_eq!(
            present(a).classify(&directory),
            Some(ConflictKind::TypeChanged)
        );
        let chmod = Observed {
            mode: Some(0o600),
            ..file(a)
        };
        assert_eq!(present(a).classify(&chmod), Some(ConflictKind::Modified));
        assert_eq!(
            Expected::Absent.classify(&file(a)),
            Some(ConflictKind::Created)
        );
        assert_eq!(Expected::Absent.classify(&Observed::default()), None);
    }

    #[test]
    fn unknown_fields_are_not_compared() {
        let a = hash_of(b"a");
        let loose = Expected::Present {
            kind: None,
            size: None,
            mode: None,
            hash: None,
        };
        assert_eq!(loose.classify(&file(a)), None);
        assert_eq!(loose.to_expect(), Expect::Unchecked);
        assert_eq!(present(a).to_expect(), Expect::Hash(a));
        assert_eq!(Expected::Absent.to_expect(), Expect::Absent);
    }

    #[test]
    fn expected_states_follow_recorded_existence() {
        let gone = AfterState {
            exists: false,
            ..AfterState::default()
        };
        assert_eq!(Expected::after(&gone), Expected::Absent);
        let absent = BeforeState {
            exists: false,
            ..BeforeState::default()
        };
        assert_eq!(Expected::before(&absent), Expected::Absent);
        let kept = BeforeState {
            exists: true,
            hash: Some(hash_of(b"x")),
            ..BeforeState::default()
        };
        assert_eq!(Expected::before(&kept).hash(), Some(hash_of(b"x")));
    }

    #[test]
    fn observe_fs_reports_missing_entries_as_absent() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"abc");
        journal.put_dir("/home/user/d");
        journal.with_fs(|fs| {
            let found = observe_fs(fs, &abs("/home/user/a"), true).unwrap();
            assert!(found.exists);
            assert_eq!(found.size, Some(3));
            assert_eq!(found.hash, Some(hash_of(b"abc")));
            let unhashed = observe_fs(fs, &abs("/home/user/a"), false).unwrap();
            assert_eq!(unhashed.hash, None);
            let directory = observe_fs(fs, &abs("/home/user/d"), true).unwrap();
            assert_eq!(
                (directory.kind, directory.size, directory.hash),
                (Some(FileKind::Directory), None, None)
            );
            let missing = observe_fs(fs, &abs("/home/user/none"), true).unwrap();
            assert_eq!(missing, Observed::default());
            let failing = observe_fs(fs, &abs("/denied/a"), true).unwrap_err();
            assert_eq!(failing.code, ErrorCode::Permission);
        });
    }

    #[test]
    fn errors_carry_codes_and_details() {
        let noop = noop_error(OpId(4), Some(EntryState::Undone));
        assert_eq!(noop.code, ErrorCode::JournalNoop);
        assert_eq!(noop.details["state"], "undone");
        let running = noop_error(OpId(4), Some(EntryState::Prepared));
        assert_eq!(running.details["state"], "prepared");
        assert!(running.message.contains("in progress"));
        let aborted = aborted_error(2);
        assert_eq!(aborted.code, ErrorCode::JournalConflict);
        assert!(!aborted.details.contains_key("conflicts"));
        let path = TargetPath::Fs(abs("/home/user/a"));
        let partial = partial_error(&[OpId(1), OpId(2)], &[], std::slice::from_ref(&path));
        assert_eq!(partial.details["ops"], serde_json::json!([1, 2]));
        let failed = rollback_failed_error(&[path]);
        assert_eq!(failed.code, ErrorCode::RollbackFailed);
        assert_eq!(failed.details["failed"].as_array().map(Vec::len), Some(1));
    }

    use std::cell::RefCell;
    use std::collections::HashMap;

    use nurt_contracts::error::Error;
    use nurt_contracts::fs::{ProviderKey, ProviderScheme};
    use nurt_contracts::journal::{
        Conflict, PreserveMode, TargetAction, TargetIndex, TargetOutcome,
    };

    use super::{
        BlobHashes, ItemKey, TargetProbe, addresses, check_address, check_blob, conflict_error,
        corrupt_error, display_path, expected_after, expired_error, preserved_hashes, primary,
    };
    use crate::manifest::TargetRole;

    /// Probe with a fixed observation and fixed preserved hashes that records
    /// how it was called.
    struct Probe {
        observed: Observed,
        blobs: HashMap<TargetIndex, ContentHash>,
        hash_requests: RefCell<Vec<bool>>,
        blob_calls: RefCell<Vec<(OpId, Vec<TargetIndex>)>>,
    }

    impl Probe {
        fn new(observed: Observed) -> Self {
            Self {
                observed,
                blobs: HashMap::new(),
                hash_requests: RefCell::new(Vec::new()),
                blob_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl TargetProbe for Probe {
        fn observe(
            &self,
            _path: &TargetPath,
            with_hash: bool,
        ) -> nurt_contracts::error::Result<Observed> {
            self.hash_requests.borrow_mut().push(with_hash);
            Ok(self.observed.clone())
        }

        fn item_key(&self, _path: &TargetPath) -> nurt_contracts::error::Result<ItemKey> {
            Err(Error::new(ErrorCode::Io, "no keys"))
        }

        fn blob_hashes(
            &self,
            op: OpId,
            indexes: &[TargetIndex],
        ) -> nurt_contracts::error::Result<HashMap<TargetIndex, ContentHash>> {
            self.blob_calls.borrow_mut().push((op, indexes.to_vec()));
            Ok(indexes
                .iter()
                .filter_map(|index| self.blobs.get(index).map(|hash| (*index, *hash)))
                .collect())
        }
    }

    fn outcome(index: u32, path: &str, action: TargetAction, mode: PreserveMode) -> TargetOutcome {
        TargetOutcome {
            index: TargetIndex(index),
            path: TargetPath::Fs(abs(path)),
            action,
            mode,
            bytes_held: 0,
            before: BeforeState {
                exists: true,
                kind: Some(FileKind::File),
                size: Some(6),
                hash: Some(hash_of(b"before")),
                ..BeforeState::default()
            },
            after: Some(AfterState {
                exists: true,
                kind: Some(FileKind::File),
                size: Some(5),
                hash: Some(hash_of(b"after")),
                mode: Some(0o644),
                ..AfterState::default()
            }),
            irreversible: None,
            notable: Vec::new(),
        }
    }

    fn rename(from: &str, to: &str) -> TargetOutcome {
        outcome(
            0,
            from,
            TargetAction::Rename {
                to: abs(to),
                replace: false,
            },
            PreserveMode::Rename,
        )
    }

    #[test]
    fn a_rename_within_one_volume_changes_both_addresses() {
        let target = rename("/home/user/a", "/home/user/b");
        let source = TargetPath::Fs(abs("/home/user/a"));
        let destination = TargetPath::Fs(abs("/home/user/b"));
        assert_eq!(
            addresses(&target, TargetRole::Whole),
            [source.clone(), destination.clone()]
        );
        assert_eq!(primary(&target, TargetRole::Whole), destination);
        let after = target.after.as_ref().unwrap();
        assert_eq!(
            expected_after(&target, TargetRole::Whole),
            [
                (source, Expected::Absent),
                (destination, Expected::after(after))
            ]
        );
    }

    #[test]
    fn the_parts_of_a_rename_across_volumes_each_describe_their_own_address() {
        let target = rename("/mnt/a/file", "/mnt/b/file");
        let own = TargetPath::Fs(abs("/mnt/a/file"));
        for role in [TargetRole::Source, TargetRole::Destination] {
            assert_eq!(addresses(&target, role), [own.clone()]);
            assert_eq!(primary(&target, role), own);
            let expected = expected_after(&target, role);
            assert_eq!(expected.len(), 1);
            assert_eq!(expected[0].0, own);
        }
    }

    #[test]
    fn nothing_is_expected_of_a_target_without_a_state_after_the_change() {
        let mut target = outcome(0, "/home/user/a", TargetAction::Modify, PreserveMode::Copy);
        target.after = None;
        assert!(expected_after(&target, TargetRole::Whole).is_empty());
        assert_eq!(
            addresses(&target, TargetRole::Whole),
            [TargetPath::Fs(abs("/home/user/a"))]
        );
    }

    #[test]
    fn addresses_are_shown_to_the_language_as_paths_and_provider_keys() {
        assert_eq!(
            display_path(&TargetPath::Fs(abs("/home/użytkownik/zażółć.txt"))),
            "/home/użytkownik/zażółć.txt"
        );
        let key = |segments: &[&str], value: Option<&str>| {
            TargetPath::Provider(ProviderKey {
                scheme: ProviderScheme("registry".to_owned()),
                segments: segments.iter().map(|part| (*part).to_owned()).collect(),
                value: value.map(str::to_owned),
            })
        };
        assert_eq!(
            display_path(&key(&["HKCU", "Software"], None)),
            "registry:HKCU/Software"
        );
        assert_eq!(
            display_path(&key(&["HKCU", "Software"], Some("Ścieżka"))),
            "registry:HKCU/Software@Ścieżka"
        );
        assert_eq!(display_path(&key(&[], None)), "registry:");
    }

    #[test]
    fn every_known_field_of_an_expected_state_is_compared() {
        let a = hash_of(b"a");
        let expected = present(a);
        let symlink = Observed {
            kind: Some(FileKind::Symlink),
            ..file(a)
        };
        assert_eq!(expected.classify(&symlink), Some(ConflictKind::TypeChanged));
        let resized = Observed {
            size: Some(2),
            ..file(a)
        };
        assert_eq!(expected.classify(&resized), Some(ConflictKind::Modified));
        let directory = Observed {
            exists: true,
            kind: Some(FileKind::Directory),
            ..Observed::default()
        };
        assert_eq!(
            Expected::Absent.classify(&directory),
            Some(ConflictKind::Created)
        );
        let unknown_kind = Observed {
            exists: true,
            ..Observed::default()
        };
        assert_eq!(expected.classify(&unknown_kind), None);
    }

    #[test]
    fn recorded_states_before_and_after_decide_the_expected_state() {
        let hash = hash_of(b"x");
        let before = BeforeState {
            exists: true,
            kind: Some(FileKind::File),
            size: Some(1),
            hash: Some(hash),
            mode: Some(0o600),
        };
        let expected = Expected::before(&before);
        assert_eq!(expected.hash(), Some(hash));
        assert_eq!(expected.to_expect(), Expect::Hash(hash));
        let after = AfterState {
            exists: true,
            hash: None,
            ..AfterState::default()
        };
        assert_eq!(Expected::after(&after).to_expect(), Expect::Unchecked);
        assert_eq!(Expected::Absent.hash(), None);
    }

    #[test]
    fn address_check_requests_the_hash_only_when_one_is_expected() {
        let a = hash_of(b"a");
        let b = hash_of(b"b");
        let path = TargetPath::Fs(abs("/home/user/a"));
        let probe = Probe::new(file(b));
        let conflict = check_address(&probe, OpId(4), &path, &present(a))
            .unwrap()
            .unwrap();
        assert_eq!(conflict.op, OpId(4));
        assert_eq!(conflict.kind, ConflictKind::Modified);
        assert_eq!((conflict.expected, conflict.actual), (Some(a), Some(b)));
        let loose = Expected::Present {
            kind: None,
            size: None,
            mode: None,
            hash: None,
        };
        assert!(
            check_address(&probe, OpId(4), &path, &loose)
                .unwrap()
                .is_none()
        );
        assert!(
            check_address(&probe, OpId(4), &path, &present(b))
                .unwrap()
                .is_none()
        );
        assert_eq!(*probe.hash_requests.borrow(), [true, false, true]);
    }

    #[test]
    fn preserved_content_is_checked_only_for_kept_files() {
        let recorded = hash_of(b"before");
        let target = outcome(3, "/home/user/a", TargetAction::Delete, PreserveMode::Moved);
        let mut hashes = BlobHashes::new();
        hashes.insert((OpId(1), TargetIndex(3)), recorded);
        assert!(check_blob(&hashes, OpId(1), &target).unwrap().is_none());

        hashes.insert((OpId(1), TargetIndex(3)), hash_of(b"changed"));
        let conflict = check_blob(&hashes, OpId(1), &target).unwrap().unwrap();
        assert_eq!(conflict.kind, ConflictKind::BlobModified);
        assert_eq!(conflict.expected, Some(recorded));
        assert_eq!(conflict.actual, Some(hash_of(b"changed")));
        assert_eq!(conflict.path, target.path);

        let missing = check_blob(&BlobHashes::new(), OpId(1), &target).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::JournalCorrupt);

        for mode in [PreserveMode::Copy, PreserveMode::Clone, PreserveMode::None] {
            let private = outcome(3, "/home/user/a", TargetAction::Delete, mode);
            assert!(
                check_blob(&BlobHashes::new(), OpId(1), &private)
                    .unwrap()
                    .is_none()
            );
        }
        let mut directory = outcome(3, "/home/user/d", TargetAction::Delete, PreserveMode::Moved);
        directory.before.kind = Some(FileKind::Directory);
        assert!(
            check_blob(&BlobHashes::new(), OpId(1), &directory)
                .unwrap()
                .is_none()
        );
        let mut unhashed = outcome(3, "/home/user/a", TargetAction::Delete, PreserveMode::Moved);
        unhashed.before.hash = None;
        assert!(
            check_blob(&BlobHashes::new(), OpId(1), &unhashed)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn preserved_hashes_are_read_once_per_entry_for_checked_targets_only() {
        let first = outcome(0, "/home/user/a", TargetAction::Delete, PreserveMode::Moved);
        let copied = outcome(1, "/home/user/b", TargetAction::Modify, PreserveMode::Copy);
        let second = outcome(
            5,
            "/home/user/c",
            TargetAction::Modify,
            PreserveMode::Exchange,
        );
        let mut probe = Probe::new(Observed::default());
        probe.blobs.insert(TargetIndex(0), hash_of(b"zero"));
        probe.blobs.insert(TargetIndex(1), hash_of(b"one"));
        probe.blobs.insert(TargetIndex(5), hash_of(b"five"));
        let hashes = preserved_hashes(
            &probe,
            [(OpId(2), &second), (OpId(1), &first), (OpId(1), &copied)],
        )
        .unwrap();
        assert_eq!(hashes.len(), 2);
        assert_eq!(hashes[&(OpId(1), TargetIndex(0))], hash_of(b"zero"));
        assert_eq!(hashes[&(OpId(2), TargetIndex(5))], hash_of(b"five"));
        assert_eq!(
            *probe.blob_calls.borrow(),
            [
                (OpId(1), vec![TargetIndex(0)]),
                (OpId(2), vec![TargetIndex(5)])
            ]
        );
        let none = Probe::new(Observed::default());
        assert!(
            preserved_hashes(&none, [(OpId(1), &copied)])
                .unwrap()
                .is_empty()
        );
        assert!(none.blob_calls.borrow().is_empty());
    }

    #[test]
    fn errors_name_the_entries_and_count_the_items() {
        let expired = expired_error(&[OpId(3), OpId(4)]);
        assert_eq!(expired.code, ErrorCode::JournalExpired);
        assert_eq!(expired.details["expired"], serde_json::json!([3, 4]));
        assert!(expired.message.starts_with("2 operations of the selection"));
        assert!(
            expired_error(&[OpId(3)])
                .message
                .starts_with("1 operation of")
        );

        let corrupt = corrupt_error(OpId(8));
        assert_eq!(corrupt.code, ErrorCode::JournalCorrupt);
        assert_eq!(corrupt.details["op"], 8);

        let conflict = |op: u64| Conflict {
            op: OpId(op),
            path: TargetPath::Fs(abs("/home/user/a")),
            kind: ConflictKind::Modified,
            expected: None,
            actual: None,
        };
        let one = conflict_error(&[conflict(1)], &[]);
        assert_eq!(one.code, ErrorCode::JournalConflict);
        assert!(one.message.starts_with("1 item changed"));
        let two = conflict_error(&[conflict(1), conflict(2)], &["undo".to_owned()]);
        assert!(two.message.starts_with("2 items changed"));
        assert_eq!(two.details["conflicts"].as_array().map(Vec::len), Some(2));

        let unknown = noop_error(OpId(5), None);
        assert!(unknown.message.contains("is not in the journal"));
        assert!(!unknown.details.contains_key("state"));
        let partial = noop_error(OpId(5), Some(EntryState::Partial));
        assert!(partial.message.contains("nothing left to restore"));
        assert_eq!(partial.details["state"], "partial");
    }

    #[test]
    fn an_entry_below_a_file_is_observed_as_absent() {
        let journal = TestJournal::new();
        journal.put("/home/user/a", b"abc");
        journal.with_fs(|fs| {
            let below = observe_fs(fs, &abs("/home/user/a/b/c"), true).unwrap();
            assert_eq!(below, Observed::default());
        });
    }
}
