// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.branch` command.
//!
//! [`list`] reads local or remote-tracking branches as `branch` records in
//! the order of `git branch --list`. [`plan_create`], [`plan_rename`] and
//! [`plan_delete`] read the repository and plan the change without writing;
//! [`BranchPlan::apply`] changes the references, their reflogs and the
//! `branch.<name>` section of the repository configuration through one
//! [`MutationTx`], and checks the new branch out when the plan was made with
//! [`CreateOptions::switch`]. [`GitBranchCommand`] is the implementation used
//! by the executor.

pub(crate) mod command;
mod switch;
mod trash;

use std::path::Path;

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_ref::{FullName, Target};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    ExemptWrite, Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec,
};
use nurt_contracts::record::{Branch, GitCheckpoint};
use nurt_contracts::report::Change;
use nurt_contracts::value::{Instant, Path as RecordPath};

pub use command::GitBranchCommand;
pub use trash::{list_trash, plan_purge};

use crate::refs_tx::{PreparedRefTx, PreviousValue, RefEdit, RefLayout, RefOutcome, RefTx};
use switch::SwitchPart;

const HEADS: &str = "refs/heads/";
const REMOTES: &str = "refs/remotes/";
const TRASH: &str = "refs/nurt/trash/";

/// Options of [`plan_create_with`].
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateOptions<'a> {
    /// Check the new branch out, carrying local changes over as
    /// `git switch -c` does.
    pub switch: bool,
    /// Checkpoint whose objects are written and whose reference is created
    /// in the same transaction, before the switch.
    pub checkpoint: Option<&'a GitCheckpoint>,
}

/// Planned change of branches.
#[derive(Debug)]
pub struct BranchPlan {
    refs: PreparedRefTx,
    config: Option<ConfigWrite>,
    switch: Option<SwitchPart>,
    objects_written: bool,
    unmerged: Vec<String>,
}

/// New content of the repository configuration file.
#[derive(Debug)]
struct ConfigWrite {
    path: AbsPath,
    old: Vec<u8>,
    new: Vec<u8>,
}

impl BranchPlan {
    fn from_refs(refs: PreparedRefTx) -> Self {
        Self {
            refs,
            config: None,
            switch: None,
            objects_written: false,
            unmerged: Vec::new(),
        }
    }

    /// Outcome of every reference edit as planned.
    #[must_use]
    pub fn outcomes(&self) -> &[RefOutcome] {
        self.refs.outcomes()
    }

    /// Whether the repository configuration changes.
    #[must_use]
    pub fn changes_config(&self) -> bool {
        self.config.is_some()
    }

    /// Whether the plan checks a new branch out.
    #[must_use]
    pub fn switches(&self) -> bool {
        self.switch.is_some()
    }

    /// Whether local changes are carried over to the checked out branch.
    #[must_use]
    pub fn carries_changes(&self) -> bool {
        self.switch
            .as_ref()
            .is_some_and(SwitchPart::carries_changes)
    }

    /// Branches deleted with `force` that were not merged.
    #[must_use]
    pub fn unmerged(&self) -> &[String] {
        &self.unmerged
    }

    /// Planned changes of files, one per target, for the working tree `root`
    /// and the repository directory `git_dir`.
    #[must_use]
    pub fn changes(&self, root: &Path, git_dir: &Path) -> Vec<Change> {
        let mut out = command::target_changes(&self.refs.targets());
        if let Some(write) = &self.config {
            out.push(Change::new(
                write.path.to_language_string(),
                nurt_contracts::record::ChangeAction::Modify,
            ));
        }
        if let Some(switch) = &self.switch {
            out.extend(switch.changes(root, git_dir));
        }
        out
    }

    /// Changes the references and the configuration, and checks the new
    /// branch out when the plan switches.
    ///
    /// With a switch, the new branch is written by the checkout together
    /// with `HEAD`, as `git switch -c` writes it; the other references of
    /// the plan are declared before the checkout and written after it.
    ///
    /// # Errors
    ///
    /// `E_STALE` when the configuration changed since the plan, and the
    /// errors of [`PreparedRefTx::apply`] and of the checkout
    /// (`E_GIT_LOCKED`, `E_STALE`).
    pub fn apply(
        mut self,
        repo: &gix::Repository,
        tx: &mut dyn MutationTx,
    ) -> Result<Vec<RefOutcome>> {
        if self.objects_written {
            let layout = RefLayout::from_repo(repo)?;
            tx.record_exempt(ExemptWrite::ObjectStore {
                repo: layout.common_dir().clone(),
            });
        }
        self.refs.declare(tx)?;
        let config = match &self.config {
            Some(write) => {
                let spec = TargetSpec {
                    path: TargetPath::Fs(write.path.clone()),
                    action: TargetAction::Modify,
                    expect: Expect::Hash(content_hash(&write.old)),
                    size_hint: u64::try_from(write.old.len()).ok(),
                };
                let preserved = tx.preserve(&[spec])?;
                preserved
                    .first()
                    .map(|item| (item.index, write.new.clone()))
            }
            None => None,
        };
        if let Some(switch) = self.switch.take() {
            switch.apply(repo, tx)?;
        }
        let outcomes = self.refs.apply(tx)?;
        if let Some((index, bytes)) = config {
            tx.apply(index, Mutation::WriteBytes(bytes))?;
        }
        Ok(outcomes)
    }
}

