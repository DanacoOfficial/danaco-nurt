// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::fetch
// Responsibility:   The `git.fetch` command.
// Dependencies:     gix, nurt_contracts, nurt_git_transport, stage
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.fetch` command.
//!
//! A fetch has two phases. [`plan_fetch`] connects to the remote, lists its
//! references, negotiates the objects to send and writes the received pack
//! into the object store, an addition that is exempt from the journal. It
//! computes the reference updates and the new content of `FETCH_HEAD` and of
//! the `shallow` file without writing them. [`write_fetch`] declares every
//! target of these writes before the first change and performs them through
//! the mutation transaction, so that undo restores the references, their
//! reflogs, `FETCH_HEAD` and `shallow` byte for byte. A pull composes the
//! plan with its own writes in one transaction.
//!
//! References are updated as by `git fetch`: a branch moves by fast-forward,
//! or by force when the refspec starts with `+`; an existing tag is never
//! moved without force; the branch checked out in the working tree is never
//! updated; tags pointing into the fetched history are followed unless
//! `remote.<name>.tagOpt` is `--no-tags`. Reflog messages are those of `git`,
//! for example `fetch origin: fast-forward`.

mod error;
pub(crate) mod head;
pub(crate) mod remote;
mod stage;
#[cfg(test)]
mod tests;
pub(crate) mod transfer;
pub(crate) mod update;

use std::path::{Path, PathBuf};

use gix::ObjectId;
use gix::protocol::fetch::Tags;
use gix::refs::{FullName, Target};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    ExemptWrite, Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec,
};
use nurt_contracts::record::{GitRefStatus, GitRefUpdate};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, NotableEntry};
use nurt_git_transport::credentials::StoreUpdate;

use crate::apply::{RefPlan, RefTarget, ReflogLine, StateFilePlan};
use crate::refs_tx::{PreviousValue, RefEdit, RefLayout, RefOutcome, RefTx};
use crate::repo::GitRepo;
use crate::repo::lock::{LockedWrite, LockedWrites, content_hash, ensure_unlocked};

pub use stage::GitFetchCommand;

/// Name of the command in the catalog.
pub const COMMAND: &str = "git.fetch";

/// File of the repository directory that lists the fetched heads.
pub const FETCH_HEAD: &str = "FETCH_HEAD";

/// File of the common directory that lists the shallow boundary.
pub const SHALLOW: &str = "shallow";

/// Which tags a fetch transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TagMode {
    /// Tags that point into the fetched history (`git fetch` without options).
    #[default]
    Auto,
    /// Every tag of the remote (`--tags`).
    All,
    /// No tags (`--no-tags`).
    None,
}

/// What to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// Name of a configured remote, a URL or a local path.
    pub remote: String,
    /// Refspecs given by the caller; empty selects `remote.<name>.fetch`.
    pub refspecs: Vec<String>,
    /// Remove remote-tracking references that no longer exist on the remote;
    /// `None` reads `remote.<name>.prune` and `fetch.prune`.
    pub prune: Option<bool>,
    /// Tags to transfer; `None` reads `remote.<name>.tagOpt`.
    pub tags: Option<TagMode>,
    /// Limit the history to this many commits from every fetched tip.
    pub depth: Option<u32>,
    /// Prefix of the reflog messages; `None` gives the prefix of the
    /// equivalent `git fetch` command line.
    pub reflog_action: Option<String>,
    /// SHA-256 fingerprint of an SSH host key to accept when the host is not
    /// known.
    pub accept_host_key: Option<String>,
}

impl FetchRequest {
    /// Fetch of `remote` with its configured refspecs and settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::fetch::FetchRequest;
    ///
    /// let request = FetchRequest::new("origin");
    /// assert!(request.refspecs.is_empty());
    /// assert_eq!(request.prune, None);
    /// ```
    #[must_use]
    pub fn new(remote: impl Into<String>) -> Self {
        Self {
            remote: remote.into(),
            refspecs: Vec::new(),
            prune: None,
            tags: None,
            depth: None,
            reflog_action: None,
            accept_host_key: None,
        }
    }
}

