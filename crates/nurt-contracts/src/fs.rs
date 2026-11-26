// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! File system interface.
//!
//! The interface is split in two layers:
//!
//! * [`Fs`] is the read-only interface. Every command implementation receives
//!   a `&dyn Fs` and uses it to inspect entries, read content and describe
//!   volumes.
//! * [`FsWrite`] performs mutations. Every mutating method requires a
//!   [`TxScope`], a proof that a mutation transaction is active. Scopes are
//!   created only by journal implementations, so command implementations never
//!   change data directly: they change data only through
//!   [`MutationTx`](crate::journal::MutationTx).
//!
//! Non-file resources addressed like paths (for example `registry:` or `env:`)
//! are served by a [`Provider`]; writable providers implement
//! [`ProviderWrite`], which also requires a [`TxScope`].
//!
//! Paths are absolute ([`AbsPath`]); the language has no current directory.
//! Errors are reported as [`FsError`] with a classification ([`FsErrorKind`])
//! that maps to a stable language error code.
//!
//! # Test implementations
//!
//! A virtual file system used in tests implements [`Fs`] and [`FsWrite`] with
//! the reference semantics documented in the journal specification of the
//! repository (section "Reference semantics for test implementations"): one
//! error kind per failure condition, deterministic directory order, entity
//! tags derived from size, modification time and file identity, and
//! timestamps taken from the injected clock.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::journal::OpId;

/// Result alias of the file system layer.
pub type FsResult<T> = std::result::Result<T, FsError>;

/// Absolute, lexically normalised path in the native form of the host system.
///
/// Invariants: the path is absolute and contains no `.` or `..` components.
/// Symbolic links are not resolved. On Windows the long-path prefix (`\\?\`)
/// is added by the back end when calling the system and is never part of an
/// `AbsPath`.
///
/// Serialization uses the language form ([`AbsPath::to_language_string`]):
/// `/` separators and an upper-case drive letter on Windows. Names that are
/// not valid Unicode are serialized lossily; implementations that persist such
/// names keep their raw form separately.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbsPath(PathBuf);

impl AbsPath {
    /// Creates an absolute path after lexical normalisation.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when the path is relative or
    /// when a `..` component would escape the root.
    pub fn new(path: impl Into<PathBuf>) -> FsResult<Self> {
        let path: PathBuf = path.into();
        if !path.is_absolute() {
            return Err(FsError::new(
                FsErrorKind::InvalidPath,
                "abs_path",
                Some(&path),
            ));
        }
        let mut out = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    let at_root = out.parent().is_none();
                    if at_root || !out.pop() {
                        return Err(FsError::new(
                            FsErrorKind::InvalidPath,
                            "abs_path",
                            Some(&path),
                        ));
                    }
                }
                other => out.push(other.as_os_str()),
            }
        }
        Ok(Self(out))
    }

    /// Native path for system calls.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Parent directory; `None` for a root.
    pub fn parent(&self) -> Option<AbsPath> {
        self.0.parent().map(|parent| AbsPath(parent.to_path_buf()))
    }

    /// Last component; `None` for a root.
    pub fn file_name(&self) -> Option<&OsStr> {
        self.0.file_name()
    }

    /// Joins a single file name.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InvalidPath`] when `name` is empty, is `.`
    /// or `..`, or contains a separator. On Windows a colon is rejected as
    /// well, because a name that starts with a drive prefix replaces the
    /// whole path when it is joined.
    pub fn join_name(&self, name: &str) -> FsResult<AbsPath> {
        let invalid = name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || (cfg!(windows) && name.contains(['\\', ':']));
        if invalid {
            return Err(FsError::new(
                FsErrorKind::InvalidPath,
                "join_name",
                Some(&self.0),
            ));
        }
        Ok(AbsPath(self.0.join(name)))
    }

    /// Whether `self` equals `other` or lies below it (lexical comparison).
    pub fn starts_with(&self, other: &AbsPath) -> bool {
        self.0.starts_with(&other.0)
    }

    /// Language form: `/` separators, upper-case drive letter on Windows.
    pub fn to_language_string(&self) -> String {
        let text = self.0.to_string_lossy();
        if cfg!(windows) {
            let mut text = text.replace('\\', "/");
            if text.as_bytes().get(1) == Some(&b':')
                && let Some(drive) = text.get_mut(..1)
            {
                drive.make_ascii_uppercase();
            }
            text
        } else {
            text.into_owned()
        }
    }
}

impl fmt::Debug for AbsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AbsPath({})", self.to_language_string())
    }
}

impl fmt::Display for AbsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_language_string())
    }
}

impl AsRef<Path> for AbsPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Serialize for AbsPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_language_string())
    }
}

impl<'de> Deserialize<'de> for AbsPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        AbsPath::new(PathBuf::from(text)).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for AbsPath {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("AbsPath")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "Absolute, normalised path in language form (`/` separators)."
        })
    }
}

/// Identity key of a path on its volume, used to detect that two spellings
/// denote the same entry (case-insensitive volumes, Unicode normalisation).
///
/// Linux uses raw bytes. macOS and Windows use NFC followed by case folding
/// when the volume is case-insensitive. Produced by [`Fs::path_key`].
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct PathKey(pub Vec<u8>);

