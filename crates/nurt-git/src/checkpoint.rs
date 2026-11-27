// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.checkpoint` command.
//!
//! A checkpoint is a commit that records the working tree as `git add -A`
//! would stage it in a temporary index: tracked files with their working tree
//! content and untracked files that are not ignored. With `index: true` a
//! second commit records the index and becomes the last parent, as in a stash
//! entry. Checkpoints live under `refs/nurt/checkpoints/cp-<N>` and never
//! change `HEAD`, the index or the working tree when created. Checkpoints
//! written under `refs/danaco/checkpoints/` are listed, compared and restored
//! but never written.
//!
//! The commit message has the form
//!
//! ```text
//! nurt checkpoint cp-3: before refactoring
//!
//! branch: main
//! head: 5b6f0e3c0a2d7c1b0d1e9f6a4c3b2a1908f7e6d5
//! index: none
//! ```

mod compare;
mod restore;
pub(crate) mod snapshot;
pub(crate) mod support;
#[cfg(test)]
pub(crate) mod test_journal;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{ExemptWrite, MutationTx};
use nurt_contracts::record::{GitCheckpoint, GitEffect, GitEffectState};
use nurt_contracts::report::{Change, ChangeAction, Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Instant, Path as RecordPath, Record};
use sha2::{Digest, Sha256};

pub use compare::{CompareRequest, Comparison, compare};
pub use restore::{RestorePlan, RestoreRequest, apply_restore, plan_restore};

use crate::repo::pathspec::PathFilter;
use snapshot::{Blobs, index_entries, snapshot_worktree};
use support::{
    Filters, TxPlan, abs, check_expected_state, committer, delete_ref, display_path, feed,
    finish_token, problem, quote, ref_content, ref_file, repo_error, workdir, write_tree,
};

/// Prefix of the references of checkpoints.
pub const CHECKPOINT_PREFIX: &str = "refs/nurt/checkpoints/";

/// Prefix of checkpoint references written by earlier tools; read only.
pub const LEGACY_CHECKPOINT_PREFIX: &str = "refs/danaco/checkpoints/";

/// Label of a checkpoint created without one.
pub const DEFAULT_LABEL: &str = "checkpoint";

/// Prefix of checkpoint identifiers.
const ID_PREFIX: &str = "cp-";

/// Subject prefix of checkpoint commits.
const SUBJECT_PREFIX: &str = "nurt checkpoint ";

/// Value of a metadata line that has no value.
const NONE_VALUE: &str = "none";

/// Prefix of a message line that records an `exclude` pattern.
const EXCLUDE_KEY: &str = "exclude: ";

/// Nanoseconds in one second.
const NANOSECONDS_PER_SECOND: i64 = 1_000_000_000;

/// Parameters of a new checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateRequest {
    /// Label stored in the commit message; [`DEFAULT_LABEL`] when absent.
    pub label: Option<String>,
    /// Also record the index as a second commit.
    pub index: bool,
    /// Patterns of files left out of the checkpoint, tracked or not. A
    /// restore leaves files that match them untouched.
    pub exclude: Vec<String>,
}

/// Checkpoint computed by [`plan_create`]; its objects are already in the
/// object database and only the reference remains to be written.
#[derive(Debug, Clone)]
pub struct CreatePlan {
    common_dir: PathBuf,
    reference: String,
    commit: ObjectId,
    record: GitCheckpoint,
    filters_skipped: bool,
    excluded: u64,
}

impl CreatePlan {
    /// Record of the checkpoint that [`apply_create`] makes reachable.
    #[must_use]
    pub fn checkpoint(&self) -> &GitCheckpoint {
        &self.record
    }

    /// Number of files left out by the `exclude` patterns.
    #[must_use]
    pub fn excluded(&self) -> u64 {
        self.excluded
    }

    pub(crate) fn reference_file(&self) -> PathBuf {
        ref_path(&self.common_dir, &self.reference)
    }

    /// Declares the write of the reference in `writes`.
    pub(crate) fn declare(&self, writes: &mut TxPlan, tx: &mut dyn MutationTx) -> Result<()> {
        writes.write_locked(&self.reference_file(), ref_content(self.commit), None)?;
        tx.record_exempt(ExemptWrite::ObjectStore {
            repo: abs(&self.common_dir)?,
        });
        Ok(())
    }
}

