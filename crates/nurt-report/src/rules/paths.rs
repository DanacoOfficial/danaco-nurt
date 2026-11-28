// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::rules::paths
// Responsibility:   Classification of canonical paths: system directories, configuration in the home directory and project roots.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Classification of canonical paths: system directories, configuration in
//! the home directory and project roots.
//!
//! Paths are compared in canonical form: `/` as separator, absolute, upper-case
//! drive letter on Windows. Components are compared without regard to letter
//! case on Windows and macOS, whose default file systems ignore case, and
//! exactly on Linux.

use nurt_contracts::env::{Host, Os};

/// System directories of Linux. A trailing `*` matches any suffix of the last
/// component; the library directories are named one by one, as `/lib*` would
/// also cover `/library` and `/libexec`.
const LINUX_SYSTEM: &[&str] = &[
    "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/boot", "/sys", "/proc",
];

/// System directories of macOS.
const MACOS_SYSTEM: &[&str] = &[
    "/etc",
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library",
    "/private/etc",
];

/// System directories of Windows.
const WINDOWS_SYSTEM: &[&str] = &["C:/Windows", "C:/Program Files*", "C:/ProgramData"];

/// Configuration entries relative to the home directory on every system.
const HOME_CONFIG: &[&str] = &[".ssh", ".gnupg", ".config", ".bashrc", ".zshrc", ".profile"];

/// Configuration entries relative to the home directory on macOS only.
const MACOS_HOME_CONFIG: &[&str] = &["Library/Preferences"];

/// Default location of `%APPDATA%` relative to the home directory on Windows.
const WINDOWS_HOME_CONFIG: &[&str] = &["AppData/Roaming"];

/// Environment variable that names the roaming application data directory on
/// Windows.
const APPDATA_VARIABLE: &str = "APPDATA";

/// Host facts used to classify target paths.
///
/// The operating system selects the list of system directories and decides
/// whether components are compared without regard to letter case; the home
/// directory anchors the configuration entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathContext {
    os: Os,
    home: Option<String>,
    app_data: Option<String>,
}

impl Default for PathContext {
    fn default() -> Self {
        Self::new(Os::Linux)
    }
}

impl PathContext {
    /// Creates a context for the given operating system, without a home
    /// directory.
    #[must_use]
    pub const fn new(os: Os) -> Self {
        Self {
            os,
            home: None,
            app_data: None,
        }
    }

    /// Reads the operating system, the home directory and, on Windows,
    /// `%APPDATA%` from the host.
    #[must_use]
    pub fn from_host(host: &dyn Host) -> Self {
        let info = host.info();
        let context = Self::new(info.os).with_home(info.home.to_string_lossy());
        if info.os != Os::Windows {
            return context;
        }
        match host.env_var(APPDATA_VARIABLE) {
            Some(dir) if !dir.is_empty() => context.with_app_data(dir.to_string_lossy()),
            _ => context,
        }
    }

    /// Sets the home directory of the account.
    #[must_use]
    pub fn with_home(mut self, home: impl AsRef<str>) -> Self {
        self.home = normalize_root(home.as_ref());
        self
    }

    /// Sets the roaming application data directory (`%APPDATA%`) when it
    /// differs from the default location under the home directory.
    #[must_use]
    pub fn with_app_data(mut self, dir: impl AsRef<str>) -> Self {
        self.app_data = normalize_root(dir.as_ref());
        self
    }

    /// Operating system of the host.
    #[must_use]
    pub const fn os(&self) -> Os {
        self.os
    }

    /// Home directory in canonical form, when known.
    #[must_use]
    pub fn home(&self) -> Option<&str> {
        self.home.as_deref()
    }

    /// Whether path components are compared without regard to letter case.
    #[must_use]
    pub fn folds_case(&self) -> bool {
        matches!(self.os, Os::Windows | Os::Macos)
    }

    /// Whether `path` is `root` or lies below it, comparing whole components.
    ///
    /// The path `/srv/application` does not lie below `/srv/app`.
    #[must_use]
    pub fn is_within(&self, path: &str, root: &str) -> bool {
        matches_pattern(path, root.trim_end_matches('/'), false, self.folds_case())
    }

