// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Entry manifest format.
//!
//! An entry part consists of the manifest `op.json` ([`Manifest`], format
//! version 1), the state file `STATE` ([`StateRecord`]), the optional item
//! state file `ITEMS` ([`Items`]) and the preserved data in `blobs/`. The
//! account index `ops.jsonl` has one [`IndexRow`] per committed operation.
//!
//! # Immutability and digest
//!
//! The manifest is written twice: when the entry becomes `prepared` (the plan:
//! targets, preserve modes, data paths and the state before) and once more,
//! atomically, after every change was applied and before the entry becomes
//! `committed` (with the state after, bytes held, irreversible markers and the
//! chain link). Both versions carry `integrity.manifest_sha256`, computed by
//! the digest rule of the integrity module: the manifest as a JSON value
//! without the member `integrity.manifest_sha256`, integers beyond 2^53 as
//! decimal strings, serialized by RFC 8785 and hashed with SHA-256. After
//! `committed` the manifest never changes; per-item state lives in `ITEMS`
//! and pins in `pins.json`, outside the digest.
//!
//! # Chain
//!
//! `integrity.chain` is the digest of the predecessor in the account index:
//! the digest named by the last complete row of `ops.jsonl` when the entry is
//! committed ([`IndexRow::digest`]), or `null` for the first entry. The final
//! manifest, the committed state and the index row are written in one
//! critical section under the account lock, so concurrent processes and
//! several volumes yield one linear order. A crash between the committed
//! state and the index row leaves the row missing; recovery appends it, and a
//! later entry committed in between names the same predecessor, which forks
//! the chain.
//!
//! # Multi-volume operations
//!
//! An operation spanning several volumes has one number and one manifest per
//! volume; `parts` of every manifest lists all parts. No part becomes
//! `committed` before every part has its changes applied and durable and its
//! final manifest written. Recovery therefore completes all parts to
//! `committed` when any part is committed, and rolls back all parts
//! otherwise.

use std::io::{Read, Seek, SeekFrom};

use nurt_contracts::env::{ClockStamp, WallTime};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::fs::{
    AbsPath, ContentHash, Durability, ExtendedMeta, FileId, FileKind, Follow, Fs, FsError,
    FsErrorKind, FsResult, FsWrite, RenameMode, TxScope, VolumeId,
};
use nurt_contracts::journal::{
    AfterState, BeforeState, EntryState, ExemptWrite, JournalNotable, OpId, OpKind, PreserveMode,
    SessionId, TargetAction, TargetIndex, TargetPath, UndoScope, UndoneCause,
};
use nurt_contracts::report::{Correction, Irreversible, IrreversibleEntry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::integrity;

/// Version of the manifest format.
pub const FORMAT_VERSION: u32 = 1;
/// Digest algorithm named in `integrity.algo`.
pub const DIGEST_ALGO: &str = "sha256";
/// Prefix of the checksum trailer of `ITEMS`.
const CRC_TRAILER: &str = "\ncrc32 ";
/// Number of bytes read from the end of the account index to find its last
/// row; rows are far shorter.
const INDEX_TAIL: u64 = 64 * 1024;
/// Hint of temporary file names of atomic writes.
const TEMP_HINT: &str = ".nurt-write";

/// Manifest of one entry part (`op.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Format version; always [`FORMAT_VERSION`].
    pub v: u32,
    /// Operation number.
    pub op: OpId,
    /// Unique identifier of the operation, equal in all parts.
    pub uuid: uuid::Uuid,
    /// Wall time when the entry was prepared, updated at commit.
    pub time: WallTime,
    /// Readings of all clocks at commit (at preparation in the prepared
    /// manifest); the basis of the age of the entry.
    pub time_basis: ClockStamp,
    /// Kind of the operation.
    pub kind: OpKind,
    /// Session of the operation.
    #[serde(default)]
    pub session: Option<SessionId>,
    /// Hash of the canonical bound program.
    pub ast_hash: ContentHash,
    /// Bound program, stored verbatim.
    pub ast: serde_json::Value,
    /// Spelling corrections applied to the program.
    #[serde(default)]
    pub corrections: Vec<Correction>,
    /// Bulk operation, undone as a whole.
    pub bulk: bool,
    /// The entry counts towards the step limit.
    pub counts_as_step: bool,
    /// Targets of this part, in declaration order.
    pub targets: Vec<TargetRecord>,
    /// All parts of the operation, ordered by volume.
    pub parts: Vec<PartRef>,
    /// Logical bytes held by this part.
    pub bytes_held: u64,
    /// Operation-level irreversible markers.
    #[serde(default)]
    pub irreversible: Vec<IrreversibleEntry>,
    /// Operation-level notable effects.
    #[serde(default)]
    pub notable: Vec<JournalNotable>,
    /// Writes exempt from the journal by design.
    #[serde(default)]
    pub exempt: Vec<ExemptWrite>,
    /// Limits in force at commit, for information only; retention evaluates
    /// the current configuration, and no result reports these values.
    #[serde(default)]
    pub expires: Option<Expires>,
    /// Entries undone by this entry.
    #[serde(default)]
    pub undo_of: Vec<OpId>,
    /// Undo entry redone by this entry.
    #[serde(default)]
    pub redo_of: Option<OpId>,
    /// Scope of an undo entry.
    #[serde(default)]
    pub scope: Option<UndoScope>,
    /// Items restored by an undo entry.
    #[serde(default)]
    pub items: Vec<ItemRef>,
    /// Digest and chain link.
    pub integrity: Integrity,
}

/// One target of an entry part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetRecord {
    /// Target index within the operation.
    pub index: TargetIndex,
    /// Address recorded in this part: the target itself, or the destination
    /// of a rename across volumes ([`TargetRole::Destination`]).
    pub path: TargetPath,
    /// Declared action.
    pub action: TargetAction,
    /// Role of the record for renames across volumes.
    #[serde(default, skip_serializing_if = "TargetRole::is_whole")]
    pub role: TargetRole,
    /// State before the change and how it is kept.
    pub before: BeforeRecord,
    /// Existing destination of a rename within the volume, preserved before
    /// it was overwritten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<BeforeRecord>,
    /// State after the change; absent in the prepared manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<AfterRecord>,
    /// The new content was written into the existing file (several hard
    /// links, or metadata that a new file cannot take over).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_place: bool,
    /// Logical bytes held for this target.
    pub bytes_held: u64,
    /// Reason, present when the previous state is not kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irreversible: Option<Irreversible>,
    /// Unusual effects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notable: Vec<JournalNotable>,
}

/// Role of a target record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetRole {
    /// The record describes the whole target.
    #[default]
    Whole,
    /// Source of a rename across volumes, preserved in its own volume.
    Source,
    /// Destination of a rename across volumes, recorded in its own volume.
    Destination,
}

impl TargetRole {
    fn is_whole(&self) -> bool {
        *self == Self::Whole
    }
}

