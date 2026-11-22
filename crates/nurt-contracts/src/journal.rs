// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Operation journal: the only path for changing user data.
//!
//! Every change made by a command is recorded in the operation journal and
//! can be undone individually:
//!
//! * [`MutationTx`] is the only path through which a command changes data:
//!   [`MutationTx::preserve`] keeps the previous state of every target before
//!   the first change, [`MutationTx::apply`] performs the changes and
//!   [`MutationTx::commit`] makes them durable and undoable;
//!   [`MutationTx::rollback`] restores the previous state.
//! * [`JournalStore`] opens transactions and serves `history`, `undo`, `redo`
//!   and the `journal.*` commands.
//! * [`Admission`] decides, per target, whether the journal may keep the
//!   previous content without endangering free space for the user.
//!
//! At every instant of a crash the previous content exists in at least one
//! place; [`JournalStore::recover`] completes or reverts interrupted entries
//! before the first new operation is admitted.
//!
//! The journal never fails a user operation: [`JournalStore::begin`] is
//! infallible, and when the previous state cannot be kept the operation
//! proceeds with [`PreserveMode::None`] and an explicit irreversible marker.
//! Results carry no expiry time; retention state is reported only by
//! [`JournalStore::status`].
//!
//! Methods return the language error [`crate::error::Error`]. The protocol,
//! the error details and the reference semantics of test implementations are
//! specified in the journal specification of the repository.

use std::path::PathBuf;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::bound::BoundDocument;
use crate::config::{
    AgeClock, AgeLimit, ByteLimit, ConfigLevel, JournalFloor, Oversize, SizeLimit, StepLimit,
};
use crate::env::WallTime;
use crate::error::Result;
use crate::fs::{
    AbsPath, ContentHash, Etag, FileKind, FileMeta, LinkKind, MetaChange, ProviderKey,
    ProviderValue, VolumeId, VolumeInfo, WriteHandle,
};
use crate::report::{Correction, Irreversible, IrreversibleEntry};

pub use crate::record::UndoScope;

/// One entry of `history`: the `operation` record of the language.
///
/// The journal fills it from its entry: the number, the kind split into
/// `kind` (command) and `action`, hashes in the text form `sha256:<hex>`. It
/// contains no expiry time and nothing from which one can be derived.
pub type OperationRecord = crate::record::Operation;

/// State of an operation as shown to the language (`operation.state`).
pub type OpState = crate::record::OperationState;

impl crate::record::OperationState {
    /// Language state for a persistent entry state.
    ///
    /// Entries in transient states (prepared, rolling back) are not listed
    /// and yield `None`; expiring and corrupt entries are shown as expired.
    ///
    /// ```
    /// use nurt_contracts::journal::{EntryState, OpState};
    ///
    /// assert_eq!(OpState::from_entry(EntryState::Corrupt), Some(OpState::Expired));
    /// assert_eq!(OpState::from_entry(EntryState::Prepared), None);
    /// ```
    #[must_use]
    pub fn from_entry(state: EntryState) -> Option<Self> {
        match state {
            EntryState::Committed => Some(Self::Available),
            EntryState::Partial => Some(Self::Partial),
            EntryState::Undone => Some(Self::Undone),
            EntryState::Expiring | EntryState::Expired | EntryState::Corrupt => Some(Self::Expired),
            EntryState::Prepared | EntryState::RollingBack => None,
        }
    }
}

/// Operation number: one sequence per account, starting at 1.
///
/// An operation spanning several volumes has one number and one entry part
/// per volume. Reads and previews do not consume numbers. Numbers are unique
/// but not necessarily dense: a transaction that records nothing releases no
/// number back.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct OpId(pub u64);

/// Session identifier from the request envelope.
///
/// Recorded in entries; it scopes `undo to:`, the default selection of
/// `undo` and session-scoped limits.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SessionId(pub String);

/// Kind of an operation: the catalog command name and, for commands with
/// several mutating actions, the action (for example `git.stash` and `push`).
///
/// The catalog is the source of the admissible values; the journal stores
/// them verbatim. Internal kinds are produced by [`OpKind::undo`] (also used
/// for the automatic undo after a failed `verify`, with
/// [`UndoneCause::VerifyFailed`] on the source entry) and [`OpKind::redo`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct OpKind {
    /// Command name as in the catalog (`write`, `remove`, `git.stash`).
    pub command: String,
    /// Mutating action or mode (`overwrite`, `push`), when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl OpKind {
    /// Kind of a catalog command without action.
    pub fn command(name: &str) -> Self {
        Self {
            command: name.to_owned(),
            action: None,
        }
    }

    /// Kind of a catalog command with an action.
    pub fn with_action(name: &str, action: &str) -> Self {
        Self {
            command: name.to_owned(),
            action: Some(action.to_owned()),
        }
    }

    /// Kind of an undo entry.
    pub fn undo() -> Self {
        Self::command("undo")
    }

    /// Kind of a redo entry.
    pub fn redo() -> Self {
        Self::command("redo")
    }
}

/// Persistent state of a journal entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EntryState {
    /// The entry is written; previous content is being preserved or changes
    /// are being applied. After a crash the entry is rolled back.
    Prepared,
    /// Changes are applied and durable; the entry can be undone.
    Committed,
    /// Rollback is in progress; recovery completes it idempotently.
    RollingBack,
    /// Some items are no longer restorable: released by space priority, or
    /// undone individually (`undo file:`, `undo N paths:`). The remaining
    /// items stay restorable.
    Partial,
    /// Undone completely (by `undo`, rollback, crash recovery or failed
    /// verification; see [`UndoneCause`]).
    Undone,
    /// Being reclaimed; data moved to the reclaim area.
    Expiring,
    /// Reclaimed; only a tombstone with metrics remains.
    Expired,
    /// The entry failed its integrity check; data is kept for manual salvage.
    Corrupt,
}

/// Why an entry reached [`EntryState::Undone`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UndoneCause {
    /// Undone by an explicit `undo`.
    Undo,
    /// Rolled back after a failure of a target during apply.
    RolledBack,
    /// Rolled back by crash recovery.
    Crash,
    /// Rolled back because `verify` failed.
    VerifyFailed,
    /// Rolled back because validation found new errors.
    ValidateFailed,
}

/// Why an entry expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExpiryReason {
    /// Step limit exceeded.
    Ops,
    /// Age limit exceeded.
    Time,
    /// Size budget exceeded.
    Size,
    /// Reclaimed because the user needed space.
    Space,
    /// Released by `journal.purge`.
    Purge,
}

