// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Execution of a plan through one mutation transaction.
//!
//! Every target is declared and preserved before the first change. The index
//! and each reference file are single `modify` targets that expect the state
//! seen by the plan. Their lock files (`<file>.lock`) are honoured but not
//! created: when one exists the execution fails with `E_GIT_LOCKED`, when the
//! targets are declared and again before the file is written. Reference logs
//! are appended to. The executable bit of a new file is set on the staged file
//! before it is switched in; for an existing file it is an `attributes` target.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use bstr::{BStr, ByteSlice};
use gix_index::entry::{Flags, Stage};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, LinkKind};
use nurt_contracts::journal::{
    Expect, Mutation, MutationTx, TargetAction, TargetIndex, TargetPath, TargetSpec,
};

use super::context::{ApplyContext, lock_path};
use super::error::{locked, object_error, repo_error};
use super::filter::{Direction, Filters};
use super::plan::{
    ApplyPlan, ApplyReport, Content, EntryAction, EntryKind, PlanEntry, RefPlan, StateFilePlan,
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
    let caps = ctx
        .repo
        .filesystem_options()
        .map_err(|err| repo_error(err, "read the core settings"))?;
    let mut targets = Targets::default();
    let declared = Declared::new(ctx, &plan, &caps, &mut targets)?;
    let preserved = tx.preserve(&targets.specs)?;
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
        caps,
        buf: Vec::new(),
    };
    run.report.notable.extend(plan.notable.iter().cloned());
    if let Some(index) = &plan.index {
        ensure_unlocked(&ctx.index_path)?;
        verify_expect(&ctx.index_path, &index.expect)?;
    }
    for (entry, targets) in plan.entries.iter().zip(&declared.entries) {
        run.entry(entry, targets)?;
    }
    if let (Some(index), Some(file)) = (plan.index, declared.index) {
        let mut state = index.state;
        run.refresh_stats(&plan.entries, &mut state)?;
        let resolved = if index.record_resolve_undo {
            super::resolve_undo::payload(&ctx.index, &state)
        } else {
            Vec::new()
        };
        let mut bytes = Vec::new();
        gix_index::File::from_state(state, ctx.index_path.clone())
            .write_to(&mut bytes, gix_index::write::Options::default())
            .map_err(|err| repo_error(err, "serialize the index"))?;
        super::resolve_undo::append(&mut bytes, &resolved, ctx.repo.object_hash());
        ensure_unlocked(&ctx.index_path)?;
        run.mutate(file, Mutation::WriteBytes(bytes))?;
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

    /// Declares `file` as one `modify` target expecting `expect`; fails when
    /// its lock file exists.
    fn guarded(&mut self, file: &Path, expect: Expect) -> Result<usize> {
        ensure_unlocked(file)?;
        self.push(file, TargetAction::Modify, expect)
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

/// Fails with `E_GIT_LOCKED` when the lock file of `file` exists.
fn ensure_unlocked(file: &Path) -> Result<()> {
    let lock = lock_path(file);
    if lock.exists() {
        return Err(locked(&lock));
    }
    Ok(())
}

/// Targets of one plan entry.
struct EntryTargets {
    main: usize,
    /// `attributes` target and the permission bits to set after the write of
    /// an existing file.
    mode: Option<(usize, u32)>,
    /// The entry is created as a new file, so its mode is set on the staged
    /// file.
    fresh: bool,
}

struct RefTargets {
    dirs: Vec<usize>,
    file: usize,
    path: PathBuf,
    log_dirs: Vec<usize>,
    log: Option<(usize, bool)>,
}

struct StateTargets {
    dirs: Vec<usize>,
    file: Option<usize>,
}

struct Declared {
    index: Option<usize>,
    entries: Vec<EntryTargets>,
    refs: Vec<RefTargets>,
    states: Vec<StateTargets>,
}

impl Declared {
    fn new(
        ctx: &ApplyContext<'_>,
        plan: &ApplyPlan,
        caps: &gix::fs::Capabilities,
        targets: &mut Targets,
    ) -> Result<Self> {
        let index = match &plan.index {
            Some(index) => Some(targets.guarded(&ctx.index_path, index.expect.clone())?),
            None => None,
        };
        let mut entries = Vec::with_capacity(plan.entries.len());
        for entry in &plan.entries {
            let path = native_path(&ctx.root, entry.path.as_bstr())?;
            let main = targets.push(
                &path,
                target_action(entry.action, caps),
                entry.expect.clone(),
            )?;
            let existing = matches!(entry.action, EntryAction::Modify)
                && std::fs::symlink_metadata(&path).is_ok();
            let fresh = match entry.action {
                EntryAction::Create | EntryAction::TypeChange => true,
                EntryAction::Modify => !existing,
                _ => false,
            };
            let mut mode = None;
            if existing && matches!(entry.kind, Some(EntryKind::File | EntryKind::Executable)) {
                let executable = entry.kind == Some(EntryKind::Executable);
                if let Some(wanted) = mode_change(&path, executable, caps)? {
                    let position =
                        targets.push(&path, TargetAction::Attributes, Expect::Unchecked)?;
                    mode = Some((position, wanted));
                }
            }
            entries.push(EntryTargets { main, mode, fresh });
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
            index,
            entries,
            refs,
            states,
        })
    }
}

fn target_action(action: EntryAction, caps: &gix::fs::Capabilities) -> TargetAction {
    match action {
        EntryAction::Create | EntryAction::Mkdir => TargetAction::Create,
        EntryAction::Modify => TargetAction::Modify,
        EntryAction::Delete | EntryAction::Rmdir => TargetAction::Delete,
        EntryAction::Symlink if caps.symlink => TargetAction::Link,
        EntryAction::Symlink => TargetAction::Create,
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
    let file = targets.guarded(&path, reference.expect.clone())?;
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
        file,
        path,
        log_dirs,
        log,
    })
}

