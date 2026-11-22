// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.merge` command: starting a merge, concluding a stopped merge
//! and aborting it.
//!
//! Every action is planned completely before anything changes:
//! [`plan_merge`] reads the repository, computes the merge in the object
//! database and returns a [`MergePlan`] holding every change of the working
//! tree, the index, the references and the state files. A refused merge
//! (local changes in the way, staged changes, an operation in progress,
//! histories that cannot be fast-forwarded) fails during planning and
//! changes nothing. [`apply_merge`] then writes the plan in one mutation
//! transaction. Merge driver programs never run while planning; when the
//! repository configures one for a conflicting path, [`apply_merge`]
//! computes the merge again with the program before the first write.

use std::collections::BTreeMap;
use std::path::Path;

use bstr::{BString, ByteSlice};
use gix_hash::ObjectId;
use gix_index::entry::Stage;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{ExemptWrite, Expect, MutationTx};
use nurt_contracts::record::{GitConflict, GitEffect, GitEffectState, GitInProgress};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::Path as RecordPath;
use nurt_git::apply::{
    ApplyContext, ApplyPlan, ApplyReport, IndexPlan, RefPlan, RefTarget, ReflogLine, StateFilePlan,
    apply_plan, plan_two_way,
};

use super::abort::plan_reset_merge;
use super::conflict::MergeConflict;
use super::driver::DriverMode;
use super::finish::{MergeCompletion, prepare_completion};
use super::message::{
    Cleanup, clean_message, merge_message, reflog_fast_forward, reflog_merge,
    with_conflicts_section,
};
use super::source::{
    MergeTarget, comment_char, missing_branch, short_branch, suppressed_destinations,
};
use super::trees::{
    ConflictStyle, Favor, MergeLabels, MergeOptions, io_error, merge_bases, merge_commits,
    revision_error, unrelated_error,
};
use crate::state::{self, OpState, StateFile, abs_path, id_content, merge_head_content, quote};

/// Reflog message of `git merge --abort`, which runs `git reset --merge`.
const ABORT_REFLOG: &str = "reset: moving to HEAD";

/// Content of `MERGE_MODE` for a merge that must create a commit.
const MODE_NO_FF: &[u8] = b"no-ff";

/// Action of `git.merge`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum MergeAction {
    /// Start a merge of a revision into the current branch.
    #[default]
    Start,
    /// Conclude a stopped merge with a commit.
    Continue,
    /// Abort a stopped merge and restore the state before it.
    Abort,
}

impl MergeAction {
    /// Parses a value of the `action` parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::merge::MergeAction;
    ///
    /// assert_eq!(MergeAction::parse("abort"), Some(MergeAction::Abort));
    /// assert_eq!(MergeAction::parse("skip"), None);
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "start" => Some(Self::Start),
            "continue" => Some(Self::Continue),
            "abort" => Some(Self::Abort),
            _ => None,
        }
    }

    /// Stable name of the action.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Continue => "continue",
            Self::Abort => "abort",
        }
    }
}

/// Fast-forward policy of a merge (`fast_forward`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum FastForward {
    /// Fast-forward when the current commit is an ancestor of the merged
    /// one, otherwise create a merge commit.
    #[default]
    Auto,
    /// Fast-forward or refuse the merge.
    Only,
    /// Always create a merge commit.
    Never,
}

impl FastForward {
    /// Parses a value of the `fast_forward` parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::merge::FastForward;
    ///
    /// assert_eq!(FastForward::parse("never"), Some(FastForward::Never));
    /// assert_eq!(FastForward::parse("no"), None);
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "only" => Some(Self::Only),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// Stable name of the policy.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Only => "only",
            Self::Never => "never",
        }
    }
}

/// Parameters of a merge action.
#[derive(Debug, Clone)]
pub struct MergeRequest {
    /// Repository as given by the caller, used in the calls of `fix` and
    /// `next_steps`.
    pub repo: String,
    /// Action.
    pub action: MergeAction,
    /// Commit to merge; required by [`MergeAction::Start`].
    pub target: Option<MergeTarget>,
    /// Fast-forward policy.
    pub fast_forward: FastForward,
    /// Message of the merge commit instead of the generated one.
    pub message: Option<String>,
    /// Stop before the merge commit, as after a conflict.
    pub no_commit: bool,
    /// Style of conflict hunks instead of `merge.conflictStyle`.
    pub conflict_style: Option<ConflictStyle>,
    /// Side that wins conflicting hunks (`strategy_option`).
    pub favor: Option<Favor>,
    /// Merge histories without a common ancestor.
    pub allow_unrelated: bool,
    /// Prefix of reflog messages; `None` uses `merge <label>`, as
    /// `git merge` does, and `git.pull` passes `pull`.
    pub reflog_action: Option<String>,
    /// Author of a created commit.
    pub author: gix::actor::Signature,
    /// Committer of a created commit and identity of reflog lines.
    pub committer: gix::actor::Signature,
}

