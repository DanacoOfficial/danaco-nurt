// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Working tree access and the one-way checkout planner used by
//! `git.reset` and `git.stash`.
//!
//! The planner compares the current index, the target entries and the
//! working tree and produces journal targets: deletions first, then the
//! removal of directories left empty, then writes. Configured filter drivers
//! are not run; the built-in line ending, `ident` and encoding filters are
//! applied.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use gix::bstr::{BStr, BString, ByteSlice};
use gix::hash::ObjectId;
use gix::index::State;
use gix::index::entry::{Mode, Stat};
use nurt_contracts::error::{ErrorCode, Result};

use super::index::{IndexUpdate, merged_entry};
use super::repo::{Repo, error, io_error, quote};
use super::txplan::{Before, TxPlan, content_hash};

/// Permission bits of executable and regular files.
const MODE_EXECUTABLE: u32 = 0o755;
const MODE_REGULAR: u32 = 0o644;

/// What the working tree holds at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Probe {
    /// Nothing, or a parent component is not a directory.
    Absent,
    /// A regular file.
    File {
        /// Executable bit.
        exec: bool,
        /// Content.
        data: Vec<u8>,
    },
    /// A symbolic link.
    Link(PathBuf),
    /// A directory.
    Dir,
    /// Another kind of entry.
    Other,
}

impl Probe {
    /// State of the path for a journal target.
    pub(crate) fn before(&self) -> Before {
        match self {
            Self::Absent => Before::Absent,
            Self::File { data, .. } => Before::File(content_hash(data)),
            Self::Link(_) | Self::Dir | Self::Other => Before::Other,
        }
    }
}

/// Reads the working tree and converts content between the repository and
/// the working tree.
pub(crate) struct Worktree<'r> {
    repo: &'r Repo,
    pipeline: gix_filter::Pipeline,
    attributes: gix_worktree::Stack,
    file_mode: bool,
    symlinks: bool,
    index_mtime: Option<u32>,
}

fn read_all(mut reader: impl Read) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    reader
        .read_to_end(&mut out)
        .map_err(|cause| io_error("cannot read file", cause))?;
    Ok(out)
}