/// Records the working tree of the repository at `repo` (and its index with
/// [`CreateRequest::index`]) in new objects and returns the plan of the
/// checkpoint reference.
///
/// Nothing but the object database changes. Filter drivers configured by the
/// repository are not run; content is recorded as found.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` when `repo` does not exist, `E_GIT_NOT_REPOSITORY` when it
/// is not inside a repository, `E_GIT_UNSUPPORTED` for a bare repository and
/// Fails with `E_IO` when the repository cannot be read or objects cannot be written.
/// Fails with `E_INVALID_VALUE` for an `exclude` pattern with a line break.
pub fn plan_create(repo: &Path, request: &CreateRequest, ctx: &ExecCtx) -> Result<CreatePlan> {
    if request.exclude.iter().any(|glob| glob.contains('\n')) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "`exclude` patterns must not contain line breaks; pass one pattern per entry",
        ));
    }
    let repo = support::open_repo(repo)?;
    let root = workdir(&repo)?;
    let filter = PathFilter::new(&root, &[], &request.exclude)?;
    let index = repo
        .index_or_empty()
        .map_err(|err| repo_error("read the index", err))?;
    let mut filters = Filters::new(&repo, &index)?;
    let mut snapshot = snapshot_worktree(&repo, &index, &mut filters, Blobs::Write)?;
    let found = snapshot.files.len();
    snapshot
        .files
        .retain(|path, _| !filter.excluded(path.as_bstr()));
    let excluded = (found - snapshot.files.len()) as u64;
    let worktree_tree = write_tree(&repo, &snapshot.files)?;
    let head = repo.head_id().ok().map(gix::Id::detach);
    let branch = current_branch(&repo);
    let number = next_number(&repo)?;
    let id = format!("{ID_PREFIX}{number}");
    let label = request
        .label
        .as_deref()
        .map(first_line)
        .filter(|label| !label.is_empty())
        .unwrap_or(DEFAULT_LABEL)
        .to_owned();
    let who = committer(&repo, ctx);
    let index_commit = if request.index {
        match index_entries(&index) {
            Some(mut entries) => {
                entries.retain(|path, _| !filter.excluded(path.as_bstr()));
                let tree = write_tree(&repo, &entries)?;
                let message = format!("{SUBJECT_PREFIX}{id} index\n");
                Some(write_commit(
                    &repo,
                    tree,
                    head.into_iter().collect(),
                    &who,
                    message,
                )?)
            }
            None => None,
        }
    } else {
        None
    };
    let message = checkpoint_message(
        &id,
        &label,
        branch.as_deref(),
        head,
        index_commit,
        &request.exclude,
    );
    let parents = head.into_iter().chain(index_commit).collect();
    let commit = write_commit(&repo, worktree_tree, parents, &who, message)?;
    let reference = format!("{CHECKPOINT_PREFIX}{id}");
    let record = GitCheckpoint {
        id,
        commit: commit.to_string(),
        time: instant(who.time.seconds),
        reference: reference.clone(),
        index_included: index_commit.is_some(),
        label: Some(label),
        files: Some(snapshot.files.len() as u64),
    };
    drop(root);
    Ok(CreatePlan {
        common_dir: support::lexical(repo.common_dir()),
        reference,
        commit,
        record,
        filters_skipped: filters.drivers_skipped(),
        excluded,
    })
}

/// Writes the reference of a planned checkpoint through `tx`.
///
/// # Errors
///
/// Fails with `E_GIT_LOCKED` when another process holds the lock of the reference,
/// Fails with `E_STALE` when the reference appeared after planning, and the errors of
/// the journal.
pub fn apply_create(plan: &CreatePlan, tx: &mut dyn MutationTx) -> Result<GitCheckpoint> {
    let mut writes = TxPlan::default();
    plan.declare(&mut writes, tx)?;
    writes.preserve(tx)?;
    writes.apply_rest(tx)?;
    Ok(plan.record.clone())
}

/// Lists the checkpoints of the repository at `repo`, newest first.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND`, `E_GIT_NOT_REPOSITORY` and `E_IO` as for [`plan_create`].
pub fn list(repo: &Path) -> Result<Vec<GitCheckpoint>> {
    let repo = support::open_repo(repo)?;
    let mut out = Vec::new();
    for found in checkpoint_refs(&repo)? {
        out.push(describe(&repo, &found)?);
    }
    Ok(out)
}

/// Parameters of a deletion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteRequest {
    /// Checkpoints to delete, as `cp-<N>`.
    pub ids: Vec<String>,
    /// Keep only this many newest checkpoints.
    pub keep: Option<u64>,
}

/// References selected by [`plan_delete`].
#[derive(Debug, Clone)]
pub struct DeletePlan {
    repo: PathBuf,
    references: Vec<String>,
}

impl DeletePlan {
    /// References that [`apply_delete`] removes.
    #[must_use]
    pub fn references(&self) -> &[String] {
        &self.references
    }
}

/// Selects the checkpoints to delete.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` when neither identifiers nor `keep` are given, `E_ARG`
/// when both are given or an identifier is malformed, `E_GIT_REF` with
/// problem `not_found` for an unknown checkpoint and `read_only` for a
/// checkpoint under [`LEGACY_CHECKPOINT_PREFIX`].
pub fn plan_delete(repo: &Path, request: &DeleteRequest) -> Result<DeletePlan> {
    let path = repo.to_path_buf();
    let repo = support::open_repo(repo)?;
    let shown = display_path(&path);
    let references = match (request.ids.is_empty(), request.keep) {
        (true, None) => {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`git.checkpoint` with `action: delete` needs `checkpoint_id` or `keep`",
            )
            .with_fix(format!(
                "git.checkpoint {} action: delete keep: 10",
                quote(&shown)
            )));
        }
        (false, Some(_)) => {
            return Err(problem(
                ErrorCode::Arg,
                "param_conflict",
                "`checkpoint_id` and `keep` exclude each other; pass one of them",
            ));
        }
        (false, None) => {
            let mut references = Vec::new();
            for id in &request.ids {
                let found = resolve(&repo, id, &shown)?;
                if found.legacy {
                    return Err(problem(
                        ErrorCode::GitRef,
                        "read_only",
                        format!("`{id}` was written by another tool and is read only"),
                    ));
                }
                references.push(found.reference);
            }
            references.sort();
            references.dedup();
            references
        }
        (true, Some(keep)) => {
            let own: Vec<CheckpointRef> = checkpoint_refs(&repo)?
                .into_iter()
                .filter(|found| !found.legacy)
                .collect();
            let keep = usize::try_from(keep).unwrap_or(usize::MAX);
            own.into_iter()
                .skip(keep)
                .map(|found| found.reference)
                .collect()
        }
    };
    Ok(DeletePlan {
        repo: path,
        references,
    })
}

