// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Locations of the configuration files and of the state directory.
//!
//! | System | System file | User file | State directory |
//! |---|---|---|---|
//! | Linux | `/etc/nurt/nurt.toml` | `$XDG_CONFIG_HOME/nurt/nurt.toml` (default `~/.config`) | `$XDG_STATE_HOME/nurt` (default `~/.local/state`) |
//! | macOS | `/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt` |
//! | Windows | `%PROGRAMDATA%\Nurt\nurt.toml` | `%APPDATA%\Nurt\nurt.toml` | `%LOCALAPPDATA%\Nurt` |
//!
//! The locations are computed from the facts and the environment reported by
//! [`Host`], never from the process environment, so that an embedding
//! application and tests control them.

use std::path::PathBuf;

use nurt_contracts::config::CONFIG_FILE_NAME;
use nurt_contracts::env::{Host, Os};

const UNIX_DIR: &str = "nurt";
const PRODUCT_DIR: &str = "Nurt";
const LINUX_SYSTEM_DIR: &str = "/etc/nurt";
const MACOS_SYSTEM_DIR: &str = "/Library/Application Support";
const MACOS_SUPPORT_DIR: &str = "Library/Application Support";
const XDG_CONFIG_DEFAULT: &str = ".config";
const XDG_STATE_DEFAULT: &str = ".local/state";
const WINDOWS_PROGRAM_DATA: &str = "C:\\ProgramData";
const WINDOWS_ROAMING: &str = "AppData\\Roaming";
const WINDOWS_LOCAL: &str = "AppData\\Local";

/// Locations of the configuration files and of the default state directory
/// on one host.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
/// use nurt_core::paths::PlatformPaths;
/// use nurt_testkit::host::TestHost;
///
/// let paths = PlatformPaths::for_host(&TestHost::linux());
/// assert_eq!(paths.system_file, Some(PathBuf::from("/etc/nurt/nurt.toml")));
/// assert_eq!(paths.user_file, Some(PathBuf::from("/home/user/.config/nurt/nurt.toml")));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlatformPaths {
    /// System configuration file.
    pub system_file: Option<PathBuf>,
    /// User configuration file; absent when the account has no home
    /// directory.
    pub user_file: Option<PathBuf>,
    /// Default state directory; absent when the account has no home
    /// directory.
    pub state_dir: Option<PathBuf>,
}

impl PlatformPaths {
    /// Locations for `host`, from its operating system, home directory and
    /// environment variables.
    #[must_use]
    pub fn for_host(host: &dyn Host) -> Self {
        let info = host.info();
        let home = Some(info.home.to_string_lossy().into_owned()).filter(|home| !home.is_empty());
        let var = |name: &str| {
            host.env_var(name)
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.is_empty())
        };
        match info.os {
            Os::Windows => windows(home.as_deref(), &var),
            Os::Macos => macos(home.as_deref()),
            _ => unix(home.as_deref(), &var),
        }
    }
}

fn unix(home: Option<&str>, var: &dyn Fn(&str) -> Option<String>) -> PlatformPaths {
    let xdg = |name: &str, fallback: &str| {
        var(name)
            .filter(|dir| dir.starts_with('/'))
            .or_else(|| home.map(|home| join(home, &[fallback], '/')))
    };
    PlatformPaths {
        system_file: Some(join(LINUX_SYSTEM_DIR, &[CONFIG_FILE_NAME], '/').into()),
        user_file: xdg("XDG_CONFIG_HOME", XDG_CONFIG_DEFAULT)
            .map(|dir| join(&dir, &[UNIX_DIR, CONFIG_FILE_NAME], '/').into()),
        state_dir: xdg("XDG_STATE_HOME", XDG_STATE_DEFAULT)
            .map(|dir| join(&dir, &[UNIX_DIR], '/').into()),
    }
}

fn macos(home: Option<&str>) -> PlatformPaths {
    let support = home.map(|home| join(home, &[MACOS_SUPPORT_DIR, PRODUCT_DIR], '/'));
    PlatformPaths {
        system_file: Some(join(MACOS_SYSTEM_DIR, &[PRODUCT_DIR, CONFIG_FILE_NAME], '/').into()),
        user_file: support
            .as_deref()
            .map(|dir| join(dir, &[CONFIG_FILE_NAME], '/').into()),
        state_dir: support.map(PathBuf::from),
    }
}

