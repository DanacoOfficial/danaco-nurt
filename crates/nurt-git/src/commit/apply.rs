// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::commit::apply
// Responsibility:   Execution of a planned commit.
// Dependencies:     bstr, gix, gix_ref, nurt_contracts, nurt_git_transport
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Execution of a planned commit: the hooks, the final message, the commit
//! object and its signature, and the index, the message file, the merge
//! state and the references written in one transaction.
//!
//! Hooks see a copy of the index and of the message in a scratch directory
//! outside the repository (`GIT_INDEX_FILE` names the copy), so that every
//! change of the repository itself goes through the transaction. Changes a
//! hook makes to the copy of the index are taken over into the commit.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bstr::BString;
use gix::ObjectId;
use gix_ref::{FullName, Target};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::services::{EnvVar, ProcessHost, ProgramInput};
use nurt_contracts::stage::ExecCtx;
use nurt_git_transport::sign::{ProgramSigner, Signer, SigningConfig, sign_commit};

use super::access;
use super::prepare::Prepared;
use super::{Selection, render_call};
use crate::hooks::{self, Hook, HookDir, HookRequest};
use crate::message::{append_trailers, cleanup, reflog_subject, sign_off};
use crate::refs_tx::{PreviousValue, RefEdit, RefLayout, RefTx};
use crate::repo::GitRepo;
use crate::repo::index::{IndexFile, index_to_tree, locked_write, serialize};
use crate::repo::lock::{LockedWrite, LockedWrites, content_hash, record_object_writes};

/// Attempts to find an unused name for the scratch directory.
const SCRATCH_ATTEMPTS: u32 = 16;

/// Argument of `prepare-commit-msg` for a message given by the caller.
const MESSAGE_SOURCE: &str = "message";

/// Editor variable set for hooks, as `git` sets it when no editor runs.
const NO_EDITOR: &str = ":";

/// Result of a written commit.
#[derive(Debug, Clone)]
pub(crate) struct Written {
    /// Identifier of the commit.
    pub(crate) id: ObjectId,
    /// Tree of the commit.
    pub(crate) tree: ObjectId,
    /// Unusual effects.
    pub(crate) notes: Vec<NotableEntry>,
    /// Irreversible effects: hooks that ran.
    pub(crate) irreversible: Vec<IrreversibleEntry>,
}

/// Runs the hooks, writes the commit object and applies every file change
/// through `tx`.
pub(crate) fn apply(
    repo: &GitRepo,
    mut prepared: Prepared,
    tx: &mut dyn MutationTx,
) -> Result<Written> {
    let ctx = repo.ctx();
    record_object_writes(tx, repo);
    let mut irreversible = Vec::new();
    let mut notes = std::mem::take(&mut prepared.notes);
    let hooks = prepared.hooks.take();
    let scratch = match (&hooks, ctx.process()) {
        (Some(_), Some(_)) => Some(Scratch::create(ctx)?),
        _ => None,
    };
    let runner = match (&hooks, ctx.process(), &scratch) {
        (Some(dir), Some(host), Some(scratch)) => Some(HookRunner {
            dir,
            host,
            ctx,
            scratch,
            fix: render_call(
                &prepared.root,
                &prepared.request,
                &prepared.request.selection,
                &["hooks: false"],
            ),
            timeout: ctx.config().git.hooks.timeout.as_std(),
        }),
        _ => None,
    };
    if let Some(runner) = &runner {
        pre_commit(repo, runner, &mut prepared, &mut irreversible)?;
    }
    let draft = draft_message(&prepared);
    let edited = match &runner {
        Some(runner) => runner.message_hooks(&draft, &mut irreversible)?,
        None => draft,
    };
    let message = cleanup(&edited, prepared.cleanup, prepared.comment);
    if message.is_empty() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the commit message is empty after cleanup; give a message with text",
        )
        .with_problem("empty_message"));
    }
    if message.contains('\0') {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "the commit message contains a NUL byte, which git does not accept; remove it",
        )
        .with_problem("nul_in_message"));
    }
    let mut object = gix::objs::Commit {
        tree: prepared.tree,
        parents: prepared.parents.iter().copied().collect(),
        author: prepared.identities.author.clone(),
        committer: prepared.identities.committer.clone(),
        encoding: prepared.encoding.clone(),
        message: BString::from(message.as_str()),
        extra_headers: Vec::new(),
    };
    if prepared.sign {
        notes.extend(sign(repo, &mut object)?);
    }
    let id = repo
        .gix()
        .write_object(&object)
        .map_err(|error| access::lib_error("write the commit object", error))?
        .detach();
    write_targets(repo, &prepared, id, &message, tx)?;
    if let Some(runner) = &runner {
        runner.run(
            Hook::PostCommit,
            Vec::new(),
            &repo.index_path(),
            &mut irreversible,
        )?;
    }
    for entry in &irreversible {
        tx.mark_irreversible(entry.clone());
    }
    Ok(Written {
        id,
        tree: prepared.tree,
        notes,
        irreversible,
    })
}