/// State of an entry before the change and the location of its preserved
/// data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeforeRecord {
    /// The entry existed.
    pub exists: bool,
    /// Kind of the entry.
    #[serde(rename = "type", default)]
    pub kind: Option<FileKind>,
    /// Logical size; for appends the length before the append.
    #[serde(default)]
    pub size: Option<u64>,
    /// Permission bits.
    #[serde(default)]
    pub mode: Option<u32>,
    /// User id of the file.
    #[serde(default)]
    pub uid: Option<u32>,
    /// Group id of the file.
    #[serde(default)]
    pub gid: Option<u32>,
    /// Modification time, nanoseconds since the Unix epoch.
    #[serde(default)]
    pub modified_ns: Option<i64>,
    /// Content hash when known without reading the content (entity tag of the
    /// read stage); for appends the hash of the previous content, which is the
    /// prefix kept by the append.
    #[serde(default)]
    pub hash: Option<ContentHash>,
    /// Number of hard links.
    #[serde(default)]
    pub links: Option<u64>,
    /// File identity from the entity tag; distinguishes the previous file
    /// from its replacement after an exchange of names. Serialized as
    /// `{"volume", "low", "high"}` with decimal strings.
    #[serde(default, with = "file_id_text")]
    pub file_id: Option<FileId>,
    /// How the previous state is kept. In the prepared manifest this is the
    /// planned mode; an overwrite planned as `exchange` may end as `clone`,
    /// `copy` or `none`, recorded in the final manifest.
    pub store: PreserveMode,
    /// Preserved data relative to the entry directory (`blobs/<id>.blob`).
    #[serde(default)]
    pub blob: Option<String>,
    /// Preserved metadata relative to the entry directory
    /// (`blobs/<id>.meta`).
    #[serde(default)]
    pub meta: Option<String>,
}

impl BeforeRecord {
    /// State before in the form of the journal interface.
    #[must_use]
    pub fn to_state(&self) -> BeforeState {
        BeforeState {
            exists: self.exists,
            kind: self.kind,
            size: self.size,
            hash: self.hash,
            mode: self.mode,
        }
    }
}

/// Serialization of file identities with decimal strings, so that values
/// beyond the exact range of doubles keep every digit in any JSON reader.
mod file_id_text {
    use nurt_contracts::fs::FileId;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Text {
        volume: String,
        low: String,
        high: String,
    }

    #[expect(
        clippy::ref_option,
        reason = "serde passes the field by reference to `serialize_with`"
    )]
    pub(super) fn serialize<S: Serializer>(
        id: &Option<FileId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        id.map(|id| Text {
            volume: id.volume.to_string(),
            low: id.low.to_string(),
            high: id.high.to_string(),
        })
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<FileId>, D::Error> {
        let Some(text) = Option::<Text>::deserialize(deserializer)? else {
            return Ok(None);
        };
        let number = |digits: &str| digits.parse::<u64>().map_err(D::Error::custom);
        Ok(Some(FileId {
            volume: number(&text.volume)?,
            low: number(&text.low)?,
            high: number(&text.high)?,
        }))
    }
}

/// State of a target after the change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterRecord {
    /// The entry exists.
    pub exists: bool,
    /// Kind of the entry.
    #[serde(rename = "type", default)]
    pub kind: Option<FileKind>,
    /// Logical size.
    #[serde(default)]
    pub size: Option<u64>,
    /// Permission bits.
    #[serde(default)]
    pub mode: Option<u32>,
    /// Content hash.
    #[serde(default)]
    pub hash: Option<ContentHash>,
    /// Modification time, nanoseconds since the Unix epoch.
    #[serde(default)]
    pub modified_ns: Option<i64>,
}

impl AfterRecord {
    /// State after in the form of the journal interface.
    #[must_use]
    pub fn to_state(&self) -> AfterState {
        AfterState {
            exists: self.exists,
            kind: self.kind,
            size: self.size,
            hash: self.hash,
            mode: self.mode,
            meta: None,
        }
    }
}

/// One part of an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartRef {
    /// Volume of the part.
    pub vol: VolumeId,
    /// Journal directory of the volume.
    pub dir: AbsPath,
}

/// Limits in force when the entry was committed (informational).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expires {
    /// Step limit; `None` when unlimited.
    #[serde(default)]
    pub after_ops: Option<u32>,
    /// Age limit in seconds; `None` when unlimited.
    #[serde(default)]
    pub after_seconds: Option<u64>,
    /// Wall time at which the age limit is reached; `None` when unlimited.
    #[serde(default)]
    pub deadline: Option<WallTime>,
}

/// Item restored by an undo entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRef {
    /// Source entry.
    pub op: OpId,
    /// Target index in the source entry.
    pub index: TargetIndex,
    /// Address of the item.
    pub path: TargetPath,
}

/// Digest and chain link of a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integrity {
    /// Digest algorithm; always [`DIGEST_ALGO`].
    pub algo: String,
    /// Digest of the manifest; `None` only before [`Manifest::seal`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<ContentHash>,
    /// Digest of the manifest of the predecessor in the account index;
    /// `None` for the first entry and in prepared manifests.
    pub chain: Option<ContentHash>,
}

impl Default for Integrity {
    fn default() -> Self {
        Self {
            algo: DIGEST_ALGO.to_owned(),
            manifest_sha256: None,
            chain: None,
        }
    }
}

impl Manifest {
    /// Records the digest of the manifest in `integrity.manifest_sha256` and
    /// returns it.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::Encoding`] when the manifest cannot be
    /// represented as canonical JSON.
    pub fn seal(&mut self) -> Result<ContentHash, FormatError> {
        let digest = integrity::manifest_sha256(self)
            .map_err(|error| FormatError::Encoding(error.to_string()))?;
        self.integrity.manifest_sha256 = Some(digest);
        Ok(digest)
    }

    /// Serialized form written to `op.json`.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::Encoding`] when the manifest cannot be
    /// serialized.
    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        serde_json::to_vec(self).map_err(|error| FormatError::Encoding(error.to_string()))
    }

    /// Parses `op.json`. Arbitrary bytes never cause a panic.
    ///
    /// The digest is not verified here; the integrity module verifies it.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::Version`] for a format version other than
    /// [`FORMAT_VERSION`] and with [`FormatError::Decoding`] for any other
    /// malformed content.
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        #[derive(Deserialize)]
        struct Versioned {
            v: u64,
        }
        let versioned: Versioned = serde_json::from_slice(bytes)
            .map_err(|error| FormatError::Decoding(error.to_string()))?;
        if versioned.v != u64::from(FORMAT_VERSION) {
            return Err(FormatError::Version(versioned.v));
        }
        serde_json::from_slice(bytes).map_err(|error| FormatError::Decoding(error.to_string()))
    }

    /// Target record of `index` in the given role.
    #[must_use]
    pub fn target(&self, index: TargetIndex, role: TargetRole) -> Option<&TargetRecord> {
        self.targets
            .iter()
            .find(|target| target.index == index && target.role == role)
    }
}

