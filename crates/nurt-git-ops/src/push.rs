// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.push` command.
//!
//! `git.push` updates references of a remote with local commits through the
//! send-pack protocol. The `branch` parameter is a refspec: `src:dst` names
//! the local revision and the remote reference, a name alone pushes the
//! branch or tag to the reference of the same name, `:dst` and `delete`
//! delete the remote reference and a leading `+` forces the update. Each
//! update is classified before the request as `git push` does: up to date,
//! new, fast-forward, forced, or rejected as non-fast-forward, `fetch first`,
//! stale lease or existing tag. Updates accepted by the remote move the
//! remote-tracking references named by `remote.<name>.fetch`, with the
//! reflog message `update by push`.
//!
//! A remote on the local file system (`file://` URL or path) is served by
//! the in-process `receive-pack` service. Its reference changes join the
//! journal transaction of the command together with the remote-tracking
//! references and the upstream configuration, so the push is undone as a
//! whole.
//!
//! A remote reached over HTTP or SSH is contacted through the transport of
//! its address after the `url.<base>.pushInsteadOf` and
//! `url.<base>.insteadOf` rewrites. Its reference changes cannot be undone:
//! they are reported as irreversible, and only the remote-tracking
//! references and the configuration of the local repository are journaled.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use gix::credentials::helper::Action;
use gix::protocol::transport::Protocol;
use gix::protocol::transport::client::TransportWithoutIO;
use gix::protocol::transport::client::blocking_io::Transport;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{ExemptWrite, FileOp, MutationTx, apply_ops};
use nurt_contracts::record::{
    Change, ChangeAction, Effect, EffectStatus, GitEffect, GitEffectState, GitRefStatus,
    GitRefUpdate,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::services::ProcessHost;
use nurt_contracts::stage::{
    BoundArgs, CancelToken, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds,
    RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{self, Record, Value};
use nurt_git_transport::connect::{self, Direction, TransportOptions, UrlRewrites};
use nurt_git_transport::credentials::{
    CredentialOptions, Credentials, EnvLookup, StoreUpdate, bearer_token,
};
use nurt_git_transport::http::{HttpNote, HttpSettings, NetworkFailure, redact_url};
use nurt_git_transport::local::receive::{
    self, HookRunner, ProcessHooks, ReceiveOptions, ReceivePack, UpdateSink,
};
use nurt_git_transport::send_pack::{
    self, Advertisement, FilePlan, Identity, LogPolicy, PackSummary, PushReport, RefCommand,
    RefEdit, RefStatus, RejectReason, SendOptions, SendOutcome,
};
use nurt_git_transport::ssh::SshOptions;

use crate::reset::args;

/// Name of the command in records and errors.
const COMMAND: &str = "git.push";

/// Reflog message of remote-tracking references moved by a push.
const TRACKING_MESSAGE: &str = "update by push";

/// Default remote.
const DEFAULT_REMOTE: &str = "origin";

/// Nanoseconds per second of the clock of the execution context.
const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// Reason of a deletion of a reference the remote does not have.
const REMOTE_REF_MISSING: &str = "remote_ref_missing";

/// Reason of a sent update the status report does not mention.
const NOT_REPORTED: &str = "not_reported";

/// Problem of the transport error raised when the remote asks for
/// credentials.
const AUTHENTICATION_REQUIRED: &str = "authentication_required";

/// Problem of the transport error raised when the remote refuses access.
const FORBIDDEN: &str = "forbidden";

/// HTTP status of a request without valid credentials.
const HTTP_UNAUTHORIZED: u16 = 401;

/// HTTP status of a request the credentials do not allow.
const HTTP_FORBIDDEN: u16 = 403;

/// How an update that is not a fast-forward is treated.
///
/// `true` of the catalog type `bool | enum(no|lease|yes)` means `lease`.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::push::ForceMode;
///
/// let mode: ForceMode = "true".parse().unwrap_or_default();
/// assert_eq!(mode, ForceMode::Lease);
/// assert_eq!(ForceMode::default().as_str(), "no");
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ForceMode {
    /// Only fast-forwards and new references.
    #[default]
    No,
    /// Overwrite only while the remote reference still has the value of
    /// its remote-tracking reference.
    Lease,
    /// Overwrite unconditionally.
    Yes,
}

impl ForceMode {
    /// Name of the mode as in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Lease => "lease",
            Self::Yes => "yes",
        }
    }

    fn from_value(value: Option<&Value>) -> Result<Self> {
        match value {
            None | Some(Value::Null) => Ok(Self::No),
            Some(Value::Bool(true)) => Ok(Self::Lease),
            Some(Value::Bool(false)) => Ok(Self::No),
            Some(Value::Enum(text) | Value::Text(text)) => text.parse(),
            Some(_) => Err(unknown_force("the given value")),
        }
    }
}

impl FromStr for ForceMode {
    type Err = Error;

    /// Parses a mode name or a boolean.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_VALUE` for any other text.
    fn from_str(text: &str) -> Result<Self> {
        match text {
            "no" | "false" => Ok(Self::No),
            "lease" | "true" => Ok(Self::Lease),
            "yes" => Ok(Self::Yes),
            other => Err(unknown_force(&format!("`{other}`"))),
        }
    }
}

fn unknown_force(what: &str) -> Error {
    Error::new(
        ErrorCode::UnknownValue,
        format!("{what} is not a force mode; use true, false, no, lease or yes"),
    )
    .with_expected(serde_json::json!(["no", "lease", "yes"]))
}

/// Parameters of a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    /// Configured remote name, URL or path.
    pub remote: String,
    /// Refspec: `src:dst`, `:dst`, a branch or tag name, or `HEAD`; a
    /// leading `+` forces the update.
    pub branch: String,
    /// Treatment of updates that are not fast-forwards.
    pub force: ForceMode,
    /// Revision the remote reference must still have for a forced update.
    pub force_with_lease: Option<String>,
    /// Also push every local tag.
    pub tags: bool,
    /// Record the pushed branch as the upstream of the local branch.
    pub set_upstream: bool,
    /// Delete the remote reference named by `branch`.
    pub delete: bool,
    /// Update all references or none.
    pub atomic: bool,
    /// Run the `pre-push` hook and the hooks of a local remote.
    pub hooks: bool,
}

impl PushRequest {
    /// Request pushing `branch` to `origin` with the defaults of the
    /// catalog.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::push::{ForceMode, PushRequest};
    ///
    /// let request = PushRequest::new("main");
    /// assert_eq!(request.remote, "origin");
    /// assert_eq!(request.force, ForceMode::No);
    /// assert!(request.hooks);
    /// ```
    #[must_use]
    pub fn new(branch: impl Into<String>) -> Self {
        Self {
            remote: DEFAULT_REMOTE.to_owned(),
            branch: branch.into(),
            force: ForceMode::No,
            force_with_lease: None,
            tags: false,
            set_upstream: false,
            delete: false,
            atomic: false,
            hooks: true,
        }
    }
}

/// One reference update of a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// Full name of the local reference pushed; `None` for deletions and
    /// revisions that are not references.
    pub local_ref: Option<String>,
    /// Full name of the remote reference.
    pub remote_ref: String,
    /// Remote-tracking reference that follows the remote reference.
    pub tracking_ref: Option<String>,
    /// Value of the remote reference before the push.
    pub old: Option<ObjectId>,
    /// Value pushed; `None` for deletions.
    pub new: Option<ObjectId>,
    /// Outcome.
    pub status: GitRefStatus,
    /// Code of the reason of a rejection, for example `non_fast_forward`.
    pub reason: Option<String>,
}

impl RefUpdate {
    /// Whether the update is sent to the remote.
    #[must_use]
    pub fn is_sent(&self) -> bool {
        matches!(
            self.status,
            GitRefStatus::New
                | GitRefStatus::Updated
                | GitRefStatus::Forced
                | GitRefStatus::Deleted
        )
    }

    /// Whether the update was rejected locally or by the remote.
    #[must_use]
    pub fn is_rejected(&self) -> bool {
        matches!(self.status, GitRefStatus::Rejected | GitRefStatus::Error)
    }

    fn command(&self, hash: gix::hash::Kind) -> RefCommand {
        let null = ObjectId::null(hash);
        let new = match self.status {
            GitRefStatus::Deleted => null,
            _ => self.new.unwrap_or(null),
        };
        RefCommand {
            old: self.old.unwrap_or(null),
            new,
            name: self.remote_ref.as_str().into(),
        }
    }

    fn reject(&mut self, reason: &str) {
        self.status = GitRefStatus::Rejected;
        self.reason = Some(reason.to_owned());
    }

    fn record(&self, remote: &str) -> GitRefUpdate {
        GitRefUpdate {
            reference: self
                .local_ref
                .clone()
                .unwrap_or_else(|| self.remote_ref.clone()),
            status: self.status,
            remote: remote.to_owned(),
            remote_ref: Some(self.remote_ref.clone()),
            old: hex(self.old),
            new: hex(self.new),
            reason: self.reason.clone(),
        }
    }
}

fn hex(id: Option<ObjectId>) -> Option<String> {
    id.map(|id| id.to_hex().to_string())
}

/// Upstream configuration written for `set_upstream`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Upstream {
    local_ref: String,
    branch: String,
    remote: String,
    merge: String,
}

/// A planned push: the classified updates and the target, computed
/// without changing anything.
#[derive(Debug)]
pub struct PlannedPush {
    root: PathBuf,
    git_dir: PathBuf,
    remote: String,
    url: String,
    target: Target,
    network: NetworkOptions,
    notes: Vec<NotableEntry>,
    updates: Vec<RefUpdate>,
    objects: u64,
    upstream: Option<Upstream>,
    pre_push: Option<PathBuf>,
    hooks: bool,
    atomic: bool,
    hash: gix::hash::Kind,
}

