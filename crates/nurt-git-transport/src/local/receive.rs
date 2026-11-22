// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-process receive-pack service for pushes to local repositories.
//!
//! For `file://` URLs and paths `git` starts the `git-receive-pack` program;
//! this service performs the same steps inside the process: it advertises
//! the references of the target repository, reads the update commands and
//! the pack, stores the pack in `objects/pack`, checks that the new tips are
//! complete, applies `receive.denyCurrentBranch`, `receive.denyDeleteCurrent`,
//! `receive.denyNonFastForwards`, `receive.denyDeletes` and
//! `receive.hideRefs`, runs the `pre-receive`, `update` and `post-receive`
//! hooks through a [`HookRunner`] and updates the references through an
//! [`UpdateSink`], which records them in the journal. The service speaks
//! protocol version 0 on byte streams ([`ReceivePack::advertise`],
//! [`ReceivePack::serve`]), so it also serves the smart HTTP and SSH test
//! servers; [`connect`] wraps it in a client transport without threads.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use bstr::{BString, ByteSlice};
use gix::ObjectId;
use gix_transport::client::git::ConnectMode;
use gix_transport::client::git::blocking_io::Connection;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{ExemptWrite, MutationTx};
use nurt_contracts::services::{EnvVar, ProcessHost, ProgramInput, ProgramSpec};
use nurt_contracts::stage::CancelToken;

use crate::send_pack::pkt::{self, Packet};
use crate::send_pack::{
    FilePlan, Identity, LogPolicy, MISSING_OBJECT, RefCommand, RefEdit, objects_to_send,
    plan_ref_edits, preserve_and_apply, read_ref, validate_name,
};

/// Reflog message of references updated by a push, as written by `git`.
const REFLOG_MESSAGE: &str = "push";

/// Size of the pack header: signature, version and object count.
const PACK_HEADER: usize = 12;

/// Size of the SHA-1 pack trailer.
const PACK_TRAILER: usize = 20;

/// Options of the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveOptions {
    /// Agent string advertised to the client.
    pub agent: String,
    /// Advertise `report-status-v2` in addition to `report-status`.
    pub report_status_v2: bool,
    /// Run the hooks of the target repository.
    pub run_hooks: bool,
    /// Identity written into reflog entries of the target repository.
    pub identity: Identity,
}

/// Output of a hook run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutput {
    /// The hook ended with exit code 0.
    pub success: bool,
    /// Standard output followed by standard error.
    pub output: Vec<u8>,
}

/// Runs repository hooks, which are user programs.
pub trait HookRunner {
    /// Runs the hook file `hook` with `args` and `stdin` in `git_dir`.
    ///
    /// # Errors
    ///
    /// Fails when the hook cannot be started.
    fn run(
        &self,
        hook: &Path,
        args: &[String],
        stdin: &[u8],
        git_dir: &Path,
        env: &[EnvVar],
    ) -> Result<HookOutput>;
}

/// Hook runner that starts hooks through the program launching service.
pub struct ProcessHooks<'a> {
    host: &'a dyn ProcessHost,
    cancel: &'a CancelToken,
}

impl<'a> ProcessHooks<'a> {
    /// Runner using `host`; `cancel` stops a running hook.
    #[must_use]
    pub fn new(host: &'a dyn ProcessHost, cancel: &'a CancelToken) -> Self {
        Self { host, cancel }
    }
}

impl std::fmt::Debug for ProcessHooks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHooks").finish_non_exhaustive()
    }
}

impl HookRunner for ProcessHooks<'_> {
    fn run(
        &self,
        hook: &Path,
        args: &[String],
        stdin: &[u8],
        git_dir: &Path,
        env: &[EnvVar],
    ) -> Result<HookOutput> {
        let mut spec = ProgramSpec::new(hook.to_string_lossy(), args.to_vec(), git_dir);
        spec.stdin = ProgramInput::Bytes(stdin.to_vec());
        spec.env = env.to_vec();
        let outcome = self.host.run_program(&spec, self.cancel)?;
        let mut output = outcome.stdout;
        output.extend_from_slice(&outcome.stderr);
        Ok(HookOutput {
            success: outcome.exit_code == Some(0),
            output,
        })
    }
}

/// Destination of the reference changes accepted by the service.
pub trait UpdateSink {
    /// Makes the changes of `plan` durable; `accepted` are the commands the
    /// plan implements.
    ///
    /// # Errors
    ///
    /// Returns the journal error; the commands are then reported as failed.
    fn apply(&mut self, plan: &FilePlan, accepted: &[RefCommand]) -> Result<()>;

    /// Records that objects were added to the object store at `git_dir`.
    fn objects_written(&mut self, git_dir: &Path);
}

/// Sink that applies the changes in a journal transaction.
pub struct TxSink<'t> {
    tx: &'t mut dyn MutationTx,
}

impl<'t> TxSink<'t> {
    /// Sink writing through `tx`.
    pub fn new(tx: &'t mut dyn MutationTx) -> Self {
        Self { tx }
    }
}

impl std::fmt::Debug for TxSink<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TxSink").finish_non_exhaustive()
    }
}

impl UpdateSink for TxSink<'_> {
    fn apply(&mut self, plan: &FilePlan, _accepted: &[RefCommand]) -> Result<()> {
        preserve_and_apply(self.tx, &[plan])
    }

    fn objects_written(&mut self, git_dir: &Path) {
        if let Ok(repo) = AbsPath::new(git_dir) {
            self.tx.record_exempt(ExemptWrite::ObjectStore { repo });
        }
    }
}

/// Outcome of one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// The command.
    pub command: RefCommand,
    /// Reason of the rejection; `None` when the reference was updated.
    pub error: Option<String>,
}