/// Persistent state of an entry (`STATE`).
///
/// The file holds the snake-case name of the state, followed for `undone`
/// and `rolling_back` by a space and the cause, and a line feed:
/// `committed`, `rolling_back rolled_back`, `undone crash`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::journal::{EntryState, UndoneCause};
/// use nurt_journal::manifest::StateRecord;
///
/// let record = StateRecord::undone(UndoneCause::RolledBack);
/// assert_eq!(record.encode(), b"undone rolled_back\n");
/// assert_eq!(StateRecord::decode(b"undone rolled_back\n"), Ok(record));
/// assert_eq!(
///     StateRecord::decode(b"committed\n").map(|r| r.state),
///     Ok(EntryState::Committed)
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRecord {
    /// State.
    pub state: EntryState,
    /// Cause of `undone`, or the cause recorded when `rolling_back` started.
    pub cause: Option<UndoneCause>,
}

impl StateRecord {
    /// State without cause.
    #[must_use]
    pub const fn new(state: EntryState) -> Self {
        Self { state, cause: None }
    }

    /// `rolling_back` with the cause the completed rollback will record.
    #[must_use]
    pub const fn rolling_back(cause: UndoneCause) -> Self {
        Self {
            state: EntryState::RollingBack,
            cause: Some(cause),
        }
    }

    /// `undone` with its cause.
    #[must_use]
    pub const fn undone(cause: UndoneCause) -> Self {
        Self {
            state: EntryState::Undone,
            cause: Some(cause),
        }
    }

    /// Content of the state file.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut text = snake_name(&self.state);
        if let Some(cause) = &self.cause {
            text.push(' ');
            text.push_str(&snake_name(cause));
        }
        text.push('\n');
        text.into_bytes()
    }

    /// Parses the state file. Unknown or malformed content yields an error,
    /// never a panic.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::State`] when the content is not a known state
    /// with an admissible cause.
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let malformed = || FormatError::State(String::from_utf8_lossy(bytes).into_owned());
        let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        let mut words = text.split(' ');
        let state: EntryState = from_snake_name(words.next()).ok_or_else(malformed)?;
        let cause: Option<UndoneCause> = match words.next() {
            Some(word) => Some(from_snake_name(Some(word)).ok_or_else(malformed)?),
            None => None,
        };
        let cause_allowed = matches!(state, EntryState::Undone | EntryState::RollingBack);
        if words.next().is_some() || (cause.is_some() && !cause_allowed) {
            return Err(malformed());
        }
        Ok(Self { state, cause })
    }
}

fn snake_name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

fn from_snake_name<T: for<'de> Deserialize<'de>>(word: Option<&str>) -> Option<T> {
    let word = word.filter(|word| !word.is_empty())?;
    serde_json::from_value(serde_json::Value::String(word.to_owned())).ok()
}

/// State of individual items of an entry (`ITEMS`), outside the digest of
/// the manifest. A missing file means that every item is available.
///
/// The file holds the JSON form followed by a line `crc32 <8 hex digits>`
/// with the CRC-32 of the JSON bytes.
///
/// # Examples
///
/// ```
/// use nurt_contracts::journal::TargetIndex;
/// use nurt_journal::manifest::{ItemState, Items};
///
/// let items = Items {
///     items: vec![ItemState { index: TargetIndex(1), released: true, undone_by: None }],
/// };
/// let bytes = items.encode().unwrap();
/// assert_eq!(Items::decode(&bytes), Ok(items));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Items {
    /// Items whose state differs from available.
    pub items: Vec<ItemState>,
}

/// State of one item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemState {
    /// Target index.
    pub index: TargetIndex,
    /// Released for space; no longer restorable.
    #[serde(default)]
    pub released: bool,
    /// Undo entry that restored this item individually.
    #[serde(default)]
    pub undone_by: Option<OpId>,
}

impl Items {
    /// Content of the item state file.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::Encoding`] when the state cannot be
    /// serialized.
    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        let mut bytes =
            serde_json::to_vec(self).map_err(|error| FormatError::Encoding(error.to_string()))?;
        let crc = crc32fast::hash(&bytes);
        bytes.extend_from_slice(format!("{CRC_TRAILER}{crc:08x}\n").as_bytes());
        Ok(bytes)
    }

    /// Parses the item state file and verifies its checksum.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::Checksum`] when the trailer is missing or
    /// does not match, and with [`FormatError::Decoding`] when the checked
    /// content is malformed.
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let text = std::str::from_utf8(bytes).map_err(|_| FormatError::Checksum)?;
        let (body, trailer) = text.rsplit_once(CRC_TRAILER).ok_or(FormatError::Checksum)?;
        let trailer = trailer.strip_suffix('\n').unwrap_or(trailer);
        let recorded = u32::from_str_radix(trailer, 16).map_err(|_| FormatError::Checksum)?;
        if trailer.len() != 8 || crc32fast::hash(body.as_bytes()) != recorded {
            return Err(FormatError::Checksum);
        }
        serde_json::from_str(body).map_err(|error| FormatError::Decoding(error.to_string()))
    }
}

/// One row of the account index (`ops.jsonl`), appended when an operation
/// is committed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexRow {
    /// Operation number.
    pub op: OpId,
    /// Commit time.
    pub time: WallTime,
    /// Kind.
    pub kind: OpKind,
    /// Session.
    #[serde(default)]
    pub session: Option<SessionId>,
    /// Bulk operation.
    #[serde(default)]
    pub bulk: bool,
    /// The entry counts towards the step limit.
    pub counts_as_step: bool,
    /// Parts of the operation with their manifest digests, ordered by volume.
    /// Empty when the journal was inactive on every volume and the
    /// operation is listed for its irreversible markers only.
    pub parts: Vec<IndexPart>,
}

/// One part named by an index row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexPart {
    /// Volume.
    pub vol: VolumeId,
    /// Journal directory of the volume.
    pub dir: AbsPath,
    /// Digest of the committed manifest of the part.
    pub manifest_sha256: ContentHash,
}

impl IndexRow {
    /// Digest that represents the row in the chain: the digest of the first
    /// part; `None` for a row without parts, which the chain skips.
    #[must_use]
    pub fn digest(&self) -> Option<ContentHash> {
        self.parts.first().map(|part| part.manifest_sha256)
    }

    /// One line of the index, including the line feed.
    ///
    /// # Errors
    ///
    /// Fails with [`FormatError::Encoding`] when the row cannot be
    /// serialized.
    pub fn encode(&self) -> Result<Vec<u8>, FormatError> {
        let mut line =
            serde_json::to_vec(self).map_err(|error| FormatError::Encoding(error.to_string()))?;
        line.push(b'\n');
        Ok(line)
    }
}