/// How the previous state of a target is kept.
///
/// The serialized names are the values of the `store` field of journal
/// entries and of the coverage table of the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PreserveMode {
    /// The entry was moved into the journal (deletion, overwritten move
    /// destination).
    Moved,
    /// The old content was kept by atomic exchange of names (overwrite).
    Exchange,
    /// The old content was kept under the backup name of `ReplaceFileW`
    /// (Windows overwrite).
    ReplaceBackup,
    /// The old content was kept by a hard link before replacement
    /// (overwrite).
    Hardlink,
    /// The old content was kept by a block-sharing clone.
    Clone,
    /// The old content was kept by a byte copy.
    Copy,
    /// The previous content was not kept; accompanied by an irreversible
    /// marker.
    None,
    /// The target did not exist; undo removes it (into the undo entry).
    Created,
    /// Only metadata changed; the previous metadata was recorded.
    Meta,
    /// The previous reference value was recorded (repository references and
    /// index).
    Ref,
    /// The previous provider value was recorded (registry, environment).
    Value,
    /// The previous length and the hash of the prefix were recorded; undo
    /// truncates.
    Append,
    /// Rename within a volume: source and destination were recorded.
    Rename,
    /// Effect outside the file system (network, process, service, package);
    /// nothing is kept and the target always has an irreversible marker.
    External,
}

impl PreserveMode {
    /// Whether the previous state is kept (every mode except
    /// [`PreserveMode::None`] and [`PreserveMode::External`]).
    pub fn is_preserved(self) -> bool {
        !matches!(self, Self::None | Self::External)
    }

    /// Stable serialized name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Moved => "moved",
            Self::Exchange => "exchange",
            Self::ReplaceBackup => "replace_backup",
            Self::Hardlink => "hardlink",
            Self::Clone => "clone",
            Self::Copy => "copy",
            Self::None => "none",
            Self::Created => "created",
            Self::Meta => "meta",
            Self::Ref => "ref",
            Self::Value => "value",
            Self::Append => "append",
            Self::Rename => "rename",
            Self::External => "external",
        }
    }
}

/// Address of a target.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "kind", content = "at", rename_all = "snake_case")]
pub enum TargetPath {
    /// File system entry.
    Fs(AbsPath),
    /// Provider key or value (for example a registry value).
    Provider(ProviderKey),
}

/// What the operation will do to a target. Matches the `change.action` field
/// of reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TargetAction {
    /// Create a new entry that must not exist.
    Create,
    /// Create a new directory populated by the operation (for example a
    /// repository clone or a new worktree). Writes inside it are not declared
    /// individually; undo moves the whole tree into the undo entry.
    CreateTree,
    /// Replace the content of an existing file, or create it when absent.
    Modify,
    /// Append to the end of a file.
    Append,
    /// Delete an entry (file, directory tree, link, provider key). A
    /// directory tree is one target, preserved by a single rename.
    Delete,
    /// Replace an entry by an entry of another kind (file, directory, link);
    /// the previous entry is preserved as a whole.
    Replace,
    /// Rename or move to `to`; an existing destination is overwritten when
    /// `replace` is true and is then preserved as well. Across volumes the
    /// journal copies and deletes, preserving the source in the journal of
    /// its own volume.
    Rename {
        /// Destination.
        to: AbsPath,
        /// Overwrite an existing destination.
        replace: bool,
    },
    /// Change metadata only.
    Attributes,
    /// Create a link.
    Link,
    /// Change the length of a file.
    Truncate,
    /// Content will be written by a library outside the file system layer
    /// (embedded database engine, repository index writer) or by a launched
    /// program. The journal preserves the full previous content by clone or
    /// copy; the after-state is recorded by [`MutationTx::observe_external`].
    ExternalWrite,
}

/// State a target must have immediately before it is changed.
///
/// A mismatch yields `E_STALE` and rolls the transaction back. The check is
/// made by the journal atomically with the change, without a window between
/// check and switch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Expect {
    /// No check (the stage did not read the target).
    #[default]
    Unchecked,
    /// The target must not exist.
    Absent,
    /// The target must match the entity tag observed by the read stage.
    Etag(Etag),
    /// The content must have this hash.
    Hash(ContentHash),
}

/// Declaration of one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TargetSpec {
    /// Address.
    pub path: TargetPath,
    /// Intended change.
    pub action: TargetAction,
    /// Expected state before the change.
    #[serde(default)]
    pub expect: Expect,
    /// Size of the previous content, when already known (avoids a `stat` and
    /// feeds admission).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_hint: Option<u64>,
}

/// Index of a target within its transaction, in declaration order across all
/// calls of [`MutationTx::preserve`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct TargetIndex(pub u32);

/// State of a target before the change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BeforeState {
    /// The entry existed.
    pub exists: bool,
    /// Kind of the entry.
    pub kind: Option<FileKind>,
    /// Size of the entry.
    pub size: Option<u64>,
    /// Content hash; may be computed lazily (for example only when undone)
    /// for moved or exchanged targets of bulk operations.
    pub hash: Option<ContentHash>,
    /// Permission bits.
    pub mode: Option<u32>,
}

/// Observed state of a target after the change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AfterState {
    /// The entry exists after the change.
    pub exists: bool,
    /// Kind of the entry.
    pub kind: Option<FileKind>,
    /// Size after the change.
    pub size: Option<u64>,
    /// Content hash after the change, used by the undo conflict check.
    pub hash: Option<ContentHash>,
    /// Permission bits after the change.
    pub mode: Option<u32>,
    /// Metadata of the entry read after the change, when the journal had to
    /// read it anyway (for example after a rename into place); callers that
    /// record file status, such as a repository index, use it instead of a
    /// second `stat`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<FileMeta>,
}

/// Unusual effect of journal handling, shown in the report as a notable
/// effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum JournalNotable {
    /// A file with several hard links was written in place so that all names
    /// see the new content.
    HardlinkSetPreserved {
        /// File.
        path: TargetPath,
    },
    /// The operation exceeds the size budget (shown by preview before the
    /// change).
    JournalOversize {
        /// Logical size of the operation in bytes.
        size: u64,
        /// Effective limit in bytes.
        limit: u64,
    },
    /// A project configuration file tried to change journal settings without
    /// being trusted; ignored.
    JournalConfigIgnored {
        /// Project file.
        file: String,
    },
    /// A project configuration file tried to declare trust; ignored.
    TrustConfigIgnored {
        /// Project file.
        file: String,
    },
    /// A value below the protection floor was raised to the floor.
    RaisedToFloor {
        /// Setting name.
        setting: String,
    },
    /// Previous content was kept by copy because a rename crossed layers or
    /// volumes.
    CrossDeviceCopy {
        /// Item.
        path: TargetPath,
    },
    /// Retained data is shared with other names and counted as zero bytes.
    SharedData {
        /// Item.
        path: TargetPath,
    },
}

