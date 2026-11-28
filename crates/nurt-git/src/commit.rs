// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::commit
// Responsibility:   `git.commit`: records a commit and reports its branch and files.
// Dependencies:     gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `git.commit`: records a commit and reports its branch and files.
//!
//! The content of the commit is selected by exactly one of three modes:
//!
//! * [`Selection::Staged`] commits the index as it is and, while a merge is
//!   in progress, concludes the merge (parents `HEAD` and `MERGE_HEAD`, the
//!   merge state removed, reflog `commit (merge): …`);
//! * [`Selection::All`] stages every change of the working tree first:
//!   modified and deleted tracked files and untracked files that are not
//!   ignored;
//! * [`Selection::Paths`] commits `HEAD` with the working tree content of
//!   the given paths, as `git commit --only` does; the index of these paths
//!   is updated and the rest of the index stays out of the commit.
//!
//! `amend` without a mode ([`Selection::MessageOnly`]) replaces only the
//! message of the last commit. The message is cleaned up by
//! `commit.cleanup`, gets the sign-off and the trailers, and passes the
//! `prepare-commit-msg` and `commit-msg` hooks. The commit object is written
//! to the object store, signed when `sign` is true or, without `sign`, when
//! `commit.gpgSign` is set, and the index,
//! `COMMIT_EDITMSG`, the merge state, the branch and the reflogs change in
//! one transaction, so undo moves the branch and the index back.
//!
//! # Examples
//!
//! ```no_run
//! use nurt_contracts::journal::MutationTx;
//! use nurt_git::commit::{self, CommitRequest, Selection};
//! use nurt_git::repo::GitRepo;
//!
//! fn commit_all(repo: &GitRepo, tx: &mut dyn MutationTx) -> nurt_contracts::error::Result<String> {
//!     let mut request = CommitRequest::new("feat: add parser", Selection::All);
//!     request.sign_off = true;
//!     let plan = commit::plan(repo, &request)?;
//!     let outcome = commit::apply(repo, plan, tx)?;
//!     Ok(outcome.commit.short_id)
//! }
//! ```

pub(crate) mod access;
mod apply;
pub(crate) mod identity;
mod prepare;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use gix::ObjectId;
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{
    Change, Commit, CommitFile, CommitStats, Effect, EffectStatus, GitEffect, GitEffectState,
    GitFileState,
};
use nurt_contracts::report::{IrreversibleEntry, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Path as RecordPath, Record};

use crate::message::Trailer;
use crate::repo::GitRepo;
use crate::repo::error::{quote, quote_path};
use crate::repo::records;

/// Name of the command in the catalog.
pub const COMMAND: &str = "git.commit";

/// Content a commit records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// The index as it is (`staged: true`).
    Staged,
    /// Every change of the working tree, untracked files included
    /// (`all: true`).
    All,
    /// `HEAD` with the working tree content of these paths (`paths: […]`).
    Paths(Vec<String>),
    /// The tree of the amended commit; only the message changes (`amend:
    /// true` alone).
    MessageOnly,
}

impl Selection {
    /// Selection from the `paths`, `all`, `staged` and `amend` arguments.
    ///
    /// # Errors
    ///
    /// Fails with `E_PARAM_CONFLICT` when more than one of `paths`, `all`
    /// and `staged` is given, or none of them without `amend`.
    pub fn from_flags(paths: Vec<String>, all: bool, staged: bool, amend: bool) -> Result<Self> {
        let given = usize::from(!paths.is_empty()) + usize::from(all) + usize::from(staged);
        match given {
            0 if amend => Ok(Self::MessageOnly),
            0 => Err(Error::new(
                ErrorCode::ParamConflict,
                "choose what to commit: `staged: true` for the index, `all: true` for every change or `paths: [...]` for files",
            )
            .with_problem("missing_selection")),
            1 if all => Ok(Self::All),
            1 if staged => Ok(Self::Staged),
            1 => Ok(Self::Paths(paths)),
            _ => Err(Error::new(
                ErrorCode::ParamConflict,
                "`paths`, `all` and `staged` exclude each other; give one of them",
            )
            .with_problem("selection_conflict")),
        }
    }