/// Listing of a preserved directory (`blobs/<id>.tree`), built on demand
/// because a directory tree is preserved by a single rename.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeListing {
    /// Entries below the directory in depth-first order of byte-sorted names.
    pub entries: Vec<TreeEntry>,
}

/// One entry of a [`TreeListing`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    /// Path relative to the listed directory, with `/` separators.
    pub path: String,
    /// Kind.
    #[serde(rename = "type")]
    pub kind: FileKind,
    /// Logical size.
    pub size: u64,
    /// Content hash of regular files.
    #[serde(default)]
    pub hash: Option<ContentHash>,
}

impl TreeListing {
    /// Lists the directory `dir` recursively, without following links, and
    /// hashes every regular file.
    ///
    /// # Errors
    ///
    /// Returns the first error of listing, inspecting or reading an entry.
    pub fn build(fs: &dyn Fs, dir: &AbsPath) -> FsResult<Self> {
        let mut entries = Vec::new();
        let mut pending = vec![(dir.clone(), String::new())];
        while let Some((current, prefix)) = pending.pop() {
            let mut children = Vec::new();
            for child in fs.read_dir(&current)? {
                children.push(child?.name);
            }
            children.sort();
            let mut subdirs = Vec::new();
            for name in children {
                let text = name.to_string_lossy().into_owned();
                let path = current.join_name(&text)?;
                let meta = fs.metadata(&path, Follow::NoFollow)?;
                let relative = if prefix.is_empty() {
                    text
                } else {
                    format!("{prefix}/{text}")
                };
                let hash = match meta.kind {
                    FileKind::File => Some(fs.content_hash(&path)?),
                    _ => None,
                };
                if meta.kind == FileKind::Directory {
                    subdirs.push((path, relative.clone()));
                }
                entries.push(TreeEntry {
                    path: relative,
                    kind: meta.kind,
                    size: meta.size,
                    hash,
                });
            }
            pending.extend(subdirs.into_iter().rev());
        }
        Ok(Self { entries })
    }
}

/// Error of decoding or encoding a journal file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormatError {
    /// The content is not a valid document of the expected format.
    Decoding(String),
    /// The value cannot be serialized.
    Encoding(String),
    /// The manifest has a format version this implementation does not read.
    Version(u64),
    /// The content of a state file is not a known state.
    State(String),
    /// The checksum of an item state file is missing or wrong.
    Checksum,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decoding(detail) => write!(f, "malformed journal file: {detail}"),
            Self::Encoding(detail) => write!(f, "journal file cannot be encoded: {detail}"),
            Self::Version(version) => write!(f, "unsupported manifest version {version}"),
            Self::State(text) => write!(f, "unknown entry state `{}`", text.trim_end()),
            Self::Checksum => f.write_str("checksum of the item state file does not match"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Error of reading a journal file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The file system failed.
    Fs(FsError),
    /// The content is malformed.
    Format(FormatError),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fs(error) => error.fmt(f),
            Self::Format(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<FsError> for ReadError {
    fn from(error: FsError) -> Self {
        Self::Fs(error)
    }
}

impl From<FormatError> for ReadError {
    fn from(error: FormatError) -> Self {
        Self::Format(error)
    }
}

impl From<ReadError> for Error {
    /// File system errors keep their code; malformed content is
    /// `E_JOURNAL_CORRUPT`.
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Fs(error) => error.into(),
            ReadError::Format(error) => Error::new(ErrorCode::JournalCorrupt, error.to_string()),
        }
    }
}

/// Reads a whole file.
///
/// # Errors
///
/// Returns the error of opening or reading the file.
pub fn read_bytes(fs: &dyn Fs, path: &AbsPath) -> FsResult<Vec<u8>> {
    let mut handle = fs.open_read(path)?;
    let mut bytes = Vec::new();
    handle
        .read_to_end(&mut bytes)
        .map_err(|_| FsError::new(FsErrorKind::Io, "read", Some(path.as_path())))?;
    Ok(bytes)
}

/// Writes a file atomically: a temporary file in `temp_dir` (on the same
/// volume) is written, flushed and renamed over `target`; then the directory
/// of `target` and, when it differs, `temp_dir` are flushed. A rename between
/// two directories is durable only when both directories are flushed.
///
/// # Errors
///
/// Returns the first error; a temporary file left behind is removed on a
/// best-effort basis.
pub fn write_atomic(
    fs: &dyn FsWrite,
    scope: &TxScope,
    temp_dir: &AbsPath,
    target: &AbsPath,
    bytes: &[u8],
) -> FsResult<()> {
    let mut handle = fs.create_temp_in(scope, temp_dir, TEMP_HINT)?;
    let temp = handle.path().clone();
    let written = write_all(handle.as_mut(), bytes)
        .and_then(|()| handle.sync(Durability::Full))
        .and_then(|()| {
            drop(handle);
            fs.rename(scope, &temp, target, RenameMode::Replace)
        });
    if let Err(error) = written {
        let _ = fs.remove_file(scope, &temp);
        return Err(error);
    }
    let parent = target.parent();
    if let Some(parent) = &parent {
        fs.sync_dir(scope, parent)?;
    }
    if parent.as_ref() != Some(temp_dir) {
        fs.sync_dir(scope, temp_dir)?;
    }
    Ok(())
}

/// Writes all bytes to a handle, mapping the I/O error to [`FsError`].
///
/// # Errors
///
/// Fails with [`FsErrorKind::NoSpace`] when the volume is full and with
/// [`FsErrorKind::Io`] otherwise.
pub fn write_all(handle: &mut dyn nurt_contracts::fs::WriteHandle, bytes: &[u8]) -> FsResult<()> {
    handle.write_all(bytes).map_err(|error| {
        let kind = if error.kind() == std::io::ErrorKind::StorageFull {
            FsErrorKind::NoSpace
        } else {
            FsErrorKind::Io
        };
        FsError::new(kind, "write", Some(handle.path().as_path()))
    })
}

/// Reads and parses a manifest.
///
/// # Errors
///
/// Returns the read error or the format error of [`Manifest::decode`].
pub fn read_manifest(fs: &dyn Fs, path: &AbsPath) -> Result<Manifest, ReadError> {
    Ok(Manifest::decode(&read_bytes(fs, path)?)?)
}

/// Reads and parses a state file.
///
/// # Errors
///
/// Returns the read error or the format error of [`StateRecord::decode`].
pub fn read_state(fs: &dyn Fs, path: &AbsPath) -> Result<StateRecord, ReadError> {
    Ok(StateRecord::decode(&read_bytes(fs, path)?)?)
}

