// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registration of the stage implementations of the crate.

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{StageImpl, StageRegistrar};

use crate::blame::GitBlame;
use crate::branch::GitBranchCommand;
use crate::checkpoint::CheckpointCommand;
use crate::clone::GitCloneCommand;
use crate::commit::GitCommit;
use crate::diff::GitDiff;
use crate::fetch::GitFetchCommand;
use crate::log::GitLog;
use crate::review::GitReview;
use crate::show::GitShow;
use crate::stage::GitStageCommand;
use crate::status::GitStatus;
use crate::tag::GitTagCommand;
use crate::worktree::WorktreeCommand;

/// Adds the stage implementations of the crate to `registrar`.
///
/// The crate implements `git.status`, `git.diff`, `git.log`, `git.show`,
/// `git.blame`, `git.branch`, `git.tag`, `git.stage`, `git.commit`,
/// `git.review`, `git.checkpoint`, `git.worktree`, `git.fetch` and `git.clone`. The
/// implementations are added in this order, one per command.
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
/// nurt_git::register(&mut stages)?;
/// assert!(stages.iter().any(|stage| stage.descriptor().command == "git.status"));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn register(registrar: &mut dyn StageRegistrar) -> Result<()> {
    for stage in stages() {
        registrar.register(stage)?;
    }
    Ok(())
}

fn stages() -> [Arc<dyn StageImpl>; 14] {
    [
        Arc::new(GitStatus),
        Arc::new(GitDiff),
        Arc::new(GitLog),
        Arc::new(GitShow),
        Arc::new(GitBlame),
        Arc::new(GitBranchCommand),
        Arc::new(GitTagCommand),
        Arc::new(GitStageCommand),
        Arc::new(GitCommit),
        Arc::new(GitReview),
        Arc::new(CheckpointCommand),
        Arc::new(WorktreeCommand),
        Arc::new(GitFetchCommand),
        Arc::new(GitCloneCommand),
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
        assert_eq!(names.len(), 14);
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
        limit: usize,
    }

    impl StageRegistrar for RejectingRegistrar {
        fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
            if self.accepted == self.limit {
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
        let mut registrar = RejectingRegistrar {
            accepted: 0,
            limit: 2,
        };
        let error = register(&mut registrar).expect_err("third stage is rejected");
        assert_eq!(error.code(), ErrorCode::UnknownCommand);
        assert_eq!(registrar.accepted, 2);
    }
}
