// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.worktree` command.
//!
//! A linked working tree is registered under `<common dir>/worktrees/<name>/`
//! with the files `git` writes: `HEAD`, `ORIG_HEAD`, `commondir`, `gitdir`,
//! `index` and `logs/HEAD`. The new directory holds a `.git` file that points
//! at the registration. The directory is one journal target of kind
//! `CreateTree`: its checked out content is written into it directly and undo
//! removes the tree as a whole. Removal moves the directory into the journal
//! before the registration is deleted.
//!
//! The registration name is the last component of the directory, with a
//! number appended when the name is taken. Without `branch` and without
//! `start_point` the branch is named after the directory as well. An existing
//! branch is checked out and a missing one is created at `start_point`, or at
//! `HEAD`. A `start_point` that is not a local branch and comes without
//! `branch` is checked out detached.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use gix::bstr::{BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::entry::{Flags, Mode, Stat};
use gix::objs::tree::EntryKind;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{Expect, MutationTx};
use nurt_contracts::record::{GitEffect, GitEffectState, GitWorktree};
use nurt_contracts::report::{Change, ChangeAction};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Path as RecordPath, Record};
use sha2::{Digest, Sha256};

use crate::checkpoint::args;
use crate::checkpoint::snapshot::{Blobs, ignored_paths, index_entries, snapshot_worktree};
use crate::checkpoint::support::{
    Filters, TreeEntry, TxPlan, check_expected_state, committer, content_hash, display_path, feed,
    finish_token, flatten_tree, open_repo, problem, quote, ref_content, reflog_line, repo_error,
    serialize_index, worktree_path,
};
use crate::checkpoint::{CreatePlan, CreateRequest, plan_create};
use crate::refs_tx::LogAllRefUpdates;
use crate::repo::pathspec::is_safe_path;

/// Name of the command in the catalog.
const COMMAND: &str = "git.worktree";

/// Directory of a registration that `git` creates empty.
const REFS_DIR: &str = "refs";

/// Directory of the registrations in the common directory.
const WORKTREES_DIR: &str = "worktrees";

/// Content of `commondir` in a registration.
const COMMONDIR_CONTENT: &[u8] = b"../..\n";

/// File whose presence locks a working tree; it holds the reason.
const LOCKED_FILE: &str = "locked";

/// Prefix of branch references.
const BRANCH_PREFIX: &str = "refs/heads/";

/// Prefix of a symbolic `HEAD`.
const SYMREF_PREFIX: &str = "ref: ";

/// Prefix of the `.git` file of a linked working tree.
const GITDIR_PREFIX: &str = "gitdir: ";

/// Name of the repository directory and of the `.git` file.
const DOT_GIT: &str = ".git";

/// Reflog message `git` records when it checks out a new working tree.
const RESET_MESSAGE: &str = "reset: moving to HEAD";

/// Registration name used when the directory name has no usable character.
const FALLBACK_NAME: &str = "worktree";

/// Suffix that a reference name component must not end with.
const LOCK_SUFFIX: &str = ".lock";

/// Permission bits of executable files.
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o755;

/// Parameters of a new working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddRequest {
    /// Directory of the new working tree; a relative path is taken relative
    /// to the `repo` argument. The directory must not exist or must be empty.
    pub path: PathBuf,
    /// Branch to check out or to create; named after the directory when
    /// absent.
    pub branch: Option<String>,
    /// Revision a new branch starts from; `HEAD` when absent. Without
    /// `branch`, a local branch name is checked out and any other revision is
    /// checked out detached.
    pub start_point: Option<String>,
}

/// New working tree computed by [`plan_add`].
#[derive(Debug, Clone)]
pub struct AddPlan {
    repo: PathBuf,
    common_dir: PathBuf,
    path: PathBuf,
    name: String,
    branch: Option<String>,
    commit: ObjectId,
    tree: ObjectId,
    created_from: Option<String>,
    who: gix::actor::Signature,
    replaces_empty_dir: bool,
}

impl AddPlan {
    /// Name of the registration under `worktrees/`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Record of the working tree that [`apply_add`] creates.
    #[must_use]
    pub fn worktree(&self) -> GitWorktree {
        GitWorktree {
            path: RecordPath::new(display_path(&self.path)),
            head: self.commit.to_string(),
            detached: self.branch.is_none(),
            bare: false,
            main: false,
            branch: self.branch.clone(),
            locked: None,
            prunable: Some(false),
        }
    }

    /// Planned changes, in the form of the effect report.
    #[must_use]
    pub fn changes(&self) -> Vec<Change> {
        let mut out = vec![
            Change::new(display_path(&self.path), ChangeAction::Create),
            Change::new(display_path(&self.registration()), ChangeAction::Create),
        ];
        if let (Some(_), Some(full)) = (&self.created_from, self.full_branch()) {
            let file = ref_path(&self.common_dir, &full);
            out.push(Change::new(display_path(&file), ChangeAction::Create));
        }
        out
    }

    fn registration(&self) -> PathBuf {
        self.common_dir.join(WORKTREES_DIR).join(&self.name)
    }

    fn full_branch(&self) -> Option<String> {
        self.branch
            .as_ref()
            .map(|branch| format!("{BRANCH_PREFIX}{branch}"))
    }
}

/// Checks the request and plans a new linked working tree of the repository
/// at `repo`.
///
/// Nothing changes until [`apply_add`].
///
/// # Errors
///
/// Fails with `E_GIT_WORKTREE` and problem `path_exists` when the directory
/// exists, `already_registered` when a registration names it. Fails with
/// `E_GIT_REF` and problem `checked_out_elsewhere` when the branch is checked
/// out in another working tree, `branch_exists` when `start_point` is given
/// for an existing branch and `invalid_name` for a malformed branch name.
/// Fails with `E_GIT_REVISION` when the start point names no commit.
pub fn plan_add(repo_path: &Path, request: &AddRequest, ctx: &ExecCtx) -> Result<AddPlan> {
    let repo = open_repo(repo_path)?;
    let path = absolute_target(repo_path, &request.path);
    if std::fs::symlink_metadata(&path).is_ok() && !is_empty_directory(&path) {
        return Err(problem(
            ErrorCode::GitWorktree,
            "path_exists",
            format!(
                "`{}` already exists and is not an empty directory; pass another path",
                path.display()
            ),
        ));
    }
    let linked = registered(&repo)?;
    if linked.iter().any(|entry| same_path(&entry.path, &path)) {
        return Err(problem(
            ErrorCode::GitWorktree,
            "already_registered",
            format!(
                "`{}` is a missing but registered working tree; remove the registration first",
                path.display()
            ),
        )
        .with_fix(format!(
            "git.worktree {} action: remove path: {}",
            quote(&display_path(repo_path)),
            quote(&display_path(&path))
        )));
    }
    let (branch, start_point) = match (&request.branch, &request.start_point) {
        (None, Some(spec)) if is_local_branch(&repo, spec)? => (Some(spec.clone()), None),
        (None, Some(_)) => (None, request.start_point.clone()),
        (None, None) => (Some(raw_directory_name(&path)), None),
        (Some(branch), start) => (Some(branch.clone()), start.clone()),
    };
    let Some(branch) = branch else {
        let spec = start_point.as_deref().unwrap_or("HEAD");
        let commit = resolve_commit(&repo, spec)?;
        return finish_plan(&repo, repo_path, path, None, commit, None, ctx);
    };
    let full = format!("{BRANCH_PREFIX}{branch}");
    if gix::refs::FullName::try_from(full.as_str()).is_err() {
        return Err(problem(
            ErrorCode::GitRef,
            "invalid_name",
            format!("`{branch}` is not a valid branch name; pass another `branch`"),
        ));
    }
    let existing = repo
        .try_find_reference(full.as_str())
        .map_err(|err| repo_error(&format!("read the branch `{branch}`"), err))?
        .is_some();
    let (commit, created_from) = if existing {
        if start_point.is_some() {
            return Err(problem(
                ErrorCode::GitRef,
                "branch_exists",
                format!(
                    "a branch named `{branch}` already exists; omit `start_point` to check it out or pass another `branch`"
                ),
            ));
        }
        if let Some(other) = checked_out_at(&repo, &linked, &full)? {
            return Err(problem(
                ErrorCode::GitRef,
                "checked_out_elsewhere",
                format!(
                    "`{branch}` is checked out in `{}`; pass another `branch`",
                    other.display()
                ),
            ));
        }
        (resolve_commit(&repo, &full)?, None)
    } else {
        let start = start_point.as_deref().unwrap_or("HEAD");
        (resolve_commit(&repo, start)?, Some(start.to_owned()))
    };
    finish_plan(
        &repo,
        repo_path,
        path,
        Some(branch),
        commit,
        created_from,
        ctx,
    )
}

