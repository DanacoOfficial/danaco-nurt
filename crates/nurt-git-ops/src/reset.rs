// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.reset` command.
//!
//! `git.reset` moves the current branch, or a detached `HEAD`, to a
//! revision. `soft` changes only the reference, `mixed` also replaces the
//! index, `hard` also brings the working tree to the revision. The
//! previous value of `HEAD` is written to `ORIG_HEAD`, reflog entries read
//! `reset: moving to <revision>`, and the state files of a merge in progress
//! are removed. Every change goes through one journal transaction.

pub(crate) mod args;
pub(crate) mod execute;
pub(crate) mod ident;
pub(crate) mod index;
pub(crate) mod repo;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
pub(crate) mod worktree;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use gix::bstr::BString;
use gix::hash::ObjectId;
use gix::index::entry::Stat;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{
    Change, ChangeAction, Effect, EffectStatus, GitEffect, GitEffectState,
};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{self, Record};
use nurt_git::apply::{ApplyContext, IndexPlan, ResetOptions};
use nurt_git::repo::GitRepo;

use self::execute::{move_head, ref_tx, state_removal, touched};
use self::ident::Identity;
use self::index::StatSources;
use self::repo::{error, root_literal};
use self::worktree::Worktree;

/// State files of a merge, cherry-pick or revert that `git reset` removes.
pub(crate) const BRANCH_STATE: [&str; 8] = [
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_RR",
    "AUTO_MERGE",
    "SQUASH_MSG",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
];

/// What `git.reset` resets besides the reference.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::reset::ResetMode;
///
/// let mode: ResetMode = "hard".parse().unwrap_or_default();
/// assert_eq!(mode, ResetMode::Hard);
/// assert_eq!(ResetMode::default().as_str(), "mixed");
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ResetMode {
    /// Only the reference.
    Soft,
    /// The reference and the index.
    #[default]
    Mixed,
    /// The reference, the index and the working tree.
    Hard,
}

impl ResetMode {
    /// Name of the mode as in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Mixed => "mixed",
            Self::Hard => "hard",
        }
    }
}

impl FromStr for ResetMode {
    type Err = nurt_contracts::error::Error;

    /// Parses a mode name.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_VALUE` for a name that is not a mode.
    fn from_str(text: &str) -> Result<Self> {
        match text {
            "soft" => Ok(Self::Soft),
            "mixed" => Ok(Self::Mixed),
            "hard" => Ok(Self::Hard),
            other => Err(error(
                ErrorCode::UnknownValue,
                format!("unknown reset mode `{other}`; use soft, mixed or hard"),
            )
            .with_expected(serde_json::json!(["soft", "mixed", "hard"]))),
        }
    }
}

/// Parameters of a reset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetRequest {
    /// Revision to move to, recorded in the reflog as given.
    pub revision: String,
    /// What is reset besides the reference.
    pub mode: ResetMode,
}

/// Result of a reset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResetOutcome {
    /// Root of the working tree.
    pub repo: PathBuf,
    /// Commit of `HEAD` before; `None` on an unborn branch.
    pub head_before: Option<ObjectId>,
    /// Commit of `HEAD` after.
    pub head_after: Option<ObjectId>,
    /// Current branch, `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// Working tree paths written, relative to the root.
    pub written: Vec<String>,
    /// Working tree paths removed, relative to the root.
    pub removed: Vec<String>,
    /// State files removed, such as `MERGE_HEAD`.
    pub state_removed: Vec<String>,
    /// The branch moved to a commit that does not contain its previous
    /// commit.
    pub history_rewritten: bool,
}

/// A planned reset: the changes to make and the outcome they have.
pub struct PlannedReset {
    repo: GitRepo,
    change: execute::Change,
    outcome: ResetOutcome,
}

impl std::fmt::Debug for PlannedReset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlannedReset")
            .field("targets", &self.change.target_count())
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

