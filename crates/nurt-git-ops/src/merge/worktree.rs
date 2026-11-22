// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Update of the working tree and the index from one tree to another, with
//! the checks Git makes before it overwrites anything.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix_hash::ObjectId;
use gix_index::entry::{Mode, Stage};
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::repo::{Filters, Repo, blob_id, path_string};
use super::trees::io_error;

/// Unix permission bits of a regular file written by Git.
const MODE_FILE: u32 = 0o644;
/// Unix permission bits of an executable file written by Git.
const MODE_EXECUTABLE: u32 = 0o755;

/// How the update treats the index and local changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateKind {
    /// Two-way update of `git merge --ff` and `git checkout`: a path changed
    /// between the trees must be unchanged in the index and the working
    /// tree, unless the index already holds the target.
    TwoWay,
    /// Update of `git reset --merge`: unmerged paths take the target, staged
    /// changes are replaced when the working tree matches the index.
    ResetMerge,
}

/// Entry of a tree or the index: object and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) id: ObjectId,
    pub(crate) mode: Mode,
}

pub(crate) type Items = BTreeMap<BString, Item>;

/// Stage 0 entries of `state` by path.
pub(crate) fn items(state: &gix_index::State) -> Items {
    state
        .entries()
        .iter()
        .filter(|entry| entry.stage() == Stage::Unconflicted)
        .map(|entry| {
            (
                entry.path(state).to_owned(),
                Item {
                    id: entry.id,
                    mode: entry.mode,
                },
            )
        })
        .collect()
}

/// Paths of `state` with conflict stages.
pub(crate) fn unmerged(state: &gix_index::State) -> BTreeSet<BString> {
    state
        .entries()
        .iter()
        .filter(|entry| entry.stage() != Stage::Unconflicted)
        .map(|entry| entry.path(state).to_owned())
        .collect()
}

/// A change of one working tree entry, relative to the working tree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Op {
    RemoveFile(BString),
    RemoveDir(BString),
    CreateDir(BString),
    Write {
        path: BString,
        data: Vec<u8>,
        mode: u32,
        existed: Existing,
    },
    Link {
        path: BString,
        target: PathBuf,
        existed: Existing,
    },
}

/// What is at the path of a write before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Existing {
    Nothing,
    File { mode: u32 },
    Link,
}

/// Paths that block the update.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Blocked {
    pub(crate) staged: Vec<String>,
    pub(crate) modified: Vec<String>,
    pub(crate) untracked: Vec<String>,
}

impl Blocked {
    pub(crate) fn is_empty(&self) -> bool {
        self.staged.is_empty() && self.modified.is_empty() && self.untracked.is_empty()
    }

    /// `E_GIT_DIRTY` listing every blocking path, as `git merge` refuses.
    pub(crate) fn into_error(self, repo: &str) -> Error {
        let kind = if !self.staged.is_empty() {
            "staged"
        } else if !self.modified.is_empty() {
            "modified"
        } else {
            "untracked"
        };
        let mut paths: Vec<String> = self.staged.into_iter().chain(self.modified).collect();
        let local = paths.len();
        paths.extend(self.untracked);
        let message = if local > 0 {
            "your local changes to the following files would be overwritten by merge; commit or stash them first"
        } else {
            "untracked working tree files would be overwritten by merge; move or remove them first"
        };
        let mut error = Error::new(ErrorCode::GitDirty, message)
            .with_problem(kind)
            .with_detail("kind", serde_json::Value::from(kind))
            .with_detail("paths", serde_json::Value::from(paths.clone()));
        for path in paths {
            error = error.with_sample(path);
        }
        error
            .with_fix(format!("git.stash {repo} action: push include_untracked: true"))
            .with_next_step(format!("git.status {repo}"))
    }
}

/// Planned update of the working tree.
#[derive(Debug, Default, Clone)]
pub(crate) struct Update {
    pub(crate) ops: Vec<Op>,
    /// Paths whose index entry takes the target without a write, because
    /// the working tree already matches it.
    pub(crate) adopted: Vec<BString>,
}