fn finish_plan(
    repo: &gix::Repository,
    repo_path: &Path,
    path: PathBuf,
    branch: Option<String>,
    commit: ObjectId,
    created_from: Option<String>,
    ctx: &ExecCtx,
) -> Result<AddPlan> {
    let tree = repo
        .find_commit(commit)
        .and_then(|found| found.tree_id())
        .map_err(|err| repo_error(&format!("read commit {commit}"), err))?
        .detach();
    let replaces_empty_dir = is_empty_directory(&path);
    Ok(AddPlan {
        repo: repo_path.to_path_buf(),
        common_dir: common_dir(repo),
        name: registration_name(&common_dir(repo), &path),
        path,
        branch,
        commit,
        tree,
        created_from,
        who: committer(repo, ctx),
        replaces_empty_dir,
    })
}

/// Whether `path` is a directory, not a link to one, without entries.
fn is_empty_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        && std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}

fn is_local_branch(repo: &gix::Repository, spec: &str) -> Result<bool> {
    let full = format!("{BRANCH_PREFIX}{spec}");
    if gix::refs::FullName::try_from(full.as_str()).is_err() {
        return Ok(false);
    }
    repo.try_find_reference(full.as_str())
        .map(|found| found.is_some())
        .map_err(|err| repo_error(&format!("read the branch `{spec}`"), err))
}

/// Registers and checks out a planned working tree through `tx`.
///
/// # Errors
///
/// Fails with `E_GIT_LOCKED` when the lock of a new branch is held, with
/// `E_STALE` when the branch, the directory or the registration appeared
/// after planning, with `E_GIT_UNSUPPORTED` and problem `unsafe_path` for a
/// tree with an unsafe path, and with the errors of the journal.
pub fn apply_add(plan: &AddPlan, tx: &mut dyn MutationTx) -> Result<GitWorktree> {
    let repo = open_repo(&plan.repo)?;
    let registration = plan.registration();
    let null = ObjectId::null(repo.object_hash());
    let log_all = LogAllRefUpdates::of_repo(&repo)?;
    let mut writes = TxPlan::default();
    if let (Some(source), Some(full)) = (&plan.created_from, plan.full_branch()) {
        declare_branch(&mut writes, plan, &full, null, source, log_all)?;
    }
    writes.create_dirs(&registration.join(REFS_DIR))?;
    if plan.replaces_empty_dir {
        writes.replace_with_tree(&plan.path)?;
    } else {
        writes.create_tree(&plan.path)?;
    }
    let dot_git = plan.path.join(DOT_GIT);
    writes.create_file(&dot_git, link_file(GITDIR_PREFIX, &registration))?;
    writes.create_file(&registration.join("gitdir"), link_file("", &dot_git))?;
    writes.create_file(&registration.join("commondir"), COMMONDIR_CONTENT.to_vec())?;
    let head = match plan.full_branch() {
        Some(full) => format!("{SYMREF_PREFIX}{full}\n").into_bytes(),
        None => ref_content(plan.commit),
    };
    writes.create_file(&registration.join("HEAD"), head)?;
    writes.create_file(&registration.join("ORIG_HEAD"), ref_content(plan.commit))?;
    let mut log = reflog_line(null, plan.commit, &plan.who, "");
    if plan.branch.is_some() {
        log.extend(reflog_line(
            plan.commit,
            plan.commit,
            &plan.who,
            RESET_MESSAGE,
        ));
    }
    if log_all.creates_log_for_name(b"HEAD") {
        writes.create_file(&registration.join("logs").join("HEAD"), log)?;
    }
    let index_path = registration.join("index");
    let index_target = writes.create_file(&index_path, Vec::new())?;
    writes.preserve(tx)?;
    writes.apply_until(tx, index_target)?;
    let index = checkout(&repo, plan.tree, &plan.path)?;
    writes.set_content(index_target, serialize_index(index, &index_path)?);
    writes.apply_rest(tx)?;
    Ok(plan.worktree())
}

/// Content of a link file between a working tree and its registration: the
/// prefix, the bytes of the path with `/` separators and a line feed.
fn link_file(prefix: &str, path: &Path) -> Vec<u8> {
    let mut content = prefix.as_bytes().to_vec();
    match gix::path::into_bstr(path) {
        Ok(bytes) => content.extend_from_slice(&gix::path::to_unix_separators_on_windows(bytes)),
        Err(_) => content.extend_from_slice(display_path(path).as_bytes()),
    }
    content.push(b'\n');
    content
}

fn declare_branch(
    writes: &mut TxPlan,
    plan: &AddPlan,
    full: &str,
    null: ObjectId,
    source: &str,
    log_all: LogAllRefUpdates,
) -> Result<()> {
    writes.write_locked(
        &ref_path(&plan.common_dir, full),
        ref_content(plan.commit),
        None,
    )?;
    let log = ref_path(&plan.common_dir.join("logs"), full);
    let line = reflog_line(
        null,
        plan.commit,
        &plan.who,
        &format!("branch: Created from {source}"),
    );
    match std::fs::read(&log) {
        Ok(mut old) => {
            let expect = Expect::Hash(content_hash(&old));
            old.extend(line);
            writes.write_file(&log, old, expect)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if log_all.creates_log_for_name(full.as_bytes()) {
                writes.create_file(&log, line)?;
            }
        }
        Err(err) => return Err(repo_error(&format!("read `{}`", log.display()), err)),
    }
    Ok(())
}

/// Writes the files of `tree` into the new directory `root` and returns the
/// index that records them with their file system state.
fn checkout(repo: &gix::Repository, tree: ObjectId, root: &Path) -> Result<gix::index::State> {
    let entries = flatten_tree(repo, tree)?;
    let attributes = repo
        .index_from_tree(&tree)
        .map_err(|err| repo_error("read the attributes of the tree", err))?;
    let mut filters = Filters::with_source(
        repo,
        &attributes,
        gix::worktree::stack::state::attributes::Source::IdMapping,
    )?;
    let mut state = gix::index::State::new(repo.object_hash());
    for (path, entry) in &entries {
        if !is_safe_path(path.as_bstr()) {
            return Err(problem(
                ErrorCode::GitUnsupported,
                "unsafe_path",
                format!("the tree contains the unsafe path `{path}`; check out another revision"),
            ));
        }
        let file = worktree_path(root, path.as_bstr())?;
        let mode = write_entry(repo, &mut filters, path, entry, &file)?;
        let stat = if mode == Mode::COMMIT {
            Stat::default()
        } else {
            stat_of(&file)?
        };
        state.dangerously_push_entry(stat, entry.1, Flags::empty(), mode, path.as_bstr());
    }
    state.sort_entries();
    Ok(state)
}

