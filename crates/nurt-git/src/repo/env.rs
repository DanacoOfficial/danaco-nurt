// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Environment variables that influence repository access, read from the
//! execution context instead of the process environment.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use nurt_contracts::env::{Host, Os};

/// Snapshot of the environment variables that `git` consults when it locates
/// configuration files and applies configuration from the environment.
///
/// Values come from [`Host::env_var`], so an embedding application or a test
/// controls them without touching the process environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitEnv {
    os: Option<Os>,
    home: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    config_global: Option<PathBuf>,
    config_system: Option<PathBuf>,
    config_nosystem: bool,
    config_pairs: Vec<(String, String)>,
    ceiling_dirs: Vec<PathBuf>,
}

impl GitEnv {
    /// Reads the variables from the host of an execution context.
    ///
    /// `HOME` falls back to `USERPROFILE` on Windows and then to the home
    /// directory reported by the host, as `git` does.
    #[must_use]
    pub fn from_host(host: &dyn Host) -> Self {
        let info = host.info();
        let var = |name: &str| host.env_var(name).filter(|value| !value.is_empty());
        let home = var("HOME")
            .or_else(|| (info.os == Os::Windows).then(|| var("USERPROFILE")).flatten())
            .map(PathBuf::from)
            .or_else(|| (!info.home.as_os_str().is_empty()).then(|| info.home.clone()));
        Self {
            os: Some(info.os),
            home,
            xdg_config_home: var("XDG_CONFIG_HOME").map(PathBuf::from),
            config_global: host.env_var("GIT_CONFIG_GLOBAL").map(PathBuf::from),
            config_system: var("GIT_CONFIG_SYSTEM").map(PathBuf::from),
            config_nosystem: var("GIT_CONFIG_NOSYSTEM").is_some_and(|value| is_true(&value)),
            config_pairs: config_pairs(host),
            ceiling_dirs: var("GIT_CEILING_DIRECTORIES")
                .map(|value| split_path_list(&value, info.os))
                .unwrap_or_default(),
        }
    }

    /// Home directory used for `~` in configuration values and for the
    /// global configuration file.
    #[must_use]
    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Directory of the XDG configuration: `$XDG_CONFIG_HOME/git` or
    /// `$HOME/.config/git`.
    #[must_use]
    pub fn xdg_git_dir(&self) -> Option<PathBuf> {
        match (&self.xdg_config_home, &self.home) {
            (Some(xdg), _) => Some(xdg.join("git")),
            (None, Some(home)) => Some(home.join(".config").join("git")),
            (None, None) => None,
        }
    }

    /// Global configuration files in the order `git` reads them: the file
    /// named by `GIT_CONFIG_GLOBAL` alone, or the XDG file followed by
    /// `~/.gitconfig`.
    #[must_use]
    pub fn global_config_files(&self) -> Vec<PathBuf> {
        if let Some(file) = &self.config_global {
            return if file.as_os_str().is_empty() {
                Vec::new()
            } else {
                vec![file.clone()]
            };
        }
        let mut files = Vec::new();
        if let Some(dir) = self.xdg_git_dir() {
            files.push(dir.join("config"));
        }
        if let Some(home) = &self.home {
            files.push(home.join(".gitconfig"));
        }
        files
    }

    /// System configuration file, unless `GIT_CONFIG_NOSYSTEM` disables it:
    /// the file named by `GIT_CONFIG_SYSTEM` or the first existing file among
    /// the locations used by `git` distributions of the operating system.
    #[must_use]
    pub fn system_config_file(&self) -> Option<PathBuf> {
        if self.config_nosystem {
            return None;
        }
        if let Some(file) = &self.config_system {
            return Some(file.clone());
        }
        system_config_candidates(self.os)
            .iter()
            .map(PathBuf::from)
            .find(|candidate| candidate.is_file())
    }

    /// Pairs from `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>` and
    /// `GIT_CONFIG_VALUE_<n>`, in order.
    #[must_use]
    pub fn config_pairs(&self) -> &[(String, String)] {
        &self.config_pairs
    }

    /// Directories from `GIT_CEILING_DIRECTORIES` at which discovery stops.
    #[must_use]
    pub fn ceiling_dirs(&self) -> &[PathBuf] {
        &self.ceiling_dirs
    }

    /// Operating system of the host, when known.
    #[must_use]
    pub fn os(&self) -> Option<Os> {
        self.os
    }

    /// Expands a leading `~/` or a bare `~` with the home directory.
    #[must_use]
    pub fn expand_home(&self, value: &str) -> Option<PathBuf> {
        let home = self.home.as_ref()?;
        if value == "~" {
            return Some(home.clone());
        }
        value
            .strip_prefix("~/")
            .map(|rest| home.join(rest))
    }
}