/// Deletes the planned checkpoint references through `tx`.
///
/// # Errors
///
/// Fails with `E_GIT_LOCKED` when a reference is locked, `E_STALE` when a reference
/// changed after planning, and the errors of the journal.
pub fn apply_delete(plan: &DeletePlan, tx: &mut dyn MutationTx) -> Result<Vec<String>> {
    let repo = support::open_repo(&plan.repo)?;
    let mut writes = TxPlan::default();
    for reference in &plan.references {
        delete_ref(&mut writes, &repo, reference)?;
    }
    writes.preserve(tx)?;
    writes.apply_rest(tx)?;
    Ok(plan.references.clone())
}

/// One checkpoint reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointRef {
    pub(crate) id: String,
    pub(crate) number: u64,
    pub(crate) reference: String,
    pub(crate) commit: ObjectId,
    pub(crate) legacy: bool,
}

/// Metadata stored in a checkpoint commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckpointCommit {
    pub(crate) tree: ObjectId,
    pub(crate) index_tree: Option<ObjectId>,
    pub(crate) head: Option<ObjectId>,
    pub(crate) label: String,
    pub(crate) time: i64,
    pub(crate) excludes: Vec<String>,
}

/// Checkpoint references, newest first; own references before legacy ones
/// with the same number.
pub(crate) fn checkpoint_refs(repo: &gix::Repository) -> Result<Vec<CheckpointRef>> {
    let mut out = Vec::new();
    for (prefix, legacy) in [(CHECKPOINT_PREFIX, false), (LEGACY_CHECKPOINT_PREFIX, true)] {
        for (reference, commit) in refs_with_prefix(repo, prefix)? {
            let Some(id) = reference.strip_prefix(prefix) else {
                continue;
            };
            let Some(number) = parse_id(id) else {
                continue;
            };
            out.push(CheckpointRef {
                id: id.to_owned(),
                number,
                reference,
                commit,
                legacy,
            });
        }
    }
    out.sort_by(|a, b| b.number.cmp(&a.number).then(a.legacy.cmp(&b.legacy)));
    Ok(out)
}

/// Finds checkpoint `id` (`cp-<N>`).
pub(crate) fn resolve(repo: &gix::Repository, id: &str, shown_repo: &str) -> Result<CheckpointRef> {
    if parse_id(id).is_none() {
        return Err(problem(
            ErrorCode::Arg,
            "invalid_id",
            format!("`{id}` is not a checkpoint identifier; use the form `cp-1`"),
        ));
    }
    checkpoint_refs(repo)?
        .into_iter()
        .find(|found| found.id == id)
        .ok_or_else(|| {
            problem(
                ErrorCode::GitRef,
                "not_found",
                format!("checkpoint `{id}` does not exist; list the checkpoints to find another"),
            )
            .with_fix(format!("git.checkpoint {} action: list", quote(shown_repo)))
        })
}

/// Reads the metadata of a checkpoint commit.
pub(crate) fn read_commit(
    repo: &gix::Repository,
    found: &CheckpointRef,
) -> Result<CheckpointCommit> {
    let commit = repo
        .find_commit(found.commit)
        .map_err(|err| repo_error(&format!("read checkpoint `{}`", found.id), err))?;
    let tree = commit
        .tree_id()
        .map_err(|err| repo_error(&format!("read checkpoint `{}`", found.id), err))?
        .detach();
    let message = commit
        .message_raw()
        .map_err(|err| repo_error(&format!("read checkpoint `{}`", found.id), err))?
        .to_str_lossy()
        .into_owned();
    let time = commit.time().map(|time| time.seconds).unwrap_or(0);
    let metadata = parse_metadata(&message);
    let index_tree = match metadata.get("index").and_then(|value| parse_oid(value)) {
        Some(index_commit) => Some(
            repo.find_commit(index_commit)
                .and_then(|commit| commit.tree_id())
                .map_err(|err| repo_error(&format!("read the index of `{}`", found.id), err))?
                .detach(),
        ),
        None => None,
    };
    Ok(CheckpointCommit {
        tree,
        index_tree,
        head: metadata.get("head").and_then(|value| parse_oid(value)),
        label: label_of(&message, &found.id),
        time,
        excludes: message
            .lines()
            .skip(1)
            .filter_map(|line| line.strip_prefix(EXCLUDE_KEY))
            .map(str::to_owned)
            .collect(),
    })
}

fn describe(repo: &gix::Repository, found: &CheckpointRef) -> Result<GitCheckpoint> {
    let commit = read_commit(repo, found)?;
    let files = support::flatten_tree(repo, commit.tree)?.len() as u64;
    Ok(GitCheckpoint {
        id: found.id.clone(),
        commit: found.commit.to_string(),
        time: instant(commit.time),
        reference: found.reference.clone(),
        index_included: commit.index_tree.is_some(),
        label: Some(commit.label),
        files: Some(files),
    })
}