fn write_entry(
    repo: &gix::Repository,
    filters: &mut Filters<'_>,
    path: &BString,
    (kind, id): &TreeEntry,
    file: &Path,
) -> Result<Mode> {
    let io = |err: std::io::Error| repo_error(&format!("write `{}`", file.display()), err);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    if *kind == EntryKind::Commit {
        std::fs::create_dir_all(file).map_err(io)?;
        return Ok(Mode::COMMIT);
    }
    let blob = repo
        .find_blob(*id)
        .map_err(|err| repo_error(&format!("read the content of `{path}`"), err))?;
    match kind {
        EntryKind::Link => {
            write_link(&blob.data, file).map_err(io)?;
            Ok(Mode::SYMLINK)
        }
        EntryKind::BlobExecutable => {
            std::fs::write(
                file,
                filters.convert_to_worktree(path.as_bstr(), &blob.data)?,
            )
            .map_err(io)?;
            set_executable(file).map_err(io)?;
            Ok(Mode::FILE_EXECUTABLE)
        }
        _ => {
            std::fs::write(
                file,
                filters.convert_to_worktree(path.as_bstr(), &blob.data)?,
            )
            .map_err(io)?;
            Ok(Mode::FILE)
        }
    }
}

#[cfg(unix)]
fn write_link(target: &[u8], file: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), file)
}

/// Without symbolic links the target is written as the content of a file,
/// as `git` does with `core.symlinks=false`.
#[cfg(not(unix))]
fn write_link(target: &[u8], file: &Path) -> std::io::Result<()> {
    std::fs::write(file, target)
}

#[cfg(unix)]
fn set_executable(file: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(EXECUTABLE_MODE))
}

#[cfg(not(unix))]
fn set_executable(_file: &Path) -> std::io::Result<()> {
    Ok(())
}

fn stat_of(file: &Path) -> Result<Stat> {
    let read_error =
        |err: &dyn std::fmt::Display| repo_error(&format!("read `{}`", file.display()), err);
    let metadata =
        gix::index::fs::Metadata::from_path_no_follow(file).map_err(|err| read_error(&err))?;
    Stat::from_fs(&metadata).map_err(|err| read_error(&err))
}

/// Lists the main working tree and the linked working trees of the
/// repository at `repo`; linked ones are sorted by path.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` when `repo` does not exist,
/// `E_GIT_NOT_REPOSITORY` when it is not inside a repository and `E_IO` when
/// a registration cannot be read.
pub fn list(repo: &Path) -> Result<Vec<GitWorktree>> {
    let repo = open_repo(repo)?;
    let (main_path, bare) = main_worktree(&repo);
    let head = read_head(&common_dir(&repo).join("HEAD"))?;
    let mut out = vec![record(&repo, &main_path, &head, true, bare)?];
    let mut linked = Vec::new();
    for entry in registered(&repo)? {
        let mut found = record(&repo, &entry.path, &entry.head, false, false)?;
        found.locked = entry.locked;
        found.prunable = Some(!entry.path.exists());
        linked.push(found);
    }
    linked.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    out.extend(linked);
    Ok(out)
}

fn record(
    repo: &gix::Repository,
    path: &Path,
    head: &Head,
    main: bool,
    bare: bool,
) -> Result<GitWorktree> {
    let (commit, branch) = match head {
        Head::Detached(id) => (Some(*id), None),
        Head::Symbolic(name) => (
            branch_commit(repo, name)?,
            Some(name.strip_prefix(BRANCH_PREFIX).unwrap_or(name).to_owned()),
        ),
    };
    Ok(GitWorktree {
        path: RecordPath::new(display_path(path)),
        head: commit
            .unwrap_or_else(|| ObjectId::null(repo.object_hash()))
            .to_string(),
        detached: matches!(head, Head::Detached(_)),
        bare,
        main,
        branch,
        locked: None,
        prunable: None,
    })
}

fn branch_commit(repo: &gix::Repository, name: &str) -> Result<Option<ObjectId>> {
    let Some(mut reference) = repo
        .try_find_reference(name)
        .map_err(|err| repo_error(&format!("read `{name}`"), err))?
    else {
        return Ok(None);
    };
    let id = reference
        .peel_to_id()
        .map_err(|err| repo_error(&format!("resolve `{name}`"), err))?;
    Ok(Some(id.detach()))
}

/// Parameters of a removal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoveRequest {
    /// Directory of the linked working tree; a relative path is taken
    /// relative to the `repo` argument.
    pub path: PathBuf,
    /// Remove the working tree even when it has uncommitted changes or
    /// untracked files.
    pub force: bool,
}

/// Removal computed by [`plan_remove`].
#[derive(Debug, Clone)]
pub struct RemovePlan {
    path: PathBuf,
    registration: PathBuf,
    exists: bool,
    saved: Option<CreatePlan>,
}

impl RemovePlan {
    /// Directory of the working tree.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the directory of the working tree exists.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.exists
    }

    /// Records the content of the working tree as a checkpoint that
    /// [`apply_remove`] makes reachable in the same transaction.
    pub fn save_current(&mut self, plan: CreatePlan) {
        self.saved = Some(plan);
    }

    /// Checkpoint taken before the removal, when one is planned.
    #[must_use]
    pub fn saved(&self) -> Option<&nurt_contracts::record::GitCheckpoint> {
        self.saved.as_ref().map(CreatePlan::checkpoint)
    }

    /// Planned changes, in the form of the effect report.
    #[must_use]
    pub fn changes(&self) -> Vec<Change> {
        let mut out = Vec::new();
        if self.exists {
            out.push(Change::new(display_path(&self.path), ChangeAction::Delete));
        }
        out.push(Change::new(
            display_path(&self.registration),
            ChangeAction::Delete,
        ));
        if let Some(saved) = &self.saved {
            out.push(Change::new(
                display_path(&saved.reference_file()),
                ChangeAction::Create,
            ));
        }
        out
    }
}