/// Writes a state file atomically through a temporary file in `temp_dir`.
///
/// # Errors
///
/// As [`write_atomic`].
pub fn write_state(
    fs: &dyn FsWrite,
    scope: &TxScope,
    temp_dir: &AbsPath,
    path: &AbsPath,
    record: StateRecord,
) -> FsResult<()> {
    write_atomic(fs, scope, temp_dir, path, &record.encode())
}

/// Reads an item state file; a missing file yields the empty state (every
/// item available).
///
/// # Errors
///
/// Returns the read error, and [`FormatError::Checksum`] for a damaged file,
/// so that the caller decides conservatively.
pub fn read_items(fs: &dyn Fs, path: &AbsPath) -> Result<Items, ReadError> {
    match read_bytes(fs, path) {
        Ok(bytes) => Ok(Items::decode(&bytes)?),
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(Items::default()),
        Err(error) => Err(error.into()),
    }
}

/// Writes an item state file atomically through a temporary file in
/// `temp_dir`.
///
/// # Errors
///
/// Returns the encoding error or the error of [`write_atomic`].
pub fn write_items(
    fs: &dyn FsWrite,
    scope: &TxScope,
    temp_dir: &AbsPath,
    path: &AbsPath,
    items: &Items,
) -> Result<(), ReadError> {
    let bytes = items.encode()?;
    Ok(write_atomic(fs, scope, temp_dir, path, &bytes)?)
}

/// Reads preserved metadata (`blobs/<id>.meta`).
///
/// # Errors
///
/// Returns the read error or [`FormatError::Decoding`].
pub fn read_meta(fs: &dyn Fs, path: &AbsPath) -> Result<ExtendedMeta, ReadError> {
    serde_json::from_slice(&read_bytes(fs, path)?)
        .map_err(|error| FormatError::Decoding(error.to_string()).into())
}

/// Reads a directory listing (`blobs/<id>.tree`).
///
/// # Errors
///
/// Returns the read error or [`FormatError::Decoding`].
pub fn read_tree(fs: &dyn Fs, path: &AbsPath) -> Result<TreeListing, ReadError> {
    serde_json::from_slice(&read_bytes(fs, path)?)
        .map_err(|error| FormatError::Decoding(error.to_string()).into())
}

/// Appends one row to the account index and flushes it. The caller holds
/// the account lock.
///
/// # Errors
///
/// Returns the encoding error or the error of opening, writing or flushing
/// the index.
pub fn append_index_row(
    fs: &dyn FsWrite,
    scope: &TxScope,
    path: &AbsPath,
    row: &IndexRow,
) -> Result<(), ReadError> {
    let line = row.encode()?;
    let mut handle = match fs.open_in_place(scope, path) {
        Ok(handle) => handle,
        Err(error) if error.kind == FsErrorKind::NotFound => {
            let handle = fs.create_new(scope, path)?;
            if let Some(parent) = path.parent() {
                fs.sync_dir(scope, &parent)?;
            }
            handle
        }
        Err(error) => return Err(error.into()),
    };
    if let Some(complete) = torn_tail(fs, path)? {
        handle.set_length(complete)?;
    }
    handle
        .seek(SeekFrom::End(0))
        .map_err(|_| FsError::new(FsErrorKind::Io, "seek", Some(path.as_path())))?;
    write_all(handle.as_mut(), &line)?;
    handle.sync(Durability::Data)?;
    Ok(())
}

/// Length of the account index without its torn last line, when it has
/// one. A crash during an append can leave a row without its line end; the
/// next row appended after it would form one malformed line.
fn torn_tail(fs: &dyn Fs, path: &AbsPath) -> FsResult<Option<u64>> {
    let io_error = || FsError::new(FsErrorKind::Io, "read", Some(path.as_path()));
    let mut reader = fs.open_read(path)?;
    let length = reader.seek(SeekFrom::End(0)).map_err(|_| io_error())?;
    if length == 0 {
        return Ok(None);
    }
    reader.seek(SeekFrom::End(-1)).map_err(|_| io_error())?;
    let mut last = [0_u8; 1];
    reader.read_exact(&mut last).map_err(|_| io_error())?;
    if last[0] == b'\n' {
        return Ok(None);
    }
    let bytes = read_bytes(fs, path)?;
    let complete = bytes
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |end| end + 1);
    Ok(Some(u64::try_from(complete).unwrap_or(length)))
}

/// Reads every complete row of the account index in append order; a missing
/// index is empty. A torn last line (crash during an append) is ignored.
///
/// # Errors
///
/// Returns the read error, or [`FormatError::Decoding`] for a malformed
/// complete line.
pub fn read_index(fs: &dyn Fs, path: &AbsPath) -> Result<Vec<IndexRow>, ReadError> {
    let bytes = match read_bytes(fs, path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    parse_rows(&bytes)
}

/// Last complete row of the account index, reading only its tail; `None` for
/// a missing or empty index.
///
/// # Errors
///
/// Returns the read error, or [`FormatError::Decoding`] when the last
/// complete line is malformed.
pub fn last_index_row(fs: &dyn Fs, path: &AbsPath) -> Result<Option<IndexRow>, ReadError> {
    let mut handle = match fs.open_read(path) {
        Ok(handle) => handle,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let io_error = || FsError::new(FsErrorKind::Io, "read", Some(path.as_path()));
    let length = handle.seek(SeekFrom::End(0)).map_err(|_| io_error())?;
    let start = length.saturating_sub(INDEX_TAIL);
    handle
        .seek(SeekFrom::Start(start))
        .map_err(|_| io_error())?;
    let mut tail = Vec::new();
    handle.read_to_end(&mut tail).map_err(|_| io_error())?;
    let complete = match tail.iter().rposition(|&byte| byte == b'\n') {
        Some(end) => &tail[..end],
        None => return Ok(None),
    };
    let last = complete
        .rsplit(|&byte| byte == b'\n')
        .next()
        .unwrap_or_default();
    if last.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(last).map_err(|error| {
        FormatError::Decoding(error.to_string())
    })?))
}

fn parse_rows(bytes: &[u8]) -> Result<Vec<IndexRow>, ReadError> {
    let complete = match bytes.iter().rposition(|&byte| byte == b'\n') {
        Some(end) => &bytes[..end],
        None => return Ok(Vec::new()),
    };
    complete
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_slice(line)
                .map_err(|error| FormatError::Decoding(error.to_string()).into())
        })
        .collect()
}

/// SHA-256 of bytes as a content hash.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

#[cfg(test)]
pub(crate) mod tests {
    use nurt_contracts::env::BootId;
    use proptest::prelude::*;

    use super::*;

    fn abs(path: &str) -> AbsPath {
        AbsPath::new(path).unwrap()
    }

