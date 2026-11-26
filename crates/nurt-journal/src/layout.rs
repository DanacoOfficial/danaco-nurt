// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! On-disk layout of a volume journal.
//!
//! ```text
//! .nurt-journal/        (.nurt-journal.noindex on macOS)
//!   VERSION  vol.json  service.hb  pins.json
//!   lock/journal.lock   journal-wide maintenance
//!   lock/<N>.lock       held by the transaction of entry <N>
//!   pending/<N>         marker of the non-terminal entry <N>
//!   ops/<N>.tmp/        entry not yet prepared
//!   ops/<N>/            op.json  STATE  ITEMS  tmp/  displaced/
//!   ops/<N>/blobs/      <id>.blob  <id>.meta  <id>.tree
//!   reap/<N>/           entry being reclaimed
//!   tomb/<N>.json       metrics of an expired entry
//! ```
//!
//! `<N>` has twelve decimal digits with leading zeros. The account state
//! directory ([`StateLayout`]) is supplied by the embedding application.
//!
//! # Non-terminal entries
//!
//! An entry is non-terminal while it is `prepared` or `rolling_back`, and
//! while it is `expiring` (located in `reap/`). Recovery finds such entries
//! through the markers in `pending/` without reading other entries. The
//! marker is durable before `ops/<N>.tmp/` is created and is removed only
//! after the terminal state is durable and, for committed entries, after the
//! index row was durably appended. A marker may be superfluous, never
//! missing. A prepared entry without a marker is an entry whose commit failed
//! after its changes were applied; recovery leaves it unchanged.
//!
//! # Live transactions
//!
//! The transaction of an entry holds an exclusive lock on `lock/<N>.lock`
//! from before the marker is created until the marker is removed; the
//! operating system releases it when the process ends. Recovery touches a
//! non-terminal entry only after it obtained this lock.

use nurt_contracts::fs::{AbsPath, Fs, FsErrorKind, FsResult};
use nurt_contracts::journal::{OpId, TargetIndex};

/// Default name of the journal directory.
pub const DIR_NAME: &str = ".nurt-journal";
/// Name of the journal directory on macOS, excluded from Spotlight indexing.
pub const DIR_NAME_MACOS: &str = ".nurt-journal.noindex";
/// Name of the format version file.
pub const VERSION_FILE: &str = "VERSION";
/// Content of the format version file of this format.
pub const VERSION_TEXT: &str = "nurt-journal 1";
/// Name of the volume description file.
pub const VOLUME_FILE: &str = "vol.json";
/// Name of the heartbeat file of the space service.
pub const HEARTBEAT_FILE: &str = "service.hb";
/// Name of the pin file.
pub const PINS_FILE: &str = "pins.json";
/// Directory of lock files.
pub const LOCK_DIR: &str = "lock";
/// Lock file of journal-wide maintenance.
pub const JOURNAL_LOCK_FILE: &str = "journal.lock";
/// Directory of entries.
pub const OPS_DIR: &str = "ops";
/// Directory of entries being reclaimed.
pub const REAP_DIR: &str = "reap";
/// Directory of tombstones.
pub const TOMB_DIR: &str = "tomb";
/// Directory of markers of non-terminal entries.
pub const PENDING_DIR: &str = "pending";
/// Manifest file of an entry.
pub const MANIFEST_FILE: &str = "op.json";
/// State file of an entry.
pub const STATE_FILE: &str = "STATE";
/// Item state file of an entry.
pub const ITEMS_FILE: &str = "ITEMS";
/// Directory of preserved data of an entry.
pub const BLOBS_DIR: &str = "blobs";
/// Directory of temporary files of an entry.
pub const ENTRY_TMP_DIR: &str = "tmp";
/// Directory of content displaced from targets by a rollback after a crash.
pub const DISPLACED_DIR: &str = "displaced";
/// Suffix of the directory of an entry that is not yet prepared.
pub const TMP_SUFFIX: &str = ".tmp";
/// Suffix of lock files.
pub const LOCK_SUFFIX: &str = ".lock";
/// Suffix of tombstone files.
pub const TOMB_SUFFIX: &str = ".json";
/// Prefix of staged files written next to targets.
pub const STAGED_PREFIX: &str = ".nurt-tmp-";
/// Number of decimal digits of an entry name.
pub const ENTRY_DIGITS: usize = 12;

