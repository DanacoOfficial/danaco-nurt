// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::package::installed
// Responsibility:   Reading of the installed packages from the databases of the package managers without starting a program.
// Dependencies:     nurt_contracts, nurt_programs
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Installed packages.
//!
//! The package commands read the installed packages through
//! [`InstalledSource`]. [`ManagerDatabases`] reads the databases of the
//! managers with the readers of the program catalog crate: dpkg, RPM and
//! pacman databases, the Homebrew cellar, the winget tracking database and
//! registry, the Scoop application directories and the Chocolatey library.
//! No program is started.

use std::fmt::Write as _;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::PackageManager;
use nurt_programs::model::{RawPackage, SourceData};
use nurt_programs::source::{Deadline, ProgramSource, SourceError, SourceErrorKind, SourceWarning};
use nurt_programs::sources::brew::BrewSource;
use nurt_programs::sources::choco::ChocoSource;
use nurt_programs::sources::scoop::ScoopSource;
use nurt_programs::sources::winget::WingetSource;
use nurt_programs::sources::{dpkg, pacman, rpm};

use super::root::HostEnv;

/// Installed packages of one manager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstalledSet {
    /// Packages in the order of the manager database.
    pub packages: Vec<RawPackage>,
    /// Entries that were skipped, described briefly.
    pub warnings: Vec<String>,
}

impl From<SourceData> for InstalledSet {
    fn from(data: SourceData) -> Self {
        Self {
            packages: data.packages,
            warnings: data.warnings.iter().map(warning_text).collect(),
        }
    }
}

fn warning_text(warning: &SourceWarning) -> String {
    let mut text = format!("{}: {}", warning.code.as_str(), warning.reason);
    if let Some(path) = &warning.path {
        let _ = write!(text, " ({path})");
    }
    text
}

/// Reader of the installed packages of a manager.
pub trait InstalledSource: Send + Sync {
    /// Returns the packages installed through `manager`.
    ///
    /// A manager the source does not read yields an empty set with a warning.
    ///
    /// # Errors
    ///
    /// Returns `E_NOT_FOUND` when the database of the manager is missing,
    /// `E_PERMISSION` when it cannot be opened, `E_FORMAT` when it is damaged
    /// and `E_TIMEOUT` when the reading did not finish in time.
    fn installed(&self, manager: PackageManager, env: &HostEnv) -> Result<InstalledSet>;
}

/// Reader of the package databases of the managers.
#[derive(Debug, Clone, Copy, Default)]
pub struct ManagerDatabases;

impl InstalledSource for ManagerDatabases {
    fn installed(&self, manager: PackageManager, env: &HostEnv) -> Result<InstalledSet> {
        let deadline = Deadline::none();
        let data = match manager {
            PackageManager::Apt | PackageManager::Dpkg => dpkg::read_packages(env),
            PackageManager::Dnf | PackageManager::Zypper | PackageManager::Rpm => {
                rpm::read_packages(env)
            }
            PackageManager::Pacman => pacman::read_packages(env),
            PackageManager::Brew => BrewSource::new().load(env, &deadline),
            PackageManager::Winget => WingetSource::new().load(env, &deadline),
            PackageManager::Scoop => ScoopSource::new().load(env, &deadline),
            PackageManager::Choco => ChocoSource::new().load(env, &deadline),
            _ => {
                return Ok(InstalledSet {
                    packages: Vec::new(),
                    warnings: vec![format!("installed packages of {manager} are not readable")],
                });
            }
        };
        data.map(InstalledSet::from)
            .map_err(|error| database_error(manager, &error))
    }
}

fn database_error(manager: PackageManager, error: &SourceError) -> Error {
    let code = match error.kind {
        SourceErrorKind::Unavailable => ErrorCode::NotFound,
        SourceErrorKind::Permission => ErrorCode::Permission,
        SourceErrorKind::Corrupt => ErrorCode::Format,
        SourceErrorKind::Timeout => ErrorCode::Timeout,
    };
    let mut converted = Error::new(
        code,
        format!(
            "cannot read the package database of {manager}: {}",
            error.message
        ),
    );
    if let Some(path) = &error.path {
        converted = converted.with_detail("path", serde_json::json!(path));
    }
    converted
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::env::Os;
    use std::fs;
    use std::path::Path;

    fn put(root: &Path, path: &str, content: &str) {
        let target = root.join(path);
        fs::create_dir_all(target.parent().expect("parent")).expect("directories");
        fs::write(target, content).expect("file");
    }

    /// Status file of dpkg with one installed package and one removed package whose
    /// configuration files remain.
    const DPKG_STATUS: &str = "Package: curl\nStatus: install ok installed\nVersion: 8.5.0-2\nArchitecture: amd64\nInstalled-Size: 500\nDescription: command line tool\n\n\
         Package: old\nStatus: deinstall ok config-files\nVersion: 1\nArchitecture: amd64\n\n";

    #[test]
    fn dpkg_database_lists_only_installed_packages() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(dir.path(), "var/lib/dpkg/status", DPKG_STATUS);
        let env = HostEnv::new(Os::Linux).with_root(dir.path());
        let set = ManagerDatabases
            .installed(PackageManager::Apt, &env)
            .expect("installed packages");
        let names: Vec<_> = set.packages.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["curl"]);
        assert_eq!(set.packages[0].version.as_deref(), Some("8.5.0-2"));
        assert_eq!(set.packages[0].installed_size, Some(512_000));
    }

    #[test]
    fn pacman_database_lists_local_packages() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(
            dir.path(),
            "var/lib/pacman/local/bash-5.2.026-2/desc",
            "%NAME%\nbash\n\n%VERSION%\n5.2.026-2\n\n%DESC%\nThe GNU Bourne Again shell\n\n%ARCH%\nx86_64\n\n",
        );
        let env = HostEnv::new(Os::Linux).with_root(dir.path());
        let set = ManagerDatabases
            .installed(PackageManager::Pacman, &env)
            .expect("installed packages");
        assert_eq!(set.packages.len(), 1);
        assert_eq!(set.packages[0].name, "bash");
        assert_eq!(set.packages[0].version.as_deref(), Some("5.2.026-2"));
    }

    #[test]
    fn missing_database_is_an_error_and_other_managers_warn() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let env = HostEnv::new(Os::Linux).with_root(dir.path());
        assert!(
            ManagerDatabases
                .installed(PackageManager::Apt, &env)
                .is_err()
        );
        let set = ManagerDatabases
            .installed(PackageManager::Other, &env)
            .expect("empty set");
        assert!(set.packages.is_empty());
        assert_eq!(set.warnings.len(), 1);
    }

    #[test]
    fn source_errors_map_to_contract_codes() {
        let id = nurt_programs::source::SourceId::new("dpkg");
        let cases = [
            (SourceErrorKind::Unavailable, ErrorCode::NotFound),
            (SourceErrorKind::Permission, ErrorCode::Permission),
            (SourceErrorKind::Corrupt, ErrorCode::Format),
            (SourceErrorKind::Timeout, ErrorCode::Timeout),
        ];
        for (kind, code) in cases {
            let error = SourceError::new(&id, kind, "status unreadable").at("/var/lib/dpkg/status");
            let converted = database_error(PackageManager::Apt, &error);
            assert_eq!(converted.code(), code);
            assert!(converted.data().message.contains("status unreadable"));
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