/// Branches in the order of `git branch --list` (`remote: false`) or
/// `git branch --remotes` (`remote: true`); symbolic remote references such
/// as `origin/HEAD` are omitted.
///
/// # Errors
///
/// `E_IO` when the references, objects or configuration cannot be read.
pub fn list(repo: &gix::Repository, remote: bool) -> Result<Vec<Branch>> {
    let prefix = if remote { REMOTES } else { HEADS };
    let head = head_branch(repo)?;
    let worktrees = worktree_heads(repo)?;
    let platform = repo.references().map_err(read_error)?;
    let iter = if remote {
        platform.remote_branches()
    } else {
        platform.local_branches()
    }
    .map_err(read_error)?;
    let mut out = Vec::new();
    for reference in iter {
        let reference = reference.map_err(read_error)?;
        let Some(id) = reference.target().try_id().map(ToOwned::to_owned) else {
            continue;
        };
        let full = reference.name().to_owned();
        let short = short_name(full.as_bstr(), prefix);
        let mut record = commit_record(repo, &short, id, remote)?;
        if !remote {
            record.current = head.as_ref() == Some(&full);
            record.worktree = worktrees
                .iter()
                .find(|(branch, _)| *branch == full)
                .map(|(_, path)| RecordPath::new(path.to_language_string()));
            fill_upstream(repo, &full, id, &mut record)?;
        }
        out.push(record);
    }
    out.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    Ok(out)
}

fn commit_record(repo: &gix::Repository, name: &str, id: ObjectId, remote: bool) -> Result<Branch> {
    let commit = repo
        .find_object(id)
        .map_err(read_error)?
        .try_into_commit()
        .ok();
    let (subject, time) = match commit {
        Some(commit) => {
            let subject = commit
                .message()
                .map_err(read_error)?
                .summary()
                .to_str_lossy()
                .into_owned();
            let time = commit.time().map_err(read_error)?;
            (Some(subject), Instant::from_unix_seconds(time.seconds))
        }
        None => (None, None),
    };
    Ok(Branch {
        name: name.to_owned(),
        current: false,
        commit: id.to_hex().to_string(),
        remote,
        upstream: None,
        ahead: None,
        behind: None,
        upstream_gone: None,
        worktree: None,
        subject,
        time,
    })
}

/// Sets `upstream`, `ahead`, `behind` and `upstream_gone` of a local branch.
fn fill_upstream(
    repo: &gix::Repository,
    full: &FullName,
    id: ObjectId,
    record: &mut Branch,
) -> Result<()> {
    let Some(tracking) = upstream_ref(repo, full)? else {
        return Ok(());
    };
    record.upstream = Some(short_name(tracking.as_bstr(), REMOTES));
    let upstream_id = repo
        .try_find_reference(tracking.as_ref())
        .map_err(read_error)?
        .and_then(|reference| reference.target().try_id().map(ToOwned::to_owned));
    let Some(upstream_id) = upstream_id else {
        record.upstream_gone = Some(true);
        return Ok(());
    };
    record.upstream_gone = Some(false);
    record.ahead = Some(count_only_in(repo, id, upstream_id)?);
    record.behind = Some(count_only_in(repo, upstream_id, id)?);
    Ok(())
}

/// Remote-tracking reference configured as the upstream of `full`.
fn upstream_ref(repo: &gix::Repository, full: &FullName) -> Result<Option<FullName>> {
    match repo.branch_remote_tracking_ref_name(full.as_ref(), gix::remote::Direction::Fetch) {
        None => Ok(None),
        Some(Ok(name)) => Ok(Some(name)),
        Some(Err(_)) => Ok(None),
    }
}

/// Number of commits reachable from `tip` and not from `hidden`.
fn count_only_in(repo: &gix::Repository, tip: ObjectId, hidden: ObjectId) -> Result<u64> {
    let walk = repo
        .rev_walk([tip])
        .with_hidden([hidden])
        .all()
        .map_err(read_error)?;
    let mut count = 0u64;
    for item in walk {
        item.map_err(read_error)?;
        count += 1;
    }
    Ok(count)
}

/// Plans `git branch <name> <from>`.
///
/// When `from` names a remote-tracking branch of a configured remote and
/// `branch.autoSetupMerge` is not `false`, the new branch tracks it, as in
/// Git.
///
/// # Errors
///
/// * `E_GIT_REF` with problem `invalid_name` or `exists`;
/// * `E_GIT_REVISION` when `from` does not resolve to a commit;
/// * the errors of [`RefTx::prepare`].
pub fn plan_create(
    repo: &gix::Repository,
    name: &str,
    from: &str,
    committer: gix_actor::Signature,
) -> Result<BranchPlan> {
    plan_create_with(repo, name, from, committer, &CreateOptions::default())
}

