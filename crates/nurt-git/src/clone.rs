// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.clone` command.
//!
//! The new directory is one journal target of kind `CreateTree`: the
//! repository, the received pack and the checked out files are written into
//! it directly, and undo removes the tree as a whole. A failure after the
//! directory was created rolls the transaction back, which removes it.
//!
//! The result has the layout `git clone` leaves: the remote `origin` with
//! the refspec `+refs/heads/*:refs/remotes/origin/*`, the remote-tracking
//! references and tags in `packed-refs`, `refs/remotes/origin/HEAD` naming
//! the default branch of the remote, the local branch with its upstream
//! configuration, reflog entries `clone: from <address>` for `HEAD`, the
//! branch and `refs/remotes/origin/HEAD`, and a checkout of the branch. A
//! clone with `depth` fetches only the branch it checks out and the tags
//! pointing into its history; a bare clone maps the branches of the remote
//! to local branches and keeps no reflog.

mod direct;
mod layout;
mod stage;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use gix::ObjectId;
use gix::protocol::fetch::{Shallow, Tags};
use gix::protocol::handshake::Ref;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::{GitRefStatus, GitRefUpdate};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;

use crate::apply::{ApplyContext, apply_plan, plan_new_tree};
use crate::fetch::transfer::{Network, Received, Session};
use crate::fetch::{abs_path, head, remote, update};
use crate::repo::GitRepo;
use crate::repo::error::quote_path;

use self::direct::DirectTx;
use self::layout::{Layout, RemoteHead};

pub use stage::GitCloneCommand;

/// Name of the command in the catalog.
pub const COMMAND: &str = "git.clone";

/// Name of the remote of a clone.
pub const REMOTE: &str = "origin";

/// Permission bits of the new directory, before the umask.
const DIR_MODE: u32 = 0o777;

/// What to clone and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneRequest {
    /// Address of the repository: `https://`, `ssh://`, `user@host:path`,
    /// `file://` or an absolute path.
    pub url: String,
    /// Directory to create; it must not exist and its parent must.
    pub to: PathBuf,
    /// Limit the history to this many commits of the checked out branch.
    pub depth: Option<u32>,
    /// Branch to check out instead of the default branch of the remote.
    pub branch: Option<String>,
    /// Create a repository without a working tree.
    pub bare: bool,
    /// SHA-256 fingerprint of an SSH host key to accept when the host is not
    /// known.
    pub accept_host_key: Option<String>,
}

impl CloneRequest {
    /// Full clone of `url` into `to`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::clone::CloneRequest;
    ///
    /// let request = CloneRequest::new("https://example.com/app.git", "/work/app");
    /// assert!(!request.bare);
    /// assert_eq!(request.depth, None);
    /// ```
    #[must_use]
    pub fn new(url: impl Into<String>, to: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            to: to.into(),
            depth: None,
            branch: None,
            bare: false,
            accept_host_key: None,
        }
    }
}

/// Result of a clone.
#[derive(Debug, Clone, Default)]
pub struct CloneOutcome {
    /// Root of the new repository.
    pub root: PathBuf,
    /// Address stored in `remote.origin.url`, without a password.
    pub url: String,
    /// Commit checked out; `None` for an empty remote.
    pub head: Option<ObjectId>,
    /// Branch `HEAD` points to; `None` when it is detached.
    pub branch: Option<String>,
    /// References created.
    pub ref_updates: Vec<GitRefUpdate>,
    /// Objects received.
    pub objects_received: u64,
    /// Bytes of the received pack.
    pub bytes_received: u64,
    /// Files written by the checkout.
    pub files_written: u64,
    /// Unusual effects.
    pub notable: Vec<NotableEntry>,
}

/// Clones `request.url` into the new directory `request.to`.
///
/// # Errors
///
/// * `E_EXISTS` with problem `destination_exists` when the directory exists,
///   `E_NOT_FOUND` when its parent does not, and `E_INVALID_VALUE` for a
///   relative local path or a depth of 0;
/// * `E_GIT_REF` with problem `remote_branch_not_found` when `branch` is
///   not a branch of the remote;
/// * the network, authentication and protocol errors of a fetch.
pub fn clone(
    request: &CloneRequest,
    tx: &mut dyn MutationTx,
    ctx: &ExecCtx,
) -> Result<CloneOutcome> {
    check_destination(request)?;
    let stored = layout::stored_url(&request.url)?;
    let shallow = match request.depth {
        Some(0) => return Err(zero_depth()),
        Some(depth) => Shallow::DepthAtRemote(depth.try_into().map_err(|_| zero_depth())?),
        None => Shallow::NoChange,
    };
    // The key of a new SSH host is accepted only when the connection is made,
    // after the first change, so the known hosts file is declared now.
    let reservation = match ssh_known_hosts_file(request, ctx) {
        Some(file) => Some(nurt_git_transport::ssh::reserve_host_key_file(tx, &file)?),
        None => None,
    };
    let preserved = tx.preserve(&[TargetSpec {
        path: TargetPath::Fs(abs_path(&request.to)?),
        action: TargetAction::CreateTree,
        expect: Expect::Absent,
        size_hint: None,
    }])?;
    let index = preserved.first().map(|item| item.index).ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            "the journal did not declare the new directory",
        )
    })?;
    tx.apply(index, Mutation::CreateDir { mode: DIR_MODE })?;
    let (outcome, host_keys) = populate(request, &stored, &shallow, ctx)?;
    if let Some(reservation) = &reservation {
        reservation.record(tx, &host_keys)?;
    }
    let mut marker = IrreversibleEntry::new(Irreversible::RemoteRefUpdated);
    marker.detail = Some(head::anonymize(&stored));
    tx.mark_irreversible(marker);
    Ok(outcome)
}

