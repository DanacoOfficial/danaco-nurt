// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Planning and execution of the `git.rebase` actions.
//!
//! [`plan_rebase`] reads the repository, replays the commits in the object
//! database and returns a [`RebasePlan`] holding every change of the working
//! tree, the index, the references, the reference logs and the state files.
//! A refused action fails during planning and changes nothing.
//! [`apply_rebase`] then writes the plan in one mutation transaction: the
//! working tree goes from its current state to the final one in one pass.

use std::collections::BTreeMap;
use std::path::Path;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use gix_index::entry::Stage;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{
    ExemptWrite, Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec,
};
use nurt_contracts::record::{GitConflict, GitEffect, GitEffectState, GitInProgress, GitStep};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::value::Path as RecordPath;
use nurt_git::apply::{
    ApplyContext, ApplyPlan, ApplyReport, IndexPlan, RefPlan, RefTarget, ReflogLine, ResetOptions,
    StateFilePlan, apply_plan, keeps_log, plan_reset, plan_two_way,
};

use crate::merge::{Cleanup, MergeConflict, clean_message, merge_bases, with_conflicts_section};
use crate::state::{self, OpState, StateFile, abs_path, id_content, quote};

use super::error;
use super::history;
use super::objects::{self, NewCommit};
use super::patch;
use super::sequencer::{AutoMerge, HeadLine, Progress, Replay, Replayed, Stop, UntrackedGuard};
use super::state_dir::{self, AuthorScript, RebaseState};

/// Prefix of the reference log messages of a rebase, unless the caller sets
/// another one, as `git.pull` does.
pub const DEFAULT_REFLOG_ACTION: &str = "rebase";

/// Prefix of branch names.
const HEADS: &str = "refs/heads/";

/// Files of the state directory that only a stop for conflicts has.
const CONFLICT_FILES: [&str; 3] = ["message", "patch", "stopped-sha"];

/// Hooks Git runs during a rebase; the console runs none of them.
const HOOKS: [&str; 2] = ["pre-rebase", "post-rewrite"];

/// Comment character of commit messages unless `core.commentChar` sets one.
const DEFAULT_COMMENT: char = '#';

/// Action of `git.rebase`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RebaseAction {
    /// Replay the commits of the current branch onto another revision.
    #[default]
    Start,
    /// Commit the resolved stopped commit and replay the rest.
    Continue,
    /// Drop the stopped commit and replay the rest.
    Skip,
    /// Return to the state before the rebase.
    Abort,
}

impl RebaseAction {
    /// Parses a value of the `action` parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::rebase::RebaseAction;
    ///
    /// assert_eq!(RebaseAction::parse("skip"), Some(RebaseAction::Skip));
    /// assert_eq!(RebaseAction::parse("edit"), None);
    /// ```
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "start" => Some(Self::Start),
            "continue" => Some(Self::Continue),
            "skip" => Some(Self::Skip),
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
            Self::Skip => "skip",
            Self::Abort => "abort",
        }
    }
}

/// Commit to rebase onto, with the text that names it in the reference log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebaseOnto {
    /// Commit.
    pub commit: ObjectId,
    /// Revision as given, written after `checkout` in the reference log.
    pub label: String,
}

impl RebaseOnto {
    /// Resolves `revision` to a commit of `repo`; `repo_label` is the
    /// repository as given by the caller, used in the calls of `fix`.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_REVISION` when the revision names no commit.
    pub fn resolve(repo: &gix::Repository, repo_label: &str, revision: &str) -> Result<Self> {
        let id = repo
            .rev_parse_single(revision)
            .map_err(|cause| error::revision(repo_label, revision, cause))?;
        let commit = id
            .object()
            .map_err(|cause| error::revision(repo_label, revision, cause))?
            .peel_to_commit()
            .map_err(|cause| error::revision(repo_label, revision, cause))?;
        Ok(Self {
            commit: commit.id,
            label: revision.to_owned(),
        })
    }
}

/// Parameters of a rebase action.
#[derive(Debug, Clone)]
pub struct RebaseRequest {
    /// Repository as given by the caller, used in the calls of `fix` and
    /// `next_steps`.
    pub repo: String,
    /// Action.
    pub action: RebaseAction,
    /// Commit to rebase onto; required by [`RebaseAction::Start`].
    pub onto: Option<RebaseOnto>,
    /// Commit whose history is not replayed; `None` uses [`Self::onto`].
    /// `git.pull` passes the fork point of the upstream branch.
    pub upstream: Option<ObjectId>,
    /// Prefix of reference log messages; `None` uses
    /// [`DEFAULT_REFLOG_ACTION`].
    pub reflog_action: Option<String>,
    /// Committer of replayed commits and identity of reference log lines.
    pub committer: gix::actor::Signature,
}

impl RebaseRequest {
    /// Request of `action` without a commit to rebase onto.
    #[must_use]
    pub fn new(
        repo: impl Into<String>,
        action: RebaseAction,
        committer: gix::actor::Signature,
    ) -> Self {
        Self {
            repo: repo.into(),
            action,
            onto: None,
            upstream: None,
            reflog_action: None,
            committer,
        }
    }

    /// Request to start a rebase onto `onto`.
    #[must_use]
    pub fn start(
        repo: impl Into<String>,
        onto: RebaseOnto,
        committer: gix::actor::Signature,
    ) -> Self {
        let mut request = Self::new(repo, RebaseAction::Start, committer);
        request.onto = Some(onto);
        request
    }

    fn reflog_action(&self) -> String {
        self.reflog_action
            .clone()
            .unwrap_or_else(|| DEFAULT_REFLOG_ACTION.to_owned())
    }
}

/// Position of a stopped rebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RebaseStep {
    /// Commands run, including the stopped one.
    pub done: usize,
    /// Commands in total.
    pub total: usize,
}

