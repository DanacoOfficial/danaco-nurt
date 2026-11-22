// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.merge` command and three-way merge of trees.
//!
//! [`merge_trees`] and [`merge_commits`] compute a merge in the object
//! database without touching the working tree, the index or the references;
//! they serve `git.merge`, `git.rebase`, `git.stash` and `git.pull`. The
//! result tree holds the content Git leaves in the working tree, and the
//! result index holds the conflict stages. Text is merged with the
//! histogram algorithm, as by `merge-ort`, whatever `diff.algorithm` says.
//!
//! [`prepare_completion`] reads a stopped merge for the commit that
//! concludes it.

mod conflict;
mod driver;
mod finish;
#[cfg(test)]
mod fixture;
mod message;
#[cfg(test)]
mod tests;
mod trees;

pub use conflict::{MergeConflict, kind_from_stages};
pub use driver::{DriverMode, ExternalDriver};
pub use finish::{MergeCompletion, prepare_completion, unmerged_error, unmerged_paths};
pub use message::{
    Cleanup, DEFAULT_SUPPRESSED_DESTINATIONS, MergeSource, clean_message, merge_message,
    reflog_commit_merge, reflog_fast_forward, reflog_merge, with_conflicts_section,
};
pub use trees::{
    ConflictStyle, DEFAULT_MARKER_SIZE, DEFAULT_RENAME_LIMIT, Favor, MergeLabels, MergeOptions,
    MergeOutcome, merge_base_virtual, merge_bases, merge_commits, merge_trees, unrelated_error,
};
