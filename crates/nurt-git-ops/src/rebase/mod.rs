// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::rebase
// Responsibility:   The `git.rebase` command: replaying the commits of the current branch onto another revision with the `merge` back end of Git.
// Dependencies:     command, stage
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.rebase` command: replaying the commits of the current branch
//! onto another revision with the `merge` back end of Git.
//!
//! The commits of `upstream..HEAD` are selected without merge commits and
//! without commits whose patch identifier equals that of a commit of the
//! upstream; each one is merged onto the new base in the object database.
//! The working tree and the index are written once, from their state before
//! the action to the final state, in one journal transaction. A pick that
//! leaves conflicts stops the rebase with the state directory
//! `rebase-merge/` in the format of Git 2.53, so that Git and the console can
//! continue, skip or abort a rebase started by the other.
//!
//! [`plan_rebase`] computes the complete plan without changing the working
//! tree, the index or the references; [`apply_rebase`] writes it.

mod command;
mod error;
mod history;
mod objects;
mod patch;
mod sequencer;
mod stage;
mod state_dir;
#[cfg(test)]
mod tests;

pub use command::{
    DEFAULT_REFLOG_ACTION, RebaseAction, RebaseOnto, RebasePlan, RebaseRequest, RebaseResult,
    RebaseStep, RebaseSummary, apply_rebase, plan_rebase,
};
pub use stage::RebaseCommand;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