impl MergeRequest {
    /// Request of `action` with default settings; `committer` is also the
    /// author.
    #[must_use]
    pub fn new(
        repo: impl Into<String>,
        action: MergeAction,
        committer: gix::actor::Signature,
    ) -> Self {
        Self {
            repo: repo.into(),
            action,
            target: None,
            fast_forward: FastForward::Auto,
            message: None,
            no_commit: false,
            conflict_style: None,
            favor: None,
            allow_unrelated: false,
            reflog_action: None,
            author: committer.clone(),
            committer,
        }
    }

    /// Request to start merging `target`.
    #[must_use]
    pub fn start(
        repo: impl Into<String>,
        target: MergeTarget,
        committer: gix::actor::Signature,
    ) -> Self {
        let mut request = Self::new(repo, MergeAction::Start, committer);
        request.target = Some(target);
        request
    }
}

/// A merge action planned without changing anything; executed by
/// [`apply_merge`] or shown as a preview.
///
/// Callers that write more in the same transaction, such as `git.pull`
/// with the fetched references, add their reference updates and state
/// files to [`Self::apply`] before calling [`apply_merge`]; every target is
/// then preserved before the first write.
#[derive(Debug, Clone)]
pub struct MergePlan {
    /// Outcome once applied.
    pub state: GitEffectState,
    /// Changes of the working tree, the index, the references and the
    /// state files.
    pub apply: ApplyPlan,
    /// Commit of `HEAD` before; `None` on an unborn branch.
    pub head_before: Option<ObjectId>,
    /// Commit of `HEAD` after.
    pub head_after: Option<ObjectId>,
    /// Full name of the current branch; `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// Commit created by the action.
    pub commit: Option<ObjectId>,
    /// Commit recorded in `MERGE_HEAD` when the merge stops.
    pub merge_head: Option<ObjectId>,
    /// Conflicts left in the working tree.
    pub conflicts: Vec<MergeConflict>,
    /// Unusual effects.
    pub notes: Vec<NotableEntry>,
    objects_written: bool,
    pending_objects: Vec<(gix_object::Kind, Vec<u8>)>,
    drivers_run: Vec<(String, BString)>,
    deferred: Option<Box<MergeRequest>>,
}

impl MergePlan {
    /// Whether configured merge driver programs run when the plan is
    /// applied with an execution context.
    #[must_use]
    pub fn runs_drivers(&self) -> bool {
        self.deferred.is_some()
    }

    fn new(state: GitEffectState, head: &Head) -> Self {
        Self {
            state,
            apply: empty_plan(),
            head_before: head.commit,
            head_after: head.commit,
            branch: head.branch.clone(),
            commit: None,
            merge_head: None,
            conflicts: Vec::new(),
            notes: Vec::new(),
            objects_written: false,
            pending_objects: Vec::new(),
            drivers_run: Vec::new(),
            deferred: None,
        }
    }
}

/// Result of an applied merge action.
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// Outcome.
    pub state: GitEffectState,
    /// Commit of `HEAD` before.
    pub head_before: Option<ObjectId>,
    /// Commit of `HEAD` after.
    pub head_after: Option<ObjectId>,
    /// Full name of the current branch; `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// Commit created by the action.
    pub commit: Option<ObjectId>,
    /// Commit recorded in `MERGE_HEAD` when the merge stopped.
    pub merge_head: Option<ObjectId>,
    /// Conflicts left in the working tree.
    pub conflicts: Vec<MergeConflict>,
    /// Unusual effects of the merge.
    pub notes: Vec<NotableEntry>,
    /// Merge driver programs that ran, by driver name and path.
    pub drivers_run: Vec<(String, BString)>,
    /// Changes made.
    pub report: ApplyReport,
}