/// A head written to `FETCH_HEAD` for merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedHead {
    /// Fetched object.
    pub id: ObjectId,
    /// Description as written to `FETCH_HEAD`, for example
    /// `branch 'main' of https://example.com/app`.
    pub description: String,
    /// Full name of the reference on the remote.
    pub remote_ref: String,
}

/// Planned change of one local reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefChange {
    /// Full name of the local reference.
    pub name: String,
    /// Full name of the reference on the remote; `None` for a pruned
    /// reference.
    pub remote_ref: Option<String>,
    /// Value before the fetch.
    pub old: Option<ObjectId>,
    /// Value after the fetch; `None` for a pruned reference.
    pub new: Option<ObjectId>,
    /// Outcome.
    pub status: GitRefStatus,
    /// Reason of a rejection, for example `non_fast_forward`.
    pub reason: Option<String>,
    /// Complete reflog message, for example `fetch origin: fast-forward`.
    pub message: String,
}

impl RefChange {
    /// Whether the change writes the reference.
    #[must_use]
    pub fn writes(&self) -> bool {
        matches!(
            self.status,
            GitRefStatus::New
                | GitRefStatus::Updated
                | GitRefStatus::Forced
                | GitRefStatus::Deleted
        )
    }
}

/// Transfer done and local writes planned, not yet made.
#[derive(Debug, Clone, Default)]
pub struct FetchPlan {
    /// Address of the remote as written to `FETCH_HEAD`, without
    /// credentials.
    pub url: String,
    /// Name of the configured remote; `None` for an address.
    pub remote: Option<String>,
    /// Changes of local references, in the order of `git fetch`.
    pub changes: Vec<RefChange>,
    /// Complete new content of `FETCH_HEAD`.
    pub fetch_head: Vec<u8>,
    /// Heads marked for merging in `FETCH_HEAD`, in file order.
    pub for_merge: Vec<FetchedHead>,
    /// New content of the `shallow` file; empty removes the file, `None`
    /// leaves it unchanged.
    pub shallow: Option<Vec<u8>>,
    /// Objects received.
    pub objects_received: u64,
    /// Bytes of the received pack.
    pub bytes_received: u64,
    /// Unusual effects of the transfer.
    pub notable: Vec<NotableEntry>,
    /// Pending changes of credential `store` files.
    pub store_updates: Vec<StoreUpdate>,
    /// Keys of new SSH hosts accepted by the transfer, recorded in the known
    /// hosts file of Nurt when the plan is written.
    pub host_key_additions: Vec<nurt_git_transport::ssh::HostKeyAddition>,
    /// Symbolic reference `refs/remotes/<remote>/HEAD` to create and the
    /// remote-tracking branch it names.
    pub remote_head: Option<(String, String)>,
}

impl FetchPlan {
    /// Records of every reference considered by the fetch.
    #[must_use]
    pub fn ref_updates(&self) -> Vec<GitRefUpdate> {
        let remote = self.remote.clone().unwrap_or_else(|| self.url.clone());
        self.changes
            .iter()
            .map(|change| GitRefUpdate {
                reference: change.name.clone(),
                status: change.status,
                remote: remote.clone(),
                remote_ref: change.remote_ref.clone(),
                old: change.old.map(|id| id.to_string()),
                new: change.new.map(|id| id.to_string()),
                reason: change.reason.clone(),
            })
            .collect()
    }

    /// Whether no reference changes and nothing was received.
    #[must_use]
    pub fn is_up_to_date(&self) -> bool {
        self.objects_received == 0
            && self.shallow.is_none()
            && self.remote_head.is_none()
            && !self.changes.iter().any(RefChange::writes)
    }