impl<'r> Worktree<'r> {
    /// Working tree of `repo`.
    pub(crate) fn new(repo: &'r Repo) -> Result<Self> {
        let (pipeline, _) = repo
            .repo
            .filter_pipeline(None)
            .map_err(|cause| io_error("cannot prepare filters", cause))?;
        let (mut pipeline, attributes) = pipeline.into_parts();
        pipeline.options_mut().drivers.clear();
        let config = repo.repo.config_snapshot();
        let index_mtime = std::fs::metadata(repo.index_path())
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|elapsed| u32::try_from(elapsed.as_secs()).ok());
        Ok(Self {
            repo,
            pipeline,
            attributes,
            file_mode: config.boolean("core.filemode").unwrap_or(true),
            symlinks: config.boolean("core.symlinks").unwrap_or(true),
            index_mtime,
        })
    }

    /// The repository.
    pub(crate) fn repo(&self) -> &'r Repo {
        self.repo
    }

    /// Entry at `rel`.
    pub(crate) fn probe(&self, rel: &BStr) -> Result<Probe> {
        let path = self.repo.worktree_path(rel);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(cause)
                if matches!(
                    cause.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(Probe::Absent);
            }
            Err(cause) => return Err(io_error("cannot read the working tree", cause)),
        };
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target = std::fs::read_link(&path)
                .map_err(|cause| io_error("cannot read link", cause))?;
            Ok(Probe::Link(target))
        } else if kind.is_dir() {
            Ok(Probe::Dir)
        } else if kind.is_file() {
            let data = std::fs::read(&path).map_err(|cause| io_error("cannot read file", cause))?;
            Ok(Probe::File {
                exec: is_executable(&meta),
                data,
            })
        } else {
            Ok(Probe::Other)
        }
    }

    /// Content of a working tree file converted for storage in the
    /// repository.
    pub(crate) fn to_git(&mut self, rel: &BStr, data: &[u8]) -> Result<Vec<u8>> {
        let path = gix::path::from_bstr(rel);
        let platform = self
            .attributes
            .at_entry(rel, None, &self.repo.repo.objects)
            .map_err(|cause| io_error("cannot read attributes", cause))?;
        let outcome = self
            .pipeline
            .convert_to_git(
                data,
                &path,
                &mut |_, attributes| {
                    platform.matching_attributes(attributes);
                },
                &mut |_| Ok(None),
            )
            .map_err(|cause| io_error("cannot convert file", cause))?;
        match outcome {
            gix_filter::pipeline::convert::ToGitOutcome::Unchanged(reader) => read_all(reader),
            gix_filter::pipeline::convert::ToGitOutcome::Process(reader) => read_all(reader),
            gix_filter::pipeline::convert::ToGitOutcome::Buffer(buffer) => Ok(buffer.to_vec()),
        }
    }

    /// Content of a blob converted for the working tree.
    pub(crate) fn to_worktree(&mut self, rel: &BStr, data: &[u8]) -> Result<Vec<u8>> {
        let platform = self
            .attributes
            .at_entry(rel, None, &self.repo.repo.objects)
            .map_err(|cause| io_error("cannot read attributes", cause))?;
        let outcome = self
            .pipeline
            .convert_to_worktree(
                data,
                rel,
                &mut |_, attributes| {
                    platform.matching_attributes(attributes);
                },
                gix_filter::pipeline::convert::to_worktree::Options::default(),
            )
            .map_err(|cause| io_error("cannot convert file", cause))?;
        Ok(match outcome {
            gix_filter::pipeline::convert::ToWorktreeOutcome::Unchanged(bytes)
            | gix_filter::pipeline::convert::ToWorktreeOutcome::Buffer(bytes) => bytes.to_vec(),
            gix_filter::pipeline::convert::ToWorktreeOutcome::Process(_) => data.to_vec(),
        })
    }

    /// Object name of a blob with this content.
    pub(crate) fn hash_blob(&self, data: &[u8]) -> Result<ObjectId> {
        gix::objs::compute_hash(self.repo.repo.object_hash(), gix::objs::Kind::Blob, data)
            .map_err(|cause| io_error("cannot hash content", cause))
    }

    fn blob(&self, id: ObjectId) -> Result<Vec<u8>> {
        let object = self
            .repo
            .repo
            .find_object(id)
            .map_err(|cause| io_error("cannot read blob", cause))?;
        Ok(object.data.clone())
    }

    /// Blob of the working tree entry at `rel` as it would be stored, with
    /// its mode; `None` when nothing that git tracks is there.
    pub(crate) fn stored_form(&mut self, rel: &BStr, probe: &Probe) -> Result<Option<(Vec<u8>, Mode)>> {
        Ok(match probe {
            Probe::File { exec, data } => {
                let mode = if *exec && self.file_mode {
                    Mode::FILE_EXECUTABLE
                } else {
                    Mode::FILE
                };
                Some((self.to_git(rel, data)?, mode))
            }
            Probe::Link(target) => Some((
                gix::path::into_bstr(target.as_path()).into_owned().into(),
                Mode::SYMLINK,
            )),
            Probe::Absent | Probe::Dir | Probe::Other => None,
        })
    }

    /// Whether the working tree holds the content of `entry`.
    pub(crate) fn matches(&mut self, rel: &BStr, entry: &gix::index::Entry) -> Result<bool> {
        if entry.mode == Mode::COMMIT {
            return Ok(true);
        }
        let probe = self.probe(rel)?;
        if let Probe::File { exec, .. } = &probe
            && self.file_mode
            && entry.mode != Mode::SYMLINK
            && *exec != (entry.mode == Mode::FILE_EXECUTABLE)
        {
            return Ok(false);
        }
        if self.stat_matches(rel, &entry.stat) {
            return Ok(true);
        }
        let stored = match (&probe, entry.mode == Mode::SYMLINK) {
            (Probe::Link(_), true) | (Probe::File { .. }, false) => self.stored_form(rel, &probe)?,
            (Probe::File { data, .. }, true) if !self.symlinks => Some((data.clone(), Mode::SYMLINK)),
            _ => None,
        };
        match stored {
            Some((data, _)) => Ok(self.hash_blob(&data)? == entry.id),
            None => Ok(false),
        }
    }

    fn stat_matches(&self, rel: &BStr, stat: &Stat) -> bool {
        if *stat == Stat::default() {
            return false;
        }
        let racy = self
            .index_mtime
            .is_none_or(|index_mtime| stat.mtime.secs >= index_mtime);
        if racy {
            return false;
        }
        let path = self.repo.worktree_path(rel);
        gix::index::fs::Metadata::from_path_no_follow(&path)
            .ok()
            .and_then(|meta| Stat::from_fs(&meta).ok())
            .is_some_and(|current| current == *stat)
    }

    /// Untracked files, and with `ignored` also ignored files, one path per
    /// file, sorted.
    pub(crate) fn untracked(&self, index: &State, ignored: bool) -> Result<Vec<BString>> {
        let interrupt = std::sync::atomic::AtomicBool::new(false);
        let mut options = self
            .repo
            .repo
            .dirwalk_options()
            .map_err(|cause| io_error("cannot list untracked files", cause))?
            .emit_untracked(gix::dir::walk::EmissionMode::Matching);
        if ignored {
            options = options.emit_ignored(Some(gix::dir::walk::EmissionMode::Matching));
        }
        let mut collect = gix::dir::walk::delegate::Collect::default();
        self.repo
            .repo
            .dirwalk(index, None::<&str>, &interrupt, options, &mut collect)
            .map_err(|cause| io_error("cannot list untracked files", cause))?;
        Ok(collect
            .into_entries_by_path()
            .into_iter()
            .filter(|(entry, _)| match entry.status {
                gix::dir::entry::Status::Untracked => true,
                gix::dir::entry::Status::Ignored(_) => ignored,
                _ => false,
            })
            .filter(|(entry, _)| {
                !matches!(entry.disk_kind, Some(gix::dir::entry::Kind::Directory))
            })
            .map(|(entry, _)| entry.rela_path)
            .collect())
    }

    /// Plans writing the blob `id` with `mode` at `rel`.
    pub(crate) fn plan_write(
        &mut self,
        tx: &mut TxPlan,
        rel: &BStr,
        id: ObjectId,
        mode: Mode,
        probe: &Probe,
    ) -> Result<()> {
        let path = self.repo.worktree_path(rel);
        tx.ensure_parent_dirs(&path, &self.repo.workdir)?;
        if mode == Mode::COMMIT {
            return Ok(());
        }
        let blob = self.blob(id)?;
        if mode == Mode::SYMLINK && self.symlinks {
            let target = gix::path::from_byte_slice(&blob).into_owned();
            return tx.link(&path, target, &probe.before());
        }
        let bytes = if mode == Mode::SYMLINK {
            blob
        } else {
            self.to_worktree(rel, &blob)?
        };
        tx.write(&path, bytes, &probe.before())?;
        if self.file_mode {
            let was_exec = matches!(probe, Probe::File { exec: true, .. });
            let want_exec = mode == Mode::FILE_EXECUTABLE;
            if want_exec != was_exec {
                tx.set_mode(&path, if want_exec { MODE_EXECUTABLE } else { MODE_REGULAR })?;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// How the working tree is brought to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    /// Overwrite local changes and colliding untracked entries.
    Hard,
    /// Keep local changes of paths whose index entry equals the target;
    /// refuse when another path has local changes (`git reset --merge`).
    Merge,
    /// Keep local changes of paths equal in `HEAD` and the target; refuse
    /// when another path has local changes (`git reset --keep`).
    Keep,
}

/// Input of the planner.
pub(crate) struct Checkout<'a> {
    /// Current index.
    pub(crate) current: &'a State,
    /// Entries of `HEAD`, needed by [`Policy::Keep`].
    pub(crate) head: Option<&'a State>,
    /// Target entries.
    pub(crate) target: &'a State,
    /// Policy.
    pub(crate) policy: Policy,
    /// Paths the checkout is limited to; all paths when `None`.
    pub(crate) scope: Option<&'a dyn Fn(&BStr) -> bool>,
    /// Untracked files removed together with the checkout.
    pub(crate) extra_removals: &'a [BString],
}

/// Planned checkout.
pub(crate) struct CheckoutPlan {
    /// Working tree targets.
    pub(crate) tx: TxPlan,
    /// New index.
    pub(crate) index: IndexUpdate,
    /// Paths written.
    pub(crate) written: Vec<BString>,
    /// Paths removed.
    pub(crate) removed: Vec<BString>,
}

#[derive(Clone, Copy)]
struct Side<'a> {
    id: ObjectId,
    mode: Mode,
    entry: &'a gix::index::Entry,
}