impl MergeResult {
    /// The `effect.git` record of the result for the repository at `root`.
    #[must_use]
    pub fn effect(&self, root: &Path) -> GitEffect {
        let stopped = matches!(
            self.state,
            GitEffectState::Conflicts | GitEffectState::Stopped
        );
        GitEffect {
            repo: RecordPath::new(root.display().to_string()),
            state: self.state,
            head_before: self.head_before.map(hex),
            head_after: self.head_after.map(hex),
            branch: self
                .branch
                .as_deref()
                .map(|name| short_branch(name).to_owned()),
            conflicts: (!self.conflicts.is_empty()).then(|| self.records()),
            in_progress: Some(if stopped {
                GitInProgress::Merge
            } else {
                GitInProgress::None
            }),
            merge_head: self.merge_head.map(hex),
            detached: Some(self.branch.is_none()),
            ..GitEffect::default()
        }
    }

    /// `E_GIT_CONFLICT` with the effect applied, when the merge stopped
    /// with conflicts; the calls of `fix` stage the files and the next
    /// steps continue or abort the merge.
    #[must_use]
    pub fn conflict_error(&self, repo: &str) -> Option<Error> {
        if self.conflicts.is_empty() {
            return None;
        }
        let paths: Vec<String> = self
            .conflicts
            .iter()
            .map(|conflict| conflict.path.to_str_lossy().into_owned())
            .collect();
        let mut error = Error::new(
            ErrorCode::GitConflict,
            format!(
                "automatic merge failed with conflicts in {} path(s); resolve them, stage the result and continue the merge",
                paths.len()
            ),
        )
        .with_problem("conflicts")
        .with_effect_applied(true);
        for path in &paths {
            error = error.with_sample(path.clone());
        }
        if let Ok(records) = serde_json::to_value(self.records()) {
            error = error.with_detail("conflicts", records);
        }
        let list = paths
            .iter()
            .map(|path| quote(path))
            .collect::<Vec<_>>()
            .join(", ");
        Some(
            error
                .with_fix(format!(
                    "git.stage {} action: add paths: [{list}]",
                    quote(repo)
                ))
                .with_next_step(format!("git.merge {} action: continue", quote(repo)))
                .with_next_step(format!("git.merge {} action: abort", quote(repo))),
        )
    }

    fn records(&self) -> Vec<GitConflict> {
        self.conflicts
            .iter()
            .map(MergeConflict::to_record)
            .collect()
    }
}

/// Plans `request` without changing anything.
///
/// # Errors
///
/// Fails with `E_GIT_IN_PROGRESS` when another operation is in progress or,
/// for `continue` and `abort`, when no merge is; `E_GIT_CONFLICT` (effect
/// not applied) when the index has unresolved paths; `E_GIT_DIRTY` when
/// staged changes, local changes or untracked files are in the way;
/// `E_GIT_NON_FAST_FORWARD` when `fast_forward` is `only` and the histories
/// diverged, or when they are unrelated; `E_MISSING_PARAM` when `start`
/// has no target; `E_GIT_REVISION` and `E_IO` when the repository cannot be
/// read.
pub fn plan_merge(ctx: &ApplyContext<'_>, request: &MergeRequest) -> Result<MergePlan> {
    match request.action {
        MergeAction::Start => plan_start(ctx, request, DriverMode::Defer),
        MergeAction::Continue => plan_continue(ctx, request),
        MergeAction::Abort => plan_abort(ctx, request),
    }
}

/// Executes `plan` in the transaction `tx`.
///
/// When the plan defers merge driver programs and `exec` is given, the
/// merge is computed again with the programs, through the program
/// launching service of `exec`, before the first write; their effects are
/// marked irreversible. Without `exec` the built-in result stays and the
/// paths are reported as `filter_not_applied`.
///
/// # Errors
///
/// Fails with the errors of [`apply_plan`] (`E_GIT_LOCKED`, `E_STALE` and
/// the errors of the transaction) and, when the merge is computed again,
/// with the errors of [`plan_merge`].
pub fn apply_merge(
    tx: &mut dyn MutationTx,
    ctx: &ApplyContext<'_>,
    mut plan: MergePlan,
    exec: Option<&ExecCtx>,
) -> Result<MergeResult> {
    let deferred = plan.deferred.take();
    let plan = match (deferred, exec) {
        (Some(request), Some(exec)) => {
            let mut again = plan_start(ctx, &request, DriverMode::Run(exec))?;
            again.apply.refs.extend(extra_refs(&plan));
            again.apply.state_files.extend(extra_states(&plan));
            again
        }
        (Some(_), None) => {
            plan.notes.push(not_run_note());
            plan
        }
        (None, _) => plan,
    };
    if plan.objects_written {
        tx.record_exempt(ExemptWrite::ObjectStore {
            repo: abs_path(ctx.repo().common_dir())?,
        });
        persist_objects(ctx.repo(), &plan.pending_objects)?;
    }
    for (name, path) in &plan.drivers_run {
        let mut marker = IrreversibleEntry::new(Irreversible::ExternalEffect);
        marker.path = Some(path.to_str_lossy().into_owned());
        marker.detail = Some(format!("merge driver {name}"));
        tx.mark_irreversible(marker);
    }
    let report = apply_plan(tx, ctx, plan.apply)?;
    Ok(MergeResult {
        state: plan.state,
        head_before: plan.head_before,
        head_after: plan.head_after,
        branch: plan.branch,
        commit: plan.commit,
        merge_head: plan.merge_head,
        conflicts: plan.conflicts,
        notes: plan.notes,
        drivers_run: plan.drivers_run,
        report,
    })
}