/// Outcome of one request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReceiveSummary {
    /// Commands with their outcome, in request order.
    pub commands: Vec<CommandResult>,
    /// Error of the unpack step.
    pub unpack_error: Option<String>,
    /// Objects received in the pack.
    pub objects_received: u64,
    /// A hook of the target repository exists but was not run.
    pub hooks_skipped: bool,
    /// Messages sent to the client, one entry per line.
    pub messages: Vec<String>,
    /// Error of the journal when the accepted changes could not be applied.
    pub update_error: Option<Error>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DenyCurrent {
    Refuse,
    Warn,
    Ignore,
    UpdateInstead,
}

#[derive(Debug)]
struct Policy {
    deny_current: DenyCurrent,
    deny_delete_current: bool,
    deny_non_fast_forwards: bool,
    deny_deletes: bool,
    hidden: Vec<String>,
    hooks_dir: PathBuf,
}

/// Requested capabilities of one request.
#[derive(Debug, Default)]
struct Requested {
    words: HashSet<String>,
    push_options: Vec<String>,
}

impl Requested {
    fn has(&self, word: &str) -> bool {
        self.words.contains(word)
    }
}

/// The receive-pack service of one repository.
pub struct ReceivePack {
    repo: gix::Repository,
    git_dir: PathBuf,
    options: ReceiveOptions,
    policy: Policy,
}