    /// Edits of a reference transaction that perform every reference change,
    /// pruned references included.
    ///
    /// # Errors
    ///
    /// `E_GIT_REF` with problem `invalid_name` for a name that is not a valid
    /// reference name.
    pub fn ref_edits(&self) -> Result<Vec<RefEdit>> {
        let mut edits = self
            .changes
            .iter()
            .filter(|change| change.writes())
            .map(ref_edit)
            .collect::<Result<Vec<_>>>()?;
        if let Some((name, target)) = &self.remote_head {
            edits.push(RefEdit::set(
                full_name(name)?,
                Target::Symbolic(full_name(target)?),
                PreviousValue::MustNotExist,
                "fetch",
            ));
        }
        Ok(edits)
    }

    /// Reference updates in the form of a working tree plan, without the
    /// deletions of pruned references, which [`FetchPlan::ref_edits`]
    /// covers.
    ///
    /// # Errors
    ///
    /// `E_GIT_REF` for an invalid name and `E_IO` when a reference file cannot
    /// be read.
    pub fn ref_plans(
        &self,
        repo: &gix::Repository,
        committer: &gix::actor::Signature,
    ) -> Result<Vec<RefPlan>> {
        let null = repo.object_hash().null();
        self.changes
            .iter()
            .filter(|change| change.writes())
            .filter_map(|change| change.new.map(|new| (change, new)))
            .map(|(change, new)| {
                RefPlan::new(
                    repo,
                    &change.name,
                    RefTarget::Object(new),
                    Some(ReflogLine {
                        old: change.old.unwrap_or(null),
                        new,
                        committer: committer.clone(),
                        message: change.message.clone(),
                    }),
                )
            })
            .collect()
    }

    /// Writes of `FETCH_HEAD` and of the `shallow` file in the form of a
    /// working tree plan; `FETCH_HEAD` is replaced without a check, as by
    /// `git`.
    #[must_use]
    pub fn state_files(&self) -> Vec<StateFilePlan> {
        let mut files = vec![StateFilePlan {
            path: FETCH_HEAD.to_owned(),
            content: Some(self.fetch_head.clone()),
            expect: Expect::Unchecked,
        }];
        if let Some(shallow) = &self.shallow {
            files.push(StateFilePlan {
                path: SHALLOW.to_owned(),
                content: (!shallow.is_empty()).then(|| shallow.clone()),
                expect: Expect::Unchecked,
            });
        }
        files
    }
}

/// Validated reference name.
fn full_name(name: &str) -> Result<FullName> {
    FullName::try_from(name).map_err(|_| {
        Error::new(
            ErrorCode::GitRef,
            format!("`{name}` is not a valid reference name; correct the refspec"),
        )
        .with_problem("invalid_name")
        .with_found(name.to_owned())
    })
}

/// Edit of the reference transaction for one change.
fn ref_edit(change: &RefChange) -> Result<RefEdit> {
    let name = full_name(&change.name)?;
    let expected = match change.old {
        Some(old) => PreviousValue::MustExistAndMatch(Target::Object(old)),
        None => PreviousValue::MustNotExist,
    };
    Ok(match change.new {
        Some(new) => RefEdit::set(name, Target::Object(new), expected, change.message.as_str()),
        None => RefEdit::delete(name, expected, change.message.as_str()),
    })
}