/// Sequence file in the account state directory.
pub const SEQ_FILE: &str = "seq";
/// Account index in the account state directory.
pub const INDEX_FILE: &str = "ops.jsonl";
/// Volume registry in the account state directory.
pub const VOLUMES_FILE: &str = "volumes.json";
/// Account lock in the account state directory; serialises the sequence and
/// the account index across processes.
pub const ACCOUNT_LOCK_FILE: &str = "account.lock";

/// Name of the journal directory used on the host system when the
/// configuration does not name one.
#[must_use]
pub fn default_dir_name() -> &'static str {
    if cfg!(target_os = "macos") {
        DIR_NAME_MACOS
    } else {
        DIR_NAME
    }
}

/// Name of the directory of the entry `op`: twelve decimal digits with
/// leading zeros, more digits only for numbers that need them.
///
/// # Examples
///
/// ```
/// use nurt_contracts::journal::OpId;
/// use nurt_journal::layout::{entry_name, parse_entry_name};
///
/// assert_eq!(entry_name(OpId(18)), "000000000018");
/// assert_eq!(parse_entry_name("000000000018"), Some(OpId(18)));
/// ```
#[must_use]
pub fn entry_name(op: OpId) -> String {
    format!("{:0width$}", op.0, width = ENTRY_DIGITS)
}

/// Operation number of an entry name; `None` for any other name, including
/// names with fewer than twelve digits, temporary directories and names with
/// superfluous leading zeros, which [`entry_name`] never produces.
#[must_use]
pub fn parse_entry_name(name: &str) -> Option<OpId> {
    if name.len() < ENTRY_DIGITS || !name.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let op = OpId(name.parse().ok()?);
    (entry_name(op) == name).then_some(op)
}

/// Name of the staged file of target `index` of operation `op`, created in
/// the directory of the target. After an exchange of names the same name
/// holds the previous content until it is moved into the entry.
///
/// # Examples
///
/// ```
/// use nurt_contracts::journal::{OpId, TargetIndex};
/// use nurt_journal::layout::{parse_staged_name, staged_name};
///
/// let name = staged_name(OpId(7), TargetIndex(2));
/// assert_eq!(name, ".nurt-tmp-7-2");
/// assert_eq!(parse_staged_name(&name), Some((OpId(7), TargetIndex(2))));
/// ```
#[must_use]
pub fn staged_name(op: OpId, index: TargetIndex) -> String {
    format!("{STAGED_PREFIX}{}-{}", op.0, index.0)
}

/// Operation and target of a staged file name; `None` for any other name.
#[must_use]
pub fn parse_staged_name(name: &str) -> Option<(OpId, TargetIndex)> {
    let rest = name.strip_prefix(STAGED_PREFIX)?;
    let (op, index) = rest.split_once('-')?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(op) || !digits(index) {
        return None;
    }
    Some((OpId(op.parse().ok()?), TargetIndex(index.parse().ok()?)))
}

/// Role of a preserved data file within the preservation of one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlobRole {
    /// Previous content of the target itself.
    Previous,
    /// Previous content of an existing destination overwritten by a rename.
    Destination,
}

impl BlobRole {
    const fn code(self) -> u32 {
        match self {
            Self::Previous => 0,
            Self::Destination => 1,
        }
    }

    const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Previous),
            1 => Some(Self::Destination),
            _ => None,
        }
    }
}

/// Identifier of preserved data within an entry: sixteen lower-case
/// hexadecimal digits, the target index followed by the role.
///
/// Identifiers are derived from the target, so recovery can name the data of
/// every target from the manifest alone.
///
/// # Examples
///
/// ```
/// use nurt_contracts::journal::TargetIndex;
/// use nurt_journal::layout::{BlobId, BlobRole};
///
/// let id = BlobId::new(TargetIndex(3), BlobRole::Previous);
/// assert_eq!(id.to_string(), "0000000300000000");
/// assert_eq!(BlobId::parse("0000000300000000"), Some(id));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobId {
    index: TargetIndex,
    role: BlobRole,
}

