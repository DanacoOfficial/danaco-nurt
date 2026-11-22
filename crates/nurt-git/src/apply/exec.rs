// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of a plan through one mutation transaction.
//!
//! Every target is declared and preserved before the first change. The
//! index is guarded by `index.lock` for the whole execution, as `git` does:
//! the lock file is created first, the index file is then checked against
//! the state seen by the plan, the working tree is changed, the new index is
//! written into the lock file and the lock file is renamed over the index.
//! References use the same protocol with `<name>.lock`, and their logs are
//! appended to.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix_index::entry::{Flags, Stage};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, LinkKind, MetaChange};
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};

use super::context::{ApplyContext, lock_path};
use super::error::{locked, object_error, repo_error};
use super::filter::{Direction, Filters};
use super::plan::{
    ApplyPlan, ApplyReport, Content, EntryAction, EntryKind, Notable, PlanEntry, RefPlan,
    StateFilePlan,
};
use super::refs::{log_path, ref_content, ref_path, reflog_bytes};
use super::worktree::{Node, native_path, observe_native, stat_of, verify_expect};

/// Permission bits requested for new directories; the process umask applies.
const DIRECTORY_MODE: u32 = 0o777;

/// Executes `plan` in the transaction `tx`.
///
/// The caller opened the transaction and commits or rolls it back after this
/// function returns; on error nothing must be committed. The plan must come
/// from a plan function called with the same `ctx`.
///
/// # Errors
///
/// `E_GIT_LOCKED` when a lock file of the index or of a reference exists,
/// `E_STALE` when a file changed after the plan was made, and the errors of
/// the transaction (`E_IO`, `E_PERMISSION`, `E_NO_SPACE`, `E_CANCELLED`).
pub fn apply_plan(
    tx: &mut dyn MutationTx,
    ctx: &ApplyContext<'_>,
    plan: ApplyPlan,
) -> Result<ApplyReport> {
    let mut targets = Targets::default();
    let declared = Declared::new(ctx, &plan, &mut targets)?;
    let preserved = tx
        .preserve(&targets.specs)
        .map_err(|err| lock_error(err, &targets))?;
    let index_of: Vec<TargetIndex> = preserved.iter().map(|item| item.index).collect();
    let mut run = Run {
        tx,
        ctx,
        index_of,
        report: ApplyReport::default(),
        filters: Filters::new(
            ctx.repo,
            plan.index.as_ref().map_or(&ctx.index, |index| &index.state),
            Direction::ToWorktree,
        )?,
        caps: ctx
            .repo
            .filesystem_options()
            .map_err(|err| repo_error(err, "read the core settings"))?,
        buf: Vec::new(),
    };
    run.report.notable.extend(plan.notable.iter().cloned());
    if let Some(lock) = declared.index_lock {
        run.mutate(lock, Mutation::WriteBytes(Vec::new()))?;
        if let Some(index) = &plan.index {
            verify_expect(&ctx.index_path, &index.expect)?;
        }
    }
    for (entry, position) in plan.entries.iter().zip(&declared.entries) {
        run.entry(entry, *position)?;
    }
    if let (Some(index), Some(lock), Some(rename)) =
        (plan.index, declared.index_lock, declared.index_rename)
    {
        let mut state = index.state;
        run.refresh_stats(&plan.entries, &mut state)?;
        let mut bytes = Vec::new();
        gix_index::File::from_state(state, ctx.index_path.clone())
            .write_to(&mut bytes, gix_index::write::Options::default())
            .map_err(|err| repo_error(err, "serialize the index"))?;
        run.mutate(lock, Mutation::WriteBytes(bytes))?;
        run.mutate(rename, Mutation::Rename)?;
        run.report.index_written = true;
    }
    for (reference, targets) in plan.refs.iter().zip(&declared.refs) {
        run.reference(reference, targets)?;
    }
    for (state, targets) in plan.state_files.iter().zip(&declared.states) {
        run.state_file(state, targets)?;
    }
    run.report.notable.sort();
    run.report.notable.dedup();
    Ok(run.report)
}

fn abs(path: &Path) -> Result<AbsPath> {
    AbsPath::new(path).map_err(|err| {
        Error::new(
            ErrorCode::Io,
            format!("'{}' is not a usable absolute path: {err}", path.display()),
        )
    })
}

#[derive(Default)]
struct Targets {
    specs: Vec<TargetSpec>,
    locks: Vec<PathBuf>,
    dirs: BTreeSet<PathBuf>,
}