impl std::fmt::Debug for ReceivePack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceivePack")
            .field("git_dir", &self.git_dir)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl ReceivePack {
    /// Opens the repository at `path` (a working tree or a repository
    /// directory) with its own configuration only.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_NOT_REPOSITORY` when `path` is not a repository.
    pub fn open(path: &Path, options: ReceiveOptions) -> Result<Self> {
        let repo = gix::open_opts(path, gix::open::Options::isolated()).map_err(|error| {
            Error::new(
                ErrorCode::GitNotRepository,
                format!(
                    "`{}` is not a git repository: {error}; check the remote URL",
                    path.display()
                ),
            )
        })?;
        let git_dir = repo.git_dir().to_path_buf();
        let policy = read_policy(&repo, &git_dir);
        Ok(Self {
            repo,
            git_dir,
            options,
            policy,
        })
    }

    /// Repository directory of the target.
    #[must_use]
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Writes the reference advertisement.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the references cannot be read or the output
    /// cannot be written.
    pub fn advertise(&self, out: &mut dyn Write) -> Result<()> {
        let refs = self.visible_refs()?;
        let capabilities = self.capabilities();
        let write = |out: &mut dyn Write| -> io::Result<()> {
            if refs.is_empty() {
                let zero = ObjectId::null(self.repo.object_hash());
                let line = format!("{zero} capabilities^{{}}\0{capabilities}");
                pkt::write_text(out, line.as_bytes())?;
            }
            for (position, (name, id)) in refs.iter().enumerate() {
                let mut line = format!("{id} ").into_bytes();
                line.extend_from_slice(name);
                if position == 0 {
                    line.push(0);
                    line.extend_from_slice(capabilities.as_bytes());
                }
                pkt::write_text(out, &line)?;
            }
            pkt::write_flush(out)
        };
        write(out).map_err(|error| io_failure("writing the advertisement failed", &error))
    }

    /// Reads one request from `input`, performs it and writes the status
    /// report to `out`.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_PROTOCOL` when the request is malformed and with
    /// `E_IO` when the output cannot be written. Rejected commands and a
    /// failed journal update are reported in the summary and to the client.
    pub fn serve(
        &mut self,
        input: &mut dyn Read,
        out: &mut dyn Write,
        hooks: Option<&dyn HookRunner>,
        sink: &mut dyn UpdateSink,
    ) -> Result<ReceiveSummary> {
        let Some((commands, requested)) = read_request(input)? else {
            return Ok(ReceiveSummary::default());
        };
        let mut summary = ReceiveSummary::default();
        let mut errors: Vec<Option<String>> = vec![None; commands.len()];
        let mut keep = None;
        if commands.iter().any(|command| !command.is_delete()) {
            match self.unpack(input) {
                Ok((objects, keep_path)) => {
                    summary.objects_received = objects;
                    keep = keep_path;
                    if objects > 0 {
                        sink.objects_written(&self.git_dir);
                    }
                }
                Err(message) => {
                    summary.unpack_error = Some(message);
                    errors.fill(Some("unpacker error".to_owned()));
                }
            }
        }
        if summary.unpack_error.is_none() {
            self.check_commands(&commands, &mut errors);
            self.run_hooks_and_policies(&commands, &requested, &mut errors, hooks, &mut summary)?;
            if requested.has("atomic") && errors.iter().any(Option::is_some) {
                for error in &mut errors {
                    error.get_or_insert_with(|| "atomic push failure".to_owned());
                }
            }
            self.update_refs(&commands, &mut errors, sink, &mut summary);
            self.post_receive(&commands, &errors, &requested, hooks, &mut summary)?;
        }
        if let Some(path) = keep {
            let _ = std::fs::remove_file(path);
        }
        summary.commands = commands
            .into_iter()
            .zip(errors)
            .map(|(command, error)| CommandResult { command, error })
            .collect();
        write_report(out, &summary, &requested)
            .map_err(|error| io_failure("writing the status report failed", &error))?;
        Ok(summary)
    }

    fn capabilities(&self) -> String {
        let mut words = vec!["report-status"];
        if self.options.report_status_v2 {
            words.push("report-status-v2");
        }
        words.extend([
            "delete-refs",
            "side-band-64k",
            "quiet",
            "atomic",
            "ofs-delta",
            "push-options",
        ]);
        let format = match self.repo.object_hash() {
            gix::hash::Kind::Sha256 => "sha256",
            _ => "sha1",
        };
        format!(
            "{} object-format={format} agent={}",
            words.join(" "),
            self.options.agent
        )
    }

    fn visible_refs(&self) -> Result<Vec<(BString, ObjectId)>> {
        let platform = self
            .repo
            .references()
            .map_err(|error| io_failure("reading references failed", &error))?;
        let all = platform
            .prefixed(b"refs/".as_bstr())
            .map_err(|error| io_failure("reading references failed", &error))?;
        let mut refs = Vec::new();
        for reference in all {
            let mut reference =
                reference.map_err(|error| io_failure("reading references failed", &error))?;
            let name = BString::from(reference.name().as_bstr());
            if self.is_hidden(&name) {
                continue;
            }
            if let Ok(id) = reference.peel_to_id() {
                if let Some(target) = reference.target().try_id() {
                    refs.push((name, target.to_owned()));
                } else {
                    refs.push((name, id.detach()));
                }
            }
        }
        refs.sort();
        Ok(refs)
    }

    fn is_hidden(&self, name: &[u8]) -> bool {
        self.policy.hidden.iter().any(|prefix| {
            let prefix = prefix.trim_end_matches('/');
            name == prefix.as_bytes()
                || (name.starts_with(prefix.as_bytes()) && name.get(prefix.len()) == Some(&b'/'))
        })
    }

    fn unpack(&self, input: &mut dyn Read) -> std::result::Result<(u64, Option<PathBuf>), String> {
        let mut header = [0_u8; PACK_HEADER];
        input
            .read_exact(&mut header)
            .map_err(|error| format!("reading the pack header failed: {error}"))?;
        if &header[..4] != b"PACK" {
            return Err("the pack signature is invalid".to_owned());
        }
        let count = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
        if count == 0 {
            let mut trailer = vec![0_u8; PACK_TRAILER];
            input
                .read_exact(&mut trailer)
                .map_err(|error| format!("reading the pack trailer failed: {error}"))?;
            return Ok((0, None));
        }
        let directory = self.git_dir.join("objects").join("pack");
        let mut reader = BufReader::new(io::Cursor::new(header.to_vec()).chain(input));
        let outcome = gix_pack::Bundle::write_to_directory(
            &mut reader,
            Some(&directory),
            &mut gix::progress::Discard,
            &AtomicBool::new(false),
            Some(self.repo.objects.clone()),
            self.repo.object_hash(),
            gix_pack::bundle::write::Options::default(),
        )
        .map_err(|error| format!("index-pack failed: {error}"))?;
        Ok((u64::from(outcome.index.num_objects), outcome.keep_path))
    }

    fn check_commands(&self, commands: &[RefCommand], errors: &mut [Option<String>]) {
        let existing: Vec<ObjectId> = self
            .visible_refs()
            .map(|refs| refs.into_iter().map(|(_, id)| id).collect())
            .unwrap_or_default();
        for (command, error) in commands.iter().zip(errors.iter_mut()) {
            if error.is_some() {
                continue;
            }
            if validate_name(&command.name).is_err() || command.name == "HEAD" {
                *error = Some("funny refname".to_owned());
            } else if self.is_hidden(&command.name) {
                *error = Some("deny updating a hidden ref".to_owned());
            } else if !command.is_delete() {
                let complete = objects_to_send(
                    &self.repo,
                    &[command.new],
                    &existing,
                    &AtomicBool::new(false),
                );
                if complete.is_err_and(|error| error.problem.as_deref() == Some(MISSING_OBJECT)) {
                    *error = Some("missing necessary objects".to_owned());
                }
            }
        }
    }

    fn run_hooks_and_policies(
        &self,
        commands: &[RefCommand],
        requested: &Requested,
        errors: &mut [Option<String>],
        hooks: Option<&dyn HookRunner>,
        summary: &mut ReceiveSummary,
    ) -> Result<()> {
        let pending: Vec<&RefCommand> = commands
            .iter()
            .zip(errors.iter())
            .filter(|(_, error)| error.is_none())
            .map(|(command, _)| command)
            .collect();
        if !pending.is_empty() {
            let stdin = hook_input(pending.iter().copied());
            let declined = !self.run_hook("pre-receive", &[], &stdin, requested, hooks, summary)?;
            if declined {
                for error in errors.iter_mut().filter(|error| error.is_none()) {
                    *error = Some("pre-receive hook declined".to_owned());
                }
                return Ok(());
            }
        }
        for (command, error) in commands.iter().zip(errors.iter_mut()) {
            if error.is_some() {
                continue;
            }
            if let Some(reason) = self.policy_error(command, summary) {
                *error = Some(reason);
                continue;
            }
            let args = vec![
                command.name.to_str_lossy().into_owned(),
                command.old.to_string(),
                command.new.to_string(),
            ];
            if !self.run_hook("update", &args, &[], requested, hooks, summary)? {
                summary
                    .messages
                    .push(format!("error: hook declined to update {}", command.name));
                *error = Some("hook declined".to_owned());
            }
        }
        Ok(())
    }

    fn policy_error(&self, command: &RefCommand, summary: &mut ReceiveSummary) -> Option<String> {
        let current = match read_ref(&self.git_dir, &command.name) {
            Ok(value) => value,
            Err(_) => return Some("failed to update ref".to_owned()),
        };
        let expected = (!command.old.is_null()).then_some(command.old);
        let is_checked_out =
            !self.repo.is_bare() && self.head_branch().as_ref() == Some(&command.name);
        if command.is_delete() {
            if self.policy.deny_deletes && command.name.starts_with(b"refs/heads/") {
                summary
                    .messages
                    .push(format!("error: denying ref deletion for {}", command.name));
                return Some("deletion prohibited".to_owned());
            }
            if is_checked_out && self.policy.deny_delete_current {
                summary.messages.push(format!(
                    "error: refusing to delete the current branch: {}",
                    command.name
                ));
                return Some("deletion of the current branch prohibited".to_owned());
            }
        } else if is_checked_out {
            match self.policy.deny_current {
                DenyCurrent::Refuse | DenyCurrent::UpdateInstead => {
                    summary.messages.push(format!(
                        "error: refusing to update checked out branch: {}",
                        command.name
                    ));
                    return Some("branch is currently checked out".to_owned());
                }
                DenyCurrent::Warn => summary.messages.push(format!(
                    "warning: updating the current branch {}",
                    command.name
                )),
                DenyCurrent::Ignore => {}
            }
        }
        if current != expected {
            return Some("failed to update ref".to_owned());
        }
        let fast_forward_check = self.policy.deny_non_fast_forwards
            && !command.is_delete()
            && !command.is_create()
            && command.name.starts_with(b"refs/");
        if fast_forward_check && !self.is_ancestor(command.old, command.new) {
            summary.messages.push(format!(
                "error: denying non-fast-forward {} (you should pull first)",
                command.name
            ));
            return Some("non-fast-forward".to_owned());
        }
        None
    }

    fn is_ancestor(&self, old: ObjectId, new: ObjectId) -> bool {
        self.repo
            .merge_base(old, new)
            .is_ok_and(|base| base.is_some_and(|base| base.detach() == old))
    }

    fn head_branch(&self) -> Option<BString> {
        let bytes = std::fs::read(self.git_dir.join("HEAD")).ok()?;
        bytes
            .trim()
            .strip_prefix(b"ref: ")
            .map(|name| name.trim().into())
    }

    fn update_refs(
        &self,
        commands: &[RefCommand],
        errors: &mut [Option<String>],
        sink: &mut dyn UpdateSink,
        summary: &mut ReceiveSummary,
    ) {
        let accepted: Vec<(usize, &RefCommand)> = commands
            .iter()
            .enumerate()
            .filter(|(position, _)| errors[*position].is_none())
            .collect();
        if accepted.is_empty() {
            return;
        }
        let edits: Vec<RefEdit> = accepted
            .iter()
            .map(|(_, command)| RefEdit {
                name: command.name.clone(),
                old: (!command.old.is_null()).then_some(command.old),
                new: (!command.is_delete()).then_some(command.new),
                message: REFLOG_MESSAGE.to_owned(),
            })
            .collect();
        let mut plan = FilePlan::new();
        let policy = LogPolicy::of(&self.repo);
        let result = plan_ref_edits(
            &self.git_dir,
            &edits,
            &self.options.identity,
            policy,
            &mut plan,
        )
        .and_then(|()| {
            let commands: Vec<RefCommand> = accepted
                .iter()
                .map(|(_, command)| (*command).clone())
                .collect();
            sink.apply(&plan, &commands)
        });
        if let Err(error) = result {
            summary.messages.push(format!("error: {}", error.message));
            for (position, _) in &accepted {
                errors[*position] = Some("failed to update ref".to_owned());
            }
            summary.update_error = Some(error);
        }
    }

    fn post_receive(
        &self,
        commands: &[RefCommand],
        errors: &[Option<String>],
        requested: &Requested,
        hooks: Option<&dyn HookRunner>,
        summary: &mut ReceiveSummary,
    ) -> Result<()> {
        let updated: Vec<&RefCommand> = commands
            .iter()
            .zip(errors)
            .filter(|(_, error)| error.is_none())
            .map(|(command, _)| command)
            .collect();
        if updated.is_empty() {
            return Ok(());
        }
        let stdin = hook_input(updated.into_iter());
        self.run_hook("post-receive", &[], &stdin, requested, hooks, summary)?;
        Ok(())
    }

    fn run_hook(
        &self,
        name: &str,
        args: &[String],
        stdin: &[u8],
        requested: &Requested,
        hooks: Option<&dyn HookRunner>,
        summary: &mut ReceiveSummary,
    ) -> Result<bool> {
        let path = self.policy.hooks_dir.join(name);
        if !is_executable(&path) {
            return Ok(true);
        }
        let runner = match hooks {
            Some(runner) if self.options.run_hooks => runner,
            _ => {
                summary.hooks_skipped = true;
                return Ok(true);
            }
        };
        let mut env = vec![EnvVar {
            name: "GIT_DIR".to_owned(),
            value: Some(".".to_owned()),
        }];
        if requested.has("push-options") {
            env.push(EnvVar {
                name: "GIT_PUSH_OPTION_COUNT".to_owned(),
                value: Some(requested.push_options.len().to_string()),
            });
            for (index, option) in requested.push_options.iter().enumerate() {
                env.push(EnvVar {
                    name: format!("GIT_PUSH_OPTION_{index}"),
                    value: Some(option.clone()),
                });
            }
        }
        let output = runner.run(&path, args, stdin, &self.git_dir, &env)?;
        summary.messages.extend(
            output
                .output
                .lines()
                .map(|line| line.to_str_lossy().trim_end().to_owned()),
        );
        Ok(output.success)
    }
}