/// SHA-256 digest of content.
///
/// The text form is `sha256:` followed by 64 lower-case hexadecimal digits;
/// it is also the serialized form.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    /// Prefix of the text form.
    pub const PREFIX: &'static str = "sha256:";

    /// Parses the text form.
    ///
    /// # Errors
    ///
    /// Fails with [`ContentHashError`] when the prefix is missing, the length
    /// is not 64 digits or a digit is not a lower-case hexadecimal digit.
    pub fn parse(text: &str) -> std::result::Result<Self, ContentHashError> {
        let digits = text.strip_prefix(Self::PREFIX).ok_or(ContentHashError)?;
        let digits = digits.as_bytes();
        let mut out = [0_u8; 32];
        if digits.len() != 2 * out.len() {
            return Err(ContentHashError);
        }
        let (pairs, _) = digits.as_chunks::<2>();
        for (slot, &[high, low]) in out.iter_mut().zip(pairs) {
            let high = hex_value(high).ok_or(ContentHashError)?;
            let low = hex_value(low).ok_or(ContentHashError)?;
            *slot = (high << 4) | low;
        }
        Ok(Self(out))
    }
}

const fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::PREFIX)?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl std::str::FromStr for ContentHash {
    type Err = ContentHashError;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl From<ContentHash> for String {
    fn from(hash: ContentHash) -> Self {
        hash.to_string()
    }
}

impl TryFrom<String> for ContentHash {
    type Error = ContentHashError;

    fn try_from(text: String) -> std::result::Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl JsonSchema for ContentHash {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ContentHash")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^sha256:[0-9a-f]{64}$",
            "description": "SHA-256 digest: `sha256:` followed by 64 lower-case hexadecimal digits."
        })
    }
}

/// Error of [`ContentHash::parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentHashError;

impl fmt::Display for ContentHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected `sha256:` followed by 64 lower-case hexadecimal digits")
    }
}

impl std::error::Error for ContentHashError {}

/// Platform file identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct FileId {
    /// Device number (Unix `st_dev`) or volume serial number (Windows).
    pub volume: u64,
    /// Inode number (Unix) or low part of the 128-bit file id (Windows).
    pub low: u64,
    /// High part of the 128-bit file id (Windows); zero on Unix.
    pub high: u64,
}

/// Entity tag of a file as observed by a read stage.
///
/// Carried in records from the read stage to the mutation stage. Two tags
/// match when size, modification time and file identity are equal; `hash` is
/// compared only when both tags carry it ([`Etag::matches`]). A mismatch
/// between the tag and the state at mutation time yields `E_STALE`. When
/// `hash` is present, the journal reuses it as the hash of the previous
/// content without reading the file again.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct Etag {
    /// Size in bytes.
    pub size: u64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub modified_ns: i64,
    /// Platform file identity.
    pub file_id: FileId,
    /// Content hash, when the read stage computed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<ContentHash>,
}

impl Etag {
    /// Whether the tag denotes the same state as `other`, by the rule given in
    /// the type documentation.
    pub fn matches(&self, other: &Etag) -> bool {
        let hashes_agree = match (&self.hash, &other.hash) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        };
        self.size == other.size
            && self.modified_ns == other.modified_ns
            && self.file_id == other.file_id
            && hashes_agree
    }
}

/// Kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
    /// Windows junction (mount-point reparse point).
    Junction,
    /// Device, FIFO, socket or other special entry.
    Special,
}

/// Whether to follow a symbolic link in the last path component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Follow {
    /// Report the link itself (`lstat`, `FILE_FLAG_OPEN_REPARSE_POINT`).
    NoFollow,
    /// Report the link target.
    Follow,
}

/// Basic metadata of an entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileMeta {
    /// Entry kind.
    pub kind: FileKind,
    /// Logical size in bytes.
    pub size: u64,
    /// Unix permission bits; on Windows synthesised from the read-only
    /// attribute.
    pub mode: u32,
    /// User id of the file (Unix); `None` on Windows, where
    /// [`ExtendedMeta::account_sid`] identifies the account.
    pub uid: Option<u32>,
    /// Group id of the file (Unix).
    pub gid: Option<u32>,
    /// Number of hard links.
    pub links: u64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub modified_ns: i64,
    /// Access time, nanoseconds since the Unix epoch.
    pub accessed_ns: Option<i64>,
    /// Birth time, when the system reports it.
    pub created_ns: Option<i64>,
    /// Status change time (Unix `ctime`), when the system reports it.
    pub changed_ns: Option<i64>,
    /// File identity.
    pub file_id: FileId,
    /// Windows file attribute bits (`FILE_ATTRIBUTE_*`); zero on Unix.
    pub windows_attributes: u32,
}

impl FileMeta {
    /// Entity tag of this state, without content hash.
    pub fn etag(&self) -> Etag {
        Etag {
            size: self.size,
            modified_ns: self.modified_ns,
            file_id: self.file_id,
            hash: None,
        }
    }
}

/// One extended attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Xattr {
    /// Attribute name including namespace (for example `user.mime`,
    /// `security.selinux`, `com.apple.quarantine`).
    pub name: OsString,
    /// Raw value.
    pub value: Vec<u8>,
}

/// One NTFS alternate data stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AlternateStream {
    /// Stream name without the leading colon and type suffix.
    pub name: String,
    /// Stream size in bytes.
    pub size: u64,
}

