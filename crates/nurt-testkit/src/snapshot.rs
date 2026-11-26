// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! File tree snapshots in the `nurt-fs-snapshot/1` format.
//!
//! A snapshot lists the entries below a base directory as JSON. It describes
//! the initial state of a test (`fs_before`) and the expected final state
//! (`fs_after`), and is the form in which a tree is compared before and after
//! a change.
//!
//! ```json
//! {
//!   "format": "nurt-fs-snapshot/1",
//!   "entries": [
//!     { "path": "src", "type": "dir" },
//!     { "path": "src/main.rs", "type": "file", "content_utf8": "fn main() {}\n", "mode": "0644" },
//!     { "path": "logo.bin", "type": "file", "content_base64": "AAEC" },
//!     { "path": "latest", "type": "symlink", "target": "src/main.rs" }
//!   ]
//! }
//! ```
//!
//! | Member | Meaning |
//! |---|---|
//! | `path` | path relative to the base, `/` as separator, UTF-8 |
//! | `type` | `file`, `dir` or `symlink` |
//! | `content_utf8`, `content_base64` | content of a file; at most one, absent for an empty file |
//! | `mode` | permission bits as four octal digits, optional |
//! | `mtime` | modification time as an RFC 3339 instant in UTC, optional |
//! | `target` | link target as stored in the link, for `symlink` only |
//!
//! Parent directories need not be listed. [`FsSnapshot::compare`] matches
//! entries by path and compares the optional members only where both sides
//! carry them and [`CompareOptions`] asks for them: permission bits are not
//! compared on Windows, and modification times are not compared unless
//! requested, because they depend on the clock of the run.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use jiff::Timestamp;
use nurt_contracts::env::Os;
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs, FsError, FsErrorKind};
use serde::{Deserialize, Serialize};

use crate::vfs::VirtualFs;

/// Format identifier of snapshots.
pub const SNAPSHOT_FORMAT: &str = "nurt-fs-snapshot/1";

/// Permission bits kept by a snapshot.
const MODE_MASK: u32 = 0o7777;

/// Kind of a snapshot entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum EntryType {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link.
    Symlink,
}

impl fmt::Display for EntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Symlink => "symlink",
        })
    }
}