fn side<'a>(state: &'a State, path: &BStr) -> Option<Side<'a>> {
    merged_entry(state, path).map(|entry| Side {
        id: entry.id,
        mode: entry.mode,
        entry,
    })
}

fn same(a: Option<Side<'_>>, b: Option<Side<'_>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.id == b.id && a.mode == b.mode,
        (None, None) => true,
        _ => false,
    }
}

enum Decision {
    Keep,
    Apply,
    Dirty,
}

struct Planner<'w, 'r> {
    wt: &'w mut Worktree<'r>,
    tx: TxPlan,
    fresh: HashSet<BString>,
    written: Vec<BString>,
    removed: Vec<BString>,
    dirty: Vec<BString>,
    target_dirs: BTreeSet<BString>,
    target_files: BTreeSet<BString>,
}

fn parent_dirs(path: &BStr) -> impl Iterator<Item = &BStr> {
    path.rfind_iter(b"/").map(move |slash| path[..slash].as_bstr())
}

impl Planner<'_, '_> {
    fn is_clean(&mut self, path: &BStr, index: Option<Side<'_>>) -> Result<bool> {
        match index {
            Some(side) => self.wt.matches(path, side.entry),
            None => Ok(matches!(self.wt.probe(path)?, Probe::Absent)),
        }
    }

