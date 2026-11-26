// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! File system snapshot `nurt-fs-snapshot/1`.
//!
//! A snapshot describes a directory tree: the initial and the expected states
//! of conformance cases, benchmark tasks and examples of the documentation.
//! Paths are relative to the root of the tree and use `/` as separator.
//!
//! # Examples
//!
//! ```
//! use nurt_conformance::snapshot::Snapshot;
//!
//! let snapshot = Snapshot::from_json(&serde_json::json!({
//!     "format": "nurt-fs-snapshot/1",
//!     "entries": [{"path": "src/main.go", "type": "file", "content_utf8": "package main\n"}]
//! }))?;
//! let root = tempfile::tempdir()?;
//! snapshot.materialize(root.path())?;
//! assert!(snapshot.diff(root.path())?.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Name and version of the format.
pub const FORMAT: &str = "nurt-fs-snapshot/1";

/// Separator of path segments in snapshots.
const SEPARATOR: char = '/';

/// Number of octal digits of a mode.
const MODE_DIGITS: usize = 4;

/// Permission bits that a mode may hold.
const MODE_MASK: u32 = 0o7777;

/// Snapshot of a directory tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// Format name; may be omitted when the snapshot is embedded in a case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Whether the snapshot describes the whole tree.
    #[serde(default = "exhaustive_default", skip_serializing_if = "is_exhaustive")]
    pub exhaustive: bool,
    /// Entries of the tree.
    pub entries: Vec<Entry>,
}

fn exhaustive_default() -> bool {
    true
}

fn is_exhaustive(value: &bool) -> bool {
    *value
}

/// Type of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    /// Regular file.
    File,
    /// Directory.
    Dir,
    /// Symbolic link.
    Symlink,
}

impl EntryType {
    fn name(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Symlink => "symlink",
        }
    }
}

/// Entry of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Path relative to the root.
    pub path: String,
    /// Type of the entry.
    #[serde(rename = "type")]
    pub kind: EntryType,
    /// Content of a file as UTF-8 text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_utf8: Option<String>,
    /// Content of a file in standard Base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    /// Permission bits in octal, for example `"0644"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Modification time of a file, RFC 3339 in UTC with whole seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<String>,
    /// Target of a symbolic link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl Entry {
    /// Returns a file entry with the given content, stored as UTF-8 text when
    /// it is valid UTF-8 and as Base64 otherwise.
    #[must_use]
    pub fn file(path: impl Into<String>, content: &[u8]) -> Self {
        let (content_utf8, content_base64) = match std::str::from_utf8(content) {
            Ok(text) => (Some(text.to_owned()), None),
            Err(_) => (None, Some(BASE64.encode(content))),
        };
        Self {
            path: path.into(),
            kind: EntryType::File,
            content_utf8,
            content_base64,
            mode: None,
            mtime: None,
            target: None,
        }
    }

    /// Returns a directory entry.
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

    /// Returns a symbolic link entry.
    #[must_use]
    pub fn symlink(path: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            target: Some(target.into()),
            kind: EntryType::Symlink,
            ..Self::dir(path)
        }
    }

    /// Returns the content of a file entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the Base64 content does not decode.
    pub fn content(&self) -> Result<Vec<u8>, SnapshotError> {
        match (&self.content_utf8, &self.content_base64) {
            (Some(text), _) => Ok(text.as_bytes().to_vec()),
            (None, Some(encoded)) => BASE64
                .decode(encoded)
                .map_err(|error| self.invalid(format!("content_base64 does not decode: {error}"))),
            (None, None) => Ok(Vec::new()),
        }
    }

    fn mode_bits(&self) -> Result<Option<u32>, SnapshotError> {
        let Some(mode) = &self.mode else {
            return Ok(None);
        };
        let valid = mode.len() == MODE_DIGITS && mode.bytes().all(|b| (b'0'..=b'7').contains(&b));
        match u32::from_str_radix(mode, 8) {
            Ok(bits) if valid && bits <= MODE_MASK => Ok(Some(bits)),
            _ => Err(self.invalid(format!(
                "mode {mode:?} is not an octal mode such as \"0644\""
            ))),
        }
    }

    fn mtime_value(&self) -> Result<Option<jiff::Timestamp>, SnapshotError> {
        let Some(mtime) = &self.mtime else {
            return Ok(None);
        };
        let timestamp: jiff::Timestamp = mtime
            .parse()
            .map_err(|_| self.invalid(format!("mtime {mtime:?} is not an RFC 3339 time")))?;
        if !mtime.ends_with('Z') || timestamp.subsec_nanosecond() != 0 {
            return Err(self.invalid(format!(
                "mtime {mtime:?} must be in UTC with whole seconds, for example \"2026-01-31T12:00:00Z\""
            )));
        }
        Ok(Some(timestamp))
    }

    fn invalid(&self, message: String) -> SnapshotError {
        SnapshotError::Invalid {
            path: self.path.clone(),
            message,
        }
    }

    fn validate(&self) -> Result<(), SnapshotError> {
        validate_path(&self.path)?;
        let content = self.content_utf8.is_some() as u8 + self.content_base64.is_some() as u8;
        match self.kind {
            EntryType::File if content != 1 => Err(self
                .invalid("a file needs exactly one of content_utf8 and content_base64".to_owned())),
            EntryType::Dir | EntryType::Symlink if content != 0 => {
                Err(self.invalid(format!("a {} has no content", self.kind.name())))
            }
            EntryType::Symlink if self.target.as_deref().is_none_or(str::is_empty) => {
                Err(self.invalid("a symbolic link needs a target".to_owned()))
            }
            EntryType::File | EntryType::Dir if self.target.is_some() => {
                Err(self.invalid("only a symbolic link has a target".to_owned()))
            }
            _ => {
                self.content()?;
                self.mode_bits()?;
                self.mtime_value()?;
                Ok(())
            }
        }
    }
}