impl Targets {
    fn push(&mut self, path: &Path, action: TargetAction, expect: Expect) -> Result<usize> {
        self.specs.push(TargetSpec {
            path: TargetPath::Fs(abs(path)?),
            action,
            expect,
            size_hint: None,
        });
        Ok(self.specs.len() - 1)
    }

    /// Declares the protocol `<file>.lock` then rename over `<file>`.
    fn lock_and_rename(&mut self, file: &Path) -> Result<(usize, usize)> {
        let lock = lock_path(file);
        self.locks.push(lock.clone());
        let created = self.push(&lock, TargetAction::Create, Expect::Absent)?;
        let renamed = self.push(
            &lock,
            TargetAction::Rename {
                to: abs(file)?,
                replace: true,
            },
            Expect::Unchecked,
        )?;
        Ok((created, renamed))
    }

    /// Declares the creation of the missing directories between `base` and
    /// the parent of `file`, shallowest first.
    fn missing_dirs(&mut self, base: &Path, file: &Path) -> Result<Vec<usize>> {
        let mut missing = Vec::new();
        let mut dir = file.parent();
        while let Some(current) = dir {
            if current == base || !current.starts_with(base) {
                break;
            }
            if self.dirs.contains(current) || observe_native(current)?.node.exists() {
                break;
            }
            missing.push(current.to_path_buf());
            dir = current.parent();
        }
        let mut positions = Vec::new();
        for dir in missing.into_iter().rev() {
            self.dirs.insert(dir.clone());
            positions.push(self.push(&dir, TargetAction::Create, Expect::Absent)?);
        }
        Ok(positions)
    }
}

fn lock_error(err: Error, targets: &Targets) -> Error {
    if err.code() != ErrorCode::Exists {
        return err;
    }
    targets
        .locks
        .iter()
        .find(|lock| lock.exists())
        .map_or(err, |lock| locked(lock))
}

struct RefTargets {
    dirs: Vec<usize>,
    lock: usize,
    rename: usize,
    path: PathBuf,
    log_dirs: Vec<usize>,
    log: Option<(usize, bool)>,
}

struct StateTargets {
    dirs: Vec<usize>,
    file: Option<usize>,
}

struct Declared {
    index_lock: Option<usize>,
    index_rename: Option<usize>,
    entries: Vec<usize>,
    refs: Vec<RefTargets>,
    states: Vec<StateTargets>,
}

impl Declared {
    fn new(ctx: &ApplyContext<'_>, plan: &ApplyPlan, targets: &mut Targets) -> Result<Self> {
        let (index_lock, index_rename) = match &plan.index {
            Some(_) => {
                let (lock, rename) = targets.lock_and_rename(&ctx.index_path)?;
                (Some(lock), Some(rename))
            }
            None => (None, None),
        };
        let mut entries = Vec::with_capacity(plan.entries.len());
        for entry in &plan.entries {
            let path = native_path(&ctx.root, entry.path.as_bstr());
            entries.push(targets.push(&path, target_action(entry.action), entry.expect.clone())?);
        }
        let refs = plan
            .refs
            .iter()
            .map(|reference| declare_ref(ctx.repo, reference, targets))
            .collect::<Result<Vec<_>>>()?;
        let states = plan
            .state_files
            .iter()
            .map(|state| declare_state(ctx.repo, state, targets))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            index_lock,
            index_rename,
            entries,
            refs,
            states,
        })
    }
}

fn target_action(action: EntryAction) -> TargetAction {
    match action {
        EntryAction::Create | EntryAction::Mkdir => TargetAction::Create,
        EntryAction::Modify => TargetAction::Modify,
        EntryAction::Delete | EntryAction::Rmdir => TargetAction::Delete,
        EntryAction::Symlink => TargetAction::Link,
        EntryAction::Chmod => TargetAction::Attributes,
        EntryAction::TypeChange => TargetAction::Replace,
    }
}

