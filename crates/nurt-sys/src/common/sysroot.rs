// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::common::sysroot
// Responsibility:   Root directory of system files and canonical paths of records.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Root directory of system files and canonical paths of records.
//!
//! Readers of `/proc`, `/etc`, `/run` and `/sys` resolve every system path
//! against a [`SysRoot`]. In production the root is `/`; tests point it at a
//! temporary directory that holds a synthetic tree.

use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value;

/// Directory against which absolute system paths are resolved.
///
/// ```
/// use nurt_sys::host::SysRoot;
///
/// let root = SysRoot::new("/tmp/tree");
/// assert_eq!(root.path("/proc/meminfo"), std::path::Path::new("/tmp/tree/proc/meminfo"));
/// assert!(!root.is_live());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysRoot {
    root: PathBuf,
}

impl SysRoot {
    /// The root of the running system.
    #[must_use]
    pub fn live() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    /// A root at `root`, holding a synthetic system tree.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Location of the absolute system path `path` under this root.
    #[must_use]
    pub fn path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        self.root.join(path.strip_prefix("/").unwrap_or(path))
    }

    /// Returns `true` for the root of the running system, where lookups that
    /// do not go through files (account databases of the system library,
    /// system calls about the current process) are allowed.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.root == Path::new("/")
    }

    /// Returns the content of the system file `path` as text.
    ///
    /// System files are not guaranteed to be UTF-8 (names of accounts, mount
    /// points and file names are bytes), so bytes that are not valid UTF-8
    /// are replaced with U+FFFD instead of failing the whole read.
    ///
    /// # Errors
    ///
    /// Returns the error of the file system, for example
    /// [`io::ErrorKind::NotFound`].
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        let bytes = std::fs::read(self.path(path))?;
        Ok(match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
        })
    }
}

impl Default for SysRoot {
    fn default() -> Self {
        Self::live()
    }
}

/// Account that can read every system source.
#[cfg(unix)]
pub(crate) const REQUIRED_PRIVILEGE: &str = "root";
/// Account that can read every system source.
#[cfg(windows)]
pub(crate) const REQUIRED_PRIVILEGE: &str = "an administrator";

/// Error for a system source `source` that cannot be read: `E_PRIVILEGE`
/// with the required account when access is denied, `E_IO` otherwise.
pub(crate) fn source_error(error: &io::Error, source: &str) -> Error {
    if error.kind() == io::ErrorKind::PermissionDenied {
        Error::new(
            ErrorCode::Privilege,
            format!("cannot read {source}: permission denied; run as {REQUIRED_PRIVILEGE}"),
        )
        .with_detail("required", serde_json::json!(REQUIRED_PRIVILEGE))
    } else {
        Error::new(ErrorCode::Io, format!("cannot read {source}: {error}"))
    }
}

/// Canonical form of a native path for the path fields of records: `/` as
/// separator, upper-case drive letter, UNC paths as `//server/share`, without
/// the verbatim prefix `\\?\`.
pub(crate) fn canonical_path(path: &Path) -> value::Path {
    let mut text = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => text.push_str(&prefix_text(prefix.kind())),
            Component::RootDir => text.push('/'),
            Component::CurDir => {}
            Component::ParentDir => push_segment(&mut text, ".."),
            Component::Normal(name) => push_segment(&mut text, &name.to_string_lossy()),
        }
    }
    value::Path::new(text)
}

fn push_segment(text: &mut String, segment: &str) {
    if !text.is_empty() && !text.ends_with('/') {
        text.push('/');
    }
    text.push_str(segment);
}

fn prefix_text(prefix: Prefix<'_>) -> String {
    match prefix {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            format!("{}:", char::from(letter).to_ascii_uppercase())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            format!("//{}/{}", server.to_string_lossy(), share.to_string_lossy())
        }
        Prefix::Verbatim(name) | Prefix::DeviceNS(name) => {
            format!("//./{}", name.to_string_lossy())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SysRoot;

    #[test]
    fn sysroot_read_to_string_replaces_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("table"), b"a\xFFb\nc\n").unwrap();
        std::fs::write(dir.path().join("plain"), "naïve café ü\n").unwrap();
        let root = SysRoot::new(dir.path());
        assert_eq!(root.read_to_string("/table").unwrap(), "a\u{FFFD}b\nc\n");
        assert_eq!(root.read_to_string("/plain").unwrap(), "naïve café ü\n");
        let missing = root.read_to_string("/absent").unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