/// Checks that the linked working tree at `request.path` may be removed.
///
/// # Errors
///
/// Fails with `E_GIT_WORKTREE` and problem `main_worktree` for the main
/// working tree, `not_found` for a directory that is not a registered working
/// tree and `locked` for a locked one. Fails with `E_GIT_DIRTY` and problem
/// `local_changes` when the working tree has uncommitted changes or untracked
/// files that are not ignored and `force` is not set.
pub fn plan_remove(repo_path: &Path, request: &RemoveRequest) -> Result<RemovePlan> {
    let repo = open_repo(repo_path)?;
    let shown = display_path(repo_path);
    let path = absolute_target(repo_path, &request.path);
    let (main_path, _) = main_worktree(&repo);
    if same_path(&main_path, &path) {
        return Err(problem(
            ErrorCode::GitWorktree,
            "main_worktree",
            "the main working tree cannot be removed; pass the path of a linked working tree",
        ));
    }
    let Some(entry) = registered(&repo)?
        .into_iter()
        .find(|entry| same_path(&entry.path, &path))
    else {
        return Err(problem(
            ErrorCode::GitWorktree,
            "not_found",
            format!(
                "`{}` is not a working tree of the repository; list the working trees to find it",
                path.display()
            ),
        )
        .with_fix(format!("git.worktree {} action: list", quote(&shown))));
    };
    if let Some(reason) = &entry.locked {
        let lock = entry.dir.join(LOCKED_FILE);
        let because = if reason.is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        };
        return Err(problem(
            ErrorCode::GitWorktree,
            "locked",
            format!(
                "`{}` is locked{because}; remove the lock file before removing the working tree",
                path.display()
            ),
        )
        .with_fix(format!("remove {}", quote(&display_path(&lock)))));
    }
    let exists = entry.path.exists();
    if exists && !request.force && is_dirty(&entry.path)? {
        return Err(problem(
            ErrorCode::GitDirty,
            "local_changes",
            format!(
                "`{}` contains modified or untracked files; commit them or remove with `force`",
                path.display()
            ),
        )
        .with_fix(format!(
            "git.worktree {} action: remove path: {} force: true",
            quote(&shown),
            quote(&display_path(&path))
        )));
    }
    Ok(RemovePlan {
        path: entry.path,
        registration: entry.dir,
        exists,
        saved: None,
    })
}

/// Moves the working tree directory into the journal and deletes its
/// registration through `tx`.
///
/// # Errors
///
/// Fails with the errors of the journal.
pub fn apply_remove(plan: &RemovePlan, tx: &mut dyn MutationTx) -> Result<()> {
    let mut writes = TxPlan::default();
    if let Some(saved) = &plan.saved {
        saved.declare(&mut writes, tx)?;
    }
    if plan.exists {
        writes.delete(&plan.path, Expect::Unchecked)?;
    }
    writes.delete(&plan.registration, Expect::Unchecked)?;
    writes.preserve(tx)?;
    writes.apply_rest(tx)
}

/// Token of the state the command depends on: `HEAD` of the main working
/// tree, the local branches and every registration with its `HEAD`, lock,
/// index and files. It does not depend on the working tree the command is
/// called from.
pub(crate) fn state_token(repo_path: &Path) -> Result<String> {
    let repo = open_repo(repo_path)?;
    let mut hasher = Sha256::new();
    feed(
        &mut hasher,
        &std::fs::read(common_dir(&repo).join("HEAD")).unwrap_or_default(),
    );
    for (name, id) in crate::checkpoint::refs_with_prefix(&repo, BRANCH_PREFIX)? {
        feed(&mut hasher, name.as_bytes());
        feed(&mut hasher, id.to_string().as_bytes());
    }
    for entry in registered(&repo)? {
        feed(&mut hasher, display_path(&entry.dir).as_bytes());
        feed(&mut hasher, display_path(&entry.path).as_bytes());
        feed(&mut hasher, format!("{:?}", entry.head).as_bytes());
        feed(&mut hasher, entry.locked.unwrap_or_default().as_bytes());
        feed(
            &mut hasher,
            &std::fs::read(entry.dir.join("index")).unwrap_or_default(),
        );
        if entry.path.exists() {
            feed_files(&mut hasher, &entry.path);
        }
    }
    Ok(finish_token(hasher))
}

/// Adds the files of the working tree at `path` to `hasher`; an unreadable
/// working tree adds nothing.
fn feed_files(hasher: &mut Sha256, path: &Path) {
    let Ok(repo) = open_repo(path) else {
        return;
    };
    let Ok(index) = repo.index_or_empty() else {
        return;
    };
    let Ok(mut filters) = Filters::new(&repo, &index) else {
        return;
    };
    let Ok(snapshot) = snapshot_worktree(&repo, &index, &mut filters, Blobs::HashOnly) else {
        return;
    };
    for (file, (kind, id)) in &snapshot.files {
        feed(hasher, file);
        feed(hasher, format!("{kind:?} {id}").as_bytes());
    }
    for file in ignored_paths(&repo, &index).unwrap_or_default() {
        feed(hasher, b"ignored");
        feed(hasher, &file);
    }
}

/// Whether the working tree at `path` differs from its index or the index
/// from `HEAD`, or holds untracked files that are not ignored.
fn is_dirty(path: &Path) -> Result<bool> {
    let repo = open_repo(path)?;
    let index = repo
        .index_or_empty()
        .map_err(|err| repo_error("read the index", err))?;
    let mut filters = Filters::new(&repo, &index)?;
    let snapshot = snapshot_worktree(&repo, &index, &mut filters, Blobs::HashOnly)?;
    if snapshot.conflicted || !snapshot.untracked.is_empty() {
        return Ok(true);
    }
    let Some(staged) = index_entries(&index) else {
        return Ok(true);
    };
    if snapshot.files != staged {
        return Ok(true);
    }
    let head = match repo.head_commit() {
        Ok(commit) => {
            let tree = commit
                .tree_id()
                .map_err(|err| repo_error("read the tree of `HEAD`", err))?;
            flatten_tree(&repo, tree.detach())?
        }
        Err(_) => BTreeMap::new(),
    };
    Ok(staged != head)
}

/// State of a `HEAD` file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Head {
    Symbolic(String),
    Detached(ObjectId),
}

/// Registration of a linked working tree.
#[derive(Debug, Clone)]
struct Registered {
    dir: PathBuf,
    path: PathBuf,
    head: Head,
    locked: Option<String>,
}

fn read_head(file: &Path) -> Result<Head> {
    let content = std::fs::read(file)
        .map_err(|err| repo_error(&format!("read `{}`", file.display()), err))?;
    let text = content.trim().to_str_lossy().into_owned();
    if let Some(name) = text.strip_prefix(SYMREF_PREFIX) {
        return Ok(Head::Symbolic(name.trim().to_owned()));
    }
    ObjectId::from_hex(text.as_bytes())
        .map(Head::Detached)
        .map_err(|err| repo_error(&format!("read `{}`", file.display()), err))
}

/// Registrations under `worktrees/`, by name.
fn registered(repo: &gix::Repository) -> Result<Vec<Registered>> {
    let dir = common_dir(repo).join(WORKTREES_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(repo_error(&format!("read `{}`", dir.display()), err)),
    };
    let mut out = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|err| repo_error(&format!("read `{}`", dir.display()), err))?;
        let registration = entry.path();
        let Ok(gitdir) = std::fs::read(registration.join("gitdir")) else {
            continue;
        };
        let gitdir = gix::path::from_bstr(gitdir.trim().as_bstr()).map_or_else(
            |_| PathBuf::from(gitdir.trim().to_str_lossy().into_owned()),
            std::borrow::Cow::into_owned,
        );
        let gitdir = if gitdir.is_relative() {
            absolute_target(&registration, &gitdir)
        } else {
            gitdir
        };
        let path = gitdir
            .parent()
            .map_or_else(|| gitdir.clone(), Path::to_path_buf);
        let locked = std::fs::read(registration.join(LOCKED_FILE))
            .ok()
            .map(|reason| reason.trim().to_str_lossy().into_owned());
        let head = read_head(&registration.join("HEAD"))?;
        out.insert(
            entry.file_name(),
            Registered {
                dir: registration,
                path,
                head,
                locked,
            },
        );
    }
    Ok(out.into_values().collect())
}