fn windows(home: Option<&str>, var: &dyn Fn(&str) -> Option<String>) -> PlatformPaths {
    let known = |name: &str, fallback: &str| {
        var(name).or_else(|| home.map(|home| join(home, &[fallback], '\\')))
    };
    let program_data = var("PROGRAMDATA").unwrap_or_else(|| WINDOWS_PROGRAM_DATA.to_owned());
    PlatformPaths {
        system_file: Some(join(&program_data, &[PRODUCT_DIR, CONFIG_FILE_NAME], '\\').into()),
        user_file: known("APPDATA", WINDOWS_ROAMING)
            .map(|dir| join(&dir, &[PRODUCT_DIR, CONFIG_FILE_NAME], '\\').into()),
        state_dir: known("LOCALAPPDATA", WINDOWS_LOCAL)
            .map(|dir| join(&dir, &[PRODUCT_DIR], '\\').into()),
    }
}

/// Joins path segments with the separator of the target system, so that the
/// result does not depend on the system the engine was built for.
fn join(base: &str, parts: &[&str], separator: char) -> String {
    let mut path = base.trim_end_matches(['/', '\\']).to_owned();
    for part in parts {
        path.push(separator);
        path.push_str(part);
    }
    path
}

#[cfg(test)]
mod tests {
    use nurt_testkit::host::TestHost;

    use super::*;

    fn text(path: Option<PathBuf>) -> String {
        path.map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    #[test]
    fn linux_uses_xdg_defaults_under_home() {
        let paths = PlatformPaths::for_host(&TestHost::linux());
        assert_eq!(text(paths.system_file), "/etc/nurt/nurt.toml");
        assert_eq!(text(paths.user_file), "/home/user/.config/nurt/nurt.toml");
        assert_eq!(text(paths.state_dir), "/home/user/.local/state/nurt");
    }

    #[test]
    fn linux_honours_absolute_xdg_variables() {
        let host = TestHost::linux()
            .with_var("XDG_CONFIG_HOME", "/cfg/")
            .with_var("XDG_STATE_HOME", "/state");
        let paths = PlatformPaths::for_host(&host);
        assert_eq!(text(paths.user_file), "/cfg/nurt/nurt.toml");
        assert_eq!(text(paths.state_dir), "/state/nurt");
    }

    #[test]
    fn linux_ignores_relative_xdg_variables() {
        let host = TestHost::linux().with_var("XDG_CONFIG_HOME", "relative/cfg");
        let paths = PlatformPaths::for_host(&host);
        assert_eq!(text(paths.user_file), "/home/user/.config/nurt/nurt.toml");
    }

    #[test]
    fn macos_uses_application_support() {
        let paths = PlatformPaths::for_host(&TestHost::macos());
        assert_eq!(
            text(paths.system_file),
            "/Library/Application Support/Nurt/nurt.toml"
        );
        assert!(text(paths.user_file).ends_with("/Library/Application Support/Nurt/nurt.toml"));
        assert!(text(paths.state_dir).ends_with("/Library/Application Support/Nurt"));
    }

    #[test]
    fn windows_uses_known_folder_variables() {
        let host = TestHost::windows()
            .with_var("PROGRAMDATA", "D:\\Data")
            .with_var("APPDATA", "D:\\Users\\user\\AppData\\Roaming")
            .with_var("LOCALAPPDATA", "D:\\Users\\user\\AppData\\Local");
        let paths = PlatformPaths::for_host(&host);
        assert_eq!(text(paths.system_file), "D:\\Data\\Nurt\\nurt.toml");
        assert_eq!(
            text(paths.user_file),
            "D:\\Users\\user\\AppData\\Roaming\\Nurt\\nurt.toml"
        );
        assert_eq!(
            text(paths.state_dir),
            "D:\\Users\\user\\AppData\\Local\\Nurt"
        );
    }

    #[test]
    fn windows_falls_back_to_profile_folders() {
        let paths = PlatformPaths::for_host(&TestHost::windows());
        assert_eq!(text(paths.system_file), "C:\\ProgramData\\Nurt\\nurt.toml");
        assert!(text(paths.user_file).ends_with("AppData\\Roaming\\Nurt\\nurt.toml"));
        assert!(text(paths.state_dir).ends_with("AppData\\Local\\Nurt"));
    }

    #[test]
    fn join_drops_trailing_separators() {
        assert_eq!(join("/home/user/", &["a", "b"], '/'), "/home/user/a/b");
        assert_eq!(join("C:\\Data\\", &["a"], '\\'), "C:\\Data\\a");
    }
}