/// Reference updates the caller added to a deferred plan beyond those of
/// the merge itself.
fn extra_refs(plan: &MergePlan) -> Vec<RefPlan> {
    plan.apply
        .refs
        .iter()
        .filter(|reference| {
            reference.name != "HEAD" && Some(&reference.name) != plan.branch.as_ref()
        })
        .cloned()
        .collect()
}

fn extra_states(plan: &MergePlan) -> Vec<StateFilePlan> {
    plan.apply
        .state_files
        .iter()
        .filter(|state| !is_merge_state(&state.path))
        .cloned()
        .collect()
}

fn is_merge_state(path: &str) -> bool {
    [
        StateFile::MergeHead,
        StateFile::MergeMsg,
        StateFile::MergeMode,
        StateFile::AutoMerge,
        StateFile::OrigHead,
    ]
    .iter()
    .any(|file| file.file_name() == path)
}

fn not_run_note() -> NotableEntry {
    let mut note = NotableEntry::new(Notable::FilterNotApplied);
    note.detail = Some(
        "a configured merge driver was not run because no execution context was given; the built-in text driver merged the files"
            .to_owned(),
    );
    note
}

struct Head {
    commit: Option<ObjectId>,
    tree: Option<ObjectId>,
    branch: Option<String>,
}

fn read_head(repo: &gix::Repository) -> Result<Head> {
    let branch = repo
        .head_name()
        .map_err(io_error("read HEAD"))?
        .map(|name| name.as_bstr().to_str_lossy().into_owned())
        .filter(|name| name.starts_with(super::source::HEADS));
    let commit = repo
        .head()
        .map_err(io_error("read HEAD"))?
        .id()
        .map(gix::Id::detach);
    let tree = commit.map(|id| tree_of(repo, id)).transpose()?;
    Ok(Head {
        commit,
        tree,
        branch,
    })
}

fn tree_of(repo: &gix::Repository, commit: ObjectId) -> Result<ObjectId> {
    repo.find_commit(commit)
        .map_err(revision_error("read a commit"))?
        .tree_id()
        .map(gix::Id::detach)
        .map_err(revision_error("read the tree of a commit"))
}

fn empty_plan() -> ApplyPlan {
    ApplyPlan {
        entries: Vec::new(),
        index: None,
        refs: Vec::new(),
        state_files: Vec::new(),
        carried: Vec::new(),
        notable: Vec::new(),
    }
}

fn write_state(file: StateFile, content: Vec<u8>) -> StateFilePlan {
    StateFilePlan {
        path: file.file_name().to_owned(),
        content: Some(content),
        expect: Expect::Unchecked,
    }
}

fn hex(id: ObjectId) -> String {
    id.to_hex().to_string()
}

fn plan_start(
    ctx: &ApplyContext<'_>,
    request: &MergeRequest,
    drivers: DriverMode<'_>,
) -> Result<MergePlan> {
    let repo = ctx.repo();
    let target = request
        .target
        .as_ref()
        .ok_or_else(|| missing_branch(&request.repo))?;
    refuse_in_progress(repo, &request.repo)?;
    refuse_unmerged(ctx, &request.repo)?;
    let head = read_head(repo)?;
    let action = request
        .reflog_action
        .clone()
        .unwrap_or_else(|| format!("merge {}", target.label));
    let (Some(head_commit), Some(head_tree)) = (head.commit, head.tree) else {
        return plan_fast_forward(ctx, request, target, &head, &action);
    };
    let bases = merge_bases(repo, head_commit, target.commit)?;
    if target.commit == head_commit || bases.contains(&target.commit) {
        return Ok(plan_up_to_date(&head, head_commit));
    }
    if bases.contains(&head_commit) && request.fast_forward != FastForward::Never {
        return plan_fast_forward(ctx, request, target, &head, &action);
    }
    if request.fast_forward == FastForward::Only {
        return Err(not_fast_forward(&request.repo, &target.label));
    }
    if bases.is_empty() && !request.allow_unrelated {
        return Err(unrelated_error());
    }
    let sides = Sides {
        head: &head,
        head_commit,
        head_tree,
        target,
        action: &action,
    };
    plan_three_way(ctx, request, &sides, drivers)
}