    /// Whether `path` lies in a system directory of the host.
    #[must_use]
    pub fn is_system_path(&self, path: &str) -> bool {
        let fold = self.folds_case();
        self.system_directories()
            .iter()
            .any(|pattern| matches_pattern(path, pattern, true, fold))
    }

    /// Whether `path` is a configuration file or directory in the home
    /// directory: `.ssh`, `.gnupg`, `.config`, shell start-up files,
    /// `Library/Preferences` on macOS and `%APPDATA%` on Windows.
    #[must_use]
    pub fn is_home_config(&self, path: &str) -> bool {
        if self
            .app_data
            .as_deref()
            .is_some_and(|dir| self.is_within(path, dir))
        {
            return true;
        }
        let Some(home) = self.home.as_deref() else {
            return false;
        };
        if !self.is_within(path, home) {
            return false;
        }
        let fold = self.folds_case();
        let relative = relative_to(path, home);
        HOME_CONFIG
            .iter()
            .chain(self.home_config_extra())
            .any(|entry| matches_pattern(relative, entry, false, fold))
    }

    fn system_directories(&self) -> &'static [&'static str] {
        match self.os {
            Os::Windows => WINDOWS_SYSTEM,
            Os::Macos => MACOS_SYSTEM,
            _ => LINUX_SYSTEM,
        }
    }

    fn home_config_extra(&self) -> &'static [&'static str] {
        match self.os {
            Os::Windows => WINDOWS_HOME_CONFIG,
            Os::Macos => MACOS_HOME_CONFIG,
            _ => &[],
        }
    }
}

/// Whether `path` is a file system path in canonical form rather than an
/// identifier of a provider such as `service:nginx` or `registry:/HKCU`.
#[must_use]
pub fn is_fs_path(path: &str) -> bool {
    match path.as_bytes() {
        [b'/', ..] => true,
        [drive, b':'] | [drive, b':', b'/', ..] => drive.is_ascii_alphabetic(),
        _ => false,
    }
}

/// Directory that contains `path`, or `None` for a root or a path without a
/// separator.
pub(crate) fn parent(path: &str) -> Option<&str> {
    let (dir, name) = path.rsplit_once('/')?;
    if name.is_empty() {
        return None;
    }
    Some(if dir.is_empty() { "/" } else { dir })
}

/// Shortens `prefix` to the longest run of leading components it shares with
/// `path`.
pub(crate) fn shrink_to_common(prefix: &mut String, path: &str) {
    let mut keep = 0;
    let mut matched = true;
    for (offset, (left, right)) in prefix.split('/').zip(path.split('/')).enumerate() {
        if left != right {
            matched = false;
            break;
        }
        keep += left.len() + usize::from(offset > 0);
    }
    if matched && prefix.len() > path.len() {
        keep = path.len();
    }
    prefix.truncate(keep.min(prefix.len()));
}

/// Part of `path` below `root`, for a `path` known to lie within `root`.
///
/// Components are counted rather than bytes, because a match without regard
/// to letter case may pair texts of different byte lengths.
fn relative_to<'p>(path: &'p str, root: &str) -> &'p str {
    if root == "/" {
        return path.trim_start_matches('/');
    }
    let depth = root.split('/').count();
    path.splitn(depth + 1, '/').nth(depth).unwrap_or("")
}

/// The common ancestor `prefix` as a reportable path: `None` for an empty
/// prefix, the root directory or a bare drive.
pub(crate) fn reportable_ancestor(prefix: String) -> Option<String> {
    let meaningful = prefix
        .split('/')
        .filter(|component| !component.is_empty())
        .nth(usize::from(is_drive(&prefix)))
        .is_some();
    meaningful.then_some(prefix)
}

