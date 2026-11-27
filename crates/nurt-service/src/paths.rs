// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::paths
// Responsibility:   Files of the service in the state directory.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Files of the service in the state directory.

use std::env;
use std::path::{Path, PathBuf};

use nurt_contracts::config::Config;

const LOCK_FILE: &str = "service.lock";
const HEARTBEAT_FILE: &str = "service.hb";
const SOCKET_FILE: &str = "service.sock";
const INSTALL_MANIFEST: &str = "service.install.json";
const STATE_DIR_NAME_POSIX: &str = "nurt";
const STATE_DIR_NAME_NATIVE: &str = "Nurt";

/// Paths of the service files under one state directory.
///
/// | File | Purpose |
/// |---|---|
/// | `service.lock` | exclusive lock held by the running service |
/// | `service.hb` | heartbeat, replaced atomically |
/// | `service.sock` | control socket (Unix) |
/// | `service.install.json` | installation manifest |
///
/// The schedule module keeps its own files under `schedules/`.
///
/// ```
/// use nurt_service::ServicePaths;
///
/// let paths = ServicePaths::new("/var/lib/nurt-state");
/// assert!(paths.heartbeat_file().ends_with("service.hb"));
/// assert!(paths.lock_file().ends_with("service.lock"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePaths {
    state_dir: PathBuf,
}

impl ServicePaths {
    /// Paths under the given state directory.
    #[must_use]
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    /// Paths under the state directory selected by `state.dir`, or under the
    /// state directory of the operating system when the key is not set.
    ///
    /// # Errors
    ///
    /// Fails when the key is not set and the home directory of the account
    /// cannot be determined.
    pub fn from_config(config: &Config) -> std::io::Result<Self> {
        match &config.state.dir {
            Some(dir) => Ok(Self::new(dir)),
            None => default_state_dir().map(Self::new),
        }
    }

    /// State directory.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Lock file of the running service.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.state_dir.join(LOCK_FILE)
    }

    /// Heartbeat file.
    #[must_use]
    pub fn heartbeat_file(&self) -> PathBuf {
        self.state_dir.join(HEARTBEAT_FILE)
    }

    /// Control socket (Unix).
    #[must_use]
    pub fn socket_file(&self) -> PathBuf {
        self.state_dir.join(SOCKET_FILE)
    }

    /// Installation manifest.
    #[must_use]
    pub fn install_manifest(&self) -> PathBuf {
        self.state_dir.join(INSTALL_MANIFEST)
    }
}

/// State directory of the operating system for the current account.
///
/// | System | Directory |
/// |---|---|
/// | Linux | `$XDG_STATE_HOME/nurt`, by default `~/.local/state/nurt` |
/// | macOS | `~/Library/Application Support/Nurt` |
/// | Windows | `%LOCALAPPDATA%\Nurt` |
///
/// # Errors
///
/// Fails when the home directory (or `LOCALAPPDATA` on Windows) is unknown.
pub fn default_state_dir() -> std::io::Result<PathBuf> {
    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join(STATE_DIR_NAME_NATIVE))
            .ok_or_else(|| missing("LOCALAPPDATA is not set"));
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home
            .map(|home| {
                home.join("Library")
                    .join("Application Support")
                    .join(STATE_DIR_NAME_NATIVE)
            })
            .ok_or_else(|| missing("HOME is not set"));
    }
    if let Some(dir) = env::var_os("XDG_STATE_HOME").filter(|dir| Path::new(dir).is_absolute()) {
        return Ok(PathBuf::from(dir).join(STATE_DIR_NAME_POSIX));
    }
    home.map(|home| home.join(".local").join("state").join(STATE_DIR_NAME_POSIX))
        .ok_or_else(|| missing("HOME is not set"))
}

fn missing(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_live_in_the_configured_state_directory() {
        let mut config = Config::default();
        config.state.dir = Some("/srv/nurt-state".to_owned());
        let paths = ServicePaths::from_config(&config).unwrap();
        assert_eq!(paths.state_dir(), Path::new("/srv/nurt-state"));
        assert_eq!(paths.lock_file(), Path::new("/srv/nurt-state/service.lock"));
        assert_eq!(
            paths.heartbeat_file(),
            Path::new("/srv/nurt-state/service.hb")
        );
        assert_eq!(
            paths.socket_file(),
            Path::new("/srv/nurt-state/service.sock")
        );
        assert_eq!(
            paths.install_manifest(),
            Path::new("/srv/nurt-state/service.install.json")
        );
        assert_eq!(
            missing("HOME is not set").kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn default_state_directory_follows_the_system_convention() {
        let Ok(dir) = default_state_dir() else {
            return;
        };
        let name = dir.file_name().and_then(|name| name.to_str());
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(name, Some(STATE_DIR_NAME_NATIVE));
        } else {
            assert_eq!(name, Some(STATE_DIR_NAME_POSIX));
        }
        assert_eq!(
            ServicePaths::from_config(&Config::default())
                .unwrap()
                .state_dir(),
            dir
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