    fn arguments(&self) -> Option<String> {
        match self {
            Self::Staged => Some("staged: true".to_owned()),
            Self::All => Some("all: true".to_owned()),
            Self::Paths(paths) => Some(format!(
                "paths: [{}]",
                paths
                    .iter()
                    .map(|path| quote(path))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Self::MessageOnly => None,
        }
    }
}

/// Arguments of one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    /// Message as given.
    pub message: String,
    /// Content of the commit.
    pub selection: Selection,
    /// Replace the commit at `HEAD`.
    pub amend: bool,
    /// Trailers appended to the message.
    pub trailers: Vec<Trailer>,
    /// Append `Signed-off-by` with the committer identity.
    pub sign_off: bool,
    /// Author, `Name <email>`.
    pub author: Option<String>,
    /// Allow a commit whose tree equals the tree of its first parent.
    pub allow_empty: bool,
    /// Run the hooks of the repository.
    pub hooks: bool,
    /// Sign the commit; `None` follows `commit.gpgSign`.
    pub sign: Option<bool>,
    /// Patterns of paths left out of the commit.
    pub exclude: Vec<String>,
    /// State token the repository must still have.
    pub expect_state: Option<String>,
}

impl CommitRequest {
    /// Request with the given message and selection and the defaults of
    /// every other argument: no amend, no trailers, hooks run.
    #[must_use]
    pub fn new(message: impl Into<String>, selection: Selection) -> Self {
        Self {
            message: message.into(),
            amend: selection == Selection::MessageOnly,
            selection,
            trailers: Vec::new(),
            sign_off: false,
            author: None,
            allow_empty: false,
            hooks: true,
            sign: None,
            exclude: Vec::new(),
            expect_state: None,
        }
    }
}

/// Renders a call of `git.commit` with the arguments of `request`, the given
/// selection and extra arguments, for the `fix` of an error.
pub(crate) fn render_call(
    root: &Path,
    request: &CommitRequest,
    selection: &Selection,
    extra: &[&str],
) -> String {
    let mut call = format!(
        "{COMMAND} {} message: {}",
        quote_path(root),
        quote(&request.message)
    );
    if let Some(arguments) = selection.arguments() {
        call.push(' ');
        call.push_str(&arguments);
    }
    if request.amend {
        call.push_str(" amend: true");
    }
    for argument in extra {
        call.push(' ');
        call.push_str(argument);
    }
    call
}

/// Planned commit, computed without changing the repository.
#[derive(Debug, Clone)]
pub struct CommitPlan {
    prepared: prepare::Prepared,
}

impl CommitPlan {
    /// Tree of the commit as planned; the `pre-commit` hook may change it.
    #[must_use]
    pub fn tree(&self) -> ObjectId {
        self.prepared.tree
    }

    /// Parents of the commit.
    #[must_use]
    pub fn parents(&self) -> &[ObjectId] {
        &self.prepared.parents
    }

    /// Files the commit changes, in target order.
    #[must_use]
    pub fn changes(&self) -> &[Change] {
        &self.prepared.changes
    }

    /// Unusual effects known before execution.
    #[must_use]
    pub fn notable(&self) -> &[NotableEntry] {
        &self.prepared.notes
    }
}

/// Plans a commit.
///
/// Trees and blobs of the selected content are added to the object store;
/// nothing else changes.
///
/// # Errors
///
/// * `E_STALE` when `expect_state` differs from the state of the repository;
/// * `E_PARAM_CONFLICT` for `amend` or `paths` while a merge is in progress;
/// * `E_GIT_NOTHING_TO_COMMIT` when the tree equals the tree of the first
///   parent and `allow_empty` is false;
/// * `E_GIT_IN_PROGRESS` when the index has unresolved conflicts;
/// * `E_CONFIG` with problem `missing_identity` when no author or committer
///   is configured;
/// * `E_NOT_FOUND` for a path that matches no file;
/// * `E_GIT_UNSUPPORTED` for `all` or `paths` in a bare repository;
/// * `E_IO` when the repository cannot be read.
pub fn plan(repo: &GitRepo, request: &CommitRequest) -> Result<CommitPlan> {
    Ok(CommitPlan {
        prepared: prepare::prepare(repo, request)?,
    })
}

/// Result of [`apply`].
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    /// Identifier of the new commit.
    pub id: ObjectId,
    /// Record of the new commit with its files and statistics.
    pub commit: Commit,
    /// Record of the effect on the repository.
    pub effect: Effect,
    /// Unusual effects.
    pub notable: Vec<NotableEntry>,
    /// Hooks that ran, as irreversible effects.
    pub irreversible: Vec<IrreversibleEntry>,
}