/// Services used while a push runs.
pub struct PushContext<'a> {
    /// Identity and time of reflog entries.
    pub identity: Identity,
    /// Program launching service for hooks; `None` skips the hooks.
    pub host: Option<&'a dyn ProcessHost>,
    /// Cancellation of running hooks.
    pub cancel: &'a CancelToken,
}

impl std::fmt::Debug for PushContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushContext")
            .field("identity", &self.identity)
            .field("hooks", &self.host.is_some())
            .finish_non_exhaustive()
    }
}

/// Result of a push.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushOutcome {
    /// Root of the local repository.
    pub repo: PathBuf,
    /// Remote as given: a configured name, a URL or a path.
    pub remote: String,
    /// Updates in request order.
    pub updates: Vec<RefUpdate>,
    /// Objects in the pack sent, or to send for a planned push.
    pub objects: u64,
    /// Upstream recorded for the local branch, as `<remote>/<branch>`.
    pub upstream: Option<String>,
    /// A hook exists but was not run.
    pub hooks_skipped: bool,
    /// Messages of the remote, one entry per line.
    pub remote_messages: Vec<String>,
    /// Files changed in the local and the target repository.
    pub changed_files: Vec<PathBuf>,
    /// The pack sent; `None` when nothing was sent.
    pub pack: Option<PackSummary>,
    /// The remote did not report the outcome of every update sent.
    pub status_unknown: bool,
    /// Conditions of the transport and of the credential lookup.
    pub transport_notable: Vec<NotableEntry>,
}

/// Settings of pushes over HTTP and SSH taken from the execution context.
#[derive(Clone, Default)]
pub struct NetworkOptions {
    /// Environment variables of the operation; they give credentials,
    /// proxies and HTTP settings.
    pub env: BTreeMap<String, String>,
    /// SSH settings; without them SSH addresses fail with `E_INTERNAL`.
    pub ssh: Option<SshOptions>,
}

impl std::fmt::Debug for NetworkOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkOptions")
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("ssh", &self.ssh.is_some())
            .finish()
    }
}

impl NetworkOptions {
    /// Settings of the execution context: its environment variables and the
    /// SSH settings of [`SshOptions::from_ctx`].
    #[must_use]
    pub fn from_ctx(ctx: &ExecCtx) -> Self {
        let env = ctx
            .host()
            .env_vars()
            .into_iter()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        Self {
            env,
            ssh: Some(SshOptions::from_ctx(ctx)),
        }
    }

    fn lookup(&self) -> EnvLookup {
        let env = self.env.clone();
        Arc::new(move |name: &str| env.get(name).cloned())
    }
}

impl PushOutcome {
    /// Updates rejected locally or by the remote.
    pub fn rejected(&self) -> impl Iterator<Item = &RefUpdate> {
        self.updates.iter().filter(|update| update.is_rejected())
    }

    /// Checks that the push did not fail as a whole.
    ///
    /// A push that updated some references and had others rejected
    /// succeeds with the state `stopped`; the rejected updates are in the
    /// records.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_PUSH_REJECTED` when updates were rejected and no
    /// reference was updated.
    pub fn check(&self) -> Result<()> {
        let rejected: Vec<&RefUpdate> = self.rejected().collect();
        if rejected.is_empty() || self.updates.iter().any(RefUpdate::is_sent) {
            return Ok(());
        }
        Err(rejected_error(self, &rejected))
    }

    fn state(&self) -> GitEffectState {
        if self.rejected().next().is_some() {
            GitEffectState::Stopped
        } else if self
            .updates
            .iter()
            .all(|update| update.status == GitRefStatus::UpToDate)
        {
            GitEffectState::UpToDate
        } else {
            GitEffectState::Completed
        }
    }

    /// Unusual effects of the push.
    #[must_use]
    pub fn notable(&self) -> Vec<NotableEntry> {
        let mut notable = Vec::new();
        for update in &self.updates {
            if update.status == GitRefStatus::Forced {
                for code in [Notable::ForcePush, Notable::HistoryRewritten] {
                    notable.push(NotableEntry {
                        detail: Some(update.remote_ref.clone()),
                        ..NotableEntry::new(code)
                    });
                }
            }
        }
        if let Some(upstream) = &self.upstream {
            notable.push(NotableEntry {
                detail: Some(upstream.clone()),
                ..NotableEntry::new(Notable::UpstreamSet)
            });
        }
        if self.hooks_skipped {
            notable.push(NotableEntry::new(Notable::HooksSkipped));
        }
        if let Some(pack) = self.pack.filter(PackSummary::without_deltas_notable) {
            notable.push(NotableEntry {
                detail: Some(format!("{} bytes", pack.bytes)),
                ..NotableEntry::new(Notable::PackWithoutDeltas)
            });
        }
        if self.status_unknown {
            notable.push(NotableEntry::new(Notable::PushStatusUnknown));
        }
        notable.extend(self.transport_notable.iter().cloned());
        notable
    }

    /// Changes of files for the effect report.
    #[must_use]
    pub fn changes(&self) -> Vec<Change> {
        self.changed_files
            .iter()
            .map(|path| Change {
                path: value::Path::new(path.to_string_lossy().into_owned()),
                action: ChangeAction::Modify,
                ..Change::default()
            })
            .collect()
    }

    /// The effect record followed by one `git_ref_update` per update.
    #[must_use]
    pub fn records(&self, status: EffectStatus) -> Vec<Record> {
        let updates: Vec<GitRefUpdate> = self
            .updates
            .iter()
            .map(|update| update.record(&self.remote))
            .collect();
        let git = GitEffect {
            repo: value::Path::new(self.repo.to_string_lossy().into_owned()),
            state: self.state(),
            ref_updates: Some(updates.clone()),
            upstream: self.upstream.clone(),
            ..GitEffect::default()
        };
        let changes = self.changes();
        let codes: Vec<String> = self
            .notable()
            .iter()
            .map(|entry| entry.code.to_string())
            .collect();
        let effect = Effect {
            status,
            kind: COMMAND.to_owned(),
            changes: (!changes.is_empty()).then_some(changes),
            notable: (!codes.is_empty()).then_some(codes),
            git: Some(git),
            ..Effect::default()
        };
        std::iter::once(Record::from(effect))
            .chain(updates.into_iter().map(Record::from))
            .collect()
    }
}

fn rejected_error(outcome: &PushOutcome, rejected: &[&RefUpdate]) -> Error {
    let reason = rejected
        .first()
        .and_then(|update| update.reason.as_deref())
        .unwrap_or(RejectReason::Other.code());
    let names: Vec<String> = rejected
        .iter()
        .map(|update| {
            format!(
                "{} ({})",
                update.remote_ref,
                update
                    .reason
                    .as_deref()
                    .unwrap_or(RejectReason::Other.code())
            )
        })
        .collect();
    let details: Vec<serde_json::Value> = outcome
        .updates
        .iter()
        .map(|update| {
            serde_json::json!({
                "ref": update.remote_ref,
                "status": update.status.as_str(),
                "reason": update.reason,
            })
        })
        .collect();
    let error = Error::new(
        ErrorCode::GitPushRejected,
        format!(
            "the push of {} was rejected; {}",
            names.join(", "),
            advice(reason)
        ),
    )
    .with_problem(reason)
    .with_detail("ref_updates", serde_json::Value::Array(details));
    let repo = quote(&outcome.repo.to_string_lossy());
    let remote = quote(&outcome.remote);
    match reason {
        "non_fast_forward" | "fetch_first" => {
            error.with_fix(format!("git.pull {repo} remote: {remote}"))
        }
        "stale_info" => error.with_next_step(format!("git.fetch {repo} remote: {remote}")),
        _ => error,
    }
}

fn advice(reason: &str) -> &'static str {
    match reason {
        "non_fast_forward" => "integrate the remote changes and push again",
        "fetch_first" => {
            "the remote has commits that are not present locally; fetch and integrate them, then push again"
        }
        "stale_info" => {
            "the remote reference changed since the last fetch; fetch and review the remote changes"
        }
        "already_exists" => "the tag exists on the remote; push with force: yes to replace it",
        "checked_out" => {
            "the branch is checked out in the remote working tree; push to a bare repository or to another branch"
        }
        "hook_declined" => "a hook of the remote declined the update; see the remote messages",
        "atomic_failed" => "another reference of the atomic push was rejected",
        "deletion_prohibited" => "the remote does not allow deleting references",
        REMOTE_REF_MISSING => "the remote reference does not exist; check the branch name",
        _ => "see the remote messages",
    }
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Local refspec resolved against both repositories.
#[derive(Debug, Clone)]
struct Spec {
    local_ref: Option<String>,
    new: Option<ObjectId>,
    remote_ref: String,
    force: bool,
}

/// Where the remote repository is.
#[derive(Debug, Clone)]
enum Target {
    /// Repository on the local file system, served in process.
    Local(PathBuf),
    /// Address of a remote reached over HTTP or SSH, after the rewrites.
    Network(String),
}

/// Remote of a push.
#[derive(Debug)]
struct Remote {
    name: String,
    url: String,
    target: Target,
    fetch: Vec<String>,
}