    fn decide(&mut self, co: &Checkout<'_>, path: &BStr) -> Result<Decision> {
        let index = side(co.current, path);
        let target = side(co.target, path);
        let unmerged = co.current.entry_index_by_path(path).is_ok() && index.is_none();
        match co.policy {
            Policy::Hard => {
                let keep = same(index, target)
                    && match index {
                        Some(side) => self.wt.matches(path, side.entry)?,
                        None => true,
                    };
                Ok(if keep { Decision::Keep } else { Decision::Apply })
            }
            Policy::Merge => {
                if unmerged {
                    Ok(Decision::Apply)
                } else if same(index, target) {
                    Ok(Decision::Keep)
                } else if self.is_clean(path, index)? {
                    Ok(Decision::Apply)
                } else {
                    Ok(Decision::Dirty)
                }
            }
            Policy::Keep => {
                let head = co.head.and_then(|head| side(head, path));
                if same(head, target) {
                    Ok(Decision::Keep)
                } else if !unmerged && same(index, head) && self.is_clean(path, index)? {
                    Ok(Decision::Apply)
                } else {
                    Ok(Decision::Dirty)
                }
            }
        }
    }

    fn keep(&mut self, co: &Checkout<'_>, path: &BStr) -> Result<()> {
        if let Some(target) = side(co.target, path) {
            let unchanged = side(co.current, path).is_some_and(|index| same(Some(index), Some(target)));
            if !unchanged && self.wt.matches(path, target.entry)? {
                self.fresh.insert(path.to_owned());
            }
        }
        Ok(())
    }

    fn apply(&mut self, co: &Checkout<'_>, path: &BStr) -> Result<()> {
        let probe = self.wt.probe(path)?;
        match side(co.target, path) {
            Some(target) => {
                if probe == Probe::Dir && co.policy != Policy::Hard {
                    self.dirty.push(path.to_owned());
                    return Ok(());
                }
                self.wt.plan_write(&mut self.tx, path, target.id, target.mode, &probe)?;
                self.fresh.insert(path.to_owned());
                self.written.push(path.to_owned());
            }
            None => {
                let covered = self.target_dirs.contains(path)
                    || parent_dirs(path).any(|dir| self.target_files.contains(dir));
                if matches!(probe, Probe::File { .. } | Probe::Link(_)) && !covered {
                    let file = self.wt.repo().worktree_path(path);
                    self.tx.remove(&file, &probe.before())?;
                    self.removed.push(path.to_owned());
                }
            }
        }
        Ok(())
    }

    fn prune_empty_dirs(&mut self) -> Result<()> {
        let root = self.wt.repo().workdir.clone();
        let mut gone: BTreeSet<PathBuf> = self
            .removed
            .iter()
            .map(|path| self.wt.repo().worktree_path(path.as_bstr()))
            .collect();
        let mut candidates: BTreeMap<(usize, PathBuf), ()> = BTreeMap::new();
        for path in &self.removed {
            for dir in parent_dirs(path.as_bstr()) {
                if self.target_dirs.contains(dir) {
                    break;
                }
                let abs = self.wt.repo().worktree_path(dir);
                candidates.insert((usize::MAX - abs.components().count(), abs), ());
            }
        }
        for (_, dir) in candidates.into_keys() {
            if dir == root || !dir.is_dir() {
                continue;
            }
            let empty_after = std::fs::read_dir(&dir)
                .map_err(|cause| io_error("cannot read directory", cause))?
                .all(|child| child.is_ok_and(|child| gone.contains(&child.path())));
            if empty_after {
                self.tx.remove(&dir, &Before::Other)?;
                gone.insert(dir);
            }
        }
        Ok(())
    }
}

/// Paths of `state` in `scope`.
fn paths_of(state: &State, scope: Option<&dyn Fn(&BStr) -> bool>) -> BTreeSet<BString> {
    state
        .entries()
        .iter()
        .map(|entry| entry.path(state).to_owned())
        .filter(|path| scope.is_none_or(|scope| scope(path.as_bstr())))
        .collect()
}

