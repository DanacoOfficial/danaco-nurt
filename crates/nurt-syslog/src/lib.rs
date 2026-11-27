// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog
// Responsibility:   System log readers for Danaco Nurt.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! System log readers for Danaco Nurt.
//!
//! The crate reads systemd journal files on Linux directly from their file
//! format, the unified log on macOS and the event log on Windows, and
//! implements the `logs` command over them. No external program is run.
//! Unsafe code is limited to operating system interfaces of the Windows and
//! macOS readers.

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod cursor;
pub mod event;
pub mod eventlog;
pub mod filter;
pub mod journald;
pub mod limits;
pub mod merge;
pub mod query;
pub mod stage;
pub mod unified;

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageRegistrar;

/// Adds the stage of the `logs` command to `registrar`.
///
/// The stage reads the log of the running system: the systemd journal on
/// Linux, the event log on Windows and the unified log on macOS.
///
/// # Errors
///
/// Fails with the error of the registrar, such as `E_UNKNOWN_COMMAND` when
/// `logs` is not in the catalog of the registry.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_syslog::register(&mut stages)?;
/// assert_eq!(stages.len(), 1);
/// assert_eq!(stages[0].descriptor().command, "logs");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    registrar.register(Arc::new(stage::LogsStage::new()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::stage::StageImpl;

    /// Names of the `[[command]]` tables of `catalog/system.toml`; the name
    /// is the first `name` key of each table.
    fn catalog_commands() -> Vec<String> {
        let path = format!(
            "{}/../../catalog/system.toml",
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR"))
        );
        let text = std::fs::read_to_string(path).unwrap();
        let mut names = Vec::new();
        let mut in_command = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_command = line == "[[command]]";
            } else if let (true, Some(value)) = (in_command, line.strip_prefix("name = ")) {
                names.push(value.trim_matches('"').to_owned());
                in_command = false;
            }
        }
        names
    }

    #[test]
    fn registers_the_logs_stage_of_the_catalog() {
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        super::register(&mut stages).unwrap();
        let names: Vec<String> = stages
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        assert_eq!(names, [super::stage::COMMAND]);
        let catalog = catalog_commands();
        assert!(
            names.iter().all(|name| catalog.contains(name)),
            "{catalog:?}"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