/// Complete metadata needed to restore an entry exactly.
///
/// Stored next to preserved content and compared in the metadata round-trip
/// tests (before the change and after undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtendedMeta {
    /// Basic metadata.
    pub basic: FileMeta,
    /// Extended attributes, including POSIX ACL attributes on Linux.
    pub xattrs: Vec<Xattr>,
    /// Platform ACL in a serialised platform form (macOS `acl_t` text,
    /// Windows self-relative security descriptor with DACL and SACL).
    pub acl: Option<Vec<u8>>,
    /// Security identifier of the account of the file (Windows), in string
    /// form.
    pub account_sid: Option<String>,
    /// File flags (Linux `FS_IOC_GETFLAGS`, BSD `st_flags`).
    pub flags: Option<u64>,
    /// NTFS alternate data streams (content is preserved separately).
    pub streams: Vec<AlternateStream>,
    /// Target of a symbolic link or junction.
    pub link_target: Option<PathBuf>,
}

/// Change of metadata requested by a command.
///
/// Every field is optional; absent fields are left unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MetaChange {
    /// New permission bits.
    pub mode: Option<u32>,
    /// New user id.
    pub uid: Option<u32>,
    /// New group id.
    pub gid: Option<u32>,
    /// New modification time, nanoseconds since the Unix epoch.
    pub modified_ns: Option<i64>,
    /// New access time, nanoseconds since the Unix epoch.
    pub accessed_ns: Option<i64>,
    /// New birth time, where the system allows setting it.
    pub created_ns: Option<i64>,
    /// Extended attributes to set.
    pub set_xattrs: Vec<Xattr>,
    /// Names of extended attributes to remove.
    pub remove_xattrs: Vec<OsString>,
    /// Replacement ACL in platform form.
    pub acl: Option<Vec<u8>>,
    /// Windows attribute bits to set.
    pub set_windows_attributes: u32,
    /// Windows attribute bits to clear.
    pub clear_windows_attributes: u32,
}

/// Result of transferring metadata from an old entry to its replacement.
///
/// Used by the exchange overwrite strategy: when any element could not be
/// transferred, the strategy is abandoned and the next one is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MetaTransferReport {
    /// Elements that could not be transferred (for example `account`,
    /// `xattr:security.selinux`, `stream:Summary`).
    pub not_transferred: Vec<String>,
}

impl MetaTransferReport {
    /// True when every element was transferred.
    pub fn is_complete(&self) -> bool {
        self.not_transferred.is_empty()
    }
}

/// One entry returned by [`Fs::read_dir`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DirEntry {
    /// Entry name as stored by the file system.
    pub name: OsString,
    /// Entry kind, when known without an extra system call.
    pub kind: Option<FileKind>,
}

/// Identity of a volume (file system instance).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct VolumeId {
    /// Device number or volume serial number.
    pub dev: u64,
    /// File system UUID when available; stable across reboots.
    pub uuid: Option<String>,
    /// Btrfs subvolume id, or zero.
    pub subvolume: u64,
}

/// Class of a volume; decides whether the journal is enabled by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VolumeKind {
    /// Local fixed disk.
    Fixed,
    /// Removable medium.
    Removable,
    /// Network file system.
    Network,
    /// Memory-backed file system (`tmpfs`, `ramfs`).
    Memory,
    /// Overlay file system (container root). The journal is enabled by
    /// default; entries that live only in a lower layer are preserved by copy,
    /// because renaming them across layers fails with a cross-device error.
    Overlay,
    /// Anything else.
    Other,
}

/// File system capabilities relevant to commands and preservation strategies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FsFeatures {
    /// Atomic exchange of two names (`RENAME_EXCHANGE`, `RENAME_SWAP`).
    pub exchange: bool,
    /// Rename that fails when the destination exists.
    pub rename_noreplace: bool,
    /// Replace with backup name (`ReplaceFileW`).
    pub replace_with_backup: bool,
    /// Hard links.
    pub hardlink: bool,
    /// Block sharing clones (`FICLONE`, `clonefile`, block cloning on ReFS).
    pub reflink: bool,
    /// Extended attributes.
    pub xattr: bool,
    /// ACLs.
    pub acl: bool,
    /// NTFS alternate data streams.
    pub alternate_streams: bool,
    /// Durable metadata after a flush (false for FAT and some network shares).
    pub durable_metadata: bool,
    /// Case-sensitive names.
    pub case_sensitive: bool,
    /// Names are stored decomposed (NFD), as on HFS+.
    pub decomposes_names: bool,
}

/// Description of a volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VolumeInfo {
    /// Volume identity.
    pub id: VolumeId,
    /// Mount point (Unix) or volume root (Windows).
    pub mount: AbsPath,
    /// File system type name (`ext4`, `apfs`, `ntfs`, `overlay`, and so on).
    pub fs_type: String,
    /// Volume class.
    pub kind: VolumeKind,
    /// Capabilities.
    pub features: FsFeatures,
    /// Rotational medium (affects reclaim parallelism).
    pub rotational: bool,
    /// Read-only mount.
    pub read_only: bool,
}

/// Space figures of a volume as seen by ordinary programs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpaceInfo {
    /// Total capacity, or the account quota when smaller.
    pub total: u64,
    /// Free bytes available to the caller, after quotas.
    pub available: u64,
    /// Remaining account quota, when quotas are enforced.
    pub quota_remaining: Option<u64>,
    /// Size of the account quota, when quotas are enforced; reported together
    /// with `quota_remaining`.
    #[serde(default)]
    pub quota_limit: Option<u64>,
    /// Purgeable bytes reported by the system (macOS); informational only.
    pub purgeable_system: Option<u64>,
}