fn is_drive(path: &str) -> bool {
    matches!(path.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
}

/// Canonical form of a configured directory: `/` separators, no trailing
/// separator except for the root; `None` when empty.
pub(crate) fn normalize_root(dir: &str) -> Option<String> {
    let dir = dir.replace('\\', "/");
    let trimmed = dir.trim_end_matches('/');
    match (trimmed.is_empty(), dir.is_empty()) {
        (_, true) => None,
        (true, false) => Some("/".to_owned()),
        (false, false) => Some(trimmed.to_owned()),
    }
}

/// Whether the leading components of `path` match `pattern`.
///
/// With `wildcard`, a trailing `*` in the last component of the pattern
/// matches any suffix of the corresponding path component.
fn matches_pattern(path: &str, pattern: &str, wildcard: bool, fold: bool) -> bool {
    if pattern.is_empty() {
        return path.starts_with('/');
    }
    let mut components = path.split('/');
    let mut expected = pattern.split('/').peekable();
    while let Some(want) = expected.next() {
        let Some(have) = components.next() else {
            return false;
        };
        let last = expected.peek().is_none();
        let matched = match want.strip_suffix('*') {
            Some(stem) if wildcard && last => starts_with(have, stem, fold),
            _ => equal(have, want, fold),
        };
        if !matched {
            return false;
        }
    }
    true
}

fn equal(left: &str, right: &str, fold: bool) -> bool {
    if fold {
        left.chars()
            .flat_map(char::to_lowercase)
            .eq(right.chars().flat_map(char::to_lowercase))
    } else {
        left == right
    }
}

fn starts_with(text: &str, stem: &str, fold: bool) -> bool {
    if !fold {
        return text.starts_with(stem);
    }
    let mut text = text.chars().flat_map(char::to_lowercase);
    stem.chars()
        .flat_map(char::to_lowercase)
        .all(|want| text.next() == Some(want))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn common(paths: &[&str]) -> Option<String> {
        let mut prefix = paths[0].to_owned();
        for path in &paths[1..] {
            shrink_to_common(&mut prefix, path);
        }
        reportable_ancestor(prefix)
    }

    #[test]
    fn common_ancestor_stops_at_component_boundaries() {
        assert_eq!(
            common(&["/srv/app/build/a.tmp", "/srv/app/build/cache/1.o"]),
            Some("/srv/app/build".to_owned())
        );
        assert_eq!(
            common(&["/srv/app/ab", "/srv/app/ac"]),
            Some("/srv/app".to_owned())
        );
        assert_eq!(
            common(&["/srv/app/a", "/srv/app"]),
            Some("/srv/app".to_owned())
        );
        assert_eq!(
            common(&["/srv/app", "/srv/app/a"]),
            Some("/srv/app".to_owned())
        );
        assert_eq!(common(&["/srv/a", "/opt/b"]), None);
        assert_eq!(common(&["C:/Work/a", "D:/Work/a"]), None);
        assert_eq!(common(&["C:/Work/a", "C:/Temp/a"]), None);
        assert_eq!(
            common(&["C:/Work/a/x", "C:/Work/a/y"]),
            Some("C:/Work/a".to_owned())
        );
        assert_eq!(common(&["service:nginx", "service:redis"]), None);
        assert_eq!(common(&["/srv/app/one"]), Some("/srv/app/one".to_owned()));
    }

    #[test]
    fn parent_of_canonical_paths() {
        assert_eq!(parent("/srv/app/a.go"), Some("/srv/app"));
        assert_eq!(parent("/a.go"), Some("/"));
        assert_eq!(parent("C:/a.go"), Some("C:"));
        assert_eq!(parent("service:nginx"), None);
        assert_eq!(parent("/srv/app/"), None);
    }

    #[test]
    fn normalize_root_converts_separators_and_trims() {
        assert_eq!(
            normalize_root("C:\\Users\\user\\"),
            Some("C:/Users/user".to_owned())
        );
        assert_eq!(normalize_root("/"), Some("/".to_owned()));
        assert_eq!(normalize_root(""), None);
    }

    #[test]
    fn root_pattern_contains_every_absolute_path() {
        let linux = PathContext::new(Os::Linux);
        assert!(linux.is_within("/srv", "/"));
        assert!(!linux.is_within("C:/srv", "/"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