/// Number of hexadecimal digits of a blob identifier.
const BLOB_ID_DIGITS: usize = 16;
/// Number of hexadecimal digits of each half of a blob identifier.
const BLOB_ID_HALF: usize = 8;

impl BlobId {
    /// Identifier of the data of `index` in `role`.
    #[must_use]
    pub const fn new(index: TargetIndex, role: BlobRole) -> Self {
        Self { index, role }
    }

    /// Target the data belongs to.
    #[must_use]
    pub const fn index(self) -> TargetIndex {
        self.index
    }

    /// Role of the data.
    #[must_use]
    pub const fn role(self) -> BlobRole {
        self.role
    }

    /// Parses the text form; `None` when it is not sixteen lower-case
    /// hexadecimal digits of a known role.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let lower_hex = |byte: u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
        if text.len() != BLOB_ID_DIGITS || !text.bytes().all(lower_hex) {
            return None;
        }
        let (index, role) = text.split_at(BLOB_ID_HALF);
        let index = u32::from_str_radix(index, 16).ok()?;
        let role = BlobRole::from_code(u32::from_str_radix(role, 16).ok()?)?;
        Some(Self::new(TargetIndex(index), role))
    }

    /// Path of the content file relative to the entry directory, as recorded
    /// in manifests.
    #[must_use]
    pub fn content_name(self) -> String {
        format!("{BLOBS_DIR}/{self}.blob")
    }

    /// Path of the metadata file relative to the entry directory.
    #[must_use]
    pub fn meta_name(self) -> String {
        format!("{BLOBS_DIR}/{self}.meta")
    }

    /// Path of the directory listing relative to the entry directory.
    #[must_use]
    pub fn tree_name(self) -> String {
        format!("{BLOBS_DIR}/{self}.tree")
    }
}

impl std::fmt::Display for BlobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08x}{:08x}", self.index.0, self.role.code())
    }
}

/// Paths inside the journal directory of one volume.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JournalLayout {
    root: AbsPath,
}

impl JournalLayout {
    /// Layout of the journal directory `root`.
    #[must_use]
    pub fn new(root: AbsPath) -> Self {
        Self { root }
    }

    /// Journal directory.
    #[must_use]
    pub fn root(&self) -> &AbsPath {
        &self.root
    }

    /// Format version file.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when the journal directory is
    /// not a valid base for file names; this applies to every path method.
    pub fn version(&self) -> FsResult<AbsPath> {
        self.root.join_name(VERSION_FILE)
    }