fn validate_path(path: &str) -> Result<(), SnapshotError> {
    let invalid = path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split(SEPARATOR)
            .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if invalid {
        return Err(SnapshotError::Invalid {
            path: path.to_owned(),
            message: "the path must be relative, use '/' and contain no empty, '.' or '..' segment"
                .to_owned(),
        });
    }
    Ok(())
}

/// Error of reading, writing or comparing a snapshot.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    /// The snapshot document does not follow the format.
    #[error("snapshot entry {path:?}: {message}")]
    Invalid {
        /// Path of the entry, or empty for the document.
        path: String,
        /// Description of the problem.
        message: String,
    },
    /// A file of the tree could not be read or written.
    #[error("{}: {source}", path.display())]
    Io {
        /// Path of the file.
        path: PathBuf,
        /// Error of the operating system.
        #[source]
        source: std::io::Error,
    },
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> SnapshotError + '_ {
    move |source| SnapshotError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Difference between an expected snapshot and a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// Path of the entry.
    pub path: String,
    /// Kind of the difference.
    pub kind: DifferenceKind,
    /// Description of the difference.
    pub detail: String,
}

/// Kind of a difference between an expected snapshot and a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferenceKind {
    /// The entry does not exist.
    Missing,
    /// The entry exists but differs.
    Changed,
    /// The entry exists but is not part of the expected snapshot.
    Extra,
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            DifferenceKind::Missing => "missing",
            DifferenceKind::Changed => "changed",
            DifferenceKind::Extra => "extra",
        };
        write!(f, "{kind} {}: {}", self.path, self.detail)
    }
}

