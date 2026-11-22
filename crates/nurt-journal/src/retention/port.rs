// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Access of retention to the journal store.
//!
//! Retention decides which entries expire and records tombstones; the store
//! owns the directory layout, the entry state files and the operation index.
//! [`RetentionStore`] is the boundary between the two.

use nurt_contracts::config::ConfigLayer;
use nurt_contracts::env::{ClockStamp, WallTime};
use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, FileLock};
use nurt_contracts::journal::{EntryState, ExpiryReason, InactiveReason, OpId, OpKind};
use nurt_contracts::report::Irreversible;
use nurt_fs::failpoint::FailPoint;

use super::age::EntryTimeBasis;

/// A volume that has, or could have, a journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeFacts {
    /// Mount point.
    pub(crate) mount: AbsPath,
    /// Capacity in bytes (the account quota when smaller).
    pub(crate) capacity: u64,
    /// Free bytes available to the account (`F`), without the bytes held by
    /// the journal.
    pub(crate) free: u64,
    /// Why the journal is off on the volume; `None` when it is on.
    pub(crate) inactive: Option<InactiveReason>,
}

/// One part of an entry: the directory of the entry on one volume, under
/// `ops/` or, while it is reclaimed, under `reap/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryFacts {
    /// Operation number.
    pub(crate) op: OpId,
    /// Mount point of the volume.
    pub(crate) volume: AbsPath,
    /// Persistent state.
    pub(crate) state: EntryState,
    /// Reason recorded when the entry was moved to `reap/`.
    pub(crate) reason: Option<ExpiryReason>,
    /// Kind of the operation.
    pub(crate) kind: OpKind,
    /// Commit time as recorded in the manifest.
    pub(crate) time: WallTime,
    /// Clock readings at commit.
    pub(crate) basis: EntryTimeBasis,
    /// Number of items of the entry.
    pub(crate) files: u64,
    /// Logical bytes held.
    pub(crate) bytes_held: u64,
    /// Estimated physical bytes held, block sharing taken into account.
    pub(crate) bytes_physical: u64,
    /// Irreversible reasons recorded for the operation.
    pub(crate) irreversible: Vec<Irreversible>,
    /// The entry is pinned by `journal.pin`.
    pub(crate) pinned: bool,
    /// The operation was rolled back before it committed; reclaiming the
    /// entry leaves no tombstone.
    pub(crate) rolled_back: bool,
}

/// How an entry counts towards the step limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// A changing operation, including operations with only external effects
    /// and the undo entry created after a failed verification; one step.
    Operation,
    /// An undo or redo requested by the user; one step unless
    /// `journal.count_undo_as_op` is false.
    Undo,
    /// Journal maintenance that does not count (`journal.limits` with user
    /// scope).
    NotCounted,
}

/// A committed entry of the account, from the operation index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StepMark {
    /// Operation number.
    pub(crate) op: OpId,
    /// Step class.
    pub(crate) kind: StepKind,
}

/// Directory a retention file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Area<'a> {
    /// Journal directory of the volume with this mount point.
    Journal(&'a AbsPath),
    /// State directory of the account.
    State,
}

/// Journal store as seen by retention.
///
/// File names are relative to the area and use `/` as the separator.
pub(crate) trait RetentionStore {
    /// Configuration layers of the instance, including the project file.
    fn config_layers(&self) -> Vec<ConfigLayer>;

    /// Whether the project file is trusted.
    fn project_trusted(&self) -> bool;

    /// Current clock readings.
    fn clock(&self) -> ClockStamp;

    /// Volumes known to the journal, ordered by mount point.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the volume registry cannot be read.
    fn volumes(&self) -> Result<Vec<VolumeFacts>>;

    /// Volume containing `path`, including a volume whose journal does not
    /// exist yet; `None` when the path is on no known volume. The default
    /// implementation knows no such volume.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the volume cannot be inspected.
    fn volume_at(&self, _path: &AbsPath) -> Result<Option<VolumeFacts>> {
        Ok(None)
    }

    /// Entry parts of a volume in every state except `expired`.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the journal directory cannot be read.
    fn entries(&self, volume: &AbsPath) -> Result<Vec<EntryFacts>>;

    /// Committed entries of the account with their step class, including
    /// entries that expired later.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the operation index cannot be read.
    fn steps(&self) -> Result<Vec<StepMark>>;

    /// Takes the lock of entry `op` in the journal of `volume`
    /// (`lock/<N>.lock`); released on drop.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the lock cannot be taken in time.
    fn lock_entry(&self, volume: &AbsPath, op: OpId) -> Result<Box<dyn FileLock>>;

    /// Renames `ops/<N>` to `reap/<N>` in one operation and records the
    /// state `expiring` with `reason`.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the entry cannot be moved.
    fn begin_reap(&self, volume: &AbsPath, op: OpId, reason: ExpiryReason) -> Result<()>;

    /// Removes `reap/<N>` with its data; succeeds when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the data cannot be removed.
    fn remove_reaped(&self, volume: &AbsPath, op: OpId) -> Result<()>;

    /// Reports a fault injection point.
    ///
    /// # Errors
    ///
    /// Returns the injected error, or an error that stops the caller at a
    /// simulated crash.
    fn hit(&self, point: FailPoint) -> Result<()>;

    /// Content of a file; `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the file exists and cannot be read.
    fn read_file(&self, area: Area<'_>, name: &str) -> Result<Option<Vec<u8>>>;

    /// Replaces a file atomically and durably, creating its directory.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the file cannot be written.
    fn write_file(&self, area: Area<'_>, name: &str, content: &[u8]) -> Result<()>;

    /// Removes a file; succeeds when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the file cannot be removed.
    fn remove_file(&self, area: Area<'_>, name: &str) -> Result<()>;

    /// Names of the files in a directory; empty when it does not exist.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the directory cannot be read.
    fn list_dir(&self, area: Area<'_>, dir: &str) -> Result<Vec<String>>;

    /// Appends one audit record to the retention audit log of the account.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the log cannot be written.
    fn audit(&self, record: &serde_json::Value) -> Result<()>;

    /// Content of the user configuration file; `None` when it does not
    /// exist.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the file exists and cannot be read.
    fn read_user_config(&self) -> Result<Option<String>>;

    /// Writes the user configuration file through a mutation transaction of
    /// kind `journal.limits` that does not count as a step.
    ///
    /// # Errors
    ///
    /// Returns `E_IO` when the file cannot be written.
    fn write_user_config(&self, content: &str) -> Result<OpId>;
}