/// Plans a push from the repository containing `path`.
///
/// The remote is contacted to read its references; nothing is changed. The
/// result is applied with [`PlannedPush::apply`].
///
/// # Errors
///
/// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not in a repository or
/// the remote is not one, `E_NOT_FOUND` for an unknown remote name,
/// `E_INVALID_VALUE` for an address that cannot be parsed,
/// `E_GIT_UNSUPPORTED` for `git://` and `ext::` addresses, `E_GIT_AUTH` when
/// the remote requires credentials that are missing or refused, `E_NETWORK`
/// when the remote cannot be reached, `E_GIT_REVISION` when the source of
/// the refspec does not resolve, `E_GIT_REF` for an invalid reference name
/// and `E_IO` when an object is missing.
pub fn plan_push(
    path: &Path,
    request: &PushRequest,
    network: &NetworkOptions,
) -> Result<PlannedPush> {
    let repo = open_local(path)?;
    let git_dir = repo.git_dir().to_path_buf();
    let root = repo.workdir().unwrap_or(&git_dir).to_path_buf();
    let remote = resolve_remote(&repo, &root, &request.remote)?;
    let (advertisement, notes) = read_advertisement(&repo, &remote, network)?;
    let specs = specs(&repo, request, &advertisement)?;
    let lease = request
        .force_with_lease
        .as_deref()
        .map(|revision| resolve_source(&repo, revision).map(|(_, id)| id))
        .transpose()?;
    let mut updates = Vec::with_capacity(specs.len());
    for spec in &specs {
        let tracking_ref = tracking_name(&remote.fetch, &spec.remote_ref);
        let tracking = match &tracking_ref {
            Some(name) => send_pack::read_ref(&git_dir, name.as_bytes())?,
            None => None,
        };
        let expected = match (&request.force_with_lease, request.force) {
            (Some(_), _) => Some(lease),
            (None, ForceMode::Lease) => Some(tracking),
            (None, _) => None,
        };
        let old = advertisement.find(spec.remote_ref.as_bytes().as_bstr());
        let force = spec.force || request.force == ForceMode::Yes;
        let (status, reason) = decide(&repo, spec, old, expected, force);
        updates.push(RefUpdate {
            local_ref: spec.local_ref.clone(),
            remote_ref: spec.remote_ref.clone(),
            tracking_ref,
            old,
            new: spec.new,
            status,
            reason: reason.map(str::to_owned),
        });
    }
    if request.atomic {
        fail_atomically(&mut updates);
    }
    let objects = count_objects(&repo, &updates, &advertisement)?;
    let upstream = request
        .set_upstream
        .then(|| upstream_of(&specs, &request.remote))
        .flatten();
    Ok(PlannedPush {
        pre_push: pre_push_hook(&repo, &git_dir),
        root,
        git_dir,
        remote: remote.name,
        url: remote.url,
        target: remote.target,
        network: network.clone(),
        notes,
        updates,
        objects,
        upstream,
        hooks: request.hooks,
        atomic: request.atomic,
        hash: repo.object_hash(),
    })
}

fn open_local(path: &Path) -> Result<gix::Repository> {
    for dir in path.ancestors() {
        if let Ok(repo) = gix::open_opts(dir, gix::open::Options::isolated()) {
            return Ok(repo);
        }
    }
    Err(Error::new(
        ErrorCode::GitNotRepository,
        format!(
            "`{}` is not inside a git repository; pass the path of a repository",
            path.display()
        ),
    ))
}

fn resolve_remote(repo: &gix::Repository, root: &Path, remote: &str) -> Result<Remote> {
    let config = repo.config_snapshot();
    let configured = ["pushurl", "url"].iter().find_map(|key| {
        config
            .string(format!("remote.{remote}.{key}").as_str())
            .map(|value| value.to_str_lossy().into_owned())
    });
    let fetch = config
        .strings(format!("remote.{remote}.fetch").as_str())
        .unwrap_or_default()
        .iter()
        .map(|value| value.to_str_lossy().into_owned())
        .collect();
    let url = match configured {
        Some(url) => url,
        None if is_address(remote) => remote.to_owned(),
        None => {
            return Err(Error::new(
                ErrorCode::NotFound,
                format!(
                    "remote `{remote}` is not configured; add remote.{remote}.url to the repository configuration or pass a URL or path"
                ),
            )
            .with_problem("unknown_remote"));
        }
    };
    let options = TransportOptions::new(HttpSettings::default())
        .with_rewrites(UrlRewrites::from_config(config.plumbing()));
    let parsed = connect::resolve_url(&url, Direction::Push, &options)?;
    let target = if parsed.scheme == gix::url::Scheme::File {
        let mut path = nurt_git_transport::local::repository_path(&parsed);
        if path.is_relative() {
            path = root.join(path);
        }
        Target::Local(path)
    } else {
        Target::Network(parsed.to_bstring().to_str_lossy().into_owned())
    };
    Ok(Remote {
        name: remote.to_owned(),
        url,
        target,
        fetch,
    })
}

fn is_address(remote: &str) -> bool {
    remote.contains(['/', '\\', ':']) || remote.starts_with('.')
}

fn receive_options(identity: Identity, run_hooks: bool) -> ReceiveOptions {
    ReceiveOptions {
        agent: format!("nurt/{}", env!("CARGO_PKG_VERSION")),
        report_status_v2: true,
        run_hooks,
        identity,
    }
}

/// Sink of the advertisement read while planning; a plan sends nothing.
struct ReadOnlySink;

impl UpdateSink for ReadOnlySink {
    fn apply(&mut self, _plan: &FilePlan, _accepted: &[RefCommand]) -> Result<()> {
        Err(Error::new(
            ErrorCode::Internal,
            "the plan of a push does not change references",
        ))
    }

    fn objects_written(&mut self, _git_dir: &Path) {}
}

fn read_advertisement(
    repo: &gix::Repository,
    remote: &Remote,
    network: &NetworkOptions,
) -> Result<(Advertisement, Vec<NotableEntry>)> {
    match &remote.target {
        Target::Local(path) => {
            let identity = Identity {
                name: BString::default(),
                email: BString::default(),
                seconds: 0,
                offset: 0,
            };
            let service = ReceivePack::open(path, receive_options(identity, false))?;
            let mut sink = ReadOnlySink;
            let (mut transport, _session) =
                receive::connect(service, &remote.url, None, &mut sink)?;
            Ok((send_pack::discover(&mut transport)?, Vec::new()))
        }
        Target::Network(address) => {
            let mut connection = NetworkConnection::open(repo, &remote.name, address, network)?;
            let advertisement = connection.discover()?;
            Ok((advertisement, connection.notable()))
        }
    }
}

/// Connection to a remote reached over HTTP or SSH.
struct NetworkConnection {
    transport: Box<dyn Transport>,
    credentials: Option<Credentials>,
    notes: Vec<NotableEntry>,
}

impl NetworkConnection {
    fn open(
        repo: &gix::Repository,
        remote: &str,
        address: &str,
        network: &NetworkOptions,
    ) -> Result<Self> {
        let config = repo.config_snapshot();
        let file = config.plumbing();
        let env = network.lookup();
        let mut http = HttpSettings::resolve(file, address, Some(remote), &*env)
            .map_err(|failure| settings_error(&failure, address))?;
        if http.bearer.is_none() {
            http.bearer = bearer_token(&*env, address);
        }
        let notes = http.notes.iter().map(http_notable).collect();
        let mut options = TransportOptions::new(http);
        options.protocol = Protocol::V0;
        if let Some(ssh) = &network.ssh {
            options = options.with_ssh(ssh.clone());
        }
        let transport = connect::connect(address, Direction::Push, &options)?;
        let credentials =
            is_http(address).then(|| Credentials::new(file, address, CredentialOptions::new(env)));
        Ok(Self {
            transport,
            credentials,
            notes,
        })
    }

    /// Reads the advertisement; a remote that asks for credentials is asked
    /// once more with the credentials found for its address, as `git` does.
    fn discover(&mut self) -> Result<Advertisement> {
        let first = send_pack::discover(self.transport.as_mut());
        let Some(credentials) = &self.credentials else {
            return first;
        };
        match first.as_ref().err().and_then(refused_status) {
            None => return first,
            Some(HTTP_UNAUTHORIZED) => {}
            Some(status) => return Err(credentials.error(Some(status))),
        }
        let mut authenticate = credentials.authenticate();
        let url = self.transport.to_url().into_owned();
        let Ok(Some(answer)) = authenticate(Action::get_for_url(url)) else {
            return Err(credentials.error(Some(HTTP_UNAUTHORIZED)));
        };
        if self.transport.set_identity(answer.identity).is_err() {
            return Err(credentials.error(Some(HTTP_UNAUTHORIZED)));
        }
        let second = send_pack::discover(self.transport.as_mut());
        match second.as_ref().err().and_then(refused_status) {
            Some(status) => {
                drop(authenticate(answer.next.erase()));
                Err(credentials.error(Some(status)))
            }
            None => {
                if second.is_ok() {
                    drop(authenticate(answer.next.store()));
                }
                second
            }
        }
    }

    fn notable(&self) -> Vec<NotableEntry> {
        let mut notable = self.notes.clone();
        if let Some(credentials) = &self.credentials {
            notable.extend(credentials.report().notable);
        }
        notable
    }

    fn store_credentials(&self, tx: &mut dyn MutationTx) -> Result<()> {
        match &self.credentials {
            Some(credentials) => write_store_updates(tx, &credentials.report().store_updates),
            None => Ok(()),
        }
    }
}