/// Result of preserving one target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Preserved {
    /// Target index.
    pub index: TargetIndex,
    /// Volume of the target (file system targets).
    pub volume: Option<VolumeId>,
    /// Strategy used.
    pub mode: PreserveMode,
    /// Logical bytes retained by the journal for this target.
    pub bytes_held: u64,
    /// State before the change.
    pub before: BeforeState,
    /// Reason, present when the previous state is not kept.
    pub irreversible: Option<Irreversible>,
    /// Unusual effects of the strategy.
    pub notable: Vec<JournalNotable>,
}

/// Clone policy of [`Mutation::CopyFrom`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CloneMode {
    /// Clone when the volume supports it, otherwise copy bytes.
    #[default]
    Auto,
    /// Clone or fail with `E_IO`.
    Always,
    /// Always copy bytes.
    Never,
}

/// A change applied to a declared target.
///
/// Content mutations transfer the metadata of the previous file (mode,
/// account and group of the file, extended attributes, ACL) to the new
/// content; the caller never transfers metadata itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Mutation {
    /// Switch in the file previously obtained from [`MutationTx::stage`] and
    /// filled by the caller (streamed content).
    CommitStaged,
    /// Write the given bytes as the complete new content.
    WriteBytes(Vec<u8>),
    /// Append bytes.
    AppendBytes(Vec<u8>),
    /// Write bytes at an offset, keeping the rest of the content.
    WriteAt {
        /// Offset in bytes.
        offset: u64,
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// Change the length of the file.
    SetLength(u64),
    /// Copy the content of `source`.
    CopyFrom {
        /// Source file.
        source: AbsPath,
        /// Clone policy.
        clone: CloneMode,
    },
    /// Delete the target (recursively for directories).
    Remove,
    /// Perform the rename declared by [`TargetAction::Rename`].
    Rename,
    /// Create a directory.
    CreateDir {
        /// Permission bits.
        mode: u32,
    },
    /// Create a symbolic link or junction.
    CreateLink {
        /// Link target as stored in the link.
        target: PathBuf,
        /// Link kind.
        kind: LinkKind,
    },
    /// Create a hard link to `existing`.
    CreateHardLink {
        /// Existing file.
        existing: AbsPath,
    },
    /// Change metadata.
    SetMeta(MetaChange),
    /// Set a provider value.
    ProviderSet(ProviderValue),
    /// Remove a provider key or value.
    ProviderRemove,
}

/// Result of applying one mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Applied {
    /// Target index.
    pub index: TargetIndex,
    /// State after the change.
    pub after: AfterState,
}

/// Kind of a change observed after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalChangeKind {
    /// The entry was created.
    Created,
    /// The content or metadata of the entry changed.
    Modified,
    /// The entry was deleted.
    Deleted,
}

/// Change made outside the file system layer, observed after the fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExternalChange {
    /// Changed entry.
    pub path: AbsPath,
    /// Kind of the change.
    pub kind: ExternalChangeKind,
    /// Declared target index, when the entry was declared with
    /// [`TargetAction::ExternalWrite`]. Changes detected by scanning after a
    /// launched program have no index; they are recorded as irreversible
    /// unless a previous content was preserved for the same path.
    pub declared: Option<TargetIndex>,
}

/// Write performed outside the journal by design and never undone.
///
/// Recorded in the entry for audit and excluded from the mutation audit of
/// test builds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExemptWrite {
    /// Content-addressed objects added to the object store of a repository
    /// (loose objects, packs and their indexes). They are immutable and stay
    /// in the repository; references to them are journaled.
    ObjectStore {
        /// Repository directory.
        repo: AbsPath,
    },
}

/// Why a transaction or committed entry is rolled back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RollbackReason {
    /// A target failed during apply.
    TargetFailed,
    /// The operation was cancelled.
    Cancelled,
    /// The `verify` stage failed.
    VerifyFailed,
    /// Validation or diagnostics found new errors after the change.
    ValidateFailed,
}

/// Result of a rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RollbackReport {
    /// Operation rolled back; `None` when no number was assigned yet.
    pub op: Option<OpId>,
    /// Targets restored.
    pub restored: Vec<TargetPath>,
    /// Targets that could not be restored; a non-empty list is reported as
    /// `E_ROLLBACK_FAILED` by the caller.
    pub failed: Vec<TargetPath>,
}

/// Outcome of one target of a committed operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TargetOutcome {
    /// Target index.
    pub index: TargetIndex,
    /// Address.
    pub path: TargetPath,
    /// Declared action.
    pub action: TargetAction,
    /// Strategy used.
    pub mode: PreserveMode,
    /// Logical bytes retained.
    pub bytes_held: u64,
    /// State before the change.
    pub before: BeforeState,
    /// State after the change; `None` for declared external targets that did
    /// not change.
    pub after: Option<AfterState>,
    /// Reason, present when the previous state is not kept.
    pub irreversible: Option<Irreversible>,
    /// Unusual effects.
    pub notable: Vec<JournalNotable>,
}

/// Handle of a committed operation, consumed by the report builder.
///
/// It contains no retention information.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OpHandle {
    /// Operation number; `None` when the transaction changed nothing and
    /// marked nothing irreversible, in which case no entry exists.
    pub op: Option<OpId>,
    /// Kind.
    pub kind: OpKind,
    /// Bulk operation, undone as a whole.
    pub bulk: bool,
    /// Per-target outcome, in declaration order. Declared external targets
    /// that did not change are omitted.
    pub targets: Vec<TargetOutcome>,
    /// Aggregated irreversible markers (target and operation level).
    pub irreversible: Vec<IrreversibleEntry>,
    /// Unusual effects to show in the report.
    pub notable: Vec<JournalNotable>,
    /// Writes exempt from the journal by design.
    pub exempt: Vec<ExemptWrite>,
}