/// One entry of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEntry {
    /// Path relative to the base, with `/` as separator.
    pub path: String,
    /// Kind.
    #[serde(rename = "type")]
    pub kind: EntryType,
    /// Content of a file that is valid UTF-8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_utf8: Option<String>,
    /// Content of any other file, in standard base64 with padding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    /// Permission bits as four octal digits (`0644`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Modification time as an RFC 3339 instant in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<String>,
    /// Link target as stored in the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl SnapshotEntry {
    /// Directory entry.
    #[must_use]
    pub fn dir(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: EntryType::Dir,
            content_utf8: None,
            content_base64: None,
            mode: None,
            mtime: None,
            target: None,
        }
    }

    /// File entry with `content`, stored as text when it is valid UTF-8.
    #[must_use]
    pub fn file(path: impl Into<String>, content: impl AsRef<[u8]>) -> Self {
        let content = content.as_ref();
        let (utf8, base64) = if content.is_empty() {
            (None, None)
        } else {
            match std::str::from_utf8(content) {
                Ok(text) => (Some(text.to_owned()), None),
                Err(_) => (None, Some(BASE64.encode(content))),
            }
        };
        Self {
            kind: EntryType::File,
            content_utf8: utf8,
            content_base64: base64,
            ..Self::dir(path)
        }
    }

    /// Symbolic link entry pointing to `target`.
    #[must_use]
    pub fn symlink(path: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            kind: EntryType::Symlink,
            target: Some(target.into()),
            ..Self::dir(path)
        }
    }

    /// The same entry with permission bits.
    #[must_use]
    pub fn with_mode(mut self, mode: u32) -> Self {
        self.mode = Some(format!("{:04o}", mode & MODE_MASK));
        self
    }

    /// The same entry with a modification time in nanoseconds since the
    /// Unix epoch; an instant outside the supported range is left out.
    #[must_use]
    pub fn with_mtime(mut self, modified_ns: i64) -> Self {
        self.mtime = Timestamp::from_nanosecond(i128::from(modified_ns))
            .ok()
            .map(|instant| instant.to_string());
        self
    }

    /// Content of a file entry.
    ///
    /// # Errors
    ///
    /// Fails with [`SnapshotError::InvalidEntry`] when both content members
    /// are present or the base64 text does not decode.
    pub fn content(&self) -> Result<Vec<u8>, SnapshotError> {
        match (&self.content_utf8, &self.content_base64) {
            (Some(_), Some(_)) => Err(self.invalid("has both content_utf8 and content_base64")),
            (Some(text), None) => Ok(text.as_bytes().to_vec()),
            (None, Some(encoded)) => BASE64
                .decode(encoded)
                .map_err(|error| self.invalid(&format!("content_base64 does not decode: {error}"))),
            (None, None) => Ok(Vec::new()),
        }
    }

    /// Permission bits, when present.
    ///
    /// # Errors
    ///
    /// Fails with [`SnapshotError::InvalidEntry`] when `mode` is not octal.
    pub fn mode_bits(&self) -> Result<Option<u32>, SnapshotError> {
        self.mode
            .as_deref()
            .map(|text| {
                u32::from_str_radix(text, 8)
                    .map_err(|_| self.invalid("mode is not an octal number"))
            })
            .transpose()
    }

    /// Modification time in nanoseconds since the Unix epoch, when present.
    ///
    /// # Errors
    ///
    /// Fails with [`SnapshotError::InvalidEntry`] when `mtime` is not an
    /// RFC 3339 instant in the range of 64-bit nanoseconds.
    pub fn mtime_ns(&self) -> Result<Option<i64>, SnapshotError> {
        self.mtime
            .as_deref()
            .map(|text| {
                let instant: Timestamp = text
                    .parse()
                    .map_err(|_| self.invalid("mtime is not an RFC 3339 instant"))?;
                i64::try_from(instant.as_nanosecond())
                    .map_err(|_| self.invalid("mtime is out of range"))
            })
            .transpose()
    }

    fn invalid(&self, message: &str) -> SnapshotError {
        SnapshotError::InvalidEntry {
            path: self.path.clone(),
            message: message.to_owned(),
        }
    }

    fn check(&self) -> Result<(), SnapshotError> {
        let segments_valid = !self.path.is_empty()
            && !self.path.starts_with('/')
            && self
                .path
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
        if !segments_valid {
            return Err(self.invalid("path must be relative, without empty, `.` or `..` segments"));
        }
        let has_content = self.content_utf8.is_some() || self.content_base64.is_some();
        match self.kind {
            EntryType::File if self.target.is_some() => Err(self.invalid("a file has no target")),
            EntryType::Dir if has_content || self.target.is_some() => {
                Err(self.invalid("a directory has no content and no target"))
            }
            EntryType::Symlink if has_content || self.target.is_none() => {
                Err(self.invalid("a symlink has a target and no content"))
            }
            _ => {
                self.content()?;
                self.mode_bits()?;
                self.mtime_ns()?;
                Ok(())
            }
        }
    }
}