    /// Volume description file.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn volume_file(&self) -> FsResult<AbsPath> {
        self.root.join_name(VOLUME_FILE)
    }

    /// Heartbeat file of the space service.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn heartbeat(&self) -> FsResult<AbsPath> {
        self.root.join_name(HEARTBEAT_FILE)
    }

    /// Pin file.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn pins(&self) -> FsResult<AbsPath> {
        self.root.join_name(PINS_FILE)
    }

    /// Directory of lock files.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn lock_dir(&self) -> FsResult<AbsPath> {
        self.root.join_name(LOCK_DIR)
    }

    /// Lock file of journal-wide maintenance.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn journal_lock(&self) -> FsResult<AbsPath> {
        self.lock_dir()?.join_name(JOURNAL_LOCK_FILE)
    }

    /// Lock file held by the transaction of the entry `op`.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn entry_lock(&self, op: OpId) -> FsResult<AbsPath> {
        self.lock_dir()?
            .join_name(&format!("{}{LOCK_SUFFIX}", entry_name(op)))
    }

    /// Directory of markers of non-terminal entries.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn pending_dir(&self) -> FsResult<AbsPath> {
        self.root.join_name(PENDING_DIR)
    }

    /// Marker of the non-terminal entry `op`.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn pending(&self, op: OpId) -> FsResult<AbsPath> {
        self.pending_dir()?.join_name(&entry_name(op))
    }

    /// Directory of entries.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn ops_dir(&self) -> FsResult<AbsPath> {
        self.root.join_name(OPS_DIR)
    }

    /// Entry `op` in `ops/`.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn entry(&self, op: OpId) -> FsResult<EntryLayout> {
        Ok(EntryLayout::new(
            self.ops_dir()?.join_name(&entry_name(op))?,
        ))
    }

    /// Directory of the entry `op` while it is written, before it becomes
    /// prepared.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn entry_tmp(&self, op: OpId) -> FsResult<EntryLayout> {
        let name = format!("{}{TMP_SUFFIX}", entry_name(op));
        Ok(EntryLayout::new(self.ops_dir()?.join_name(&name)?))
    }

    /// Directory of entries being reclaimed.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn reap_dir(&self) -> FsResult<AbsPath> {
        self.root.join_name(REAP_DIR)
    }

    /// Entry `op` moved to the reclaim area.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn reaped(&self, op: OpId) -> FsResult<EntryLayout> {
        Ok(EntryLayout::new(
            self.reap_dir()?.join_name(&entry_name(op))?,
        ))
    }

    /// Directory of tombstones.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn tomb_dir(&self) -> FsResult<AbsPath> {
        self.root.join_name(TOMB_DIR)
    }

    /// Tombstone of the expired entry `op`.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn tomb(&self, op: OpId) -> FsResult<AbsPath> {
        self.tomb_dir()?
            .join_name(&format!("{}{TOMB_SUFFIX}", entry_name(op)))
    }

    /// Directories created inside an existing journal directory before the
    /// first entry is written, in creation order.
    ///
    /// # Errors
    ///
    /// As [`JournalLayout::version`].
    pub fn structure(&self) -> FsResult<[AbsPath; 5]> {
        Ok([
            self.lock_dir()?,
            self.pending_dir()?,
            self.ops_dir()?,
            self.reap_dir()?,
            self.tomb_dir()?,
        ])
    }
}

/// Paths inside the directory of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryLayout {
    dir: AbsPath,
}

impl EntryLayout {
    /// Layout of the entry directory `dir`.
    #[must_use]
    pub fn new(dir: AbsPath) -> Self {
        Self { dir }
    }

    /// Entry directory.
    #[must_use]
    pub fn dir(&self) -> &AbsPath {
        &self.dir
    }

    /// Manifest.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when the entry directory is not
    /// a valid base for file names; this applies to every path method.
    pub fn manifest(&self) -> FsResult<AbsPath> {
        self.dir.join_name(MANIFEST_FILE)
    }

    /// State file.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn state(&self) -> FsResult<AbsPath> {
        self.dir.join_name(STATE_FILE)
    }

    /// Item state file.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn items(&self) -> FsResult<AbsPath> {
        self.dir.join_name(ITEMS_FILE)
    }

    /// Directory of preserved data.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn blobs(&self) -> FsResult<AbsPath> {
        self.dir.join_name(BLOBS_DIR)
    }

    /// Directory of temporary files of atomic writes.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn tmp(&self) -> FsResult<AbsPath> {
        self.dir.join_name(ENTRY_TMP_DIR)
    }

    /// Content file of preserved data.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn blob(&self, id: BlobId) -> FsResult<AbsPath> {
        self.blobs()?.join_name(&format!("{id}.blob"))
    }

    /// Metadata file of preserved data.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn blob_meta(&self, id: BlobId) -> FsResult<AbsPath> {
        self.blobs()?.join_name(&format!("{id}.meta"))
    }

    /// Directory listing of preserved data.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn blob_tree(&self, id: BlobId) -> FsResult<AbsPath> {
        self.blobs()?.join_name(&format!("{id}.tree"))
    }

    /// Directory of content displaced from targets by a rollback after a
    /// crash; such content is moved here, never deleted.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn displaced_dir(&self) -> FsResult<AbsPath> {
        self.dir.join_name(DISPLACED_DIR)
    }

    /// Displaced content of target `index`.
    ///
    /// # Errors
    ///
    /// As [`EntryLayout::manifest`].
    pub fn displaced(&self, index: TargetIndex) -> FsResult<AbsPath> {
        self.displaced_dir()?.join_name(&index.0.to_string())
    }

    /// Resolves a path recorded in a manifest relative to the entry
    /// directory (`blobs/<id>.blob`).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when the recorded path is not
    /// a blob path of the entry.
    pub fn resolve(&self, recorded: &str) -> FsResult<AbsPath> {
        let invalid = || {
            nurt_contracts::fs::FsError::new(
                FsErrorKind::InvalidPath,
                "resolve",
                Some(self.dir.as_path()),
            )
        };
        let (dir, name) = recorded.split_once('/').ok_or_else(invalid)?;
        if dir != BLOBS_DIR {
            return Err(invalid());
        }
        let (stem, extension) = name.split_once('.').ok_or_else(invalid)?;
        if BlobId::parse(stem).is_none() || !matches!(extension, "blob" | "meta" | "tree") {
            return Err(invalid());
        }
        self.blobs()?.join_name(name)
    }
}

