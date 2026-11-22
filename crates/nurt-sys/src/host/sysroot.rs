// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Root directory of system files and canonical paths of records.
//!
//! Readers of `/proc`, `/etc`, `/run` and `/sys` resolve every system path
//! against a [`SysRoot`]. In production the root is `/`; tests point it at a
//! temporary directory that holds a synthetic tree.

use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

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
    /// # Errors
    ///
    /// Returns the error of the file system, for example
    /// [`io::ErrorKind::NotFound`].
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        std::fs::read_to_string(self.path(path))
    }
}

impl Default for SysRoot {
    fn default() -> Self {
        Self::live()
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
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
            "//{}/{}",
            server.to_string_lossy(),
            share.to_string_lossy()
        ),
        Prefix::Verbatim(name) | Prefix::DeviceNS(name) => {
            format!("//./{}", name.to_string_lossy())
        }
    }
}