/// The only path through which commands change data.
///
/// The executor opens the transaction, passes it to the effect stage as
/// `&mut dyn MutationTx`, and commits or rolls it back after the stage
/// returns; the stage declares, preserves and applies, and never commits.
///
/// A transaction is driven by one thread at a time; `Sync` is required so
/// that it can be referenced from the execution context. Methods fail only
/// because of the targets or cancellation; journal failures degrade to
/// [`PreserveMode::None`] with an irreversible marker. After a method fails
/// the transaction is rolled back and every further call fails with
/// `E_ROLLED_BACK`.
///
/// Misuse of the protocol (preserve after the first change, an undeclared
/// target, a mutation that does not match the declared action) is a defect of
/// the caller: test builds panic, production builds fail with
/// `E_STAGE_FAILED` and roll back.
pub trait MutationTx: Send + Sync {
    /// Operation number, assigned at the first [`MutationTx::preserve`] or,
    /// for transactions without targets, at commit.
    fn op(&self) -> Option<OpId>;

    /// Declares targets and preserves their previous state before any change.
    ///
    /// Precondition: no change has been applied in this transaction. Indexes
    /// continue across calls in declaration order; the result has one
    /// [`Preserved`] per declared target, in order.
    ///
    /// After a crash during or after this call, recovery rolls the entry
    /// back; the previous state of every target is intact or restorable.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when a target that must exist does not,
    /// `E_EXISTS` when a target declared with [`TargetAction::Create`] exists,
    /// `E_PERMISSION`, `E_IN_USE` or `E_IO` for the target itself, and
    /// `E_CANCELLED` when the operation was cancelled. Lack of space for the
    /// journal never fails: the target gets [`PreserveMode::None`].
    fn preserve(&mut self, targets: &[TargetSpec]) -> Result<Vec<Preserved>>;

    /// Opens a staged file in the directory of a declared target for writing
    /// new content; [`Mutation::CommitStaged`] switches it in.
    ///
    /// Precondition: the target was declared with an action that writes
    /// content. The staged file is removed by rollback and by recovery.
    ///
    /// # Errors
    ///
    /// Fails with `E_NO_SPACE`, `E_PERMISSION` or `E_IO` for the target
    /// directory.
    fn stage(&mut self, target: TargetIndex) -> Result<Box<dyn WriteHandle>>;

    /// Applies a change to a declared target.
    ///
    /// Precondition: the target was declared and the mutation matches the
    /// declared action. The expected state ([`Expect`]) is checked atomically
    /// with the change.
    ///
    /// After a crash, recovery restores the previous state of every target
    /// changed so far.
    ///
    /// # Errors
    ///
    /// Fails with `E_STALE` when the expected state does not match, and with
    /// `E_NOT_FOUND`, `E_PERMISSION`, `E_IN_USE`, `E_NO_SPACE`, `E_IO` or
    /// `E_CANCELLED`. On failure the transaction is rolled back before the
    /// error is returned; when the rollback is incomplete the error is
    /// `E_ROLLBACK_FAILED` with the targets not restored.
    fn apply(&mut self, target: TargetIndex, mutation: Mutation) -> Result<Applied>;

    /// Records changes made outside the file system layer: declared
    /// [`TargetAction::ExternalWrite`] targets and changes detected after a
    /// launched program.
    ///
    /// Declared external targets not reported here are treated as unchanged
    /// at commit: they are omitted from the entry and their preserved copies
    /// are released.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the after-state of a reported entry cannot be
    /// read; the transaction is then rolled back.
    fn observe_external(&mut self, changes: &[ExternalChange]) -> Result<Vec<Applied>>;

    /// Marks an operation-level irreversible effect that has no file target
    /// (network request, stopped process, changed service, installed
    /// package). A transaction may consist of markers only.
    fn mark_irreversible(&mut self, marker: IrreversibleEntry);

    /// Records a write that is exempt from the journal by design.
    fn record_exempt(&mut self, write: ExemptWrite);

    /// Makes the operation durable and undoable and returns its handle.
    ///
    /// After a crash before this call returns, the entry is either committed
    /// or rolled back by recovery, never half applied.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the targets cannot be made durable; the
    /// transaction is then rolled back. A failure to update the journal
    /// index after the entry became durable does not fail the commit.
    fn commit(self: Box<Self>) -> Result<OpHandle>;

    /// Restores all targets changed so far, in reverse order, and removes
    /// staged files.
    ///
    /// Rollback is idempotent and is completed by recovery after a crash.
    /// Targets that cannot be restored are listed in
    /// [`RollbackReport::failed`]; an error is returned only when the entry
    /// itself cannot be read (`E_JOURNAL_CORRUPT`).
    fn rollback(self: Box<Self>, reason: RollbackReason) -> Result<RollbackReport>;
}

/// A file system change described at the level of a command: the declaration
/// of the target and the mutation that performs the change.
///
/// The constructors pair the declared action with the matching mutation, so
/// a command does not choose preserve modes, the order of the protocol or the
/// strategy across volumes. [`apply_ops`] runs a list of operations as one
/// protocol pass. Streamed content uses [`MutationTx::stage`] and
/// [`Mutation::CommitStaged`] directly after [`MutationTx::preserve`].
///
/// ```
/// use nurt_contracts::fs::AbsPath;
/// use nurt_contracts::journal::{Expect, FileOp, Mutation, TargetAction};
///
/// let path = AbsPath::new("/home/user/notes.txt").unwrap();
/// let op = FileOp::write(path, b"new text".to_vec()).expect(Expect::Absent);
/// assert_eq!(op.target.action, TargetAction::Modify);
/// assert_eq!(op.target.expect, Expect::Absent);
/// assert_eq!(op.mutation, Mutation::WriteBytes(b"new text".to_vec()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOp {
    /// Declaration of the target.
    pub target: TargetSpec,
    /// Change applied after every target of the pass is preserved.
    pub mutation: Mutation,
}

impl FileOp {
    fn new(path: AbsPath, action: TargetAction, mutation: Mutation) -> Self {
        Self {
            target: TargetSpec {
                path: TargetPath::Fs(path),
                action,
                expect: Expect::Unchecked,
                size_hint: None,
            },
            mutation,
        }
    }

    /// Replaces the content of a file, creating it when absent.
    #[must_use]
    pub fn write(path: AbsPath, content: Vec<u8>) -> Self {
        Self::new(path, TargetAction::Modify, Mutation::WriteBytes(content))
    }

    /// Creates a file that must not exist.
    #[must_use]
    pub fn create(path: AbsPath, content: Vec<u8>) -> Self {
        Self::new(path, TargetAction::Create, Mutation::WriteBytes(content)).expect(Expect::Absent)
    }

    /// Appends bytes to a file; undo truncates it to the previous length.
    #[must_use]
    pub fn append(path: AbsPath, bytes: Vec<u8>) -> Self {
        Self::new(path, TargetAction::Append, Mutation::AppendBytes(bytes))
    }