/// Paths inside the account state directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateLayout {
    root: AbsPath,
}

impl StateLayout {
    /// Layout of the account state directory `root`.
    #[must_use]
    pub fn new(root: AbsPath) -> Self {
        Self { root }
    }

    /// Account state directory.
    #[must_use]
    pub fn root(&self) -> &AbsPath {
        &self.root
    }

    /// Identifier of the account: the text of its state directory. Entries
    /// record it, because the journal directory of a volume is shared by
    /// every account that works on the volume.
    #[must_use]
    pub fn account_id(&self) -> String {
        self.root.as_path().to_string_lossy().into_owned()
    }

    /// Sequence file: eight bytes of the last assigned number in
    /// little-endian order followed by the CRC-32 of those bytes, also in
    /// little-endian order.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when the state directory is not
    /// a valid base for file names; this applies to every path method.
    pub fn seq(&self) -> FsResult<AbsPath> {
        self.root.join_name(SEQ_FILE)
    }

    /// Account index.
    ///
    /// # Errors
    ///
    /// As [`StateLayout::seq`].
    pub fn index(&self) -> FsResult<AbsPath> {
        self.root.join_name(INDEX_FILE)
    }

    /// Volume registry.
    ///
    /// # Errors
    ///
    /// As [`StateLayout::seq`].
    pub fn volumes(&self) -> FsResult<AbsPath> {
        self.root.join_name(VOLUMES_FILE)
    }

    /// Account lock.
    ///
    /// # Errors
    ///
    /// As [`StateLayout::seq`].
    pub fn account_lock(&self) -> FsResult<AbsPath> {
        self.root.join_name(ACCOUNT_LOCK_FILE)
    }
}

/// Operation numbers of the markers in `pending/`, in ascending order.
///
/// The cost is proportional to the number of markers, not to the number of
/// entries. A missing `pending/` directory yields an empty list; names that
/// are not entry names are ignored.
///
/// # Errors
///
/// Returns the error of listing the directory or of reading a directory
/// entry.
pub fn pending_entries(fs: &dyn Fs, layout: &JournalLayout) -> FsResult<Vec<OpId>> {
    list_entry_names(fs, &layout.pending_dir()?)
}

/// Operation numbers of the entries in `ops/` (prepared or later; directories
/// of entries not yet prepared are excluded), in ascending order.
///
/// Reads one directory listing; used to rebuild the sequence and by the
/// fallback scan of recovery.
///
/// # Errors
///
/// As [`pending_entries`].
pub fn entries(fs: &dyn Fs, layout: &JournalLayout) -> FsResult<Vec<OpId>> {
    list_entry_names(fs, &layout.ops_dir()?)
}

/// Largest operation number named by any entry, temporary entry, reclaimed
/// entry, tombstone or marker of the journal; `None` for an empty journal.
///
/// # Errors
///
/// As [`pending_entries`].
pub fn highest_number(fs: &dyn Fs, layout: &JournalLayout) -> FsResult<Option<OpId>> {
    let mut highest = None;
    for dir in [
        layout.ops_dir()?,
        layout.reap_dir()?,
        layout.tomb_dir()?,
        layout.pending_dir()?,
    ] {
        for name in list_names(fs, &dir)? {
            let stem = name
                .strip_suffix(TMP_SUFFIX)
                .or_else(|| name.strip_suffix(TOMB_SUFFIX))
                .unwrap_or(&name);
            if let Some(op) = parse_entry_name(stem) {
                highest = highest.max(Some(op));
            }
        }
    }
    Ok(highest)
}