/// Process holding a file open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Holder {
    /// Process id.
    pub pid: u32,
    /// Process name.
    pub name: String,
}

/// Classified file system error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FsErrorKind {
    /// Entry does not exist.
    NotFound,
    /// Access denied by permissions.
    PermissionDenied,
    /// Operation requires a privilege the account lacks.
    Privilege,
    /// Destination exists.
    Exists,
    /// Entry is in use (Windows sharing violation, busy mount).
    InUse,
    /// No space left on the volume.
    NoSpace,
    /// Account quota exceeded.
    QuotaExceeded,
    /// Source and destination are on different volumes or layers.
    CrossDevice,
    /// Operation not supported by the file system or system version.
    Unsupported,
    /// The entry changed since it was read.
    Stale,
    /// Read-only file system.
    ReadOnly,
    /// Path is invalid for this system.
    InvalidPath,
    /// A directory was expected.
    NotADirectory,
    /// A non-directory was expected.
    IsADirectory,
    /// Directory not empty.
    NotEmpty,
    /// Name or path too long.
    NameTooLong,
    /// Too many levels of symbolic links.
    Loop,
    /// Interrupted; may be retried.
    Interrupted,
    /// Any other input/output error.
    Io,
}

impl FsErrorKind {
    /// Stable identifier of the language error code for this kind, as
    /// returned by [`ErrorCode::as_str`](crate::error::ErrorCode::as_str).
    ///
    /// Kinds without a code of their own map to `E_IO`; the kind itself is
    /// kept in the error details so that callers can distinguish them.
    pub fn code_id(self) -> &'static str {
        match self {
            Self::NotFound => "E_NOT_FOUND",
            Self::PermissionDenied | Self::ReadOnly => "E_PERMISSION",
            Self::Privilege => "E_PRIVILEGE",
            Self::Exists => "E_EXISTS",
            Self::InUse => "E_IN_USE",
            Self::NoSpace | Self::QuotaExceeded => "E_NO_SPACE",
            Self::Stale => "E_STALE",
            Self::CrossDevice
            | Self::Unsupported
            | Self::InvalidPath
            | Self::NotADirectory
            | Self::IsADirectory
            | Self::NotEmpty
            | Self::NameTooLong
            | Self::Loop
            | Self::Interrupted
            | Self::Io => "E_IO",
        }
    }

    /// Stable snake-case name of the kind, used in error details.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::PermissionDenied => "permission_denied",
            Self::Privilege => "privilege",
            Self::Exists => "exists",
            Self::InUse => "in_use",
            Self::NoSpace => "no_space",
            Self::QuotaExceeded => "quota_exceeded",
            Self::CrossDevice => "cross_device",
            Self::Unsupported => "unsupported",
            Self::Stale => "stale",
            Self::ReadOnly => "read_only",
            Self::InvalidPath => "invalid_path",
            Self::NotADirectory => "not_a_directory",
            Self::IsADirectory => "is_a_directory",
            Self::NotEmpty => "not_empty",
            Self::NameTooLong => "name_too_long",
            Self::Loop => "loop",
            Self::Interrupted => "interrupted",
            Self::Io => "io",
        }
    }
}

/// File system error with context.
///
/// Converted to the language error ([`Error`](crate::error::Error)) with the
/// code given by [`FsErrorKind::code_id`]; `kind`, `op`, `path`, `os_error`
/// and `holders` become error details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FsError {
    /// Classification.
    pub kind: FsErrorKind,
    /// Name of the failing operation (for example `rename`, `exchange`).
    pub op: String,
    /// Path concerned.
    pub path: Option<PathBuf>,
    /// Raw operating system error code.
    pub os_code: Option<i32>,
    /// Processes holding the entry, for [`FsErrorKind::InUse`].
    pub holders: Vec<Holder>,
}

impl FsError {
    /// Creates an error without an operating system code.
    pub fn new(kind: FsErrorKind, op: &str, path: Option<&Path>) -> Self {
        Self {
            kind,
            op: op.to_owned(),
            path: path.map(Path::to_path_buf),
            os_code: None,
            holders: Vec::new(),
        }
    }

    /// Stable identifier of the language error code, as given by
    /// [`FsErrorKind::code_id`].
    pub fn code_id(&self) -> &'static str {
        self.kind.code_id()
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) in {}",
            self.code_id(),
            self.kind.as_str(),
            self.op
        )?;
        if let Some(path) = &self.path {
            write!(f, ": {}", path.display())?;
        }
        Ok(())
    }
}

impl std::error::Error for FsError {}

/// Durability level of a flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Durability {
    /// Data and the metadata needed to read it (`fdatasync`).
    Data,
    /// Data and all metadata; on macOS `F_FULLFSYNC`, on Windows
    /// `FlushFileBuffers`.
    Full,
}

/// Rename mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenameMode {
    /// Replace an existing destination atomically.
    Replace,
    /// Fail with [`FsErrorKind::Exists`] when the destination exists.
    NoReplace,
}

/// Kind of link to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Symbolic link to a file.
    SymlinkFile,
    /// Symbolic link to a directory.
    SymlinkDirectory,
    /// Windows junction.
    Junction,
}

/// Statistics of a recursive removal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoveStats {
    /// Files removed.
    pub files: u64,
    /// Directories removed.
    pub directories: u64,
    /// Logical bytes removed.
    pub bytes: u64,
}

