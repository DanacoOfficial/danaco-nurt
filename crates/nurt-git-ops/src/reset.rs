// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.reset` command.
//!
//! `git.reset` moves the current branch, or a detached `HEAD`, to a
//! revision. `soft` changes only the reference, `mixed` also replaces the
//! index, `hard` also brings the working tree to the revision; `merge` and
//! `keep` behave like `git reset --merge` and `git reset --keep`. The
//! previous value of `HEAD` is written to `ORIG_HEAD`, reflog entries read
//! `reset: moving to <revision>`, and the state files of a merge in progress
//! are removed. Every change goes through one journal transaction.

pub(crate) mod args;
pub(crate) mod ident;
pub(crate) mod index;
pub(crate) mod refs;
pub(crate) mod repo;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
pub(crate) mod txplan;
pub(crate) mod worktree;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::journal::{MutationTx, TargetPath, TargetSpec};
use nurt_contracts::record::{
    Change, ChangeAction, Effect, EffectStatus, GitEffect, GitEffectState, Record,
};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value;

pub use self::ident::Environment;
use self::ident::Identity;
use self::index::IndexUpdate;
use self::refs::{check_unlocked, plan_head_update, plan_ref_write, plan_state_removal};
use self::repo::{Repo, error, quote};
use self::txplan::TxPlan;
use self::worktree::{Checkout, Policy, Worktree, plan_checkout, plan_index_only};

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
    /// Like `hard`, keeping local changes of paths the reset does not
    /// change; refused when such changes would be lost.
    Merge,
    /// Like `hard` for paths that differ between `HEAD` and the revision;
    /// refused when one of them has local changes.
    Keep,
}