/// Connects to the remote, receives the objects and plans the local writes.
///
/// The received pack is added to the object store and recorded with
/// [`MutationTx::record_exempt`]; no other target of `tx` is declared or
/// written.
///
/// # Errors
///
/// * `E_GIT_REF` with problem `unknown_remote` for a name that is neither a
///   configured remote nor an address, `invalid_refspec` for a refspec that
///   cannot be parsed, `no_match` when no refspec matches a remote
///   reference, and `checked_out` when a refspec updates the branch checked
///   out in the working tree;
/// * `E_NETWORK` with the problem `dns`, `connect`, `tls`, `timeout`,
///   `proxy` or `http_status`, `E_GIT_AUTH` and `E_GIT_HOST_KEY` for
///   failures of the connection;
/// * `E_GIT_PROTOCOL` when the remote sends data that cannot be read;
/// * `E_CANCELLED` and `E_TIMEOUT` when the operation was interrupted.
pub fn plan_fetch(
    repo: &GitRepo,
    request: &FetchRequest,
    tx: &mut dyn MutationTx,
) -> Result<FetchPlan> {
    let resolved = remote::resolve(repo, request)?;
    let shallow = remote::shallow_request(repo, request.depth)?;
    let network = transfer::Network::new(
        repo.config().plumbing(),
        &resolved.connect_url,
        resolved.name.as_deref(),
        request.accept_host_key.clone(),
        &retry_command(repo, request),
        repo.ctx(),
    )?;
    let session = transfer::Session::open(&network, repo.ctx())?;
    let ref_map = session.map(resolved.specs.clone(), resolved.extra_specs.clone())?;
    remote::check_mapping(&ref_map, &resolved)?;
    let remote_head = remote::head_symref(session.remote_refs());
    let mut received = session.receive(
        repo.gix(),
        &ref_map,
        &shallow,
        resolved.tags,
        &network,
        repo.ctx(),
    )?;
    if resolved.tags == Tags::Included {
        let missing = transfer::missing_tag_objects(repo.gix(), &ref_map);
        if !missing.is_empty() {
            let later = transfer::fetch_tag_objects(&network, repo.gix(), &missing, repo.ctx())?;
            received.absorb(&later);
        }
    }
    if received.pack_written {
        tx.record_exempt(ExemptWrite::ObjectStore {
            repo: abs_path(repo.common_dir())?,
        });
    }
    let action = request
        .reflog_action
        .clone()
        .unwrap_or_else(|| reflog_action(request, true));
    let boundary = received
        .shallow
        .as_deref()
        .map(update::shallow_commits)
        .unwrap_or_default();
    let selection = update::select(repo, &ref_map, &resolved, &action, &boundary)?;
    let remote_head =
        remote::new_remote_head(repo, &resolved, remote_head.as_deref(), &selection.changes);
    let display_url = head::display_url(&resolved.url);
    let (fetch_head, for_merge) = head::render(&selection.heads, &display_url);
    let mut notable = network.notable.clone();
    let report = network.credentials.report();
    notable.extend(report.notable);
    Ok(FetchPlan {
        url: display_url,
        remote: resolved.name,
        changes: selection.changes,
        fetch_head,
        for_merge,
        shallow: received.shallow,
        objects_received: received.objects,
        bytes_received: received.bytes,
        notable,
        store_updates: report.store_updates,
        host_key_additions: network.host_key_additions(),
        remote_head,
    })
}

/// Declares every target of `plan` before the first change and writes the
/// references with their reflogs, `FETCH_HEAD`, the `shallow` file and the
/// pending credential `store` files.
///
/// # Errors
///
/// `E_GIT_LOCKED` when a lock file of `git` exists, `E_STALE` when a
/// reference or a file changed since the plan was made, `E_GIT_REF` for a
/// reference that does not satisfy its expected value, and the errors of the
/// transaction.
pub fn write_fetch(
    repo: &GitRepo,
    plan: &FetchPlan,
    tx: &mut dyn MutationTx,
) -> Result<Vec<RefOutcome>> {
    let committer = crate::checkpoint::support::committer(repo.gix(), repo.ctx());
    let mut refs = RefTx::new(RefLayout::from_repo(repo.gix())?, committer);
    for edit in plan.ref_edits()? {
        refs.push(edit);
    }
    let mut prepared = refs.prepare()?;
    let mut writes = vec![LockedWrite::new(
        repo.git_dir().join(FETCH_HEAD),
        plan.fetch_head.clone(),
    )];
    let shallow_path = repo.common_dir().join(SHALLOW);
    let mut shallow_removal = None;
    match &plan.shallow {
        Some(content) if content.is_empty() => {
            if shallow_path.exists() {
                shallow_removal = Some(shallow_path);
            }
        }
        Some(content) => writes.push(LockedWrite::new(shallow_path, content.clone())),
        None => {}
    }
    for update in &plan.store_updates {
        writes.push(LockedWrite::new(
            update.path.clone(),
            update.contents.clone(),
        ));
    }
    let writes = LockedWrites::new(writes);
    prepared.declare(tx)?;
    let first = writes.declare(tx)?;
    let removal = match &shallow_removal {
        Some(path) => Some(declare_removal(tx, path)?),
        None => None,
    };
    let host_keys = nurt_git_transport::ssh::host_keys_plan(&plan.host_key_additions)?;
    let host_keys_first = if host_keys.is_empty() {
        None
    } else {
        tx.preserve(host_keys.targets())?
            .first()
            .map(|target| target.index)
    };
    writes.apply(tx, first)?;
    if let (Some(index), Some(path)) = (removal, &shallow_removal) {
        ensure_unlocked(path)?;
        tx.apply(index, Mutation::Remove)?;
    }
    let outcomes = prepared.apply(tx)?;
    if let Some(first) = host_keys_first {
        host_keys.apply(tx, first)?;
    }
    Ok(outcomes)
}