/// Message before the hooks and the cleanup: the given message completed
/// with a line feed, the sign-off and the trailers, in the order of `git
/// commit -m … --signoff --trailer …`.
fn draft_message(prepared: &Prepared) -> String {
    let mut text = prepared.request.message.clone();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    if prepared.request.sign_off {
        let committer = &prepared.identities.committer;
        text = sign_off(&text, &format!("{} <{}>", committer.name, committer.email));
    }
    append_trailers(&text, &prepared.request.trailers)
}

/// Signs `object`; returns the notable entries of the signature.
fn sign(repo: &GitRepo, object: &mut gix::objs::Commit) -> Result<Vec<NotableEntry>> {
    let ctx = repo.ctx();
    let identity = format!("{} <{}>", object.committer.name, object.committer.email);
    let snapshot = repo.gix().config_snapshot();
    let mut config = SigningConfig::resolve(snapshot.plumbing(), ctx, Some(&identity))?;
    if let Some(workdir) = repo.workdir() {
        config = config.in_work_dir(workdir);
    }
    let signer: Arc<dyn Signer> = ctx
        .extensions()
        .get::<dyn Signer>()
        .unwrap_or_else(|| Arc::new(ProgramSigner::from_ctx(ctx)));
    let signature = sign_commit(object, signer.as_ref(), &config)?;
    let mut note = NotableEntry::new(Notable::Signed);
    note.detail = Some(format!("signed with key {}", signature.key));
    let mut notes = vec![note];
    notes.extend(config.ignored_programs_note());
    Ok(notes)
}

fn abs(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|_| {
        Error::new(
            ErrorCode::Io,
            format!("repository path is not absolute: {}", path.display()),
        )
    })
}

fn head_name() -> Result<FullName> {
    FullName::try_from("HEAD").map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!("invalid reference name HEAD: {error}"),
        )
    })
}

/// Writes the index, `COMMIT_EDITMSG`, removes the merge state and moves the
/// branch, all through `tx`.
fn write_targets(
    repo: &GitRepo,
    prepared: &Prepared,
    id: ObjectId,
    message: &str,
    tx: &mut dyn MutationTx,
) -> Result<()> {
    let mut writes = Vec::new();
    if let Some(index) = &prepared.index {
        let mut index = index.clone();
        writes.push(locked_write(&mut index)?);
    }
    writes.push(LockedWrite::new(
        repo.git_dir().join("COMMIT_EDITMSG"),
        message.as_bytes().to_vec(),
    ));
    let writes = LockedWrites::new(writes);
    let mut targets = writes.targets()?;
    for file in &prepared.merge {
        targets.push(TargetSpec {
            path: TargetPath::Fs(abs(&file.path)?),
            action: TargetAction::Delete,
            expect: Expect::Hash(content_hash(&file.content)),
            size_hint: None,
        });
    }
    let mut refs = reference_update(repo, prepared, id, message)?;
    let preserved = tx.preserve(&targets)?;
    let first = preserved
        .first()
        .map_or(TargetIndex(0), |target| target.index);
    refs.declare(tx)?;
    writes.apply(tx, first)?;
    for target in preserved.iter().skip(writes.writes().len()) {
        tx.apply(target.index, Mutation::Remove)?;
    }
    refs.apply(tx)?;
    Ok(())
}

fn reference_update(
    repo: &GitRepo,
    prepared: &Prepared,
    id: ObjectId,
    message: &str,
) -> Result<crate::refs_tx::PreparedRefTx> {
    let layout = RefLayout::from_repo(repo.gix())?;
    let mut refs = RefTx::new(layout, prepared.identities.committer.clone());
    let name = match &prepared.branch {
        Some(branch) => branch.clone(),
        None => head_name()?,
    };
    let expected = match prepared.head {
        Some(old) => PreviousValue::MustExistAndMatch(Target::Object(old)),
        None => PreviousValue::MustNotExist,
    };
    refs.push(RefEdit::set(
        name,
        Target::Object(id),
        expected,
        prepared.kind.message(reflog_subject(message)),
    ));
    refs.prepare()
}

/// Bytes of the index the `pre-commit` hook sees.
fn hook_index(repo: &GitRepo, prepared: &Prepared) -> Result<Vec<u8>> {
    match &prepared.state {
        Some(state) => serialize(&mut state.clone(), &[]),
        None => {
            let mut index = repo.read_index()?;
            serialize(&mut index.state, &index.resolve_undo)
        }
    }
}