fn parse_id(id: &str) -> Option<u64> {
    let digits = id.strip_prefix(ID_PREFIX)?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn parse_oid(text: &str) -> Option<ObjectId> {
    if text == NONE_VALUE {
        return None;
    }
    ObjectId::from_hex(text.as_bytes()).ok()
}

fn parse_metadata(message: &str) -> BTreeMap<String, String> {
    message
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(": "))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn label_of(message: &str, id: &str) -> String {
    let subject = message.lines().next().unwrap_or_default();
    let own = format!("{SUBJECT_PREFIX}{id}: ");
    subject.strip_prefix(&own).unwrap_or(subject).to_owned()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default().trim()
}

fn checkpoint_message(
    id: &str,
    label: &str,
    branch: Option<&str>,
    head: Option<ObjectId>,
    index: Option<ObjectId>,
    exclude: &[String],
) -> String {
    let show =
        |oid: Option<ObjectId>| oid.map_or_else(|| NONE_VALUE.to_owned(), |oid| oid.to_string());
    let mut message = format!(
        "{SUBJECT_PREFIX}{id}: {label}\n\nbranch: {}\nhead: {}\nindex: {}\n",
        branch.unwrap_or(NONE_VALUE),
        show(head),
        show(index)
    );
    for glob in exclude {
        message.push_str(EXCLUDE_KEY);
        message.push_str(glob);
        message.push('\n');
    }
    message
}

/// Token of the state a checkpoint command depends on: `HEAD`, the index,
/// the files of the working tree and the checkpoint references.
///
/// The token is compared with the `expect_state` argument.
pub(crate) fn state_token(repo_path: &Path) -> Result<String> {
    let repo = support::open_repo(repo_path)?;
    let index = repo
        .index_or_empty()
        .map_err(|err| repo_error("read the index", err))?;
    let mut filters = Filters::new(&repo, &index)?;
    let snapshot = snapshot_worktree(&repo, &index, &mut filters, Blobs::HashOnly)?;
    let mut hasher = Sha256::new();
    feed(
        &mut hasher,
        current_branch(&repo).unwrap_or_default().as_bytes(),
    );
    let head = repo.head_id().ok().map(|id| id.to_string());
    feed(&mut hasher, head.unwrap_or_default().as_bytes());
    feed(
        &mut hasher,
        &std::fs::read(repo.index_path()).unwrap_or_default(),
    );
    for (path, (kind, id)) in &snapshot.files {
        feed(&mut hasher, path);
        feed(&mut hasher, format!("{kind:?} {id}").as_bytes());
    }
    for found in checkpoint_refs(&repo)? {
        feed(&mut hasher, found.reference.as_bytes());
        feed(&mut hasher, found.commit.to_string().as_bytes());
    }
    Ok(finish_token(hasher))
}

/// Fails with `E_STALE` when `expect_state` of `args` differs from the
/// current state of the repository at `repo_path`.
fn check_expected(args: &BoundArgs, repo_path: &Path) -> Result<()> {
    let expected = args::text(args, "expect_state")?;
    if expected.is_none() {
        return Ok(());
    }
    check_expected_state(expected.as_deref(), &state_token(repo_path)?, repo_path)
}

fn write_commit(
    repo: &gix::Repository,
    tree: ObjectId,
    parents: Vec<ObjectId>,
    who: &gix::actor::Signature,
    message: String,
) -> Result<ObjectId> {
    let commit = gix::objs::Commit {
        tree,
        parents: parents.into(),
        author: who.clone(),
        committer: who.clone(),
        encoding: None,
        message: BString::from(message),
        extra_headers: Vec::new(),
    };
    repo.write_object(&commit)
        .map(gix::Id::detach)
        .map_err(|err| repo_error("write the checkpoint commit", err))
}

pub(crate) fn current_branch(repo: &gix::Repository) -> Option<String> {
    repo.head_name()
        .ok()
        .flatten()
        .map(|name| name.shorten().to_str_lossy().into_owned())
}

fn next_number(repo: &gix::Repository) -> Result<u64> {
    Ok(checkpoint_refs(repo)?
        .iter()
        .map(|found| found.number)
        .max()
        .unwrap_or(0)
        + 1)
}

fn instant(seconds: i64) -> Instant {
    Instant::from_unix_nanos(seconds.saturating_mul(NANOSECONDS_PER_SECOND))
}

fn ref_path(common_dir: &Path, reference: &str) -> PathBuf {
    let mut path = common_dir.to_path_buf();
    for part in reference.split('/') {
        path.push(part);
    }
    path
}

/// References under `prefix`, loose and packed, with their values.
pub(crate) fn refs_with_prefix(
    repo: &gix::Repository,
    prefix: &str,
) -> Result<BTreeMap<String, ObjectId>> {
    let mut out = BTreeMap::new();
    let packed = repo.common_dir().join("packed-refs");
    if let Ok(content) = std::fs::read(&packed) {
        for line in content.lines() {
            let Some((oid, name)) = line
                .to_str_lossy()
                .split_once(' ')
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
            else {
                continue;
            };
            if name.starts_with(prefix)
                && gix::refs::FullName::try_from(name.as_str()).is_ok()
                && let Ok(oid) = ObjectId::from_hex(oid.as_bytes())
            {
                out.insert(name, oid);
            }
        }
    }
    let dir = ref_file(repo, prefix.trim_end_matches('/'));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".lock") || !entry.path().is_file() {
            continue;
        }
        let content = std::fs::read(entry.path())
            .map_err(|err| repo_error(&format!("read reference `{prefix}{name}`"), err))?;
        if let Ok(oid) = ObjectId::from_hex(content.trim()) {
            out.insert(format!("{prefix}{name}"), oid);
        }
    }
    Ok(out)
}

/// Implementation of `git.checkpoint` for the executor.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckpointCommand;

/// Name of the command in the catalog.
const COMMAND: &str = "git.checkpoint";

/// Planned effect carried from [`EffectStage::plan`] to [`EffectStage::apply`].
enum Planned {
    Create(CreatePlan),
    Restore(Box<RestorePlan>),
    Delete(DeletePlan),
}

impl StageImpl for CheckpointCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
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