impl Snapshot {
    /// Reads a snapshot from its JSON form and checks it.
    ///
    /// # Errors
    ///
    /// Returns an error when the document does not follow the format.
    pub fn from_json(value: &Value) -> Result<Self, SnapshotError> {
        let snapshot: Self =
            serde_json::from_value(value.clone()).map_err(|error| SnapshotError::Invalid {
                path: String::new(),
                message: error.to_string(),
            })?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Reads a snapshot from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or does not follow the
    /// format.
    pub fn load(path: &Path) -> Result<Self, SnapshotError> {
        let text = fs::read_to_string(path).map_err(io_error(path))?;
        let value: Value = serde_json::from_str(&text).map_err(|error| SnapshotError::Invalid {
            path: String::new(),
            message: error.to_string(),
        })?;
        Self::from_json(&value)
    }

    /// Checks the format name and every entry.
    ///
    /// # Errors
    ///
    /// Returns the first problem found.
    pub fn validate(&self) -> Result<(), SnapshotError> {
        if let Some(format) = &self.format
            && format != FORMAT
        {
            return Err(SnapshotError::Invalid {
                path: String::new(),
                message: format!("format must be {FORMAT:?}, found {format:?}"),
            });
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if !seen.insert(entry.path.as_str()) {
                return Err(entry.invalid("the path is listed twice".to_owned()));
            }
        }
        Ok(())
    }

    /// Returns the JSON form of the snapshot.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Creates the entries of the snapshot below `root`.
    ///
    /// Parent directories are created as needed. Modes are applied on Unix
    /// only; modification times are applied to files.
    ///
    /// # Errors
    ///
    /// Returns an error when an entry cannot be created.
    pub fn materialize(&self, root: &Path) -> Result<(), SnapshotError> {
        fs::create_dir_all(root).map_err(io_error(root))?;
        let mut entries: Vec<&Entry> = self.entries.iter().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        for entry in &entries {
            create_entry(root, entry)?;
        }
        // A directory without write permission would refuse its children, so
        // the modes of directories are applied last, the deepest first.
        for entry in entries.iter().rev().filter(|e| e.kind == EntryType::Dir) {
            apply_mode(&native_path(root, &entry.path), entry.mode_bits()?)?;
        }
        Ok(())
    }

    /// Reads the tree below `root` into a snapshot.
    ///
    /// The snapshot lists files, symbolic links and empty directories,
    /// sorted by path. Modes are recorded on Unix when `modes` is true;
    /// modification times of files when `mtimes` is true.
    ///
    /// # Errors
    ///
    /// Returns an error when the tree cannot be read.
    pub fn capture(root: &Path, modes: bool, mtimes: bool) -> Result<Self, SnapshotError> {
        let tree = read_tree(root)?;
        let entries = tree
            .into_iter()
            .filter(|(_, actual)| actual.leaf)
            .map(|(path, actual)| actual.into_entry(path, modes, mtimes))
            .collect();
        Ok(Self {
            format: Some(FORMAT.to_owned()),
            exhaustive: true,
            entries,
        })
    }

    /// Compares the snapshot, as the expected state, with the tree below
    /// `root`.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot is not valid or the tree cannot be
    /// read.
    pub fn diff(&self, root: &Path) -> Result<Vec<Difference>, SnapshotError> {
        self.validate()?;
        let tree = read_tree(root)?;
        let mut differences = Vec::new();
        for entry in &self.entries {
            match tree.get(&entry.path) {
                None => differences.push(Difference {
                    path: entry.path.clone(),
                    kind: DifferenceKind::Missing,
                    detail: format!("expected a {}", entry.kind.name()),
                }),
                Some(actual) => {
                    if let Some(detail) = actual.differs_from(entry)? {
                        differences.push(Difference {
                            path: entry.path.clone(),
                            kind: DifferenceKind::Changed,
                            detail,
                        });
                    }
                }
            }
        }
        if self.exhaustive {
            differences.extend(self.extra_entries(&tree));
        }
        Ok(differences)
    }

    fn extra_entries(&self, tree: &BTreeMap<String, Actual>) -> Vec<Difference> {
        let listed: BTreeSet<&str> = self.entries.iter().map(|e| e.path.as_str()).collect();
        tree.iter()
            .filter(|(path, actual)| actual.leaf && !listed.contains(path.as_str()))
            .map(|(path, actual)| Difference {
                path: path.clone(),
                kind: DifferenceKind::Extra,
                detail: format!("unexpected {}", actual.kind.name()),
            })
            .collect()
    }
}

fn native_path(root: &Path, path: &str) -> PathBuf {
    path.split(SEPARATOR)
        .fold(root.to_path_buf(), |joined, segment| joined.join(segment))
}

fn create_entry(root: &Path, entry: &Entry) -> Result<(), SnapshotError> {
    let path = native_path(root, &entry.path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error(parent))?;
    }
    match entry.kind {
        EntryType::Dir => fs::create_dir_all(&path).map_err(io_error(&path))?,
        EntryType::File => {
            fs::write(&path, entry.content()?).map_err(io_error(&path))?;
            if let Some(mtime) = entry.mtime_value()? {
                let file = fs::File::options()
                    .write(true)
                    .open(&path)
                    .map_err(io_error(&path))?;
                file.set_modified(system_time(mtime))
                    .map_err(io_error(&path))?;
            }
        }
        EntryType::Symlink => create_symlink(&path, entry.target.as_deref().unwrap_or_default())?,
    }
    if entry.kind == EntryType::Dir {
        return Ok(());
    }
    apply_mode(&path, entry.mode_bits()?)
}