/// Whether Git keeps the file `name` of the repository directory once for
/// all working trees (the common directory) and not per working tree.
fn is_shared_state(name: &str) -> bool {
    const PER_WORKTREE: [&str; 5] = [
        "info/sparse-checkout",
        "logs/HEAD",
        "logs/refs/bisect",
        "logs/refs/worktree",
        "logs/refs/rewritten",
    ];
    const PER_WORKTREE_REFS: [&str; 3] = ["refs/bisect", "refs/worktree", "refs/rewritten"];
    const SHARED_DIRECTORIES: [&str; 12] = [
        "branches",
        "hooks",
        "info",
        "logs",
        "lost-found",
        "modules",
        "objects",
        "refs",
        "remotes",
        "rr-cache",
        "svn",
        "worktrees",
    ];
    const SHARED_FILES: [&str; 4] = ["config", "gc.pid", "packed-refs", "shallow"];
    let under = |prefix: &str| {
        name == prefix
            || name
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    if PER_WORKTREE.iter().chain(&PER_WORKTREE_REFS).any(|prefix| under(prefix)) {
        return false;
    }
    SHARED_FILES.contains(&name) || SHARED_DIRECTORIES.iter().any(|prefix| under(prefix))
}

fn declare_state(
    repo: &gix::Repository,
    state: &StateFilePlan,
    targets: &mut Targets,
) -> Result<StateTargets> {
    let base = if is_shared_state(&state.path) {
        repo.common_dir()
    } else {
        repo.git_dir()
    };
    let path = base.join(state.path.split('/').collect::<PathBuf>());
    let exists = observe_native(&path)?.node.exists();
    let (dirs, file) = match (&state.content, exists) {
        (Some(_), true) => (
            Vec::new(),
            Some(targets.push(&path, TargetAction::Modify, state.expect.clone())?),
        ),
        (Some(_), false) => (
            targets.missing_dirs(base, &path)?,
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
    caps: gix::fs::Capabilities,
    buf: Vec<u8>,
}

impl Run<'_, '_, '_> {
    fn index_at(&self, position: usize) -> Result<TargetIndex> {
        self.index_of.get(position).copied().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                "the journal returned fewer targets than declared",
            )
        })
    }

    fn mutate(&mut self, position: usize, mutation: Mutation) -> Result<()> {
        let index = self.index_at(position)?;
        self.tx.apply(index, mutation).map(|_| ())
    }

    fn entry(&mut self, entry: &PlanEntry, targets: &EntryTargets) -> Result<()> {
        let position = targets.main;
        let path = native_path(&self.ctx.root, entry.path.as_bstr())?;
        self.ctx
            .guards
            .verify(&self.ctx.root, entry.path.as_bstr())?;
        if matches!(
            entry.action,
            EntryAction::Mkdir | EntryAction::Rmdir | EntryAction::Delete | EntryAction::TypeChange
        ) {
            self.ctx.guards.forget(&path);
        }
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
                let executable = entry.kind == Some(EntryKind::Executable);
                if let Some(wanted) = mode_change(&path, executable, &self.caps)? {
                    self.set_mode(position, wanted)?;
                }
                self.report.written.push(entry.path.clone());
            }
            EntryAction::TypeChange => {
                self.mutate(position, Mutation::Remove)?;
                self.create(entry, targets)?;
            }
            EntryAction::Create | EntryAction::Modify | EntryAction::Symlink => {
                self.create(entry, targets)?;
            }
        }
        Ok(())
    }

    fn create(&mut self, entry: &PlanEntry, targets: &EntryTargets) -> Result<()> {
        let position = targets.main;
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
            let target = gix_path::to_native_path_on_windows(bytes.as_bstr())
                .map_err(|err| {
                    Error::new(
                        ErrorCode::Io,
                        format!(
                            "the link target of '{}' cannot be represented on this system: {err}",
                            entry.path
                        ),
                    )
                    .with_found(entry.path.to_string())
                })?
                .into_owned();
            self.mutate(
                position,
                Mutation::CreateLink {
                    target,
                    kind: LinkKind::SymlinkFile,
                },
            )?;
        } else if targets.fresh {
            self.write_new(position, bytes, kind == EntryKind::Executable)?;
        } else {
            self.mutate(position, Mutation::WriteBytes(bytes))?;
            if let Some((mode_position, wanted)) = targets.mode {
                self.set_mode(mode_position, wanted)?;
            }
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
                let (bytes, notable) = self.filters.smudge(entry.path.as_bstr(), &data)?;
                self.report.notable.extend(notable);
                Ok(bytes)
            }
            None => Err(Error::new(
                ErrorCode::StageFailed,
                format!("the plan has no content for '{}'", entry.path),
            )),
        }
    }

    fn set_mode(&mut self, position: usize, mode: u32) -> Result<()> {
        self.mutate(
            position,
            Mutation::SetMeta(nurt_contracts::fs::MetaChange {
                mode: Some(mode),
                ..nurt_contracts::fs::MetaChange::default()
            }),
        )
    }

    /// Writes the content of a file that does not exist yet. An executable
    /// file is written through a staged file whose mode is set before it is
    /// switched in, because a mutation of the declared `create` or `modify`
    /// target cannot change metadata.
    #[cfg(unix)]
    fn write_new(&mut self, position: usize, bytes: Vec<u8>, executable: bool) -> Result<()> {
        if !(executable && self.caps.executable_bit) {
            return self.mutate(position, Mutation::WriteBytes(bytes));
        }
        let index = self.index_at(position)?;
        let mut handle = self.tx.stage(index)?;
        let staged = handle.path().as_path().to_path_buf();
        handle
            .write_all(&bytes)
            .map_err(|err| super::error::io_error(&err, &staged))?;
        let meta = std::fs::symlink_metadata(&staged)
            .map_err(|err| super::error::io_error(&err, &staged))?;
        let wanted = mode::with_executable(mode::bits(&meta), true);
        handle.apply_meta(&mode::extended(&meta, wanted))?;
        drop(handle);
        self.tx.apply(index, Mutation::CommitStaged).map(|_| ())
    }

    #[cfg(not(unix))]
    fn write_new(&mut self, position: usize, bytes: Vec<u8>, _executable: bool) -> Result<()> {
        self.mutate(position, Mutation::WriteBytes(bytes))
    }

    /// Takes the statistics of written files into their index entries, so
    /// that `git status` does not need to read them again.
    fn refresh_stats(&self, entries: &[PlanEntry], state: &mut gix_index::State) -> Result<()> {
        for entry in entries {
            let Some(Content::Blob(id)) = &entry.content else {
                continue;
            };
            let path: &BStr = entry.path.as_bstr();
            let native = native_path(&self.ctx.root, path)?;
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
        ensure_unlocked(&targets.path)?;
        self.mutate(
            targets.file,
            Mutation::WriteBytes(ref_content(&reference.target)),
        )?;
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

/// Whether `path` is a directory without entries.
pub(crate) fn is_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut reader| reader.next().is_none())
}

