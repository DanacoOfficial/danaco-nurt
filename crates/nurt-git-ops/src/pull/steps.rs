// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Transfer and rebase engines that a pull composes.
//!
//! A pull writes the transfer and the integration in one journal
//! transaction, with every target preserved before the first write. The
//! engines therefore leave the local part of their work to the pull: the
//! transfer adds the received objects to the object store, records that
//! exempt write and returns the updates of the remote-tracking references
//! and the content of `FETCH_HEAD` as a plan; the rebase receives that plan
//! and writes it together with its own changes.

use gix::ObjectId;
use nurt_contracts::error::{Error, Result};
use nurt_contracts::journal::{Expect, MutationTx};
use nurt_contracts::record::{GitEffect, GitRefUpdate};
use nurt_contracts::report::{Change, NotableEntry};
use nurt_contracts::stage::ExecCtx;
use nurt_git::apply::{ApplyPlan, RefPlan, StateFilePlan};
use nurt_git::repo::GitRepo;

use super::settings::RebaseKind;

/// File of the repository directory that lists the fetched heads.
pub const FETCH_HEAD: &str = "FETCH_HEAD";

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
    /// SHA-256 fingerprint of an SSH host key to accept when the host is not
    /// known.
    pub accept_host_key: Option<String>,
}

/// Head written to `FETCH_HEAD` for merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedHead {
    /// Fetched commit.
    pub id: ObjectId,
    /// Description as written to `FETCH_HEAD`, for example
    /// `branch 'main' of https://example.com/app.git`.
    pub description: String,
    /// Full name of the reference on the remote, for example
    /// `refs/heads/main`.
    pub remote_ref: String,
}

/// Result of the transfer: received objects are in the object store, the
/// local writes are planned and not yet made.
#[derive(Debug, Clone, Default)]
pub struct FetchOutcome {
    /// URL of the remote as written to `FETCH_HEAD`.
    pub url: String,
    /// Updates of the remote-tracking references with their reflog lines.
    pub refs: Vec<RefPlan>,
    /// Writes of the repository directory: `FETCH_HEAD` and, after a
    /// shallow transfer, `shallow`.
    pub state_files: Vec<StateFilePlan>,
    /// Records of the reference updates.
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

impl FetchOutcome {
    /// Write of `FETCH_HEAD` with `content`, which `git` replaces without
    /// checking the previous content.
    #[must_use]
    pub fn fetch_head_file(content: Vec<u8>) -> StateFilePlan {
        StateFilePlan {
            path: FETCH_HEAD.to_owned(),
            content: Some(content),
            expect: Expect::Unchecked,
        }
    }

    /// Every local write of the transfer: the remote-tracking references and
    /// the state files.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::pull::{FETCH_HEAD, FetchOutcome};
    ///
    /// let line = b"0123\t\tbranch 'main' of https://example.com/app.git\n".to_vec();
    /// let fetched = FetchOutcome {
    ///     state_files: vec![FetchOutcome::fetch_head_file(line)],
    ///     ..FetchOutcome::default()
    /// };
    /// let plan = fetched.local_plan();
    /// assert_eq!(plan.state_files[0].path, FETCH_HEAD);
    /// assert!(plan.entries.is_empty() && plan.index.is_none());
    /// ```
    #[must_use]
    pub fn local_plan(&self) -> ApplyPlan {
        ApplyPlan {
            entries: Vec::new(),
            index: None,
            refs: self.refs.clone(),
            state_files: self.state_files.clone(),
            carried: Vec::new(),
            notable: Vec::new(),
        }
    }
}

/// Transfer engine of a pull.
pub trait PullFetch: Send + Sync {
    /// Fetches from the remote into the object store and plans the local
    /// writes of the transfer.
    ///
    /// The received objects are added to the object store and recorded with
    /// [`MutationTx::record_exempt`]; no other target of `tx` is declared or
    /// written. The returned [`FetchOutcome::refs`] are planned against the
    /// state after the transfer.
    ///
    /// # Errors
    ///
    /// `E_NETWORK`, `E_GIT_AUTH`, `E_GIT_HOST_KEY`, `E_GIT_PROTOCOL` and
    /// `E_GIT_REF` for an unknown remote or refspec.
    fn fetch(
        &self,
        repo: &GitRepo,
        request: &FetchRequest,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> Result<FetchOutcome>;
}

/// Rebase of the local commits onto the fetched head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseRequest {
    /// Repository as given by the program, used in the calls of `fix` and
    /// `next_steps`.
    pub repo: String,
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

/// Failure of a rebase engine.
#[derive(Debug, Clone)]
pub enum RebaseError {
    /// The rebase was refused before any target was declared; the pull then
    /// writes the transfer alone and reports the error with
    /// `effect_applied`.
    Refused(Error),
    /// The rebase stopped or failed after using the transaction; the error
    /// is reported as it is.
    Failed(Error),
}

/// Rebase engine of a pull.
pub trait PullRebase: Send + Sync {
    /// Rebases the current branch onto `request.onto` and writes `transfer`
    /// in the same transaction.
    ///
    /// Every target of `transfer` is declared together with the targets of
    /// the rebase, before the first write.
    ///
    /// # Errors
    ///
    /// [`RebaseError::Refused`] with `E_GIT_DIRTY` or `E_GIT_IN_PROGRESS`
    /// before any write; [`RebaseError::Failed`] with `E_GIT_CONFLICT` and
    /// `effect_applied` when the rebase stopped with conflicts, or with
    /// `E_GIT_LOCKED` and `E_STALE` when a write failed.
    fn rebase(
        &self,
        repo: &GitRepo,
        request: &RebaseRequest,
        transfer: ApplyPlan,
        tx: &mut dyn MutationTx,
        ctx: &ExecCtx,
    ) -> std::result::Result<LocalOutcome, RebaseError>;
}
