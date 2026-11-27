// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-programs::catalog::paths
// Responsibility:   Relative paths of a catalog and their confinement to the catalog root.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Relative paths of a catalog and their confinement to the catalog root.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Longest relative path accepted in a catalog.
pub const MAX_RELPATH_CHARS: usize = 512;

/// Device names that Windows reserves in every directory, compared without
/// case and without extension.
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters that a path segment never contains: separators and characters
/// that Windows rejects in file names.
const FORBIDDEN_CHARS: [char; 8] = ['\\', ':', '*', '?', '"', '<', '>', '|'];

/// Reason a path named by a catalog is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// The text does not satisfy the rules of a relative catalog path.
    Syntax(String),
    /// The path does not name an existing file or directory.
    Missing,
    /// The resolved path lies outside the catalog root.
    Outside,
    /// The path cannot be resolved.
    Unreadable(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(reason) => f.write_str(reason),
            Self::Missing => f.write_str("file does not exist; add it or correct the path"),
            Self::Outside => f.write_str(
                "path resolves outside the catalog root; keep files and links inside the catalog",
            ),
            Self::Unreadable(reason) => write!(f, "path cannot be resolved: {reason}"),
        }
    }
}

/// Checks that `path` is a relative catalog path: `/` separators, no empty
/// segment, no segment of dots only, no backslash, colon, wildcard or control
/// character, no reserved device name of Windows and at most
/// [`MAX_RELPATH_CHARS`] characters.
///
/// # Errors
///
/// Returns [`PathError::Syntax`] with the rule that the path breaks.
///
/// # Examples
///
/// ```
/// use nurt_programs::catalog::check_relpath;
///
/// assert!(check_relpath("guides/ffmpeg.md").is_ok());
/// assert!(check_relpath("../outside.md").is_err());
/// assert!(check_relpath("/etc/passwd").is_err());
/// ```
pub fn check_relpath(path: &str) -> Result<(), PathError> {
    let syntax = |reason: &str| Err(PathError::Syntax(reason.to_owned()));
    if path.is_empty() {
        return syntax("path is empty; name a file relative to the catalog root");
    }
    if path.chars().count() > MAX_RELPATH_CHARS {
        return syntax("path is longer than 512 characters; shorten it");
    }
    if path.starts_with('/') {
        return syntax("path is absolute; write it relative to the catalog root");
    }
    for segment in path.split('/') {
        check_segment(segment)?;
    }
    Ok(())
}

fn check_segment(segment: &str) -> Result<(), PathError> {
    let syntax = |reason: String| Err(PathError::Syntax(reason));
    if segment.is_empty() {
        return syntax("path has an empty segment; remove the repeated `/`".to_owned());
    }
    if segment.chars().all(|c| c == '.') {
        return syntax(format!(
            "path segment `{segment}` is not permitted; name files inside the catalog root"
        ));
    }
    if let Some(c) = segment
        .chars()
        .find(|c| FORBIDDEN_CHARS.contains(c) || c.is_control())
    {
        return syntax(format!(
            "path contains the character {c:?}; use `/` as separator and portable file names"
        ));
    }
    let stem = segment.split('.').next().unwrap_or_default();
    if RESERVED_NAMES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(stem))
    {
        return syntax(format!(
            "path segment `{segment}` is a reserved device name of Windows; rename the file"
        ));
    }
    Ok(())
}

/// Root directory of a catalog with the operations that keep every file
/// access inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Root {
    canonical: PathBuf,
}

impl Root {
    /// Opens the root directory; symbolic links in `dir` are resolved.
    pub(crate) fn open(dir: &Path) -> io::Result<Self> {
        let canonical = dir.canonicalize()?;
        if canonical.is_dir() {
            Ok(Self { canonical })
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "catalog root is not a directory",
            ))
        }
    }

    /// Resolved path of the root.
    pub(crate) fn path(&self) -> &Path {
        &self.canonical
    }

    /// Resolves a relative catalog path to an existing file system path
    /// inside the root, following symbolic links.
    pub(crate) fn resolve(&self, relative: &str) -> Result<PathBuf, PathError> {
        check_relpath(relative)?;
        let joined = relative
            .split('/')
            .fold(self.canonical.clone(), |path, segment| path.join(segment));
        let resolved = match joined.canonicalize() {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(PathError::Missing);
            }
            Err(error) => return Err(PathError::Unreadable(error.to_string())),
        };
        if resolved.starts_with(&self.canonical) {
            Ok(resolved)
        } else {
            Err(PathError::Outside)
        }
    }

    /// Relative catalog path of a resolved path inside the root.
    pub(crate) fn relative(&self, path: &Path) -> Option<String> {
        let rest = path.strip_prefix(&self.canonical).ok()?;
        let parts: Vec<String> = rest
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        (!parts.is_empty()).then(|| parts.join("/"))
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