/// Fields shared by a plan and its result.
#[derive(Debug, Clone)]
pub struct RebaseSummary {
    /// Outcome.
    pub state: GitEffectState,
    /// Commit of `HEAD` before.
    pub head_before: Option<ObjectId>,
    /// Commit of `HEAD` after.
    pub head_after: Option<ObjectId>,
    /// Full name of the rebased branch; `None` for a detached `HEAD`.
    pub branch: Option<String>,
    /// Commits written.
    pub commits: Vec<ObjectId>,
    /// Commits dropped because their change is already in the new base.
    pub dropped: Vec<ObjectId>,
    /// Position of a stop.
    pub step: Option<RebaseStep>,
    /// Commit at which the rebase stopped, as `<short> (<subject>)`.
    pub stopped_at: Option<String>,
    /// Conflicts left in the working tree.
    pub conflicts: Vec<MergeConflict>,
    /// Untracked files that stopped the rebase.
    pub blocked: Vec<BString>,
    /// Unusual effects.
    pub notes: Vec<NotableEntry>,
}

impl RebaseSummary {
    fn new(state: GitEffectState, head: Option<ObjectId>, branch: Option<String>) -> Self {
        Self {
            state,
            head_before: head,
            head_after: head,
            branch,
            commits: Vec::new(),
            dropped: Vec::new(),
            step: None,
            stopped_at: None,
            conflicts: Vec::new(),
            blocked: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// The `effect.git` record for the repository at `root`.
    #[must_use]
    pub fn effect(&self, root: &Path) -> GitEffect {
        let stopped = self.is_stopped();
        GitEffect {
            repo: RecordPath::new(root.display().to_string()),
            state: self.state,
            head_before: self.head_before.map(hex),
            head_after: self.head_after.map(hex),
            branch: self.branch.as_deref().map(short_branch),
            conflicts: (!self.conflicts.is_empty()).then(|| self.records()),
            in_progress: Some(if stopped {
                GitInProgress::Rebase
            } else {
                GitInProgress::None
            }),
            step: self.step.map(|step| GitStep {
                done: count(step.done),
                total: count(step.total),
            }),
            detached: Some(stopped || self.branch.is_none()),
            ..GitEffect::default()
        }
    }

    /// Error reported when the rebase stopped: `E_GIT_CONFLICT` for
    /// conflicts and `E_GIT_DIRTY` for untracked files in the way, both
    /// with the effect applied; `None` when it did not stop.
    #[must_use]
    pub fn stop_error(&self, repo: &str) -> Option<Error> {
        let label = self.stopped_at.clone().unwrap_or_default();
        let (error, paths) = match self.state {
            GitEffectState::Conflicts => {
                let paths: Vec<String> = self.conflicts.iter().map(|c| text(&c.path)).collect();
                let error = Error::new(
                    ErrorCode::GitConflict,
                    format!(
                        "could not apply {label}: {} path(s) have conflicts; resolve them, stage the result and continue the rebase",
                        paths.len()
                    ),
                )
                .with_problem("conflicts");
                (error, paths)
            }
            GitEffectState::Stopped => {
                let error = Error::new(
                    ErrorCode::GitDirty,
                    format!(
                        "could not apply {label}: untracked files would be overwritten; move or remove them and continue the rebase"
                    ),
                )
                .with_problem("untracked_overwritten");
                (error, self.blocked.iter().map(text).collect())
            }
            _ => return None,
        };
        Some(self.decorate(error.with_effect_applied(true), &paths, repo))
    }

    fn decorate(&self, mut error: Error, paths: &[String], repo: &str) -> Error {
        for path in paths {
            error = error.with_sample(path.clone());
        }
        if !self.conflicts.is_empty() {
            if let Ok(records) = serde_json::to_value(self.records()) {
                error = error.with_detail("conflicts", records);
            }
            let list = paths
                .iter()
                .map(|path| quote(path))
                .collect::<Vec<_>>()
                .join(", ");
            error = error.with_fix(format!(
                "git.stage {} action: add paths: [{list}]",
                quote(repo)
            ));
        }
        if let Some(step) = self.step {
            error = error.with_detail(
                "step",
                serde_json::json!({ "done": step.done, "total": step.total }),
            );
        }
        error
            .with_next_step(format!("git.rebase {} action: continue", quote(repo)))
            .with_next_step(format!("git.rebase {} action: skip", quote(repo)))
            .with_next_step(format!("git.rebase {} action: abort", quote(repo)))
    }

    fn is_stopped(&self) -> bool {
        matches!(
            self.state,
            GitEffectState::Conflicts | GitEffectState::Stopped
        )
    }

    fn records(&self) -> Vec<GitConflict> {
        self.conflicts
            .iter()
            .map(MergeConflict::to_record)
            .collect()
    }
}

/// A rebase action planned without changing the working tree, the index or
/// the references; executed by [`apply_rebase`] or shown as a preview.
///
/// Planning writes the replayed commits and merged trees to the object
/// database, which the journal exempts. Callers that write more in the same
/// transaction, such as `git.pull` with the fetched references, add their
/// reference updates and state files to [`Self::apply`] before calling
/// [`apply_rebase`].
#[derive(Debug, Clone)]
pub struct RebasePlan {
    /// Outcome once applied.
    pub summary: RebaseSummary,
    /// Changes of the working tree, the index, the references and the
    /// state files.
    pub apply: ApplyPlan,
    remove_state_dir: bool,
    objects_written: bool,
}

/// Result of an applied rebase action.
#[derive(Debug, Clone)]
pub struct RebaseResult {
    /// Outcome.
    pub summary: RebaseSummary,
    /// Changes made.
    pub report: ApplyReport,
}

/// Plans `request` without changing the working tree, the index or the
/// references.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` when `start` has no commit to rebase onto;
/// `E_GIT_IN_PROGRESS` when another operation is in progress or, for
/// `continue`, `skip` and `abort`, when no rebase is; `E_GIT_CONFLICT`
/// (effect not applied) when the index has unresolved paths; `E_GIT_DIRTY`
/// for uncommitted changes or untracked files in the way; `E_GIT_REF` on an
/// unborn branch; `E_GIT_SIGN` when `commit.gpgSign` is set;
/// `E_GIT_UNSUPPORTED` for `rebase.updateRefs`, `rebase.autoStash` with
/// changes, an interactive todo list or a rebase of the `apply` back end;
/// `E_GIT_REVISION` and `E_IO` when the repository cannot be read.
pub fn plan_rebase(ctx: &ApplyContext<'_>, request: &RebaseRequest) -> Result<RebasePlan> {
    match request.action {
        RebaseAction::Start => plan_start(ctx, request),
        RebaseAction::Continue => plan_continue(ctx, request),
        RebaseAction::Skip => plan_skip(ctx, request),
        RebaseAction::Abort => plan_abort(ctx, request),
    }
}

/// Executes `plan` in the transaction `tx`: every target is preserved
/// before the first change, and the state directory `rebase-merge/` is
/// removed as one target when the rebase ends.
///
/// # Errors
///
/// Fails with the errors of [`apply_plan`] (`E_GIT_LOCKED`, `E_STALE` and
/// the errors of the transaction).
pub fn apply_rebase(
    tx: &mut dyn MutationTx,
    ctx: &ApplyContext<'_>,
    plan: RebasePlan,
) -> Result<RebaseResult> {
    let RebasePlan {
        summary,
        apply,
        remove_state_dir,
        objects_written,
    } = plan;
    let repo = ctx.repo();
    let state_dir = if remove_state_dir {
        let spec = TargetSpec {
            path: TargetPath::Fs(abs_path(&repo.git_dir().join(state_dir::DIR))?),
            action: TargetAction::Delete,
            expect: Expect::Unchecked,
            size_hint: None,
        };
        tx.preserve(&[spec])?.first().map(|item| item.index)
    } else {
        None
    };
    if objects_written {
        tx.record_exempt(ExemptWrite::ObjectStore {
            repo: abs_path(repo.common_dir())?,
        });
    }
    let report = apply_plan(tx, ctx, apply)?;
    if let Some(index) = state_dir {
        tx.apply(index, Mutation::Remove)?;
    }
    Ok(RebaseResult { summary, report })
}

/// The rebase being planned.
struct Session {
    /// Content of `head-name`: the branch or `detached HEAD`.
    head_name: String,
    onto: ObjectId,
    orig_head: ObjectId,
    /// Commit of `HEAD` before the action.
    head: ObjectId,
    /// `HEAD` names a commit, not a branch.
    head_detached: bool,
    end: usize,
    done_text: BString,
    done_count: usize,
    rewritten_text: BString,
    /// Files of the state directory before the action.
    files: BTreeMap<String, Vec<u8>>,
    /// Content of `git-rebase-todo.backup` for a starting rebase.
    backup: Option<Vec<u8>>,
}

impl Session {
    fn branch(&self) -> Option<&str> {
        (self.head_name != state_dir::DETACHED).then_some(self.head_name.as_str())
    }

    fn of_state(repo: &gix::Repository, state: &RebaseState, head: ObjectId) -> Result<Self> {
        let head_detached = repo
            .head_name()
            .map_err(|cause| error::io("read HEAD", cause))?
            .is_none();
        Ok(Self {
            head_name: state.head_name.clone(),
            onto: state.onto,
            orig_head: state.orig_head,
            head,
            head_detached,
            end: state.end,
            done_text: state.done.clone(),
            done_count: state.done_count,
            rewritten_text: state.rewritten.clone(),
            files: state.files.clone(),
            backup: None,
        })
    }
}

/// How the working tree reaches the final state.
enum Base {
    /// Two-tree update from the tree the index matches.
    TwoWay(ObjectId),
    /// Reset that discards conflicts, as `git rebase --skip`.
    Reset,
}

struct Head {
    commit: Option<ObjectId>,
    branch: Option<String>,
}

fn read_head(repo: &gix::Repository) -> Result<Head> {
    let branch = repo
        .head_name()
        .map_err(|cause| error::io("read HEAD", cause))?
        .map(|name| name.as_bstr().to_str_lossy().into_owned())
        .filter(|name| name.starts_with(HEADS));
    let commit = repo
        .head()
        .map_err(|cause| error::io("read HEAD", cause))?
        .id()
        .map(gix::Id::detach);
    Ok(Head { commit, branch })
}

fn plan_start(ctx: &ApplyContext<'_>, request: &RebaseRequest) -> Result<RebasePlan> {
    let repo = ctx.repo();
    let label = request.repo.as_str();
    let onto = request
        .onto
        .as_ref()
        .ok_or_else(|| error::missing_onto(label))?;
    refuse_in_progress(repo, label)?;
    refuse_settings(repo, label)?;
    let head = read_head(repo)?;
    let head_commit = head.commit.ok_or_else(|| error::unborn(label))?;
    refuse_unmerged(ctx, label)?;
    let head_tree = objects::tree_of(repo, head_commit)?;
    refuse_dirty(ctx, head_tree, label)?;
    let upstream = request.upstream.unwrap_or(onto.commit);
    if is_up_to_date(repo, onto.commit, upstream, head_commit)? {
        return Ok(up_to_date(head_commit, head.branch));
    }
    let selection = history::select(repo, head_commit, upstream)?;
    let onto_tree = objects::tree_of(repo, onto.commit)?;
    let mut guard = UntrackedGuard::new(repo, ctx.workdir(), ctx.index())?;
    let blocked = guard.collisions(head_tree, onto_tree)?;
    if !blocked.is_empty() {
        return Err(error::untracked(label, &blocked));
    }
    let action = request.reflog_action();
    let mut progress = Progress::new(head_commit, head_tree);
    progress.advance(
        onto.commit,
        onto_tree,
        format!("{action} (start): checkout {}", onto.label),
    );
    let short = |id| objects::short(repo, id);
    let range = format!("{}..{}", short(upstream), short(head_commit));
    let session = Session {
        head_name: head
            .branch
            .clone()
            .unwrap_or_else(|| state_dir::DETACHED.to_owned()),
        onto: onto.commit,
        orig_head: head_commit,
        head: head_commit,
        head_detached: head.branch.is_none(),
        end: selection.items.len(),
        done_text: BString::default(),
        done_count: 0,
        rewritten_text: BString::default(),
        files: BTreeMap::new(),
        backup: Some(state_dir::todo_backup(
            &selection.items,
            &range,
            &short(onto.commit),
        )),
    };
    let mut replay = Replay::new(repo, request.committer.clone(), action, guard)?;
    let replayed = replay.run(progress, selection.items)?;
    let mut plan = conclude(ctx, request, &session, replayed, Base::TwoWay(head_tree))?;
    plan.summary.dropped.splice(0..0, selection.skipped);
    Ok(plan)
}

fn up_to_date(head: ObjectId, branch: Option<String>) -> RebasePlan {
    RebasePlan {
        summary: RebaseSummary::new(GitEffectState::UpToDate, Some(head), branch),
        apply: empty_plan(),
        remove_state_dir: false,
        objects_written: false,
    }
}

fn plan_continue(ctx: &ApplyContext<'_>, request: &RebaseRequest) -> Result<RebasePlan> {
    let repo = ctx.repo();
    let label = request.repo.as_str();
    let state = read_state(repo, label, RebaseAction::Continue)?;
    refuse_signing(repo, label)?;
    refuse_unmerged(ctx, label)?;
    let unstaged = unstaged_paths(repo)?;
    if !unstaged.is_empty() {
        return Err(error::unstaged_on_continue(label, &unstaged));
    }
    let head_commit = read_head(repo)?
        .commit
        .ok_or_else(|| error::unborn(label))?;
    let head_tree = objects::tree_of(repo, head_commit)?;
    let index_tree = index_tree(repo, ctx.index())?;
    let session = Session::of_state(repo, &state, head_commit)?;
    let guard = UntrackedGuard::new(repo, ctx.workdir(), ctx.index())?;
    let mut replay = Replay::new(
        repo,
        request.committer.clone(),
        request.reflog_action(),
        guard,
    )?;
    let mut progress = Progress::new(head_commit, head_tree);
    if index_tree != head_tree {
        commit_resolution(repo, &state, &replay, &mut progress, index_tree, request)?;
    }
    let replayed = replay.run(progress, state.todo.clone())?;
    conclude(ctx, request, &session, replayed, Base::TwoWay(index_tree))
}

/// Commits the resolved index with the author and the message of the
/// stopped commit, as `git rebase --continue` does after a conflict.
fn commit_resolution(
    repo: &gix::Repository,
    state: &RebaseState,
    replay: &Replay<'_>,
    progress: &mut Progress,
    tree: ObjectId,
    request: &RebaseRequest,
) -> Result<()> {
    let author = state.author.as_ref().ok_or_else(|| {
        error::corrupt(
            "the stopped rebase has no rebase-merge/author-script",
            "the author of the stopped commit is unknown",
        )
    })?;
    let original = state
        .stopped
        .map(|id| objects::read_commit(repo, id))
        .transpose()?;
    let raw = state
        .message
        .clone()
        .or_else(|| original.as_ref().map(|commit| commit.message.clone()))
        .unwrap_or_default();
    let message = clean_message(&raw.to_str_lossy(), Cleanup::Strip, comment_char(repo));
    if message.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the message of the stopped commit is empty; write a message into rebase-merge/message",
        )
        .with_problem("empty_message"));
    }
    let header = author.header();
    let new = objects::write_commit(
        repo,
        &NewCommit {
            tree,
            parent: progress.head,
            author: header.as_bstr(),
            encoding: original
                .as_ref()
                .and_then(|commit| commit.encoding.as_ref())
                .map(|encoding| encoding.as_bstr()),
            message: message.as_bytes(),
        },
        &request.committer,
    )?;
    let subject = objects::first_line(message.as_bytes().as_bstr());
    progress.advance(new, tree, replay.line("continue", subject));
    if let Some(stopped) = state.stopped {
        progress.rewritten.push((stopped, new));
    }
    progress.created.push(new);
    progress.auto_merge = AutoMerge::Remove;
    Ok(())
}