impl ReadStage for CheckpointCommand {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let repo = args::repo(args)?;
        let records: Vec<Record> = match args::action(args)?.as_str() {
            "list" => list(&repo)?.into_iter().map(Record::from).collect(),
            "compare" => match compare(&repo, &args::compare_request(args)?)? {
                Comparison::Hunks(hunks) => hunks.into_iter().map(Record::from).collect(),
                Comparison::Changes(changes) => changes.into_iter().map(Record::from).collect(),
            },
            other => return Err(args::not_a_read(other)),
        };
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

impl EffectStage for CheckpointCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let repo = args::repo(args)?;
        check_expected(args, &repo)?;
        match args::action(args)?.as_str() {
            "create" => {
                let request = CreateRequest {
                    label: args.text("label")?.map(str::to_owned),
                    index: args.bool("index")?.unwrap_or(false),
                    exclude: args::texts(args, "exclude")?,
                };
                let plan = plan_create(&repo, &request, ctx)?;
                let mut effect = EffectPlan::new();
                effect.changes.push(Change::new(
                    display_path(&plan.reference_file()),
                    ChangeAction::Create,
                ));
                if plan.filters_skipped {
                    effect
                        .notable
                        .push(NotableEntry::new(Notable::FilterNotApplied));
                }
                if plan.excluded > 0 {
                    let mut entry = NotableEntry::new(Notable::Excluded);
                    entry.count = Some(plan.excluded);
                    effect.notable.push(entry);
                }
                Ok(effect.with_state(Planned::Create(plan)))
            }
            "restore" => {
                let request = args::restore_request(args)?;
                let mut plan = plan_restore(&repo, &request)?;
                if args.bool("save_current")?.unwrap_or(true) && !plan.is_empty() {
                    let before = CreateRequest {
                        label: Some(format!("before restore of {}", request.checkpoint)),
                        index: request.index,
                        exclude: Vec::new(),
                    };
                    plan.save_current(plan_create(&repo, &before, ctx)?);
                }
                let mut effect = EffectPlan::new();
                effect.changes = plan.changes();
                effect.notable = plan.notable();
                effect.bulk = effect.changes.len() > 1;
                Ok(effect.with_state(Planned::Restore(Box::new(plan))))
            }
            "delete" => {
                let request = DeleteRequest {
                    ids: args::texts(args, "checkpoint_id")?,
                    keep: args::count(args, "keep")?,
                };
                let plan = plan_delete(&repo, &request)?;
                let open = support::open_repo(&repo)?;
                let mut effect = EffectPlan::new();
                for reference in &plan.references {
                    effect.changes.push(Change::new(
                        display_path(&ref_file(&open, reference)),
                        ChangeAction::Delete,
                    ));
                }
                if plan.references.is_empty() {
                    effect
                        .notable
                        .push(NotableEntry::new(Notable::NothingMatched));
                }
                effect.bulk = effect.changes.len() > 1;
                Ok(effect.with_state(Planned::Delete(plan)))
            }
            other => Err(args::not_an_effect(other)),
        }
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let repo = args::repo(args)?;
        let changes = std::mem::take(&mut plan.changes);
        let notable = std::mem::take(&mut plan.notable);
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "`git.checkpoint` received a plan of another stage",
            )
        })?;
        let record: Record = match planned {
            Planned::Create(plan) => apply_create(&plan, tx)?.into(),
            Planned::Restore(plan) => {
                let state = if plan.is_empty() {
                    GitEffectState::NothingToDo
                } else {
                    GitEffectState::Completed
                };
                apply_restore(&plan, tx)?;
                let saved = plan.saved().map(|saved| saved.id.clone());
                effect_record(&repo, state, saved)?
            }
            Planned::Delete(plan) => {
                let deleted = apply_delete(&plan, tx)?;
                let state = if deleted.is_empty() {
                    GitEffectState::NothingToDo
                } else {
                    GitEffectState::Completed
                };
                effect_record(&repo, state, None)?
            }
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        outcome.changes = changes;
        outcome.notable = notable;
        Ok(outcome)
    }
}

fn effect_record(
    repo_path: &Path,
    state: GitEffectState,
    checkpoint_before: Option<String>,
) -> Result<Record> {
    let repo = support::open_repo(repo_path)?;
    let root = workdir(&repo)?;
    let head = repo.head_id().ok().map(|id| id.to_string());
    Ok(GitEffect {
        repo: RecordPath::new(display_path(&root)),
        state,
        head_before: head.clone(),
        head_after: head,
        branch: current_branch(&repo),
        checkpoint_before,
        state_token: Some(state_token(repo_path)?),
        ..GitEffect::default()
    }
    .into())
}

/// Reading of bound arguments shared by the commands of this crate.
pub(crate) mod args {
    use std::path::PathBuf;

    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::stage::BoundArgs;
    use nurt_contracts::value::Value;

    use super::{CompareRequest, RestoreRequest};

    /// Largest number of context lines.
    const MAX_CONTEXT: i64 = 200;

    /// Text of a scalar value given as text, enumeration member or path.
    fn scalar(value: &Value) -> Option<String> {
        match value {
            Value::Text(text) | Value::Enum(text) | Value::Path(text) | Value::Glob(text) => {
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

    /// Optional text, enumeration or path argument.
    pub(crate) fn text(args: &BoundArgs, name: &str) -> Result<Option<String>> {
        match args.value(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => scalar(value)
                .map(Some)
                .ok_or_else(|| type_error(args, name, "text")),
        }
    }

    /// Argument given as one text or a list of texts.
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

    /// Repository argument.
    pub(crate) fn repo(args: &BoundArgs) -> Result<PathBuf> {
        let first = texts(args, "repo")?.into_iter().next();
        first.map(PathBuf::from).ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("`{}` needs a value for `repo`", args.command()),
            )
        })
    }