/// Runs `pre-commit` and takes over the changes it made to the index.
fn pre_commit(
    repo: &GitRepo,
    runner: &HookRunner<'_>,
    prepared: &mut Prepared,
    irreversible: &mut Vec<IrreversibleEntry>,
) -> Result<()> {
    if runner.dir.find(Hook::PreCommit).is_none() {
        return Ok(());
    }
    let before = hook_index(repo, prepared)?;
    let path = runner.scratch.index();
    write_scratch(&path, &before)?;
    runner.run(Hook::PreCommit, Vec::new(), &path, irreversible)?;
    let after = std::fs::read(&path).map_err(|error| crate::repo::error::io(&path, &error))?;
    if after == before || prepared.state.is_none() {
        return Ok(());
    }
    let file = gix::index::File::at(&path, repo.gix().object_hash(), false, Default::default())
        .map_err(|error| {
            access::lib_error("read the index changed by the pre-commit hook", error)
        })?;
    let state: gix::index::State = file.into();
    prepared.tree = index_to_tree(repo, &state)?;
    if matches!(
        prepared.request.selection,
        Selection::Staged | Selection::All
    ) {
        let read = match prepared.index.take() {
            Some(index) => index,
            None => repo.read_index()?,
        };
        prepared.index = Some(IndexFile {
            state: state.clone(),
            resolve_undo: read.resolve_undo,
            path: repo.index_path(),
            expect: read.expect,
        });
    }
    prepared.state = Some(state);
    prepared.check_not_empty()
}

fn write_scratch(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|error| crate::repo::error::io(path, &error))
}

/// Hooks of one commit with the program service that starts them.
struct HookRunner<'a> {
    dir: &'a HookDir,
    host: &'a dyn ProcessHost,
    ctx: &'a ExecCtx,
    scratch: &'a Scratch,
    fix: String,
    timeout: Duration,
}

impl HookRunner<'_> {
    /// Runs `hook` when it exists; a failure of any hook but `post-commit`
    /// stops the commit.
    fn run(
        &self,
        hook: Hook,
        args: Vec<String>,
        index: &Path,
        irreversible: &mut Vec<IrreversibleEntry>,
    ) -> Result<()> {
        let mut request = HookRequest::new(hook, self.timeout);
        request.args = args;
        request.env = vec![
            EnvVar {
                name: "GIT_INDEX_FILE".to_owned(),
                value: Some(index.display().to_string()),
            },
            EnvVar {
                name: "GIT_EDITOR".to_owned(),
                value: Some(NO_EDITOR.to_owned()),
            },
        ];
        request.stdin = ProgramInput::Null;
        request.detect_changes = true;
        let Some(run) = hooks::run(self.host, self.dir, request, self.ctx.cancel())? else {
            return Ok(());
        };
        let changed = run.outcome.changed_paths.len();
        irreversible.push(IrreversibleEntry {
            reason: Irreversible::ExternalEffect,
            path: Some(run.program.display().to_string()),
            raw_path: None,
            count: (changed > 0).then(|| u64::try_from(changed).unwrap_or(u64::MAX)),
            detail: Some(format!("the {hook} hook ran as a user program")),
        });
        if hook == Hook::PostCommit {
            return Ok(());
        }
        run.into_result(Some(self.fix.clone())).map(|_| ())
    }

    /// Runs `prepare-commit-msg` and `commit-msg` on a copy of the message
    /// and returns the message as the hooks left it.
    fn message_hooks(
        &self,
        draft: &str,
        irreversible: &mut Vec<IrreversibleEntry>,
    ) -> Result<String> {
        let file = self.scratch.message();
        write_scratch(&file, draft.as_bytes())?;
        let index = self.scratch.index();
        let argument = file.display().to_string();
        self.run(
            Hook::PrepareCommitMsg,
            vec![argument.clone(), MESSAGE_SOURCE.to_owned()],
            &index,
            irreversible,
        )?;
        self.run(Hook::CommitMsg, vec![argument], &index, irreversible)?;
        let bytes = std::fs::read(&file).map_err(|error| crate::repo::error::io(&file, &error))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Directory outside the repository for the files hooks receive; removed
/// when dropped.
#[derive(Debug)]
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn create(ctx: &ExecCtx) -> Result<Self> {
        let base = &ctx.host().info().temp;
        let stamp = ctx.clock().now().unix_nanos();
        for attempt in 0..SCRATCH_ATTEMPTS {
            let dir = base.join(format!(
                "nurt-commit-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            match create_private_dir(&dir) {
                Ok(()) => return Ok(Self { dir }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(crate::repo::error::io(&dir, &error)),
            }
        }
        Err(Error::new(
            ErrorCode::Io,
            format!(
                "cannot create a scratch directory for the hooks in {}; remove old `nurt-commit-*` directories",
                base.display()
            ),
        ))
    }

    fn index(&self) -> PathBuf {
        self.dir.join("index")
    }

    fn message(&self) -> PathBuf {
        self.dir.join("COMMIT_EDITMSG")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Creates `path` accessible to the creating account only, so that the index and the
/// message the hooks work on are not visible to other users of the host.
#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir(path)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::testutil::test_ctx;

    #[test]
    fn scratch_directory_is_private_to_its_account() {
        let scratch = Scratch::create(&test_ctx()).unwrap();
        let mode = std::fs::metadata(&scratch.dir)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