fn plan_skip(ctx: &ApplyContext<'_>, request: &RebaseRequest) -> Result<RebasePlan> {
    let repo = ctx.repo();
    let label = request.repo.as_str();
    let state = read_state(repo, label, RebaseAction::Skip)?;
    refuse_signing(repo, label)?;
    let head_commit = read_head(repo)?
        .commit
        .ok_or_else(|| error::unborn(label))?;
    let head_tree = objects::tree_of(repo, head_commit)?;
    let session = Session::of_state(repo, &state, head_commit)?;
    let guard = UntrackedGuard::new(repo, ctx.workdir(), ctx.index())?;
    let mut replay = Replay::new(
        repo,
        request.committer.clone(),
        request.reflog_action(),
        guard,
    )?;
    let mut progress = Progress::new(head_commit, head_tree);
    progress.auto_merge = AutoMerge::Remove;
    let replayed = replay.run(progress, state.todo.clone())?;
    conclude(ctx, request, &session, replayed, Base::Reset)
}

fn plan_abort(ctx: &ApplyContext<'_>, request: &RebaseRequest) -> Result<RebasePlan> {
    let repo = ctx.repo();
    let label = request.repo.as_str();
    let state = read_state(repo, label, RebaseAction::Abort)?;
    let head = read_head(repo)?.commit.unwrap_or(state.orig_head);
    let orig_tree = objects::tree_of(repo, state.orig_head)?;
    let branch = state.branch().map(str::to_owned);
    let mut summary = RebaseSummary::new(GitEffectState::Completed, Some(head), branch.clone());
    summary.head_after = Some(state.orig_head);
    let mut apply = plan_reset(
        ctx,
        orig_tree.as_ref(),
        ResetOptions {
            overwrite_untracked: true,
        },
    )?;
    let (target, returning) = match &branch {
        Some(name) => (RefTarget::Symbolic(name.clone()), name.clone()),
        None => (RefTarget::Object(state.orig_head), hex(state.orig_head)),
    };
    apply.refs.push(RefPlan::new(repo, "HEAD", target, None)?);
    for file in [
        StateFile::AutoMerge,
        StateFile::RebaseHead,
        StateFile::MergeMsg,
    ] {
        apply.state_files.push(remove_state(file));
    }
    let line = HeadLine {
        old: head,
        new: state.orig_head,
        message: format!(
            "{} (abort): returning to {returning}",
            request.reflog_action()
        ),
    };
    push_head_log(repo, &request.committer, &[line], &mut apply)?;
    Ok(RebasePlan {
        summary,
        apply,
        remove_state_dir: true,
        objects_written: false,
    })
}

