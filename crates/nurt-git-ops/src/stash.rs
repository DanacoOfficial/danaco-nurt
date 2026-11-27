// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::stash
// Responsibility:   The `git.stash` command.
// Dependencies:     gix, nurt_contracts, nurt_git, sha2
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.stash` command.
//!
//! `push` saves the changes of the index and the working tree as a stash
//! entry with the commit structure git uses and resets the saved paths;
//! `apply` and `pop` merge an entry back; `drop` and `clear` remove entries
//! by rewriting the reflog of `refs/stash`; `list` and `show` read. Every
//! change of the repository directory and the working tree goes through one
//! journal transaction; new objects are added to the object store, which
//! the journal exempts.

pub(crate) mod apply;
pub(crate) mod entries;
pub(crate) mod push;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use gix::hash::ObjectId;
use nurt_contracts::error::{ErrorCode, Result, Stage};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{GitConflict, GitEffect, GitEffectState, GitStash};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{self, Instant, Record};
use nurt_git::repo::GitRepo;
use sha2::{Digest, Sha256};

pub use self::entries::{StashEntry, StashFile, StashFileChange};
use crate::reset::execute::{Change, ref_tx};
use crate::reset::ident::Identity;
use crate::reset::repo::{self, error, root_literal};
use crate::reset::{args, effect_record, file_changes};

/// Nanoseconds in a second.
const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// Operation of `git.stash`.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::stash::StashAction;
///
/// let action: StashAction = "pop".parse().unwrap_or_default();
/// assert_eq!(action, StashAction::Pop);
/// assert!(StashAction::List.is_read());
/// assert!(!StashAction::Drop.is_read());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum StashAction {
    /// Save the changes as a new entry and reset them.
    #[default]
    Push,
    /// Apply an entry and remove it when the merge is clean.
    Pop,
    /// Apply an entry and keep it.
    Apply,
    /// List the entries.
    List,
    /// Show the files an entry changes.
    Show,
    /// Remove an entry.
    Drop,
    /// Remove all entries.
    Clear,
}

/// Names of the actions in the order of the catalog.
const ACTIONS: [&str; 7] = ["push", "pop", "apply", "list", "show", "drop", "clear"];

impl StashAction {
    /// Name of the action as in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pop => "pop",
            Self::Apply => "apply",
            Self::List => "list",
            Self::Show => "show",
            Self::Drop => "drop",
            Self::Clear => "clear",
        }
    }

    /// Whether the action only reads the repository.
    #[must_use]
    pub const fn is_read(self) -> bool {
        matches!(self, Self::List | Self::Show)
    }
}

impl FromStr for StashAction {
    type Err = nurt_contracts::error::Error;

    /// Parses an action name.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_VALUE` for a name that is not an action.
    fn from_str(text: &str) -> Result<Self> {
        match text {
            "push" => Ok(Self::Push),
            "pop" => Ok(Self::Pop),
            "apply" => Ok(Self::Apply),
            "list" => Ok(Self::List),
            "show" => Ok(Self::Show),
            "drop" => Ok(Self::Drop),
            "clear" => Ok(Self::Clear),
            other => Err(error(
                ErrorCode::UnknownValue,
                format!(
                    "unknown stash action `{other}`; use one of {}",
                    ACTIONS.join(", ")
                ),
            )
            .with_expected(serde_json::json!(ACTIONS))),
        }
    }
}

/// Parameters of a stash operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StashRequest {
    /// Operation.
    pub action: StashAction,
    /// Entry for `pop`, `apply`, `show` and `drop`: `stash@{N}`, `N` or a
    /// commit; the newest entry when `None`.
    pub entry: Option<String>,
    /// Message of a new entry.
    pub message: Option<String>,
    /// `push` also saves untracked files.
    pub include_untracked: bool,
    /// `push` saves only these files or directories.
    pub paths: Vec<PathBuf>,
    /// `push` leaves out paths matching these patterns.
    pub exclude: Vec<String>,
    /// Token of [`state_token`]; the operation fails with `E_STALE` when
    /// the repository changed since the token was read.
    pub expect_state: Option<String>,
}

impl StashRequest {
    /// Request of `action` with every option at its default.
    #[must_use]
    pub fn new(action: StashAction) -> Self {
        Self {
            action,
            ..Self::default()
        }
    }
}