/// HTTP status behind a transport error that refused the request.
fn refused_status(error: &Error) -> Option<u16> {
    if error.code() != ErrorCode::GitAuth {
        return None;
    }
    match error.problem.as_deref() {
        Some(AUTHENTICATION_REQUIRED) => Some(HTTP_UNAUTHORIZED),
        Some(FORBIDDEN) => Some(HTTP_FORBIDDEN),
        _ => None,
    }
}

fn is_http(address: &str) -> bool {
    address.starts_with("http://") || address.starts_with("https://")
}

fn http_notable(note: &HttpNote) -> NotableEntry {
    match note {
        HttpNote::TlsVerificationDisabled => NotableEntry::new(Notable::TlsVerificationDisabled),
        HttpNote::OptionIgnored { key } => NotableEntry {
            detail: Some(key.clone()),
            ..NotableEntry::new(Notable::HttpOptionIgnored)
        },
    }
}

fn settings_error(failure: &NetworkFailure, address: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!(
            "the HTTP settings for {} are invalid: {}; correct the http.* configuration of the repository",
            redact_url(address),
            failure.message()
        ),
    )
    .with_problem(failure.problem().as_str())
}

fn remote_updated(address: &str, count: usize) -> IrreversibleEntry {
    IrreversibleEntry {
        count: Some(u64::try_from(count).unwrap_or(u64::MAX)),
        detail: Some(redact_url(address)),
        ..IrreversibleEntry::new(Irreversible::RemoteRefUpdated)
    }
}

/// Writes the `store` files changed by the credential lookup. A file that
/// does not exist yet is not created, because `git credential-store`
/// creates it with mode 0600 and a journaled write of a new file does not
/// set the mode.
fn write_store_updates(tx: &mut dyn MutationTx, updates: &[StoreUpdate]) -> Result<()> {
    let mut ops = Vec::new();
    for update in updates.iter().filter(|update| update.existed) {
        if std::fs::read(&update.path).is_ok_and(|current| current == update.contents) {
            continue;
        }
        let path = AbsPath::new(&update.path).map_err(|_| {
            Error::new(
                ErrorCode::Io,
                format!(
                    "the credential file `{}` is not an absolute path; set HOME to an absolute directory",
                    update.path.display()
                ),
            )
        })?;
        ops.push(FileOp::write(path, update.contents.clone()));
    }
    if !ops.is_empty() {
        apply_ops(tx, ops)?;
    }
    Ok(())
}

fn specs(
    repo: &gix::Repository,
    request: &PushRequest,
    advertisement: &Advertisement,
) -> Result<Vec<Spec>> {
    let mut specs = vec![parse_spec(
        repo,
        &request.branch,
        request.delete,
        advertisement,
    )?];
    if request.tags {
        for (name, id) in local_tags(repo)? {
            if specs.iter().all(|spec| spec.remote_ref != name) {
                specs.push(Spec {
                    local_ref: Some(name.clone()),
                    new: Some(id),
                    remote_ref: name,
                    force: false,
                });
            }
        }
    }
    Ok(specs)
}

fn parse_spec(
    repo: &gix::Repository,
    text: &str,
    delete: bool,
    advertisement: &Advertisement,
) -> Result<Spec> {
    let (force, text) = match text.trim().strip_prefix('+') {
        Some(rest) => (true, rest),
        None => (false, text.trim()),
    };
    let deletion = |name: &str| -> Result<Spec> {
        Ok(Spec {
            local_ref: None,
            new: None,
            remote_ref: qualify(name, None, advertisement)?,
            force,
        })
    };
    match text.split_once(':') {
        _ if text.is_empty() => Err(invalid_spec(text)),
        None if delete => deletion(text),
        Some(_) if delete => Err(invalid_spec(text)),
        Some(("", dst)) => deletion(dst),
        Some((src, dst)) => {
            let (local_ref, id) = resolve_source(repo, src)?;
            let remote_ref = qualify(dst, local_ref.as_deref(), advertisement)?;
            Ok(Spec {
                local_ref,
                new: Some(id),
                remote_ref,
                force,
            })
        }
        None => {
            let (local_ref, id) = resolve_source(repo, text)?;
            let remote_ref = local_ref.clone().ok_or_else(|| {
                Error::new(
                    ErrorCode::GitRevision,
                    format!(
                        "`{text}` is not a branch or tag; name the remote reference as `{text}:refs/heads/<name>`"
                    ),
                )
                .with_problem("no_destination")
            })?;
            Ok(Spec {
                local_ref,
                new: Some(id),
                remote_ref,
                force,
            })
        }
    }
}

fn invalid_spec(text: &str) -> Error {
    Error::new(
        ErrorCode::GitRef,
        format!("`{text}` is not a valid refspec; use a branch name, `src:dst` or `:dst`"),
    )
    .with_problem("invalid_refspec")
}

fn resolve_source(repo: &gix::Repository, source: &str) -> Result<(Option<String>, ObjectId)> {
    let revision_error = |error: &dyn std::fmt::Display| {
        Error::new(
            ErrorCode::GitRevision,
            format!("`{source}` does not name a local branch, tag or commit: {error}"),
        )
        .with_problem("unknown_revision")
    };
    if source == "HEAD" {
        let id = repo
            .head_id()
            .map_err(|error| revision_error(&error))?
            .detach();
        let name = repo
            .head_name()
            .ok()
            .flatten()
            .map(|name| name.as_bstr().to_str_lossy().into_owned());
        return Ok((name, id));
    }
    if let Ok(Some(mut reference)) = repo.try_find_reference(source) {
        let name = reference.name().as_bstr().to_str_lossy().into_owned();
        let id = match reference.target().try_id() {
            Some(id) => id.to_owned(),
            None => reference
                .peel_to_id()
                .map_err(|error| revision_error(&error))?
                .detach(),
        };
        return Ok((Some(name), id));
    }
    let id = repo
        .rev_parse_single(source)
        .map_err(|error| revision_error(&error))?
        .detach();
    Ok((None, id))
}

fn qualify(dst: &str, source: Option<&str>, advertisement: &Advertisement) -> Result<String> {
    let name = if dst.starts_with("refs/") || dst == "HEAD" {
        dst.to_owned()
    } else if let Some(known) = ["refs/heads/", "refs/tags/"]
        .iter()
        .map(|prefix| format!("{prefix}{dst}"))
        .find(|name| advertisement.find(name.as_bytes().as_bstr()).is_some())
    {
        known
    } else if source.is_some_and(|name| name.starts_with("refs/tags/")) {
        format!("refs/tags/{dst}")
    } else {
        format!("refs/heads/{dst}")
    };
    send_pack::validate_name(name.as_bytes())?;
    Ok(name)
}

fn local_tags(repo: &gix::Repository) -> Result<Vec<(String, ObjectId)>> {
    let read_error = |error: &dyn std::fmt::Display| {
        Error::new(
            ErrorCode::Io,
            format!("reading the local tags failed: {error}"),
        )
    };
    let platform = repo.references().map_err(|error| read_error(&error))?;
    let mut tags = Vec::new();
    for reference in platform
        .prefixed(b"refs/tags/".as_bstr())
        .map_err(|error| read_error(&error))?
    {
        let reference = reference.map_err(|error| read_error(&error))?;
        if let Some(id) = reference.target().try_id() {
            let name = reference.name().as_bstr().to_str_lossy().into_owned();
            tags.push((name, id.to_owned()));
        }
    }
    tags.sort();
    Ok(tags)
}

/// Remote-tracking reference of `remote_ref` according to the fetch
/// refspecs of the remote.
fn tracking_name(fetch: &[String], remote_ref: &str) -> Option<String> {
    for spec in fetch {
        let spec = spec.trim_start_matches('+');
        let Some((src, dst)) = spec.split_once(':') else {
            continue;
        };
        match (src.split_once('*'), dst.split_once('*')) {
            (Some((src_prefix, src_suffix)), Some((dst_prefix, dst_suffix))) => {
                let middle = remote_ref
                    .strip_prefix(src_prefix)
                    .and_then(|rest| rest.strip_suffix(src_suffix));
                if let Some(middle) = middle {
                    return Some(format!("{dst_prefix}{middle}{dst_suffix}"));
                }
            }
            (None, None) if src == remote_ref && !dst.is_empty() => return Some(dst.to_owned()),
            _ => {}
        }
    }
    None
}

fn decide(
    repo: &gix::Repository,
    spec: &Spec,
    old: Option<ObjectId>,
    lease: Option<Option<ObjectId>>,
    force: bool,
) -> (GitRefStatus, Option<&'static str>) {
    let stale = lease.is_some_and(|expected| expected != old);
    let Some(new) = spec.new else {
        return match old {
            None => (GitRefStatus::Error, Some(REMOTE_REF_MISSING)),
            Some(_) if stale => rejected(RejectReason::StaleInfo),
            Some(_) => (GitRefStatus::Deleted, None),
        };
    };
    if old == Some(new) {
        return (GitRefStatus::UpToDate, None);
    }
    if stale {
        return rejected(RejectReason::StaleInfo);
    }
    let Some(old) = old else {
        return (GitRefStatus::New, None);
    };
    let forced = force || lease.is_some();
    if spec.remote_ref.starts_with("refs/tags/") && !forced {
        return rejected(RejectReason::AlreadyExists);
    }
    if is_ancestor(repo, old, new) {
        return (GitRefStatus::Updated, None);
    }
    if forced {
        return (GitRefStatus::Forced, None);
    }
    if repo.has_object(old) {
        rejected(RejectReason::NonFastForward)
    } else {
        rejected(RejectReason::FetchFirst)
    }
}