/// Turns a replay into the plan of the working tree, the index, the
/// references and the state files.
fn conclude(
    ctx: &ApplyContext<'_>,
    request: &RebaseRequest,
    session: &Session,
    replayed: Replayed,
    base: Base,
) -> Result<RebasePlan> {
    let repo = ctx.repo();
    let Replayed {
        progress,
        stop,
        todo,
    } = replayed;
    let target = match &stop {
        Some(Stop::Conflict { outcome, .. }) => outcome.tree,
        _ => progress.tree,
    };
    let apply = match base {
        Base::TwoWay(from) => plan_two_way(ctx, Some(from.as_ref()), target.as_ref()),
        Base::Reset => plan_reset(
            ctx,
            target.as_ref(),
            ResetOptions {
                overwrite_untracked: false,
            },
        ),
    }
    .map_err(|error| with_dirty_fixes(error, &request.repo))?;
    let mut plan = RebasePlan {
        summary: RebaseSummary::new(
            GitEffectState::Completed,
            Some(session.head),
            session.branch().map(str::to_owned),
        ),
        apply,
        remove_state_dir: false,
        objects_written: true,
    };
    if session.backup.is_some() {
        plan.apply.state_files.push(write_state(
            StateFile::OrigHead,
            id_content(&session.orig_head),
        ));
    }
    let mut lines = progress.lines.clone();
    match &stop {
        None => finish(repo, request, session, &progress, &mut plan, &mut lines)?,
        Some(stop) => stop_at(ctx, session, &progress, stop, &todo, &mut plan)?,
    }
    push_head_log(repo, &request.committer, &lines, &mut plan.apply)?;
    plan.summary.head_after = Some(progress.head);
    plan.summary.commits = progress.created;
    plan.summary.dropped = progress.dropped;
    plan.summary.notes.extend(hook_notes(repo));
    Ok(plan)
}