/// Common directory of the repository without `.` and `..` components;
/// opened from a linked working tree it is reported as
/// `<git dir>/worktrees/<name>/../..`.
fn common_dir(repo: &gix::Repository) -> PathBuf {
    absolute_target(repo.common_dir(), Path::new(""))
}

/// Directory of the main working tree and whether the repository is bare.
///
/// A repository directory that is not named `.git` belongs to the main
/// working tree when the repository was opened from it, as with a separate
/// git directory or a submodule.
fn main_worktree(repo: &gix::Repository) -> (PathBuf, bool) {
    let common = common_dir(repo);
    match common.parent() {
        Some(parent) if common.file_name().is_some_and(|name| name == DOT_GIT) => {
            (parent.to_path_buf(), false)
        }
        _ => match repo.workdir() {
            Some(workdir) if repo.git_dir() == repo.common_dir() => {
                (absolute_target(workdir, Path::new("")), false)
            }
            _ => (common.to_path_buf(), true),
        },
    }
}

fn checked_out_at(
    repo: &gix::Repository,
    linked: &[Registered],
    full: &str,
) -> Result<Option<PathBuf>> {
    let wanted = Head::Symbolic(full.to_owned());
    let (main_path, bare) = main_worktree(repo);
    if !bare && read_head(&common_dir(repo).join("HEAD"))? == wanted {
        return Ok(Some(main_path));
    }
    Ok(linked
        .iter()
        .find(|entry| entry.head == wanted)
        .map(|entry| entry.path.clone()))
}

fn resolve_commit(repo: &gix::Repository, spec: &str) -> Result<ObjectId> {
    let not_found = |err: gix::Error| {
        problem(
            ErrorCode::GitRevision,
            "not_found",
            format!("`{spec}` does not name a commit: {err}"),
        )
    };
    let id = repo.rev_parse_single(spec).map_err(not_found)?;
    let commit = id
        .object()
        .map_err(not_found)?
        .peel_to_commit()
        .map_err(not_found)?;
    Ok(commit.id)
}

/// Registration name for `path`: its last component reduced to characters
/// valid in a reference name, with the smallest number appended that makes
/// it unique.
fn registration_name(common_dir: &Path, path: &Path) -> String {
    let base = directory_name(path);
    let dir = common_dir.join(WORKTREES_DIR);
    if !dir.join(&base).exists() {
        return base;
    }
    let mut counter: u64 = 1;
    loop {
        let candidate = format!("{base}{counter}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
        counter += 1;
    }
}

/// Last component of `path` as written, the branch name `git worktree add`
/// takes when none is given; a name git would refuse is refused as well.
fn raw_directory_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn directory_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut cleaned = String::with_capacity(raw.len());
    let mut last = '\0';
    for ch in raw.chars() {
        let forbidden = ch.is_ascii_control()
            || matches!(ch, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
            || (ch == '{' && last == '@');
        let ch = if forbidden { '-' } else { ch };
        if ch == '.' && last == '.' {
            continue;
        }
        cleaned.push(ch);
        last = ch;
    }
    while cleaned.ends_with(LOCK_SUFFIX) {
        cleaned.truncate(cleaned.len() - LOCK_SUFFIX.len());
    }
    if cleaned.starts_with('.') {
        cleaned.replace_range(..1, "-");
    }
    if cleaned.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        cleaned
    }
}

/// Absolute form of `input`: relative paths are taken relative to `base`;
/// `.` and `..` are resolved lexically.
fn absolute_target(base: &Path, input: &Path) -> PathBuf {
    let joined = if input.is_absolute() {
        input.to_path_buf()
    } else {
        base.join(input)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Whether two paths name the same entry; existing entries are compared by
/// their canonical form.
fn same_path(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn ref_path(base: &Path, name: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for part in name.split('/') {
        path.push(part);
    }
    path
}

/// Implementation of `git.worktree` for the executor.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorktreeCommand;

/// Planned effect carried from [`EffectStage::plan`] to [`EffectStage::apply`].
enum Planned {
    Add(Box<AddPlan>),
    Remove(Box<RemovePlan>),
}

impl StageImpl for WorktreeCommand {
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

impl ReadStage for WorktreeCommand {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let repo = args::repo(args)?;
        match args::action(args)?.as_str() {
            "list" => {
                let records: Vec<Record> = list(&repo)?.into_iter().map(Record::from).collect();
                Ok(StageOutput::records(RecordStream::from_records(records)))
            }
            other => Err(args::not_a_read(other)),
        }
    }
}

impl EffectStage for WorktreeCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let repo = args::repo(args)?;
        let expected = args::text(args, "expect_state")?;
        if expected.is_some() {
            check_expected_state(expected.as_deref(), &state_token(&repo)?, &repo)?;
        }
        match args::action(args)?.as_str() {
            "add" => {
                let request = AddRequest {
                    path: required_path(args)?,
                    branch: args::text(args, "branch")?,
                    start_point: args::text(args, "start_point")?,
                };
                let plan = plan_add(&repo, &request, ctx)?;
                let mut effect = EffectPlan::new();
                effect.changes = plan.changes();
                Ok(effect.with_state(Planned::Add(Box::new(plan))))
            }
            "remove" => {
                let request = RemoveRequest {
                    path: required_path(args)?,
                    force: args.bool("force")?.unwrap_or(false),
                };
                let mut plan = plan_remove(&repo, &request)?;
                if args.bool("checkpoint")?.unwrap_or(false) && plan.exists() {
                    let before = CreateRequest {
                        label: Some(format!("before removal of {}", display_path(plan.path()))),
                        ..CreateRequest::default()
                    };
                    plan.save_current(plan_create(plan.path(), &before, ctx)?);
                }
                let mut effect = EffectPlan::new();
                effect.changes = plan.changes();
                effect.bulk = effect.changes.len() > 1;
                Ok(effect.with_state(Planned::Remove(Box::new(plan))))
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
        let changes = std::mem::take(&mut plan.changes);
        let notable = std::mem::take(&mut plan.notable);
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "`git.worktree` received a plan of another stage",
            )
        })?;
        let record: Record = match planned {
            Planned::Add(plan) => apply_add(&plan, tx)?.into(),
            Planned::Remove(plan) => {
                apply_remove(&plan, tx)?;
                effect_record(
                    &args::repo(args)?,
                    plan.saved().map(|saved| saved.id.clone()),
                )?
            }
        };
        let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(record)));
        outcome.changes = changes;
        outcome.notable = notable;
        Ok(outcome)
    }
}

