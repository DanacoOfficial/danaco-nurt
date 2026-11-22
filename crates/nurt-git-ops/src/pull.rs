// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.pull` command.
//!
//! A pull fetches from a remote and integrates the fetched branch into the
//! current branch by a fast-forward, a merge or a rebase, as `git pull` 2.53
//! does. The integration mode follows the `mode` parameter, otherwise
//! `branch.<name>.rebase`, `pull.rebase` and `pull.ff`; when none of them is
//! set and the histories diverged, the pull stops with
//! `E_GIT_NON_FAST_FORWARD` and asks for a mode.
//!
//! Checks that `git` makes before contacting the remote (unmerged files, an
//! unfinished merge or rebase, uncommitted changes before a rebase) run in the
//! plan step and change nothing. The transfer and the local integration run
//! in one journal transaction: the remote-tracking references and
//! `FETCH_HEAD` are recorded as previous content, the transfer itself is
//! marked irreversible (`remote_ref_updated`). A failure after the transfer
//! keeps the fetched state, as `git` does, and the error carries
//! `effect_applied`.

mod decide;
mod errors;
mod message;
mod params;
mod probe;
mod settings;
mod stage;
mod steps;

pub use decide::{Action, Refusal, Relation, decide};
pub use message::{MergeSource, SourceKind, merge_message};
pub use params::{PullMode, PullParams};
pub use settings::{
    ConfigSource, DEFAULT_REMOTE, FfPolicy, InvalidSetting, RebaseKind, Strategy, Target,
    full_source_ref, reflog_action, resolve_strategy, resolve_target, tracking_ref,
};
pub use stage::GitPull;
pub use steps::{
    FetchOutcome, FetchRequest, FetchedHead, InitialRequest, LocalOutcome, MergeRequest, PullSteps,
    RebaseRequest,
};