/// Completes the rebase: moves the branch, returns `HEAD` to it and
/// removes the state.
fn finish(
    repo: &gix::Repository,
    request: &RebaseRequest,
    session: &Session,
    progress: &Progress,
    plan: &mut RebasePlan,
    lines: &mut Vec<HeadLine>,
) -> Result<()> {
    let action = request.reflog_action();
    let apply = &mut plan.apply;
    match session.branch() {
        Some(branch) => {
            let line = ReflogLine {
                old: session.orig_head,
                new: progress.head,
                committer: request.committer.clone(),
                message: format!("{action} (finish): {branch} onto {}", hex(session.onto)),
            };
            apply.refs.push(RefPlan::new(
                repo,
                branch,
                RefTarget::Object(progress.head),
                Some(line),
            )?);
            if session.head_detached {
                apply.refs.push(RefPlan::new(
                    repo,
                    "HEAD",
                    RefTarget::Symbolic(branch.to_owned()),
                    None,
                )?);
            }
            lines.push(HeadLine {
                old: progress.head,
                new: progress.head,
                message: format!("{action} (finish): returning to {branch}"),
            });
        }
        None => apply.refs.push(RefPlan::new(
            repo,
            "HEAD",
            RefTarget::Object(progress.head),
            None,
        )?),
    }
    if progress.attempted {
        apply.state_files.push(remove_state(StateFile::RebaseHead));
    }
    match progress.auto_merge {
        AutoMerge::Keep => {}
        AutoMerge::Set(tree) => apply
            .state_files
            .push(write_state(StateFile::AutoMerge, id_content(&tree))),
        AutoMerge::Remove => apply.state_files.push(remove_state(StateFile::AutoMerge)),
    }
    apply.state_files.push(remove_state(StateFile::MergeMsg));
    plan.remove_state_dir = !session.files.is_empty();
    let fast_forward = session.backup.is_some() && progress.done.is_empty();
    let summary = &mut plan.summary;
    if fast_forward {
        summary.state = GitEffectState::FastForward;
        summary.notes.push(NotableEntry::new(Notable::FastForward));
    } else if session.branch().is_some() {
        summary
            .notes
            .push(NotableEntry::new(Notable::HistoryRewritten));
    }
    Ok(())
}