/// Plans `git branch <name> <from>` with the options of `git.branch`:
/// checking the branch out (`git switch -c <name> <from>`) and keeping a
/// checkpoint of the working tree before the switch.
///
/// # Errors
///
/// The errors of [`plan_create`]; with [`CreateOptions::switch`] also
/// `E_GIT_DIRTY` when local changes would be overwritten and `E_GIT_CONFLICT`
/// when the index has conflicts; `E_GIT_REVISION` for a checkpoint whose
/// commit is not a valid identifier.
pub fn plan_create_with(
    repo: &gix::Repository,
    name: &str,
    from: &str,
    committer: gix_actor::Signature,
    options: &CreateOptions<'_>,
) -> Result<BranchPlan> {
    let full = branch_name(repo, name)?;
    let id = resolve_commit(repo, from)?;
    let mut refs = RefTx::new(RefLayout::from_repo(repo)?, committer.clone());
    let switch = if options.switch {
        if repo
            .try_find_reference(full.as_ref())
            .map_err(read_error)?
            .is_some()
        {
            return Err(with_ref_fixes(repo, name, exists(name)));
        }
        Some(SwitchPart::plan(repo, &full, id, from, &committer)?)
    } else {
        refs.push(RefEdit::set(
            full,
            Target::Object(id),
            PreviousValue::MustNotExist,
            format!("branch: Created from {from}"),
        ));
        None
    };
    if let Some(checkpoint) = options.checkpoint {
        let reference = FullName::try_from(checkpoint.reference.as_str())
            .map_err(|_| invalid_name(repo, &checkpoint.reference))?;
        let commit = ObjectId::from_hex(checkpoint.commit.as_bytes())
            .map_err(|error| revision_error(repo, &checkpoint.commit, &error))?;
        refs.push(RefEdit::set(
            reference,
            Target::Object(commit),
            PreviousValue::MustNotExist,
            "",
        ));
    }
    let refs = refs
        .prepare()
        .map_err(|error| with_ref_fixes(repo, name, error))?;
    let config = match tracking_setup(repo, from)? {
        Some((remote, merge)) => {
            let (path, old, mut file) = read_config(repo)?;
            let mut section = file
                .new_section("branch", Some(BString::from(name)))
                .map_err(config_error)?;
            section
                .push("remote", Some(remote.as_bstr()))
                .map_err(config_error)?;
            section
                .push("merge", Some(merge.as_bstr()))
                .map_err(config_error)?;
            Some(ConfigWrite {
                path,
                old,
                new: file.to_bstring().into(),
            })
        }
        None => None,
    };
    Ok(BranchPlan {
        config,
        switch,
        objects_written: options.checkpoint.is_some(),
        ..BranchPlan::from_refs(refs)
    })
}

/// Remote and merge reference for a branch created from `from`.
fn tracking_setup(repo: &gix::Repository, from: &str) -> Result<Option<(BString, BString)>> {
    let config = repo.config_snapshot();
    if config
        .string("branch.autoSetupMerge")
        .is_some_and(|value| is_false_value(value.as_ref()))
    {
        return Ok(None);
    }
    let Ok(Some(reference)) = repo.try_find_reference(from) else {
        return Ok(None);
    };
    let full = reference.name().as_bstr().to_owned();
    let Some(rest) = full.strip_prefix(REMOTES.as_bytes()) else {
        return Ok(None);
    };
    let Some((remote, branch)) = rest.split_once_str("/") else {
        return Ok(None);
    };
    let remote = remote.as_bstr().to_owned();
    if !repo.remote_names().contains(&remote) {
        return Ok(None);
    }
    let mut merge = BString::from(HEADS);
    merge.extend_from_slice(branch);
    Ok(Some((remote, merge)))
}

