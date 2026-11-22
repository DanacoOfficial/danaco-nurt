// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Installed programs and external program catalogs for Danaco Nurt.
//!
//! The crate implements the `program` command group. Providers read the
//! search path, package databases (dpkg, rpm, pacman, Homebrew, winget,
//! Scoop, Chocolatey), registered applications, manual pages and external
//! catalogs in the format `nurt-program-catalog/1`. The index merges their
//! entries into program cards, keeps them in a cache and ranks them for
//! `program.search`. Providers only read files and registry keys; the only
//! program ever started is the one whose `--help` is requested explicitly.

#![deny(missing_docs)]
#![deny(unsafe_code)]

pub mod catalog;
pub mod check;
pub mod help;
pub mod index;
pub mod model;
pub mod search;
pub mod source;
pub mod sources;
pub mod stages;
pub mod taxonomy;
pub mod text;
#[cfg(windows)]
pub mod win;

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageRegistrar;

/// Adds the four `program.*` stages with the default setup to `registrar`.
///
/// # Errors
///
/// Fails with the first error of the registrar, such as `E_UNKNOWN_COMMAND`
/// when a command is not in the catalog of the registry.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_programs::register(&mut stages)?;
/// assert_eq!(stages.len(), 4);
/// assert_eq!(stages[0].descriptor().command, "program.find");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    stages::stages()
        .into_iter()
        .try_for_each(|stage| registrar.register(stage))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::stage::StageImpl;

    /// Sorted command names of the catalog file `catalog/<file>`.
    fn catalog_commands(file: &str) -> Vec<String> {
        let path = format!("{}/../../catalog/{file}", env!("CARGO_MANIFEST_DIR"));
        let catalog: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
        let mut names: Vec<String> = catalog["command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|command| command["name"].as_str().unwrap().to_owned())
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn registers_one_stage_per_catalog_command() {
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        super::register(&mut stages).unwrap();
        let mut names: Vec<String> = stages
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        names.sort_unstable();
        assert_eq!(names, catalog_commands("program.toml"));
    }
}