/// Open file for reading.
pub trait ReadHandle: io::Read + io::Seek + Send + Sync {
    /// Metadata of the open file.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Io`] when the handle cannot be queried.
    fn meta(&self) -> FsResult<FileMeta>;

    /// Whole content mapped in memory, when the back end provides a mapping.
    ///
    /// The default returns `None`; callers then read through [`io::Read`].
    fn as_bytes(&self) -> Option<&[u8]> {
        None
    }
}

/// Open file for writing, created by [`FsWrite::create_temp_in`],
/// [`FsWrite::open_in_place`] or
/// [`MutationTx::stage`](crate::journal::MutationTx::stage).
pub trait WriteHandle: io::Write + io::Seek + Send + Sync {
    /// Path of the file.
    fn path(&self) -> &AbsPath;

    /// Flushes to stable storage.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Io`] or [`FsErrorKind::NoSpace`].
    fn sync(&mut self, durability: Durability) -> FsResult<()>;

    /// Sets the file length.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Io`] or [`FsErrorKind::NoSpace`].
    fn set_length(&mut self, length: u64) -> FsResult<()>;

    /// Applies metadata through the open handle (`fchown`, `fchmod`,
    /// `fsetxattr`, security descriptor) and reports what could not be
    /// applied.
    ///
    /// # Errors
    ///
    /// Fails with an [`FsError`] when the handle itself fails; elements that
    /// are refused individually are listed in the report instead.
    fn apply_meta(&mut self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport>;
}

/// Advisory exclusive lock (`flock`, `LockFileEx`); released on drop.
pub trait FileLock: Send + Sync {
    /// Path of the lock file.
    fn path(&self) -> &AbsPath;
}

/// Proof that a mutation transaction is active.
///
/// Every [`FsWrite`] and [`ProviderWrite`] method requires a scope. Only
/// journal and space implementations create scopes; other crates are denied
/// the constructor by the workspace lint configuration. In test builds the
/// auditing file system wrapper verifies that the operation of the scope is
/// in progress and panics otherwise, which detects mutations made outside a
/// transaction.
///
/// A scope is a capability, not data: it is neither cloneable nor
/// serializable.
#[derive(Debug)]
pub struct TxScope {
    op: OpId,
    purpose: ScopePurpose,
}

/// Why a scope exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopePurpose {
    /// Preserving and applying a user operation.
    Operation,
    /// Recovery after a crash.
    Recovery,
    /// Reclaiming expired entries.
    Reclaim,
    /// Maintaining the journal directory itself.
    Maintenance,
}

impl TxScope {
    /// Creates a scope. Reserved for journal and space implementations.
    pub fn new_for_journal(op: OpId, purpose: ScopePurpose) -> Self {
        Self { op, purpose }
    }

    /// Operation the scope belongs to.
    pub fn op(&self) -> OpId {
        self.op
    }

    /// Purpose of the scope.
    pub fn purpose(&self) -> ScopePurpose {
        self.purpose
    }
}

/// Read-only file system interface available to all command implementations.
///
/// Every method takes absolute paths and never changes the file system.
/// Implementations are platform back ends and virtual file systems for tests.
///
/// Every method reports failures as [`FsError`]. The kind for each failure
/// condition is the same on all systems: a missing entry or missing parent is
/// [`FsErrorKind::NotFound`], a file used as a directory component is
/// [`FsErrorKind::NotADirectory`], a refused access is
/// [`FsErrorKind::PermissionDenied`], and a sharing violation is
/// [`FsErrorKind::InUse`] with `holders` filled when known.
pub trait Fs: Send + Sync {
    /// Metadata of an entry.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`], [`FsErrorKind::NotADirectory`],
    /// [`FsErrorKind::PermissionDenied`], [`FsErrorKind::Loop`] (with
    /// [`Follow::Follow`]) or [`FsErrorKind::Io`].
    fn metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<FileMeta>;