fn declare_ref(
    repo: &gix::Repository,
    reference: &RefPlan,
    targets: &mut Targets,
) -> Result<RefTargets> {
    let path = ref_path(repo, &reference.name)?;
    let base = if path.starts_with(repo.git_dir()) {
        repo.git_dir()
    } else {
        repo.common_dir()
    };
    let dirs = targets.missing_dirs(base, &path)?;
    let (lock, rename) = targets.lock_and_rename(&path)?;
    let (log_dirs, log) = match &reference.reflog {
        Some(_) => {
            let log = log_path(repo, &reference.name)?;
            let dirs = targets.missing_dirs(base, &log)?;
            let exists = observe_native(&log)?.node.exists();
            let action = if exists {
                TargetAction::Append
            } else {
                TargetAction::Create
            };
            let expect = if exists {
                Expect::Unchecked
            } else {
                Expect::Absent
            };
            (dirs, Some((targets.push(&log, action, expect)?, exists)))
        }
        None => (Vec::new(), None),
    };
    Ok(RefTargets {
        dirs,
        lock,
        rename,
        path,
        log_dirs,
        log,
    })
}

fn declare_state(
    repo: &gix::Repository,
    state: &StateFilePlan,
    targets: &mut Targets,
) -> Result<StateTargets> {
    let path = repo
        .git_dir()
        .join(state.path.split('/').collect::<PathBuf>());
    let exists = observe_native(&path)?.node.exists();
    let (dirs, file) = match (&state.content, exists) {
        (Some(_), true) => (
            Vec::new(),
            Some(targets.push(&path, TargetAction::Modify, state.expect.clone())?),
        ),
        (Some(_), false) => (
            targets.missing_dirs(repo.git_dir(), &path)?,
            Some(targets.push(&path, TargetAction::Create, Expect::Absent)?),
        ),
        (None, true) => (
            Vec::new(),
            Some(targets.push(&path, TargetAction::Delete, state.expect.clone())?),
        ),
        (None, false) => (Vec::new(), None),
    };
    Ok(StateTargets { dirs, file })
}

struct Run<'t, 'c, 'r> {
    tx: &'t mut dyn MutationTx,
    ctx: &'c ApplyContext<'r>,
    index_of: Vec<TargetIndex>,
    report: ApplyReport,
    filters: Filters<'r>,
    caps: gix_fs::Capabilities,
    buf: Vec<u8>,
}