/// New index: entries of `target` in scope, entries of `current` outside.
fn combine(current: &State, target: &State, scope: Option<&dyn Fn(&BStr) -> bool>) -> State {
    let Some(scope) = scope else {
        return target.clone();
    };
    let mut state = State::new(target.object_hash());
    for entry in current.entries() {
        let path = entry.path(current);
        if !scope(path) {
            state.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, path);
        }
    }
    for entry in target.entries() {
        let path = entry.path(target);
        if scope(path) {
            state.dangerously_push_entry(entry.stat, entry.id, entry.flags, entry.mode, path);
        }
    }
    state.sort_entries();
    state
}

/// Plans the checkout of `co.target` into the index and the working tree.
///
/// With [`Policy::Merge`] and [`Policy::Keep`] local changes that would be
/// lost fail with `E_GIT_DIRTY` before anything is planned.
pub(crate) fn plan_checkout(wt: &mut Worktree<'_>, co: &Checkout<'_>) -> Result<CheckoutPlan> {
    let mut paths = paths_of(co.current, co.scope);
    paths.extend(paths_of(co.target, co.scope));
    if let Some(head) = co.head {
        paths.extend(paths_of(head, co.scope));
    }
    let target_files = paths_of(co.target, co.scope);
    let target_dirs = target_files
        .iter()
        .flat_map(|path| parent_dirs(path.as_bstr()).map(ToOwned::to_owned).collect::<Vec<_>>())
        .collect();
    let mut planner = Planner {
        wt,
        tx: TxPlan::new(),
        fresh: HashSet::new(),
        written: Vec::new(),
        removed: Vec::new(),
        dirty: Vec::new(),
        target_dirs,
        target_files,
    };
    let mut keeps = Vec::new();
    let mut removals = Vec::new();
    let mut writes = Vec::new();
    for path in &paths {
        match planner.decide(co, path.as_bstr())? {
            Decision::Dirty => planner.dirty.push(path.clone()),
            Decision::Keep => keeps.push(path),
            Decision::Apply if side(co.target, path.as_bstr()).is_none() => removals.push(path),
            Decision::Apply => writes.push(path),
        }
    }
    if !planner.dirty.is_empty() {
        return Err(dirty_error(planner.wt.repo(), &planner.dirty));
    }
    for path in keeps {
        planner.keep(co, path.as_bstr())?;
    }
    for path in removals {
        planner.apply(co, path.as_bstr())?;
    }
    for path in co.extra_removals {
        let probe = planner.wt.probe(path.as_bstr())?;
        if matches!(probe, Probe::File { .. } | Probe::Link(_)) {
            let file = planner.wt.repo().worktree_path(path.as_bstr());
            planner.tx.remove(&file, &probe.before())?;
            planner.removed.push(path.clone());
        }
    }
    planner.prune_empty_dirs()?;
    for path in writes {
        planner.apply(co, path.as_bstr())?;
    }
    if !planner.dirty.is_empty() {
        return Err(dirty_error(planner.wt.repo(), &planner.dirty));
    }
    let state = combine(co.current, co.target, co.scope);
    let fresh = std::mem::take(&mut planner.fresh);
    let index = IndexUpdate::new(planner.wt.repo(), state, co.current, &mut |path, _| {
        Ok(fresh.contains(path))
    })?;
    Ok(CheckoutPlan {
        tx: planner.tx,
        index,
        written: planner.written,
        removed: planner.removed,
    })
}

/// `E_GIT_DIRTY` listing the paths whose local changes would be lost.
pub(crate) fn dirty_error(repo: &Repo, paths: &[BString]) -> nurt_contracts::error::Error {
    let mut sorted: Vec<String> = paths.iter().map(|path| path.to_str_lossy().into_owned()).collect();
    sorted.sort();
    sorted.dedup();
    let root = quote(&repo.workdir.to_string_lossy());
    let mut error = error(
        ErrorCode::GitDirty,
        format!(
            "local changes to {} path(s) would be overwritten; save them first",
            sorted.len()
        ),
    )
    .with_problem("local_changes")
    .with_fix(format!("git.stash {root} action: push"))
    .with_detail("paths", serde_json::Value::from(sorted.clone()));
    for path in sorted {
        error = error.with_sample(path);
    }
    error
}

/// Plans a one-way update of the index only: the index becomes `target`
/// with stat data of unchanged entries kept and fresh stat data for entries
/// the working tree already holds.
pub(crate) fn plan_index_only(wt: &mut Worktree<'_>, current: &State, target: State) -> Result<IndexUpdate> {
    IndexUpdate::new(wt.repo(), target, current, &mut |path, entry| wt.matches(path, entry))
}