fn rejected(reason: RejectReason) -> (GitRefStatus, Option<&'static str>) {
    (GitRefStatus::Rejected, Some(reason.code()))
}

fn is_ancestor(repo: &gix::Repository, old: ObjectId, new: ObjectId) -> bool {
    repo.has_object(old)
        && repo
            .merge_base(old, new)
            .is_ok_and(|base| base.is_some_and(|base| base.detach() == old))
}

fn fail_atomically(updates: &mut [RefUpdate]) {
    if !updates.iter().any(RefUpdate::is_rejected) {
        return;
    }
    for update in updates.iter_mut().filter(|update| update.is_sent()) {
        update.reject(RejectReason::AtomicFailed.code());
    }
}

fn count_objects(
    repo: &gix::Repository,
    updates: &[RefUpdate],
    advertisement: &Advertisement,
) -> Result<u64> {
    let tips: Vec<ObjectId> = updates
        .iter()
        .filter(|update| update.is_sent())
        .filter_map(|update| update.new)
        .collect();
    if tips.is_empty() {
        return Ok(0);
    }
    let mut known: Vec<ObjectId> = advertisement
        .refs()
        .iter()
        .map(|advertised| advertised.id)
        .collect();
    known.extend_from_slice(advertisement.extra_haves());
    let objects = send_pack::objects_to_send(repo, &tips, &known, &AtomicBool::new(false))?;
    Ok(u64::try_from(objects.len()).unwrap_or(u64::MAX))
}

fn upstream_of(specs: &[Spec], remote: &str) -> Option<Upstream> {
    let spec = specs.first()?;
    let local_ref = spec.local_ref.clone()?;
    let branch = local_ref.strip_prefix("refs/heads/")?.to_owned();
    Some(Upstream {
        local_ref,
        branch,
        remote: remote.to_owned(),
        merge: spec.remote_ref.clone(),
    })
}

fn pre_push_hook(repo: &gix::Repository, git_dir: &Path) -> Option<PathBuf> {
    let config = repo.config_snapshot();
    let dir = config
        .trusted_path("core.hooksPath")
        .ok()
        .flatten()
        .map_or_else(
            || git_dir.join("hooks"),
            |path| repo.workdir().unwrap_or(git_dir).join(path),
        );
    let hook = dir.join("pre-push");
    is_executable(&hook).then_some(hook)
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Changes of the local repository that follow the accepted updates.
struct LocalRefs {
    git_dir: PathBuf,
    identity: Identity,
    policy: LogPolicy,
    updates: Vec<RefUpdate>,
    upstream: Option<Upstream>,
}

impl LocalRefs {
    fn follows(update: &RefUpdate, accepted: &[BString]) -> bool {
        update.status == GitRefStatus::UpToDate
            || accepted
                .iter()
                .any(|name| name.as_bytes() == update.remote_ref.as_bytes())
    }

    fn plan(&self, accepted: &[BString]) -> Result<(FilePlan, bool)> {
        let mut edits = Vec::new();
        for update in self
            .updates
            .iter()
            .filter(|update| Self::follows(update, accepted))
        {
            let Some(tracking) = &update.tracking_ref else {
                continue;
            };
            let current = send_pack::read_ref(&self.git_dir, tracking.as_bytes())?;
            let target = match update.status {
                GitRefStatus::Deleted => None,
                _ => update.new,
            };
            if current != target {
                edits.push(RefEdit {
                    name: tracking.as_str().into(),
                    old: current,
                    new: target,
                    message: TRACKING_MESSAGE.to_owned(),
                });
            }
        }
        let mut plan = FilePlan::new();
        send_pack::plan_ref_edits(
            &self.git_dir,
            &edits,
            &self.identity,
            self.policy,
            &mut plan,
        )?;
        let upstream = self.upstream.as_ref().filter(|upstream| {
            self.updates.iter().any(|update| {
                update.local_ref.as_deref() == Some(upstream.local_ref.as_str())
                    && Self::follows(update, accepted)
            })
        });
        if let Some(upstream) = upstream {
            plan_upstream(&self.git_dir, upstream, &mut plan)?;
        }
        Ok((plan, upstream.is_some()))
    }
}

fn plan_upstream(git_dir: &Path, upstream: &Upstream, plan: &mut FilePlan) -> Result<()> {
    let path = git_dir.join("config");
    let bytes = std::fs::read(&path).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("reading `{}` failed: {error}", path.display()),
        )
    })?;
    let updated = set_upstream_config(&bytes, &upstream.branch, &upstream.remote, &upstream.merge);
    if updated != bytes {
        plan.replace_locked(git_dir, &path, updated)?;
    }
    Ok(())
}

/// Sets `branch.<branch>.remote` and `branch.<branch>.merge` in the text of
/// a configuration file; existing values are replaced in place and missing
/// keys are added at the end of the section, as `git config` does.
fn set_upstream_config(text: &[u8], branch: &str, remote: &str, merge: &str) -> Vec<u8> {
    let header = format!("[branch \"{branch}\"]");
    let wanted = [
        ("remote", config_value(remote)),
        ("merge", config_value(merge)),
    ];
    let mut lines: Vec<Vec<u8>> = text.lines_with_terminator().map(<[u8]>::to_vec).collect();
    if let Some(last) = lines.last_mut()
        && !last.ends_with(b"\n")
    {
        last.push(b'\n');
    }
    let Some(start) = lines
        .iter()
        .position(|line| line.trim() == header.as_bytes())
    else {
        let mut out = lines.concat();
        out.extend_from_slice(header.as_bytes());
        out.push(b'\n');
        for (key, value) in &wanted {
            out.extend_from_slice(format!("\t{key} = {value}\n").as_bytes());
        }
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.trim_start().starts_with(b"["))
        .map_or(lines.len(), |offset| start + 1 + offset);
    let mut missing = Vec::new();
    for (key, value) in &wanted {
        let existing = lines[start + 1..end]
            .iter()
            .position(|line| config_key(line) == Some(*key));
        match existing {
            Some(offset) => lines[start + 1 + offset] = format!("\t{key} = {value}\n").into_bytes(),
            None => missing.push(format!("\t{key} = {value}\n").into_bytes()),
        }
    }
    let mut out = lines[..end].concat();
    for line in missing {
        out.extend_from_slice(&line);
    }
    out.extend_from_slice(&lines[end..].concat());
    out
}

fn config_key(line: &[u8]) -> Option<&'static str> {
    let key = line.split_str(b"=").next()?.trim().to_ascii_lowercase();
    match key.as_slice() {
        b"remote" => Some("remote"),
        b"merge" => Some("merge"),
        _ => None,
    }
}

fn config_value(value: &str) -> String {
    if value.contains([' ', '#', ';', '"', '\\', '\t']) {
        quote(value)
    } else {
        value.to_owned()
    }
}

/// Sink of the reference changes accepted by a local remote: the changes of
/// the remote, the remote-tracking references and the upstream
/// configuration are declared in one call of the transaction.
struct PushSink<'t> {
    tx: &'t mut dyn MutationTx,
    local: LocalRefs,
    applied: bool,
    upstream_set: bool,
    changed: Vec<PathBuf>,
}

impl PushSink<'_> {
    fn apply_plans(&mut self, remote: Option<&FilePlan>, accepted: &[BString]) -> Result<()> {
        let (local, upstream_set) = self.local.plan(accepted)?;
        let mut plans = Vec::with_capacity(2);
        plans.extend(remote);
        plans.push(&local);
        send_pack::preserve_and_apply(&mut *self.tx, &plans)?;
        for plan in plans {
            self.changed.extend(plan.changed_files());
        }
        self.applied = true;
        self.upstream_set = upstream_set;
        Ok(())
    }

    fn finish(&mut self, accepted: &[BString]) -> Result<()> {
        if self.applied {
            return Ok(());
        }
        self.apply_plans(None, accepted)
    }
}

impl UpdateSink for PushSink<'_> {
    fn apply(&mut self, plan: &FilePlan, accepted: &[RefCommand]) -> Result<()> {
        let names: Vec<BString> = accepted
            .iter()
            .map(|command| command.name.clone())
            .collect();
        self.apply_plans(Some(plan), &names)
    }

    fn objects_written(&mut self, git_dir: &Path) {
        if let Ok(repo) = AbsPath::new(git_dir) {
            self.tx.record_exempt(ExemptWrite::ObjectStore { repo });
        }
    }
}

impl PlannedPush {
    /// Updates as planned.
    #[must_use]
    pub fn updates(&self) -> &[RefUpdate] {
        &self.updates
    }

    /// Irreversible effect of the push: updates sent to a remote that is not
    /// on the local file system.
    #[must_use]
    pub fn irreversible(&self) -> Option<IrreversibleEntry> {
        let Target::Network(address) = &self.target else {
            return None;
        };
        let count = self
            .updates
            .iter()
            .filter(|update| update.is_sent())
            .count();
        (count > 0).then(|| remote_updated(address, count))
    }

    /// Outcome of the push as planned, shown by a preview.
    #[must_use]
    pub fn preview(&self) -> PushOutcome {
        PushOutcome {
            repo: self.root.clone(),
            remote: self.remote.clone(),
            updates: self.updates.clone(),
            objects: self.objects,
            upstream: self
                .upstream
                .as_ref()
                .map(|upstream| format!("{}/{}", upstream.remote, upstream.branch)),
            transport_notable: self.notes.clone(),
            ..PushOutcome::default()
        }
    }