fn read_policy(repo: &gix::Repository, git_dir: &Path) -> Policy {
    let config = repo.config_snapshot();
    let deny_current = match config
        .string("receive.denyCurrentBranch")
        .map(|value| value.to_str_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("warn") => DenyCurrent::Warn,
        Some("ignore" | "false" | "no" | "off" | "0") => DenyCurrent::Ignore,
        Some("updateinstead") => DenyCurrent::UpdateInstead,
        _ => DenyCurrent::Refuse,
    };
    let deny_delete_current = !matches!(
        config
            .string("receive.denyDeleteCurrent")
            .map(|value| value.to_str_lossy().to_ascii_lowercase())
            .as_deref(),
        Some("warn" | "ignore" | "false" | "no" | "off" | "0")
    );
    let mut hidden = Vec::new();
    for key in ["transfer.hideRefs", "receive.hideRefs"] {
        if let Some(values) = config.strings(key) {
            hidden.extend(values.iter().map(|value| value.to_str_lossy().into_owned()));
        }
    }
    let hooks_dir = config
        .trusted_path("core.hooksPath")
        .ok()
        .flatten()
        .map_or_else(
            || git_dir.join("hooks"),
            |path| {
                let base = repo.workdir().unwrap_or(git_dir);
                base.join(path)
            },
        );
    Policy {
        deny_current,
        deny_delete_current,
        deny_non_fast_forwards: config
            .boolean("receive.denyNonFastForwards")
            .unwrap_or(false),
        deny_deletes: config.boolean("receive.denyDeletes").unwrap_or(false),
        hidden,
        hooks_dir,
    }
}

