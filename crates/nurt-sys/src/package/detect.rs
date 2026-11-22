// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Detection of the package managers of a system.

use nurt_contracts::env::Os;
use nurt_contracts::record::PackageManager;

use super::root::HostEnv;

const DPKG_STATUS: &str = "/var/lib/dpkg/status";
const ZYPP_CONFIG: &str = "/etc/zypp";
const RPM_DATABASES: [&str; 3] = [
    "/var/lib/rpm/rpmdb.sqlite",
    "/usr/lib/sysimage/rpm/rpmdb.sqlite",
    "/var/lib/rpm/Packages",
];
const PACMAN_LOCAL: &str = "/var/lib/pacman/local";
const LINUXBREW_PREFIX: &str = "/home/linuxbrew/.linuxbrew";
const MACOS_BREW_PREFIXES: [&str; 2] = ["/opt/homebrew", "/usr/local"];
const WINGET_PACKAGE: &str = "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe";
const CHOCOLATEY_DEFAULT: &str = "C:/ProgramData/chocolatey";

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
    let root = env.root();
    match manager {
        PackageManager::Apt | PackageManager::Dpkg => root.exists(DPKG_STATUS),
        PackageManager::Zypper => root.is_dir(ZYPP_CONFIG) && has_rpm_database(env),
        PackageManager::Dnf | PackageManager::Rpm => {
            !root.is_dir(ZYPP_CONFIG) && has_rpm_database(env)
        }
        PackageManager::Pacman => root.is_dir(PACMAN_LOCAL),
        PackageManager::Brew => brew_prefix(env).is_some(),
        PackageManager::Winget => env
            .var_nonempty("LOCALAPPDATA")
            .is_some_and(|base| root.is_dir(&format!("{base}/Packages/{WINGET_PACKAGE}"))),
        PackageManager::Scoop => scoop_root(env).is_some(),
        PackageManager::Choco => chocolatey_root(env).is_some(),
        _ => false,
    }
}

fn has_rpm_database(env: &HostEnv) -> bool {
    RPM_DATABASES.iter().any(|path| env.root().exists(path))
}

/// Homebrew prefix of the system, when Homebrew is installed.
#[must_use]
pub(crate) fn brew_prefix(env: &HostEnv) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(prefix) = env.var_nonempty("HOMEBREW_PREFIX") {
        candidates.push(prefix.to_owned());
    }
    match env.os() {
        Os::Linux => candidates.push(LINUXBREW_PREFIX.to_owned()),
        Os::Macos => candidates.extend(MACOS_BREW_PREFIXES.iter().map(|p| (*p).to_owned())),
        _ => {}
    }
    candidates.into_iter().find(|prefix| {
        env.root().exists(&format!("{prefix}/bin/brew"))
            || env.root().is_dir(&format!("{prefix}/Cellar"))
    })
}

/// Root directory of Scoop, when it exists.
#[must_use]
pub(crate) fn scoop_root(env: &HostEnv) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(scoop) = env.var_nonempty("SCOOP") {
        candidates.push(scoop.to_owned());
    }
    if let Some(home) = env.home() {
        candidates.push(format!("{home}/scoop"));
    }
    candidates
        .into_iter()
        .find(|dir| env.root().is_dir(dir))
}

/// Root directory of Chocolatey, when it exists.
#[must_use]
pub(crate) fn chocolatey_root(env: &HostEnv) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(dir) = env.var_nonempty("ChocolateyInstall") {
        candidates.push(dir.to_owned());
    }
    candidates.push(CHOCOLATEY_DEFAULT.to_owned());
    candidates
        .into_iter()
        .find(|dir| env.root().is_dir(dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::root::SysRoot;
    use std::fs;

    fn tree(paths: &[&str]) -> (tempfile::TempDir, SysRoot) {
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
        let root = SysRoot::at(dir.path());
        (dir, root)
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
        for (paths, expected) in cases {
            let (_dir, root) = tree(paths);
            let env = HostEnv::new(root, Os::Linux);
            assert_eq!(detect_managers(&env), [expected]);
        }
        let (_dir, root) = tree(&[]);
        assert!(detect_managers(&HostEnv::new(root, Os::Linux)).is_empty());
    }

    #[test]
    fn detect_multiple_managers_reports_each() {
        let (_dir, root) = tree(&[
            "var/lib/dpkg/status",
            "home/linuxbrew/.linuxbrew/bin/brew",
        ]);
        let env = HostEnv::new(root, Os::Linux);
        assert_eq!(
            detect_managers(&env),
            [PackageManager::Apt, PackageManager::Brew]
        );
    }

    #[test]
    fn detect_macos_and_windows_managers_from_variables() {
        let (_dir, root) = tree(&["opt/homebrew/Cellar/"]);
        let mac = HostEnv::new(root, Os::Macos);
        assert_eq!(detect_managers(&mac), [PackageManager::Brew]);

        let (_dir, root) = tree(&[
            "Users/a/AppData/Local/Packages/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe/",
            "Users/a/scoop/",
            "ProgramData/chocolatey/",
        ]);
        let windows = HostEnv::new(root, Os::Windows)
            .with_var("LOCALAPPDATA", "C:\\Users\\a\\AppData\\Local")
            .with_var("USERPROFILE", "C:\\Users\\a");
        assert_eq!(
            detect_managers(&windows),
            [
                PackageManager::Winget,
                PackageManager::Scoop,
                PackageManager::Choco
            ]
        );
    }
}