fn refuse_in_progress(repo: &gix::Repository, label: &str) -> Result<()> {
    match state::detect(repo.git_dir())? {
        None | Some(OpState::Bisect) => Ok(()),
        Some(operation) => Err(operation.in_progress_error(label)),
    }
}

fn refuse_unmerged(ctx: &ApplyContext<'_>, label: &str) -> Result<()> {
    let paths = ctx.unmerged_paths();
    if paths.is_empty() {
        return Ok(());
    }
    let mut error = Error::new(
        ErrorCode::GitConflict,
        "merging is not possible because of unmerged files; resolve them and stage the result",
    )
    .with_problem("unmerged_paths")
    .with_effect_applied(false);
    for path in &paths {
        error = error.with_sample(path.to_string());
    }
    let list = paths
        .iter()
        .map(|path| quote(&path.to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(error
        .with_fix(format!(
            "git.stage {} action: add paths: [{list}]",
            quote(label)
        ))
        .with_next_step(format!("git.status {}", quote(label))))
}

fn not_fast_forward(label: &str, revision: &str) -> Error {
    Error::new(
        ErrorCode::GitNonFastForward,
        format!(
            "not possible to fast-forward to {revision}; merge with fast_forward: auto or rebase onto it"
        ),
    )
    .with_problem("not_possible")
    .with_found(revision.to_owned())
    .with_fix(format!(
        "git.merge {} branch: {} fast_forward: auto",
        quote(label),
        quote(revision)
    ))
    .with_fix(format!("git.rebase {} onto: {}", quote(label), quote(revision)))
}

fn with_dirty_fixes(error: Error, label: &str) -> Error {
    if error.code() != ErrorCode::GitDirty {
        return error;
    }
    error
        .with_fix(format!("git.stash {} action: push", quote(label)))
        .with_next_step(format!("git.status {}", quote(label)))
}

fn plan_up_to_date(head: &Head, head_commit: ObjectId) -> MergePlan {
    let mut plan = MergePlan::new(GitEffectState::UpToDate, head);
    plan.apply
        .state_files
        .push(write_state(StateFile::OrigHead, id_content(&head_commit)));
    plan
}

fn plan_fast_forward(
    ctx: &ApplyContext<'_>,
    request: &MergeRequest,
    target: &MergeTarget,
    head: &Head,
    action: &str,
) -> Result<MergePlan> {
    let repo = ctx.repo();
    let target_tree = tree_of(repo, target.commit)?;
    let mut plan = MergePlan::new(GitEffectState::FastForward, head);
    plan.apply = plan_two_way(ctx, head.tree.as_ref().map(|id| id.as_ref()), &target_tree)
        .map_err(|error| with_dirty_fixes(error, &request.repo))?;
    let old = head
        .commit
        .unwrap_or_else(|| ObjectId::null(repo.object_hash()));
    let line = ReflogLine {
        old,
        new: target.commit,
        committer: request.committer.clone(),
        message: reflog_fast_forward(action),
    };
    push_head_refs(repo, &mut plan.apply, head, line)?;
    if let Some(commit) = head.commit {
        plan.apply
            .state_files
            .push(write_state(StateFile::OrigHead, id_content(&commit)));
    }
    plan.head_after = Some(target.commit);
    plan.notes.push(NotableEntry::new(Notable::FastForward));
    Ok(plan)
}

/// Updates the current branch, or the detached `HEAD`, to `line.new`, and
/// adds the line to the logs of both, as Git does for a symbolic `HEAD`.
fn push_head_refs(
    repo: &gix::Repository,
    plan: &mut ApplyPlan,
    head: &Head,
    line: ReflogLine,
) -> Result<()> {
    match &head.branch {
        Some(branch) => {
            plan.refs.push(RefPlan::new(
                repo,
                branch,
                RefTarget::Object(line.new),
                Some(line.clone()),
            )?);
            plan.refs.push(RefPlan::new(
                repo,
                "HEAD",
                RefTarget::Symbolic(branch.clone()),
                Some(line),
            )?);
        }
        None => plan.refs.push(RefPlan::new(
            repo,
            "HEAD",
            RefTarget::Object(line.new),
            Some(line),
        )?),
    }
    Ok(())
}

struct Sides<'a> {
    head: &'a Head,
    head_commit: ObjectId,
    head_tree: ObjectId,
    target: &'a MergeTarget,
    action: &'a str,
}

/// Plans the three-way merge with objects kept in memory, so that a refusal
/// leaves the object database untouched. The objects are stored by
/// [`apply_merge`] inside the transaction.
fn plan_three_way(
    ctx: &ApplyContext<'_>,
    request: &MergeRequest,
    sides: &Sides<'_>,
    drivers: DriverMode<'_>,
) -> Result<MergePlan> {
    let mut memory = ctx.repo().clone().with_object_memory();
    let mut plan = {
        let scratch = ApplyContext::open(&memory)?;
        plan_three_way_in(&scratch, request, sides, drivers)?
    };
    plan.pending_objects = memory
        .objects
        .take_object_memory()
        .map(|storage| {
            storage
                .iter()
                .map(|(_, (kind, data))| (*kind, data.clone()))
                .collect()
        })
        .unwrap_or_default();
    plan.objects_written = !plan.pending_objects.is_empty();
    Ok(plan)
}

fn persist_objects(repo: &gix::Repository, objects: &[(gix_object::Kind, Vec<u8>)]) -> Result<()> {
    for (kind, data) in objects {
        gix_object::Write::write_buf(repo, *kind, data)
            .map_err(io_error("store the merge objects"))?;
    }
    Ok(())
}

fn plan_three_way_in(
    ctx: &ApplyContext<'_>,
    request: &MergeRequest,
    sides: &Sides<'_>,
    drivers: DriverMode<'_>,
) -> Result<MergePlan> {
    let repo = ctx.repo();
    refuse_staged_changes(ctx, sides.head_tree, &request.repo)?;
    let options = merge_options(repo, request, sides.target)?;
    let outcome = merge_commits(
        repo,
        sides.head_commit,
        sides.target.commit,
        &options,
        drivers,
    )?;
    let mut plan = MergePlan::new(GitEffectState::Completed, sides.head);
    plan.apply = plan_two_way(ctx, Some(&sides.head_tree), &outcome.tree)
        .map_err(|error| with_dirty_fixes(error, &request.repo))?;
    plan.notes.extend(outcome.notes.iter().cloned());
    plan.drivers_run.clone_from(&outcome.drivers_run);
    if !outcome.pending_drivers.is_empty() {
        plan.deferred = Some(Box::new(request.clone()));
    }
    plan.apply.state_files.push(write_state(
        StateFile::OrigHead,
        id_content(&sides.head_commit),
    ));
    let message = commit_message(repo, request, sides);
    if outcome.is_clean() && !request.no_commit {
        let parents = [sides.head_commit, sides.target.commit];
        let commit = write_commit(repo, outcome.tree, &parents, &message, request)?;
        let line = ReflogLine {
            old: sides.head_commit,
            new: commit,
            committer: request.committer.clone(),
            message: reflog_merge(sides.action),
        };
        push_head_refs(repo, &mut plan.apply, sides.head, line)?;
        plan.commit = Some(commit);
        plan.head_after = Some(commit);
        return Ok(plan);
    }
    let merge_msg = if outcome.is_clean() {
        plan.state = GitEffectState::Stopped;
        message
    } else {
        plan.state = GitEffectState::Conflicts;
        let paths: Vec<String> = outcome
            .conflicted_paths()
            .iter()
            .map(|path| path.to_str_lossy().into_owned())
            .collect();
        let planned = plan.apply.index.take();
        plan.apply.index = Some(conflict_index(ctx, planned, &outcome.index));
        with_conflicts_section(&message, paths, comment_char(repo))
    };
    let mode = if request.fast_forward == FastForward::Never {
        MODE_NO_FF.to_vec()
    } else {
        Vec::new()
    };
    plan.apply.state_files.extend([
        write_state(
            StateFile::MergeHead,
            merge_head_content(&[sides.target.commit]),
        ),
        write_state(StateFile::MergeMsg, merge_msg.into_bytes()),
        write_state(StateFile::MergeMode, mode),
        write_state(StateFile::AutoMerge, id_content(&outcome.tree)),
    ]);
    plan.merge_head = Some(sides.target.commit);
    plan.conflicts = outcome.conflicts;
    Ok(plan)
}

fn merge_options(
    repo: &gix::Repository,
    request: &MergeRequest,
    target: &MergeTarget,
) -> Result<MergeOptions> {
    let mut options = MergeOptions::from_repo(repo)?;
    if let Some(style) = request.conflict_style {
        options.conflict_style = style;
    }
    options.favor = request.favor;
    options.allow_unrelated = request.allow_unrelated;
    options.labels = MergeLabels::head_and(target.label.clone());
    Ok(options)
}

fn commit_message(repo: &gix::Repository, request: &MergeRequest, sides: &Sides<'_>) -> String {
    match &request.message {
        Some(text) => clean_message(text, Cleanup::Whitespace, comment_char(repo)),
        None => merge_message(
            &sides.target.source,
            sides.head.branch.as_deref().map(short_branch),
            &suppressed_destinations(repo),
        ),
    }
}

/// Fails with `E_GIT_DIRTY` when the index differs from the tree of
/// `HEAD`: a three-way merge would overwrite staged changes.
fn refuse_staged_changes(ctx: &ApplyContext<'_>, head_tree: ObjectId, label: &str) -> Result<()> {
    let repo = ctx.repo();
    let head = gix_index::State::from_tree(&head_tree, &repo.objects, Default::default())
        .map_err(revision_error("read the tree of HEAD"))?;
    let expected = entry_map(&head);
    let current = entry_map(ctx.index());
    let mut changed: Vec<String> = expected
        .iter()
        .filter(|(path, entry)| current.get(*path) != Some(*entry))
        .map(|(path, _)| path.to_string())
        .chain(
            current
                .keys()
                .filter(|path| !expected.contains_key(*path))
                .map(ToString::to_string),
        )
        .collect();
    if changed.is_empty() {
        return Ok(());
    }
    changed.sort();
    let mut error = Error::new(
        ErrorCode::GitDirty,
        format!(
            "staged changes in {} path(s) would be overwritten by the merge; commit or stash them first",
            changed.len()
        ),
    )
    .with_problem("staged_changes");
    for path in changed {
        error = error.with_sample(path);
    }
    Err(with_dirty_fixes(error, label))
}

fn entry_map(index: &gix_index::State) -> BTreeMap<BString, (ObjectId, u32)> {
    index
        .entries()
        .iter()
        .filter(|entry| entry.stage() == Stage::Unconflicted)
        .map(|entry| (entry.path(index).to_owned(), (entry.id, entry.mode.bits())))
        .collect()
}

/// Index of a merge stopped with conflicts: the entries of the merge
/// result with their stages, keeping the file status of unchanged entries
/// of the planned index so that they are not read again.
fn conflict_index(
    ctx: &ApplyContext<'_>,
    planned: Option<IndexPlan>,
    merged: &gix_index::State,
) -> IndexPlan {
    let (known, expect) = match planned {
        Some(plan) => (plan.state, plan.expect),
        None => (ctx.index().clone(), ctx.index_expect().clone()),
    };
    let mut state = merged.clone();
    let paths: Vec<BString> = state
        .entries()
        .iter()
        .map(|entry| entry.path(&state).to_owned())
        .collect();
    for (entry, path) in state.entries_mut().iter_mut().zip(paths) {
        if entry.stage() != Stage::Unconflicted {
            continue;
        }
        if let Some(old) = known.entry_by_path_and_stage(path.as_bstr(), Stage::Unconflicted)
            && old.id == entry.id
            && old.mode == entry.mode
        {
            entry.stat = old.stat;
            entry.flags = old.flags;
        }
    }
    IndexPlan {
        state,
        expect,
        record_resolve_undo: false,
    }
}

fn write_commit(
    repo: &gix::Repository,
    tree: ObjectId,
    parents: &[ObjectId],
    message: &str,
    request: &MergeRequest,
) -> Result<ObjectId> {
    let commit = gix_object::Commit {
        tree,
        parents: parents.iter().copied().collect(),
        author: request.author.clone(),
        committer: request.committer.clone(),
        encoding: None,
        message: message.into(),
        extra_headers: Vec::new(),
    };
    repo.write_object(&commit)
        .map(gix::Id::detach)
        .map_err(io_error("write the merge commit"))
}

fn no_merge_error(label: &str, action: MergeAction) -> Error {
    Error::new(
        ErrorCode::GitInProgress,
        format!(
            "there is no merge to {} because MERGE_HEAD does not exist; start a merge first",
            action.as_str()
        ),
    )
    .with_problem("no_merge_in_progress")
    .with_fix(format!("git.merge {} branch: \"<name>\"", quote(label)))
    .with_next_step(format!("git.status {}", quote(label)))
}

fn unborn_error() -> Error {
    Error::new(
        ErrorCode::GitRef,
        "HEAD has no commit; a stopped merge needs the commit it started from",
    )
    .with_problem("unborn_branch")
}

fn plan_continue(ctx: &ApplyContext<'_>, request: &MergeRequest) -> Result<MergePlan> {
    let repo = ctx.repo();
    let completion = prepare_completion(repo.git_dir(), ctx.index(), &request.repo)?
        .ok_or_else(|| no_merge_error(&request.repo, MergeAction::Continue))?;
    let head = read_head(repo)?;
    let head_commit = head.commit.ok_or_else(unborn_error)?;
    let message = completion.message(
        request.message.as_deref(),
        Cleanup::Strip,
        comment_char(repo),
    );
    if message.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the merge message is empty; give message: \"<text>\"",
        )
        .with_problem("empty_message"));
    }
    let tree = index_tree(repo, ctx.index())?;
    let mut parents = vec![head_commit];
    parents.extend(completion.parents.iter().copied());
    let commit = write_commit(repo, tree, &parents, &message, request)?;
    let mut plan = MergePlan::new(GitEffectState::Completed, &head);
    let line = ReflogLine {
        old: head_commit,
        new: commit,
        committer: request.committer.clone(),
        message: MergeCompletion::reflog_message(&message),
    };
    push_head_refs(repo, &mut plan.apply, &head, line)?;
    plan.apply.state_files.extend(
        completion
            .state_changes
            .iter()
            .map(state::StateChange::to_plan),
    );
    plan.objects_written = true;
    plan.commit = Some(commit);
    plan.head_after = Some(commit);
    Ok(plan)
}