    /// Writes bytes at an offset, keeping the rest of the content.
    #[must_use]
    pub fn write_at(path: AbsPath, offset: u64, bytes: Vec<u8>) -> Self {
        Self::new(
            path,
            TargetAction::Modify,
            Mutation::WriteAt { offset, bytes },
        )
    }

    /// Changes the length of a file.
    #[must_use]
    pub fn set_length(path: AbsPath, length: u64) -> Self {
        Self::new(path, TargetAction::Truncate, Mutation::SetLength(length))
    }

    /// Removes a file, a link or a directory tree (one target).
    #[must_use]
    pub fn remove(path: AbsPath) -> Self {
        Self::new(path, TargetAction::Delete, Mutation::Remove)
    }

    /// Renames or moves an entry, also across volumes; an existing
    /// destination is overwritten and preserved when `replace` is true.
    #[must_use]
    pub fn rename(from: AbsPath, to: AbsPath, replace: bool) -> Self {
        Self::new(from, TargetAction::Rename { to, replace }, Mutation::Rename)
    }

    /// Copies the content of `source` into a new file.
    #[must_use]
    pub fn copy(source: AbsPath, to: AbsPath, clone: CloneMode) -> Self {
        Self::new(
            to,
            TargetAction::Create,
            Mutation::CopyFrom { source, clone },
        )
        .expect(Expect::Absent)
    }

    /// Copies the content of `source` over an existing file, creating it
    /// when absent.
    #[must_use]
    pub fn copy_over(source: AbsPath, to: AbsPath, clone: CloneMode) -> Self {
        Self::new(
            to,
            TargetAction::Modify,
            Mutation::CopyFrom { source, clone },
        )
    }

    /// Creates a directory.
    #[must_use]
    pub fn create_dir(path: AbsPath, mode: u32) -> Self {
        Self::new(path, TargetAction::Create, Mutation::CreateDir { mode }).expect(Expect::Absent)
    }

    /// Creates a symbolic link or a junction.
    #[must_use]
    pub fn create_link(path: AbsPath, target: PathBuf, kind: LinkKind) -> Self {
        Self::new(
            path,
            TargetAction::Link,
            Mutation::CreateLink { target, kind },
        )
    }

    /// Creates a hard link to an existing file.
    #[must_use]
    pub fn create_hard_link(path: AbsPath, existing: AbsPath) -> Self {
        Self::new(
            path,
            TargetAction::Link,
            Mutation::CreateHardLink { existing },
        )
    }

    /// Changes metadata (times, permissions, attributes).
    #[must_use]
    pub fn set_meta(path: AbsPath, change: MetaChange) -> Self {
        Self::new(path, TargetAction::Attributes, Mutation::SetMeta(change))
    }

    /// Sets the state the target must have immediately before the change,
    /// for example the entity tag observed by the read stage.
    #[must_use]
    pub fn expect(mut self, expect: Expect) -> Self {
        self.target.expect = expect;
        self
    }

    /// Sets the size of the previous content, when already known.
    #[must_use]
    pub fn size_hint(mut self, size: u64) -> Self {
        self.target.size_hint = Some(size);
        self
    }
}

/// Outcome of one [`FileOp`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileOpOutcome {
    /// How the previous state was kept.
    pub preserved: Preserved,
    /// State after the change.
    pub applied: Applied,
}

/// Runs file operations as one protocol pass: declares and preserves every
/// target, then applies the mutations in order.
///
/// Precondition: no change has been applied in this transaction before the
/// call. Several calls in one transaction are allowed only when every target
/// is declared by the first call; otherwise use [`MutationTx::preserve`] for
/// all targets and [`MutationTx::apply`] for each.
///
/// # Errors
///
/// Fails with the errors of [`MutationTx::preserve`] and
/// [`MutationTx::apply`]; the transaction is then rolled back by the journal.
pub fn apply_ops(tx: &mut dyn MutationTx, ops: Vec<FileOp>) -> Result<Vec<FileOpOutcome>> {
    let targets: Vec<TargetSpec> = ops.iter().map(|op| op.target.clone()).collect();
    let preserved = tx.preserve(&targets)?;
    preserved
        .into_iter()
        .zip(ops)
        .map(|(preserved, op)| {
            let applied = tx.apply(preserved.index, op.mutation)?;
            Ok(FileOpOutcome { preserved, applied })
        })
        .collect()
}

/// Parameters of a new transaction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BeginRequest {
    /// Kind of the operation.
    pub kind: OpKind,
    /// Session from the request envelope.
    pub session: Option<SessionId>,
    /// Bound program that caused the operation, stored in the entry; the
    /// journal computes its canonical hash. Values of parameters marked
    /// secret in the catalog (credentials in URLs, authorization headers) are
    /// masked by the executor before the program is passed here.
    pub program: BoundDocument,
    /// Spelling corrections applied to the program.
    pub corrections: Vec<Correction>,
    /// More than one target: reported as a bulk operation and undone as a
    /// whole.
    pub bulk: bool,
    /// The entry counts towards the step limit. True for every user
    /// operation, including operations with only external effects; false only
    /// for journal maintenance that changes a user file (`journal.limits` with
    /// user scope writing the configuration file).
    pub counts_as_step: bool,
    /// Time budget for preserving targets declared with
    /// [`TargetAction::ExternalWrite`] (pre-image of launched programs). When
    /// exceeded, the remaining targets get [`PreserveMode::None`] and the
    /// operation proceeds.
    pub preserve_budget: Option<Duration>,
}

/// Point in history used by `since:`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Since {
    /// Operations with a number greater than this one.
    Op(OpId),
    /// Operations committed at or after this instant.
    Time(WallTime),
}

/// Which entries `undo` selects; exactly one form per call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UndoSelector {
    /// The newest available entry of the session (or of the account without a
    /// session) whose kind is not `undo`.
    Last,
    /// One entry, completely (`undo N`).
    Op(OpId),
    /// All available entries newer than the given one, newest first, as one
    /// transaction (`undo to: N`). Entries of other sessions are skipped and
    /// reported unless `all_sessions` is true. The conflict check covers the
    /// whole plan before the first write.
    To {
        /// Entries with a greater number are undone.
        after: OpId,
        /// Include entries of other sessions.
        all_sessions: bool,
    },
    /// The newest entries that touched one file, for that file only
    /// (`undo file:`). Source entries become partial.
    File {
        /// File.
        path: AbsPath,
        /// Lower bound; `None` undoes back to the oldest available entry.
        since: Option<Since>,
    },
    /// Selected items of one entry (`undo N paths:`). The entry becomes
    /// partial until all items are undone.
    Paths {
        /// Entry.
        op: OpId,
        /// Items.
        paths: Vec<TargetPath>,
    },
}