#[cfg(unix)]
fn create_symlink(path: &Path, target: &str) -> Result<(), SnapshotError> {
    std::os::unix::fs::symlink(target, path).map_err(io_error(path))
}

#[cfg(windows)]
fn create_symlink(path: &Path, target: &str) -> Result<(), SnapshotError> {
    let native = target.replace(SEPARATOR, "\\");
    let resolved = path.parent().map(|parent| parent.join(&native));
    if resolved.is_some_and(|resolved| resolved.is_dir()) {
        std::os::windows::fs::symlink_dir(&native, path).map_err(io_error(path))
    } else {
        std::os::windows::fs::symlink_file(&native, path).map_err(io_error(path))
    }
}

#[cfg(not(any(unix, windows)))]
fn create_symlink(path: &Path, _target: &str) -> Result<(), SnapshotError> {
    Err(SnapshotError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::from(std::io::ErrorKind::Unsupported),
    })
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode: Option<u32>) -> Result<(), SnapshotError> {
    use std::os::unix::fs::PermissionsExt as _;
    let Some(mode) = mode else {
        return Ok(());
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_error(path))
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: Option<u32>) -> Result<(), SnapshotError> {
    Ok(())
}

#[cfg(unix)]
fn read_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(metadata.permissions().mode() & MODE_MASK)
}

#[cfg(not(unix))]
fn read_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

/// Entry found in a directory tree.
struct Actual {
    kind: EntryType,
    content: Vec<u8>,
    target: Option<String>,
    mode: Option<u32>,
    mtime: Option<jiff::Timestamp>,
    leaf: bool,
}

impl Actual {
    fn into_entry(self, path: String, modes: bool, mtimes: bool) -> Entry {
        let mut entry = match self.kind {
            EntryType::File => Entry::file(path, &self.content),
            EntryType::Dir => Entry::dir(path),
            EntryType::Symlink => Entry::symlink(path, self.target.unwrap_or_default()),
        };
        if modes && self.kind != EntryType::Symlink {
            entry.mode = self.mode.map(|mode| format!("{mode:04o}"));
        }
        if mtimes && self.kind == EntryType::File {
            entry.mtime = self.mtime.map(|mtime| mtime.to_string());
        }
        entry
    }

    fn differs_from(&self, expected: &Entry) -> Result<Option<String>, SnapshotError> {
        if self.kind != expected.kind {
            return Ok(Some(format!(
                "expected a {}, found a {}",
                expected.kind.name(),
                self.kind.name()
            )));
        }
        if self.kind == EntryType::File && self.content != expected.content()? {
            return Ok(Some(content_detail(&expected.content()?, &self.content)));
        }
        if self.kind == EntryType::Symlink && self.target != expected.target {
            return Ok(Some(format!(
                "target {:?} instead of {:?}",
                self.target.as_deref().unwrap_or_default(),
                expected.target.as_deref().unwrap_or_default()
            )));
        }
        if let (Some(wanted), Some(found)) = (expected.mode_bits()?, self.mode)
            && wanted != found
        {
            return Ok(Some(format!("mode 0{found:o} instead of 0{wanted:o}")));
        }
        if let Some(wanted) = expected.mtime_value()?
            && self.kind == EntryType::File
            && self.mtime != Some(wanted)
        {
            let found = self.mtime.map(|m| m.to_string()).unwrap_or_default();
            return Ok(Some(format!("modified {found} instead of {wanted}")));
        }
        Ok(None)
    }
}