/// Runs the hooks, writes the commit and applies the planned changes through
/// `tx`. The caller commits or rolls back `tx`.
///
/// # Errors
///
/// * `E_GIT_HOOK`, `E_TIMEOUT` or `E_CANCELLED` when `pre-commit`,
///   `prepare-commit-msg` or `commit-msg` fails;
/// * `E_INVALID_VALUE` with problem `empty_message` when the message is
///   empty after cleanup;
/// * `E_GIT_SIGN` when the commit cannot be signed;
/// * `E_GIT_REF` or `E_STALE` when the branch moved since [`plan`];
/// * `E_GIT_LOCKED` when a lock file of `git` exists;
/// * the errors of the transaction.
pub fn apply(repo: &GitRepo, plan: CommitPlan, tx: &mut dyn MutationTx) -> Result<CommitOutcome> {
    let prepared = plan.prepared;
    let written = apply::apply(repo, prepared.clone(), tx)?;
    let (commit, effect) = outcome_records(repo, &prepared, &written)?;
    Ok(CommitOutcome {
        id: written.id,
        commit,
        effect,
        notable: written.notes,
        irreversible: written.irreversible,
    })
}

fn short_branch(prepared: &prepare::Prepared) -> Option<String> {
    prepared
        .branch
        .as_ref()
        .map(|name| records::short_ref_name(&name.as_bstr().to_string()).to_owned())
}

fn outcome_records(
    repo: &GitRepo,
    prepared: &prepare::Prepared,
    written: &apply::Written,
) -> Result<(Commit, Effect)> {
    let mut commit = records::commit_by_id(repo, written.id)?;
    let files = changed_files(repo, prepared.base_tree, written.tree)?;
    commit.stats = Some(CommitStats {
        files: u64::try_from(files.len()).unwrap_or(u64::MAX),
        added: files.iter().filter_map(|file| file.added).sum(),
        removed: files.iter().filter_map(|file| file.removed).sum(),
    });
    commit.files = Some(files);
    let branch = short_branch(prepared);
    commit.refs = branch.clone().map(|name| vec![name]);
    commit.branch.clone_from(&branch);
    let git = GitEffect {
        repo: RecordPath::new(prepared.root.display().to_string()),
        state: GitEffectState::Completed,
        head_before: prepared.head.map(|id| id.to_string()),
        head_after: Some(written.id.to_string()),
        branch: branch.clone(),
        branch_before: branch,
        detached: Some(prepared.branch.is_none()),
        state_token: Some(prepare::state_token(repo)?),
        ..GitEffect::default()
    };
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: COMMAND.to_owned(),
        targets: u64::try_from(prepared.changes.len()).ok(),
        changes: Some(prepared.changes.clone()),
        irreversible: (!written.irreversible.is_empty()).then(|| written.irreversible.clone()),
        git: Some(git),
        ..Effect::default()
    };
    Ok((commit, effect))
}

/// Files that differ between the tree of the first parent and the new tree,
/// with line counts.
fn changed_files(
    repo: &GitRepo,
    before: Option<ObjectId>,
    after: ObjectId,
) -> Result<Vec<CommitFile>> {
    let old = access::tree_files(repo, before)?;
    let new = access::tree_files(repo, Some(after))?;
    let mut paths: Vec<&bstr::BString> = old.keys().chain(new.keys()).collect();
    paths.sort();
    paths.dedup();
    let mut files = Vec::new();
    for path in paths {
        let (status, old_entry, new_entry) = match (old.get(path), new.get(path)) {
            (None, Some(entry)) => (GitFileState::Added, None, Some(*entry)),
            (Some(entry), None) => (GitFileState::Deleted, Some(*entry), None),
            (Some(left), Some(right)) if left != right => {
                let link_changed = (left.1 == EntryKind::Link) != (right.1 == EntryKind::Link);
                let status = if link_changed {
                    GitFileState::TypeChanged
                } else {
                    GitFileState::Modified
                };
                (status, Some(*left), Some(*right))
            }
            _ => continue,
        };
        let content = |entry: Option<(ObjectId, EntryKind)>| -> Result<Vec<u8>> {
            match entry {
                Some((id, kind)) if kind != EntryKind::Commit => access::blob(repo, id),
                _ => Ok(Vec::new()),
            }
        };
        let counts = access::line_counts(&content(old_entry)?, &content(new_entry)?);
        files.push(CommitFile {
            path: path.to_string(),
            status,
            old_path: None,
            added: counts.map(|(added, _)| added),
            removed: counts.map(|(_, removed)| removed),
            binary: Some(counts.is_none()),
        });
    }
    Ok(files)
}