/// Behaviour when the undo check finds differences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    /// Change nothing and fail with `E_JOURNAL_CONFLICT` (or
    /// `E_JOURNAL_PARTIAL` for partial entries) listing the differences.
    #[default]
    Report,
    /// Undo anyway; the current content is preserved in the undo entry.
    Force,
    /// Also undo later entries touching the same items, newest first.
    IncludeLater,
    /// For partial entries, undo the remaining items and report the released
    /// ones.
    Partial,
    /// Change nothing; for partial entries fail with `E_JOURNAL_PARTIAL`.
    Abort,
}

/// Parameters of `undo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UndoRequest {
    /// Selected entries.
    pub selector: UndoSelector,
    /// Conflict behaviour.
    pub on_conflict: OnConflict,
    /// Session of the caller.
    pub session: Option<SessionId>,
    /// Bound program of the `undo` call.
    pub program: BoundDocument,
}

/// Parameters of `redo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RedoRequest {
    /// Undo entry to redo; `None` selects the latest undo of the session.
    pub undo_entry: Option<OpId>,
    /// Session of the caller.
    pub session: Option<SessionId>,
    /// Bound program of the `redo` call.
    pub program: BoundDocument,
}

/// Entry skipped by `undo to:`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedEntry {
    /// Entry.
    pub op: OpId,
    /// Session of the entry.
    pub session: Option<SessionId>,
}

/// Kind of difference found by the undo check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConflictKind {
    /// Content changed after the operation.
    Modified,
    /// Entry deleted after the operation.
    Deleted,
    /// Entry created where the operation left nothing.
    Created,
    /// Entry replaced by an entry of another kind.
    TypeChanged,
    /// Preserved content changed (a process kept the old file open and wrote
    /// to it after an exchange).
    BlobModified,
    /// A later entry touched the same item.
    LaterEntry {
        /// The later entry.
        op: OpId,
    },
}

/// One difference found by the undo check; carried in the details of
/// `E_JOURNAL_CONFLICT`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Conflict {
    /// Entry whose item conflicts.
    pub op: OpId,
    /// Item.
    pub path: TargetPath,
    /// Difference.
    pub kind: ConflictKind,
    /// Hash recorded by the journal.
    pub expected: Option<ContentHash>,
    /// Hash found now.
    pub actual: Option<ContentHash>,
}

/// Result of `undo` or `redo`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UndoOutcome {
    /// New entry recording the undo or redo.
    pub entry: OpId,
    /// Scope of the new entry.
    pub scope: UndoScope,
    /// Entries undone (or redone), newest first.
    pub sources: Vec<OpId>,
    /// Items restored.
    pub restored: Vec<TargetOutcome>,
    /// Items not restorable (released for space), reported for partial
    /// entries.
    pub unavailable: Vec<TargetPath>,
    /// Entries of other sessions skipped by `undo to:`.
    pub skipped: Vec<SkippedEntry>,
    /// Conflicts overridden by [`OnConflict::Force`].
    pub forced: Vec<Conflict>,
    /// Irreversible markers of the undo entry itself.
    pub irreversible: Vec<IrreversibleEntry>,
}

/// Filter of `history`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HistoryQuery {
    /// Single entry.
    pub id: Option<OpId>,
    /// Maximum number of entries, newest first.
    pub limit: Option<u32>,
    /// State filter; `None` means any.
    pub state: Option<OpState>,
    /// Entries older than this one.
    pub before: Option<OpId>,
    /// Entries newer than this one.
    pub since: Option<OpId>,
    /// Entries touching this path.
    pub path: Option<AbsPath>,
    /// Entries containing deletions only.
    pub deleted_only: bool,
}

/// One item of `history details: true`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ItemDetail {
    /// Address.
    pub path: TargetPath,
    /// Action.
    pub action: TargetAction,
    /// Strategy.
    pub mode: PreserveMode,
    /// Hash before.
    pub before_hash: Option<ContentHash>,
    /// Hash after.
    pub after_hash: Option<ContentHash>,
    /// Item already undone individually.
    pub undone: bool,
    /// Item released for space.
    pub released: bool,
}

/// Result of `history details: true` for one entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OperationDetails {
    /// Entry.
    pub record: OperationRecord,
    /// Items.
    pub items: Vec<ItemDetail>,
}

/// Parameters of `journal.pin`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PinRequest {
    /// Entries.
    pub ids: Vec<OpId>,
    /// Release instead of pin.
    pub release: bool,
    /// Pin duration; `None` uses the configured default.
    pub until: Option<Duration>,
}

/// Result of `journal.pin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PinOutcome {
    /// Entries pinned or released.
    pub changed: Vec<OpId>,
    /// Entries that were not available.
    pub not_available: Vec<OpId>,
}

/// Selection of `journal.purge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PurgeSelector {
    /// All entries of the volumes.
    All,
    /// Entries up to and including this number.
    UpTo(OpId),
    /// Exactly these entries, without releasing older ones.
    Ids(Vec<OpId>),
}

/// Parameters of `journal.purge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PurgeRequest {
    /// Volume; `None` means all volumes.
    pub volume: Option<AbsPath>,
    /// Selection.
    pub selector: PurgeSelector,
}

/// Result of `journal.purge`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PurgeOutcome {
    /// Entries released.
    pub released: Vec<OpId>,
    /// Logical bytes released.
    pub bytes: u64,
}

/// Why the journal holds no data on a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum InactiveReason {
    /// The space service is not running.
    ServiceDown,
    /// Network volume.
    NetworkVolume,
    /// Removable volume.
    RemovableVolume,
    /// Memory-backed volume.
    MemoryVolume,
    /// Free space below the reserve.
    BelowReserve,
    /// No writable anchor directory on the volume.
    NoAnchor,
    /// Disabled by configuration.
    Disabled,
    /// File system without durable metadata.
    NotDurable,
}

impl InactiveReason {
    /// Stable name used in `journal.status` and `journal.space`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServiceDown => "service_down",
            Self::NetworkVolume => "network_volume",
            Self::RemovableVolume => "removable_volume",
            Self::MemoryVolume => "memory_volume",
            Self::BelowReserve => "below_reserve",
            Self::NoAnchor => "no_anchor",
            Self::Disabled => "disabled",
            Self::NotDurable => "not_durable",
        }
    }
}

/// A value with the configuration level that determined it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sourced<T> {
    /// Effective value.
    pub value: T,
    /// Level that set it.
    pub source: ConfigLevel,
    /// The value was raised to the protection floor.
    pub raised_to_floor: bool,
}