/// Whether a configuration value is one of the spellings of `false` that Git
/// accepts for a boolean: `false`, `no`, `off`, `0` and the empty string.
fn is_false_value(value: &BStr) -> bool {
    const FALSE_VALUES: [&[u8]; 5] = [b"false", b"no", b"off", b"0", b""];
    let value: &[u8] = value.trim();
    FALSE_VALUES
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

/// Plans `git branch -m <from> <to>`.
///
/// # Errors
///
/// `E_GIT_REF` with problem `invalid_name`, `missing` or `exists`, and the
/// errors of [`RefTx::prepare`].
pub fn plan_rename(
    repo: &gix::Repository,
    from: &str,
    to: &str,
    committer: gix_actor::Signature,
) -> Result<BranchPlan> {
    let old = branch_name(repo, from)?;
    let new = branch_name(repo, to)?;
    let message = format!("Branch: renamed {} to {}", old.as_bstr(), new.as_bstr());
    let mut refs = RefTx::new(RefLayout::from_repo(repo)?, committer);
    refs.push(RefEdit::rename(old, new, message));
    let refs = refs
        .prepare()
        .map_err(|error| with_ref_fixes(repo, to, error))?;
    let (path, old_bytes, mut file) = read_config(repo)?;
    let renamed = file
        .rename_section(
            "branch",
            Some(BStr::new(from)),
            "branch",
            Some(BString::from(to)),
        )
        .is_ok();
    let config = renamed.then(|| ConfigWrite {
        path,
        old: old_bytes,
        new: file.to_bstring().into(),
    });
    Ok(BranchPlan {
        config,
        ..BranchPlan::from_refs(refs)
    })
}

/// Plans `git branch -d` (or `-D` with `force`) of every name.
///
/// A branch is merged when its commit is reachable from its upstream, or
/// from `HEAD` when it has none. With `trash`, the value of every deleted
/// branch is kept as `refs/nurt/trash/<name>-<seconds>`.
///
/// # Errors
///
/// * `E_GIT_REF` with problem `missing`, `current_branch` (checked out here),
///   `checked_out` (checked out in another working tree) or `not_merged`;
/// * the errors of [`RefTx::prepare`].
pub fn plan_delete(
    repo: &gix::Repository,
    names: &[&str],
    force: bool,
    trash: bool,
    committer: gix_actor::Signature,
) -> Result<BranchPlan> {
    let head = head_branch(repo)?;
    let worktrees = worktree_heads(repo)?;
    let seconds = committer.time.seconds;
    let mut refs = RefTx::new(RefLayout::from_repo(repo)?, committer);
    let (path, old_bytes, mut file) = read_config(repo)?;
    let mut config_changed = false;
    let mut unmerged = Vec::new();
    for name in names {
        let full = branch_name(repo, name)?;
        let id = branch_id(repo, &full, name)?;
        check_deletable(repo, name, &full, head.as_ref(), &worktrees)?;
        if !is_merged(repo, &full, id)? {
            if !force {
                return Err(not_merged(repo, name));
            }
            unmerged.push((*name).to_owned());
        }
        refs.push(RefEdit::delete(
            full,
            PreviousValue::MustExistAndMatch(Target::Object(id)),
            "",
        ));
        if trash {
            let kept = FullName::try_from(format!("{TRASH}{name}-{seconds}"))
                .map_err(|_| invalid_name(repo, name))?;
            refs.push(RefEdit::set(
                kept,
                Target::Object(id),
                PreviousValue::Any,
                "",
            ));
        }
        config_changed |= file
            .remove_section("branch", Some(BStr::new(name)))
            .is_some();
    }
    let refs = refs.prepare()?;
    let config = config_changed.then(|| ConfigWrite {
        path,
        old: old_bytes,
        new: file.to_bstring().into(),
    });
    Ok(BranchPlan {
        config,
        unmerged,
        ..BranchPlan::from_refs(refs)
    })
}

fn branch_id(repo: &gix::Repository, full: &FullName, name: &str) -> Result<ObjectId> {
    repo.try_find_reference(full.as_ref())
        .map_err(read_error)?
        .and_then(|reference| reference.target().try_id().map(ToOwned::to_owned))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::GitRef,
                format!("branch {name} does not exist; list the branches to check the name"),
            )
            .with_problem("missing")
            .with_found(name.to_owned())
            .with_next_step(format!("git.branch {}", quoted_repo(repo)))
        })
}

fn check_deletable(
    repo: &gix::Repository,
    name: &str,
    full: &FullName,
    head: Option<&FullName>,
    worktrees: &[(FullName, AbsPath)],
) -> Result<()> {
    if head == Some(full) {
        return Err(Error::new(
            ErrorCode::GitRef,
            format!("branch {name} is checked out; switch to another branch before deleting it"),
        )
        .with_problem("current_branch")
        .with_found(name.to_owned())
        .with_next_step(format!(
            "git.switch {} branch: \"<other>\"",
            quoted_repo(repo)
        )));
    }
    if let Some((_, path)) = worktrees.iter().find(|(branch, _)| branch == full) {
        return Err(Error::new(
            ErrorCode::GitRef,
            format!(
                "branch {name} is checked out in the working tree {}; switch that working tree to another branch first",
                path.to_language_string()
            ),
        )
        .with_problem("checked_out")
        .with_found(name.to_owned()));
    }
    Ok(())
}

/// Whether `id` is reachable from the upstream of `full`, or from `HEAD`
/// when the upstream is not configured or missing.
fn is_merged(repo: &gix::Repository, full: &FullName, id: ObjectId) -> Result<bool> {
    let upstream = match upstream_ref(repo, full)? {
        Some(name) => repo
            .try_find_reference(name.as_ref())
            .map_err(read_error)?
            .and_then(|reference| reference.target().try_id().map(ToOwned::to_owned)),
        None => None,
    };
    let reference = match upstream {
        Some(id) => Some(id),
        None => repo.head_id().ok().map(gix::Id::detach),
    };
    let Some(reference) = reference else {
        return Ok(false);
    };
    if reference == id {
        return Ok(true);
    }
    Ok(matches!(repo.merge_base(id, reference), Ok(Some(base)) if base.detach() == id))
}

fn not_merged(repo: &gix::Repository, name: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("branch {name} is not fully merged; merge it first or delete it with force: true"),
    )
    .with_problem("not_merged")
    .with_found(name.to_owned())
    .with_fix(format!(
        "git.branch {} delete: {} force: true",
        quoted_repo(repo),
        serde_json::Value::String(name.to_owned())
    ))
}