/// Failure to read, write, capture or apply a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The text is not a JSON snapshot.
    Json {
        /// Parser message.
        message: String,
    },
    /// The `format` member names another format.
    Format {
        /// The format found.
        found: String,
    },
    /// An entry is malformed.
    InvalidEntry {
        /// Path of the entry.
        path: String,
        /// Description of the problem.
        message: String,
    },
    /// A file system call failed.
    Fs {
        /// Path of the call.
        path: PathBuf,
        /// Error kind.
        kind: FsErrorKind,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json { message } => write!(f, "invalid snapshot JSON: {message}"),
            Self::Format { found } => {
                write!(
                    f,
                    "unsupported snapshot format `{found}`; expected `{SNAPSHOT_FORMAT}`"
                )
            }
            Self::InvalidEntry { path, message } => {
                write!(f, "invalid snapshot entry `{path}`: {message}")
            }
            Self::Fs { path, kind } => {
                write!(
                    f,
                    "file system error {} at {}",
                    kind.as_str(),
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<FsError> for SnapshotError {
    fn from(error: FsError) -> Self {
        Self::Fs {
            path: error.path.unwrap_or_default(),
            kind: error.kind,
        }
    }
}

/// Optional members recorded by [`FsSnapshot::capture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureOptions {
    /// Record permission bits.
    pub mode: bool,
    /// Record modification times.
    pub mtime: bool,
}

/// Rules of [`FsSnapshot::compare`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompareOptions {
    /// Compare permission bits where both sides carry them.
    pub mode: bool,
    /// Compare modification times where both sides carry them.
    pub mtime: bool,
    /// Match paths without regard to letter case.
    pub case_insensitive: bool,
}

impl CompareOptions {
    /// Rules for results produced on `os`: permission bits are compared
    /// except on Windows, paths are matched without regard to case on
    /// Windows and macOS, and modification times are not compared.
    #[must_use]
    pub fn for_os(os: Os) -> Self {
        match os {
            Os::Windows => Self {
                mode: false,
                mtime: false,
                case_insensitive: true,
            },
            Os::Macos => Self {
                mode: true,
                mtime: false,
                case_insensitive: true,
            },
            _ => Self::default(),
        }
    }
}

impl Default for CompareOptions {
    /// Permission bits compared, modification times not compared, paths
    /// matched exactly.
    fn default() -> Self {
        Self {
            mode: true,
            mtime: false,
            case_insensitive: false,
        }
    }
}

/// Difference found by [`FsSnapshot::compare`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotDifference {
    /// An expected entry is absent.
    Missing {
        /// Path.
        path: String,
    },
    /// An entry is present that was not expected.
    Unexpected {
        /// Path.
        path: String,
    },
    /// The entry has another kind.
    Kind {
        /// Path.
        path: String,
        /// Expected kind.
        expected: EntryType,
        /// Actual kind.
        actual: EntryType,
    },
    /// The content of a file differs.
    Content {
        /// Path.
        path: String,
        /// Expected size in bytes.
        expected_size: usize,
        /// Actual size in bytes.
        actual_size: usize,
    },
    /// An optional member differs (`mode`, `mtime` or `target`).
    Member {
        /// Path.
        path: String,
        /// Member name.
        member: &'static str,
        /// Expected value.
        expected: String,
        /// Actual value.
        actual: String,
    },
}

impl fmt::Display for SnapshotDifference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => write!(f, "{path}: missing"),
            Self::Unexpected { path } => write!(f, "{path}: unexpected entry"),
            Self::Kind {
                path,
                expected,
                actual,
            } => write!(f, "{path}: expected {expected}, actual {actual}"),
            Self::Content {
                path,
                expected_size,
                actual_size,
            } => write!(
                f,
                "{path}: content differs ({expected_size} byte(s) expected, {actual_size} actual)"
            ),
            Self::Member {
                path,
                member,
                expected,
                actual,
            } => write!(f, "{path}: {member} expected {expected}, actual {actual}"),
        }
    }
}

/// Snapshot of a file tree.
///
/// ```
/// use nurt_testkit::snapshot::{CaptureOptions, CompareOptions, FsSnapshot, SnapshotEntry};
/// use nurt_testkit::vfs::{VirtualFs, test_path};
///
/// let before = FsSnapshot::from_entries(vec![
///     SnapshotEntry::file("notes/a.txt", "one\n"),
///     SnapshotEntry::symlink("latest", "notes/a.txt"),
/// ]);
/// let fs = VirtualFs::default();
/// let base = test_path("/work").unwrap();
/// before.apply(&fs, &base).unwrap();
/// fs.write_file(&test_path("/work/notes/b.txt").unwrap(), b"two\n").unwrap();
///
/// let after = FsSnapshot::capture(&fs, &base, CaptureOptions::default()).unwrap();
/// let differences = before.compare(&after, CompareOptions::default());
/// assert_eq!(differences.len(), 1);
/// assert_eq!(differences[0].to_string(), "notes/b.txt: unexpected entry");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FsSnapshot {
    /// Format identifier, [`SNAPSHOT_FORMAT`].
    pub format: String,
    /// Entries.
    pub entries: Vec<SnapshotEntry>,
}

impl Default for FsSnapshot {
    fn default() -> Self {
        Self::from_entries(Vec::new())
    }
}