/// Tree of the stage 0 entries of `index`, as `git write-tree` writes it.
fn index_tree(repo: &gix::Repository, index: &gix_index::State) -> Result<ObjectId> {
    let mut editor =
        gix_object::tree::Editor::new(gix_object::Tree::empty(), &repo.objects, repo.object_hash());
    for entry in index.entries() {
        if entry.stage() != Stage::Unconflicted {
            continue;
        }
        let Some(kind) = tree_kind(entry.mode) else {
            continue;
        };
        editor
            .upsert(
                entry.path(index).split(|byte| *byte == b'/'),
                kind,
                entry.id,
            )
            .map_err(io_error("build the tree of the index"))?;
    }
    editor
        .write(|tree| repo.write_object(tree).map(gix::Id::detach))
        .map_err(io_error("write the tree of the index"))
}

fn tree_kind(mode: gix_index::entry::Mode) -> Option<gix_object::tree::EntryKind> {
    use gix_index::entry::Mode;
    use gix_object::tree::EntryKind;
    [
        (Mode::FILE, EntryKind::Blob),
        (Mode::FILE_EXECUTABLE, EntryKind::BlobExecutable),
        (Mode::SYMLINK, EntryKind::Link),
        (Mode::COMMIT, EntryKind::Commit),
    ]
    .into_iter()
    .find(|(known, _)| *known == mode)
    .map(|(_, kind)| kind)
}