impl PlannedReset {
    /// Number of journal targets the reset declares.
    #[must_use]
    pub fn target_count(&self) -> usize {
        self.change.target_count()
    }

    /// The outcome the reset will have.
    #[must_use]
    pub fn outcome(&self) -> &ResetOutcome {
        &self.outcome
    }

    /// Performs the reset through `tx`.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the journal after the transaction was rolled
    /// back: `E_STALE` when a target changed after the plan, `E_GIT_LOCKED`
    /// when a lock file appeared and `E_IO`.
    pub fn apply(self, tx: &mut dyn MutationTx) -> Result<ResetOutcome> {
        self.change.run(&self.repo, tx)?;
        Ok(self.outcome)
    }
}

/// Index of a mixed reset: the entries of `tree`, with the file status data
/// of unchanged entries kept and fresh data for files that already hold the
/// content.
fn mixed_index(repo: &GitRepo, context: &ApplyContext<'_>, tree: ObjectId) -> Result<IndexPlan> {
    let old = context.index();
    let state = index::from_tree(repo, tree)?;
    let mut worktree = Worktree::new(repo)?;
    let mut read_now: HashSet<BString> = HashSet::new();
    for entry in state.entries() {
        let path = entry.path(&state);
        let reusable = index::merged_entry(old, path).is_some_and(|previous| {
            previous.id == entry.id
                && previous.mode == entry.mode
                && previous.stat != Stat::default()
        });
        if !reusable && worktree.matches(path, entry)? {
            read_now.insert(path.to_owned());
        }
    }
    let sources = StatSources {
        written: None,
        read_now: Some(&read_now),
    };
    Ok(IndexPlan {
        state: index::with_stats(repo.root(), state, old, &sources),
        expect: context.index_expect().clone(),
        record_resolve_undo: false,
    })
}

/// Plans a reset of the repository containing `path`.
///
/// Nothing is changed; the result is applied with [`PlannedReset::apply`].
///
/// # Errors
///
/// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not in a repository
/// with a working tree, `E_GIT_REVISION` when the revision does not name a
/// commit, `E_GIT_IN_PROGRESS` for a soft reset during a merge,
/// `E_GIT_LOCKED` when `index.lock` or a reference lock exists and `E_IO`
/// when the repository cannot be read.
pub fn plan_reset(path: &Path, request: &ResetRequest, ctx: &ExecCtx) -> Result<PlannedReset> {
    let repo = repo::open(path, ctx)?;
    let identity = Identity::resolve(&repo);
    let context = ApplyContext::open(repo.gix())?;
    let head = repo::head(&repo)?;
    let target = repo.resolve_commit(&request.revision)?;
    if request.mode == ResetMode::Soft
        && (repo.git_dir().join("MERGE_HEAD").exists() || index::has_conflicts(context.index()))
    {
        return Err(error(
            ErrorCode::GitInProgress,
            "cannot do a soft reset in the middle of a merge; use mode hard or mixed",
        )
        .with_problem("merge_in_progress")
        .with_fix(format!(
            "git.reset {} revision: {} mode: mixed",
            root_literal(&repo),
            nurt_git::repo::error::quote(&request.revision)
        )));
    }
    let tree = repo::commit_tree(&repo, target)?;
    let mut change = execute::Change::new();
    match request.mode {
        ResetMode::Soft => {}
        ResetMode::Mixed => change.plan.index = Some(mixed_index(&repo, &context, tree)?),
        ResetMode::Hard => {
            change.plan = nurt_git::apply::plan_reset(
                &context,
                &tree,
                ResetOptions {
                    overwrite_untracked: true,
                },
            )?;
        }
    }
    let (written, removed) = touched(&change.plan);
    let (files, state_removed) = state_removal(&repo, &BRANCH_STATE)?;
    change.plan.state_files.extend(files);
    let mut refs = ref_tx(&repo, &identity.committer)?;
    move_head(
        &mut refs,
        &head,
        target,
        &format!("reset: moving to {}", request.revision),
    )?;
    change.set_refs(refs)?;
    let history_rewritten = head.branch.is_some()
        && head
            .id
            .is_some_and(|old| old != target && !repo::is_ancestor(&repo, old, target));
    let outcome = ResetOutcome {
        repo: repo.root().to_path_buf(),
        head_before: head.id,
        head_after: Some(target),
        branch: head.short_branch(),
        written,
        removed,
        state_removed,
        history_rewritten,
    };
    drop(context);
    Ok(PlannedReset {
        repo,
        change,
        outcome,
    })
}

