// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! System root and host environment of the package readers.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use nurt_contracts::env::{Host, Os};

/// Directory under which the readers look for system files.
///
/// The root of a running system is the file system itself. A directory
/// substitutes the root in tests and when an image of a system is inspected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SysRoot {
    base: Option<PathBuf>,
}

impl SysRoot {
    /// The file system of the running host.
    #[must_use]
    pub fn host() -> Self {
        Self { base: None }
    }

    /// A directory that stands for the root of a system.
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            base: Some(dir.into()),
        }
    }

    /// Path of `path` in this root.
    ///
    /// Drive prefixes and leading separators of `path` are dropped when the
    /// root is a directory, and both separators are accepted.
    #[must_use]
    pub fn resolve(&self, path: &str) -> PathBuf {
        let Some(base) = &self.base else {
            return PathBuf::from(path);
        };
        let bytes = path.as_bytes();
        let without_drive = if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
        {
            &path[2..]
        } else {
            path
        };
        let mut resolved = base.clone();
        for part in without_drive.split(['/', '\\']).filter(|part| !part.is_empty()) {
            resolved.push(part);
        }
        resolved
    }

    /// Returns `true` when `path` exists in this root.
    #[must_use]
    pub fn exists(&self, path: &str) -> bool {
        self.resolve(path).exists()
    }

    /// Returns `true` when `path` is a directory in this root.
    #[must_use]
    pub fn is_dir(&self, path: &str) -> bool {
        self.resolve(path).is_dir()
    }

    /// Entries of the directory `path`, sorted by path; empty when the
    /// directory cannot be read.
    #[must_use]
    pub fn list_dir(&self, path: &str) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.resolve(path)) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok()).map(|e| e.path()).collect();
        paths.sort();
        paths
    }
}

/// Root, operating system and environment variables seen by the readers.
#[derive(Debug, Clone)]
pub struct HostEnv {
    root: SysRoot,
    os: Os,
    vars: BTreeMap<String, String>,
}

impl HostEnv {
    /// An environment without variables.
    #[must_use]
    pub fn new(root: SysRoot, os: Os) -> Self {
        Self {
            root,
            os,
            vars: BTreeMap::new(),
        }
    }

    /// Environment of the host of an execution.
    #[must_use]
    pub fn from_host(root: SysRoot, host: &dyn Host) -> Self {
        let vars = host
            .env_vars()
            .into_iter()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        Self {
            root,
            os: host.info().os,
            vars,
        }
    }

    /// The same environment with one more variable.
    #[must_use]
    pub fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }

    /// System root.
    #[must_use]
    pub fn root(&self) -> &SysRoot {
        &self.root
    }

    /// Operating system family.
    #[must_use]
    pub fn os(&self) -> Os {
        self.os
    }

    /// Value of a variable; names are case-insensitive on Windows.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        if let Some(value) = self.vars.get(name) {
            return Some(value.as_str());
        }
        if self.os == Os::Windows {
            return self
                .vars
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str());
        }
        None
    }

    /// Value of a variable that is set and not empty.
    #[must_use]
    pub fn var_nonempty(&self, name: &str) -> Option<&str> {
        self.var(name).filter(|value| !value.is_empty())
    }

    /// Home directory of the user.
    #[must_use]
    pub fn home(&self) -> Option<&str> {
        match self.os {
            Os::Windows => self.var_nonempty("USERPROFILE"),
            _ => self.var_nonempty("HOME"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_strips_drive_and_separators_under_directory_root() {
        let root = SysRoot::at("/tmp/image");
        assert_eq!(
            root.resolve("C:\\ProgramData\\chocolatey"),
            PathBuf::from("/tmp/image/ProgramData/chocolatey")
        );
        assert_eq!(
            root.resolve("/var/lib/dpkg/status"),
            PathBuf::from("/tmp/image/var/lib/dpkg/status")
        );
    }

    #[test]
    fn resolve_keeps_path_of_host_root() {
        assert_eq!(SysRoot::host().resolve("/etc"), PathBuf::from("/etc"));
    }

    #[test]
    fn windows_variable_lookup_ignores_case() {
        let env = HostEnv::new(SysRoot::host(), Os::Windows).with_var("LocalAppData", "C:\\L");
        assert_eq!(env.var("LOCALAPPDATA"), Some("C:\\L"));
        let linux = HostEnv::new(SysRoot::host(), Os::Linux).with_var("Home", "/h");
        assert_eq!(linux.var("HOME"), None);
    }

    #[test]
    fn list_dir_is_sorted_and_empty_for_missing_directory() {
        let dir = tempfile::tempdir().expect("temporary directory");
        fs::write(dir.path().join("b"), "").expect("write");
        fs::write(dir.path().join("a"), "").expect("write");
        let root = SysRoot::at(dir.path());
        let names: Vec<_> = root
            .list_dir("/")
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, ["a", "b"]);
        assert!(root.list_dir("/missing").is_empty());
        assert!(root.exists("/a") && !root.is_dir("/a"));
    }
}