fn system_config_candidates(os: Option<Os>) -> &'static [&'static str] {
    match os {
        Some(Os::Windows) => &[
            "C:/Program Files/Git/etc/gitconfig",
            "C:/Program Files/Git/mingw64/etc/gitconfig",
        ],
        Some(Os::Macos) => &[
            "/opt/homebrew/etc/gitconfig",
            "/usr/local/etc/gitconfig",
            "/etc/gitconfig",
        ],
        _ => &["/etc/gitconfig"],
    }
}

fn is_true(value: &OsString) -> bool {
    let text = value.to_string_lossy().to_ascii_lowercase();
    matches!(text.as_str(), "1" | "true" | "yes" | "on")
}

fn config_pairs(host: &dyn Host) -> Vec<(String, String)> {
    let Some(count) = host
        .env_var("GIT_CONFIG_COUNT")
        .and_then(|value| value.to_string_lossy().trim().parse::<usize>().ok())
    else {
        return Vec::new();
    };
    (0..count)
        .filter_map(|n| {
            let key = host.env_var(&format!("GIT_CONFIG_KEY_{n}"))?;
            let value = host.env_var(&format!("GIT_CONFIG_VALUE_{n}")).unwrap_or_default();
            Some((
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            ))
        })
        .collect()
}

fn split_path_list(value: &OsString, os: Os) -> Vec<PathBuf> {
    let separator = if os == Os::Windows { ';' } else { ':' };
    value
        .to_string_lossy()
        .split(separator)
        .filter(|item| !item.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TestHost;

    #[test]
    fn global_files_follow_xdg_then_home() {
        let host = TestHost::new("/home/ada")
            .with_var("XDG_CONFIG_HOME", "/xdg");
        let env = GitEnv::from_host(&host);
        assert_eq!(
            env.global_config_files(),
            vec![PathBuf::from("/xdg/git/config"), PathBuf::from("/home/ada/.gitconfig")]
        );
    }

    #[test]
    fn git_config_global_replaces_default_files() {
        let host = TestHost::new("/home/ada").with_var("GIT_CONFIG_GLOBAL", "/etc/custom");
        let env = GitEnv::from_host(&host);
        assert_eq!(env.global_config_files(), vec![PathBuf::from("/etc/custom")]);
    }

    #[test]
    fn empty_git_config_global_disables_global_configuration() {
        let host = TestHost::new("/home/ada").with_var("GIT_CONFIG_GLOBAL", "");
        assert!(GitEnv::from_host(&host).global_config_files().is_empty());
    }

    #[test]
    fn nosystem_disables_system_configuration() {
        let host = TestHost::new("/home/ada")
            .with_var("GIT_CONFIG_NOSYSTEM", "1")
            .with_var("GIT_CONFIG_SYSTEM", "/etc/other");
        assert_eq!(GitEnv::from_host(&host).system_config_file(), None);
    }

    #[test]
    fn git_config_system_names_the_system_file() {
        let host = TestHost::new("/home/ada").with_var("GIT_CONFIG_SYSTEM", "/opt/gitconfig");
        assert_eq!(
            GitEnv::from_host(&host).system_config_file(),
            Some(PathBuf::from("/opt/gitconfig"))
        );
    }

    #[test]
    fn config_pairs_are_read_in_order_up_to_the_count() {
        let host = TestHost::new("/home/ada")
            .with_var("GIT_CONFIG_COUNT", "2")
            .with_var("GIT_CONFIG_KEY_0", "core.abbrev")
            .with_var("GIT_CONFIG_VALUE_0", "12")
            .with_var("GIT_CONFIG_KEY_1", "user.name")
            .with_var("GIT_CONFIG_VALUE_1", "Ada")
            .with_var("GIT_CONFIG_KEY_2", "ignored.key");
        let env = GitEnv::from_host(&host);
        assert_eq!(
            env.config_pairs(),
            &[
                ("core.abbrev".to_owned(), "12".to_owned()),
                ("user.name".to_owned(), "Ada".to_owned())
            ]
        );
    }

    #[test]
    fn home_expansion_handles_tilde_forms() {
        let env = GitEnv::from_host(&TestHost::new("/home/ada"));
        assert_eq!(env.expand_home("~"), Some(PathBuf::from("/home/ada")));
        assert_eq!(env.expand_home("~/x/y"), Some(PathBuf::from("/home/ada/x/y")));
        assert_eq!(env.expand_home("/abs"), None);
    }

    #[test]
    fn ceiling_directories_are_split_by_the_platform_separator() {
        let host = TestHost::new("/home/ada").with_var("GIT_CEILING_DIRECTORIES", "/a::/b");
        assert_eq!(
            GitEnv::from_host(&host).ceiling_dirs(),
            &[PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }
}
