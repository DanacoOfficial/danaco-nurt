// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Working tree update engine: plans changes of the working tree and the
//! index and applies them through a mutation transaction.
//!
//! An update has two phases. A plan function reads the repository through an
//! [`ApplyContext`] and computes an [`ApplyPlan`] in memory: the ordered
//! working tree changes, the new index, reference updates and state files.
//! Every check that can refuse the update (local changes, untracked files in
//! the way, conflict stages, unsafe paths) runs during planning, so a refused
//! update changes nothing. [`apply_plan`] then executes the plan in one
//! [`MutationTx`](nurt_contracts::journal::MutationTx): each target is
//! preserved before the first change, files are written in the order used by
//! `git`, and the index and references are replaced through `.lock` files.
//!
//! | Plan function | Semantics |
//! |---|---|
//! | [`plan_two_way`] | `git switch`, `git checkout <branch>`, fast-forward merges: local changes of untouched paths are carried over |
//! | [`plan_reset`] | `git reset --hard`, `git switch --discard-changes`: the index and the working tree become a tree |
//! | [`plan_paths`] | `git restore`: selected paths of the index or the working tree from the index or a tree |
//! | [`plan_new_tree`] | checkout of a new working tree with an empty index |
//!
//! Content is converted by the built-in filters of `gix-filter`; filter
//! drivers configured by the user are not started and the affected paths are
//! reported as `filter_not_applied`.

mod changes;
mod context;
mod error;
mod exec;
mod filter;
mod layout;
mod oneway;
mod paths;
mod plan;
mod refs;
mod resolve_undo;
mod sparse;
#[cfg(any(test, feature = "testutil"))]
pub mod testjournal;
#[cfg(test)]
mod tests;
mod twoway;
mod worktree;

pub use context::ApplyContext;
pub use error::Rejections;
pub use exec::apply_plan;
pub use oneway::{ResetOptions, plan_new_tree, plan_reset};
pub use paths::{PathSource, PathsRequest, plan_paths};
pub use plan::{
    ApplyPlan, ApplyReport, Content, EntryAction, EntryKind, IndexPlan, Notable, PlanEntry,
    RefPlan, RefTarget, ReflogLine, StateFilePlan,
};
pub use refs::{keeps_log, log_path, ref_path};
pub use twoway::plan_two_way;