/// Effective limits of a volume with their sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EffectiveRetention {
    /// Step limit.
    pub max_ops: Sourced<StepLimit>,
    /// Age limit.
    pub max_age: Sourced<AgeLimit>,
    /// Size budget as configured.
    pub max_size: Sourced<SizeLimit>,
    /// Size budget in bytes for this volume after percentage resolution and
    /// clamping; `None` when unlimited.
    pub max_size_bytes: Option<u64>,
    /// Oversize handling.
    pub oversize: Sourced<Oversize>,
    /// Age clock.
    pub age_clock: Sourced<AgeClock>,
    /// Single-operation limit.
    pub max_op_size: Sourced<ByteLimit>,
}

/// Oldest available entry of a volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OldestEntry {
    /// Entry.
    pub op: OpId,
    /// Commit time.
    pub time: WallTime,
}

/// Journal state of one volume (`journal_state` record of `journal.status`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JournalState {
    /// Volume mount point.
    pub volume: AbsPath,
    /// Entries, total.
    pub entries_total: u64,
    /// Entries available or partial.
    pub entries_available: u64,
    /// Tombstones of expired entries kept for `history`.
    pub entries_expired: u64,
    /// Logical bytes held.
    pub bytes_held: u64,
    /// Estimated physical bytes, block sharing taken into account.
    pub bytes_physical: u64,
    /// Oldest available entry.
    pub oldest: Option<OldestEntry>,
    /// Effective limits with their sources.
    pub limits: EffectiveRetention,
    /// Protection floor in force and the level that set it; `None` when no
    /// floor beyond the project minimum applies.
    pub floor: Option<Sourced<JournalFloor>>,
    /// Why the journal is off; `None` when it is on.
    pub inactive: Option<InactiveReason>,
    /// Entries currently over a limit and awaiting reclaim.
    pub over_limit: u64,
    /// Configuration notes (for example an untrusted project file ignored).
    pub notes: Vec<JournalNotable>,
}

/// Space report of one volume (`volume` record of `journal.space`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VolumeSpace {
    /// Mount point.
    pub mount: AbsPath,
    /// File system type.
    pub filesystem: String,
    /// Capacity.
    pub total: u64,
    /// Free bytes as seen by other programs.
    pub free: u64,
    /// Free bytes if the journal were empty.
    pub free_without_journal: u64,
    /// Bytes the journal could release now.
    pub reclaimable: u64,
    /// Reserve.
    pub reserve: u64,
    /// Soft threshold.
    pub soft: u64,
    /// Logical bytes held by the journal.
    pub journal_bytes: u64,
    /// Why the journal is off; `None` when it is on.
    pub inactive: Option<InactiveReason>,
}

/// Scope of a run-time limit change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitsScope {
    /// Until the end of the session of the request envelope.
    Session(SessionId),
    /// Written to the user configuration file.
    User,
}

/// Parameters of `journal.limits`; without any value the call only reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitsRequest {
    /// New step limit.
    pub steps: Option<StepLimit>,
    /// New age limit.
    pub age: Option<AgeLimit>,
    /// New size budget.
    pub size: Option<SizeLimit>,
    /// Scope of the change.
    pub scope: LimitsScope,
    /// Volume; `None` means all volumes.
    pub volume: Option<AbsPath>,
    /// Report the effect without changing anything.
    pub preview: bool,
}

/// Effective limits of one volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VolumeRetention {
    /// Volume mount point.
    pub volume: AbsPath,
    /// Effective limits.
    pub limits: EffectiveRetention,
}

/// Result of `journal.limits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LimitsOutcome {
    /// Effective limits per volume after the change (or current, when only
    /// reporting).
    pub effective: Vec<VolumeRetention>,
    /// Entries that expire (or would expire, with preview) because of the
    /// change.
    pub expiring: Vec<OpId>,
    /// Logical bytes of those entries.
    pub expiring_bytes: u64,
    /// The change was applied.
    pub applied: bool,
}

/// Result of crash recovery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecoveryReport {
    /// Prepared entries rolled back.
    pub rolled_back: Vec<OpId>,
    /// Rollbacks completed.
    pub completed_rollbacks: Vec<OpId>,
    /// Reclaims completed.
    pub completed_reclaims: Vec<OpId>,
    /// Entries marked corrupt.
    pub corrupt: Vec<OpId>,
    /// Entries expired by the synchronous review.
    pub expired: Vec<OpId>,
    /// Wall time spent, milliseconds.
    pub elapsed_ms: u64,
    /// Entries left unchanged because their item list cannot be read; they
    /// are repaired by `journal salvage`.
    #[serde(default)]
    pub needs_salvage: Vec<OpId>,
    /// Entries left unchanged because another live process holds their lock.
    #[serde(default)]
    pub skipped_live: Vec<OpId>,
    /// Entries left unchanged because their state cannot be decided, for
    /// example a prepared entry without its marker or an unreadable part of
    /// an entry stored in several parts.
    #[serde(default)]
    pub unresolved: Vec<OpId>,
}

/// Entry expired by a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExpiredEntry {
    /// Entry.
    pub op: OpId,
    /// Reason.
    pub reason: ExpiryReason,
}

/// Result of an expiry pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SweepReport {
    /// Entries expired, with reasons.
    pub expired: Vec<ExpiredEntry>,
    /// Logical bytes released.
    pub bytes: u64,
}

/// Operation journal of an account.
///
/// One instance per process; implementations coordinate across processes with
/// a lock file per volume journal and an account-wide sequence file. All
/// methods may be called concurrently from several threads.
pub trait JournalStore: Send + Sync {
    /// Completes or reverts interrupted entries and expires overdue ones.
    ///
    /// Called automatically before the first transaction of a process; the
    /// space service calls it at start. Idempotent: a crash during recovery is
    /// repaired by the next recovery.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a journal directory cannot be read; affected
    /// volumes are then treated as inactive and user operations proceed.
    fn recover(&self) -> Result<RecoveryReport>;

    /// Opens a mutation transaction.
    ///
    /// Infallible by design: journal problems degrade preservation, never the
    /// operation. No number is consumed until the first preserve or commit.
    fn begin(&self, request: BeginRequest) -> Box<dyn MutationTx + '_>;