/// Branch `HEAD` of this working tree points to.
fn head_branch(repo: &gix::Repository) -> Result<Option<FullName>> {
    let head = repo.head().map_err(read_error)?;
    Ok(head.referent_name().map(ToOwned::to_owned))
}

/// Branches checked out in the other working trees, with their directories.
fn worktree_heads(repo: &gix::Repository) -> Result<Vec<(FullName, AbsPath)>> {
    let own = repo.git_dir().to_path_buf();
    let mut dirs = vec![repo.common_dir().to_path_buf()];
    let linked = repo.common_dir().join("worktrees");
    if let Ok(entries) = std::fs::read_dir(&linked) {
        for entry in entries.flatten() {
            dirs.push(entry.path());
        }
    }
    let mut out = Vec::new();
    for dir in dirs.into_iter().filter(|dir| *dir != own) {
        let Ok(head) = std::fs::read(dir.join("HEAD")) else {
            continue;
        };
        let Some(target) = head.trim().strip_prefix(b"ref:") else {
            continue;
        };
        let Ok(branch) = FullName::try_from(target.trim().as_bstr()) else {
            continue;
        };
        if let Some(path) = worktree_dir(repo, &dir) {
            out.push((branch, path));
        }
    }
    Ok(out)
}

/// Working tree directory of a repository directory of the common
/// repository or of a linked working tree.
fn worktree_dir(repo: &gix::Repository, dir: &std::path::Path) -> Option<AbsPath> {
    if dir == repo.common_dir() {
        return repo
            .common_dir()
            .parent()
            .and_then(|path| AbsPath::new(path).ok());
    }
    let gitdir = std::fs::read(dir.join("gitdir")).ok()?;
    let gitfile = gix_path::from_bstr(gitdir.trim().as_bstr()).ok()?;
    gitfile.parent().and_then(|path| AbsPath::new(path).ok())
}

/// Full name of the branch `name`.
fn branch_name(repo: &gix::Repository, name: &str) -> Result<FullName> {
    if name.is_empty() || name.starts_with('-') || name == "HEAD" {
        return Err(invalid_name(repo, name));
    }
    FullName::try_from(format!("{HEADS}{name}")).map_err(|_| invalid_name(repo, name))
}

fn invalid_name(repo: &gix::Repository, name: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("{name} is not a valid branch name; use a name accepted by git check-ref-format"),
    )
    .with_problem("invalid_name")
    .with_found(name.to_owned())
    .with_next_step(format!("git.branch {}", quoted_repo(repo)))
}

fn resolve_commit(repo: &gix::Repository, revision: &str) -> Result<ObjectId> {
    let id = repo
        .rev_parse_single(revision)
        .map_err(|error| revision_error(repo, revision, &error))?;
    let commit = id
        .object()
        .map_err(read_error)?
        .peel_to_commit()
        .map_err(|error| revision_error(repo, revision, &error))?;
    Ok(commit.id)
}

fn exists(name: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("a branch named {name} already exists; switch to it or choose another name"),
    )
    .with_problem("exists")
    .with_found(name.to_owned())
}

/// Adds ready programs to errors about an existing branch.
fn with_ref_fixes(repo: &gix::Repository, name: &str, error: Error) -> Error {
    if error.problem.as_deref() != Some("exists") {
        return error;
    }
    let quoted = serde_json::Value::String(name.to_owned());
    error
        .with_fix(format!("git.switch {} branch: {quoted}", quoted_repo(repo)))
        .with_fix(format!("git.branch {} delete: {quoted}", quoted_repo(repo)))
}

/// Short name below `prefix`.
fn short_name(full: &BStr, prefix: &str) -> String {
    full.strip_prefix(prefix.as_bytes())
        .unwrap_or(full)
        .to_str_lossy()
        .into_owned()
}

/// Repository argument of ready programs: the working tree, or the
/// repository directory of a bare repository, as a quoted string.
pub(crate) fn quoted_repo(repo: &gix::Repository) -> String {
    let path = repo.workdir().unwrap_or_else(|| repo.git_dir());
    let text = AbsPath::new(path).map_or_else(
        |_| path.to_string_lossy().into_owned(),
        |path| path.to_language_string(),
    );
    serde_json::Value::String(text).to_string()
}

/// `E_GIT_REVISION` for a revision that cannot be resolved.
pub(crate) fn revision_error(
    repo: &gix::Repository,
    revision: &str,
    error: &dyn std::fmt::Display,
) -> Error {
    Error::new(
        ErrorCode::GitRevision,
        format!("revision {revision} cannot be resolved: {error}; check the name in the history"),
    )
    .with_found(revision.to_owned())
    .with_next_step(format!("git.log {}", quoted_repo(repo)))
}

/// Configuration file of the repository: path, raw content and parsed form.
fn read_config(repo: &gix::Repository) -> Result<(AbsPath, Vec<u8>, gix_config::File)> {
    let path = repo.common_dir().join("config");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(Error::new(
                ErrorCode::Io,
                format!("cannot read {}: {error}", path.display()),
            ));
        }
    };
    let meta = gix_config::file::Metadata::from(gix_config::Source::Local);
    let file = gix_config::File::from_bytes_owned(
        &mut bytes.clone(),
        meta,
        gix_config::file::init::Options::default(),
    )
    .map_err(config_error)?;
    let path = AbsPath::new(&path).map_err(|error| Error::new(ErrorCode::Io, error.to_string()))?;
    Ok((path, bytes, file))
}