/// Converts a timestamp to a system time; times before 1970 are not used by
/// snapshots and map to the epoch.
fn system_time(timestamp: jiff::Timestamp) -> SystemTime {
    let seconds = u64::try_from(timestamp.as_second()).unwrap_or_default();
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// Converts a system time to a timestamp with whole seconds.
fn timestamp(time: SystemTime) -> Option<jiff::Timestamp> {
    let seconds = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    jiff::Timestamp::from_second(i64::try_from(seconds).ok()?).ok()
}

fn content_detail(expected: &[u8], actual: &[u8]) -> String {
    match (std::str::from_utf8(expected), std::str::from_utf8(actual)) {
        (Ok(expected), Ok(actual)) => format!("content {actual:?} instead of {expected:?}"),
        _ => format!(
            "binary content of {} bytes instead of {} bytes",
            actual.len(),
            expected.len()
        ),
    }
}

fn read_tree(root: &Path) -> Result<BTreeMap<String, Actual>, SnapshotError> {
    let mut tree = BTreeMap::new();
    read_dir_into(root, "", &mut tree)?;
    Ok(tree)
}

/// Text of a file name. A name that is not valid UTF-8 keeps every invalid
/// byte as U+FFFD and its two hexadecimal digits, so that two different names
/// never give the same text.
fn name_text(name: &std::ffi::OsStr) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let mut text = String::new();
        for chunk in name.as_bytes().utf8_chunks() {
            text.push_str(chunk.valid());
            for byte in chunk.invalid() {
                text.push_str(&format!("\u{FFFD}{byte:02x}"));
            }
        }
        text
    }
    #[cfg(not(unix))]
    {
        name.to_string_lossy().into_owned()
    }
}

fn read_dir_into(
    dir: &Path,
    prefix: &str,
    tree: &mut BTreeMap<String, Actual>,
) -> Result<bool, SnapshotError> {
    let mut empty = true;
    for item in fs::read_dir(dir).map_err(io_error(dir))? {
        empty = false;
        let item = item.map_err(io_error(dir))?;
        let name = name_text(&item.file_name());
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}{SEPARATOR}{name}")
        };
        let path = item.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error(&path))?;
        let actual = read_entry(&path, &relative, &metadata, tree)?;
        tree.insert(relative, actual);
    }
    Ok(empty)
}