/// Known hosts file of Nurt, when the address of the clone is an SSH one.
fn ssh_known_hosts_file(request: &CloneRequest, ctx: &ExecCtx) -> Option<PathBuf> {
    let parsed = gix::url::parse(request.url.as_str()).ok()?;
    if parsed.scheme != gix::url::Scheme::Ssh {
        return None;
    }
    nurt_git_transport::ssh::SshOptions::from_ctx(ctx).nurt_known_hosts
}

/// Whether the system and global configuration enable symbolic links
/// (`core.symlinks`); an unreadable configuration counts as not enabling
/// them, the default of `git` for Windows.
fn links_enabled(ctx: &ExecCtx, git_dir: &Path) -> bool {
    let env = crate::repo::env::GitEnv::from_host(ctx.host());
    crate::repo::config::load_outer_layers(&env, git_dir, None)
        .ok()
        .and_then(|config| config.boolean("core.symlinks").ok().flatten())
        .unwrap_or(false)
}

/// Writes the repository into the created directory.
fn populate(
    request: &CloneRequest,
    stored: &str,
    shallow: &Shallow,
    ctx: &ExecCtx,
) -> Result<(CloneOutcome, Vec<nurt_git_transport::ssh::HostKeyAddition>)> {
    let kind = if request.bare {
        gix::create::Kind::Bare
    } else {
        gix::create::Kind::WithWorktree
    };
    gix::create::into(&request.to, kind, gix::create::Options::default())
        .map_err(|failure| crate::fetch::error_io(&request.to, &failure.to_string()))?;
    let git_dir = if request.bare {
        request.to.clone()
    } else {
        request.to.join(".git")
    };
    let mut layout = Layout::new(&git_dir, request.bare, stored);
    layout.set_links(links_enabled(ctx, &git_dir));
    layout.write_config(None, None)?;
    let repo = GitRepo::open(&request.to, ctx)?;
    let network = Network::new(
        repo.config().plumbing(),
        &request.url,
        Some(REMOTE),
        request.accept_host_key.clone(),
        &retry_command(request),
        ctx,
    )?;
    let session = Session::open(&network, ctx)?;
    let remote_head = RemoteHead::of(session.remote_refs());
    let branch = chosen_branch(request, &remote_head, session.remote_refs())?;
    let single = request.depth.is_some();
    let spec = layout::fetch_spec(request.bare, single.then_some(branch.as_deref()).flatten());
    let tags = if single { Tags::Included } else { Tags::All };
    let ref_map = session.map(
        vec![remote::parse_spec(&spec)?],
        vec![remote::parse_spec(remote::TAG_SPEC)?],
    )?;
    let mut received = session.receive(repo.gix(), &ref_map, shallow, tags, &network, ctx)?;
    if matches!(tags, Tags::Included) {
        let missing = crate::fetch::transfer::missing_tag_objects(repo.gix(), &ref_map);
        if !missing.is_empty() {
            let later =
                crate::fetch::transfer::fetch_tag_objects(&network, repo.gix(), &missing, ctx)?;
            received.absorb(&later);
        }
    }
    let committer = crate::checkpoint::support::committer(repo.gix(), ctx);
    let refs = layout::collect(repo.gix(), &ref_map, tags);
    let head = layout::head_target(&refs, &remote_head, branch.as_deref(), request.bare);
    layout.set_committer(committer);
    layout.write_refs(&refs, &head)?;
    let config_branch = (!request.bare).then(|| head.branch()).flatten();
    layout.write_config((!request.bare).then_some(spec.as_str()), config_branch)?;
    write_shallow(&git_dir, &received)?;
    let files_written = match head.commit() {
        Some(commit) if !request.bare => checkout(&request.to, commit, ctx)?,
        _ => 0,
    };
    let mut notable = network.notable.clone();
    notable.extend(network.credentials.report().notable);
    if single {
        let mut note = NotableEntry::new(Notable::ShallowRepository);
        note.path = Some(request.to.display().to_string());
        note.detail = Some(format!(
            "history limited to {} commits",
            request.depth.unwrap_or(1)
        ));
        notable.push(note);
    }
    let outcome = CloneOutcome {
        root: request.to.clone(),
        url: stored.to_owned(),
        head: head.commit(),
        branch: head.branch().map(str::to_owned),
        ref_updates: ref_updates(&refs, &head),
        objects_received: received.objects,
        bytes_received: received.bytes,
        files_written,
        notable,
    };
    Ok((outcome, network.host_key_additions()))
}