fn plan_abort(ctx: &ApplyContext<'_>, request: &MergeRequest) -> Result<MergePlan> {
    let repo = ctx.repo();
    if state::read_merge_state(repo.git_dir())?.is_none() {
        return Err(no_merge_error(&request.repo, MergeAction::Abort));
    }
    let head = read_head(repo)?;
    let (Some(head_commit), Some(head_tree)) = (head.commit, head.tree) else {
        return Err(unborn_error());
    };
    let mut plan = MergePlan::new(GitEffectState::Completed, &head);
    plan.apply = plan_reset_merge(ctx, &head_tree, &request.repo)?;
    let line = ReflogLine {
        old: head_commit,
        new: head_commit,
        committer: request.committer.clone(),
        message: ABORT_REFLOG.to_owned(),
    };
    let target = match &head.branch {
        Some(branch) => RefTarget::Symbolic(branch.clone()),
        None => RefTarget::Object(head_commit),
    };
    plan.apply
        .refs
        .push(RefPlan::new(repo, "HEAD", target, Some(line))?);
    plan.apply
        .state_files
        .push(write_state(StateFile::OrigHead, id_content(&head_commit)));
    plan.apply.state_files.extend(
        state::removals(repo.git_dir(), StateFile::BRANCH_STATE)?
            .iter()
            .map(state::StateChange::to_plan),
    );
    Ok(plan)
}