    /// Manifest of a single overwrite with fixed values, used by the digest
    /// vector.
    pub(crate) fn sample() -> Manifest {
        Manifest {
            v: FORMAT_VERSION,
            op: OpId(18),
            uuid: uuid::Uuid::from_u128(0x0b6a_8a2e_6a6e_4c53_8a2b_7a5f_2a1f_9d10),
            time: WallTime(1_763_720_047_412_000_000),
            time_basis: ClockStamp {
                wall: WallTime(1_763_720_047_412_000_000),
                boot: BootId("e0e6".to_owned()),
                since_boot_with_suspend: std::time::Duration::from_secs(3600),
                since_boot_active: std::time::Duration::from_secs(3000),
            },
            kind: OpKind::with_action("write", "overwrite"),
            session: Some(SessionId("s-1".to_owned())),
            ast_hash: hash_bytes(b"program"),
            ast: serde_json::json!({"nurt": "1.0", "program": {"statements": []}}),
            corrections: Vec::new(),
            bulk: false,
            counts_as_step: true,
            targets: vec![TargetRecord {
                index: TargetIndex(0),
                path: TargetPath::Fs(abs("/srv/app/src/a.go")),
                action: TargetAction::Modify,
                role: TargetRole::Whole,
                before: BeforeRecord {
                    exists: true,
                    kind: Some(FileKind::File),
                    size: Some(1127),
                    mode: Some(0o644),
                    uid: Some(1000),
                    gid: Some(1000),
                    modified_ns: Some(1_763_720_031_123_456_789),
                    hash: Some(hash_bytes(b"before")),
                    links: Some(1),
                    file_id: Some(FileId {
                        volume: 2049,
                        low: 77,
                        high: 0,
                    }),
                    store: PreserveMode::Exchange,
                    blob: Some("blobs/0000000000000000.blob".to_owned()),
                    meta: Some("blobs/0000000000000000.meta".to_owned()),
                },
                replaced: None,
                after: Some(AfterRecord {
                    exists: true,
                    kind: Some(FileKind::File),
                    size: Some(1131),
                    mode: Some(0o644),
                    hash: Some(hash_bytes(b"after")),
                    modified_ns: Some(1_763_720_047_412_000_000),
                }),
                in_place: false,
                bytes_held: 1127,
                irreversible: None,
                notable: Vec::new(),
            }],
            parts: vec![PartRef {
                vol: VolumeId {
                    dev: 2049,
                    uuid: Some("uuid-1".to_owned()),
                    subvolume: 0,
                },
                dir: abs("/home/user/.nurt-journal"),
            }],
            bytes_held: 1127,
            irreversible: Vec::new(),
            notable: Vec::new(),
            exempt: Vec::new(),
            expires: Some(Expires {
                after_ops: Some(30),
                after_seconds: Some(1800),
                deadline: Some(WallTime(1_763_721_847_412_000_000)),
            }),
            undo_of: Vec::new(),
            redo_of: None,
            scope: None,
            items: Vec::new(),
            integrity: Integrity::default(),
        }
    }

    #[test]
    fn manifest_round_trips_through_its_serialized_form() {
        let mut manifest = sample();
        manifest.seal().unwrap();
        let decoded = Manifest::decode(&manifest.encode().unwrap()).unwrap();
        assert_eq!(decoded, manifest);
        assert_eq!(
            decoded
                .target(TargetIndex(0), TargetRole::Whole)
                .map(|t| t.bytes_held),
            Some(1127)
        );
        assert!(decoded.target(TargetIndex(0), TargetRole::Source).is_none());
    }

    #[test]
    fn manifest_digest_matches_the_recorded_vector() {
        let mut manifest = sample();
        let digest = manifest.seal().unwrap();
        assert_eq!(
            digest.to_string(),
            "sha256:ed4645d1180df8cd7e98424d56da0ebe5533bd629bc7342d0bd7f9ba4d5396f8"
        );
    }

    #[test]
    fn digest_is_independent_of_the_recorded_digest_and_covers_the_chain() {
        let mut manifest = sample();
        let first = manifest.seal().unwrap();
        assert_eq!(manifest.seal().unwrap(), first);
        manifest.integrity.chain = Some(hash_bytes(b"previous"));
        assert_ne!(manifest.seal().unwrap(), first);
    }

    #[test]
    fn sealed_manifest_passes_integrity_verification() {
        let mut manifest = sample();
        let digest = manifest.seal().unwrap();
        let verdict = integrity::verify_manifest(&manifest.encode().unwrap());
        assert!(
            matches!(verdict, integrity::ManifestVerdict::Intact { .. }),
            "{verdict:?}"
        );
        let mut altered = manifest.clone();
        altered.bytes_held += 1;
        let verdict = integrity::verify_manifest(&altered.encode().unwrap());
        assert!(!matches!(
            verdict,
            integrity::ManifestVerdict::Intact { .. }
        ));
        assert_eq!(manifest.integrity.manifest_sha256, Some(digest));
    }