impl Update {
    pub(crate) fn written(&self) -> impl Iterator<Item = &BString> {
        self.ops.iter().filter_map(|op| match op {
            Op::Write { path, .. } | Op::Link { path, .. } => Some(path),
            _ => None,
        })
    }
}

pub(crate) struct Planner<'a, 'r> {
    repo: &'a Repo,
    filters: Filters<'r>,
    index: &'a gix_index::State,
    file_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Keep,
    Take,
}

impl<'a, 'r> Planner<'a, 'r>
where
    'a: 'r,
{
    pub(crate) fn new(repo: &'a Repo, index: &'a gix_index::State) -> Result<Self> {
        Ok(Self {
            repo,
            filters: Filters::new(repo, index)?,
            index,
            file_mode: repo.file_mode_enabled(),
        })
    }

    /// Plans the update of the working tree from `from` to `to` (stage 0
    /// entries of trees) over the current index.
    pub(crate) fn plan(&mut self, from: &Items, to: &Items, kind: UpdateKind) -> Result<(Update, Blocked)> {
        let current = items(self.index);
        let conflicted = unmerged(self.index);
        let mut paths: BTreeSet<&BString> = from.keys().chain(to.keys()).collect();
        if kind == UpdateKind::ResetMerge {
            paths.extend(current.keys());
            paths.extend(conflicted.iter());
        }
        let mut blocked = Blocked::default();
        let mut takes: Vec<(BString, Option<Item>)> = Vec::new();
        let mut adopted = Vec::new();
        for path in paths {
            let (h, m, i) = (from.get(path), to.get(path), current.get(path));
            let decision = match kind {
                UpdateKind::TwoWay => self.two_way(path, h, m, i, &mut blocked)?,
                UpdateKind::ResetMerge => self.reset_merge(path, m, i, conflicted.contains(path), &mut blocked)?,
            };
            match decision {
                Some(Action::Take) => takes.push((path.clone(), m.copied())),
                Some(Action::Keep) => adopted.push(path.clone()),
                None => {}
            }
        }
        let ops = self.build_ops(&takes, &mut blocked)?;
        Ok((Update { ops, adopted }, blocked))
    }

    fn two_way(
        &mut self,
        path: &BString,
        h: Option<&Item>,
        m: Option<&Item>,
        i: Option<&Item>,
        blocked: &mut Blocked,
    ) -> Result<Option<Action>> {
        if h == m {
            return Ok(None);
        }
        if i != h {
            if i == m {
                return Ok(None);
            }
            blocked.staged.push(path_string(path));
            return Ok(None);
        }
        match i {
            Some(item) => {
                if !self.matches(path, Some(item), true)? {
                    blocked.modified.push(path_string(path));
                    return Ok(None);
                }
                Ok(Some(Action::Take))
            }
            None => self.untracked(path, m, blocked),
        }
    }

    fn reset_merge(
        &mut self,
        path: &BString,
        target: Option<&Item>,
        i: Option<&Item>,
        conflicted: bool,
        blocked: &mut Blocked,
    ) -> Result<Option<Action>> {
        if conflicted {
            return Ok(Some(Action::Take));
        }
        if i == target {
            return Ok(None);
        }
        match i {
            Some(item) => {
                if !self.matches(path, Some(item), true)? {
                    blocked.modified.push(path_string(path));
                    return Ok(None);
                }
                Ok(Some(Action::Take))
            }
            None => self.untracked(path, target, blocked),
        }
    }

    fn untracked(&mut self, path: &BString, target: Option<&Item>, blocked: &mut Blocked) -> Result<Option<Action>> {
        let Some(target) = target else {
            return Ok(None);
        };
        if self.exists(path)? {
            if self.matches(path, Some(target), false)? {
                return Ok(Some(Action::Keep));
            }
            blocked.untracked.push(path_string(path));
            return Ok(None);
        }
        Ok(Some(Action::Take))
    }

    fn full(&self, path: &BStr) -> Result<PathBuf> {
        let rela = path.to_path().map_err(io_error("convert a path"))?;
        Ok(self.repo.workdir.join(rela))
    }

    fn exists(&self, path: &BStr) -> Result<bool> {
        match fs::symlink_metadata(self.full(path)?) {
            Ok(_) => Ok(true),
            Err(error) if is_absent(&error) => Ok(false),
            Err(error) => Err(read_error(path, &error)),
        }
    }

    /// Returns `true` when the working tree entry at `path` has the content
    /// and mode of `expected`; `None` expects no entry.
    pub(crate) fn matches(&mut self, path: &BStr, expected: Option<&Item>, use_stat: bool) -> Result<bool> {
        let full = self.full(path)?;
        let meta = match fs::symlink_metadata(&full) {
            Ok(meta) => meta,
            Err(error) if is_absent(&error) => return Ok(expected.is_none()),
            Err(error) => return Err(read_error(path, &error)),
        };
        let Some(expected) = expected else {
            return Ok(false);
        };
        if meta.is_dir() {
            return Ok(false);
        }
        if use_stat && self.stat_unchanged(path, &full, expected)? {
            return Ok(true);
        }
        if meta.file_type().is_symlink() {
            if expected.mode != Mode::SYMLINK {
                return Ok(false);
            }
            let target = fs::read_link(&full).map_err(|error| read_error(path, &error))?;
            let bytes = target.to_string_lossy().replace('\\', "/").into_bytes();
            return Ok(blob_id(self.repo.hash_kind(), &bytes)? == expected.id);
        }
        if expected.mode == Mode::SYMLINK || expected.mode == Mode::COMMIT {
            return Ok(false);
        }
        if self.file_mode && is_executable(&meta) != (expected.mode == Mode::FILE_EXECUTABLE) {
            return Ok(false);
        }
        let data = fs::read(&full).map_err(|error| read_error(path, &error))?;
        let canonical = self.filters.to_git(&data, path)?;
        Ok(blob_id(self.repo.hash_kind(), &canonical)? == expected.id)
    }

    fn stat_unchanged(&self, path: &BStr, full: &Path, expected: &Item) -> Result<bool> {
        let Some(entry) = self.index.entry_by_path_and_stage(path, Stage::Unconflicted) else {
            return Ok(false);
        };
        if entry.id != expected.id || entry.mode != expected.mode {
            return Ok(false);
        }
        let Ok(meta) = gix_index::fs::Metadata::from_path_no_follow(full) else {
            return Ok(false);
        };
        let Ok(stat) = gix_index::entry::Stat::from_fs(&meta) else {
            return Ok(false);
        };
        let options = self.repo.repo.stat_options().map_err(io_error("read stat settings"))?;
        let racy = entry.stat.mtime.secs >= self.index.timestamp().unix_seconds() as u32;
        Ok(!racy && entry.stat.matches(&stat, options))
    }

    fn build_ops(&mut self, takes: &[(BString, Option<Item>)], blocked: &mut Blocked) -> Result<Vec<Op>> {
        let removed: BTreeSet<&BString> = takes
            .iter()
            .filter(|(path, target)| target.is_none() || self.is_file_at(path))
            .map(|(path, _)| path)
            .collect();
        let mut ops: Vec<Op> = Vec::new();
        for path in &removed {
            if self.exists(path.as_bstr())? && takes.iter().any(|(p, t)| p == *path && t.is_none()) {
                ops.push(Op::RemoveFile((*path).clone()));
            }
        }
        let writes: Vec<(&BString, Item)> = takes
            .iter()
            .filter_map(|(path, target)| target.map(|item| (path, item)))
            .collect();
        let dirs_removed = self.empty_dirs_after(&ops, &writes)?;
        ops.extend(dirs_removed.iter().map(|dir| Op::RemoveDir(dir.clone())));
        let created = self.dirs_to_create(&writes, &ops, blocked)?;
        ops.extend(created.into_iter().map(Op::CreateDir));
        for (path, item) in writes {
            if let Some(op) = self.write_op(path, item, &dirs_removed, blocked)? {
                ops.push(op);
            }
        }
        Ok(ops)
    }

    fn is_file_at(&self, path: &BStr) -> bool {
        self.full(path)
            .ok()
            .and_then(|full| fs::symlink_metadata(full).ok())
            .is_some_and(|meta| !meta.is_dir())
    }

    fn empty_dirs_after(&self, removals: &[Op], writes: &[(&BString, Item)]) -> Result<Vec<BString>> {
        let removed: BTreeSet<BString> = removals
            .iter()
            .filter_map(|op| match op {
                Op::RemoveFile(path) => Some(path.clone()),
                _ => None,
            })
            .collect();
        let mut candidates: BTreeSet<BString> = BTreeSet::new();
        for path in &removed {
            let mut current = parent(path.as_bstr());
            while let Some(dir) = current {
                candidates.insert(dir.to_owned());
                current = parent(dir);
            }
        }
        for (path, _) in writes {
            if self.full(path.as_bstr())?.is_dir() {
                candidates.insert((*path).clone());
            }
        }
        let mut ordered: Vec<BString> = candidates.into_iter().collect();
        ordered.sort_by_key(|dir| std::cmp::Reverse(dir.iter().filter(|b| **b == b'/').count()));
        let mut gone: BTreeSet<BString> = removed;
        let mut out = Vec::new();
        for dir in ordered {
            let needed = writes.iter().any(|(path, _)| path.starts_with(&dir) && path.get(dir.len()) == Some(&b'/'));
            if needed || !self.only_contains(&dir, &gone)? {
                continue;
            }
            gone.insert(dir.clone());
            out.push(dir);
        }
        Ok(out)
    }

    fn only_contains(&self, dir: &BString, gone: &BTreeSet<BString>) -> Result<bool> {
        let full = self.full(dir.as_bstr())?;
        let entries = match fs::read_dir(&full) {
            Ok(entries) => entries,
            Err(error) if is_absent(&error) => return Ok(false),
            Err(error) => return Err(read_error(dir.as_bstr(), &error)),
        };
        for entry in entries {
            let entry = entry.map_err(|error| read_error(dir.as_bstr(), &error))?;
            let mut child = dir.clone();
            child.push(b'/');
            child.extend_from_slice(entry.file_name().to_string_lossy().as_bytes());
            if !gone.contains(&child) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn dirs_to_create(&self, writes: &[(&BString, Item)], ops: &[Op], blocked: &mut Blocked) -> Result<Vec<BString>> {
        let removed: BTreeSet<&BString> = ops
            .iter()
            .filter_map(|op| match op {
                Op::RemoveFile(path) | Op::RemoveDir(path) => Some(path),
                _ => None,
            })
            .collect();
        let mut out: BTreeSet<BString> = BTreeSet::new();
        for (path, _) in writes {
            let mut ancestors = Vec::new();
            let mut current = parent(path.as_bstr());
            while let Some(dir) = current {
                ancestors.push(dir.to_owned());
                current = parent(dir);
            }
            for dir in ancestors.into_iter().rev() {
                let full = self.full(dir.as_bstr())?;
                let meta = fs::symlink_metadata(&full);
                let present_dir = meta.as_ref().is_ok_and(fs::Metadata::is_dir);
                let in_the_way = meta.is_ok() && !present_dir && !removed.contains(&dir);
                if in_the_way {
                    blocked.untracked.push(path_string(&dir));
                    break;
                }
                if !present_dir || removed.contains(&dir) {
                    out.insert(dir);
                }
            }
        }
        let mut ordered: Vec<BString> = out.into_iter().collect();
        ordered.sort_by_key(|dir| dir.iter().filter(|b| **b == b'/').count());
        Ok(ordered)
    }

    fn write_op(
        &mut self,
        path: &BString,
        item: Item,
        dirs_removed: &[BString],
        blocked: &mut Blocked,
    ) -> Result<Option<Op>> {
        let full = self.full(path.as_bstr())?;
        let existed = match fs::symlink_metadata(&full) {
            Ok(meta) if meta.is_dir() => {
                if !dirs_removed.contains(path) {
                    blocked.untracked.push(path_string(path));
                    return Ok(None);
                }
                Existing::Nothing
            }
            Ok(meta) if meta.file_type().is_symlink() => Existing::Link,
            Ok(meta) => Existing::File {
                mode: permissions(&meta),
            },
            Err(error) if is_absent(&error) => Existing::Nothing,
            Err(error) => return Err(read_error(path.as_bstr(), &error)),
        };
        let data = self.repo.blob(item.id)?;
        if item.mode == Mode::SYMLINK {
            let target = PathBuf::from(data.to_str_lossy().into_owned());
            return Ok(Some(Op::Link {
                path: path.clone(),
                target,
                existed,
            }));
        }
        if item.mode == Mode::COMMIT {
            return Ok(Some(Op::CreateDir(path.clone())));
        }
        let data = self.filters.to_worktree(&data, path.as_bstr())?;
        let mode = if item.mode == Mode::FILE_EXECUTABLE {
            MODE_EXECUTABLE
        } else {
            MODE_FILE
        };
        Ok(Some(Op::Write {
            path: path.clone(),
            data,
            mode,
            existed,
        }))
    }
}

fn parent(path: &BStr) -> Option<&BStr> {
    path.rfind_byte(b'/').map(|at| path[..at].as_bstr())
}

fn is_absent(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory)
}

fn read_error(path: &BStr, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("cannot read {path} in the working tree: {error}"))
}