fn read_request(input: &mut dyn Read) -> Result<Option<(Vec<RefCommand>, Requested)>> {
    let mut commands = Vec::new();
    let mut requested = Requested::default();
    loop {
        match pkt::read_packet(input).map_err(|error| malformed(&error.to_string()))? {
            None if commands.is_empty() => return Ok(None),
            None => return Err(malformed("the request ended before its flush packet")),
            Some(Packet::Flush) => break,
            Some(Packet::Delimiter) => return Err(malformed("unexpected delimiter packet")),
            Some(Packet::Data(data)) => {
                let line = pkt::trim_newline(&data);
                let (command, words) = match line.split_once_str(b"\0") {
                    Some((command, words)) => (command, Some(words)),
                    None => (line, None),
                };
                if let Some(words) = words {
                    requested.words = words
                        .split_str(b" ")
                        .filter(|word| !word.is_empty())
                        .map(|word| word.to_str_lossy().into_owned())
                        .collect();
                }
                commands.push(parse_command(command)?);
            }
        }
    }
    if commands.is_empty() {
        return Ok(None);
    }
    if requested.has("push-options") {
        loop {
            match pkt::read_packet(input).map_err(|error| malformed(&error.to_string()))? {
                Some(Packet::Data(data)) => requested
                    .push_options
                    .push(pkt::trim_newline(&data).to_str_lossy().into_owned()),
                Some(Packet::Flush) | None => break,
                Some(Packet::Delimiter) => return Err(malformed("unexpected delimiter packet")),
            }
        }
    }
    Ok(Some((commands, requested)))
}

fn parse_command(line: &[u8]) -> Result<RefCommand> {
    let mut parts = line.splitn_str(3, b" ");
    let (Some(old), Some(new), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(malformed(&format!(
            "malformed command `{}`",
            line.as_bstr()
        )));
    };
    let parse = |hex: &[u8]| {
        ObjectId::from_hex(hex)
            .map_err(|_| malformed(&format!("malformed command `{}`", line.as_bstr())))
    };
    Ok(RefCommand {
        old: parse(old)?,
        new: parse(new)?,
        name: name.into(),
    })
}

fn hook_input<'a>(commands: impl Iterator<Item = &'a RefCommand>) -> Vec<u8> {
    let mut stdin = Vec::new();
    for command in commands {
        stdin.extend_from_slice(format!("{} {} ", command.old, command.new).as_bytes());
        stdin.extend_from_slice(&command.name);
        stdin.push(b'\n');
    }
    stdin
}

fn write_report(
    out: &mut dyn Write,
    summary: &ReceiveSummary,
    requested: &Requested,
) -> io::Result<()> {
    let report_status = requested.has("report-status") || requested.has("report-status-v2");
    let side_band = requested.has("side-band-64k") || requested.has("side-band");
    let mut report = Vec::new();
    if report_status {
        let unpack = summary
            .unpack_error
            .as_deref()
            .map_or_else(|| "unpack ok".to_owned(), |error| format!("unpack {error}"));
        pkt::write_text(&mut report, unpack.as_bytes())?;
        for result in &summary.commands {
            let mut line = match &result.error {
                None => b"ok ".to_vec(),
                Some(_) => b"ng ".to_vec(),
            };
            line.extend_from_slice(&result.command.name);
            if let Some(error) = &result.error {
                line.push(b' ');
                line.extend_from_slice(error.as_bytes());
            }
            pkt::write_text(&mut report, &line)?;
        }
        pkt::write_flush(&mut report)?;
    }
    if !side_band {
        out.write_all(&report)?;
        return out.flush();
    }
    if !requested.has("quiet") || !summary.messages.is_empty() {
        for message in &summary.messages {
            pkt::write_band(out, pkt::BAND_PROGRESS, format!("{message}\n").as_bytes())?;
        }
    }
    if !report.is_empty() {
        pkt::write_band(out, pkt::BAND_DATA, &report)?;
    }
    pkt::write_flush(out)?;
    out.flush()
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

fn malformed(detail: &str) -> Error {
    Error::new(
        ErrorCode::GitProtocol,
        format!("the push request is malformed: {detail}"),
    )
    .with_problem("malformed_request")
}

fn io_failure(context: &str, error: &dyn std::fmt::Display) -> Error {
    Error::new(ErrorCode::Io, format!("{context}: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Advertising,
    Requesting,
    Responding,
}

struct Session<'a> {
    service: ReceivePack,
    hooks: Option<&'a dyn HookRunner>,
    sink: &'a mut dyn UpdateSink,
    request: Vec<u8>,
    response: Vec<u8>,
    position: usize,
    phase: Phase,
    result: Option<Result<ReceiveSummary>>,
}

impl Session<'_> {
    fn respond(&mut self) {
        let request = std::mem::take(&mut self.request);
        let mut response = Vec::new();
        let result = self.service.serve(
            &mut request.as_slice(),
            &mut response,
            self.hooks,
            self.sink,
        );
        self.response = response;
        self.position = 0;
        self.phase = Phase::Responding;
        self.result = Some(result);
    }
}

/// Handle of an in-process push, used to collect the outcome after the
/// client finished.
pub struct LocalSession<'a> {
    state: Rc<RefCell<Session<'a>>>,
}

impl std::fmt::Debug for LocalSession<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSession").finish_non_exhaustive()
    }
}

impl LocalSession<'_> {
    /// Outcome of the request; `None` when the client sent no commands.
    ///
    /// # Errors
    ///
    /// Returns the error of [`ReceivePack::serve`].
    pub fn finish(&self) -> Result<Option<ReceiveSummary>> {
        let mut state = self.state.borrow_mut();
        if state.phase == Phase::Requesting && !state.request.is_empty() {
            state.respond();
        }
        state.result.take().transpose()
    }
}