impl Run<'_, '_, '_> {
    fn mutate(&mut self, position: usize, mutation: Mutation) -> Result<()> {
        let index = self.index_of.get(position).copied().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "the journal returned fewer targets than declared",
            )
        })?;
        self.tx.apply(index, mutation).map(|_| ())
    }

    fn entry(&mut self, entry: &PlanEntry, position: usize) -> Result<()> {
        let path = native_path(&self.ctx.root, entry.path.as_bstr());
        match entry.action {
            EntryAction::Delete => {
                self.mutate(position, Mutation::Remove)?;
                self.report.removed.push(entry.path.clone());
            }
            EntryAction::Rmdir => {
                if is_empty_dir(&path) {
                    self.mutate(position, Mutation::Remove)?;
                    self.report.removed.push(entry.path.clone());
                }
            }
            EntryAction::Mkdir => {
                self.mutate(
                    position,
                    Mutation::CreateDir {
                        mode: DIRECTORY_MODE,
                    },
                )?;
                self.report.created_dirs.push(entry.path.clone());
            }
            EntryAction::Chmod => {
                self.set_executable(position, &path, entry.kind == Some(EntryKind::Executable))?;
                self.report.written.push(entry.path.clone());
            }
            EntryAction::TypeChange => {
                self.mutate(position, Mutation::Remove)?;
                self.create(entry, position, &path)?;
            }
            EntryAction::Create | EntryAction::Modify | EntryAction::Symlink => {
                self.create(entry, position, &path)?;
            }
        }
        Ok(())
    }

    fn create(&mut self, entry: &PlanEntry, position: usize, path: &Path) -> Result<()> {
        let kind = entry.kind.unwrap_or(EntryKind::File);
        if kind == EntryKind::Directory {
            self.mutate(
                position,
                Mutation::CreateDir {
                    mode: DIRECTORY_MODE,
                },
            )?;
            self.report.created_dirs.push(entry.path.clone());
            return Ok(());
        }
        let bytes = self.content(entry, kind)?;
        if kind == EntryKind::Symlink && self.caps.symlink {
            let target = gix_path::from_bstr(bytes.as_bstr()).into_owned();
            let target =
                gix_path::to_native_path_on_windows(gix_path::into_bstr(target)).into_owned();
            self.mutate(
                position,
                Mutation::CreateLink {
                    target,
                    kind: LinkKind::SymlinkFile,
                },
            )?;
        } else {
            self.mutate(position, Mutation::WriteBytes(bytes))?;
            self.set_executable(position, path, kind == EntryKind::Executable)?;
        }
        self.report.written.push(entry.path.clone());
        Ok(())
    }

    fn content(&mut self, entry: &PlanEntry, kind: EntryKind) -> Result<Vec<u8>> {
        match &entry.content {
            Some(Content::Bytes(bytes)) => Ok(bytes.clone()),
            Some(Content::Blob(id)) => {
                let data = gix_object::Find::try_find(&self.ctx.repo.objects, id, &mut self.buf)
                    .map_err(|err| object_error(err, id))?
                    .ok_or_else(|| object_error("not found", id))?
                    .data
                    .to_vec();
                if kind == EntryKind::Symlink {
                    return Ok(data);
                }
                let (bytes, notable) = self.filters.to_worktree(entry.path.as_bstr(), &data)?;
                self.report.notable.extend(notable);
                Ok(bytes)
            }
            None => Err(Error::new(
                ErrorCode::StageFailed,
                format!("the plan has no content for '{}'", entry.path),
            )),
        }
    }

    #[cfg(unix)]
    fn set_executable(&mut self, position: usize, path: &Path, executable: bool) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        const EXECUTE_BITS: u32 = 0o111;
        const READ_TO_EXECUTE_SHIFT: u32 = 2;
        if !self.caps.executable_bit {
            return Ok(());
        }
        let meta =
            std::fs::symlink_metadata(path).map_err(|err| super::error::io_error(&err, path))?;
        let mode = meta.permissions().mode() & 0o7777;
        let wanted = if executable {
            mode | ((mode & 0o444) >> READ_TO_EXECUTE_SHIFT)
        } else {
            mode & !EXECUTE_BITS
        };
        if wanted != mode {
            self.mutate(
                position,
                Mutation::SetMeta(MetaChange {
                    mode: Some(wanted),
                    ..MetaChange::default()
                }),
            )?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn set_executable(&mut self, _position: usize, _path: &Path, _executable: bool) -> Result<()> {
        Ok(())
    }

    /// Takes the statistics of written files into their index entries, so
    /// that `git status` does not need to read them again.
    fn refresh_stats(&self, entries: &[PlanEntry], state: &mut gix_index::State) -> Result<()> {
        for entry in entries {
            let Some(Content::Blob(id)) = &entry.content else {
                continue;
            };
            let path: &BStr = entry.path.as_bstr();
            let native = native_path(&self.ctx.root, path);
            if let Some(index_entry) = state.entry_mut_by_path_and_stage(path, Stage::Unconflicted)
                && index_entry.id == *id
                && !index_entry.flags.contains(Flags::SKIP_WORKTREE)
                && observe_native(&native)?.node != Node::Missing
            {
                index_entry.stat = stat_of(&native)?;
            }
        }
        Ok(())
    }

    fn reference(&mut self, reference: &RefPlan, targets: &RefTargets) -> Result<()> {
        for dir in &targets.dirs {
            self.mutate(
                *dir,
                Mutation::CreateDir {
                    mode: DIRECTORY_MODE,
                },
            )?;
        }
        self.mutate(
            targets.lock,
            Mutation::WriteBytes(ref_content(&reference.target)),
        )?;
        verify_expect(&targets.path, &reference.expect)?;
        self.mutate(targets.rename, Mutation::Rename)?;
        if let (Some(line), Some((log, exists))) = (&reference.reflog, targets.log) {
            for dir in &targets.log_dirs {
                self.mutate(
                    *dir,
                    Mutation::CreateDir {
                        mode: DIRECTORY_MODE,
                    },
                )?;
            }
            let bytes = reflog_bytes(line)?;
            let mutation = if exists {
                Mutation::AppendBytes(bytes)
            } else {
                Mutation::WriteBytes(bytes)
            };
            self.mutate(log, mutation)?;
        }
        self.report.refs_updated.push(reference.name.clone());
        Ok(())
    }

    fn state_file(&mut self, state: &StateFilePlan, targets: &StateTargets) -> Result<()> {
        for dir in &targets.dirs {
            self.mutate(
                *dir,
                Mutation::CreateDir {
                    mode: DIRECTORY_MODE,
                },
            )?;
        }
        if let Some(file) = targets.file {
            let mutation = match &state.content {
                Some(bytes) => Mutation::WriteBytes(bytes.clone()),
                None => Mutation::Remove,
            };
            self.mutate(file, mutation)?;
        }
        Ok(())
    }
}

fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut reader| reader.next().is_none())
}