    /// Required `action` argument.
    pub(crate) fn action(args: &BoundArgs) -> Result<String> {
        text(args, "action")?.ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("`{}` needs a value for `action`", args.command()),
            )
        })
    }

    /// Non-negative integer argument.
    pub(crate) fn count(args: &BoundArgs, name: &str) -> Result<Option<u64>> {
        match args.int(name)? {
            None => Ok(None),
            Some(value) => u64::try_from(value).map(Some).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{name}` must not be negative; pass 0 or more"),
                )
            }),
        }
    }

    /// Error for an action that changes the world, requested as a read.
    pub(crate) fn not_a_read(action: &str) -> Error {
        Error::new(
            ErrorCode::StageFailed,
            format!("action `{action}` changes the repository and runs as an effect stage"),
        )
    }

    /// Error for an action that only reads, requested as an effect.
    pub(crate) fn not_an_effect(action: &str) -> Error {
        Error::new(
            ErrorCode::StageFailed,
            format!("action `{action}` only reads the repository and runs as a read stage"),
        )
    }

    /// Request of `action: compare`.
    pub(crate) fn compare_request(args: &BoundArgs) -> Result<CompareRequest> {
        for name in [
            "function_context",
            "word_diff",
            "ignore_whitespace",
            "ignore_moved",
        ] {
            if args.bool(name)?.unwrap_or(false) {
                return Err(unsupported_option(name));
            }
        }
        if args.is_given("symbol") {
            return Err(unsupported_option("symbol"));
        }
        let context = args.int("context")?.unwrap_or(3);
        let context = u32::try_from(context)
            .ok()
            .filter(|value| i64::from(*value) <= MAX_CONTEXT)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`context` must be between 0 and {MAX_CONTEXT}"),
                )
            })?;
        Ok(CompareRequest {
            checkpoint: single_checkpoint(args)?,
            against: text(args, "against")?,
            stat: args.bool("stat")?.unwrap_or(false),
            context,
            paths: texts(args, "paths")?,
        })
    }

    /// Request of `action: restore`.
    pub(crate) fn restore_request(args: &BoundArgs) -> Result<RestoreRequest> {
        Ok(RestoreRequest {
            checkpoint: single_checkpoint(args)?,
            paths: texts(args, "paths")?,
            index: args.bool("index")?.unwrap_or(false),
        })
    }

    fn single_checkpoint(args: &BoundArgs) -> Result<String> {
        let mut ids = texts(args, "checkpoint_id")?;
        match ids.len() {
            1 => Ok(ids.remove(0)),
            0 => Err(Error::new(
                ErrorCode::MissingParam,
                format!("`{}` needs a value for `checkpoint_id`", args.command()),
            )),
            _ => Err(Error::new(
                ErrorCode::Arity,
                "`checkpoint_id` takes one checkpoint for this action; pass a single identifier",
            )),
        }
    }

    fn unsupported_option(name: &str) -> Error {
        Error::new(
            ErrorCode::GitUnsupported,
            format!("`{name}` is not available for checkpoint comparisons; use `git.diff` with the checkpoint reference"),
        )
        .with_problem("diff_option")
    }
}

#[cfg(test)]
mod tests {
    use super::test_journal::{Fixture, RecordingJournal, test_ctx};
    use super::*;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;

    fn args(pairs: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        for (name, value) in pairs {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn create(
        fixture: &Fixture,
        journal: &RecordingJournal,
        request: &CreateRequest,
    ) -> GitCheckpoint {
        let ctx = test_ctx();
        let plan = plan_create(&fixture.root, request, &ctx).unwrap();
        let mut tx = journal.begin(fixture.base());
        let record = apply_create(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        record
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn create_records_untracked_and_modified_files_without_touching_the_repository() {
        let fixture = Fixture::new(&[
            ("a.txt", "one\n"),
            ("src/b.txt", "two\n"),
            (".gitignore", "*.log\n"),
        ]);
        fixture.write("a.txt", "changed\n");
        fixture.write("new.txt", "new\n");
        fixture.write("debug.log", "ignored\n");
        let before = fixture.state();
        let journal = RecordingJournal::default();
        let record = create(&fixture, &journal, &CreateRequest::default());
        assert_eq!(record.id, "cp-1");
        assert_eq!(record.reference, "refs/nurt/checkpoints/cp-1");
        assert_eq!(record.label.as_deref(), Some(DEFAULT_LABEL));
        assert_eq!(record.files, Some(4));
        assert!(!record.index_included);
        assert_eq!(fixture.state_without_refs(), before);
        let repo = support::open_repo(&fixture.root).unwrap();
        let found = resolve(&repo, "cp-1", "repo").unwrap();
        let commit = read_commit(&repo, &found).unwrap();
        let files = support::flatten_tree(&repo, commit.tree).unwrap();
        assert!(files.contains_key(&BString::from("new.txt")));
        assert!(!files.contains_key(&BString::from("debug.log")));
        let blob = repo.find_blob(files[&BString::from("a.txt")].1).unwrap();
        assert_eq!(blob.data.as_slice(), b"changed\n");
        assert_eq!(commit.head, repo.head_id().ok().map(gix::Id::detach));
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }

    #[test]
    fn create_numbers_checkpoints_and_keeps_label_and_index() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        create(&fixture, &journal, &CreateRequest::default());
        let second = create(
            &fixture,
            &journal,
            &CreateRequest {
                label: Some("before refactoring\nsecond line".into()),
                index: true,
                exclude: Vec::new(),
            },
        );
        assert_eq!(second.id, "cp-2");
        assert_eq!(second.label.as_deref(), Some("before refactoring"));
        assert!(second.index_included);
        let listed = list(&fixture.root).unwrap();
        let ids: Vec<&str> = listed.iter().map(|record| record.id.as_str()).collect();
        assert_eq!(ids, ["cp-2", "cp-1"]);
        assert!(listed[0].index_included);
        assert_eq!(listed[0].label.as_deref(), Some("before refactoring"));
    }

    #[test]
    fn create_in_repository_without_commits_has_no_parent() {
        let fixture = Fixture::empty();
        fixture.write("a.txt", "one\n");
        let journal = RecordingJournal::default();
        let record = create(&fixture, &journal, &CreateRequest::default());
        let repo = support::open_repo(&fixture.root).unwrap();
        let commit = repo
            .find_commit(ObjectId::from_hex(record.commit.as_bytes()).unwrap())
            .unwrap();
        assert_eq!(commit.parent_ids().count(), 0);
        assert_eq!(record.files, Some(1));
    }

    #[test]
    fn delete_by_identifier_and_by_keep_and_undo_restores_references() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        for _ in 0..3 {
            create(&fixture, &journal, &CreateRequest::default());
        }
        let before = fixture.state();
        let plan = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec!["cp-2".into()],
                keep: None,
            },
        )
        .unwrap();
        assert_eq!(plan.references(), ["refs/nurt/checkpoints/cp-2"]);
        let mut tx = journal.begin(fixture.base());
        apply_delete(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        let ids: Vec<String> = list(&fixture.root)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["cp-3", "cp-1"]);
        journal.undo_last();
        assert_eq!(fixture.state(), before);
        let keep = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec![],
                keep: Some(1),
            },
        )
        .unwrap();
        assert_eq!(
            keep.references(),
            ["refs/nurt/checkpoints/cp-2", "refs/nurt/checkpoints/cp-1"]
        );
        let none = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec![],
                keep: Some(5),
            },
        )
        .unwrap();
        assert!(none.references().is_empty());
    }

    #[test]
    fn delete_rejects_missing_conflicting_and_unknown_selection() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let missing = plan_delete(&fixture.root, &DeleteRequest::default()).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::MissingParam);
        assert!(!missing.data().fix.is_empty());
        let both = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec!["cp-1".into()],
                keep: Some(1),
            },
        )
        .unwrap_err();
        assert_eq!(both.code(), ErrorCode::Arg);
        let unknown = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec!["cp-9".into()],
                keep: None,
            },
        )
        .unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::GitRef);
        assert_eq!(unknown.data().problem.as_deref(), Some("not_found"));
        let malformed = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec!["cp-01".into()],
                keep: None,
            },
        )
        .unwrap_err();
        assert_eq!(malformed.code(), ErrorCode::Arg);
    }

    #[test]
    fn delete_removes_packed_checkpoint_reference() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        let record = create(&fixture, &journal, &CreateRequest::default());
        let git_dir = fixture.root.join(".git");
        std::fs::remove_file(git_dir.join("refs/nurt/checkpoints/cp-1")).unwrap();
        std::fs::write(
            git_dir.join("packed-refs"),
            format!(
                "# pack-refs with: peeled fully-peeled sorted \n{} refs/nurt/checkpoints/cp-1\n",
                record.commit
            ),
        )
        .unwrap();
        assert_eq!(list(&fixture.root).unwrap().len(), 1);
        let before = fixture.state();
        let plan = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec!["cp-1".into()],
                keep: None,
            },
        )
        .unwrap();
        let mut tx = journal.begin(fixture.base());
        apply_delete(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        assert!(list(&fixture.root).unwrap().is_empty());
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }

    #[test]
    fn legacy_checkpoints_are_listed_but_read_only() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let journal = RecordingJournal::default();
        let record = create(&fixture, &journal, &CreateRequest::default());
        let legacy = fixture.root.join(".git/refs/danaco/checkpoints");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("cp-4"), format!("{}\n", record.commit)).unwrap();
        let ids: Vec<String> = list(&fixture.root)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["cp-4", "cp-1"]);
        let err = plan_delete(
            &fixture.root,
            &DeleteRequest {
                ids: vec!["cp-4".into()],
                keep: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.data().problem.as_deref(), Some("read_only"));
        let next = create(&fixture, &journal, &CreateRequest::default());
        assert_eq!(next.id, "cp-5");
    }

    #[test]
    fn existing_lock_of_reference_is_reported_as_locked() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let ctx = test_ctx();
        let plan = plan_create(&fixture.root, &CreateRequest::default(), &ctx).unwrap();
        let dir = fixture.root.join(".git/refs/nurt/checkpoints");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cp-1.lock"), b"").unwrap();
        let journal = RecordingJournal::default();
        let mut tx = journal.begin(fixture.base());
        let err = apply_create(&plan, tx.as_mut()).unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitLocked);
        assert!(!err.data().fix.is_empty());
    }

    #[test]
    fn stage_runs_create_list_and_delete_from_bound_arguments() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let ctx = test_ctx();
        let repo = Value::Path(fixture.root.to_string_lossy().into_owned());
        let command = CheckpointCommand;
        assert_eq!(command.descriptor().command, COMMAND);
        let create_args = args(&[
            ("repo", repo.clone()),
            ("action", Value::Enum("create".into())),
            ("label", text("first")),
        ]);
        let effect = command.as_effect().unwrap();
        let plan = effect.plan(&create_args, StageInput::none(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 1);
        let journal = RecordingJournal::default();
        let mut tx = journal.begin(fixture.base());
        let outcome = effect.apply(&create_args, plan, tx.as_mut(), &ctx).unwrap();
        journal.commit(tx);
        let records = records_of(outcome.output);
        assert_eq!(records[0].get("id"), Some(&text("cp-1")));
        let list_args = args(&[
            ("repo", repo.clone()),
            ("action", Value::Enum("list".into())),
        ]);
        let read = command.as_read().unwrap();
        let listed = records_of(read.execute(&list_args, StageInput::none(), &ctx).unwrap());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].get("label"), Some(&text("first")));
        let delete_args = args(&[
            ("repo", repo.clone()),
            ("action", Value::Enum("delete".into())),
            ("checkpoint_id", Value::List(vec![text("cp-1")])),
        ]);
        let plan = effect.plan(&delete_args, StageInput::none(), &ctx).unwrap();
        let mut tx = journal.begin(fixture.base());
        effect.apply(&delete_args, plan, tx.as_mut(), &ctx).unwrap();
        journal.commit(tx);
        assert!(list(&fixture.root).unwrap().is_empty());
        let wrong = effect
            .plan(&list_args, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::StageFailed);
        let unsupported = args(&[
            ("repo", repo),
            ("action", Value::Enum("compare".into())),
            ("checkpoint_id", text("cp-1")),
            ("word_diff", Value::Bool(true)),
        ]);
        let err = read
            .execute(&unsupported, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitUnsupported);
    }

    fn records_of(output: StageOutput<'_>) -> Vec<Record> {
        match output.data {
            nurt_contracts::stage::StageData::Records(stream) => {
                stream.collect::<Result<Vec<_>>>().unwrap()
            }
            _ => Vec::new(),
        }
    }

    #[test]
    fn identifiers_and_messages_round_trip() {
        assert_eq!(parse_id("cp-12"), Some(12));
        assert_eq!(parse_id("cp-0"), None);
        assert_eq!(parse_id("cp-"), None);
        assert_eq!(parse_id("x-1"), None);
        let head = ObjectId::null(gix::hash::Kind::Sha1);
        let message = checkpoint_message(
            "cp-3",
            "label: with colon",
            Some("main"),
            Some(head),
            None,
            &["*.log".to_owned()],
        );
        assert_eq!(label_of(&message, "cp-3"), "label: with colon");
        let metadata = parse_metadata(&message);
        assert_eq!(metadata.get("branch").map(String::as_str), Some("main"));
        assert_eq!(metadata.get("index").map(String::as_str), Some(NONE_VALUE));
        assert_eq!(parse_oid(&metadata["head"]), Some(head));
        assert_eq!(label_of("plain label\n", "cp-1"), "plain label");
    }

    #[test]
    fn create_leaves_out_excluded_files_and_restore_keeps_them() {
        let fixture = Fixture::new(&[("a.txt", "one\n"), ("keep.log", "tracked log\n")]);
        fixture.write("scratch/out.log", "x\n");
        fixture.write("new.txt", "new\n");
        let journal = RecordingJournal::default();
        let request = CreateRequest {
            exclude: vec!["*.log".into()],
            ..CreateRequest::default()
        };
        let plan = plan_create(&fixture.root, &request, &test_ctx()).unwrap();
        assert_eq!(plan.excluded(), 2);
        let mut tx = journal.begin(fixture.base());
        let record = apply_create(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        assert_eq!(record.files, Some(2));
        let repo = support::open_repo(&fixture.root).unwrap();
        let found = resolve(&repo, "cp-1", "repo").unwrap();
        assert_eq!(read_commit(&repo, &found).unwrap().excludes, ["*.log"]);
        fixture.write("a.txt", "changed\n");
        fixture.write("keep.log", "changed log\n");
        let restore = plan_restore(
            &fixture.root,
            &RestoreRequest {
                checkpoint: "cp-1".into(),
                ..RestoreRequest::default()
            },
        )
        .unwrap();
        let mut tx = journal.begin(fixture.base());
        apply_restore(&restore, tx.as_mut()).unwrap();
        journal.commit(tx);
        assert_eq!(fixture.read("a.txt"), "one\n");
        assert_eq!(fixture.read("keep.log"), "changed log\n");
        assert_eq!(fixture.read("scratch/out.log"), "x\n");
    }

    #[test]
    fn create_rejects_exclude_pattern_with_line_break() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let request = CreateRequest {
            exclude: vec!["a\nb".into()],
            ..CreateRequest::default()
        };
        let err = plan_create(&fixture.root, &request, &test_ctx()).unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidValue);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn stage_restore_saves_current_state_and_checks_expected_state() {
        let fixture = Fixture::new(&[("a.txt", "one\n")]);
        let ctx = test_ctx();
        let journal = RecordingJournal::default();
        create(&fixture, &journal, &CreateRequest::default());
        fixture.write("a.txt", "two\n");
        let before = fixture.state();
        let repo = Value::Path(fixture.root.to_string_lossy().into_owned());
        let restore_args = |extra: &[(&str, Value)]| {
            let mut pairs = vec![
                ("repo", repo.clone()),
                ("action", Value::Enum("restore".into())),
                ("checkpoint_id", text("cp-1")),
            ];
            pairs.extend(extra.iter().cloned());
            args(&pairs)
        };
        let command = CheckpointCommand;
        let effect = command.as_effect().unwrap();
        let stale = restore_args(&[("expect_state", text("0000"))]);
        let err = effect.plan(&stale, StageInput::none(), &ctx).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Stale);
        assert_eq!(err.data().problem.as_deref(), Some("state_changed"));
        let without = restore_args(&[("save_current", Value::Bool(false))]);
        let plan = effect.plan(&without, StageInput::none(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 1);
        let token = state_token(&fixture.root).unwrap();
        let given = restore_args(&[("expect_state", text(&token))]);
        let plan = effect.plan(&given, StageInput::none(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 2);
        let mut tx = journal.begin(fixture.base());
        let outcome = effect.apply(&given, plan, tx.as_mut(), &ctx).unwrap();
        journal.commit(tx);
        let records = records_of(outcome.output);
        assert_eq!(records[0].get("checkpoint_before"), Some(&text("cp-2")));
        assert!(records[0].get("state_token").is_some());
        assert_eq!(fixture.read("a.txt"), "one\n");
        let listed = list(&fixture.root).unwrap();
        assert_eq!(listed[0].label.as_deref(), Some("before restore of cp-1"));
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }
}