/// Reading end of the in-process connection.
pub struct SessionReader<'a> {
    state: Rc<RefCell<Session<'a>>>,
}

/// Writing end of the in-process connection.
pub struct SessionWriter<'a> {
    state: Rc<RefCell<Session<'a>>>,
}

impl std::fmt::Debug for SessionReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionReader").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SessionWriter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionWriter").finish_non_exhaustive()
    }
}

impl Read for SessionReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        if state.position >= state.response.len() {
            match state.phase {
                Phase::Advertising => state.phase = Phase::Requesting,
                Phase::Requesting if !state.request.is_empty() => state.respond(),
                _ => {}
            }
        }
        let available = &state.response[state.position.min(state.response.len())..];
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        state.position += count;
        Ok(count)
    }
}

impl Write for SessionWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        if state.phase == Phase::Advertising {
            state.phase = Phase::Requesting;
        }
        state.request.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Client transport connected to an in-process receive-pack service.
pub type LocalTransport<'a> = Connection<SessionReader<'a>, SessionWriter<'a>>;

/// Connects a client transport to `service` without threads.
///
/// The request written by the client is buffered and served when the
/// client starts reading the response. `url` is reported by the transport.
///
/// # Errors
///
/// Fails with `E_IO` when the advertisement cannot be produced.
///
/// # Examples
///
/// ```no_run
/// use nurt_git_transport::local::receive::{ReceiveOptions, ReceivePack, TxSink, connect};
/// use nurt_git_transport::send_pack::{Identity, discover};
///
/// # fn run(tx: &mut dyn nurt_contracts::journal::MutationTx) -> nurt_contracts::error::Result<()> {
/// let identity = Identity { name: "Ada".into(), email: "ada@example.com".into(), seconds: 0, offset: 0 };
/// let options = ReceiveOptions { agent: "nurt/1".into(), report_status_v2: true, run_hooks: false, identity };
/// let service = ReceivePack::open(std::path::Path::new("/srv/repo.git"), options)?;
/// let mut sink = TxSink::new(tx);
/// let (mut transport, session) = connect(service, "file:///srv/repo.git", None, &mut sink)?;
/// let advertisement = discover(&mut transport)?;
/// assert!(advertisement.supports("report-status"));
/// assert!(session.finish()?.is_none());
/// # Ok(())
/// # }
/// ```
pub fn connect<'a>(
    service: ReceivePack,
    url: &str,
    hooks: Option<&'a dyn HookRunner>,
    sink: &'a mut dyn UpdateSink,
) -> Result<(LocalTransport<'a>, LocalSession<'a>)> {
    let mut advertisement = Vec::new();
    service.advertise(&mut advertisement)?;
    let path = service.git_dir().to_string_lossy().into_owned();
    let state = Rc::new(RefCell::new(Session {
        service,
        hooks,
        sink,
        request: Vec::new(),
        response: advertisement,
        position: 0,
        phase: Phase::Advertising,
        result: None,
    }));
    let reader = SessionReader {
        state: Rc::clone(&state),
    };
    let writer = SessionWriter {
        state: Rc::clone(&state),
    };
    let transport = Connection::new(
        reader,
        writer,
        gix_transport::Protocol::V0,
        path,
        None::<(String, Option<u16>)>,
        ConnectMode::Process,
        false,
    )
    .custom_url(Some(url.into()));
    Ok((transport, LocalSession { state }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::send_pack::testing::{Fixture, RecordingTx, ScriptHost};
    use crate::send_pack::{RefStatus, SendOptions, discover, send};

    fn identity() -> Identity {
        Identity {
            name: "Receiver".into(),
            email: "receiver@example.com".into(),
            seconds: 1_700_000_100,
            offset: 0,
        }
    }

    fn options() -> ReceiveOptions {
        ReceiveOptions {
            agent: "nurt/test".to_owned(),
            report_status_v2: true,
            run_hooks: true,
            identity: identity(),
        }
    }

    struct Outcome {
        send: crate::send_pack::SendOutcome,
        summary: Option<ReceiveSummary>,
        tx: RecordingTx,
    }

    fn push(
        local: &Fixture,
        remote: &Fixture,
        commands: &[RefCommand],
        send_options: &SendOptions,
    ) -> Outcome {
        let host = ScriptHost::default();
        let cancel = CancelToken::new();
        let hooks = ProcessHooks::new(&host, &cancel);
        let mut tx = RecordingTx::new();
        let (send_outcome, summary) = {
            let mut sink = TxSink::new(&mut tx);
            let service = ReceivePack::open(remote.root(), options()).unwrap();
            let (mut transport, session) =
                connect(service, "file:///remote", Some(&hooks), &mut sink).unwrap();
            let advertisement = discover(&mut transport).unwrap();
            let outcome = send(
                &mut transport,
                &local.repo(),
                &advertisement,
                commands,
                &[],
                send_options,
                &AtomicBool::new(false),
            )
            .unwrap();
            drop(transport);
            (outcome, session.finish().unwrap())
        };
        Outcome {
            send: send_outcome,
            summary,
            tx,
        }
    }

    fn command(old: Option<ObjectId>, new: Option<ObjectId>, name: &str) -> RefCommand {
        let zero = ObjectId::null(gix::hash::Kind::Sha1);
        RefCommand {
            old: old.unwrap_or(zero),
            new: new.unwrap_or(zero),
            name: name.into(),
        }
    }

    fn statuses(outcome: &Outcome) -> Vec<(String, RefStatus)> {
        outcome
            .send
            .report
            .as_ref()
            .unwrap()
            .refs
            .iter()
            .map(|report| (report.name.to_string(), report.status.clone()))
            .collect()
    }

    #[test]
    fn creates_a_branch_in_an_empty_bare_repository_and_undoes_it() {
        let local = Fixture::new();
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let remote = Fixture::bare();
        let mut outcome = push(
            &local,
            &remote,
            &[command(None, Some(tip), "refs/heads/main")],
            &SendOptions::default(),
        );
        assert_eq!(
            statuses(&outcome),
            [("refs/heads/main".to_owned(), RefStatus::Ok)]
        );
        assert_eq!(remote.ref_value("refs/heads/main"), Some(tip));
        assert!(remote.repo().has_object(tip));
        let summary = outcome.summary.unwrap();
        assert_eq!(summary.objects_received, 3);
        assert!(
            !remote.git_dir().join("logs/refs/heads/main").exists(),
            "bare: no reflog"
        );
        assert_eq!(outcome.tx.exempt.len(), 1);
        assert!(outcome.send.pack.is_some());
        outcome.tx.undo();
        assert_eq!(remote.ref_value("refs/heads/main"), None);
    }

    #[test]
    fn fast_forwards_and_deletes_with_reflog_in_non_bare_target() {
        let local = Fixture::new();
        let base = local.commit("main", &[("a.txt", "one\n")], &[]);
        let next = local.commit("main", &[("a.txt", "two\n")], &[base]);
        let remote = Fixture::new();
        remote.config("[receive]\n\tdenyCurrentBranch = ignore\n");
        let first = push(
            &local,
            &remote,
            &[
                command(None, Some(base), "refs/heads/main"),
                command(None, Some(base), "refs/heads/old"),
            ],
            &SendOptions::default(),
        );
        assert_eq!(first.summary.unwrap().commands.len(), 2);
        let second = push(
            &local,
            &remote,
            &[
                command(Some(base), Some(next), "refs/heads/main"),
                command(Some(base), None, "refs/heads/old"),
            ],
            &SendOptions::default(),
        );
        assert!(
            statuses(&second)
                .iter()
                .all(|(_, status)| *status == RefStatus::Ok)
        );
        assert_eq!(remote.ref_value("refs/heads/main"), Some(next));
        assert_eq!(remote.ref_value("refs/heads/old"), None);
        let log = std::fs::read_to_string(remote.git_dir().join("logs/refs/heads/main")).unwrap();
        assert!(log.lines().last().unwrap().ends_with("\tpush"));
        assert!(remote.git_dir().join("logs/HEAD").is_file());
    }

    #[test]
    fn refuses_the_checked_out_branch_by_default() {
        let local = Fixture::new();
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let remote = Fixture::new();
        let outcome = push(
            &local,
            &remote,
            &[
                command(None, Some(tip), "refs/heads/main"),
                command(None, Some(tip), "refs/heads/side"),
            ],
            &SendOptions::default(),
        );
        assert_eq!(
            statuses(&outcome),
            [
                (
                    "refs/heads/main".to_owned(),
                    RefStatus::Rejected("branch is currently checked out".to_owned())
                ),
                ("refs/heads/side".to_owned(), RefStatus::Ok),
            ]
        );
        assert!(
            outcome
                .send
                .remote_messages
                .iter()
                .any(|line| line.contains("refusing to update checked out branch"))
        );
    }

    #[test]
    fn applies_receive_policies_and_atomic_failure() {
        let local = Fixture::new();
        let base = local.commit("main", &[("a.txt", "one\n")], &[]);
        let other = local.commit("other", &[("a.txt", "other\n")], &[]);
        let remote = Fixture::bare();
        push(
            &local,
            &remote,
            &[
                command(None, Some(base), "refs/heads/main"),
                command(None, Some(base), "refs/heads/gone"),
            ],
            &SendOptions::default(),
        );
        remote.config("[receive]\n\tdenyNonFastForwards = true\n\tdenyDeletes = true\n");
        let outcome = push(
            &local,
            &remote,
            &[
                command(Some(base), Some(other), "refs/heads/main"),
                command(Some(base), None, "refs/heads/gone"),
            ],
            &SendOptions::default(),
        );
        assert_eq!(
            statuses(&outcome),
            [
                (
                    "refs/heads/main".to_owned(),
                    RefStatus::Rejected("non-fast-forward".to_owned())
                ),
                (
                    "refs/heads/gone".to_owned(),
                    RefStatus::Rejected("deletion prohibited".to_owned())
                ),
            ]
        );
        let atomic = SendOptions {
            atomic: true,
            ..SendOptions::default()
        };
        let outcome = push(
            &local,
            &remote,
            &[
                command(Some(base), Some(other), "refs/heads/main"),
                command(None, Some(other), "refs/heads/new"),
            ],
            &atomic,
        );
        assert_eq!(
            statuses(&outcome)[1].1,
            RefStatus::Rejected("atomic push failure".to_owned())
        );
        assert_eq!(remote.ref_value("refs/heads/new"), None);
        let stale = push(
            &local,
            &remote,
            &[command(Some(other), Some(base), "refs/heads/main")],
            &SendOptions::default(),
        );
        assert_eq!(
            statuses(&stale)[0].1,
            RefStatus::Rejected("failed to update ref".to_owned())
        );
    }

    #[cfg(unix)]
    #[test]
    fn runs_receive_hooks_through_the_process_host() {
        let local = Fixture::new();
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let remote = Fixture::bare();
        remote.hook("pre-receive", "#!/bin/sh\necho no way >&2\nexit 1\n");
        let outcome = push(
            &local,
            &remote,
            &[command(None, Some(tip), "refs/heads/main")],
            &SendOptions::default(),
        );
        assert_eq!(
            statuses(&outcome)[0].1,
            RefStatus::Rejected("pre-receive hook declined".to_owned())
        );
        assert_eq!(outcome.send.remote_messages, ["no way"]);
        std::fs::remove_file(remote.git_dir().join("hooks/pre-receive")).unwrap();
        remote.hook(
            "update",
            "#!/bin/sh\n[ \"$1\" = refs/heads/b2 ] && exit 1\nexit 0\n",
        );
        remote.hook("post-receive", "#!/bin/sh\ncat > post-receive.out\n");
        let outcome = push(
            &local,
            &remote,
            &[
                command(None, Some(tip), "refs/heads/main"),
                command(None, Some(tip), "refs/heads/b2"),
            ],
            &SendOptions::default(),
        );
        assert_eq!(
            statuses(&outcome)[1].1,
            RefStatus::Rejected("hook declined".to_owned())
        );
        let post = std::fs::read_to_string(remote.git_dir().join("post-receive.out")).unwrap();
        assert!(post.ends_with(" refs/heads/main\n"));
    }

    #[test]
    fn skips_hooks_when_disabled_and_hides_refs() {
        let local = Fixture::new();
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let remote = Fixture::bare();
        remote.hook("pre-receive", "#!/bin/sh\nexit 1\n");
        remote.config("[receive]\n\thideRefs = refs/hidden\n");
        let mut tx = RecordingTx::new();
        let mut sink = TxSink::new(&mut tx);
        let mut service = ReceivePack::open(
            remote.root(),
            ReceiveOptions {
                run_hooks: false,
                ..options()
            },
        )
        .unwrap();
        let mut request = Vec::new();
        let commands = [
            command(None, Some(tip), "refs/heads/main"),
            command(None, Some(tip), "refs/hidden/x"),
            command(None, Some(tip), "HEAD"),
        ];
        let negotiated = crate::send_pack::Negotiated {
            words: vec!["report-status".to_owned()],
            report_status: true,
            ..Default::default()
        };
        crate::send_pack::write_commands(&mut request, &commands, &negotiated, &[]).unwrap();
        crate::send_pack::write_pack(
            &local.repo(),
            crate::send_pack::objects_to_send(&local.repo(), &[tip], &[], &AtomicBool::new(false))
                .unwrap(),
            &mut request,
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut response = Vec::new();
        let summary = service
            .serve(&mut request.as_slice(), &mut response, None, &mut sink)
            .unwrap();
        assert!(summary.hooks_skipped);
        let report = crate::send_pack::parse_report(&response).unwrap();
        assert_eq!(report.refs[0].status, RefStatus::Ok);
        assert_eq!(
            report.refs[1].status,
            RefStatus::Rejected("deny updating a hidden ref".to_owned())
        );
        assert_eq!(
            report.refs[2].status,
            RefStatus::Rejected("funny refname".to_owned())
        );
        let mut advertisement = Vec::new();
        service.advertise(&mut advertisement).unwrap();
        assert!(!advertisement.contains_str("refs/hidden"));
    }

    #[test]
    fn rejects_incomplete_packs_and_malformed_requests() {
        let local = Fixture::new();
        let tip = local.commit("main", &[("a.txt", "one\n")], &[]);
        let remote = Fixture::bare();
        let mut tx = RecordingTx::new();
        let mut sink = TxSink::new(&mut tx);
        let mut service = ReceivePack::open(remote.root(), options()).unwrap();
        let negotiated = crate::send_pack::Negotiated {
            words: vec!["report-status".to_owned()],
            report_status: true,
            ..Default::default()
        };
        let mut request = Vec::new();
        crate::send_pack::write_commands(
            &mut request,
            &[command(None, Some(tip), "refs/heads/main")],
            &negotiated,
            &[],
        )
        .unwrap();
        crate::send_pack::write_pack(
            &local.repo(),
            Vec::new(),
            &mut request,
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut response = Vec::new();
        service
            .serve(&mut request.as_slice(), &mut response, None, &mut sink)
            .unwrap();
        let report = crate::send_pack::parse_report(&response).unwrap();
        assert_eq!(
            report.refs[0].status,
            RefStatus::Rejected("missing necessary objects".to_owned())
        );

        let mut request = Vec::new();
        crate::send_pack::write_commands(
            &mut request,
            &[command(None, Some(tip), "refs/heads/main")],
            &negotiated,
            &[],
        )
        .unwrap();
        request.extend_from_slice(b"JUNKJUNKJUNKJUNK");
        let mut response = Vec::new();
        let summary = service
            .serve(&mut request.as_slice(), &mut response, None, &mut sink)
            .unwrap();
        assert!(summary.unpack_error.is_some());

        for bad in [&b"0010not a command"[..], b"0009abcde", b"0001"] {
            let error = service
                .serve(&mut &bad[..], &mut Vec::new(), None, &mut sink)
                .unwrap_err();
            assert_eq!(error.code(), ErrorCode::GitProtocol);
        }
        assert!(
            service
                .serve(&mut &b""[..], &mut Vec::new(), None, &mut sink)
                .unwrap()
                .commands
                .is_empty()
        );
        assert!(
            service
                .serve(&mut &b"0000"[..], &mut Vec::new(), None, &mut sink)
                .unwrap()
                .commands
                .is_empty()
        );
    }

    #[test]
    fn advertises_references_and_capabilities() {
        let remote = Fixture::bare();
        let service = ReceivePack::open(remote.root(), options()).unwrap();
        let mut empty = Vec::new();
        service.advertise(&mut empty).unwrap();
        assert!(empty.contains_str("capabilities^{}\0report-status report-status-v2"));
        let tip = remote.commit("main", &[("a.txt", "one\n")], &[]);
        remote.tag("v1", tip);
        let mut tx = RecordingTx::new();
        let mut sink = TxSink::new(&mut tx);
        let service = ReceivePack::open(
            remote.root(),
            ReceiveOptions {
                report_status_v2: false,
                ..options()
            },
        )
        .unwrap();
        let (mut transport, _session) =
            connect(service, "file:///remote", None, &mut sink).unwrap();
        let advertisement = discover(&mut transport).unwrap();
        assert_eq!(advertisement.refs().len(), 2);
        assert!(!advertisement.supports("report-status-v2"));
        assert_eq!(advertisement.capability("agent"), Some("nurt/test"));
        assert!(ReceivePack::open(Path::new("/nonexistent/repository"), options()).is_err());
        assert!(
            format!("{:?}", ReceivePack::open(remote.root(), options()).unwrap())
                .contains("ReceivePack")
        );
    }
}