    /// Complete metadata for exact restoration (extended attributes, ACL,
    /// flags, alternate streams, link target).
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::metadata`]; elements unsupported by the volume are
    /// returned empty, not as errors.
    fn extended_metadata(&self, path: &AbsPath, follow: Follow) -> FsResult<ExtendedMeta>;

    /// Opens a file for reading, following symbolic links.
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::metadata`], with [`FsErrorKind::IsADirectory`] for a
    /// directory and with [`FsErrorKind::InUse`] for a file locked against
    /// reading.
    fn open_read(&self, path: &AbsPath) -> FsResult<Box<dyn ReadHandle>>;

    /// Opens an alternate data stream for reading.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`] outside Windows or on volumes
    /// without streams, and with [`FsErrorKind::NotFound`] for a missing
    /// stream.
    fn open_stream_read(&self, path: &AbsPath, stream: &str) -> FsResult<Box<dyn ReadHandle>>;

    /// Lists a directory without recursion.
    ///
    /// The order is unspecified for platform back ends (callers sort); test
    /// implementations return entries in byte order of their names. The
    /// entries `.` and `..` are never returned.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`], [`FsErrorKind::NotADirectory`]
    /// or [`FsErrorKind::PermissionDenied`]; errors of individual entries are
    /// yielded by the iterator.
    fn read_dir<'a>(
        &'a self,
        path: &AbsPath,
    ) -> FsResult<Box<dyn Iterator<Item = FsResult<DirEntry>> + Send + 'a>>;

    /// Target of a symbolic link or junction, as stored in the link.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`], and with
    /// [`FsErrorKind::InvalidPath`] when the entry is not a link.
    fn read_link(&self, path: &AbsPath) -> FsResult<PathBuf>;

    /// Real path: links resolved and every component in the spelling stored
    /// by the file system (used to report case aliases and followed links).
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::metadata`] with [`Follow::Follow`].
    fn real_path(&self, path: &AbsPath) -> FsResult<AbsPath>;

    /// SHA-256 of file content.
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::open_read`].
    fn content_hash(&self, path: &AbsPath) -> FsResult<ContentHash>;

    /// Volume that contains `path`. The path itself need not exist; the
    /// nearest existing ancestor decides.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] only when no ancestor exists, and
    /// with [`FsErrorKind::Io`].
    fn volume(&self, path: &AbsPath) -> FsResult<VolumeInfo>;

    /// Space figures of a volume.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] for an unknown volume, and with
    /// [`FsErrorKind::Io`].
    fn space(&self, volume: &VolumeId) -> FsResult<SpaceInfo>;

    /// Identity key of a path on its volume.
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::volume`].
    fn path_key(&self, path: &AbsPath) -> FsResult<PathKey>;

    /// Processes holding the entry open; empty when unknown.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`]; an unsupported query yields an
    /// empty list, not an error.
    fn holders(&self, path: &AbsPath) -> FsResult<Vec<Holder>>;

    /// Whether the account may create entries in `dir`.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] or [`FsErrorKind::NotADirectory`].
    fn can_write_dir(&self, dir: &AbsPath) -> FsResult<bool>;

    /// Entity tag of an entry, without content hash.
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::metadata`].
    fn etag(&self, path: &AbsPath, follow: Follow) -> FsResult<Etag> {
        self.metadata(path, follow).map(|meta| meta.etag())
    }

    /// Whether two paths are on the same volume.
    ///
    /// # Errors
    ///
    /// Fails as [`Fs::volume`].
    fn same_volume(&self, a: &AbsPath, b: &AbsPath) -> FsResult<bool> {
        Ok(self.volume(a)?.id == self.volume(b)?.id)
    }
}

/// Mutating file system interface.
///
/// Implemented by platform back ends and by test wrappers (fault injection,
/// crash simulation, mutation audit). Held only by journal and space
/// implementations; command implementations change data through
/// [`MutationTx`](crate::journal::MutationTx).
///
/// A method documented as atomic leaves, after a crash, either the previous
/// or the new state of the names it changes, never a mixture. Other methods
/// rely on the journal protocol for recovery.
pub trait FsWrite: Fs {
    /// Creates a new file with a unique name derived from `name_hint` in `dir`
    /// (`O_EXCL`, `CREATE_NEW`).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] for a missing directory, with
    /// [`FsErrorKind::PermissionDenied`] or with [`FsErrorKind::NoSpace`].
    fn create_temp_in(
        &self,
        scope: &TxScope,
        dir: &AbsPath,
        name_hint: &str,
    ) -> FsResult<Box<dyn WriteHandle>>;

    /// Creates a file at exactly `path`, failing when it exists (used for
    /// lock files whose names other programs expect, such as `index.lock`).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Exists`] when the file exists, and as
    /// [`FsWrite::create_temp_in`] otherwise.
    fn create_new(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>>;

    /// Opens an existing file for in-place writing (used when a file has more
    /// than one hard link, so that every name sees the new content).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`], [`FsErrorKind::PermissionDenied`]
    /// or [`FsErrorKind::InUse`].
    fn open_in_place(&self, scope: &TxScope, path: &AbsPath) -> FsResult<Box<dyn WriteHandle>>;

    /// Opens an alternate data stream for writing.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`] outside Windows.
    fn open_stream_write(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        stream: &str,
    ) -> FsResult<Box<dyn WriteHandle>>;

    /// Renames within one volume; atomic.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::CrossDevice`] across volumes or overlay
    /// layers, with [`FsErrorKind::Exists`] for [`RenameMode::NoReplace`] and
    /// with [`FsErrorKind::InUse`].
    fn rename(
        &self,
        scope: &TxScope,
        from: &AbsPath,
        to: &AbsPath,
        mode: RenameMode,
    ) -> FsResult<()>;

    /// Atomically exchanges two names (`renameat2(RENAME_EXCHANGE)`,
    /// `renamex_np(RENAME_SWAP)`).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`] when the volume cannot
    /// exchange names.
    fn exchange(&self, scope: &TxScope, a: &AbsPath, b: &AbsPath) -> FsResult<()>;

    /// Replaces `target` by `replacement`, keeping the old file under
    /// `backup` (`ReplaceFileW`); atomic for `target`.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`] outside Windows.
    fn replace_with_backup(
        &self,
        scope: &TxScope,
        target: &AbsPath,
        replacement: &AbsPath,
        backup: &AbsPath,
    ) -> FsResult<()>;

    /// Creates a hard link `new` to `existing`.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`], [`FsErrorKind::CrossDevice`]
    /// or [`FsErrorKind::Exists`].
    fn hard_link(&self, scope: &TxScope, existing: &AbsPath, new: &AbsPath) -> FsResult<()>;

    /// Clones `src` to the new file `dst`, sharing blocks.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`] when the volume cannot clone,
    /// and with [`FsErrorKind::Exists`] when `dst` exists.
    fn clone_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<()>;

    /// Copies bytes of `src` to the new file `dst`, including alternate data
    /// streams, and returns the number of copied bytes.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NoSpace`], and with [`FsErrorKind::Exists`]
    /// when `dst` exists.
    fn copy_file(&self, scope: &TxScope, src: &AbsPath, dst: &AbsPath) -> FsResult<u64>;

    /// Removes a non-directory entry (a link is removed, not followed).
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`], [`FsErrorKind::IsADirectory`] or
    /// [`FsErrorKind::InUse`].
    fn remove_file(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()>;

    /// Removes an empty directory.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotEmpty`] or [`FsErrorKind::NotADirectory`].
    fn remove_dir(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()>;

    /// Removes a tree bottom-up with up to `parallelism` workers, never
    /// descending into symbolic links, junctions or other reparse points.
    ///
    /// # Errors
    ///
    /// Fails with the first error encountered; entries removed before it stay
    /// removed.
    fn remove_tree(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        parallelism: usize,
    ) -> FsResult<RemoveStats>;

    /// Creates a directory with the given permission bits.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Exists`], and with [`FsErrorKind::NotFound`]
    /// for a missing parent.
    fn create_dir(&self, scope: &TxScope, path: &AbsPath, mode: u32) -> FsResult<()>;

    /// Creates a symbolic link or junction at `link` pointing to `target`.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Exists`], and with [`FsErrorKind::Privilege`]
    /// on Windows without the symbolic link privilege.
    fn create_link(
        &self,
        scope: &TxScope,
        target: &Path,
        link: &AbsPath,
        kind: LinkKind,
    ) -> FsResult<()>;

    /// Changes metadata of an entry.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::PermissionDenied`], [`FsErrorKind::Privilege`]
    /// or, for elements the volume lacks, [`FsErrorKind::Unsupported`].
    fn set_meta(&self, scope: &TxScope, path: &AbsPath, change: &MetaChange) -> FsResult<()>;

    /// Restores complete metadata of an entry and reports what could not be
    /// restored.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`]; elements refused individually are
    /// listed in the report instead.
    fn restore_meta(
        &self,
        scope: &TxScope,
        path: &AbsPath,
        meta: &ExtendedMeta,
    ) -> FsResult<MetaTransferReport>;

    /// Changes the length of a file.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] or [`FsErrorKind::NoSpace`].
    fn truncate(&self, scope: &TxScope, path: &AbsPath, length: u64) -> FsResult<()>;

    /// Flushes a directory so that renames and creations in it are durable.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Io`].
    fn sync_dir(&self, scope: &TxScope, dir: &AbsPath) -> FsResult<()>;

    /// Takes an exclusive advisory lock on `path`, creating the file when
    /// needed, and waits no longer than `timeout`.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::InUse`] when the lock is not obtained in
    /// time.
    fn lock_exclusive(&self, path: &AbsPath, timeout: Duration) -> FsResult<Box<dyn FileLock>>;

    /// Sets hidden, system and not-content-indexed attributes on Windows and
    /// equivalent markers elsewhere; used for the journal directory.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`].
    fn mark_hidden(&self, scope: &TxScope, path: &AbsPath) -> FsResult<()>;
}