    /// Runs the `pre-push` hook, sends the updates and moves the
    /// remote-tracking references through `tx`.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_HOOK` when the `pre-push` hook declines,
    /// `E_GIT_PROTOCOL` when the remote breaks the protocol, `E_STALE` when
    /// a reference changed after the plan, `E_GIT_LOCKED` when a lock file
    /// exists and with the errors of the journal; the caller rolls the
    /// transaction back. Rejected updates are not errors here; see
    /// [`PushOutcome::check`].
    pub fn apply(self, tx: &mut dyn MutationTx, context: &PushContext<'_>) -> Result<PushOutcome> {
        let repo = open_local(&self.root)?;
        let hooks_skipped = self.run_pre_push(context)?;
        let mut outcome = self.preview();
        outcome.hooks_skipped = hooks_skipped;
        outcome.upstream = None;
        let commands: Vec<RefCommand> = self
            .updates
            .iter()
            .filter(|update| update.is_sent())
            .map(|update| update.command(self.hash))
            .collect();
        let mut sink = PushSink {
            tx,
            local: LocalRefs {
                git_dir: self.git_dir.clone(),
                identity: context.identity.clone(),
                policy: LogPolicy::of(&repo),
                updates: self.updates.clone(),
                upstream: self.upstream.clone(),
            },
            applied: false,
            upstream_set: false,
            changed: Vec::new(),
        };
        if !commands.is_empty() {
            let sent = match &self.target {
                Target::Local(path) => {
                    self.transfer_local(&repo, path, &commands, &mut sink, context, &mut outcome)?
                }
                Target::Network(address) => {
                    self.transfer_network(&repo, address, &commands, &mut sink, &mut outcome)?
                }
            };
            merge_report(&mut outcome.updates, sent.report.as_ref());
            outcome.status_unknown = sent.report.is_none()
                || outcome
                    .updates
                    .iter()
                    .any(|update| update.reason.as_deref() == Some(NOT_REPORTED));
            outcome.objects = sent.pack.map_or(0, |pack| pack.objects);
            outcome.pack = sent.pack;
            outcome.remote_messages = sent.remote_messages;
        }
        let accepted: Vec<BString> = outcome
            .updates
            .iter()
            .filter(|update| update.is_sent())
            .map(|update| update.remote_ref.as_str().into())
            .collect();
        sink.finish(&accepted)?;
        if sink.upstream_set {
            outcome.upstream = self.preview().upstream;
        }
        outcome.changed_files = sink.changed;
        Ok(outcome)
    }

    fn run_pre_push(&self, context: &PushContext<'_>) -> Result<bool> {
        let Some(hook) = &self.pre_push else {
            return Ok(false);
        };
        if !self.updates.iter().any(RefUpdate::is_sent) {
            return Ok(false);
        }
        let host = match context.host {
            Some(host) if self.hooks => host,
            _ => return Ok(true),
        };
        let stdin = pre_push_input(&self.updates, self.hash);
        let args = vec![self.remote.clone(), self.url.clone()];
        let output =
            ProcessHooks::new(host, context.cancel).run(hook, &args, &stdin, &self.root, &[])?;
        if output.success {
            return Ok(false);
        }
        Err(Error::new(
            ErrorCode::GitHook,
            "the pre-push hook declined the push; correct the problem reported by the hook and push again",
        )
        .with_problem("pre_push_declined")
        .with_detail(
            "output",
            serde_json::Value::String(output.output.to_str_lossy().trim_end().to_owned()),
        ))
    }

    fn send_options(&self) -> SendOptions {
        SendOptions {
            atomic: self.atomic,
            ..SendOptions::default()
        }
    }

    fn transfer_local(
        &self,
        repo: &gix::Repository,
        path: &Path,
        commands: &[RefCommand],
        sink: &mut PushSink<'_>,
        context: &PushContext<'_>,
        outcome: &mut PushOutcome,
    ) -> Result<SendOutcome> {
        let runner = context
            .host
            .filter(|_| self.hooks)
            .map(|host| ProcessHooks::new(host, context.cancel));
        let hooks = runner.as_ref().map(|runner| runner as &dyn HookRunner);
        let options = receive_options(context.identity.clone(), self.hooks);
        let service = ReceivePack::open(path, options)?;
        let (mut transport, session) = receive::connect(service, &self.url, hooks, sink)?;
        let sent = send_pack::discover(&mut transport).and_then(|advertisement| {
            send_pack::send(
                &mut transport,
                repo,
                &advertisement,
                commands,
                &[],
                &self.send_options(),
                &AtomicBool::new(false),
            )
        });
        drop(transport);
        let summary = session.finish();
        drop(session);
        let sent = sent?;
        if let Some(mut summary) = summary? {
            if let Some(error) = summary.update_error.take() {
                return Err(error);
            }
            outcome.hooks_skipped |= summary.hooks_skipped;
        }
        Ok(sent)
    }

    /// Sends the updates over HTTP or SSH. Updates the remote may have
    /// applied are marked irreversible, including those of a request that
    /// failed after it was sent.
    fn transfer_network(
        &self,
        repo: &gix::Repository,
        address: &str,
        commands: &[RefCommand],
        sink: &mut PushSink<'_>,
        outcome: &mut PushOutcome,
    ) -> Result<SendOutcome> {
        let mut connection = NetworkConnection::open(repo, &self.remote, address, &self.network)?;
        let advertisement = connection.discover()?;
        let sent = send_pack::send(
            connection.transport.as_mut(),
            repo,
            &advertisement,
            commands,
            &[],
            &self.send_options(),
            &AtomicBool::new(false),
        );
        let accepted = match &sent {
            Ok(SendOutcome {
                report: Some(report),
                ..
            }) => commands
                .iter()
                .filter(|command| {
                    report
                        .get(command.name.as_slice())
                        .is_some_and(|entry| matches!(entry.status, RefStatus::Ok))
                })
                .count(),
            Ok(_) | Err(_) => commands.len(),
        };
        if accepted > 0 {
            sink.tx.mark_irreversible(remote_updated(address, accepted));
        }
        outcome.transport_notable = connection.notable();
        if sent.is_ok() {
            connection.store_credentials(&mut *sink.tx)?;
        }
        sent
    }
}

fn pre_push_input(updates: &[RefUpdate], hash: gix::hash::Kind) -> Vec<u8> {
    let null = ObjectId::null(hash);
    let mut stdin = Vec::new();
    for update in updates.iter().filter(|update| update.is_sent()) {
        let (local, new) = match (update.status, update.new) {
            (GitRefStatus::Deleted, _) | (_, None) => ("(delete)".to_owned(), null),
            (_, Some(new)) => (
                update.local_ref.clone().unwrap_or_else(|| new.to_string()),
                new,
            ),
        };
        let old = update.old.unwrap_or(null);
        stdin.extend_from_slice(format!("{local} {new} {} {old}\n", update.remote_ref).as_bytes());
    }
    stdin
}

fn merge_report(updates: &mut [RefUpdate], report: Option<&PushReport>) {
    let Some(report) = report else {
        return;
    };
    for update in updates.iter_mut().filter(|update| update.is_sent()) {
        match report
            .get(update.remote_ref.as_bytes())
            .map(|entry| &entry.status)
        {
            Some(RefStatus::Ok) => {}
            Some(RefStatus::Rejected(text)) => update.reject(RejectReason::classify(text).code()),
            None => {
                update.status = GitRefStatus::Error;
                update.reason = Some(NOT_REPORTED.to_owned());
            }
        }
    }
}

/// Identity of reflog entries: `GIT_COMMITTER_NAME` and
/// `GIT_COMMITTER_EMAIL`, then `user.name` and `user.email` of the
/// repository, then the account and host names; the time of the clock in the
/// local time zone of the host.
fn identity_of(ctx: &ExecCtx, repo: &gix::Repository) -> Identity {
    let host = ctx.host();
    let var = |name: &str| {
        host.env_var(name)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.is_empty())
    };
    let config = repo.config_snapshot();
    let setting = |key: &str| {
        config
            .string(key)
            .map(|value| value.to_str_lossy().into_owned())
    };
    let info = host.info();
    let name = var("GIT_COMMITTER_NAME")
        .or_else(|| setting("user.name"))
        .unwrap_or_else(|| info.user.clone());
    let email = var("GIT_COMMITTER_EMAIL")
        .or_else(|| setting("user.email"))
        .unwrap_or_else(|| format!("{}@{}", info.user, info.hostname));
    let since_epoch = ctx.clock().now().unix_nanos();
    let seconds = i64::try_from(since_epoch.div_euclid(NANOSECONDS_PER_SECOND)).unwrap_or_default();
    let offset = jiff::tz::TimeZone::get(&info.timezone)
        .ok()
        .zip(jiff::Timestamp::from_second(seconds).ok())
        .map_or(0, |(tz, instant)| tz.to_offset(instant).seconds());
    Identity {
        name: name.into(),
        email: email.into(),
        seconds,
        offset,
    }
}

fn request_of(arguments: &BoundArgs) -> Result<(PathBuf, PushRequest)> {
    let repo = args::path(arguments, "repo")?;
    let mut request = PushRequest::new(args::required_text(arguments, "branch")?);
    if let Some(remote) = args::text(arguments, "remote")? {
        request.remote = remote;
    }
    request.force = ForceMode::from_value(arguments.value("force"))?;
    request.force_with_lease = args::text(arguments, "force_with_lease")?;
    request.tags = args::flag(arguments, "tags")?;
    request.set_upstream = args::flag(arguments, "set_upstream")?;
    request.delete = args::flag(arguments, "delete")?;
    Ok((repo, request))
}

/// Stage implementation of `git.push`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PushCommand;