#[cfg(unix)]
fn permissions(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn permissions(_meta: &fs::Metadata) -> u32 {
    MODE_FILE
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

/// Index after the update: `base` with the file system fields of every
/// entry taken from the working tree when the entry was written or adopted,
/// and from `previous` when its object and mode are unchanged.
pub(crate) fn refresh_stat(
    repo: &Repo,
    mut base: gix_index::State,
    previous: &gix_index::State,
    touched: &BTreeSet<BString>,
) -> Result<gix_index::State> {
    let old = previous_stats(previous);
    let workdir = repo.workdir.clone();
    let (entries, backing) = base.entries_mut_and_pathbacking();
    for entry in entries.iter_mut() {
        if entry.stage() != Stage::Unconflicted {
            continue;
        }
        let path = entry.path_in(backing).to_owned();
        if !touched.contains(&path)
            && let Some((id, mode, stat)) = old.get(&path)
            && *id == entry.id
            && *mode == entry.mode
        {
            entry.stat = *stat;
            continue;
        }
        let rela = path.to_path().map_err(io_error("convert a path"))?;
        if let Ok(meta) = gix_index::fs::Metadata::from_path_no_follow(&workdir.join(rela))
            && let Ok(stat) = gix_index::entry::Stat::from_fs(&meta)
        {
            entry.stat = stat;
        }
    }
    base.remove_tree();
    base.remove_resolve_undo();
    Ok(base)
}

fn previous_stats(previous: &gix_index::State) -> BTreeMap<BString, (ObjectId, Mode, gix_index::entry::Stat)> {
    previous
        .entries()
        .iter()
        .filter(|entry| entry.stage() == Stage::Unconflicted)
        .map(|entry| (entry.path(previous).to_owned(), (entry.id, entry.mode, entry.stat)))
        .collect()
}

/// Bytes of the index file for `state`.
pub(crate) fn index_bytes(state: gix_index::State) -> Result<Vec<u8>> {
    let file = gix_index::File::from_state(state, PathBuf::from("index"));
    let mut out = Vec::new();
    file.write_to(
        &mut out,
        gix_index::write::Options {
            extensions: gix_index::write::Extensions::All,
            skip_hash: false,
        },
    )
    .map_err(io_error("encode the index"))?;
    Ok(out)
}
