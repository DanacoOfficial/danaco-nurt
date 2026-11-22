// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Directory of installed extensions.
//!
//! The directory is the configuration key `ext.dir` (environment variable
//! `NURT_EXT_DIR`, applied by the configuration loader). Without the key the
//! directory is `ext` in the configuration directory. A relative `ext.dir`
//! is resolved against the configuration directory. Each extension lives in
//! a subdirectory named after the extension.

use std::path::{Path, PathBuf};

use nurt_contracts::config::ExtConfig;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::spec::is_identifier;

use crate::error::error;

/// Name of the extension directory inside the configuration directory, used
/// when `ext.dir` is not set.
pub const DEFAULT_DIR_NAME: &str = "ext";

/// Where extensions are installed and whether native libraries may load.
///
/// ```
/// use std::path::Path;
/// use nurt_contracts::config::ExtConfig;
/// use nurt_ext::locations::ExtLocations;
///
/// let root = if cfg!(windows) { Path::new(r"C:\cfg") } else { Path::new("/cfg") };
/// let locations = ExtLocations::from_config(&ExtConfig::default(), root)?;
/// assert_eq!(locations.dir(), root.join("ext"));
/// assert!(!locations.native_allowed());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtLocations {
    dir: PathBuf,
    native: bool,
}

impl ExtLocations {
    /// Locations with the extension directory `dir`; native libraries are
    /// not allowed.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            native: false,
        }
    }

    /// Locations from the section `[ext]` of the effective configuration.
    ///
    /// # Errors
    ///
    /// Fails with `E_CONFIG` when `config_dir` is not absolute or `ext.dir`
    /// is empty.
    pub fn from_config(config: &ExtConfig, config_dir: &Path) -> Result<Self> {
        if !config_dir.is_absolute() {
            return Err(error(
                ErrorCode::Config,
                format!(
                    "the configuration directory {} must be absolute",
                    config_dir.display()
                ),
            ));
        }
        let dir = match config.dir.as_deref() {
            None => config_dir.join(DEFAULT_DIR_NAME),
            Some(text) if text.trim().is_empty() => {
                return Err(error(
                    ErrorCode::Config,
                    "`ext.dir` is empty; set a directory or remove the key",
                ));
            }
            Some(text) => config_dir.join(text),
        };
        Ok(Self {
            dir,
            native: config.native,
        })
    }

    /// Allows or forbids native extension libraries (`ext.native`).
    #[must_use]
    pub fn with_native(mut self, allowed: bool) -> Self {
        self.native = allowed;
        self
    }

    /// Directory of installed extensions.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns `true` when native extension libraries may be loaded.
    #[must_use]
    pub fn native_allowed(&self) -> bool {
        self.native
    }

    /// Directory of the extension `name`.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` when `name` is not an extension name, so
    /// that no path outside the extension directory can be formed.
    pub fn extension_dir(&self, name: &str) -> Result<PathBuf> {
        if is_identifier(name) {
            Ok(self.dir.join(name))
        } else {
            Err(error(
                ErrorCode::InvalidValue,
                format!("`{name}` is not an extension name; use lowercase letters, digits and `_`"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nurt_contracts::config::ExtConfig;
    use nurt_contracts::error::ErrorCode;

    use super::ExtLocations;

    fn root() -> PathBuf {
        std::env::temp_dir().join("nurt-config")
    }

    #[test]
    fn configured_directory_is_resolved_against_the_configuration_directory() {
        let mut config = ExtConfig::default();
        config.dir = Some("plugins".to_owned());
        config.native = true;
        let locations = ExtLocations::from_config(&config, &root()).unwrap();
        assert_eq!(locations.dir(), root().join("plugins"));
        assert!(locations.native_allowed());

        let absolute = root().join("elsewhere");
        config.dir = Some(absolute.display().to_string());
        let locations = ExtLocations::from_config(&config, &root()).unwrap();
        assert_eq!(locations.dir(), absolute);
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let mut config = ExtConfig::default();
        let relative = ExtLocations::from_config(&config, std::path::Path::new("cfg"));
        assert_eq!(relative.unwrap_err().code(), ErrorCode::Config);
        config.dir = Some("  ".to_owned());
        let empty = ExtLocations::from_config(&config, &root());
        assert_eq!(empty.unwrap_err().code(), ErrorCode::Config);
    }

    #[test]
    fn extension_directories_stay_inside_the_location() {
        let locations = ExtLocations::new(root()).with_native(false);
        assert_eq!(
            locations.extension_dir("acme").unwrap(),
            root().join("acme")
        );
        for name in ["../up", "a/b", "", "Acme", "a-b"] {
            let error = locations.extension_dir(name).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{name}");
        }
    }
}