/// Result of a stash operation.
#[derive(Debug, Clone, Default)]
pub struct StashOutcome {
    /// Root of the working tree.
    pub repo: PathBuf,
    /// Operation.
    pub action: StashAction,
    /// Commit of `HEAD`.
    pub head: Option<ObjectId>,
    /// Current branch, `None` when `HEAD` is detached.
    pub branch: Option<String>,
    /// Entry created by `push`.
    pub created: Option<ObjectId>,
    /// Entry applied by `apply` or `pop`.
    pub applied: Option<ObjectId>,
    /// Entry removed by `pop` or `drop`.
    pub dropped: Option<ObjectId>,
    /// Number of entries removed by `clear`.
    pub cleared: usize,
    /// `pop` stopped with conflicts and kept the entry.
    pub kept: bool,
    /// Conflicts left by `apply` or `pop`.
    pub conflicts: Vec<GitConflict>,
    /// There was nothing to save or to remove.
    pub nothing_to_do: bool,
    /// Changed files `push` left out because of `exclude`.
    pub excluded: usize,
    /// Working tree paths written, relative to the root.
    pub written: Vec<String>,
    /// Working tree paths removed, relative to the root.
    pub removed: Vec<String>,
    /// Unusual effects of the merge.
    pub notable: Vec<NotableEntry>,
}

/// A planned stash operation: the changes to make and the outcome they
/// have.
pub struct PlannedStash {
    repo: GitRepo,
    change: Change,
    outcome: StashOutcome,
}

impl std::fmt::Debug for PlannedStash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlannedStash")
            .field("targets", &self.change.target_count())
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

impl PlannedStash {
    fn new(repo: GitRepo, (change, outcome): (Change, StashOutcome)) -> Self {
        Self {
            repo,
            change,
            outcome,
        }
    }

    /// Number of journal targets the operation declares.
    #[must_use]
    pub fn target_count(&self) -> usize {
        self.change.target_count()
    }

    /// The outcome the operation will have.
    #[must_use]
    pub fn outcome(&self) -> &StashOutcome {
        &self.outcome
    }

    /// Performs the operation through `tx`.
    ///
    /// # Errors
    ///
    /// Fails with the errors of the journal after the transaction was rolled
    /// back: `E_STALE` when a target changed after the plan,
    /// `E_GIT_LOCKED` when a git lock file appeared and `E_IO`.
    pub fn apply(self, tx: &mut dyn MutationTx) -> Result<StashOutcome> {
        self.change.run(&self.repo, tx)?;
        Ok(self.outcome)
    }
}

/// Stash entries of the repository containing `path`, newest first.
///
/// # Errors
///
/// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not in a repository
/// with a working tree and `E_IO` when the stash cannot be read.
pub fn list(path: &Path, ctx: &ExecCtx) -> Result<Vec<StashEntry>> {
    entries::list(&repo::open(path, ctx)?)
}

/// Files changed by the entry `entry` (the newest when `None`), sorted by
/// path.
///
/// # Errors
///
/// Fails with `E_GIT_REF` when the stash is empty, `E_GIT_REVISION` when
/// the entry does not exist or is not a stash commit and `E_IO`.
pub fn show(path: &Path, entry: Option<&str>, ctx: &ExecCtx) -> Result<Vec<StashFile>> {
    let repo = repo::open(path, ctx)?;
    let (_, id) = entries::resolve(&repo, entry)?;
    entries::files(&repo, id)
}

