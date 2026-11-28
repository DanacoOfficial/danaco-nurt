// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::registry
// Responsibility:   Registration of the stage implementations of the crate.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registration of the stage implementations of the crate.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{StageImpl, StageRegistrar};

use crate::merge::MergeCommand;
use crate::pull::{GitPull, PlanFetch, PlanRebase};
use crate::push::PushCommand;
use crate::rebase::RebaseCommand;
use crate::reset::ResetCommand;
use crate::restore::RestoreCommand;
use crate::stash::StashCommand;
use crate::switch::SwitchCommand;

/// Adds the stage implementations of the crate to `registrar`.
///
/// The crate implements `git.reset`, `git.merge`, `git.stash`, `git.push`,
/// `git.pull`, `git.switch` and `git.restore`, added in this order, one per
/// command.
///
/// # Errors
///
/// Returns the first error of the registrar, usually `E_UNKNOWN_COMMAND`
/// when the catalog of the registrar has no entry for a command; the
/// implementations after the rejected one are not added.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::StageImpl;
///
/// let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
/// nurt_git_ops::register(&mut stages)?;
/// assert!(stages.iter().any(|stage| stage.descriptor().command == "git.merge"));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    for stage in stages() {
        registrar.register(stage)?;
    }
    Ok(())
}

fn stages() -> [Arc<dyn StageImpl>; 8] {
    [
        Arc::new(ResetCommand),
        Arc::new(MergeCommand),
        Arc::new(RebaseCommand),
        Arc::new(StashCommand),
        Arc::new(PushCommand),
        Arc::new(GitPull::new(Arc::new(PlanFetch)).with_rebase(Arc::new(PlanRebase))),
        Arc::new(SwitchCommand),
        Arc::new(RestoreCommand),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::stage::{InputKinds, StageImpl, StageRegistrar};

    use super::register;

    const CATALOG: &str = include_str!("../../../catalog/git.toml");

    /// Command name to the accepted input kinds declared in the catalog.
    fn catalog_inputs() -> BTreeMap<String, Vec<String>> {
        let catalog: toml::Table = toml::from_str(CATALOG).expect("catalog/git.toml parses");
        let commands = catalog["command"]
            .as_array()
            .expect("catalog has a command array");
        commands
            .iter()
            .map(|command| {
                let name = command["name"].as_str().expect("command name").to_owned();
                let input = command["input"]
                    .as_array()
                    .expect("command input")
                    .iter()
                    .map(|kind| kind.as_str().expect("input kind").to_owned())
                    .collect();
                (name, input)
            })
            .collect()
    }

    fn registered() -> Vec<Arc<dyn StageImpl>> {
        let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
        register(&mut stages).expect("registration succeeds");
        stages
    }

    #[test]
    fn registers_only_commands_of_the_catalog() {
        let catalog = catalog_inputs();
        for stage in registered() {
            let command = stage.descriptor().command;
            assert!(
                catalog.contains_key(&command),
                "{command} is not in catalog/git.toml"
            );
        }
    }

    #[test]
    fn registers_each_command_once() {
        let names: Vec<String> = registered()
            .iter()
            .map(|stage| stage.descriptor().command)
            .collect();
        let unique: BTreeSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "duplicate command in {names:?}");
        assert_eq!(names.len(), 8);
    }

    #[test]
    fn registered_stages_match_the_declared_input() {
        let catalog = catalog_inputs();
        for stage in registered() {
            let descriptor = stage.descriptor();
            let declared = &catalog[&descriptor.command];
            if declared.iter().all(|kind| kind == "none") {
                assert!(
                    matches!(descriptor.input, InputKinds::None),
                    "{} takes input the catalog does not declare",
                    descriptor.command
                );
            }
        }
    }

    #[test]
    fn registered_stages_provide_an_execution_interface() {
        for stage in registered() {
            assert!(
                stage.as_read().is_some() || stage.as_effect().is_some(),
                "{} has neither a read nor an effect interface",
                stage.descriptor().command
            );
        }
    }

    struct RejectingRegistrar {
        accepted: usize,
    }

    impl StageRegistrar for RejectingRegistrar {
        fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
            if self.accepted == 1 {
                return Err(Error::new(
                    ErrorCode::UnknownCommand,
                    format!("unknown command `{}`", stage.descriptor().command),
                ));
            }
            self.accepted += 1;
            Ok(())
        }
    }

    #[test]
    fn stops_at_the_first_rejected_stage() {
        let mut registrar = RejectingRegistrar { accepted: 0 };
        let error = register(&mut registrar).expect_err("second stage is rejected");
        assert_eq!(error.code(), ErrorCode::UnknownCommand);
        assert_eq!(registrar.accepted, 1);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
