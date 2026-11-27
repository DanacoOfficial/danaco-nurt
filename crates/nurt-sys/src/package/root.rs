// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::root
// Responsibility:   System paths of the package readers.
// Dependencies:     nurt_contracts, nurt_programs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! System paths of the package readers.
//!
//! The readers write system paths as on the host (`/var/lib/dpkg/status`,
//! `C:\ProgramData\chocolatey`) and resolve them through
//! [`HostEnv::sys_path`], so that a directory can stand for the root of a
//! system in tests and when an image of a system is inspected. The same
//! environment is passed to the database readers of the program catalog
//! crate, so both follow one convention of paths.

use std::fs;
use std::path::PathBuf;

pub(crate) use nurt_programs::source::HostEnv;

/// File system access to host paths of a [`HostEnv`].
pub(crate) trait SysPaths {
    /// File system path of the host path `path`.
    fn resolve(&self, path: &str) -> PathBuf;

    /// Returns `true` when the host path `path` exists.
    fn exists(&self, path: &str) -> bool {
        self.resolve(path).exists()
    }

    /// Returns `true` when the host path `path` is a directory.
    fn is_dir(&self, path: &str) -> bool {
        self.resolve(path).is_dir()
    }

    /// Entries of the directory at the host path `path`, as file system
    /// paths sorted by name; empty when the directory cannot be read.
    fn list_dir(&self, path: &str) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.resolve(path)) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        paths
    }
}

impl SysPaths for HostEnv {
    fn resolve(&self, path: &str) -> PathBuf {
        self.sys_path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::env::Os;

    #[test]
    fn resolve_maps_drive_letter_to_directory_below_root() {
        let env = HostEnv::new(Os::Windows).with_root("/srv/image");
        assert_eq!(
            env.resolve("C:\\ProgramData\\chocolatey"),
            PathBuf::from("/srv/image/c/ProgramData/chocolatey")
        );
        let linux = HostEnv::new(Os::Linux).with_root("/srv/image");
        assert_eq!(
            linux.resolve("/var/lib/dpkg/status"),
            PathBuf::from("/srv/image/var/lib/dpkg/status")
        );
    }

    #[test]
    fn resolve_keeps_host_path_without_root() {
        assert_eq!(
            HostEnv::new(Os::Linux).resolve("/etc"),
            PathBuf::from("/etc")
        );
    }

    #[test]
    fn list_dir_is_sorted_and_empty_for_missing_directory() {
        let dir = tempfile::tempdir().expect("temporary directory");
        fs::write(dir.path().join("b"), "").expect("write");
        fs::write(dir.path().join("a"), "").expect("write");
        let env = HostEnv::new(Os::Linux).with_root(dir.path());
        let names: Vec<_> = env
            .list_dir("/")
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(names, ["a", "b"]);
        assert!(env.list_dir("/missing").is_empty());
        assert!(env.exists("/a") && !env.is_dir("/a"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