    /// Undoes a committed entry because a check after the commit (`verify`)
    /// failed; creates an entry of kind `undo` and marks the source entry
    /// undone with [`UndoneCause::VerifyFailed`]. Failures detected before
    /// commit (validation, entity tag mismatch) use [`MutationTx::rollback`]
    /// instead and create no new entry.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` when the entry does not exist or is not
    /// committed, and with `E_ROLLBACK_FAILED` listing the items not restored.
    fn rollback_committed(&self, op: OpId, reason: RollbackReason) -> Result<UndoOutcome>;

    /// Lists entries, newest first.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the journal index cannot be read.
    fn history(&self, query: &HistoryQuery) -> Result<Vec<OperationRecord>>;

    /// Details of one entry.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` for an unknown entry and with
    /// `E_JOURNAL_CORRUPT` for an entry that failed its integrity check.
    fn details(&self, op: OpId) -> Result<OperationDetails>;

    /// Undoes entries.
    ///
    /// The conflict check covers the whole selection before the first write;
    /// on any failure nothing is changed. The undo is itself a transaction:
    /// the current state of every restored item is preserved in the undo
    /// entry, so it can be redone.
    ///
    /// # Errors
    ///
    /// Fails with `E_JOURNAL_NOOP` (nothing to undo, already undone),
    /// `E_JOURNAL_EXPIRED` (an entry in the selection expired; details list
    /// the entries), `E_JOURNAL_CONFLICT` (details list [`Conflict`] values),
    /// `E_JOURNAL_PARTIAL` (details list available and unavailable items),
    /// `E_JOURNAL_CORRUPT` and `E_ROLLBACK_FAILED`.
    fn undo(&self, request: &UndoRequest) -> Result<UndoOutcome>;

    /// Redoes an undo entry, with the same check and errors as
    /// [`JournalStore::undo`].
    fn redo(&self, request: &RedoRequest) -> Result<UndoOutcome>;

    /// Pins or releases entries.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when an entry cannot be updated; entries that are
    /// not available are reported, not failed.
    fn pin(&self, request: &PinRequest) -> Result<PinOutcome>;

    /// Releases entries immediately; irreversible.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a journal directory cannot be changed.
    fn purge(&self, request: &PurgeRequest) -> Result<PurgeOutcome>;

    /// Retention state per volume; the only place where it is reported.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` for an unknown volume.
    fn status(&self, volume: Option<&AbsPath>) -> Result<Vec<JournalState>>;

    /// Shows or changes limits.
    ///
    /// Lowering a limit expires entries immediately; raising it never
    /// restores expired entries.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the user configuration file cannot be
    /// written.
    fn limits(&self, request: &LimitsRequest) -> Result<LimitsOutcome>;

    /// Space report per volume.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` for an unknown volume.
    fn space(&self, volume: Option<&AbsPath>) -> Result<Vec<VolumeSpace>>;

    /// Expires entries over any limit (steps, age, size); reclaim for space is
    /// driven by the space service through [`Admission`].
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when a journal directory cannot be changed.
    fn sweep(&self) -> Result<SweepReport>;
}

/// State of a volume with respect to the space thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VolumeState {
    /// Free space at or above the soft threshold: the journal accepts data
    /// and entries expire by limits only.
    Normal,
    /// Free space between reserve and soft threshold: data is accepted only
    /// when free space after the write stays at or above reserve plus margin;
    /// background reclaim runs.
    Soft,
    /// Free space below the reserve: no data is accepted and reclaim runs
    /// immediately.
    Hard,
    /// Free space below half the reserve: as hard, with reclaim on all cores;
    /// callers do not wait for the service.
    Emergency,
}

/// Thresholds of one volume in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Thresholds {
    /// Reserve; for quota-limited accounts no more than 15 % of the quota.
    pub reserve: u64,
    /// Soft threshold, above the reserve.
    pub soft: u64,
    /// Margin.
    pub margin: u64,
    /// Hysteresis.
    pub hysteresis: u64,
}

/// State and thresholds of a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VolumeStatus {
    /// State.
    pub state: VolumeState,
    /// Thresholds.
    pub thresholds: Thresholds,
}

/// Liveness of the space service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceLiveness {
    /// Heartbeat fresh and probe answered.
    Alive,
    /// Heartbeat stale, process missing or probe not answered in time.
    Down,
}

/// Whether the journal is active on a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Eligibility {
    /// The journal may keep data on the volume.
    Eligible,
    /// The journal is inactive on the volume.
    Inactive(InactiveReason),
}

/// Request of journal data for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdmissionRequest {
    /// Volume.
    pub volume: VolumeId,
    /// Bytes the journal will additionally occupy: previous content that
    /// would otherwise be released, or copied bytes.
    pub extra: u64,
    /// Logical size of the whole operation, for the single-operation limit and
    /// the size budget.
    pub op_size: u64,
    /// Logical bytes currently held on the volume.
    pub bytes_held: u64,
    /// Size budget in bytes, when set.
    pub size_budget: Option<u64>,
    /// Keep entries above the budget (oversize handling `keep`).
    pub oversize_keep: bool,
    /// Single-operation limit in bytes, when set.
    pub op_size_limit: Option<u64>,
}

/// Decision of admission for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionDecision {
    /// Keep the previous content.
    Keep,
    /// Keep after synchronously releasing the listed entries.
    KeepAfterReclaim {
        /// Released entries.
        released: Vec<OpId>,
    },
    /// Change without preservation.
    Skip {
        /// Reason reported to the user as the irreversible marker of the
        /// target: `journal_inactive`, `no_space_for_copy`,
        /// `too_large_for_journal`, `network_volume` or `removable_volume`.
        reason: Irreversible,
    },
}

/// Admission control, called by the journal before preserving targets.
///
/// User space always has priority: admission keeps data only when, after the
/// write, free space stays at or above reserve plus margin and the size
/// budget holds, the space service is alive and the volume is eligible.
/// Otherwise it releases the oldest entries within the synchronous time
/// budget; if that is not enough, it returns [`AdmissionDecision::Skip`].
/// Admission never fails and never blocks longer than the synchronous budget.
pub trait Admission: Send + Sync {
    /// Whether the journal is active on a volume, and why not.
    fn eligibility(&self, volume: &VolumeInfo) -> Eligibility;

    /// Decides for one target; may release old entries within the
    /// synchronous budget.
    fn admit(&self, request: &AdmissionRequest) -> AdmissionDecision;

    /// Notifies the service after an operation (for example when free space
    /// fell below the reserve).
    fn after_operation(&self, volume: &VolumeId);

    /// Current state and thresholds of a volume.
    fn volume_status(&self, volume: &VolumeId) -> VolumeStatus;

    /// Liveness of the space service.
    fn liveness(&self) -> ServiceLiveness;
}