impl ResetMode {
    /// Name of the mode as in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Mixed => "mixed",
            Self::Hard => "hard",
            Self::Merge => "merge",
            Self::Keep => "keep",
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
            "merge" => Ok(Self::Merge),
            "keep" => Ok(Self::Keep),
            other => Err(error(
                ErrorCode::UnknownValue,
                format!("unknown reset mode `{other}`; use soft, mixed, hard, merge or keep"),
            )
            .with_expected(serde_json::json!(["soft", "mixed", "hard", "merge", "keep"]))),
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

/// A planned reset: the journal targets and the content to write.
pub struct PlannedReset {
    tx: TxPlan,
    index: Option<IndexUpdate>,
    outcome: ResetOutcome,
}

impl std::fmt::Debug for PlannedReset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlannedReset")
            .field("targets", &self.tx.targets().count())
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

impl PlannedReset {
    /// Journal targets in the order they are changed.
    #[must_use]
    pub fn targets(&self) -> Vec<TargetSpec> {
        self.tx.targets().cloned().collect()
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
    /// back: `E_STALE` when a target changed after the plan, `E_EXISTS` when
    /// a lock file appeared and `E_IO`.
    pub fn apply(self, tx: &mut dyn MutationTx) -> Result<ResetOutcome> {
        let mut index = self.index;
        let mut deferred = || -> Result<Vec<u8>> {
            index
                .take()
                .ok_or_else(|| error(ErrorCode::Internal, "the index was requested twice"))?
                .serialize()
        };
        self.tx.execute(tx, &mut deferred)?;
        Ok(self.outcome)
    }
}

fn policy(mode: ResetMode) -> Option<Policy> {
    match mode {
        ResetMode::Soft | ResetMode::Mixed => None,
        ResetMode::Hard => Some(Policy::Hard),
        ResetMode::Merge => Some(Policy::Merge),
        ResetMode::Keep => Some(Policy::Keep),
    }
}

fn to_strings(paths: Vec<BString>) -> Vec<String> {
    paths
        .into_iter()
        .map(|path| path.to_str_lossy().into_owned())
        .collect()
}

/// Plans a reset of the repository containing `path`.
///
/// Nothing is changed; the result lists the journal targets and is applied
/// with [`PlannedReset::apply`].
///
/// # Errors
///
/// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not in a repository
/// with a working tree, `E_GIT_REVISION` when the revision does not name a
/// commit, `E_GIT_IN_PROGRESS` for a soft reset during a merge,
/// `E_GIT_DIRTY` when `merge` or `keep` would lose local changes,
/// `E_GIT_LOCKED` when `index.lock` or a reference lock exists and `E_IO`
/// when the repository cannot be read.
pub fn plan_reset(path: &Path, request: &ResetRequest, env: &Environment) -> Result<PlannedReset> {
    let repo = Repo::open(path)?;
    let identity = Identity::resolve(&repo, env);
    check_unlocked(&repo.index_path())?;
    let head = repo.head()?;
    let target = repo.resolve_commit(&request.revision)?;
    let current = index::load(&repo)?;
    let root = quote(&repo.workdir.to_string_lossy());
    if request.mode == ResetMode::Soft
        && (repo.git_dir.join("MERGE_HEAD").exists() || index::has_conflicts(&current))
    {
        return Err(error(
            ErrorCode::GitInProgress,
            "cannot do a soft reset in the middle of a merge; use mode hard or mixed",
        )
        .with_problem("merge_in_progress")
        .with_fix(format!(
            "git.reset {root} revision: {} mode: mixed",
            quote(&request.revision)
        )));
    }
    let target_state = index::from_tree(&repo, repo.commit_tree(target)?)?;
    let mut tx = TxPlan::new();
    let mut outcome = ResetOutcome {
        repo: repo.workdir.clone(),
        head_before: head.id,
        head_after: Some(target),
        branch: head.branch.as_ref().map(|name| name.shorten().to_string()),
        ..ResetOutcome::default()
    };
    let index_update = match (request.mode, policy(request.mode)) {
        (ResetMode::Soft, _) => None,
        (_, None) => {
            let mut wt = Worktree::new(&repo)?;
            Some(plan_index_only(&mut wt, &current, target_state)?)
        }
        (_, Some(policy)) => {
            let head_state = match head.id {
                Some(id) => Some(index::from_tree(&repo, repo.commit_tree(id)?)?),
                None => None,
            };
            let mut wt = Worktree::new(&repo)?;
            let plan = plan_checkout(
                &mut wt,
                &Checkout {
                    current: &current,
                    head: head_state.as_ref(),
                    target: &target_state,
                    policy,
                    scope: None,
                    extra_removals: &[],
                },
            )?;
            tx.extend(plan.tx);
            outcome.written = to_strings(plan.written);
            outcome.removed = to_strings(plan.removed);
            Some(plan.index)
        }
    };
    if index_update.is_some() {
        tx.locked_write(&repo.index_path(), None)?;
    }
    let message = format!("reset: moving to {}", request.revision);
    plan_head_update(&mut tx, &repo, &head, target, &identity.committer, &message)?;
    if let Some(old) = head.id {
        plan_ref_write(&mut tx, &repo, "ORIG_HEAD", old)?;
        outcome.history_rewritten =
            head.branch.is_some() && old != target && !repo.is_ancestor(old, target);
    }
    outcome.state_removed = plan_state_removal(&mut tx, &repo, &BRANCH_STATE)?;
    Ok(PlannedReset {
        tx,
        index: index_update,
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
        .chain(removed.iter().map(|path| change(path, ChangeAction::Delete)))
        .collect()
}

/// Effect record of a repository operation.
pub(crate) fn effect_record(kind: &str, git: GitEffect, changes: &[Change], notable: &[NotableEntry]) -> Record {
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
    fn plan<'a>(&'a self, args: &'a BoundArgs, _input: StageInput<'a>, ctx: &'a ExecCtx) -> Result<EffectPlan> {
        let (repo, request) = request_of(args)?;
        let planned = plan_reset(&repo, &request, &Environment::from_ctx(ctx))?;
        let outcome = planned.outcome();
        let mut plan = EffectPlan::new();
        plan.changes = file_changes(&outcome.repo, &outcome.written, &outcome.removed);
        plan.notable = outcome.notable();
        plan.bulk = planned
            .targets()
            .iter()
            .filter(|target| matches!(target.path, TargetPath::Fs(_)))
            .count()
            > 1;
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