fn read_entry(
    path: &Path,
    relative: &str,
    metadata: &fs::Metadata,
    tree: &mut BTreeMap<String, Actual>,
) -> Result<Actual, SnapshotError> {
    let mode = read_mode(metadata);
    let file_type = metadata.file_type();
    let mut actual = Actual {
        kind: EntryType::File,
        content: Vec::new(),
        target: None,
        mode,
        mtime: None,
        leaf: true,
    };
    if file_type.is_symlink() {
        let target = fs::read_link(path).map_err(io_error(path))?;
        actual.kind = EntryType::Symlink;
        actual.target = Some(target.to_string_lossy().replace('\\', "/"));
    } else if file_type.is_dir() {
        actual.kind = EntryType::Dir;
        actual.leaf = read_dir_into(path, relative, tree)?;
    } else {
        actual.content = fs::read(path).map_err(io_error(path))?;
        actual.mtime = metadata.modified().ok().and_then(timestamp);
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(entries: Value) -> Snapshot {
        Snapshot::from_json(&json!({"format": FORMAT, "entries": entries})).unwrap()
    }

    #[test]
    fn materialized_snapshot_has_no_differences() {
        let root = tempfile::tempdir().unwrap();
        let expected = snapshot(json!([
            {"path": "src/main.go", "type": "file", "content_utf8": "package main\n"},
            {"path": "bin/data", "type": "file", "content_base64": "AAEC/w=="},
            {"path": "empty", "type": "dir"},
            {"path": "old.log", "type": "file", "content_utf8": "", "mtime": "2026-01-31T12:00:00Z"}
        ]));
        expected.materialize(root.path()).unwrap();
        assert_eq!(expected.diff(root.path()).unwrap(), Vec::new());
        let captured = Snapshot::capture(root.path(), false, true).unwrap();
        let paths: Vec<&str> = captured.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["bin/data", "empty", "old.log", "src/main.go"]);
        assert_eq!(
            captured.entries[0].content_base64.as_deref(),
            Some("AAEC/w==")
        );
        assert_eq!(
            captured.entries[2].mtime.as_deref(),
            Some("2026-01-31T12:00:00Z")
        );
        assert_eq!(Snapshot::from_json(&captured.to_json()).unwrap(), captured);
    }

    #[cfg(unix)]
    #[test]
    fn names_that_are_not_utf8_stay_distinct() {
        use std::os::unix::ffi::OsStrExt as _;
        let root = tempfile::tempdir().unwrap();
        for name in [&b"a\xff"[..], &b"a\xfe"[..], "ż.txt".as_bytes()] {
            fs::write(root.path().join(std::ffi::OsStr::from_bytes(name)), "x").unwrap();
        }
        let captured = Snapshot::capture(root.path(), false, false).unwrap();
        let paths: Vec<&str> = captured.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a\u{FFFD}fe", "a\u{FFFD}ff", "ż.txt"]);
    }

    #[test]
    fn diff_reports_missing_changed_and_extra_entries() {
        let root = tempfile::tempdir().unwrap();
        snapshot(json!([
            {"path": "a.txt", "type": "file", "content_utf8": "old"},
            {"path": "dir", "type": "file", "content_utf8": ""},
            {"path": "extra/x.txt", "type": "file", "content_utf8": "x"}
        ]))
        .materialize(root.path())
        .unwrap();
        let expected = snapshot(json!([
            {"path": "a.txt", "type": "file", "content_utf8": "new"},
            {"path": "dir", "type": "dir"},
            {"path": "b.txt", "type": "file", "content_utf8": "b"}
        ]));
        let differences = expected.diff(root.path()).unwrap();
        let kinds: Vec<(String, DifferenceKind)> = differences
            .iter()
            .map(|d| (d.path.clone(), d.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("a.txt".to_owned(), DifferenceKind::Changed),
                ("dir".to_owned(), DifferenceKind::Changed),
                ("b.txt".to_owned(), DifferenceKind::Missing),
                ("extra/x.txt".to_owned(), DifferenceKind::Extra),
            ]
        );
        assert!(differences[0].to_string().starts_with("changed a.txt"));
        assert!(differences[2].to_string().starts_with("missing b.txt"));
        assert!(differences[3].to_string().starts_with("extra extra/x.txt"));
    }

    #[test]
    fn non_exhaustive_snapshot_ignores_unlisted_entries() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("other.txt"), "x").unwrap();
        fs::write(root.path().join("a.txt"), "a").unwrap();
        let expected = Snapshot::from_json(&json!({
            "exhaustive": false,
            "entries": [{"path": "a.txt", "type": "file", "content_utf8": "a"}]
        }))
        .unwrap();
        assert!(expected.diff(root.path()).unwrap().is_empty());
        assert!(
            !expected
                .to_json()
                .as_object()
                .unwrap()
                .contains_key("format")
        );
    }

    #[test]
    fn binary_difference_names_sizes() {
        assert_eq!(
            content_detail(&[0xff], &[0xfe, 0]),
            "binary content of 2 bytes instead of 1 bytes"
        );
    }

    #[test]
    fn mtime_difference_is_detected() {
        let root = tempfile::tempdir().unwrap();
        let written = snapshot(json!([
            {"path": "f", "type": "file", "content_utf8": "", "mtime": "2026-01-01T00:00:00Z"}
        ]));
        written.materialize(root.path()).unwrap();
        let expected = snapshot(json!([
            {"path": "f", "type": "file", "content_utf8": "", "mtime": "2026-01-02T00:00:00Z"}
        ]));
        let differences = expected.diff(root.path()).unwrap();
        assert_eq!(differences.len(), 1);
        assert!(
            differences[0]
                .detail
                .contains("modified 2026-01-01T00:00:00Z")
        );
    }

    #[cfg(unix)]
    #[test]
    fn modes_and_symlinks_round_trip_on_unix() {
        let root = tempfile::tempdir().unwrap();
        let expected = snapshot(json!([
            {"path": "run.sh", "type": "file", "content_utf8": "#!/bin/sh\n", "mode": "0755"},
            {"path": "current", "type": "symlink", "target": "run.sh"}
        ]));
        expected.materialize(root.path()).unwrap();
        assert!(expected.diff(root.path()).unwrap().is_empty());
        let captured = Snapshot::capture(root.path(), true, false).unwrap();
        assert_eq!(captured.entries[0].target.as_deref(), Some("run.sh"));
        assert_eq!(captured.entries[1].mode.as_deref(), Some("0755"));

        let changed = snapshot(json!([
            {"path": "run.sh", "type": "file", "content_utf8": "#!/bin/sh\n", "mode": "0700"},
            {"path": "current", "type": "symlink", "target": "other"}
        ]));
        let details: Vec<String> = changed
            .diff(root.path())
            .unwrap()
            .into_iter()
            .map(|d| d.detail)
            .collect();
        assert_eq!(
            details,
            vec![
                "mode 0755 instead of 0700".to_owned(),
                "target \"run.sh\" instead of \"other\"".to_owned()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn directories_without_write_permission_still_receive_their_children() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let expected = snapshot(json!([
            {"path": "locked", "type": "dir", "mode": "0555"},
            {"path": "locked/inner", "type": "dir", "mode": "0500"},
            {"path": "locked/inner/a.txt", "type": "file", "content_utf8": "a"}
        ]));
        expected.materialize(root.path()).unwrap();
        let mode = |path: &str| {
            fs::metadata(root.path().join(path)).unwrap().permissions().mode() & MODE_MASK
        };
        assert_eq!(mode("locked"), 0o555);
        assert_eq!(mode("locked/inner"), 0o500);
        assert_eq!(
            fs::read_to_string(root.path().join("locked/inner/a.txt")).unwrap(),
            "a"
        );
        // The tree is removed by the temporary directory.
        for path in ["locked/inner", "locked"] {
            fs::set_permissions(root.path().join(path), fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }

    #[test]
    fn invalid_snapshots_are_rejected() {
        let cases = [
            json!({"entries": [{"path": "../a", "type": "dir"}]}),
            json!({"entries": [{"path": "/a", "type": "dir"}]}),
            json!({"entries": [{"path": "a\\b", "type": "dir"}]}),
            json!({"entries": [{"path": "a", "type": "file"}]}),
            json!({"entries": [{"path": "a", "type": "file", "content_utf8": "", "content_base64": ""}]}),
            json!({"entries": [{"path": "a", "type": "dir", "content_utf8": ""}]}),
            json!({"entries": [{"path": "a", "type": "symlink"}]}),
            json!({"entries": [{"path": "a", "type": "dir", "target": "b"}]}),
            json!({"entries": [{"path": "a", "type": "file", "content_base64": "!!"}]}),
            json!({"entries": [{"path": "a", "type": "dir", "mode": "755"}]}),
            json!({"entries": [{"path": "a", "type": "dir", "mode": "0999"}]}),
            json!({"entries": [{"path": "a", "type": "dir", "mtime": "yesterday"}]}),
            json!({"entries": [{"path": "a", "type": "dir", "mtime": "2026-01-01T00:00:00.5Z"}]}),
            json!({"entries": [{"path": "a", "type": "dir"}, {"path": "a", "type": "dir"}]}),
            json!({"format": "other/1", "entries": []}),
            json!({"entries": [], "unknown": 1}),
        ];
        for case in cases {
            assert!(Snapshot::from_json(&case).is_err(), "{case}");
        }
    }

    #[test]
    fn load_reads_files_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        fs::write(&path, r#"{"entries": [{"path": "a", "type": "dir"}]}"#).unwrap();
        assert_eq!(Snapshot::load(&path).unwrap().entries.len(), 1);
        fs::write(&path, "{").unwrap();
        assert!(Snapshot::load(&path).is_err());
        let missing = Snapshot::load(&dir.path().join("missing.json")).unwrap_err();
        assert!(missing.to_string().contains("missing.json"));
    }
}