/// Scheme of a provider, without the trailing colon (`registry`, `env`,
/// `archive`).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct ProviderScheme(pub String);

/// Key inside a provider: a path of segments and an optional value name (for
/// the registry: hive, keys and value name).
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub struct ProviderKey {
    /// Scheme of the provider.
    pub scheme: ProviderScheme,
    /// Key segments from the root.
    pub segments: Vec<String>,
    /// Value name inside the key; `None` addresses the key itself.
    pub value: Option<String>,
}

/// Typed value stored by a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderValue {
    /// Text value (`REG_SZ`, environment variable).
    Text(String),
    /// Text with unexpanded references (`REG_EXPAND_SZ`).
    ExpandableText(String),
    /// List of texts (`REG_MULTI_SZ`).
    TextList(Vec<String>),
    /// Unsigned 32-bit number (`REG_DWORD`).
    U32(u32),
    /// Unsigned 64-bit number (`REG_QWORD`).
    U64(u64),
    /// Raw bytes (`REG_BINARY` and other types) with the platform type code.
    Binary {
        /// Platform type code.
        raw_type: u32,
        /// Bytes.
        bytes: Vec<u8>,
    },
}

/// Preserved state of a key or value, sufficient for exact restoration
/// (including absence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderSnapshot {
    /// Key the snapshot was taken of.
    pub key: ProviderKey,
    /// Whether the key or value existed.
    pub existed: bool,
    /// Value, when a value was addressed and existed.
    pub value: Option<ProviderValue>,
    /// Complete subtree in a provider-defined serialized form, when a key was
    /// addressed (values, subkeys, security descriptor).
    pub subtree: Option<Vec<u8>>,
}

/// Capabilities of a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderCaps {
    /// Values can be changed.
    pub writable: bool,
    /// Keys form a hierarchy.
    pub hierarchical: bool,
    /// Changes persist beyond the current process.
    pub persistent: bool,
}

/// One child returned by [`Provider::list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderChild {
    /// Full key of the child.
    pub key: ProviderKey,
    /// True for a key (container), false for a value.
    pub is_key: bool,
}