/// Stops the rebase: detaches `HEAD` at the last replayed commit and writes
/// the state directory, as Git leaves it for `git rebase --continue`.
fn stop_at(
    ctx: &ApplyContext<'_>,
    session: &Session,
    progress: &Progress,
    stop: &Stop,
    todo: &[history::TodoItem],
    plan: &mut RebasePlan,
) -> Result<()> {
    let repo = ctx.repo();
    let item = stop.item();
    let commit = stop.commit();
    plan.apply.refs.push(RefPlan::new(
        repo,
        "HEAD",
        RefTarget::Object(progress.head),
        None,
    )?);
    let mut files = dir_files(session, progress, todo);
    files.insert(
        "author-script".to_owned(),
        AuthorScript::from_header(commit.author.as_bstr())?.render(),
    );
    plan.apply
        .state_files
        .push(write_state(StateFile::RebaseHead, id_content(&item.id)));
    match stop {
        Stop::Conflict {
            parent_tree,
            outcome,
            ..
        } => {
            let paths: Vec<String> = outcome.conflicted_paths().iter().map(text).collect();
            let message =
                with_conflicts_section(&commit.message.to_str_lossy(), &paths, comment_char(repo));
            let short = |id| objects::short(repo, id);
            files.insert("message".to_owned(), message.clone().into_bytes());
            files.insert(
                "patch".to_owned(),
                patch::patch_text(repo, &short, *parent_tree, commit.tree)?,
            );
            files.insert("stopped-sha".to_owned(), state_dir::object_line(item.id));
            plan.apply.state_files.extend([
                write_state(StateFile::MergeMsg, message.into_bytes()),
                write_state(StateFile::AutoMerge, id_content(&outcome.tree)),
            ]);
            let planned = plan.apply.index.take();
            plan.apply.index = Some(conflict_index(ctx, planned, &outcome.index));
            plan.summary.state = GitEffectState::Conflicts;
            plan.summary.conflicts.clone_from(&outcome.conflicts);
        }
        Stop::Untracked { paths, .. } => {
            plan.apply.state_files.extend([
                remove_state(StateFile::MergeMsg),
                remove_state(StateFile::AutoMerge),
            ]);
            plan.summary.state = GitEffectState::Stopped;
            plan.summary.blocked.clone_from(paths);
        }
    }
    dir_changes(&session.files, files, &mut plan.apply);
    plan.summary.step = Some(RebaseStep {
        done: session.done_count + progress.done.len(),
        total: session.end,
    });
    plan.summary.stopped_at = Some(format!(
        "{} ({})",
        objects::short(repo, item.id),
        commit.subject().to_str_lossy()
    ));
    Ok(())
}

/// Files of the state directory of a stopped rebase, other than those of
/// the stopped commit.
fn dir_files(
    session: &Session,
    progress: &Progress,
    todo: &[history::TodoItem],
) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut put = |name: &str, content: Vec<u8>| {
        files.insert(name.to_owned(), content);
    };
    put("head-name", format!("{}\n", session.head_name).into_bytes());
    put("onto", state_dir::object_line(session.onto));
    put("orig-head", state_dir::object_line(session.orig_head));
    put("end", state_dir::number(session.end));
    put(
        "msgnum",
        state_dir::number(session.done_count + progress.done.len()),
    );
    let mut done = session.done_text.to_vec();
    done.extend_from_slice(&state_dir::todo_text(&progress.done));
    put("done", done);
    put("git-rebase-todo", state_dir::todo_text(todo));
    for marker in state_dir::MARKERS {
        put(marker, Vec::new());
    }
    if let Some(backup) = &session.backup {
        put("git-rebase-todo.backup", backup.clone());
    }
    let mut rewritten = session.rewritten_text.to_vec();
    for (old, new) in &progress.rewritten {
        rewritten.extend_from_slice(format!("{old} {new}\n").as_bytes());
    }
    if !rewritten.is_empty() {
        put("rewritten-list", rewritten);
    }
    files
}

/// Writes the files of `wanted` that differ from `existing` and removes the
/// files of a previous stop for conflicts that are no longer wanted.
fn dir_changes(
    existing: &BTreeMap<String, Vec<u8>>,
    wanted: BTreeMap<String, Vec<u8>>,
    apply: &mut ApplyPlan,
) {
    for name in CONFLICT_FILES {
        if existing.contains_key(name) && !wanted.contains_key(name) {
            apply.state_files.push(StateFilePlan {
                path: format!("{}/{name}", state_dir::DIR),
                content: None,
                expect: Expect::Unchecked,
            });
        }
    }
    for (name, content) in wanted {
        if existing.get(&name) != Some(&content) {
            apply.state_files.push(StateFilePlan {
                path: format!("{}/{name}", state_dir::DIR),
                content: Some(content),
                expect: Expect::Unchecked,
            });
        }
    }
}

/// Appends `lines` to the log of `HEAD` when Git keeps one.
///
/// The log is planned as a state file with its complete new content, so
/// that its directory is created only once together with the logs of the
/// references of the same plan.
fn push_head_log(
    repo: &gix::Repository,
    committer: &gix::actor::Signature,
    lines: &[HeadLine],
    apply: &mut ApplyPlan,
) -> Result<()> {
    if lines.is_empty() || !keeps_log(repo, "HEAD")? {
        return Ok(());
    }
    let path = repo.git_dir().join("logs").join("HEAD");
    let mut content = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(cause) => return Err(error::io(&format!("read {}", path.display()), cause)),
    };
    for line in lines {
        content.extend_from_slice(&reflog_line(line, committer)?);
    }
    apply.state_files.push(StateFilePlan {
        path: "logs/HEAD".to_owned(),
        content: Some(content),
        expect: Expect::Unchecked,
    });
    Ok(())
}

/// One reference log line in the format of Git; white space of the message
/// is folded into single spaces.
fn reflog_line(line: &HeadLine, committer: &gix::actor::Signature) -> Result<Vec<u8>> {
    let mut out = format!("{} {} ", line.old, line.new).into_bytes();
    committer
        .write_to(&mut out)
        .map_err(|cause| error::io("format a reflog line", cause))?;
    out.push(b'\t');
    out.extend_from_slice(
        line.message
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .as_bytes(),
    );
    out.push(b'\n');
    Ok(out)
}

fn read_state(repo: &gix::Repository, label: &str, action: RebaseAction) -> Result<RebaseState> {
    match state::detect(repo.git_dir())? {
        Some(OpState::Rebase { .. }) => {}
        Some(OpState::RebaseApply) => {
            return Err(error::unsupported(
                "apply_backend",
                "the rebase in progress uses the apply back end, which git.rebase cannot continue; abort it with git or finish it with git",
            ));
        }
        _ => return Err(error::no_rebase(label, action.as_str())),
    }
    let resolve = |name: &str| -> Result<(ObjectId, String)> {
        let id = repo
            .rev_parse_single(name)
            .map_err(|cause| error::corrupt(&format!("the todo list names {name}"), cause))?
            .detach();
        Ok((id, objects::read_commit(repo, id)?.oneline()))
    };
    state_dir::read(&repo.git_dir().join(state_dir::DIR), &resolve)?
        .ok_or_else(|| error::no_rebase(label, action.as_str()))
}