impl StageImpl for PushCommand {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for PushCommand {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let (repo, request) = request_of(args)?;
        let planned = plan_push(&repo, &request, &NetworkOptions::from_ctx(ctx))?;
        let preview = planned.preview();
        let mut plan = EffectPlan::new();
        plan.notable = preview.notable();
        plan.irreversible.extend(planned.irreversible());
        plan.preview = preview.records(EffectStatus::Planned);
        plan.bulk = planned
            .updates
            .iter()
            .filter(|update| update.is_sent())
            .count()
            > 1;
        Ok(plan.with_state(planned))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan
            .take_state::<PlannedPush>()
            .ok_or_else(|| Error::new(ErrorCode::Internal, "the plan of git.push is missing"))?;
        let identity = identity_of(ctx, &open_local(&planned.root)?);
        let context = PushContext {
            identity,
            host: ctx.process(),
            cancel: ctx.cancel(),
        };
        let outcome = planned.apply(tx, &context)?;
        outcome.check().map_err(|mut error| {
            error.effect_applied = !outcome.changed_files.is_empty();
            error
        })?;
        let records = outcome.records(EffectStatus::Ok);
        let mut result = EffectOutcome::new(StageOutput::records(RecordStream::new(
            records.into_iter().map(Ok),
        )));
        result.changes = outcome.changes();
        result.notable = outcome.notable();
        Ok(result)
    }
}