fn config_error(error: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::GitUnsupported,
        format!("the repository configuration cannot be edited: {error}; repair the config file"),
    )
    .with_problem("invalid_config")
}

fn read_error(error: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorCode::Io,
        format!("cannot read the repository: {error}"),
    )
}

fn content_hash(bytes: &[u8]) -> nurt_contracts::fs::ContentHash {
    use sha2::Digest;
    nurt_contracts::fs::ContentHash(sha2::Sha256::digest(bytes).into())
}

#[cfg(test)]
pub(crate) mod test_repo {
    //! Repositories with commits built without an external program.

    use std::path::PathBuf;

    use gix_hash::ObjectId;
    use nurt_contracts::error::Result;
    use nurt_contracts::journal::{MutationTx, RollbackReason};

    use crate::refs_tx::test_journal::{TestJournal, assert_only_declared_changes, tree_state};

    pub(crate) const IDENT: &str = "Nurt Test <nurt@example.com> 1767225600 +0100";

    /// Repository with a working tree in a temporary directory.
    pub(crate) struct TestRepo {
        dir: tempfile::TempDir,
        journal: TestJournal,
    }

    impl TestRepo {
        pub(crate) fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            gix::init(dir.path()).unwrap();
            Self {
                dir,
                journal: TestJournal::new(),
            }
        }

        pub(crate) fn open(&self) -> gix::Repository {
            gix::open(self.dir.path()).unwrap()
        }

        pub(crate) fn git_dir(&self) -> PathBuf {
            self.dir.path().join(".git")
        }

        pub(crate) fn path(&self, relative: &str) -> PathBuf {
            self.git_dir().join(relative)
        }

        pub(crate) fn read(&self, relative: &str) -> String {
            std::fs::read_to_string(self.path(relative)).unwrap()
        }

        pub(crate) fn write(&self, relative: &str, content: &str) {
            let path = self.path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        pub(crate) fn append_config(&self, text: &str) {
            let mut config = self.read("config");
            config.push_str(text);
            self.write("config", &config);
        }

        pub(crate) fn ident(&self) -> &'static str {
            IDENT
        }

        pub(crate) fn signature(&self) -> gix_actor::Signature {
            gix_actor::Signature {
                name: "Nurt Test".into(),
                email: "nurt@example.com".into(),
                time: gix_date::Time {
                    seconds: 1_767_225_600,
                    offset: 3600,
                },
            }
        }

        /// Commit with an empty tree on top of the current branch.
        pub(crate) fn commit(&self, message: &str) -> ObjectId {
            let repo = self.open();
            let tree = repo
                .write_object(gix_object::Tree::empty())
                .unwrap()
                .detach();
            let parents: Vec<ObjectId> = repo
                .head_id()
                .ok()
                .map(gix::Id::detach)
                .into_iter()
                .collect();
            let commit = gix_object::Commit {
                tree,
                parents: parents.into(),
                author: self.signature(),
                committer: self.signature(),
                encoding: None,
                message: format!("{message}\n").into(),
                extra_headers: Vec::new(),
            };
            let id = repo.write_object(&commit).unwrap().detach();
            let branch = match repo.head_name().unwrap() {
                Some(name) => name.as_bstr().to_string(),
                None => "HEAD".to_owned(),
            };
            self.write(&branch, &format!("{id}\n"));
            id
        }

        /// Runs `change` in a recording transaction, checks that only
        /// declared targets and the object store changed, commits, and checks
        /// that undo restores the repository; then restores the state after
        /// the change.
        pub(crate) fn apply<T>(
            &self,
            change: impl FnOnce(&mut dyn MutationTx) -> Result<T>,
        ) -> Result<T> {
            let objects = self.path("objects");
            let snapshot = || {
                let mut state = tree_state(&self.git_dir());
                state.retain(|path, _| !path.starts_with(&objects));
                state
            };
            let before = snapshot();
            let mut tx = self.journal.begin();
            let result = change(tx.as_mut());
            let value = match result {
                Ok(value) => value,
                Err(error) => {
                    tx.rollback(RollbackReason::TargetFailed).unwrap();
                    assert_eq!(snapshot(), before, "failed change left files behind");
                    return Err(error);
                }
            };
            let after = snapshot();
            assert_only_declared_changes(&before, &after, tx.declared(), &[&objects]);
            tx.commit().unwrap();
            if after != before {
                let full_after = tree_state(&self.git_dir());
                self.journal.undo_last();
                assert_eq!(snapshot(), before, "undo did not restore the repository");
                for (path, content) in full_after {
                    if path.starts_with(&objects) {
                        continue;
                    }
                    match content {
                        None => std::fs::create_dir_all(&path).unwrap(),
                        Some(bytes) => {
                            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                            std::fs::write(&path, bytes).unwrap();
                        }
                    }
                }
                for path in snapshot().keys() {
                    if !after.contains_key(path) && path.exists() {
                        if path.is_dir() {
                            std::fs::remove_dir_all(path).unwrap();
                        } else {
                            std::fs::remove_file(path).unwrap();
                        }
                    }
                }
            }
            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_repo::{IDENT, TestRepo};
    use super::*;

    fn null() -> String {
        "0".repeat(40)
    }

    #[test]
    fn create_writes_branch_and_reflog_like_git() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        let plan = plan_create(&repo.open(), "topic", "HEAD", repo.signature()).unwrap();
        assert!(!plan.changes_config());
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert_eq!(repo.read("refs/heads/topic"), format!("{head}\n"));
        assert_eq!(
            repo.read("logs/refs/heads/topic"),
            format!("{} {head} {IDENT}\tbranch: Created from HEAD\n", null())
        );
    }