fn list_entry_names(fs: &dyn Fs, dir: &AbsPath) -> FsResult<Vec<OpId>> {
    let mut ops: Vec<OpId> = list_names(fs, dir)?
        .iter()
        .filter_map(|name| parse_entry_name(name))
        .collect();
    ops.sort_unstable();
    Ok(ops)
}

fn list_names(fs: &dyn Fs, dir: &AbsPath) -> FsResult<Vec<String>> {
    let entries = match fs.read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    for entry in entries {
        if let Some(name) = entry?.name.to_str() {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> AbsPath {
        AbsPath::new(if cfg!(windows) { "C:\\j" } else { "/j" }).unwrap()
    }

    #[test]
    fn entry_names_have_twelve_digits_and_round_trip() {
        assert_eq!(entry_name(OpId(0)), "000000000000");
        assert_eq!(entry_name(OpId(1_000_000_000_000)), "1000000000000");
        for op in [1, 18, 999_999_999_999, 1_000_000_000_000] {
            assert_eq!(parse_entry_name(&entry_name(OpId(op))), Some(OpId(op)));
        }
    }

    #[test]
    fn rejects_names_that_are_not_entries() {
        for name in ["18", "00000000001x", "000000000018.tmp", "", "-00000000001"] {
            assert_eq!(parse_entry_name(name), None, "{name}");
        }
    }

    #[test]
    fn staged_names_are_derived_from_operation_and_target() {
        assert_eq!(
            parse_staged_name(".nurt-tmp-12-0"),
            Some((OpId(12), TargetIndex(0)))
        );
        for name in [
            ".nurt-tmp-12",
            ".nurt-tmp--1",
            ".nurt-tmp-a-1",
            "x.nurt-tmp-1-1",
        ] {
            assert_eq!(parse_staged_name(name), None, "{name}");
        }
    }

    #[test]
    fn blob_ids_encode_target_and_role() {
        let id = BlobId::new(TargetIndex(0x1234), BlobRole::Destination);
        assert_eq!(id.to_string(), "0000123400000001");
        assert_eq!(BlobId::parse(&id.to_string()), Some(id));
        assert_eq!(id.content_name(), "blobs/0000123400000001.blob");
        assert_eq!(id.meta_name(), "blobs/0000123400000001.meta");
        assert_eq!(id.tree_name(), "blobs/0000123400000001.tree");
        for text in [
            "000012340000000",
            "0000123400000002",
            "0000123400000O01",
            "ABCDEF0000000000",
        ] {
            assert_eq!(BlobId::parse(text), None, "{text}");
        }
    }

    #[test]
    fn entry_paths_are_inside_the_journal_directory() {
        let layout = JournalLayout::new(root());
        let entry = layout.entry(OpId(18)).unwrap();
        let id = BlobId::new(TargetIndex(1), BlobRole::Previous);
        let paths = [
            (entry.dir().clone(), "ops/000000000018"),
            (
                layout.entry_tmp(OpId(18)).unwrap().dir().clone(),
                "ops/000000000018.tmp",
            ),
            (entry.manifest().unwrap(), "000000000018/op.json"),
            (entry.state().unwrap(), "000000000018/STATE"),
            (entry.items().unwrap(), "000000000018/ITEMS"),
            (entry.tmp().unwrap(), "000000000018/tmp"),
            (entry.blob(id).unwrap(), "blobs/0000000100000000.blob"),
            (entry.blob_meta(id).unwrap(), "blobs/0000000100000000.meta"),
            (entry.blob_tree(id).unwrap(), "blobs/0000000100000000.tree"),
            (entry.displaced(TargetIndex(4)).unwrap(), "displaced/4"),
            (layout.pending(OpId(3)).unwrap(), "pending/000000000003"),
            (
                layout.entry_lock(OpId(3)).unwrap(),
                "lock/000000000003.lock",
            ),
            (layout.journal_lock().unwrap(), "lock/journal.lock"),
            (layout.tomb(OpId(3)).unwrap(), "tomb/000000000003.json"),
            (
                layout.reaped(OpId(3)).unwrap().dir().clone(),
                "reap/000000000003",
            ),
            (layout.version().unwrap(), VERSION_FILE),
            (layout.volume_file().unwrap(), VOLUME_FILE),
            (layout.heartbeat().unwrap(), HEARTBEAT_FILE),
            (layout.pins().unwrap(), PINS_FILE),
        ];
        for (path, suffix) in &paths {
            assert!(
                path.as_path().ends_with(suffix),
                "{} does not end with {suffix}",
                path.as_path().display()
            );
        }
        assert_eq!(
            entry.resolve(&id.content_name()).unwrap(),
            entry.blob(id).unwrap()
        );
        assert_eq!(layout.structure().unwrap().len(), 5);
        assert_eq!(layout.root(), &root());
    }

    #[test]
    fn resolve_accepts_only_blob_paths() {
        let entry = JournalLayout::new(root()).entry(OpId(1)).unwrap();
        for recorded in [
            "op.json",
            "blobs/../op.json",
            "blobs/x.blob",
            "tmp/0000000000000000.blob",
        ] {
            assert_eq!(
                entry.resolve(recorded).unwrap_err().kind,
                FsErrorKind::InvalidPath,
                "{recorded}"
            );
        }
        assert!(entry.resolve("blobs/0000000000000000.tree").is_ok());
    }

    #[test]
    fn state_paths_are_inside_the_state_directory() {
        let state = StateLayout::new(root());
        assert!(state.seq().unwrap().as_path().ends_with(SEQ_FILE));
        assert!(state.index().unwrap().as_path().ends_with(INDEX_FILE));
        assert!(state.volumes().unwrap().as_path().ends_with(VOLUMES_FILE));
        assert!(
            state
                .account_lock()
                .unwrap()
                .as_path()
                .ends_with(ACCOUNT_LOCK_FILE)
        );
        assert_eq!(state.root(), &root());
    }

    #[test]
    fn default_name_depends_on_the_host_system() {
        let expected = if cfg!(target_os = "macos") {
            DIR_NAME_MACOS
        } else {
            DIR_NAME
        };
        assert_eq!(default_dir_name(), expected);
    }

    #[test]
    fn rejects_entry_names_with_superfluous_leading_zeros_or_foreign_digits() {
        for name in [
            "0000000000018",
            "00000000000000000018",
            "0000000000a8",
            "００００００００００１８",
            "00000000001é",
            "000000000018\n",
            "000000000018 ",
        ] {
            assert_eq!(parse_entry_name(name), None, "{name:?}");
        }
        assert_eq!(parse_entry_name("000000000018"), Some(OpId(18)));
        assert_eq!(
            parse_entry_name("18446744073709551615"),
            Some(OpId(u64::MAX))
        );
        assert_eq!(parse_entry_name("18446744073709551616"), None);
        assert_eq!(parse_entry_name("99999999999999999999999"), None);
    }

    #[test]
    fn entry_names_of_large_numbers_round_trip() {
        for op in [999_999_999_999, 1_000_000_000_000, u64::MAX] {
            let name = entry_name(OpId(op));
            assert!(name.len() >= ENTRY_DIGITS, "{name}");
            assert_eq!(parse_entry_name(&name), Some(OpId(op)), "{name}");
        }
    }

    #[test]
    fn rejects_staged_names_with_overflowing_or_foreign_digits() {
        for name in [
            ".nurt-tmp-18446744073709551616-0",
            ".nurt-tmp-1-4294967296",
            ".nurt-tmp-1-",
            ".nurt-tmp--",
            ".nurt-tmp-1-1-1",
            ".nurt-tmp-１-1",
            "",
        ] {
            assert_eq!(parse_staged_name(name), None, "{name:?}");
        }
        assert_eq!(
            parse_staged_name(".nurt-tmp-0-4294967295"),
            Some((OpId(0), TargetIndex(u32::MAX)))
        );
    }

    #[test]
    fn rejects_blob_names_with_uppercase_signs_or_multibyte_characters() {
        for text in [
            "",
            "0000000000000000 ",
            "00000000000000000",
            "0000000A00000000",
            "0000000é00000000",
            "ffffffff00000002",
            "+000000100000000",
        ] {
            assert_eq!(BlobId::parse(text), None, "{text:?}");
        }
        let largest = BlobId::new(TargetIndex(u32::MAX), BlobRole::Destination);
        assert_eq!(largest.to_string(), "ffffffff00000001");
        assert_eq!(BlobId::parse("ffffffff00000001"), Some(largest));
    }

    #[test]
    fn resolve_rejects_paths_that_leave_the_blob_directory() {
        let entry = JournalLayout::new(root()).entry(OpId(1)).unwrap();
        for recorded in [
            "",
            "blobs",
            "blobs/",
            "blobs/0000000000000000",
            "blobs/0000000000000000.blob/../op.json",
            "blobs/0000000000000000.exe",
            "blobs/0000000000000000.blob.blob",
            "/blobs/0000000000000000.blob",
            "blobs\\0000000000000000.blob",
            "bloßs/0000000000000000.blob",
        ] {
            assert_eq!(
                entry.resolve(recorded).unwrap_err().kind,
                FsErrorKind::InvalidPath,
                "{recorded:?}"
            );
        }
        assert!(entry.resolve("blobs/0000000100000001.meta").is_ok());
    }

    fn journal_fs() -> nurt_fs::mem::MemFs {
        use nurt_fs::mem::{MemFs, VolumeSpec};
        MemFs::builder()
            .volume(VolumeSpec::posix(AbsPath::new("/v").unwrap()))
            .build()
            .unwrap()
    }

    fn seeded(fs: &nurt_fs::mem::MemFs, path: &str) {
        fs.seed_file(&AbsPath::new(path).unwrap(), b"x").unwrap();
    }

    #[test]
    fn listings_of_a_missing_journal_are_empty() {
        let fs = journal_fs();
        let journal = JournalLayout::new(AbsPath::new("/v/j").unwrap());
        assert_eq!(pending_entries(&fs, &journal), Ok(Vec::new()));
        assert_eq!(entries(&fs, &journal), Ok(Vec::new()));
        assert_eq!(highest_number(&fs, &journal), Ok(None));
    }

    #[test]
    fn listings_return_sorted_entry_numbers_and_ignore_foreign_names() {
        let fs = journal_fs();
        let journal = JournalLayout::new(AbsPath::new("/v/j").unwrap());
        for path in [
            "/v/j/ops/000000000010/STATE",
            "/v/j/ops/000000000002/STATE",
            "/v/j/ops/000000000007.tmp/op.json",
            "/v/j/ops/00000000000x/STATE",
            "/v/j/ops/ż000000000003/STATE",
            "/v/j/pending/000000000005",
            "/v/j/pending/notes",
        ] {
            seeded(&fs, path);
        }
        assert_eq!(
            entries(&fs, &journal),
            Ok(vec![OpId(2), OpId(10)]),
            "temporary entries are not entries"
        );
        assert_eq!(pending_entries(&fs, &journal), Ok(vec![OpId(5)]));
    }

    #[test]
    fn highest_number_covers_every_directory_of_the_journal() {
        let fs = journal_fs();
        let journal = JournalLayout::new(AbsPath::new("/v/j").unwrap());
        seeded(&fs, "/v/j/ops/000000000003/STATE");
        assert_eq!(highest_number(&fs, &journal), Ok(Some(OpId(3))));
        seeded(&fs, "/v/j/ops/000000000004.tmp/op.json");
        assert_eq!(highest_number(&fs, &journal), Ok(Some(OpId(4))));
        seeded(&fs, "/v/j/pending/000000000005");
        assert_eq!(highest_number(&fs, &journal), Ok(Some(OpId(5))));
        seeded(&fs, "/v/j/reap/000000000006/STATE");
        assert_eq!(highest_number(&fs, &journal), Ok(Some(OpId(6))));
        seeded(&fs, "/v/j/tomb/000000000009.json");
        assert_eq!(highest_number(&fs, &journal), Ok(Some(OpId(9))));
        seeded(&fs, "/v/j/tomb/000000000012.json.tmp");
        seeded(&fs, "/v/j/ops/readme");
        assert_eq!(highest_number(&fs, &journal), Ok(Some(OpId(9))));
    }
}
