// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::merge
// Responsibility:   The `git.merge` command and three-way merge of trees.
// Dependencies:     command, conflict, driver, finish, message, source, stage, trees
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.merge` command and three-way merge of trees.
//!
//! [`merge_trees`] and [`merge_commits`] compute a merge in the object
//! database without touching the working tree, the index or the references;
//! they serve `git.merge`, `git.rebase`, `git.stash` and `git.pull`. The
//! result tree holds the content Git leaves in the working tree, and the
//! result index holds the conflict stages. Text is merged with the
//! histogram algorithm, as by `merge-ort`, whatever `diff.algorithm` says.
//!
//! [`plan_merge`] and [`apply_merge`] implement `git.merge`: the start of a
//! merge (fast-forward, merge commit or stop with conflicts), its
//! conclusion and its abort. [`prepare_completion`] reads a stopped merge
//! for the commit that concludes it, also when `git.commit` concludes it.

mod abort;
mod command;
mod conflict;
mod driver;
mod finish;
#[cfg(test)]
mod fixture;
mod message;
mod source;
mod stage;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_command;
mod trees;

pub use command::{
    FastForward, MergeAction, MergePlan, MergeRequest, MergeResult, apply_merge, plan_merge,
};
pub use conflict::{MergeConflict, kind_from_stages};
pub use driver::ExternalDriver;
pub use finish::{MergeCompletion, prepare_completion, unmerged_error, unmerged_paths};
pub use message::{
    Cleanup, DEFAULT_SUPPRESSED_DESTINATIONS, MergeSource, clean_message, merge_message,
    reflog_commit_merge, reflog_fast_forward, reflog_merge, with_conflicts_section,
};
pub use source::MergeTarget;
pub use stage::MergeCommand;
pub use trees::{
    ConflictStyle, DEFAULT_MARKER_SIZE, DEFAULT_RENAME_LIMIT, Favor, MergeLabels, MergeOptions,
    MergeOutcome, merge_base_virtual, merge_bases, merge_commits, merge_trees, unrelated_error,
};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