/// Permission bits `path` needs to get the executable state `executable`;
/// `None` when it has them or the file system has no executable bit.
#[cfg(unix)]
fn mode_change(path: &Path, executable: bool, caps: &gix::fs::Capabilities) -> Result<Option<u32>> {
    if !caps.executable_bit {
        return Ok(None);
    }
    let meta = std::fs::symlink_metadata(path).map_err(|err| super::error::io_error(&err, path))?;
    let current = mode::bits(&meta);
    let wanted = mode::with_executable(current, executable);
    Ok((wanted != current).then_some(wanted))
}

#[cfg(not(unix))]
fn mode_change(
    _path: &Path,
    _executable: bool,
    _caps: &gix::fs::Capabilities,
) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(unix)]
mod mode {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use nurt_contracts::fs::{ExtendedMeta, FileId, FileKind, FileMeta};

    const PERMISSION_BITS: u32 = 0o7777;
    const EXECUTE_BITS: u32 = 0o111;
    const READ_BITS: u32 = 0o444;
    const READ_TO_EXECUTE_SHIFT: u32 = 2;
    const NANOS_PER_SECOND: i64 = 1_000_000_000;

    /// Permission bits of `meta`.
    pub(super) fn bits(meta: &std::fs::Metadata) -> u32 {
        meta.permissions().mode() & PERMISSION_BITS
    }