fn refuse_in_progress(repo: &gix::Repository, label: &str) -> Result<()> {
    match state::detect(repo.git_dir())? {
        None | Some(OpState::Bisect) => Ok(()),
        Some(operation) => Err(operation.in_progress_error(label)),
    }
}

fn refuse_settings(repo: &gix::Repository, label: &str) -> Result<()> {
    if repo.config_snapshot().boolean("rebase.updateRefs") == Some(true) {
        return Err(error::unsupported(
            "update_refs",
            "rebase.updateRefs is set, but git.rebase does not move other branches; set rebase.updateRefs to false for this repository",
        ));
    }
    refuse_signing(repo, label)
}

fn refuse_signing(repo: &gix::Repository, label: &str) -> Result<()> {
    if repo.config_snapshot().boolean("commit.gpgSign") == Some(true) {
        return Err(error::signing(label));
    }
    Ok(())
}

fn refuse_unmerged(ctx: &ApplyContext<'_>, label: &str) -> Result<()> {
    let paths = ctx.unmerged_paths();
    if paths.is_empty() {
        Ok(())
    } else {
        Err(error::unmerged(label, &paths))
    }
}

/// Fails with `E_GIT_DIRTY` for staged or unstaged changes; with
/// `rebase.autoStash` set the changes would have to be stashed, which is
/// `E_GIT_UNSUPPORTED`.
fn refuse_dirty(ctx: &ApplyContext<'_>, head_tree: ObjectId, label: &str) -> Result<()> {
    let staged = staged_paths(ctx, head_tree)?;
    let found = if staged.is_empty() {
        let unstaged = unstaged_paths(ctx.repo())?;
        (!unstaged.is_empty()).then(|| error::dirty(label, "unstaged_changes", &unstaged))
    } else {
        Some(error::dirty(label, "staged_changes", &staged))
    };
    let Some(found) = found else {
        return Ok(());
    };
    if ctx.repo().config_snapshot().boolean("rebase.autoStash") == Some(true) {
        return Err(error::unsupported(
            "autostash",
            "rebase.autoStash is set, but git.rebase does not stash changes; stash them with git.stash first",
        )
        .with_fix(format!("git.stash {} action: push", quote(label))));
    }
    Err(found)
}

/// Paths whose index entry differs from the tree `head_tree`.
fn staged_paths(ctx: &ApplyContext<'_>, head_tree: ObjectId) -> Result<Vec<BString>> {
    let repo = ctx.repo();
    let head = gix_index::State::from_tree(&head_tree, &repo.objects, Default::default())
        .map_err(|cause| error::io("read the tree of HEAD", cause))?;
    let expected = entry_map(&head);
    let current = entry_map(ctx.index());
    let mut changed: Vec<BString> = expected
        .iter()
        .filter(|(path, entry)| current.get(*path) != Some(*entry))
        .map(|(path, _)| path.clone())
        .chain(
            current
                .keys()
                .filter(|path| !expected.contains_key(*path))
                .cloned(),
        )
        .collect();
    changed.sort();
    Ok(changed)
}

fn entry_map(index: &gix_index::State) -> BTreeMap<BString, (ObjectId, u32)> {
    index
        .entries()
        .iter()
        .filter(|entry| entry.stage() == Stage::Unconflicted)
        .map(|entry| (entry.path(index).to_owned(), (entry.id, entry.mode.bits())))
        .collect()
}