#[cfg(test)]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_git_transport::send_pack::testing::{Fixture, RecordingTx, ScriptHost};

    fn plan_push(path: &Path, request: &PushRequest) -> Result<PlannedPush> {
        super::plan_push(path, request, &NetworkOptions::default())
    }

    fn identity() -> Identity {
        Identity {
            name: "Pusher".into(),
            email: "pusher@example.com".into(),
            seconds: 1_700_000_500,
            offset: 3600,
        }
    }

    fn linked(remote: &Fixture) -> Fixture {
        let local = Fixture::new();
        local.config(&format!(
            "[remote \"origin\"]\n\turl = {}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
            remote.root().display()
        ));
        local
    }

    fn run(local: &Fixture, request: &PushRequest) -> (Result<PushOutcome>, RecordingTx) {
        let host = ScriptHost::default();
        let cancel = CancelToken::new();
        let context = PushContext {
            identity: identity(),
            host: Some(&host),
            cancel: &cancel,
        };
        let mut tx = RecordingTx::new();
        let outcome =
            plan_push(local.root(), request).and_then(|planned| planned.apply(&mut tx, &context));
        (outcome, tx)
    }

    fn push(local: &Fixture, request: &PushRequest) -> (PushOutcome, RecordingTx) {
        let (outcome, tx) = run(local, request);
        (outcome.unwrap(), tx)
    }

    fn status(outcome: &PushOutcome) -> Vec<(String, GitRefStatus, Option<String>)> {
        outcome
            .updates
            .iter()
            .map(|update| {
                (
                    update.remote_ref.clone(),
                    update.status,
                    update.reason.clone(),
                )
            })
            .collect()
    }

    fn one(
        name: &str,
        status: GitRefStatus,
        reason: Option<&str>,
    ) -> Vec<(String, GitRefStatus, Option<String>)> {
        vec![(name.to_owned(), status, reason.map(str::to_owned))]
    }

    #[test]
    fn pushes_a_new_branch_moves_the_tracking_ref_and_undoes_both() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let (outcome, mut tx) = push(&local, &PushRequest::new("main"));
        assert_eq!(
            status(&outcome),
            one("refs/heads/main", GitRefStatus::New, None)
        );
        assert_eq!(outcome.objects, 3);
        assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
        assert_eq!(local.ref_value("refs/remotes/origin/main"), Some(tip));
        let log =
            std::fs::read_to_string(local.git_dir().join("logs/refs/remotes/origin/main")).unwrap();
        assert!(log.ends_with("Pusher <pusher@example.com> 1700000500 +0100\tupdate by push\n"));
        assert_eq!(tx.exempt.len(), 1);
        assert!(outcome.check().is_ok());
        assert!(!outcome.changed_files.is_empty());
        tx.undo();
        assert_eq!(remote.ref_value("refs/heads/main"), None);
        assert_eq!(local.ref_value("refs/remotes/origin/main"), None);
    }

    #[test]
    fn fast_forwards_and_reports_up_to_date() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let base = local.commit("main", &[("a.txt", "one\n")], &[]);
        push(&local, &PushRequest::new("main"));
        let next = local.commit("main", &[("a.txt", "two\n")], &[base]);
        let (outcome, _) = push(&local, &PushRequest::new("main"));
        assert_eq!(
            status(&outcome),
            one("refs/heads/main", GitRefStatus::Updated, None)
        );
        assert_eq!(outcome.objects, 3);
        assert_eq!(remote.ref_value("refs/heads/main"), Some(next));
        let (outcome, tx) = push(&local, &PushRequest::new("main"));
        assert_eq!(
            status(&outcome),
            one("refs/heads/main", GitRefStatus::UpToDate, None)
        );
        assert_eq!(outcome.state(), GitEffectState::UpToDate);
        assert!(tx.targets().is_empty());
    }

    #[test]
    fn rejects_fetch_first_and_non_fast_forward() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let unknown = remote.commit("main", &[("r.txt", "remote\n")], &[]);
        local.commit("main", &[("a.txt", "one\n")], &[]);
        let (outcome, _) = push(&local, &PushRequest::new("main"));
        assert_eq!(
            status(&outcome),
            one(
                "refs/heads/main",
                GitRefStatus::Rejected,
                Some("fetch_first")
            )
        );
        let error = outcome.check().unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitPushRejected);
        assert!(error.fix.iter().any(|fix| fix.starts_with("git.pull")));
        assert_eq!(remote.ref_value("refs/heads/main"), Some(unknown));

        let other = Fixture::bare();
        let local = linked(&other);
        local.commit("main", &[("a.txt", "one\n")], &[]);
        push(&local, &PushRequest::new("main"));
        local.commit("main", &[("b.txt", "unrelated\n")], &[]);
        let (outcome, _) = push(&local, &PushRequest::new("main"));
        assert_eq!(
            status(&outcome),
            one(
                "refs/heads/main",
                GitRefStatus::Rejected,
                Some("non_fast_forward")
            )
        );
    }

    #[test]
    fn forces_updates_with_yes_and_with_a_valid_lease() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        local.commit("main", &[("a.txt", "one\n")], &[]);
        push(&local, &PushRequest::new("main"));
        let rewritten = local.commit("main", &[("b.txt", "unrelated\n")], &[]);
        let lease = PushRequest {
            force: ForceMode::Lease,
            ..PushRequest::new("main")
        };
        let (outcome, mut tx) = push(&local, &lease);
        assert_eq!(
            status(&outcome),
            one("refs/heads/main", GitRefStatus::Forced, None)
        );
        let codes: Vec<Notable> = outcome.notable().iter().map(|entry| entry.code).collect();
        assert_eq!(codes, [Notable::ForcePush, Notable::HistoryRewritten]);
        assert_eq!(remote.ref_value("refs/heads/main"), Some(rewritten));
        tx.undo();
        let forced = PushRequest {
            force: ForceMode::Yes,
            ..PushRequest::new("main")
        };
        let (outcome, _) = push(&local, &forced);
        assert_eq!(
            status(&outcome),
            one("refs/heads/main", GitRefStatus::Forced, None)
        );
        assert_eq!(outcome.state(), GitEffectState::Completed);
    }

    #[test]
    fn rejects_a_stale_lease() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let base = local.commit("main", &[("a.txt", "one\n")], &[]);
        push(&local, &PushRequest::new("main"));
        let rewritten = local.commit("main", &[("b.txt", "unrelated\n")], &[]);
        local.set_ref("refs/remotes/origin/main", rewritten);
        let lease = PushRequest {
            force: ForceMode::Lease,
            ..PushRequest::new("main")
        };
        let (outcome, _) = push(&local, &lease);
        assert_eq!(
            status(&outcome),
            one(
                "refs/heads/main",
                GitRefStatus::Rejected,
                Some("stale_info")
            )
        );
        assert!(!outcome.check().unwrap_err().next_steps.is_empty());
        let explicit = PushRequest {
            force_with_lease: Some(base.to_string()),
            ..PushRequest::new("main")
        };
        let (outcome, _) = push(&local, &explicit);
        assert_eq!(
            status(&outcome),
            one("refs/heads/main", GitRefStatus::Forced, None)
        );
    }

    #[test]
    fn deletes_a_remote_branch_and_its_tracking_ref() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let tip = local.commit("topic", &[("a.txt", "one\n")], &[]);
        push(&local, &PushRequest::new("topic"));
        assert_eq!(local.ref_value("refs/remotes/origin/topic"), Some(tip));
        let delete = PushRequest {
            delete: true,
            ..PushRequest::new("topic")
        };
        let (outcome, mut tx) = push(&local, &delete);
        assert_eq!(
            status(&outcome),
            one("refs/heads/topic", GitRefStatus::Deleted, None)
        );
        assert_eq!(remote.ref_value("refs/heads/topic"), None);
        assert_eq!(local.ref_value("refs/remotes/origin/topic"), None);
        tx.undo();
        assert_eq!(remote.ref_value("refs/heads/topic"), Some(tip));
        assert_eq!(local.ref_value("refs/remotes/origin/topic"), Some(tip));
        let (outcome, _) = push(&local, &PushRequest::new(":refs/heads/missing"));
        assert_eq!(
            status(&outcome),
            one(
                "refs/heads/missing",
                GitRefStatus::Error,
                Some(REMOTE_REF_MISSING)
            )
        );
        assert_eq!(
            outcome.check().unwrap_err().problem.as_deref(),
            Some(REMOTE_REF_MISSING)
        );
    }

    #[test]
    fn pushes_tags_and_rejects_a_moved_tag() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let base = local.commit("main", &[("a.txt", "one\n")], &[]);
        let tag = local.tag("v1", base);
        let tags = PushRequest {
            tags: true,
            ..PushRequest::new("main")
        };
        let (outcome, _) = push(&local, &tags);
        assert_eq!(outcome.updates[1].remote_ref, "refs/tags/v1");
        assert_eq!(outcome.updates[1].status, GitRefStatus::New);
        assert_eq!(remote.ref_value("refs/tags/v1"), Some(tag));
        let next = local.commit("main", &[("a.txt", "two\n")], &[base]);
        local.set_ref("refs/tags/v1", next);
        let (outcome, _) = push(&local, &tags);
        assert_eq!(outcome.updates[0].status, GitRefStatus::Updated);
        assert_eq!(outcome.updates[1].reason.as_deref(), Some("already_exists"));
        assert_eq!(outcome.state(), GitEffectState::Stopped);
        assert!(outcome.check().is_ok(), "the branch was updated");
        let atomic = PushRequest {
            atomic: true,
            ..tags.clone()
        };
        local.commit("main", &[("a.txt", "three\n")], &[next]);
        let (outcome, _) = push(&local, &atomic);
        assert_eq!(outcome.updates[0].reason.as_deref(), Some("atomic_failed"));
        assert_eq!(remote.ref_value("refs/heads/main"), Some(next));
    }

    #[test]
    fn sets_the_upstream_and_restores_the_configuration_on_undo() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        local.commit("main", &[("a.txt", "one\n")], &[]);
        let before = std::fs::read(local.git_dir().join("config")).unwrap();
        let request = PushRequest {
            set_upstream: true,
            ..PushRequest::new("HEAD")
        };
        let (outcome, mut tx) = push(&local, &request);
        assert_eq!(outcome.upstream.as_deref(), Some("origin/main"));
        let config = std::fs::read_to_string(local.git_dir().join("config")).unwrap();
        assert!(
            config.ends_with("[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n")
        );
        assert!(
            outcome
                .notable()
                .iter()
                .any(|entry| entry.code == Notable::UpstreamSet)
        );
        tx.undo();
        assert_eq!(
            std::fs::read(local.git_dir().join("config")).unwrap(),
            before
        );
    }

    #[test]
    fn reports_the_checked_out_branch_of_a_non_bare_remote() {
        let remote = Fixture::new();
        let local = linked(&remote);
        local.commit("main", &[("a.txt", "one\n")], &[]);
        let (outcome, _) = push(&local, &PushRequest::new("main"));
        assert_eq!(
            status(&outcome),
            one(
                "refs/heads/main",
                GitRefStatus::Rejected,
                Some("checked_out")
            )
        );
        assert!(
            outcome
                .remote_messages
                .iter()
                .any(|line| line.contains("refusing to update checked out branch"))
        );
        assert_eq!(
            outcome.check().unwrap_err().problem.as_deref(),
            Some("checked_out")
        );
    }

    #[cfg(unix)]
    #[test]
    fn runs_the_pre_push_hook_and_skips_it_when_disabled() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        local.hook(
            "pre-push",
            "#!/bin/sh\ncat > pre-push.in\necho \"$1\" >> pre-push.in\nexit 1\n",
        );
        let (outcome, _) = run(&local, &PushRequest::new("main"));
        assert_eq!(outcome.unwrap_err().code(), ErrorCode::GitHook);
        let input = std::fs::read_to_string(local.root().join("pre-push.in")).unwrap();
        let zero = "0".repeat(40);
        assert_eq!(
            input,
            format!("refs/heads/main {tip} refs/heads/main {zero}\norigin\n")
        );
        assert_eq!(remote.ref_value("refs/heads/main"), None);
        let request = PushRequest {
            hooks: false,
            ..PushRequest::new("main")
        };
        let (outcome, _) = push(&local, &request);
        assert!(outcome.hooks_skipped);
        assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
    }

    #[test]
    fn resolves_refspecs_and_reports_bad_input() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let (outcome, _) = push(&local, &PushRequest::new("main:other"));
        assert_eq!(
            status(&outcome),
            one("refs/heads/other", GitRefStatus::New, None)
        );
        assert_eq!(
            outcome.updates[0].tracking_ref.as_deref(),
            Some("refs/remotes/origin/other")
        );
        let (outcome, _) = push(&local, &PushRequest::new(format!("+{tip}:refs/heads/raw")));
        assert_eq!(outcome.updates[0].local_ref, None);
        assert_eq!(remote.ref_value("refs/heads/raw"), Some(tip));
        let planned = plan_push(local.root(), &PushRequest::new("main")).unwrap();
        assert_eq!(planned.updates()[0].status, GitRefStatus::New);
        assert!(planned.preview().records(EffectStatus::Planned).len() == 2);
        for (branch, code) in [
            ("nonexistent", ErrorCode::GitRevision),
            (&*tip.to_string(), ErrorCode::GitRevision),
            ("", ErrorCode::GitRef),
            ("main:bad..name", ErrorCode::GitRef),
        ] {
            let error = plan_push(local.root(), &PushRequest::new(branch)).unwrap_err();
            assert_eq!(error.code(), code, "{branch}");
        }
        let delete = PushRequest {
            delete: true,
            ..PushRequest::new("a:b")
        };
        assert_eq!(
            plan_push(local.root(), &delete).unwrap_err().code(),
            ErrorCode::GitRef
        );
    }

    #[test]
    fn reports_unknown_remotes_and_unsupported_transports() {
        let remote = Fixture::bare();
        let local = linked(&remote);
        local.commit("main", &[("a.txt", "one\n")], &[]);
        local.config("[remote \"web\"]\n\turl = git://example.com/r.git\n");
        let error = plan_push(
            local.root(),
            &PushRequest {
                remote: "web".to_owned(),
                ..PushRequest::new("main")
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitUnsupported);
        let error = plan_push(
            local.root(),
            &PushRequest {
                remote: "nowhere".to_owned(),
                ..PushRequest::new("main")
            },
        )
        .unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("unknown_remote"));
        let missing = local.root().join("missing.git").display().to_string();
        let error = plan_push(
            local.root(),
            &PushRequest {
                remote: missing,
                ..PushRequest::new("main")
            },
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitNotRepository);
        let error = plan_push(
            &tempfile::tempdir().unwrap().path().join("no-repository"),
            &PushRequest::new("main"),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitNotRepository);
        let direct = PushRequest {
            remote: remote.root().display().to_string(),
            ..PushRequest::new("main")
        };
        let (outcome, _) = push(&local, &direct);
        assert_eq!(outcome.updates[0].tracking_ref, None);
        assert_eq!(outcome.updates[0].status, GitRefStatus::New);
    }

    #[test]
    fn edits_the_upstream_configuration_like_git_config() {
        let appended = set_upstream_config(
            b"[core]\n\tbare = false",
            "main",
            "origin",
            "refs/heads/main",
        );
        assert_eq!(
            appended,
            b"[core]\n\tbare = false\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n"
        );
        let existing = b"[branch \"main\"]\n\tmerge = refs/heads/old\n\trebase = true\n[core]\n\tbare = false\n";
        let replaced = set_upstream_config(existing, "main", "my remote", "refs/heads/main");
        assert_eq!(
            String::from_utf8(replaced).unwrap(),
            "[branch \"main\"]\n\tmerge = refs/heads/main\n\trebase = true\n\tremote = \"my remote\"\n[core]\n\tbare = false\n"
        );
    }

    #[test]
    fn maps_remote_references_to_tracking_references() {
        let fetch = vec![
            "+refs/heads/*:refs/remotes/origin/*".to_owned(),
            "refs/notes/commits:refs/notes/origin".to_owned(),
            "invalid".to_owned(),
        ];
        assert_eq!(
            tracking_name(&fetch, "refs/heads/a/b").as_deref(),
            Some("refs/remotes/origin/a/b")
        );
        assert_eq!(
            tracking_name(&fetch, "refs/notes/commits").as_deref(),
            Some("refs/notes/origin")
        );
        assert_eq!(tracking_name(&fetch, "refs/tags/v1"), None);
    }

    #[test]
    fn parses_force_modes() {
        assert_eq!(
            ForceMode::from_value(Some(&Value::Bool(true))).unwrap(),
            ForceMode::Lease
        );
        assert_eq!(
            ForceMode::from_value(Some(&Value::Bool(false))).unwrap(),
            ForceMode::No
        );
        assert_eq!(ForceMode::from_value(None).unwrap(), ForceMode::No);
        assert_eq!(
            ForceMode::from_value(Some(&Value::Enum("yes".into()))).unwrap(),
            ForceMode::Yes
        );
        assert_eq!(ForceMode::Yes.as_str(), "yes");
        assert_eq!(ForceMode::Lease.as_str(), "lease");
        for value in [Value::Int(1), Value::Text("always".into())] {
            assert_eq!(
                ForceMode::from_value(Some(&value)).unwrap_err().code(),
                ErrorCode::UnknownValue
            );
        }
    }

    #[test]
    fn describes_the_stage() {
        let descriptor = PushCommand.descriptor();
        assert_eq!(descriptor.command, "git.push");
        assert!(PushCommand.as_effect().is_some());
    }
}