fn required_path(args: &BoundArgs) -> Result<PathBuf> {
    args::text(args, "path")?.map(PathBuf::from).ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{COMMAND}` needs a value for `path` with this action"),
        )
    })
}

fn effect_record(repo_path: &Path, checkpoint: Option<String>) -> Result<Record> {
    let repo = open_repo(repo_path)?;
    let (main_path, _) = main_worktree(&repo);
    let head = repo.head_id().ok().map(|id| id.to_string());
    Ok(GitEffect {
        repo: RecordPath::new(display_path(&main_path)),
        state: GitEffectState::Completed,
        head_before: head.clone(),
        head_after: head,
        branch: crate::checkpoint::current_branch(&repo),
        checkpoint,
        state_token: Some(state_token(repo_path)?),
        ..GitEffect::default()
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::test_journal::{Fixture, RecordingJournal, test_ctx};
    use crate::testutil::RepoBuilder;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Value;

    fn add(fixture: &Fixture, journal: &RecordingJournal, request: &AddRequest) -> GitWorktree {
        let plan = plan_add(&fixture.root, request, &test_ctx()).unwrap();
        let mut tx = journal.begin(fixture.base());
        let record = apply_add(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        record
    }

    fn request(path: &Path, branch: Option<&str>, start_point: Option<&str>) -> AddRequest {
        AddRequest {
            path: path.to_path_buf(),
            branch: branch.map(str::to_owned),
            start_point: start_point.map(str::to_owned),
        }
    }

    fn remove(fixture: &Fixture, journal: &RecordingJournal, path: &Path, force: bool) {
        let plan = plan_remove(
            &fixture.root,
            &RemoveRequest {
                path: path.to_path_buf(),
                force,
            },
        )
        .unwrap();
        let mut tx = journal.begin(fixture.base());
        apply_remove(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn add_uses_an_existing_empty_directory_and_undo_restores_it() {
        let fixture = Fixture::new(&[("a.txt", "a\n"), ("d/b.txt", "b\n")]);
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        std::fs::create_dir(&path).unwrap();
        let before = fixture.state();
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        assert_eq!(read(&path.join("d/b.txt")), "b\n");
        journal.undo_last();
        assert_eq!(fixture.state(), before);
        assert!(path.is_dir());
    }

    #[test]
    fn add_writes_no_reflogs_when_the_repository_does_not_log_ref_updates() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file("a.txt", "a\n")
                .commit("initial")
                .config("core.logAllRefUpdates", "false"),
        );
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        assert!(!fixture.root.join(".git/worktrees/wt/logs").exists());
        assert!(!fixture.root.join(".git/logs/refs/heads/feat").exists());
        assert!(fixture.root.join(".git/refs/heads/feat").is_file());
    }

    #[test]
    fn add_refuses_a_directory_with_entries() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let path = fixture.base().join("wt");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("x"), "x").unwrap();
        let err =
            plan_add(&fixture.root, &request(&path, Some("f"), None), &test_ctx()).unwrap_err();
        assert_eq!(err.data().problem.as_deref(), Some("path_exists"));
    }

    #[test]
    fn state_token_changes_with_the_names_of_ignored_files() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file(".gitignore", "*.log\nbuild/\n")
                .file("a.txt", "a\n")
                .commit("initial"),
        );
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        let first = state_token(&fixture.root).unwrap();
        std::fs::write(path.join("debug.log"), "x").unwrap();
        let second = state_token(&fixture.root).unwrap();
        assert_ne!(first, second);
        std::fs::create_dir(path.join("build")).unwrap();
        std::fs::write(path.join("build/out.bin"), "x").unwrap();
        assert_ne!(second, state_token(&fixture.root).unwrap());
    }

    #[test]
    fn add_with_new_branch_writes_registration_like_git_and_undo_removes_it() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file("a.txt", "a\n")
                .file("d/b.txt", "b\n")
                .executable("run.sh", "#!/bin/sh\n")
                .commit("initial"),
        );
        let before = fixture.state();
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        let record = add(&fixture, &journal, &request(&path, Some("feat"), None));
        let head = fixture.rev("HEAD");
        assert_eq!(record.branch.as_deref(), Some("feat"));
        assert_eq!(record.head, head.to_string());
        assert!(!record.main);
        assert_eq!(read(&path.join("a.txt")), "a\n");
        assert_eq!(read(&path.join("d/b.txt")), "b\n");
        assert_registration_like_git(&fixture, &path, head);
        assert_eq!(fixture.rev("feat"), head);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path.join("run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        assert!(!is_dirty(&path).unwrap());
        let opened = open_repo(&path).unwrap();
        assert_eq!(
            crate::checkpoint::current_branch(&opened).as_deref(),
            Some("feat")
        );
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }

    /// Checks the registration files of `wt` with branch `feat` against the
    /// content `git worktree add` writes.
    fn assert_registration_like_git(fixture: &Fixture, path: &Path, head: ObjectId) {
        let registration = fixture.root.join(".git/worktrees/wt");
        assert_eq!(read(&registration.join("HEAD")), "ref: refs/heads/feat\n");
        assert_eq!(read(&registration.join("commondir")), "../..\n");
        assert_eq!(
            read(&registration.join("gitdir")),
            format!("{}\n", display_path(&path.join(".git")))
        );
        assert_eq!(read(&registration.join("ORIG_HEAD")), format!("{head}\n"));
        assert_eq!(
            read(&path.join(".git")),
            format!("gitdir: {}\n", display_path(&registration))
        );
        let log = read(&registration.join("logs/HEAD"));
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(!lines[0].contains('\t'));
        assert!(lines[1].ends_with("\treset: moving to HEAD"));
        assert!(
            read(&fixture.root.join(".git/logs/refs/heads/feat"))
                .ends_with("\tbranch: Created from HEAD\n")
        );
    }

    #[test]
    fn add_checks_out_existing_branch_and_names_registration_after_directory() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file("a.txt", "a\n")
                .commit("initial")
                .reference("refs/heads/side", "HEAD"),
        );
        let journal = RecordingJournal::default();
        let first = fixture.base().join("wt");
        let record = add(&fixture, &journal, &request(&first, Some("side"), None));
        assert_eq!(record.branch.as_deref(), Some("side"));
        let second = fixture.base().join("x").join("wt");
        let err = plan_add(
            &fixture.root,
            &request(&second, Some("side"), None),
            &test_ctx(),
        )
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitRef);
        assert_eq!(err.data().problem.as_deref(), Some("checked_out_elsewhere"));
        let plan = plan_add(&fixture.root, &request(&second, None, None), &test_ctx()).unwrap();
        assert_eq!(plan.name(), "wt1");
        let mut tx = journal.begin(fixture.base());
        apply_add(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        assert_eq!(
            read(&fixture.root.join(".git/worktrees/wt1/HEAD")),
            "ref: refs/heads/wt\n"
        );
        assert_eq!(read(&second.join("a.txt")), "a\n");
    }

    #[test]
    fn add_with_start_point_creates_branch_at_that_commit() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file("a.txt", "one\n")
                .commit("one")
                .file("a.txt", "two\n")
                .commit("two"),
        );
        let journal = RecordingJournal::default();
        let path = fixture.base().join("old");
        add(
            &fixture,
            &journal,
            &request(&path, Some("old"), Some("HEAD~1")),
        );
        assert_eq!(read(&path.join("a.txt")), "one\n");
        assert_eq!(fixture.rev("old"), fixture.rev("HEAD~1"));
        assert!(
            read(&fixture.root.join(".git/logs/refs/heads/old"))
                .ends_with("\tbranch: Created from HEAD~1\n")
        );
    }

    #[test]
    fn add_rejects_conflicting_requests() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let ctx = test_ctx();
        let path = fixture.base().join("wt");
        let problem_of = |request: &AddRequest| {
            let err = plan_add(&fixture.root, request, &ctx).unwrap_err();
            (err.code(), err.data().problem.clone().unwrap_or_default())
        };
        assert_eq!(
            problem_of(&request(&path, Some("main"), None)),
            (ErrorCode::GitRef, "checked_out_elsewhere".to_owned())
        );
        assert_eq!(
            problem_of(&request(&fixture.root, Some("x"), None)),
            (ErrorCode::GitWorktree, "path_exists".to_owned())
        );
        assert_eq!(
            problem_of(&request(&path, Some("main"), Some("HEAD"))),
            (ErrorCode::GitRef, "branch_exists".to_owned())
        );
        assert_eq!(
            problem_of(&request(&path, Some("new"), Some("no-such-revision"))),
            (ErrorCode::GitRevision, "not_found".to_owned())
        );
        assert_eq!(
            problem_of(&request(&path, Some("bad..name"), None)),
            (ErrorCode::GitRef, "invalid_name".to_owned())
        );
        let empty = Fixture::empty();
        let err = plan_add(
            &empty.root,
            &request(&empty.base().join("wt"), None, None),
            &ctx,
        )
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitRevision);
    }

    #[test]
    fn a_directory_name_that_is_not_a_branch_name_is_refused_like_git_does() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        for name in ["my tree", ".hidden", "a..b", "wip.lock", "tilde~1"] {
            let path = fixture.base().join(name);
            let err =
                plan_add(&fixture.root, &request(&path, None, None), &test_ctx()).expect_err(name);
            assert_eq!(err.code(), ErrorCode::GitRef, "{name}");
            assert_eq!(err.problem.as_deref(), Some("invalid_name"), "{name}");
        }
        let branch = fixture.base().join("my tree");
        let plan = plan_add(
            &fixture.root,
            &request(&branch, Some("fine"), None),
            &test_ctx(),
        );
        assert!(plan.is_ok());
    }

    #[test]
    fn list_reports_main_linked_locked_and_prunable_working_trees() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let journal = RecordingJournal::default();
        let gone = fixture.base().join("wt-a");
        let locked = fixture.base().join("wt-b");
        add(&fixture, &journal, &request(&gone, Some("a"), None));
        add(&fixture, &journal, &request(&locked, Some("b"), None));
        std::fs::write(fixture.root.join(".git/worktrees/wt-b/locked"), "on usb").unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        let records = list(&fixture.root).unwrap();
        assert_eq!(records.len(), 3);
        assert!(records[0].main);
        assert_eq!(records[0].branch.as_deref(), Some("main"));
        assert_eq!(records[0].path.as_str(), display_path(&fixture.root));
        assert_eq!(records[0].head, fixture.rev("HEAD").to_string());
        assert_eq!(records[1].path.as_str(), display_path(&gone));
        assert_eq!(records[1].prunable, Some(true));
        assert_eq!(records[1].locked, None);
        assert_eq!(records[2].locked.as_deref(), Some("on usb"));
        assert_eq!(records[2].prunable, Some(false));
        let from_linked = list(&locked).unwrap();
        assert_eq!(from_linked.len(), 3);
        assert_eq!(from_linked[0].path.as_str(), display_path(&fixture.root));
    }

    #[test]
    fn relative_gitdir_of_a_registration_is_resolved_against_the_registration() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        std::fs::write(
            fixture.root.join(".git/worktrees/wt/gitdir"),
            "../../../../wt/.git\n",
        )
        .unwrap();
        let records = list(&fixture.root).unwrap();
        assert_eq!(records[1].path.as_str(), display_path(&path));
        assert_eq!(records[1].prunable, Some(false));
        let plan = plan_remove(
            &fixture.root,
            &RemoveRequest {
                path: path.clone(),
                force: true,
            },
        )
        .unwrap();
        assert_eq!(plan.path(), path.as_path());
    }

    #[cfg(unix)]
    #[test]
    fn main_working_tree_with_a_separate_git_directory_is_not_bare() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let separate = fixture.base().join("separate.git");
        std::fs::rename(fixture.root.join(".git"), &separate).unwrap();
        std::fs::write(
            fixture.root.join(".git"),
            format!("gitdir: {}\n", separate.display()),
        )
        .unwrap();
        let records = list(&fixture.root).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].main);
        assert!(!records[0].bare);
        assert_eq!(records[0].path.as_str(), display_path(&fixture.root));
        assert_eq!(records[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn remove_clean_working_tree_and_undo_restores_directory_and_registration() {
        let fixture = Fixture::new(&[("a.txt", "a\n"), ("d/b.txt", "b\n")]);
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        let before = fixture.state();
        let plan = plan_remove(
            &fixture.root,
            &RemoveRequest {
                path: path.clone(),
                force: false,
            },
        )
        .unwrap();
        assert_eq!(plan.changes().len(), 2);
        let mut tx = journal.begin(fixture.base());
        apply_remove(&plan, tx.as_mut()).unwrap();
        journal.commit(tx);
        assert!(!path.exists());
        assert!(!fixture.root.join(".git/worktrees/wt").exists());
        assert_eq!(list(&fixture.root).unwrap().len(), 1);
        assert_eq!(fixture.rev("feat"), fixture.rev("HEAD"));
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }

    #[test]
    fn remove_refuses_modified_or_untracked_files_unless_forced() {
        let fixture = Fixture::new(&[("a.txt", "a\n"), (".gitignore", "*.log\n")]);
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        std::fs::write(path.join("debug.log"), "ignored\n").unwrap();
        let clean = RemoveRequest {
            path: path.clone(),
            force: false,
        };
        assert!(plan_remove(&fixture.root, &clean).is_ok());
        std::fs::write(path.join("u.txt"), "untracked\n").unwrap();
        let err = plan_remove(&fixture.root, &clean).unwrap_err();
        assert_eq!(err.code(), ErrorCode::GitDirty);
        assert!(!err.data().fix.is_empty());
        std::fs::remove_file(path.join("u.txt")).unwrap();
        std::fs::write(path.join("a.txt"), "changed\n").unwrap();
        assert_eq!(
            plan_remove(&fixture.root, &clean).unwrap_err().code(),
            ErrorCode::GitDirty
        );
        remove(&fixture, &journal, &path, true);
        assert!(!path.exists());
    }

    #[test]
    fn remove_rejects_main_locked_and_unknown_working_trees() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        let problem_of = |target: &Path, force: bool| {
            let err = plan_remove(
                &fixture.root,
                &RemoveRequest {
                    path: target.to_path_buf(),
                    force,
                },
            )
            .unwrap_err();
            (err.code(), err.data().problem.clone().unwrap_or_default())
        };
        assert_eq!(
            problem_of(&fixture.root, true),
            (ErrorCode::GitWorktree, "main_worktree".to_owned())
        );
        assert_eq!(
            problem_of(&fixture.base().join("other"), false),
            (ErrorCode::GitWorktree, "not_found".to_owned())
        );
        std::fs::write(fixture.root.join(".git/worktrees/wt/locked"), "").unwrap();
        assert_eq!(
            problem_of(&path, true),
            (ErrorCode::GitWorktree, "locked".to_owned())
        );
    }

    #[test]
    fn registration_names_and_paths_are_normalized() {
        assert_eq!(directory_name(Path::new("/srv/my tree")), "my-tree");
        assert_eq!(directory_name(Path::new("/srv/.hidden")), "-hidden");
        assert_eq!(directory_name(Path::new("/")), FALLBACK_NAME);
    }

    #[cfg(unix)]
    #[test]
    fn link_files_keep_the_bytes_of_a_non_utf8_path() {
        use std::os::unix::ffi::OsStrExt;
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let journal = RecordingJournal::default();
        let path = fixture.base().join(std::ffi::OsStr::from_bytes(b"w\xfft"));
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        let registration = fixture.root.join(".git/worktrees/w\u{fffd}t");
        let mut expected = path.join(".git").as_os_str().as_bytes().to_vec();
        expected.push(b'\n');
        assert_eq!(
            std::fs::read(registration.join("gitdir")).unwrap(),
            expected
        );
        let mut expected = b"gitdir: ".to_vec();
        expected.extend_from_slice(registration.as_os_str().as_bytes());
        expected.push(b'\n');
        assert_eq!(std::fs::read(path.join(".git")).unwrap(), expected);
        assert_eq!(
            list(&fixture.root).unwrap()[1].path.as_str(),
            display_path(&path)
        );
    }

    #[test]
    fn registration_names_follow_the_sanitizing_of_git() {
        let name = |text: &str| directory_name(&Path::new("/srv").join(text));
        assert_eq!(name("a  b"), "a--b");
        assert_eq!(name("my+app"), "my+app");
        assert_eq!(name("zażółć"), "zażółć");
        assert_eq!(name("a...b"), "a.b");
        assert_eq!(name("..lead"), "-lead");
        assert_eq!(name("x~y^z:w?v*u[t"), "x-y-z-w-v-u-t");
        assert_eq!(name("tab\tx"), "tab-x");
        assert_eq!(name("a@{b"), "a@-b");
        assert_eq!(name("z.lock.lock"), "z");
        assert_eq!(name("ends."), "ends.");
        assert_eq!(name(".lock"), FALLBACK_NAME);
    }

    #[test]
    fn paths_are_taken_relative_to_the_repository() {
        assert_eq!(
            absolute_target(Path::new("/srv/repo"), Path::new("../wt/./a")),
            PathBuf::from("/srv/wt/a")
        );
        assert_eq!(
            absolute_target(Path::new("/srv/repo"), Path::new("/tmp/wt")),
            PathBuf::from("/tmp/wt")
        );
    }

    fn args(pairs: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        for (name, value) in pairs {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
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
    fn stage_runs_add_list_and_remove_from_bound_arguments() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let ctx = test_ctx();
        let repo = Value::Path(fixture.root.to_string_lossy().into_owned());
        let path = Value::Path(fixture.base().join("wt").to_string_lossy().into_owned());
        let command = WorktreeCommand;
        assert_eq!(command.descriptor().command, COMMAND);
        let effect = command.as_effect().unwrap();
        let journal = RecordingJournal::default();
        let add_args = args(&[
            ("repo", repo.clone()),
            ("action", Value::Enum("add".into())),
            ("path", path.clone()),
            ("branch", Value::Text("feat".into())),
        ]);
        let plan = effect.plan(&add_args, StageInput::none(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 3);
        let mut tx = journal.begin(fixture.base());
        let outcome = effect.apply(&add_args, plan, tx.as_mut(), &ctx).unwrap();
        journal.commit(tx);
        let added = records_of(outcome.output);
        assert_eq!(added[0].get("branch"), Some(&Value::Text("feat".into())));
        let list_args = args(&[
            ("repo", repo.clone()),
            ("action", Value::Enum("list".into())),
        ]);
        let read = command.as_read().unwrap();
        let listed = records_of(read.execute(&list_args, StageInput::none(), &ctx).unwrap());
        assert_eq!(listed.len(), 2);
        let remove_args = args(&[
            ("repo", repo.clone()),
            ("action", Value::Enum("remove".into())),
            ("path", path),
        ]);
        let plan = effect.plan(&remove_args, StageInput::none(), &ctx).unwrap();
        let mut tx = journal.begin(fixture.base());
        effect.apply(&remove_args, plan, tx.as_mut(), &ctx).unwrap();
        journal.commit(tx);
        assert_eq!(list(&fixture.root).unwrap().len(), 1);
        let missing = args(&[("repo", repo), ("action", Value::Enum("add".into()))]);
        let err = effect.plan(&missing, StageInput::none(), &ctx).unwrap_err();
        assert_eq!(err.code(), ErrorCode::MissingParam);
        assert_eq!(
            effect
                .plan(&list_args, StageInput::none(), &ctx)
                .unwrap_err()
                .code(),
            ErrorCode::StageFailed
        );
    }

    #[test]
    fn add_without_branch_checks_out_revision_detached() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file("a.txt", "one\n")
                .commit("one")
                .file("a.txt", "two\n")
                .commit("two"),
        );
        let before = fixture.state();
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        let record = add(&fixture, &journal, &request(&path, None, Some("HEAD~1")));
        let head = fixture.rev("HEAD~1");
        assert!(record.detached);
        assert_eq!(record.branch, None);
        assert_eq!(record.head, head.to_string());
        assert_eq!(read(&path.join("a.txt")), "one\n");
        let registration = fixture.root.join(".git/worktrees/wt");
        assert_eq!(read(&registration.join("HEAD")), format!("{head}\n"));
        assert!(registration.join("refs").is_dir());
        let log = read(&registration.join("logs/HEAD"));
        assert_eq!(log.lines().count(), 1);
        assert!(!log.contains('\t'));
        assert!(!fixture.root.join(".git/refs/heads/wt").exists());
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }

    #[test]
    fn add_with_local_branch_as_start_point_checks_the_branch_out() {
        let fixture = Fixture::build(
            RepoBuilder::new()
                .file("a.txt", "a\n")
                .commit("initial")
                .reference("refs/heads/side", "HEAD"),
        );
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        let record = add(&fixture, &journal, &request(&path, None, Some("side")));
        assert!(!record.detached);
        assert_eq!(record.branch.as_deref(), Some("side"));
        assert_eq!(
            read(&fixture.root.join(".git/worktrees/wt/HEAD")),
            "ref: refs/heads/side\n"
        );
    }

    #[test]
    fn stage_remove_with_checkpoint_keeps_the_content_and_checks_expected_state() {
        let fixture = Fixture::new(&[("a.txt", "a\n")]);
        let ctx = test_ctx();
        let journal = RecordingJournal::default();
        let path = fixture.base().join("wt");
        add(&fixture, &journal, &request(&path, Some("feat"), None));
        std::fs::write(path.join("u.txt"), "untracked\n").unwrap();
        let before = fixture.state();
        let repo = Value::Path(fixture.root.to_string_lossy().into_owned());
        let remove_args = |extra: &[(&str, Value)]| {
            let mut pairs = vec![
                ("repo", repo.clone()),
                ("action", Value::Enum("remove".into())),
                ("path", Value::Path(path.to_string_lossy().into_owned())),
                ("force", Value::Bool(true)),
            ];
            pairs.extend(extra.iter().cloned());
            args(&pairs)
        };
        let command = WorktreeCommand;
        let effect = command.as_effect().unwrap();
        let stale = remove_args(&[("expect_state", Value::Text("0000".into()))]);
        let err = effect.plan(&stale, StageInput::none(), &ctx).unwrap_err();
        assert_eq!(err.code(), ErrorCode::Stale);
        let token = state_token(&fixture.root).unwrap();
        let given = remove_args(&[
            ("expect_state", Value::Text(token)),
            ("checkpoint", Value::Bool(true)),
        ]);
        let plan = effect.plan(&given, StageInput::none(), &ctx).unwrap();
        let mut tx = journal.begin(fixture.base());
        let outcome = effect.apply(&given, plan, tx.as_mut(), &ctx).unwrap();
        journal.commit(tx);
        let records = records_of(outcome.output);
        assert_eq!(
            records[0].get("checkpoint"),
            Some(&Value::Text("cp-1".into()))
        );
        assert!(!path.exists());
        let saved = crate::checkpoint::list(&fixture.root).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].files, Some(2));
        journal.undo_last();
        assert_eq!(fixture.state(), before);
    }
}
