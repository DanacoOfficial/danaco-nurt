// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Transfer and integration steps that a pull composes.
//!
//! The steps are provided by the fetch, merge, rebase and checkout engines
//! of the Git crates. Every step writes only through the transaction it
//! receives; a pull passes the same transaction to the transfer and to the
//! integration, so the local part is one journal operation.

use std::path::Path;

use gix::ObjectId;
use nurt_contracts::error::Result;
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{GitEffect, GitRefUpdate};
use nurt_contracts::report::{Change, NotableEntry};
use nurt_contracts::stage::ExecCtx;

use super::settings::{FfPolicy, RebaseKind};

/// Transfer requested by a pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// Remote name, URL or `.`.
    pub remote: String,
    /// Refspecs given by the program; empty selects `remote.<name>.fetch`.
    pub refspecs: Vec<String>,
    /// Prefix of the reflog messages of the updated remote-tracking
    /// references, for example `pull --rebase`.
    pub reflog_action: String,
}

/// Head written to `FETCH_HEAD` for merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedHead {
    /// Fetched commit.
    pub id: ObjectId,
    /// Description as written to `FETCH_HEAD`, for example
    /// `branch 'main' of ../upstream`.
    pub description: String,
    /// Full name of the reference on the remote, for example
    /// `refs/heads/main`.
    pub remote_ref: String,
}

/// Result of the transfer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FetchOutcome {
    /// Updates of the remote-tracking references.
    pub ref_updates: Vec<GitRefUpdate>,
    /// Heads marked for merging in `FETCH_HEAD`, in file order.
    pub for_merge: Vec<FetchedHead>,
    /// Objects received.
    pub objects_received: u64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Unusual effects of the transfer.
    pub notable: Vec<NotableEntry>,
}

/// Merge of the fetched heads into the current branch, as `git merge`
/// performs it on behalf of `git pull`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    /// Heads to merge, in `FETCH_HEAD` order.
    pub theirs: Vec<ObjectId>,
    /// Message of a merge commit.
    pub message: String,
    /// Fast-forward policy.
    pub fast_forward: FfPolicy,
    /// Prefix of the reflog messages, for example `pull`.
    pub reflog_action: String,
}

/// Rebase of the local commits onto the fetched head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseRequest {
    /// New base of the local commits.
    pub onto: ObjectId,
    /// Upstream that limits the commits to replay: the fork point recorded
    /// in the reflog of the remote-tracking branch when it differs from the
    /// merge base, otherwise `onto`.
    pub upstream: ObjectId,
    /// Rebase setting; [`RebaseKind::Merges`] recreates merge commits.
    pub kind: RebaseKind,
    /// Stash uncommitted changes before and apply them after the rebase.
    pub autostash: bool,
    /// Prefix of the reflog messages, for example `pull --rebase`.
    pub reflog_action: String,
}

/// Checkout of the fetched head on a branch without commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialRequest {
    /// Fetched head.
    pub theirs: ObjectId,
    /// Branch to create, for example `refs/heads/main`.
    pub branch: String,
    /// Complete reflog message; `git` writes `initial pull`.
    pub reflog_message: String,
}

/// Result of the local integration.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocalOutcome {
    /// Effect on the repository; the pull adds the reference updates of the
    /// transfer.
    pub effect: GitEffect,
    /// Changed files of the working tree.
    pub changes: Vec<Change>,
    /// Unusual effects.
    pub notable: Vec<NotableEntry>,
}

/// Engines that a pull composes.
///
/// Implementations write every change of the working tree and of the
/// repository directory through `tx` and add objects to the object store
/// as exempt writes. Errors keep the codes of the engines: `E_GIT_DIRTY`
/// when local changes would be overwritten and `E_GIT_CONFLICT` with
/// `effect_applied` when a merge or rebase stopped with conflicts.
pub trait PullSteps: Send + Sync {
    /// Fetches from the remote, updates the remote-tracking references and
    /// writes `FETCH_HEAD`.
    ///
    /// # Errors
    ///
    /// `E_NETWORK`, `E_GIT_AUTH`, `E_GIT_HOST_KEY`, `E_GIT_PROTOCOL` and
    /// `E_GIT_REF` for an unknown remote or refspec.
    fn fetch(
        &self,
        repo: &Path,
        request: &FetchRequest,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<FetchOutcome>;

    /// Merges the fetched heads into the current branch.
    ///
    /// # Errors
    ///
    /// `E_GIT_DIRTY`, `E_GIT_CONFLICT` and `E_GIT_LOCKED`.
    fn merge(
        &self,
        repo: &Path,
        request: &MergeRequest,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<LocalOutcome>;

    /// Rebases the current branch onto the fetched head.
    ///
    /// # Errors
    ///
    /// `E_GIT_DIRTY`, `E_GIT_CONFLICT` and `E_GIT_LOCKED`.
    fn rebase(
        &self,
        repo: &Path,
        request: &RebaseRequest,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<LocalOutcome>;

    /// Checks out the fetched head on the unborn current branch.
    ///
    /// # Errors
    ///
    /// `E_GIT_DIRTY` when untracked files would be overwritten and
    /// `E_GIT_LOCKED`.
    fn checkout_initial(
        &self,
        repo: &Path,
        request: &InitialRequest,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<LocalOutcome>;
}