    /// `mode` with the execute bits set where the read bits are set, or
    /// cleared.
    pub(super) fn with_executable(mode: u32, executable: bool) -> u32 {
        if executable {
            mode | ((mode & READ_BITS) >> READ_TO_EXECUTE_SHIFT)
        } else {
            mode & !EXECUTE_BITS
        }
    }

    fn nanos(seconds: i64, nanoseconds: i64) -> i64 {
        seconds
            .saturating_mul(NANOS_PER_SECOND)
            .saturating_add(nanoseconds)
    }

    /// Metadata of the regular file `meta` with the permission bits `mode`,
    /// in the form accepted by `WriteHandle::apply_meta`.
    pub(super) fn extended(meta: &std::fs::Metadata, mode: u32) -> ExtendedMeta {
        ExtendedMeta {
            basic: FileMeta {
                kind: FileKind::File,
                size: meta.len(),
                mode,
                uid: Some(meta.uid()),
                gid: Some(meta.gid()),
                links: meta.nlink(),
                modified_ns: nanos(meta.mtime(), meta.mtime_nsec()),
                accessed_ns: Some(nanos(meta.atime(), meta.atime_nsec())),
                created_ns: None,
                changed_ns: Some(nanos(meta.ctime(), meta.ctime_nsec())),
                file_id: FileId {
                    volume: meta.dev(),
                    low: meta.ino(),
                    high: 0,
                },
                windows_attributes: 0,
            },
            xattrs: Vec::new(),
            acl: None,
            account_sid: None,
            flags: None,
            streams: Vec::new(),
            link_target: None,
        }
    }
}

#[cfg(test)]
mod shared_state_tests {
    use super::is_shared_state;

    #[test]
    fn state_files_of_a_working_tree_stay_in_its_directory() {
        for name in [
            "HEAD",
            "ORIG_HEAD",
            "MERGE_HEAD",
            "MERGE_MSG",
            "AUTO_MERGE",
            "FETCH_HEAD",
            "REBASE_HEAD",
            "index",
            "rebase-merge/head-name",
            "logs/HEAD",
            "refs/bisect/bad",
            "refs/worktree/x",
            "info/sparse-checkout",
        ] {
            assert!(!is_shared_state(name), "{name}");
        }
    }

    #[test]
    fn files_shared_by_all_working_trees_live_in_the_common_directory() {
        for name in [
            "config",
            "packed-refs",
            "shallow",
            "info/exclude",
            "hooks/pre-commit",
            "refs/stash",
            "logs/refs/stash",
            "objects/info/alternates",
        ] {
            assert!(is_shared_state(name), "{name}");
        }
        assert!(!is_shared_state("refsx"));
    }
}