/// Declares the deletion of `path`, expecting its current content.
fn declare_removal(
    tx: &mut dyn MutationTx,
    path: &Path,
) -> Result<nurt_contracts::journal::TargetIndex> {
    let previous = std::fs::read(path).map_err(|failure| crate::repo::error::io(path, &failure))?;
    let preserved = tx.preserve(&[TargetSpec {
        path: TargetPath::Fs(abs_path(path)?),
        action: TargetAction::Delete,
        expect: Expect::Hash(content_hash(&previous)),
        size_hint: u64::try_from(previous.len()).ok(),
    }])?;
    preserved.first().map(|item| item.index).ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            "the journal did not declare the shallow file",
        )
    })
}

/// Fetches from the remote and writes the result in `tx`.
///
/// # Errors
///
/// As [`plan_fetch`] and [`write_fetch`].
pub fn fetch(repo: &GitRepo, request: &FetchRequest, tx: &mut dyn MutationTx) -> Result<FetchPlan> {
    let plan = plan_fetch(repo, request, tx)?;
    write_fetch(repo, &plan, tx)?;
    let mut marker = IrreversibleEntry::new(Irreversible::RemoteRefUpdated);
    marker.detail = Some(plan.url.clone());
    tx.mark_irreversible(marker);
    Ok(plan)
}

/// Prefix of the reflog messages of the equivalent command line
/// `git fetch [--prune] [--tags] [--depth=<n>] [<remote>]`; `git` names the
/// remote only when it is given.
pub(crate) fn reflog_action(request: &FetchRequest, with_remote: bool) -> String {
    let mut action = String::from("fetch");
    if request.prune == Some(true) {
        action.push_str(" --prune");
    }
    if request.tags == Some(TagMode::All) {
        action.push_str(" --tags");
    }
    if let Some(depth) = request.depth {
        action.push_str(&format!(" --depth={depth}"));
    }
    if with_remote {
        action.push(' ');
        action.push_str(&head::anonymize(&request.remote));
    }
    action
}

/// Call repeated with an accepted host key in the fix of host key errors.
fn retry_command(repo: &GitRepo, request: &FetchRequest) -> String {
    format!(
        "{COMMAND} {} remote: {}",
        crate::repo::error::quote_path(repo.root()),
        crate::repo::error::quote(&head::anonymize(&request.remote))
    )
}

/// Absolute path of a repository file.
pub(crate) fn abs_path(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|_| {
        Error::new(
            ErrorCode::Io,
            format!("repository path is not absolute: {}", path.display()),
        )
    })
}

/// Path of a reference file below the repository directory, for reports.
pub(crate) fn ref_file(git_dir: &Path, name: &str) -> PathBuf {
    name.split('/')
        .fold(git_dir.to_path_buf(), |path, part| path.join(part))
}

/// Error for a failed write below the repository directory.
pub(crate) fn error_io(path: &Path, message: &str) -> Error {
    Error::new(
        ErrorCode::Io,
        format!(
            "cannot write {}: {message}",
            crate::repo::error::quote_path(path)
        ),
    )
    .with_problem("write_failed")
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