/// Read interface of a provider of path-like resources.
///
/// Errors use [`FsError`] with the same kinds as files:
/// [`FsErrorKind::NotFound`] for a missing key and
/// [`FsErrorKind::PermissionDenied`] for a refused access.
pub trait Provider: Send + Sync {
    /// Scheme handled by this provider.
    fn scheme(&self) -> ProviderScheme;

    /// Capabilities.
    fn caps(&self) -> ProviderCaps;

    /// Reads a value; `None` when it does not exist.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::PermissionDenied`] or [`FsErrorKind::Io`].
    fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>>;

    /// Lists children of a key.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] for a missing key.
    fn list(&self, key: &ProviderKey) -> FsResult<Vec<ProviderChild>>;

    /// Takes a snapshot of a key or value for preservation.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::PermissionDenied`] or [`FsErrorKind::Io`]; a
    /// missing key yields a snapshot with `existed: false`, not an error.
    fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot>;
}

/// Write interface of a provider, used only by journal implementations.
pub trait ProviderWrite: Provider {
    /// Sets a value, creating missing keys.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::PermissionDenied`] or
    /// [`FsErrorKind::Privilege`].
    fn set(&self, scope: &TxScope, key: &ProviderKey, value: &ProviderValue) -> FsResult<()>;

    /// Removes a value, or a key with its subtree.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::NotFound`] or
    /// [`FsErrorKind::PermissionDenied`].
    fn remove(&self, scope: &TxScope, key: &ProviderKey) -> FsResult<()>;

    /// Restores the exact state captured by a snapshot, including absence.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::PermissionDenied`] or [`FsErrorKind::Io`].
    fn restore(&self, scope: &TxScope, snapshot: &ProviderSnapshot) -> FsResult<()>;

    /// Copies a key with its subtree, or a value, to `to`, creating the
    /// missing keys above it. A provider that cannot copy keeps the default,
    /// which fails.
    ///
    /// # Errors
    ///
    /// Fails with [`FsErrorKind::Unsupported`] by default; providers fail
    /// with [`FsErrorKind::NotFound`] for a missing source and
    /// [`FsErrorKind::Exists`] for an existing destination.
    fn duplicate(&self, scope: &TxScope, from: &ProviderKey, to: &ProviderKey) -> FsResult<()> {
        let _ = (scope, from, to);
        Err(FsError::new(FsErrorKind::Unsupported, "copy", None))
    }
}

/// Kind of a registry address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistryKind {
    /// A key, which holds subkeys and values.
    Key,
    /// A value of a key; the default value has the empty name.
    Value,
}

/// Key or value of the registry, as the provider key the journal declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryTarget {
    /// Address in the form of the provider: the root key and the keys in
    /// `segments`, the name of the value in `value`.
    pub key: ProviderKey,
    /// Whether the address names a key or a value.
    pub kind: RegistryKind,
}

/// Kind of a node found by [`RegistryAccess::walk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegistryNodeKind {
    /// A key.
    Key,
    /// A symbolic link key, which is reported but not entered.
    Link,
    /// A value.
    Value,
}

/// Key or value below a walked key.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryNode {
    /// The `entry` record of a key or the `registry_value` record of a value.
    pub record: crate::value::Record,
    /// Kind of the node.
    pub kind: RegistryNodeKind,
    /// Name of the key or value; empty for the default value.
    pub name: String,
    /// Path below the walked key, with `/` between the segments.
    pub relative: String,
}

/// Read access to the Windows registry for the file commands, which address
/// it with `registry:` paths.
///
/// The host registers an implementation in the extensions of the execution
/// context; where none is registered the registry does not exist and the
/// commands reject `registry:` paths with `E_OS_ONLY`. Changes are not made
/// here: the commands declare them to the journal as targets of a provider
/// key, and the journal writes them through [`ProviderWrite`].
///
/// # Errors
///
/// Every method fails with `E_INVALID_VALUE` for a path that does not parse
/// and with the code of the [`FsErrorKind`] of the failure otherwise.
pub trait RegistryAccess: Send + Sync {
    /// The `registry_value` record of the value a path names.
    ///
    /// # Errors
    ///
    /// Fails with `E_IS_A_DIRECTORY` when the path names a key and with
    /// `E_NOT_FOUND` when it names nothing.
    fn read_value(&self, path: &str) -> crate::error::Result<crate::value::Record>;

    /// Description of a key (an `entry` record) or of a value (a
    /// `registry_value` record); `None` when the path names nothing.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the registry other than `E_NOT_FOUND`.
    fn describe(&self, path: &str) -> crate::error::Result<Option<crate::value::Record>>;

    /// Keys and values below a key, depth first, down to `max_depth`
    /// levels, or without a limit for `None`; symbolic link keys are
    /// reported and not entered.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` for a missing key and `E_NOT_A_DIRECTORY`
    /// when the path names a value.
    fn walk(&self, path: &str, max_depth: Option<u32>) -> crate::error::Result<Vec<RegistryNode>>;

    /// The key or value a path names, which must exist.
    ///
    /// # Errors
    ///
    /// Fails with `E_NOT_FOUND` when the path names nothing.
    fn locate(&self, path: &str) -> crate::error::Result<RegistryTarget>;

    /// The address a path names read as `kind`, whether or not it exists.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` when the path cannot name that kind, for
    /// example a value at the root key.
    fn address(&self, path: &str, kind: RegistryKind) -> crate::error::Result<RegistryTarget>;
}