/// The `git.commit` command.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitCommit;

/// Plan of [`GitCommit`] between the plan and apply steps.
struct Planned {
    repo: PathBuf,
    plan: CommitPlan,
}

fn missing(args: &BoundArgs, name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        format!("`{}` needs a value for `{name}`", args.command()),
    )
}

/// Reading of bound arguments shared by `git.commit` and `git.review`.
pub(crate) mod args {
    use std::path::PathBuf;

    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::stage::BoundArgs;
    use nurt_contracts::value::Value;

    fn scalar(value: &Value) -> Option<String> {
        match value {
            Value::Text(text) | Value::Path(text) | Value::Glob(text) | Value::Enum(text) => {
                Some(text.clone())
            }
            _ => None,
        }
    }

    fn type_error(args: &BoundArgs, name: &str, expected: &str) -> Error {
        Error::new(
            ErrorCode::Type,
            format!("`{}` expects {expected} for `{name}`", args.command()),
        )
    }

    /// Optional text, path or pattern argument.
    pub(crate) fn text(args: &BoundArgs, name: &str) -> Result<Option<String>> {
        match args.value(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => scalar(value)
                .map(Some)
                .ok_or_else(|| type_error(args, name, "text")),
        }
    }

    /// Argument given as one value or a list of values.
    pub(crate) fn texts(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
        match args.value(name) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::List(items)) => items
                .iter()
                .map(|item| scalar(item).ok_or_else(|| type_error(args, name, "list<text>")))
                .collect(),
            Some(value) => scalar(value)
                .map(|text| vec![text])
                .ok_or_else(|| type_error(args, name, "text")),
        }
    }

    /// The `repo` argument.
    pub(crate) fn repo(args: &BoundArgs) -> Result<PathBuf> {
        text(args, "repo")?.map(PathBuf::from).ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("`{}` needs a value for `repo`", args.command()),
            )
        })
    }
}

/// Builds the request from bound arguments.
///
/// # Errors
///
/// `E_MISSING_PARAM` without `message`, `E_PARAM_CONFLICT` for an invalid
/// selection and `E_INVALID_VALUE` for a malformed trailer.
pub fn request_from_args(args: &BoundArgs) -> Result<CommitRequest> {
    let message = args::text(args, "message")?.ok_or_else(|| missing(args, "message"))?;
    let amend = args.bool("amend")?.unwrap_or(false);
    let selection = Selection::from_flags(
        args::texts(args, "paths")?,
        args.bool("all")?.unwrap_or(false),
        args.bool("staged")?.unwrap_or(false),
        amend,
    )?;
    let trailers = args::texts(args, "trailers")?
        .iter()
        .map(|text| {
            Trailer::parse(text).map_err(|error| {
                Error::new(ErrorCode::InvalidValue, error.to_string())
                    .with_problem("invalid_trailer")
                    .with_found(text.clone())
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(CommitRequest {
        message,
        selection,
        amend,
        trailers,
        sign_off: args.bool("sign_off")?.unwrap_or(false),
        author: args::text(args, "author")?,
        allow_empty: args.bool("allow_empty")?.unwrap_or(false),
        hooks: args.bool("hooks")?.unwrap_or(true),
        sign: args.bool("sign")?,
        exclude: args::texts(args, "exclude")?,
        expect_state: args::text(args, "expect_state")?,
    })
}

impl StageImpl for GitCommit {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        request_from_args(args)?;
        Ok(StaticInfo::default())
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for GitCommit {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let request = request_from_args(args)?;
        let path = args::repo(args)?;
        let repo = GitRepo::discover(&path, ctx)?;
        let plan = plan(&repo, &request)?;
        let mut effect = EffectPlan::new();
        effect.changes = plan.changes().to_vec();
        effect.notable = plan.notable().to_vec();
        effect.bulk = effect.changes.len() > 1;
        Ok(effect.with_state(Planned {
            repo: repo.root().to_path_buf(),
            plan,
        }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let changes = std::mem::take(&mut plan.changes);
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "`git.commit` received a plan of another stage",
            )
        })?;
        let repo = GitRepo::open(&planned.repo, ctx)?;
        let outcome = apply(&repo, planned.plan, tx)?;
        let records = vec![Record::from(outcome.effect), Record::from(outcome.commit)];
        let mut result =
            EffectOutcome::new(StageOutput::records(RecordStream::from_records(records)));
        result.changes = changes;
        result.notable = outcome.notable;
        result.irreversible = outcome.irreversible;
        Ok(result)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