/// File changes of the working tree for the effect report.
pub(crate) fn file_changes(root: &Path, written: &[String], removed: &[String]) -> Vec<Change> {
    let change = |path: &String, action| Change {
        path: value::Path::new(root.join(path).to_string_lossy().into_owned()),
        action,
        ..Change::default()
    };
    written
        .iter()
        .map(|path| change(path, ChangeAction::Modify))
        .chain(
            removed
                .iter()
                .map(|path| change(path, ChangeAction::Delete)),
        )
        .collect()
}

/// Effect record of a repository operation.
pub(crate) fn effect_record(
    kind: &str,
    git: GitEffect,
    changes: &[Change],
    notable: &[NotableEntry],
) -> Record {
    let codes: Vec<String> = notable.iter().map(|entry| entry.code.to_string()).collect();
    Effect {
        status: EffectStatus::Ok,
        kind: kind.to_owned(),
        changes: (!changes.is_empty()).then(|| changes.to_vec()),
        notable: (!codes.is_empty()).then_some(codes),
        git: Some(git),
        ..Effect::default()
    }
    .into()
}

fn hex(id: Option<ObjectId>) -> Option<String> {
    id.map(|id| id.to_hex().to_string())
}

impl ResetOutcome {
    fn notable(&self) -> Vec<NotableEntry> {
        let mut notable = Vec::new();
        if self.history_rewritten {
            notable.push(NotableEntry::new(Notable::HistoryRewritten));
        }
        if self.branch.is_none() {
            notable.push(NotableEntry::new(Notable::DetachedHead));
        }
        notable
    }

    fn git_effect(&self) -> GitEffect {
        GitEffect {
            repo: value::Path::new(self.repo.to_string_lossy().into_owned()),
            state: GitEffectState::Completed,
            head_before: hex(self.head_before),
            head_after: hex(self.head_after),
            branch: self.branch.clone(),
            ..GitEffect::default()
        }
    }
}

/// Stage implementation of `git.reset`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResetCommand;

fn request_of(args: &BoundArgs) -> Result<(PathBuf, ResetRequest)> {
    let repo = args::path(args, "repo")?;
    let revision = args::required_text(args, "revision")?;
    let mode = match args::text(args, "mode")? {
        Some(mode) => mode.parse()?,
        None => ResetMode::default(),
    };
    Ok((repo, ResetRequest { revision, mode }))
}

impl StageImpl for ResetCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "git.reset".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for ResetCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let (repo, request) = request_of(args)?;
        let planned = plan_reset(&repo, &request, ctx)?;
        let outcome = planned.outcome();
        let mut plan = EffectPlan::new();
        plan.changes = file_changes(&outcome.repo, &outcome.written, &outcome.removed);
        plan.notable = outcome.notable();
        plan.bulk = planned.target_count() > 1;
        Ok(plan.with_state(planned))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan
            .take_state::<PlannedReset>()
            .ok_or_else(|| error(ErrorCode::Internal, "the plan of git.reset is missing"))?;
        let outcome = planned.apply(tx)?;
        let changes = file_changes(&outcome.repo, &outcome.written, &outcome.removed);
        let notable = outcome.notable();
        let record = effect_record("git.reset", outcome.git_effect(), &changes, &notable);
        let mut result = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        result.changes = changes;
        result.notable = notable;
        Ok(result)
    }
}