/// Branch to check out: the requested one, which must exist on the remote,
/// or the branch the remote `HEAD` names.
fn chosen_branch(
    request: &CloneRequest,
    remote_head: &RemoteHead,
    remote_refs: &[Ref],
) -> Result<Option<String>> {
    let Some(branch) = &request.branch else {
        return Ok(remote_head.branch());
    };
    let name = format!("{}{branch}", crate::repo::records::HEADS);
    let exists = remote_refs
        .iter()
        .filter_map(update::advertised)
        .any(|reference| reference.name == name);
    if exists {
        return Ok(Some(branch.clone()));
    }
    let mut error = Error::new(
        ErrorCode::GitRef,
        format!(
            "the remote {} has no branch `{branch}`; pass one of its branches or omit branch",
            head::anonymize(&request.url)
        ),
    )
    .with_problem("remote_branch_not_found")
    .with_found(branch.clone());
    for reference in remote_refs.iter().filter_map(update::advertised) {
        if let Some(short) = reference.name.strip_prefix(crate::repo::records::HEADS) {
            error = error.with_sample(short.to_owned());
        }
    }
    Err(error)
}

/// Checks that the destination does not exist and that its parent does.
fn check_destination(request: &CloneRequest) -> Result<()> {
    let to = &request.to;
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(Error::new(
            ErrorCode::Exists,
            format!(
                "destination {} already exists; pass a directory that does not exist",
                quote_path(to)
            ),
        )
        .with_problem("destination_exists")
        .with_found(to.display().to_string()));
    }
    let parent = to.parent().filter(|parent| !parent.as_os_str().is_empty());
    match parent {
        Some(parent) if parent.is_dir() => Ok(()),
        _ => Err(Error::new(
            ErrorCode::NotFound,
            format!(
                "the parent directory of {} does not exist; create it first",
                quote_path(to)
            ),
        )
        .with_problem("parent_missing")
        .with_found(to.display().to_string())),
    }
}

/// Error for a depth of 0.
fn zero_depth() -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        "depth must be at least 1; omit depth for the whole history",
    )
    .with_problem("invalid_depth")
    .with_found("0")
}

/// Call repeated with an accepted host key in the fix of host key errors.
fn retry_command(request: &CloneRequest) -> String {
    format!(
        "{COMMAND} {} {}",
        crate::repo::error::quote(&head::anonymize(&request.url)),
        quote_path(&request.to)
    )
}

/// Writes the `shallow` file received with a depth.
fn write_shallow(git_dir: &Path, received: &Received) -> Result<()> {
    match &received.shallow {
        Some(content) if !content.is_empty() => {
            layout::write_file(&git_dir.join(crate::fetch::SHALLOW), content)
        }
        _ => Ok(()),
    }
}

/// Checks out `commit` into the new working tree `root` and writes the
/// index; returns the number of files written.
fn checkout(root: &Path, commit: ObjectId, ctx: &ExecCtx) -> Result<u64> {
    let repo = GitRepo::open(root, ctx)?;
    let tree = repo
        .gix()
        .find_commit(commit)
        .and_then(|commit| commit.tree_id())
        .map_err(|failure| crate::fetch::error_io(root, &failure.to_string()))?
        .detach();
    let apply = ApplyContext::open(repo.gix())?;
    let plan = plan_new_tree(&apply, &tree)?;
    let mut tx = DirectTx::new(root);
    ctx.progress(
        "checkout",
        0,
        Some(u64::try_from(plan.entries.len()).unwrap_or(u64::MAX)),
    );
    let report = apply_plan(&mut tx, &apply, plan)?;
    Ok(u64::try_from(report.written.len()).unwrap_or(u64::MAX))
}

/// Records of the references a clone creates.
fn ref_updates(refs: &[layout::Packed], head: &layout::HeadTarget) -> Vec<GitRefUpdate> {
    let mut updates: Vec<GitRefUpdate> = refs
        .iter()
        .map(|reference| GitRefUpdate {
            reference: reference.name.clone(),
            status: GitRefStatus::New,
            remote: REMOTE.to_owned(),
            remote_ref: Some(reference.remote_name.clone()),
            old: None,
            new: Some(reference.id.to_string()),
            reason: None,
        })
        .collect();
    if let layout::HeadTarget::Branch {
        name,
        commit: Some(commit),
        ..
    } = head
    {
        let local = format!("{}{name}", crate::repo::records::HEADS);
        if !refs.iter().any(|reference| reference.name == local) {
            updates.push(GitRefUpdate {
                remote_ref: Some(local.clone()),
                reference: local,
                status: GitRefStatus::New,
                remote: REMOTE.to_owned(),
                old: None,
                new: Some(commit.to_string()),
                reason: None,
            });
        }
    }
    updates
}
