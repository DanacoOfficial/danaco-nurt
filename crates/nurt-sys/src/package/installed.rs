// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Source of the installed packages.
//!
//! The package commands read the installed packages through
//! [`InstalledSource`]. The record of [`InstalledPackage`] mirrors the package
//! metadata of the program catalog crate, so that its readers can be attached
//! with a mapping of fields. [`DpkgStatusSource`] reads the dpkg status file
//! only and serves until those readers are attached.

use std::fs::File;
use std::io::BufReader;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::PackageManager;

use super::control::for_each_stanza;
use super::root::HostEnv;

/// Path of the dpkg status file under the system root.
pub(crate) const DPKG_STATUS: &str = "/var/lib/dpkg/status";

/// Number of bytes in one kibibyte, the unit of `Installed-Size`.
const KIBIBYTE: u64 = 1024;

/// Metadata of one installed package.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstalledPackage {
    /// Package name.
    pub name: String,
    /// Full version as the manager writes it.
    pub version: Option<String>,
    /// Architecture.
    pub architecture: Option<String>,
    /// One-line summary.
    pub summary: Option<String>,
    /// Source package or repository.
    pub origin: Option<String>,
    /// Installed size in bytes.
    pub installed_size: Option<u64>,
}

/// Installed packages of one manager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstalledSet {
    /// Packages in the order of the manager database.
    pub packages: Vec<InstalledPackage>,
    /// Entries that were skipped, described briefly.
    pub warnings: Vec<String>,
}

/// Reader of the installed packages of a manager.
pub trait InstalledSource: Send + Sync {
    /// Packages installed through `manager`.
    ///
    /// A manager the source does not read yields an empty set with a warning.
    ///
    /// # Errors
    ///
    /// Returns `E_FORMAT` when the database of the manager exists and cannot
    /// be read.
    fn installed(&self, manager: PackageManager, env: &HostEnv) -> Result<InstalledSet>;
}

/// Reader of the dpkg status file.
///
/// It answers for `apt` and `dpkg` only.
#[derive(Debug, Clone, Copy, Default)]
pub struct DpkgStatusSource;

impl InstalledSource for DpkgStatusSource {
    fn installed(&self, manager: PackageManager, env: &HostEnv) -> Result<InstalledSet> {
        let mut set = InstalledSet::default();
        if !matches!(manager, PackageManager::Apt | PackageManager::Dpkg) {
            set.warnings
                .push(format!("installed packages of {manager} are not readable"));
            return Ok(set);
        }
        let path = env.root().resolve(DPKG_STATUS);
        let file = File::open(&path).map_err(|source| {
            Error::new(
                ErrorCode::NotFound,
                format!("cannot open the dpkg status file: {source}"),
            )
        })?;
        for_each_stanza(BufReader::new(file), |stanza| {
            let installed = stanza
                .get("Status")
                .and_then(|status| status.split_whitespace().nth(2))
                == Some("installed");
            let Some(name) = stanza.get("Package") else {
                return;
            };
            if !installed {
                return;
            }
            set.packages.push(InstalledPackage {
                name: name.to_owned(),
                version: stanza.get("Version").map(str::to_owned),
                architecture: stanza.get("Architecture").map(str::to_owned),
                summary: stanza.get("Description").map(str::to_owned),
                origin: None,
                installed_size: stanza
                    .get("Installed-Size")
                    .and_then(|size| size.parse::<u64>().ok())
                    .map(|kibibytes| kibibytes.saturating_mul(KIBIBYTE)),
            });
        })
        .map_err(|source| {
            Error::new(
                ErrorCode::Format,
                format!("cannot read the dpkg status file: {source}"),
            )
        })?;
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::root::SysRoot;
    use nurt_contracts::env::Os;
    use std::fs;

    fn env_with_status(content: &str) -> (tempfile::TempDir, HostEnv) {
        let dir = tempfile::tempdir().expect("temporary directory");
        let status = dir.path().join("var/lib/dpkg");
        fs::create_dir_all(&status).expect("directories");
        fs::write(status.join("status"), content).expect("status file");
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        (dir, env)
    }

    #[test]
    fn dpkg_status_lists_only_installed_packages() {
        let (_dir, env) = env_with_status(
            "Package: curl\nStatus: install ok installed\nVersion: 8.5.0-2\nArchitecture: amd64\nInstalled-Size: 500\nDescription: command line tool\n\n\
             Package: old\nStatus: deinstall ok config-files\nVersion: 1\n\n",
        );
        let set = DpkgStatusSource
            .installed(PackageManager::Apt, &env)
            .expect("installed packages");
        assert_eq!(set.packages.len(), 1);
        assert_eq!(set.packages[0].name, "curl");
        assert_eq!(set.packages[0].installed_size, Some(512_000));
        assert_eq!(set.packages[0].summary.as_deref(), Some("command line tool"));
    }

    #[test]
    fn dpkg_status_source_warns_for_other_managers() {
        let (_dir, env) = env_with_status("");
        let set = DpkgStatusSource
            .installed(PackageManager::Pacman, &env)
            .expect("empty set");
        assert!(set.packages.is_empty());
        assert_eq!(set.warnings.len(), 1);
    }
}