    #[test]
    fn create_from_a_remote_tracking_branch_sets_the_upstream() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.append_config("[remote \"origin\"]\n\turl = /nowhere\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n");
        repo.write("refs/remotes/origin/feature", &format!("{head}\n"));
        let plan =
            plan_create(&repo.open(), "feature", "origin/feature", repo.signature()).unwrap();
        assert!(plan.changes_config());
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        let config = repo.read("config");
        assert!(
            config.ends_with(
                "[branch \"feature\"]\n\tremote = origin\n\tmerge = refs/heads/feature\n"
            ),
            "{config}"
        );
        let branches = list(&repo.open(), false).unwrap();
        let feature = branches.iter().find(|b| b.name == "feature").unwrap();
        assert_eq!(feature.upstream.as_deref(), Some("origin/feature"));
        assert_eq!((feature.ahead, feature.behind), (Some(0), Some(0)));
        assert_eq!(feature.upstream_gone, Some(false));
        let remotes = list(&repo.open(), true).unwrap();
        assert_eq!(remotes[0].name, "origin/feature");
        assert!(remotes[0].remote);
    }

    #[test]
    fn auto_setup_merge_accepts_every_false_spelling() {
        for value in ["false", "no", "off", "0", "FALSE"] {
            let repo = TestRepo::new();
            let head = repo.commit("first");
            repo.append_config(&format!(
                "[branch]\n\tautoSetupMerge = {value}\n[remote \"origin\"]\n\turl = /nowhere\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
            ));
            repo.write("refs/remotes/origin/feature", &format!("{head}\n"));
            let plan =
                plan_create(&repo.open(), "feature", "origin/feature", repo.signature()).unwrap();
            assert!(!plan.changes_config(), "{value}");
        }
    }

    #[test]
    fn create_rejects_existing_invalid_and_unknown_start_points() {
        let repo = TestRepo::new();
        repo.commit("first");
        let current = repo.open().head_name().unwrap().unwrap();
        let short = short_name(current.as_bstr(), HEADS);
        let error = plan_create(&repo.open(), &short, "HEAD", repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("exists"));
        assert_eq!(error.fix.len(), 2);
        for bad in ["", "-x", "HEAD", "a..b", "a b", "x.lock"] {
            let error = plan_create(&repo.open(), bad, "HEAD", repo.signature()).unwrap_err();
            assert_eq!(error.problem.as_deref(), Some("invalid_name"), "{bad}");
        }
        let error = plan_create(&repo.open(), "t", "nope", repo.signature()).unwrap_err();
        assert_eq!(error.code, ErrorCode::GitRevision);
        assert!(error.next_steps[0].starts_with("git.log "));
    }

    #[test]
    fn list_marks_current_branch_and_reports_ahead_behind() {
        let repo = TestRepo::new();
        let first = repo.commit("first");
        let second = repo.commit("second subject\n\nbody");
        repo.append_config("[remote \"origin\"]\n\turl = /nowhere\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n");
        let current = repo.open().head_name().unwrap().unwrap();
        let short = short_name(current.as_bstr(), HEADS);
        repo.append_config(&format!(
            "[branch \"{short}\"]\n\tremote = origin\n\tmerge = refs/heads/{short}\n"
        ));
        repo.write(
            &format!("refs/remotes/origin/{short}"),
            &format!("{first}\n"),
        );
        repo.write("refs/heads/gone", &format!("{first}\n"));
        repo.append_config("[branch \"gone\"]\n\tremote = origin\n\tmerge = refs/heads/gone\n");
        let branches = list(&repo.open(), false).unwrap();
        let main = branches.iter().find(|b| b.name == short).unwrap();
        assert!(main.current);
        assert_eq!(main.commit, second.to_string());
        assert_eq!(main.subject.as_deref(), Some("second subject"));
        assert_eq!((main.ahead, main.behind), (Some(1), Some(0)));
        let gone = branches.iter().find(|b| b.name == "gone").unwrap();
        assert!(!gone.current);
        assert_eq!(gone.upstream_gone, Some(true));
        assert!(branches.windows(2).all(|w| w[0].name < w[1].name));
    }

    #[test]
    fn rename_moves_ref_reflog_head_and_config_section() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        let current = repo.open().head_name().unwrap().unwrap();
        let short = short_name(current.as_bstr(), HEADS);
        repo.append_config(&format!(
            "[branch \"{short}\"]\n\tremote = origin\n\tmerge = refs/heads/{short}\n"
        ));
        let plan = plan_rename(&repo.open(), &short, "trunk", repo.signature()).unwrap();
        assert!(plan.changes_config());
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert_eq!(repo.read("HEAD"), "ref: refs/heads/trunk\n");
        assert_eq!(repo.read("refs/heads/trunk"), format!("{head}\n"));
        assert!(
            repo.read("config")
                .contains("[branch \"trunk\"]\n\tremote = origin\n\tmerge = refs/heads/")
        );
        assert!(
            !repo
                .read("config")
                .contains(&format!("[branch \"{short}\"]"))
        );
        let message = format!("Branch: renamed refs/heads/{short} to refs/heads/trunk");
        assert!(repo.read("logs/HEAD").ends_with(&format!(
            "{head} {} {IDENT}\t{message}\n{} {head} {IDENT}\t{message}\n",
            null(),
            null()
        )));
    }

    #[test]
    fn rename_rejects_an_existing_target_and_a_missing_source() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write("refs/heads/other", &format!("{head}\n"));
        let current = repo.open().head_name().unwrap().unwrap();
        let short = short_name(current.as_bstr(), HEADS);
        let error = plan_rename(&repo.open(), &short, "other", repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("exists"));
        let error = plan_rename(&repo.open(), "nope", "fresh", repo.signature()).unwrap_err();
        assert_eq!(error.code, ErrorCode::GitRef);
        let error = plan_rename(&repo.open(), &short, "a..b", repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("invalid_name"));
    }

    #[test]
    fn delete_checks_merge_state_and_current_branch() {
        let repo = TestRepo::new();
        let first = repo.commit("first");
        let second = repo.commit("second");
        let current = repo.open().head_name().unwrap().unwrap();
        let short = short_name(current.as_bstr(), HEADS);
        repo.write("refs/heads/merged", &format!("{first}\n"));
        let ahead = {
            let r = repo.open();
            let tree = r.write_object(gix_object::Tree::empty()).unwrap().detach();
            let commit = gix_object::Commit {
                tree,
                parents: vec![second].into(),
                author: repo.signature(),
                committer: repo.signature(),
                encoding: None,
                message: "side\n".into(),
                extra_headers: Vec::new(),
            };
            r.write_object(&commit).unwrap().detach()
        };
        repo.write("refs/heads/side", &format!("{ahead}\n"));
        repo.append_config("[branch \"merged\"]\n\tremote = origin\n");
        let error =
            plan_delete(&repo.open(), &["side"], false, false, repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("not_merged"));
        assert!(error.fix[0].ends_with("delete: \"side\" force: true"));
        let error =
            plan_delete(&repo.open(), &[&short], true, false, repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("current_branch"));
        let error =
            plan_delete(&repo.open(), &["none"], true, false, repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("missing"));
        let plan = plan_delete(
            &repo.open(),
            &["merged", "side"],
            true,
            false,
            repo.signature(),
        )
        .unwrap();
        assert!(plan.changes_config());
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert!(!repo.path("refs/heads/merged").exists());
        assert!(!repo.path("refs/heads/side").exists());
        assert!(!repo.read("config").contains("[branch \"merged\"]"));
    }

    #[test]
    fn delete_with_trash_keeps_the_value_under_refs_nurt_trash() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write("packed-refs", &format!("{head} refs/heads/old\n"));
        let plan = plan_delete(&repo.open(), &["old"], false, true, repo.signature()).unwrap();
        assert_eq!(plan.outcomes().len(), 2);
        repo.apply(|tx| plan.apply(&repo.open(), tx)).unwrap();
        assert_eq!(
            repo.read("refs/nurt/trash/old-1767225600"),
            format!("{head}\n")
        );
        assert!(!repo.read("packed-refs").contains("refs/heads/old"));
    }

    #[test]
    fn delete_refuses_a_branch_checked_out_in_another_worktree() {
        let repo = TestRepo::new();
        let head = repo.commit("first");
        repo.write("refs/heads/wt", &format!("{head}\n"));
        repo.write("worktrees/wt/HEAD", "ref: refs/heads/wt\n");
        let elsewhere = repo
            .git_dir()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("wt/.git");
        repo.write("worktrees/wt/gitdir", &format!("{}\n", elsewhere.display()));
        let error = plan_delete(&repo.open(), &["wt"], true, false, repo.signature()).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("checked_out"));
        let branches = list(&repo.open(), false).unwrap();
        let wt = branches.iter().find(|b| b.name == "wt").unwrap();
        assert!(wt.worktree.as_ref().unwrap().as_str().ends_with("/wt"));
    }

    #[test]
    fn repository_argument_is_the_quoted_working_tree() {
        let repo = TestRepo::new();
        let quoted = quoted_repo(&repo.open());
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
        assert!(!quoted.ends_with(".git\""));
    }
}
