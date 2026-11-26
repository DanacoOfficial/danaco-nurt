// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! System inspection and administration for Danaco Nurt.
//!
//! The crate reports processes, ports, disks, host information, environment,
//! accounts and open files, and manages services, packages, the Windows
//! registry and CIM classes on Linux, macOS and Windows. Unsafe code is limited
//! to operating system calls and every block documents its safety conditions.

#![deny(missing_docs)]
#![allow(unsafe_code, reason = "calls of the operating system")]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod accounts;
pub mod cim;
mod common;
pub mod console;
pub mod disks;
pub mod environment;
pub mod host;
pub mod open_files;
pub mod package;
pub mod ports;
pub mod processes;
pub mod registry;
pub mod services;
pub mod sleep;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{StageImpl, StageRegistrar};

/// Adds the stages of the system, administration and package commands to
/// `registrar`, in the order of the catalog.
///
/// The stages read the running system; the package stages read the installed
/// packages from the databases of the package managers.
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
/// nurt_sys::register(&mut stages)?;
/// assert_eq!(stages.len(), 19);
/// assert_eq!(stages[0].descriptor().command, "processes");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    let system: [Arc<dyn StageImpl>; 8] = [
        Arc::new(processes::Processes::new()),
        Arc::new(ports::Ports::new()),
        Arc::new(disks::Disks::new()),
        Arc::new(host::HostStage::new()),
        Arc::new(environment::EnvironmentStage::new()),
        Arc::new(open_files::OpenFilesStage::new()),
        Arc::new(accounts::AccountsStage::new()),
        Arc::new(sleep::Sleep::new()),
    ];
    let packages = Arc::new(package::PackageEnv::system());
    system
        .into_iter()
        .chain(services::stages())
        .chain([Arc::new(cim::WindowsCim::new()) as Arc<dyn StageImpl>])
        .chain(package::stages(&packages))
        .try_for_each(|stage| registrar.register(stage))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use nurt_contracts::stage::StageImpl;

    /// Catalog files whose commands the crate implements.
    const CATALOG_FILES: [&str; 3] = ["system.toml", "admin.toml", "package.toml"];
    /// Commands of [`CATALOG_FILES`] implemented by other crates.
    const FOREIGN_COMMANDS: [&str; 3] = ["logs", "suspend", "resume"];

    /// Names of the `[[command]]` tables of [`CATALOG_FILES`] without
    /// [`FOREIGN_COMMANDS`].
    fn catalog_commands() -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for file in CATALOG_FILES {
            let path = format!(
                "{}/../../catalog/{file}",
                std::env::var("CARGO_MANIFEST_DIR")
                    .as_deref()
                    .unwrap_or(env!("CARGO_MANIFEST_DIR"))
            );
            let table: toml::Table = std::fs::read_to_string(path).unwrap().parse().unwrap();
            for command in table["command"].as_array().unwrap() {
                let name = command["name"].as_str().unwrap();
                if !FOREIGN_COMMANDS.contains(&name) {
                    names.insert(name.to_owned());
                }
            }
        }
        names
    }

    #[test]
    fn registers_one_stage_per_catalog_command() {
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        super::register(&mut stages).unwrap();
        let names: Vec<String> = stages
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        let unique: BTreeSet<String> = names.iter().cloned().collect();
        assert_eq!(unique.len(), names.len(), "a command is registered twice");
        assert_eq!(unique, catalog_commands());
    }
}
