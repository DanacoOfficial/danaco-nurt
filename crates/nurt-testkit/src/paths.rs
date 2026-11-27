// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Absolute paths of the tests on the current system.
//!
//! Tests write absolute paths in POSIX form, such as `/data/a.txt`. On
//! Windows such a path has no drive and reads as relative, so a test builds
//! its absolute paths with [`abs`], which puts [`ROOT`] in front of them.

use std::path::PathBuf;

/// Prefix of an absolute path: empty on Unix, the drive `C:` on Windows.
pub const ROOT: &str = if cfg!(windows) { "C:" } else { "" };

/// `path`, an absolute path in POSIX form, as an absolute path of the
/// current system.
#[must_use]
pub fn abs(path: &str) -> String {
    format!("{ROOT}{path}")
}

/// [`abs`] of `path` as a [`PathBuf`].
#[must_use]
pub fn abs_path(path: &str) -> PathBuf {
    PathBuf::from(abs(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_are_absolute_on_the_current_system() {
        assert!(abs_path("/data/a.txt").is_absolute());
        assert!(abs("/data/a.txt").ends_with("/data/a.txt"));
    }
}