    #[test]
    fn rejects_other_format_versions() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value["v"] = serde_json::json!(2);
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(Manifest::decode(&bytes), Err(FormatError::Version(2)));
        assert!(matches!(
            Manifest::decode(b"{}"),
            Err(FormatError::Decoding(_))
        ));
    }

    #[test]
    fn before_and_after_records_convert_to_interface_states() {
        let target = &sample().targets[0];
        let before = target.before.to_state();
        assert!(before.exists);
        assert_eq!(before.size, Some(1127));
        let after = target.after.as_ref().unwrap().to_state();
        assert_eq!(after.size, Some(1131));
    }

    #[test]
    fn state_records_round_trip_and_reject_garbage() {
        for record in [
            StateRecord::new(EntryState::Prepared),
            StateRecord::new(EntryState::Committed),
            StateRecord::rolling_back(UndoneCause::Crash),
            StateRecord::undone(UndoneCause::VerifyFailed),
            StateRecord::new(EntryState::Corrupt),
        ] {
            assert_eq!(StateRecord::decode(&record.encode()), Ok(record));
        }
        for garbage in [
            &b""[..],
            b"\n",
            b"committed crash\n",
            b"undone sometimes\n",
            b"finished\n",
            b"undone crash extra\n",
            b"\xff\xfe",
        ] {
            assert!(StateRecord::decode(garbage).is_err(), "{garbage:?}");
        }
        assert_eq!(
            StateRecord::decode(b"partial").map(|record| record.state),
            Ok(EntryState::Partial)
        );
    }

    #[test]
    fn items_detect_damage_by_checksum() {
        let items = Items {
            items: vec![ItemState {
                index: TargetIndex(2),
                released: false,
                undone_by: Some(OpId(40)),
            }],
        };
        let mut bytes = items.encode().unwrap();
        assert_eq!(Items::decode(&bytes), Ok(items));
        bytes[3] ^= 1;
        assert_eq!(Items::decode(&bytes), Err(FormatError::Checksum));
        for garbage in [&b"{}"[..], b"{}\ncrc32 zz\n", b"{}\ncrc32 0\n"] {
            assert_eq!(
                Items::decode(garbage),
                Err(FormatError::Checksum),
                "{garbage:?}"
            );
        }
    }

    #[test]
    fn index_rows_name_the_digest_of_the_first_part() {
        let row = IndexRow {
            op: OpId(3),
            time: WallTime(5),
            kind: OpKind::command("remove"),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: vec![IndexPart {
                vol: VolumeId {
                    dev: 1,
                    uuid: None,
                    subvolume: 0,
                },
                dir: abs("/j"),
                manifest_sha256: hash_bytes(b"m"),
            }],
        };
        assert_eq!(row.digest(), Some(hash_bytes(b"m")));
        let mut bytes = row.encode().unwrap();
        bytes.extend_from_slice(b"{\"op\":");
        assert_eq!(parse_rows(&bytes).unwrap(), vec![row.clone()]);
        let empty = IndexRow {
            parts: Vec::new(),
            ..row
        };
        assert_eq!(empty.digest(), None);
    }

    #[test]
    fn append_after_a_torn_row_drops_the_fragment() {
        let fixture = crate::store::test_support::Fixture::new();
        let path = fixture.fs.path("vol-a/state/ops.jsonl");
        let row = |op: u64| IndexRow {
            op: OpId(op),
            time: WallTime(5),
            kind: OpKind::command("write"),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: Vec::new(),
        };
        let mut bytes = row(1).encode().unwrap();
        bytes.extend_from_slice(b"{\"op\":2,\"time\":");
        fixture.write(&path, &bytes);
        let scope = crate::store::scope(OpId(2), nurt_contracts::fs::ScopePurpose::Maintenance);
        append_index_row(&fixture.fs, &scope, &path, &row(2)).unwrap();
        assert_eq!(
            read_index(&fixture.fs, &path).unwrap(),
            vec![row(1), row(2)]
        );
    }

    #[test]
    fn errors_have_short_messages_and_map_to_language_codes() {
        let corrupt: Error = ReadError::Format(FormatError::Checksum).into();
        assert_eq!(corrupt.code(), ErrorCode::JournalCorrupt);
        let missing: Error =
            ReadError::Fs(FsError::new(FsErrorKind::NotFound, "open", None)).into();
        assert_eq!(missing.code(), ErrorCode::NotFound);
        assert_eq!(
            FormatError::Version(7).to_string(),
            "unsupported manifest version 7"
        );
        assert_eq!(
            FormatError::State("x\n".to_owned()).to_string(),
            "unknown entry state `x`"
        );
        assert!(
            ReadError::Format(FormatError::Checksum)
                .to_string()
                .starts_with("checksum")
        );
        assert!(
            FormatError::Decoding("d".into())
                .to_string()
                .starts_with("malformed")
        );
        assert!(
            FormatError::Encoding("e".into())
                .to_string()
                .ends_with(": e")
        );
    }

    fn row_of(op: u64, command: &str) -> IndexRow {
        IndexRow {
            op: OpId(op),
            time: WallTime(5),
            kind: OpKind::command(command),
            session: None,
            bulk: false,
            counts_as_step: true,
            parts: Vec::new(),
        }
    }

    #[test]
    fn index_rows_with_multibyte_text_and_crlf_line_ends_are_read() {
        let first = row_of(1, "zapisz-żółć");
        let second = row_of(2, "write");
        let mut bytes = first.encode().unwrap();
        bytes.pop();
        bytes.extend_from_slice(b"\r\n");
        bytes.extend(second.encode().unwrap());
        assert_eq!(parse_rows(&bytes).unwrap(), vec![first, second]);
    }

    #[test]
    fn blank_lines_and_a_missing_final_line_end_are_tolerated_in_the_index() {
        let row = row_of(3, "write");
        let mut bytes = b"\n\n".to_vec();
        bytes.extend(row.encode().unwrap());
        bytes.extend_from_slice(b"\n");
        assert_eq!(parse_rows(&bytes).unwrap(), vec![row.clone()]);
        let mut torn = row.encode().unwrap();
        torn.pop();
        assert!(parse_rows(&torn).unwrap().is_empty());
        assert!(parse_rows(b"").unwrap().is_empty());
        assert!(parse_rows(b"\n").unwrap().is_empty());
    }

    #[test]
    fn malformed_complete_index_lines_are_decoding_errors() {
        for bytes in [&b"not json\n"[..], b"\xff\xfe\n", b"[]\n", b"{}\n"] {
            assert!(
                matches!(
                    parse_rows(bytes),
                    Err(ReadError::Format(FormatError::Decoding(_)))
                ),
                "{bytes:?}"
            );
        }
    }

    #[test]
    fn last_index_row_names_the_last_complete_row() {
        let fixture = Fixture::new();
        let missing = fixture.fs.path("vol-a/state/last-missing.jsonl");
        assert_eq!(last_index_row(&fixture.fs, &missing).unwrap(), None);
        assert_eq!(read_index(&fixture.fs, &missing).unwrap(), Vec::new());

        let path = fixture.fs.path("vol-a/state/last-rows.jsonl");
        let mut bytes = row_of(1, "a").encode().unwrap();
        bytes.extend(row_of(2, "żółć").encode().unwrap());
        bytes.extend_from_slice(b"{\"op\":3,\"ti");
        fixture.write(&path, &bytes);
        assert_eq!(
            last_index_row(&fixture.fs, &path).unwrap(),
            Some(row_of(2, "żółć"))
        );

        let torn = fixture.fs.path("vol-a/state/last-torn.jsonl");
        fixture.write(&torn, b"{\"op\":");
        assert_eq!(last_index_row(&fixture.fs, &torn).unwrap(), None);
        assert!(read_index(&fixture.fs, &torn).unwrap().is_empty());

        let empty = fixture.fs.path("vol-a/state/last-empty.jsonl");
        fixture.write(&empty, b"");
        assert_eq!(last_index_row(&fixture.fs, &empty).unwrap(), None);
        assert!(read_index(&fixture.fs, &empty).unwrap().is_empty());
    }

    #[test]
    fn index_readers_report_a_malformed_complete_line() {
        let fixture = Fixture::new();
        let path = fixture.fs.path("vol-a/state/malformed.jsonl");
        fixture.write(&path, b"not json\n");
        assert!(matches!(
            read_index(&fixture.fs, &path),
            Err(ReadError::Format(FormatError::Decoding(_)))
        ));
        assert!(matches!(
            last_index_row(&fixture.fs, &path),
            Err(ReadError::Format(FormatError::Decoding(_)))
        ));
    }

    #[test]
    fn state_and_item_files_round_trip_through_atomic_writes() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = crate::store::scope(OpId(2), nurt_contracts::fs::ScopePurpose::Maintenance);
        let temp = fs.mount(0);

        let state_path = fs.path("vol-a/STATE");
        let record = StateRecord::rolling_back(UndoneCause::Crash);
        write_state(fs, &scope, &temp, &state_path, record).unwrap();
        assert_eq!(read_state(fs, &state_path).unwrap(), record);
        assert_eq!(fixture.read(&state_path), b"rolling_back crash\n");

        let items_path = fs.path("vol-a/ITEMS");
        assert_eq!(read_items(fs, &items_path).unwrap(), Items::default());
        let items = Items {
            items: vec![ItemState {
                index: TargetIndex(4),
                released: true,
                undone_by: Some(OpId(9)),
            }],
        };
        write_items(fs, &scope, &temp, &items_path, &items).unwrap();
        assert_eq!(read_items(fs, &items_path).unwrap(), items);

        let damaged = fs.path("vol-a/ITEMS-damaged");
        fixture.write(&damaged, b"{\"items\":[]}\ncrc32 00000000\n");
        assert!(matches!(
            read_items(fs, &damaged),
            Err(ReadError::Format(FormatError::Checksum))
        ));
        assert!(matches!(
            read_state(fs, &fs.path("vol-a/STATE-missing")),
            Err(ReadError::Fs(_))
        ));
    }

    #[test]
    fn item_files_tolerate_a_missing_final_line_end_but_not_a_carriage_return() {
        let items = Items {
            items: vec![ItemState {
                index: TargetIndex(1),
                released: false,
                undone_by: None,
            }],
        };
        let mut bytes = items.encode().unwrap();
        bytes.pop();
        assert_eq!(Items::decode(&bytes), Ok(items.clone()));
        bytes.extend_from_slice(b"\r\n");
        assert_eq!(Items::decode(&bytes), Err(FormatError::Checksum));
        assert_eq!(Items::decode("żółć".as_bytes()), Err(FormatError::Checksum));
        assert_eq!(Items::decode(b""), Err(FormatError::Checksum));
    }

    #[test]
    fn state_files_reject_carriage_returns_blank_words_and_multibyte_names() {
        for garbage in [
            &b"committed\r\n"[..],
            b"undone  crash\n",
            b" committed\n",
            b"committed \n",
            "zatwierdzony\n".as_bytes(),
            b"undone crash\n\n",
        ] {
            assert!(StateRecord::decode(garbage).is_err(), "{garbage:?}");
        }
        assert_eq!(
            StateRecord::decode(b"undone\n"),
            Ok(StateRecord::new(EntryState::Undone))
        );
    }

    #[test]
    fn manifests_with_other_versions_or_wrong_member_types_are_rejected() {
        assert_eq!(
            Manifest::decode(br#"{"v": 4294967297}"#),
            Err(FormatError::Version(4_294_967_297))
        );
        for bytes in [
            &b""[..],
            b"[]",
            b"null",
            br#"{"v": "1"}"#,
            br#"{"v": 1.5}"#,
            br#"{"v": -1}"#,
            br#"{"v": 1}"#,
        ] {
            assert!(
                matches!(Manifest::decode(bytes), Err(FormatError::Decoding(_))),
                "{bytes:?}"
            );
        }
        let deep = "[".repeat(100_000);
        assert!(matches!(
            Manifest::decode(deep.as_bytes()),
            Err(FormatError::Decoding(_))
        ));
    }

    #[test]
    fn file_identities_keep_every_digit_as_decimal_text() {
        let mut before = sample().targets[0].before.clone();
        before.file_id = Some(FileId {
            volume: u64::MAX,
            low: u64::MAX - 1,
            high: 1,
        });
        let value = serde_json::to_value(&before).unwrap();
        assert_eq!(value["file_id"]["volume"], "18446744073709551615");
        assert_eq!(value["file_id"]["low"], "18446744073709551614");
        assert_eq!(value["file_id"]["high"], "1");
        let back: BeforeRecord = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(back, before);
        for text in ["12x", "-1", "", "18446744073709551616"] {
            let mut damaged = value.clone();
            damaged["file_id"]["low"] = serde_json::json!(text);
            assert!(
                serde_json::from_value::<BeforeRecord>(damaged).is_err(),
                "{text:?}"
            );
        }
        before.file_id = None;
        let value = serde_json::to_value(&before).unwrap();
        assert!(value["file_id"].is_null());
        assert_eq!(
            serde_json::from_value::<BeforeRecord>(value).unwrap().file_id,
            None
        );
    }

    #[test]
    fn byte_hashes_match_the_published_sha256_vectors() {
        assert_eq!(
            hash_bytes(b"").to_string(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_bytes(b"abc").to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn tree_listing_covers_files_and_directories_with_hashes() {
        use nurt_fs::mem::{MemFs, VolumeSpec};

        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(abs("/v")))
            .build()
            .unwrap();
        fs.seed_file(&abs("/v/t/b.txt"), b"bb").unwrap();
        fs.seed_file(&abs("/v/t/a/x.txt"), b"x").unwrap();
        let goose = "gęś".as_bytes();
        fs.seed_file(&abs("/v/t/a/zażółć.txt"), goose).unwrap();
        fs.seed_dir(&abs("/v/t/empty")).unwrap();
        let listing = TreeListing::build(&fs, &abs("/v/t")).unwrap();
        let mut found: Vec<(&str, FileKind, Option<ContentHash>)> = listing
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind, entry.hash))
            .collect();
        found.sort_by(|left, right| left.0.cmp(right.0));
        assert_eq!(
            found,
            vec![
                ("a", FileKind::Directory, None),
                ("a/x.txt", FileKind::File, Some(hash_bytes(b"x"))),
                ("a/zażółć.txt", FileKind::File, Some(hash_bytes(goose))),
                ("b.txt", FileKind::File, Some(hash_bytes(b"bb"))),
                ("empty", FileKind::Directory, None),
            ]
        );
        let sizes: Vec<(&str, u64)> = listing
            .entries
            .iter()
            .filter(|entry| entry.kind == FileKind::File)
            .map(|entry| (entry.path.as_str(), entry.size))
            .collect();
        assert!(sizes.contains(&("b.txt", 2)));
        let goose_size = u64::try_from(goose.len()).unwrap();
        assert!(sizes.contains(&("a/zażółć.txt", goose_size)));
        assert!(TreeListing::build(&fs, &abs("/v/missing")).is_err());
        let encoded = serde_json::to_vec(&listing).unwrap();
        assert_eq!(serde_json::from_slice::<TreeListing>(&encoded).unwrap(), listing);
    }

    proptest! {
        #[test]
        fn decoding_arbitrary_bytes_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = Manifest::decode(&bytes);
            let _ = StateRecord::decode(&bytes);
            let _ = Items::decode(&bytes);
            let _ = parse_rows(&bytes);
        }

        #[test]
        fn decoding_mutated_manifests_never_panics(position in 0usize..2048, byte in any::<u8>()) {
            let mut bytes = sample().encode().unwrap();
            let position = position % bytes.len();
            bytes[position] = byte;
            let _ = Manifest::decode(&bytes);
        }
    }
}