fn token_of(repo: &gix::Repository) -> Result<String> {
    let mut hasher = Sha256::new();
    for file in [repo.git_dir().join("HEAD"), repo.index_path()] {
        hasher.update(std::fs::read(&file).unwrap_or_default());
        hasher.update([0]);
    }
    if let Ok(head) = repo.head_id() {
        hasher.update(head.as_bytes());
    }
    if let Some(stash) = entries::read_ref(repo, entries::STASH_REF)? {
        hasher.update(stash.as_bytes());
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Token of the state `git.stash` depends on: `HEAD`, the index and the
/// newest stash entry. Passed back as `expect_state`, it makes an operation
/// fail with `E_STALE` when any of them changed in between.
///
/// # Errors
///
/// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not a repository and
/// `E_IO` when the stash reference cannot be read.
pub fn state_token(path: &Path) -> Result<String> {
    let repo = gix::open_opts(path, nurt_git::repo::open_options()).map_err(|cause| {
        error(
            ErrorCode::GitNotRepository,
            format!("{} is not a repository: {cause}", path.display()),
        )
    })?;
    token_of(&repo)
}

fn check_state(repo: &GitRepo, expected: &str) -> Result<()> {
    if token_of(repo.gix())? == expected {
        return Ok(());
    }
    Err(error(
        ErrorCode::Stale,
        "the repository changed since the state token was read; read the state again",
    )
    .with_stage(Stage::Plan)
    .with_problem("state_changed")
    .with_fix(format!("git.stash {} action: list", root_literal(repo))))
}

fn entry_position(repo: &GitRepo, entry: Option<&str>) -> Result<usize> {
    let (position, id) = entries::resolve(repo, entry)?;
    position.ok_or_else(|| {
        error(
            ErrorCode::GitRef,
            format!(
                "{} is not a stash entry; drop takes entries of the list",
                id.to_hex()
            ),
        )
        .with_problem("not_a_stash_reference")
        .with_fix(format!("git.stash {} action: list", root_literal(repo)))
    })
}

fn base_outcome(repo: &GitRepo, action: StashAction) -> Result<StashOutcome> {
    let head = repo::head(repo)?;
    Ok(StashOutcome {
        repo: repo.root().to_path_buf(),
        action,
        head: head.id,
        branch: head.short_branch(),
        ..StashOutcome::default()
    })
}

fn plan_drop(
    repo: &GitRepo,
    request: &StashRequest,
    identity: &Identity,
) -> Result<(Change, StashOutcome)> {
    let position = entry_position(repo, request.entry.as_deref())?;
    let mut change = Change::new();
    let mut refs = ref_tx(repo, &identity.committer)?;
    let id = entries::plan_drop(repo, &mut refs, &mut change.writes, position)?;
    change.set_refs(refs)?;
    let mut outcome = base_outcome(repo, StashAction::Drop)?;
    outcome.dropped = Some(id);
    Ok((change, outcome))
}

fn plan_clear(repo: &GitRepo, identity: &Identity) -> Result<(Change, StashOutcome)> {
    let mut change = Change::new();
    let mut refs = ref_tx(repo, &identity.committer)?;
    let count = entries::plan_clear(repo, &mut refs)?;
    change.set_refs(refs)?;
    let mut outcome = base_outcome(repo, StashAction::Clear)?;
    outcome.cleared = count;
    outcome.nothing_to_do = change.is_empty();
    Ok((change, outcome))
}

/// Plans a changing stash operation on the repository containing `path`.
///
/// Nothing in the working tree, the index or the references is changed;
/// `push` and `apply` add objects to the object store. The result is applied
/// with [`PlannedStash::apply`].
///
/// # Errors
///
/// Fails with `E_GIT_NOT_REPOSITORY` outside a repository with a working
/// tree, `E_STALE` when `expect_state` does not match, `E_GIT_REF` on an
/// unborn branch, for an empty stash and for `pop` or `drop` of a commit
/// outside the list, `E_GIT_REVISION` for an entry that does not exist,
/// `E_NO_MATCH` when a path of `paths` names nothing, `E_GIT_IN_PROGRESS`
/// when the index has conflicts, `E_GIT_DIRTY` when local changes or
/// untracked files would be overwritten, `E_GIT_LOCKED`, `E_IO`, and
/// `E_INTERNAL` for `list` and `show`, which do not change anything.
pub fn plan(path: &Path, request: &StashRequest, ctx: &ExecCtx) -> Result<PlannedStash> {
    let repo = repo::open(path, ctx)?;
    if let Some(expected) = &request.expect_state {
        check_state(&repo, expected)?;
    }
    let identity = Identity::resolve(&repo);
    let planned = match request.action {
        StashAction::Push => push::plan_push(&repo, request, &identity)?,
        StashAction::Apply => apply::plan_apply(&repo, request, false, &identity)?,
        StashAction::Pop => apply::plan_apply(&repo, request, true, &identity)?,
        StashAction::Drop => plan_drop(&repo, request, &identity)?,
        StashAction::Clear => plan_clear(&repo, &identity)?,
        StashAction::List | StashAction::Show => {
            return Err(error(
                ErrorCode::Internal,
                format!(
                    "stash action {} only reads; run it as a read",
                    request.action.as_str()
                ),
            ));
        }
    };
    Ok(PlannedStash::new(repo, planned))
}

impl StashOutcome {
    fn notable(&self) -> Vec<NotableEntry> {
        let mut notable = self.notable.clone();
        if !self.conflicts.is_empty() {
            let mut entry = NotableEntry::new(Notable::Conflicts);
            entry.count = u64::try_from(self.conflicts.len()).ok();
            notable.push(entry);
        }
        if self.kept {
            notable.push(NotableEntry::new(Notable::StashKept));
        }
        if self.excluded > 0 {
            let mut entry = NotableEntry::new(Notable::Excluded);
            entry.count = u64::try_from(self.excluded).ok();
            notable.push(entry);
        }
        notable
    }

    fn git_effect(&self) -> GitEffect {
        let state = if !self.conflicts.is_empty() {
            GitEffectState::Conflicts
        } else if self.nothing_to_do {
            GitEffectState::NothingToDo
        } else {
            GitEffectState::Completed
        };
        let head = self.head.map(|id| id.to_hex().to_string());
        GitEffect {
            repo: value::Path::new(self.repo.to_string_lossy().into_owned()),
            state,
            head_before: head.clone(),
            head_after: head,
            branch: self.branch.clone(),
            conflicts: (!self.conflicts.is_empty()).then(|| self.conflicts.clone()),
            detached: self.branch.is_none().then_some(true),
            ..GitEffect::default()
        }
    }

    fn conflict_error(&self) -> nurt_contracts::error::Error {
        let root = nurt_git::repo::error::quote_path(&self.repo);
        let paths: Vec<String> = self.conflicts.iter().map(|c| c.path.clone()).collect();
        let kept = if self.kept { "; the entry is kept" } else { "" };
        let mut err = error(
            ErrorCode::GitConflict,
            format!(
                "the stash entry conflicts in {} path(s){kept}; resolve them, then stage the result",
                paths.len()
            ),
        )
        .with_problem("conflicts")
        .with_effect_applied(true)
        .with_next_step(format!("git.status {root}"))
        .with_detail("conflicts", serde_json::Value::from(paths.clone()));
        if self.kept {
            err = err.with_next_step(format!("git.stash {root} action: drop"));
        }
        for path in paths {
            err = err.with_sample(path);
        }
        err
    }
}

/// `git_stash` record of an entry.
fn stash_record(entry: &StashEntry) -> Record {
    GitStash {
        index: u64::try_from(entry.index).unwrap_or(u64::MAX),
        id: entry.id.to_hex().to_string(),
        message: entry.message.clone(),
        time: Instant::from_unix_nanos(entry.time.saturating_mul(NANOS_PER_SECOND)),
        includes_untracked: entry.includes_untracked,
        branch: entry.branch.clone(),
    }
    .into()
}

/// Stage implementation of `git.stash`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StashCommand;

fn request_of(args: &BoundArgs) -> Result<(PathBuf, StashRequest)> {
    let repo = args::path(args, "repo")?;
    let action = args::required_text(args, "action")?.parse()?;
    let request = StashRequest {
        action,
        entry: args::text(args, "stash")?,
        message: args::text(args, "message")?,
        include_untracked: args::flag(args, "include_untracked")?,
        paths: args::list(args, "paths")?
            .into_iter()
            .map(PathBuf::from)
            .collect(),
        exclude: args::list(args, "exclude")?,
        expect_state: args::text(args, "expect_state")?,
    };
    Ok((repo, request))
}

fn read_records(repo: &Path, request: &StashRequest, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let all = list(repo, ctx)?;
    if request.action == StashAction::List {
        return Ok(all.iter().map(stash_record).collect());
    }
    let opened = repo::open(repo, ctx)?;
    let (position, id) = entries::resolve(&opened, request.entry.as_deref())?;
    let entry = match position.and_then(|position| all.get(position).cloned()) {
        Some(entry) => entry,
        None => StashEntry {
            index: 0,
            id,
            message: repo::commit_subject(&opened, id)?,
            time: 0,
            includes_untracked: entries::parts(&opened, id)?.untracked.is_some(),
            branch: None,
        },
    };
    Ok(vec![stash_record(&entry)])
}

impl StageImpl for StashCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "git.stash".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl ReadStage for StashCommand {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let (repo, request) = request_of(args)?;
        if !request.action.is_read() {
            return Err(error(
                ErrorCode::Internal,
                format!(
                    "stash action {} changes the repository; run it as an effect",
                    request.action.as_str()
                ),
            ));
        }
        let records = read_records(&repo, &request, ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

impl EffectStage for StashCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let (repo, request) = request_of(args)?;
        let planned = plan(&repo, &request, ctx)?;
        let outcome = planned.outcome();
        let mut effect = EffectPlan::new();
        effect.changes = file_changes(&outcome.repo, &outcome.written, &outcome.removed);
        effect.notable = outcome.notable();
        effect.bulk = planned.target_count() > 1;
        Ok(effect.with_state(planned))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan
            .take_state::<PlannedStash>()
            .ok_or_else(|| error(ErrorCode::Internal, "the plan of git.stash is missing"))?;
        let outcome = planned.apply(tx)?;
        if !outcome.conflicts.is_empty() {
            return Err(outcome.conflict_error());
        }
        let changes = file_changes(&outcome.repo, &outcome.written, &outcome.removed);
        let notable = outcome.notable();
        let record = effect_record("git.stash", outcome.git_effect(), &changes, &notable);
        let mut result = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        result.changes = changes;
        result.notable = notable;
        Ok(result)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