fn join(base: &AbsPath, relative: &str) -> Result<AbsPath, SnapshotError> {
    let mut path = base.as_path().to_path_buf();
    for segment in relative.split('/') {
        path.push(segment);
    }
    Ok(AbsPath::new(path)?)
}

impl FsSnapshot {
    /// Snapshot of the given entries.
    #[must_use]
    pub fn from_entries(entries: Vec<SnapshotEntry>) -> Self {
        Self {
            format: SNAPSHOT_FORMAT.to_owned(),
            entries,
        }
    }

    /// Parses and checks a snapshot.
    ///
    /// # Errors
    ///
    /// Fails with [`SnapshotError::Json`] for text that is not a snapshot,
    /// [`SnapshotError::Format`] for another format and
    /// [`SnapshotError::InvalidEntry`] for a malformed or duplicate entry.
    pub fn from_json(text: &str) -> Result<Self, SnapshotError> {
        let snapshot: Self = serde_json::from_str(text).map_err(|error| SnapshotError::Json {
            message: error.to_string(),
        })?;
        if snapshot.format != SNAPSHOT_FORMAT {
            return Err(SnapshotError::Format {
                found: snapshot.format,
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &snapshot.entries {
            entry.check()?;
            if !seen.insert(entry.path.as_str()) {
                return Err(entry.invalid("the path is listed twice"));
            }
        }
        Ok(snapshot)
    }

    /// Pretty-printed JSON with a final line break, entries in the stored
    /// order.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The same snapshot with entries sorted by path.
    #[must_use]
    pub fn sorted(mut self) -> Self {
        self.entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        self
    }

    /// Snapshot of the tree below `base`, entries sorted by path. Links are
    /// recorded, not followed.
    ///
    /// # Errors
    ///
    /// Fails with [`SnapshotError::Fs`] when an entry cannot be read, and
    /// with [`SnapshotError::InvalidEntry`] for a name that is not UTF-8 and
    /// for an entry of a kind the format has no type for (special files,
    /// junctions).
    pub fn capture(
        fs: &dyn Fs,
        base: &AbsPath,
        options: CaptureOptions,
    ) -> Result<Self, SnapshotError> {
        let mut entries = Vec::new();
        let mut pending = vec![(base.clone(), String::new())];
        while let Some((dir, prefix)) = pending.pop() {
            for item in fs.read_dir(&dir)? {
                let item = item?;
                let name = item
                    .name
                    .to_str()
                    .ok_or_else(|| SnapshotError::InvalidEntry {
                        path: format!("{prefix}{}", item.name.to_string_lossy()),
                        message: "the name is not valid UTF-8".to_owned(),
                    })?;
                let relative = format!("{prefix}{name}");
                let path = AbsPath::new(dir.as_path().join(&item.name))?;
                let meta = fs.metadata(&path, Follow::NoFollow)?;
                let mut entry = match meta.kind {
                    FileKind::File => {
                        let mut handle = fs.open_read(&path)?;
                        let mut content = Vec::new();
                        std::io::Read::read_to_end(&mut handle, &mut content).map_err(|_| {
                            SnapshotError::Fs {
                                path: path.as_path().to_path_buf(),
                                kind: FsErrorKind::Io,
                            }
                        })?;
                        SnapshotEntry::file(relative.clone(), content)
                    }
                    FileKind::Directory => {
                        pending.push((path.clone(), format!("{relative}/")));
                        SnapshotEntry::dir(relative.clone())
                    }
                    FileKind::Symlink => {
                        let target = fs.read_link(&path)?;
                        SnapshotEntry::symlink(
                            relative.clone(),
                            target.to_string_lossy().replace('\\', "/"),
                        )
                    }
                    _ => {
                        return Err(SnapshotError::InvalidEntry {
                            path: relative,
                            message: "the entry kind has no snapshot type".to_owned(),
                        });
                    }
                };
                if options.mode && entry.kind != EntryType::Symlink {
                    entry = entry.with_mode(meta.mode);
                }
                if options.mtime && entry.kind == EntryType::File {
                    entry = entry.with_mtime(meta.modified_ns);
                }
                entries.push(entry);
            }
        }
        Ok(Self::from_entries(entries).sorted())
    }

    /// Creates the entries below `base` in a virtual file system, parents
    /// first, with the recorded permission bits and modification times.
    ///
    /// # Errors
    ///
    /// Fails with [`SnapshotError::InvalidEntry`] for a malformed entry and
    /// with [`SnapshotError::Fs`] when an entry cannot be created, for
    /// example because it exists.
    pub fn apply(&self, fs: &VirtualFs, base: &AbsPath) -> Result<(), SnapshotError> {
        fs.create_dir_all(base)?;
        let mut ordered: Vec<&SnapshotEntry> = self.entries.iter().collect();
        ordered.sort_by_key(|entry| entry.path.matches('/').count());
        for entry in ordered {
            entry.check()?;
            let path = join(base, &entry.path)?;
            match entry.kind {
                EntryType::Dir => fs.create_dir_all(&path)?,
                EntryType::File => fs.write_file(&path, &entry.content()?)?,
                EntryType::Symlink => {
                    let target = entry.target.clone().unwrap_or_default();
                    fs.symlink(PathBuf::from(target), &path)?;
                }
            }
            if let Some(mode) = entry.mode_bits()? {
                fs.set_mode(&path, mode)?;
            }
            if let Some(modified) = entry.mtime_ns()? {
                fs.set_modified(&path, modified)?;
            }
        }
        Ok(())
    }

    /// Differences between this snapshot as expected and `actual`, sorted by
    /// path.
    #[must_use]
    pub fn compare(&self, actual: &Self, options: CompareOptions) -> Vec<SnapshotDifference> {
        let key = |path: &str| {
            if options.case_insensitive {
                path.to_lowercase()
            } else {
                path.to_owned()
            }
        };
        let expected: BTreeMap<String, &SnapshotEntry> = self
            .entries
            .iter()
            .map(|entry| (key(&entry.path), entry))
            .collect();
        let found: BTreeMap<String, &SnapshotEntry> = actual
            .entries
            .iter()
            .map(|entry| (key(&entry.path), entry))
            .collect();
        let mut out = Vec::new();
        for (name, entry) in &expected {
            match found.get(name) {
                Some(other) => compare_entry(entry, other, options, &mut out),
                None if implied_dir(entry, &found, name) => {}
                None => out.push(SnapshotDifference::Missing {
                    path: entry.path.clone(),
                }),
            }
        }
        for (name, entry) in &found {
            if !expected.contains_key(name) && !implied_dir(entry, &expected, name) {
                out.push(SnapshotDifference::Unexpected {
                    path: entry.path.clone(),
                });
            }
        }
        out.sort_by(|left, right| difference_path(left).cmp(difference_path(right)));
        out
    }
}

/// A directory that is not listed on the other side is implied there when
/// the other side lists an entry below it.
fn implied_dir(entry: &SnapshotEntry, other: &BTreeMap<String, &SnapshotEntry>, key: &str) -> bool {
    entry.kind == EntryType::Dir
        && other
            .keys()
            .any(|candidate| candidate.starts_with(&format!("{key}/")))
}

fn difference_path(difference: &SnapshotDifference) -> &str {
    match difference {
        SnapshotDifference::Missing { path }
        | SnapshotDifference::Unexpected { path }
        | SnapshotDifference::Kind { path, .. }
        | SnapshotDifference::Content { path, .. }
        | SnapshotDifference::Member { path, .. } => path,
    }
}

fn compare_member(
    path: &str,
    member: &'static str,
    expected: Option<&String>,
    actual: Option<&String>,
    out: &mut Vec<SnapshotDifference>,
) {
    if let (Some(expected), Some(actual)) = (expected, actual)
        && expected != actual
    {
        out.push(SnapshotDifference::Member {
            path: path.to_owned(),
            member,
            expected: expected.clone(),
            actual: actual.clone(),
        });
    }
}

fn compare_entry(
    expected: &SnapshotEntry,
    actual: &SnapshotEntry,
    options: CompareOptions,
    out: &mut Vec<SnapshotDifference>,
) {
    let path = expected.path.as_str();
    if expected.kind != actual.kind {
        out.push(SnapshotDifference::Kind {
            path: path.to_owned(),
            expected: expected.kind,
            actual: actual.kind,
        });
        return;
    }
    if expected.kind == EntryType::File {
        let left = expected.content().unwrap_or_default();
        let right = actual.content().unwrap_or_default();
        if left != right {
            out.push(SnapshotDifference::Content {
                path: path.to_owned(),
                expected_size: left.len(),
                actual_size: right.len(),
            });
        }
    }
    compare_member(
        path,
        "target",
        expected.target.as_ref(),
        actual.target.as_ref(),
        out,
    );
    if options.mode {
        // Spellings of one number (`755` and `0755`) are not a difference.
        match (expected.mode_bits(), actual.mode_bits()) {
            (Ok(Some(left)), Ok(Some(right))) if left == right => {}
            _ => compare_member(
                path,
                "mode",
                expected.mode.as_ref(),
                actual.mode.as_ref(),
                out,
            ),
        }
    }
    if options.mtime {
        // One instant has several spellings (`Z`, `+00:00`, fractions).
        match (expected.mtime_ns(), actual.mtime_ns()) {
            (Ok(Some(left)), Ok(Some(right))) if left == right => {}
            _ => compare_member(
                path,
                "mtime",
                expected.mtime.as_ref(),
                actual.mtime.as_ref(),
                out,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::env::Os;

    use super::{
        CaptureOptions, CompareOptions, EntryType, FsSnapshot, SNAPSHOT_FORMAT, SnapshotDifference,
        SnapshotEntry, SnapshotError,
    };
    use crate::vfs::{VirtualFs, test_path};

    #[test]
    fn json_round_trip_keeps_every_member() {
        let snapshot = FsSnapshot::from_entries(vec![
            SnapshotEntry::dir("d").with_mode(0o755),
            SnapshotEntry::file("d/bin", [0_u8, 255, 1]).with_mtime(1_000_000_000),
            SnapshotEntry::file("d/empty", b""),
            SnapshotEntry::symlink("l", "d/bin"),
        ]);
        let text = snapshot.to_json();
        assert!(text.contains("\"content_base64\": \"AP8B\""));
        assert!(text.contains("\"mtime\": \"1970-01-01T00:00:01Z\""));
        assert!(text.contains("\"mode\": \"0755\""));
        assert_eq!(FsSnapshot::from_json(&text).unwrap(), snapshot);
    }

    #[test]
    fn malformed_snapshots_are_rejected() {
        let other = "{\"format\": \"other/1\", \"entries\": []}";
        assert!(matches!(
            FsSnapshot::from_json(other),
            Err(SnapshotError::Format { .. })
        ));
        assert!(matches!(
            FsSnapshot::from_json("[]"),
            Err(SnapshotError::Json { .. })
        ));
        for entry in [
            r#"{"path": "../x", "type": "file"}"#,
            r#"{"path": "x", "type": "dir", "content_utf8": "a"}"#,
            r#"{"path": "x", "type": "symlink"}"#,
            r#"{"path": "x", "type": "file", "content_utf8": "a", "content_base64": "YQ=="}"#,
            r#"{"path": "x", "type": "file", "mode": "rw"}"#,
            r#"{"path": "x", "type": "file", "mtime": "noon"}"#,
            r#"{"path": "x", "type": "file", "target": "y"}"#,
        ] {
            let text = format!("{{\"format\": \"{SNAPSHOT_FORMAT}\", \"entries\": [{entry}]}}");
            assert!(
                matches!(
                    FsSnapshot::from_json(&text),
                    Err(SnapshotError::InvalidEntry { .. })
                ),
                "{entry}"
            );
        }
        let twice = format!(
            "{{\"format\": \"{SNAPSHOT_FORMAT}\", \"entries\": [{{\"path\": \"a\", \"type\": \"dir\"}}, {{\"path\": \"a\", \"type\": \"dir\"}}]}}"
        );
        assert!(FsSnapshot::from_json(&twice).is_err());
    }

    #[test]
    fn apply_and_capture_restore_the_same_tree() {
        let fs = VirtualFs::default();
        let base = test_path("/case").unwrap();
        let snapshot = FsSnapshot::from_entries(vec![
            SnapshotEntry::file("a/b/c.txt", "text")
                .with_mode(0o600)
                .with_mtime(5),
            SnapshotEntry::dir("a").with_mode(0o700),
            SnapshotEntry::dir("a/b").with_mode(0o755),
            SnapshotEntry::symlink("link", "a/b/c.txt"),
        ]);
        snapshot.apply(&fs, &base).unwrap();
        let captured = FsSnapshot::capture(
            &fs,
            &base,
            CaptureOptions {
                mode: true,
                mtime: true,
            },
        )
        .unwrap();
        assert_eq!(captured, snapshot.clone().sorted());
        let options = CompareOptions {
            mtime: true,
            ..CompareOptions::default()
        };
        assert!(snapshot.compare(&captured, options).is_empty());
    }

    #[test]
    fn compare_reports_each_kind_of_difference() {
        let expected = FsSnapshot::from_entries(vec![
            SnapshotEntry::file("gone", "x"),
            SnapshotEntry::file("kind", "x"),
            SnapshotEntry::file("text", "old").with_mode(0o644),
            SnapshotEntry::symlink("link", "a"),
        ]);
        let actual = FsSnapshot::from_entries(vec![
            SnapshotEntry::dir("kind"),
            SnapshotEntry::file("text", "new!").with_mode(0o600),
            SnapshotEntry::symlink("link", "b"),
            SnapshotEntry::file("extra", ""),
        ]);
        let lines: Vec<String> = expected
            .compare(&actual, CompareOptions::default())
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            lines,
            [
                "extra: unexpected entry",
                "gone: missing",
                "kind: expected file, actual dir",
                "link: target expected a, actual b",
                "text: content differs (3 byte(s) expected, 4 actual)",
                "text: mode expected 0644, actual 0600",
            ]
        );
        let windows = expected.compare(&actual, CompareOptions::for_os(Os::Windows));
        assert!(!windows.iter().any(|difference| matches!(
            difference,
            SnapshotDifference::Member { member: "mode", .. }
        )));
    }

    #[test]
    fn compare_implies_unlisted_parents_and_folds_case_when_asked() {
        let expected = FsSnapshot::from_entries(vec![SnapshotEntry::file("Dir/File.txt", "a")]);
        let actual = FsSnapshot::from_entries(vec![
            SnapshotEntry::dir("dir"),
            SnapshotEntry::file("dir/file.txt", "a"),
        ]);
        assert!(
            expected
                .compare(&actual, CompareOptions::for_os(Os::Macos))
                .is_empty()
        );
        assert_eq!(
            expected.compare(&actual, CompareOptions::default()).len(),
            3
        );
    }

    #[test]
    fn compare_treats_spellings_of_one_mode_and_one_instant_as_equal() {
        let spelled = |mode: &str, mtime: &str| {
            let mut entry = SnapshotEntry::file("f", "x");
            entry.mode = Some(mode.to_owned());
            entry.mtime = Some(mtime.to_owned());
            FsSnapshot::from_entries(vec![entry])
        };
        let options = CompareOptions {
            mtime: true,
            ..CompareOptions::default()
        };
        let expected = spelled("755", "2026-01-01T00:00:00+00:00");
        let same = spelled("0755", "2026-01-01T00:00:00.000Z");
        assert!(expected.compare(&same, options).is_empty());
        let other = spelled("0700", "2026-01-01T00:00:01Z");
        let members: Vec<&str> = expected
            .compare(&other, options)
            .iter()
            .filter_map(|difference| match difference {
                SnapshotDifference::Member { member, .. } => Some(*member),
                _ => None,
            })
            .collect();
        assert_eq!(members, ["mode", "mtime"]);
    }

    #[test]
    fn capture_rejects_entries_without_a_type() {
        let fs = VirtualFs::default();
        let base = test_path("/dev").unwrap();
        fs.create_special(&test_path("/dev/null").unwrap()).unwrap();
        let error = FsSnapshot::capture(&fs, &base, CaptureOptions::default()).unwrap_err();
        assert!(matches!(error, SnapshotError::InvalidEntry { .. }));
        assert_eq!(EntryType::Symlink.to_string(), "symlink");
    }
}