/// Tracked paths whose working tree file differs from the index.
fn unstaged_paths(repo: &gix::Repository) -> Result<Vec<BString>> {
    use gix::status::index_worktree::Item;
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
    const WHAT: &str = "read the status of the working tree";
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(|cause| error::io(WHAT, cause))?
        .into_index_worktree_iter(Vec::<BString>::new())
        .map_err(|cause| error::io(WHAT, cause))?;
    let mut out = Vec::new();
    for item in iter {
        if let Item::Modification {
            rela_path,
            status: EntryStatus::Change(change),
            ..
        } = item.map_err(|cause| error::io(WHAT, cause))?
        {
            // `git rebase` ignores submodules when it looks for unstaged changes.
            if !matches!(change, Change::SubmoduleModification(_)) {
                out.push(rela_path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Whether `head` already contains `onto` on a line of history without
/// merges, which Git reports as up to date.
fn is_up_to_date(
    repo: &gix::Repository,
    onto: ObjectId,
    upstream: ObjectId,
    head: ObjectId,
) -> Result<bool> {
    if upstream != onto || merge_bases(repo, onto, head)?.as_slice() != [onto] {
        return Ok(false);
    }
    history::is_linear(repo, head, onto)
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
            .map_err(|cause| error::io("build the tree of the index", cause))?;
    }
    editor
        .write(|tree| repo.write_object(tree).map(gix::Id::detach))
        .map_err(|cause| error::io("write the tree of the index", cause))
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

/// Index of a rebase stopped with conflicts: the entries of the merge
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

fn with_dirty_fixes(error: Error, label: &str) -> Error {
    if error.code() != ErrorCode::GitDirty {
        return error;
    }
    error
        .with_fix(format!("git.stash {} action: push", quote(label)))
        .with_next_step(format!("git.status {}", quote(label)))
}

/// `hooks_skipped` for each rebase hook installed in the repository.
fn hook_notes(repo: &gix::Repository) -> Vec<NotableEntry> {
    let dir = repo.common_dir().join("hooks");
    HOOKS
        .iter()
        .filter(|name| dir.join(name).is_file())
        .map(|name| {
            let mut note = NotableEntry::new(Notable::HooksSkipped);
            note.detail = Some(format!("the {name} hook was not run"));
            note
        })
        .collect()
}

/// Comment character of commit messages (`core.commentChar`).
fn comment_char(repo: &gix::Repository) -> char {
    let snapshot = repo.config_snapshot();
    let value = snapshot
        .string("core.commentChar")
        .map(|value| value.to_str_lossy().into_owned());
    comment_char_of(value.as_deref())
}

/// Comment character for a value of `core.commentChar`: `auto` and an unset
/// value select the default, every other non-blank character is itself.
fn comment_char_of(value: Option<&str>) -> char {
    value
        .filter(|text| *text != "auto")
        .and_then(|text| text.chars().next())
        .filter(|c| !c.is_whitespace())
        .unwrap_or(DEFAULT_COMMENT)
}

fn write_state(file: StateFile, content: Vec<u8>) -> StateFilePlan {
    StateFilePlan {
        path: file.file_name().to_owned(),
        content: Some(content),
        expect: Expect::Unchecked,
    }
}

/// Removal of a state file; nothing happens when it does not exist.
fn remove_state(file: StateFile) -> StateFilePlan {
    StateFilePlan {
        path: file.file_name().to_owned(),
        content: None,
        expect: Expect::Unchecked,
    }
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

fn hex(id: ObjectId) -> String {
    id.to_hex().to_string()
}

fn text(path: &impl AsRef<[u8]>) -> String {
    String::from_utf8_lossy(path.as_ref()).into_owned()
}

fn short_branch(name: &str) -> String {
    name.strip_prefix(HEADS).unwrap_or(name).to_owned()
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_parse_their_names() {
        for action in [
            RebaseAction::Start,
            RebaseAction::Continue,
            RebaseAction::Skip,
            RebaseAction::Abort,
        ] {
            assert_eq!(RebaseAction::parse(action.as_str()), Some(action));
        }
        assert_eq!(RebaseAction::parse("edit"), None);
    }

    #[test]
    fn state_directory_changes_remove_files_of_an_earlier_conflict() {
        let existing: BTreeMap<String, Vec<u8>> = [
            ("message".to_owned(), b"m\n".to_vec()),
            ("end".to_owned(), b"2\n".to_vec()),
        ]
        .into_iter()
        .collect();
        let wanted: BTreeMap<String, Vec<u8>> = [
            ("end".to_owned(), b"2\n".to_vec()),
            ("msgnum".to_owned(), b"1\n".to_vec()),
        ]
        .into_iter()
        .collect();
        let mut apply = empty_plan();
        dir_changes(&existing, wanted, &mut apply);
        let changes: Vec<(String, bool)> = apply
            .state_files
            .iter()
            .map(|file| (file.path.clone(), file.content.is_some()))
            .collect();
        assert_eq!(
            changes,
            vec![
                ("rebase-merge/message".to_owned(), false),
                ("rebase-merge/msgnum".to_owned(), true)
            ]
        );
    }

    #[test]
    fn branch_names_lose_their_prefix() {
        assert_eq!(short_branch("refs/heads/topic"), "topic");
        assert_eq!(short_branch("HEAD"), "HEAD");
        assert_eq!(text(&BString::from("a/b")), "a/b");
        assert_eq!(count(3), 3);
    }

    #[test]
    fn stop_errors_name_the_stopped_commit_and_the_next_steps() {
        let mut summary = RebaseSummary::new(GitEffectState::Stopped, None, None);
        summary.stopped_at = Some("abc1234 (t1)".to_owned());
        summary.blocked = vec![BString::from("t1.txt")];
        summary.step = Some(RebaseStep { done: 1, total: 3 });
        let error = summary.stop_error("/r").expect("stop error");
        assert_eq!(error.code(), ErrorCode::GitDirty);
        assert!(error.data().effect_applied);
        assert!(error.data().message.contains("abc1234 (t1)"));
        assert_eq!(error.data().sample, vec!["t1.txt".to_owned()]);
        assert_eq!(error.data().next_steps.len(), 3);
        summary.state = GitEffectState::Completed;
        assert!(summary.stop_error("/r").is_none());
    }

    #[test]
    fn effect_of_a_stop_reports_the_rebase_in_progress() {
        let id = ObjectId::from_bytes_or_panic(&[7; 20]);
        let mut summary = RebaseSummary::new(
            GitEffectState::Conflicts,
            Some(id),
            Some("refs/heads/t".to_owned()),
        );
        summary.step = Some(RebaseStep { done: 2, total: 3 });
        let effect = summary.effect(Path::new("/r"));
        assert_eq!(effect.in_progress, Some(GitInProgress::Rebase));
        assert_eq!(effect.branch.as_deref(), Some("t"));
        assert_eq!(effect.detached, Some(true));
        assert_eq!(
            effect.step.map(|step| (step.done, step.total)),
            Some((2, 3))
        );
    }

    #[test]
    fn reflog_lines_fold_white_space() {
        let id = ObjectId::from_bytes_or_panic(&[1; 20]);
        let committer = gix::actor::Signature {
            name: "C".into(),
            email: "c@x".into(),
            time: gix::date::Time::new(1_700_000_100, 0),
        };
        let line = HeadLine {
            old: id,
            new: id,
            message: "rebase (pick):  two\twords ".to_owned(),
        };
        let text = String::from_utf8(reflog_line(&line, &committer).unwrap_or_default())
            .unwrap_or_default();
        assert!(text.ends_with(" C <c@x> 1700000100 +0000\trebase (pick): two words\n"));
    }

    #[test]
    fn comment_character_follows_git() {
        assert_eq!(comment_char_of(None), DEFAULT_COMMENT);
        assert_eq!(comment_char_of(Some("auto")), DEFAULT_COMMENT);
        assert_eq!(comment_char_of(Some(";")), ';');
        assert_eq!(comment_char_of(Some("a")), 'a');
        assert_eq!(comment_char_of(Some("ż")), 'ż');
        assert_eq!(comment_char_of(Some(" ")), DEFAULT_COMMENT);
        assert_eq!(comment_char_of(Some("")), DEFAULT_COMMENT);
    }
}
