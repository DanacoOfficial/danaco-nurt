// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Schedules for Danaco Nurt.
//!
//! The crate evaluates cron expressions, intervals and single times, keeps
//! schedule definitions and their run state, decides about missed, late and
//! overlapping runs, implements the `schedule.*` commands and provides the
//! schedule module hosted by the background service.
//!
//! Definitions live in `<state>/schedules/definitions/<id>.json` and change
//! only through the operation journal; the run state in
//! `<state>/schedules/state.json` is written only by the schedule module.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod action;
pub mod engine;
mod errors;
pub mod module;
pub mod settings;
pub mod stages;
pub mod store;
pub mod trigger;

use nurt_contracts::error::Result;
use nurt_contracts::stage::StageRegistrar;

/// Adds the stages of the `schedule.*` commands to `registrar`.
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
/// nurt_sched::register(&mut stages)?;
/// assert_eq!(stages.len(), 7);
/// assert_eq!(stages[0].descriptor().command, "schedule.create");
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

    /// Sorted names of the `[[command]]` tables of `catalog/schedule.toml`;
    /// the name is the first `name` key of each table.
    fn catalog_commands() -> Vec<String> {
        let path = format!(
            "{}/../../catalog/schedule.toml",
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
        assert_eq!(names, catalog_commands());
    }
}
