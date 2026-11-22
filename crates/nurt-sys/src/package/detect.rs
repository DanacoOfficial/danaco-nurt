// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the package managers of a system.
//!
//! The markers of Homebrew, Scoop and Chocolatey and the locations of the
//! RPM and pacman databases are those of the database readers of the program
//! catalog crate, so a detected manager is one whose database can be read.

use nurt_contracts::env::Os;
use nurt_contracts::record::PackageManager;
use nurt_programs::source::ProgramSource;
use nurt_programs::sources::brew::BrewSource;
use nurt_programs::sources::choco::ChocoSource;
use nurt_programs::sources::scoop::ScoopSource;
use nurt_programs::sources::{pacman, rpm};

use super::root::{HostEnv, SysPaths};

const DPKG_STATUS: &str = "/var/lib/dpkg/status";
const ZYPP_CONFIG: &str = "/etc/zypp";
const PACMAN_LOCAL_DIR: &str = "local";
/// Package directory of App Installer, which provides winget, below
/// `LOCALAPPDATA`.
pub(crate) const WINGET_PACKAGE_DIR: &str = "Packages\\Microsoft.DesktopAppInstaller_8wekyb3d8bbwe";

/// Managers whose markers are present, in the fixed order of preference.
///
/// Linux: apt, zypper, dnf, pacman, Homebrew. macOS: Homebrew. Windows:
/// winget, Scoop, Chocolatey.
#[must_use]
pub(crate) fn detect_managers(env: &HostEnv) -> Vec<PackageManager> {
    let candidates: &[PackageManager] = match env.os() {
        Os::Linux => &[
            PackageManager::Apt,
            PackageManager::Zypper,
            PackageManager::Dnf,
            PackageManager::Pacman,
            PackageManager::Brew,
        ],
        Os::Macos => &[PackageManager::Brew],
        Os::Windows => &[
            PackageManager::Winget,
            PackageManager::Scoop,
            PackageManager::Choco,
        ],
        _ => &[],
    };
    candidates
        .iter()
        .copied()
        .filter(|manager| is_present(env, *manager))
        .collect()
}

/// Returns `true` when the markers of `manager` are present.
#[must_use]
pub(crate) fn is_present(env: &HostEnv, manager: PackageManager) -> bool {
    match manager {
        PackageManager::Apt | PackageManager::Dpkg => env.exists(DPKG_STATUS),
        PackageManager::Zypper => env.is_dir(ZYPP_CONFIG) && rpm::locate(env).is_some(),
        PackageManager::Dnf | PackageManager::Rpm => {
            !env.is_dir(ZYPP_CONFIG) && rpm::locate(env).is_some()
        }
        PackageManager::Pacman => env
            .sys_path(pacman::db_path(env).join(PACMAN_LOCAL_DIR))
            .is_dir(),
        PackageManager::Brew => BrewSource::new().detect(env),
        PackageManager::Winget => {
            env.os() == Os::Windows
                && env
                    .var_text("LOCALAPPDATA")
                    .is_some_and(|base| env.is_dir(&format!("{base}\\{WINGET_PACKAGE_DIR}")))
        }
        PackageManager::Scoop => ScoopSource::new().detect(env),
        PackageManager::Choco => ChocoSource::new().detect(env),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Profile directory of a synthetic Windows account.
    const PROFILE: &str = "C:\\Users\\user";
    /// The same profile below a test root.
    const PROFILE_DIR: &str = "c/Users/user";

    fn tree(paths: &[String]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temporary directory");
        for path in paths {
            let target = dir.path().join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&target).expect("directory");
            } else {
                fs::create_dir_all(target.parent().expect("parent")).expect("directory");
                fs::write(&target, "").expect("file");
            }
        }
        dir
    }

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn detect_manager_from_sysroot_markers() {
        let cases: [(&[&str], PackageManager); 4] = [
            (&["var/lib/dpkg/status"], PackageManager::Apt),
            (
                &["etc/zypp/", "var/lib/rpm/rpmdb.sqlite"],
                PackageManager::Zypper,
            ),
            (&["usr/lib/sysimage/rpm/rpmdb.sqlite"], PackageManager::Dnf),
            (&["var/lib/pacman/local/"], PackageManager::Pacman),
        ];
        for (markers, expected) in cases {
            let dir = tree(&paths(markers));
            let env = HostEnv::new(Os::Linux).with_root(dir.path());
            assert_eq!(detect_managers(&env), [expected], "{markers:?}");
        }
        let dir = tree(&[]);
        assert!(detect_managers(&HostEnv::new(Os::Linux).with_root(dir.path())).is_empty());
    }

    #[test]
    fn detect_multiple_managers_reports_each() {
        let dir = tree(&paths(&["var/lib/dpkg/status", "srv/homebrew/Cellar/"]));
        let env = HostEnv::new(Os::Linux)
            .with_root(dir.path())
            .with_var("HOMEBREW_PREFIX", "/srv/homebrew");
        assert_eq!(
            detect_managers(&env),
            [PackageManager::Apt, PackageManager::Brew]
        );
    }

    #[test]
    fn detect_macos_and_windows_managers() {
        let dir = tree(&paths(&["opt/homebrew/Cellar/"]));
        let mac = HostEnv::new(Os::Macos).with_root(dir.path());
        assert_eq!(detect_managers(&mac), [PackageManager::Brew]);

        let dir = tree(&[
            format!(
                "{PROFILE_DIR}/AppData/Local/Packages/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe/"
            ),
            format!("{PROFILE_DIR}/scoop/apps/"),
            "c/ProgramData/chocolatey/lib/".to_owned(),
        ]);
        let windows = HostEnv::new(Os::Windows)
            .with_root(dir.path())
            .with_var("LOCALAPPDATA", format!("{PROFILE}\\AppData\\Local"))
            .with_home(PROFILE);
        assert_eq!(
            detect_managers(&windows),
            [
                PackageManager::Winget,
                PackageManager::Scoop,
                PackageManager::Choco
            ]
        );
        assert!(!is_present(&windows, PackageManager::Apt));
    }
}
