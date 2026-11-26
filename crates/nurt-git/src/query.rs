// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Repository information service used by other command groups.
//!
//! [`RepoQueries`] answers the read-only repository queries of the contract
//! (`repo_root`, `status`, `changed_files`, `tracked_files`, `is_ignored`,
//! `file_history`) on [`GitRepo`]. Paths are relative to the root of the
//! working tree, with `/` separators. Content of modified-looking files is
//! compared after the content filters of the repository, so line ending
//! conversions do not count as changes, as in `git status`. Renames are not
//! detected: a renamed file is a deletion and an addition.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::env::{Clock, Host, WallTime};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{FileHistoryStat, GitChange, GitPathStatus, GitQueryService};
use nurt_contracts::stage::ExecCtx;

use crate::repo::filter::Filters;
use crate::repo::index::expand_sparse;
use crate::repo::pathspec::{PathFilter, relative_path};
use crate::repo::{GitRepo, error};

/// Seconds in a day, for the period of [`GitQueryService::file_history`].
const SECONDS_PER_DAY: i64 = 86_400;
/// Nanoseconds in a second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;
/// Mode of a tree entry that is a submodule commit.
const MODE_COMMIT: u32 = 0o160_000;
/// Mode of a symbolic link.
const MODE_LINK: u32 = 0o120_000;
/// Mode of an executable file.
const MODE_EXECUTABLE: u32 = 0o100_755;
/// Name of the repository directory, skipped by walks of the working tree.
const DOT_GIT: &str = ".git";

/// Read-only repository queries for commands outside the `git` group.
///
/// Every query opens the repository containing the given path with the
/// clock and host of the console, so the service holds no repository
/// state between calls.
#[derive(Clone)]
pub struct RepoQueries {
    clock: Arc<dyn Clock>,
    host: Arc<dyn Host>,
}

impl std::fmt::Debug for RepoQueries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoQueries").finish_non_exhaustive()
    }
}

impl RepoQueries {
    /// Service that reads the environment of `host` and the time of
    /// `clock`.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, host: Arc<dyn Host>) -> Self {
        Self { clock, host }
    }

    fn ctx(&self) -> ExecCtx {
        ExecCtx::builder(Arc::clone(&self.clock), Arc::clone(&self.host)).build()
    }

    fn open(&self, root: &Path) -> Result<GitRepo> {
        GitRepo::discover(root, &self.ctx())
    }
}

/// State of one path: change in the index and change in the working tree.
type PathChanges = BTreeMap<BString, (GitChange, GitChange)>;

impl GitQueryService for RepoQueries {
    fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
        match self.open(path) {
            Ok(repo) => Ok(Some(repo.root().to_path_buf())),
            Err(failure)
                if failure.code == ErrorCode::GitNotRepository
                    && failure.problem.as_deref() == Some("not_repository") =>
            {
                Ok(None)
            }
            Err(failure) => Err(failure),
        }
    }

    fn status(&self, root: &Path, paths: &[String]) -> Result<Vec<GitPathStatus>> {
        let repo = self.open(root)?;
        let filter = PathFilter::new(repo.root(), paths, &[])?;
        let changes = changes(&repo, None, &filter)?;
        let mut statuses: Vec<GitPathStatus> = changes
            .iter()
            .map(|(path, (index, worktree))| path_status(path.as_bstr(), *index, *worktree))
            .collect();
        if !paths.is_empty() {
            let tracked = tracked_paths(&repo)?;
            for literal in filter.literal_paths() {
                let listed = changes.keys().any(|path| is_below(path.as_bstr(), literal));
                if !listed && tracked.contains(literal) {
                    statuses.push(path_status(
                        literal,
                        GitChange::Unmodified,
                        GitChange::Unmodified,
                    ));
                }
            }
            statuses.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Ok(statuses)
    }

    fn changed_files(&self, root: &Path, against: Option<&str>) -> Result<Vec<String>> {
        let repo = self.open(root)?;
        let changes = changes(&repo, against, &PathFilter::all())?;
        Ok(changes
            .into_keys()
            .map(|path| path.to_str_lossy().into_owned())
            .collect())
    }

    fn tracked_files(&self, root: &Path) -> Result<Vec<String>> {
        let repo = self.open(root)?;
        Ok(tracked_paths(&repo)?
            .into_iter()
            .map(|path| path.to_str_lossy().into_owned())
            .collect())
    }

    fn is_ignored(&self, root: &Path, path: &str) -> Result<bool> {
        let repo = self.open(root)?;
        let relative = relative_path(repo.root(), path)?;
        if relative.is_empty() {
            return Ok(false);
        }
        let index = repo.index_file()?;
        let mut excludes = excludes(&repo, &index)?;
        let mut end = 0;
        while let Some(offset) = relative[end..].find_byte(b'/') {
            let dir = relative[..end + offset].as_bstr();
            if is_excluded(&mut excludes, dir, true)? {
                return Ok(true);
            }
            end += offset + 1;
        }
        let is_dir = worktree_file(repo.root(), relative.as_bstr()).is_dir();
        is_excluded(&mut excludes, relative.as_bstr(), is_dir)
    }

    fn file_history(
        &self,
        root: &Path,
        since_days: u32,
        limit: u32,
    ) -> Result<Vec<FileHistoryStat>> {
        let repo = self.open(root)?;
        let now = self.clock.now().unix_nanos() / NANOS_PER_SECOND;
        let now = i64::try_from(now).unwrap_or(i64::MAX);
        file_history(&repo, now - i64::from(since_days) * SECONDS_PER_DAY, limit)
    }
}

fn path_status(path: &BStr, index: GitChange, worktree: GitChange) -> GitPathStatus {
    GitPathStatus {
        path: path.to_str_lossy().into_owned(),
        index,
        worktree,
        original_path: None,
    }
}

fn is_below(path: &BStr, ancestor: &BStr) -> bool {
    path == ancestor
        || (path.starts_with(ancestor.as_bytes()) && path.get(ancestor.len()) == Some(&b'/'))
}

fn join(prefix: &BStr, name: &BStr) -> BString {
    if prefix.is_empty() {
        return name.to_owned();
    }
    let mut path = prefix.to_owned();
    path.push(b'/');
    path.extend_from_slice(name);
    path
}

/// Paths of the index, each once, including paths with conflict stages.
fn tracked_paths(repo: &GitRepo) -> Result<BTreeSet<BString>> {
    let index = repo.index_file()?;
    Ok(index
        .entries()
        .iter()
        .map(|entry| entry.path(&index).to_owned())
        .collect())
}

/// Mode and object of a file of a tree or of the index.
type Blob = (u32, gix::ObjectId);

/// Files of the tree `tree`, keyed by path.
fn tree_files(repo: &GitRepo, tree: Option<gix::ObjectId>) -> Result<BTreeMap<BString, Blob>> {
    let mut files = BTreeMap::new();
    if let Some(tree) = tree {
        collect_tree(repo, tree, b"".as_bstr(), &mut files)?;
    }
    Ok(files)
}

fn collect_tree(
    repo: &GitRepo,
    tree: gix::ObjectId,
    prefix: &BStr,
    out: &mut BTreeMap<BString, Blob>,
) -> Result<()> {
    for (name, (is_tree, mode, id)) in tree_entries(repo, Some(tree))? {
        let path = join(prefix, name.as_bstr());
        if is_tree {
            collect_tree(repo, id, path.as_bstr(), out)?;
        } else {
            out.insert(path, (mode, id));
        }
    }
    Ok(())
}

/// Entries of one tree: name, whether the entry is a tree, mode and object.
fn tree_entries(
    repo: &GitRepo,
    tree: Option<gix::ObjectId>,
) -> Result<BTreeMap<BString, (bool, u32, gix::ObjectId)>> {
    let mut entries = BTreeMap::new();
    let Some(tree) = tree else {
        return Ok(entries);
    };
    let read_error = |failure: gix::Error| error::from_gix("read a tree", &failure);
    let object = repo.gix().find_tree(tree).map_err(read_error)?;
    let decoded = object.decode().map_err(read_error)?;
    for item in &decoded.entries {
        entries.insert(
            item.filename.to_owned(),
            (
                item.mode.is_tree(),
                u32::from(item.mode.value()),
                item.oid.to_owned(),
            ),
        );
    }
    Ok(entries)
}

/// Tree of the revision `spec`, or of `HEAD` when `spec` is `None`; `None`
/// on an unborn branch.
fn base_tree(repo: &GitRepo, spec: Option<&str>) -> Result<Option<gix::ObjectId>> {
    let commit = match spec {
        Some(spec) => repo.resolve_rev(spec)?,
        None => match repo.head_id()? {
            Some(id) => id,
            None => return Ok(None),
        },
    };
    let object = repo
        .gix()
        .find_object(commit)
        .map_err(|failure| error::from_gix("read an object", &failure))?;
    let tree = object
        .peel_to_tree()
        .map_err(|failure| error::from_gix("read a tree", &failure))?;
    Ok(Some(tree.id))
}

/// Changes of the index against the tree of `against` (or `HEAD`), of the
/// working tree against the index, and untracked files, for the paths
/// selected by `filter`; unmodified paths are left out.
fn changes(repo: &GitRepo, against: Option<&str>, filter: &PathFilter) -> Result<PathChanges> {
    let file = repo.index_file()?;
    let state = expand_sparse(repo, &file)?;
    let tree = tree_files(repo, base_tree(repo, against)?)?;
    let mut staged: BTreeMap<BString, (Blob, bool)> = BTreeMap::new();
    let mut result = PathChanges::new();
    for entry in state.entries() {
        let path = entry.path(&state);
        if !filter.matches(path) {
            continue;
        }
        if entry.stage_raw() != 0 {
            result.insert(
                path.to_owned(),
                (GitChange::Conflicted, GitChange::Conflicted),
            );
            continue;
        }
        let intent = entry
            .flags
            .contains(gix::index::entry::Flags::INTENT_TO_ADD);
        staged.insert(path.to_owned(), ((entry.mode.bits(), entry.id), intent));
    }
    compare_index(&tree, &staged, filter, &mut result);
    if repo.workdir().is_some() {
        compare_worktree(repo, &state, &staged, &mut result)?;
        for path in untracked(repo, &state, filter)? {
            result
                .entry(path)
                .or_insert((GitChange::Unmodified, GitChange::Unmodified))
                .1 = GitChange::Untracked;
        }
    }
    result.retain(|_, (index, worktree)| {
        *index != GitChange::Unmodified || *worktree != GitChange::Unmodified
    });
    Ok(result)
}

fn kind_of(mode: u32) -> u32 {
    match mode {
        MODE_LINK | MODE_COMMIT => mode,
        _ => 0,
    }
}

fn compare_index(
    tree: &BTreeMap<BString, Blob>,
    staged: &BTreeMap<BString, (Blob, bool)>,
    filter: &PathFilter,
    result: &mut PathChanges,
) {
    for (path, ((mode, id), intent)) in staged {
        let change = match tree.get(path) {
            _ if *intent => GitChange::Unmodified,
            None => GitChange::Added,
            Some((tree_mode, tree_id)) if tree_mode == mode && tree_id == id => {
                GitChange::Unmodified
            }
            Some((tree_mode, _)) if kind_of(*tree_mode) != kind_of(*mode) => GitChange::TypeChanged,
            Some(_) => GitChange::Modified,
        };
        result
            .entry(path.clone())
            .or_insert((change, GitChange::Unmodified))
            .0 = change;
    }
    for path in tree.keys() {
        if !staged.contains_key(path)
            && !result.contains_key(path)
            && filter.matches(path.as_bstr())
        {
            result.insert(path.clone(), (GitChange::Deleted, GitChange::Unmodified));
        }
    }
}

fn compare_worktree(
    repo: &GitRepo,
    state: &gix::index::State,
    staged: &BTreeMap<BString, (Blob, bool)>,
    result: &mut PathChanges,
) -> Result<()> {
    let root = repo.root();
    let racy_after = modified_seconds(&std::fs::metadata(repo.index_path()).ok());
    let file_mode = repo.config_bool("core.fileMode").unwrap_or(cfg!(unix));
    let mut filters: Option<Filters<'_>> = None;
    for (path, ((mode, id), intent)) in staged {
        if *mode == MODE_COMMIT {
            continue;
        }
        let full = worktree_file(root, path.as_bstr());
        let meta = std::fs::symlink_metadata(&full).ok();
        let change = match &meta {
            None => GitChange::Deleted,
            Some(_) if *intent => GitChange::Added,
            Some(meta) if meta.is_dir() => GitChange::Deleted,
            Some(meta) if meta.file_type().is_symlink() != (*mode == MODE_LINK) => {
                GitChange::TypeChanged
            }
            Some(meta) => {
                let entry = state.entry_by_path(path.as_bstr());
                let stat_clean = entry.is_some_and(|entry| stat_matches(entry, meta, racy_after));
                let mode_changed = file_mode
                    && *mode != MODE_LINK
                    && is_executable(meta) != (*mode == MODE_EXECUTABLE);
                if mode_changed {
                    GitChange::Modified
                } else if stat_clean {
                    GitChange::Unmodified
                } else {
                    let filters = match &mut filters {
                        Some(filters) => filters,
                        None => filters.insert(repo.filters()?),
                    };
                    content_change(repo, filters, path.as_bstr(), &full, meta, *id)?
                }
            }
        };
        if change != GitChange::Unmodified {
            result
                .entry(path.clone())
                .or_insert((GitChange::Unmodified, change))
                .1 = change;
        }
    }
    Ok(())
}

/// Path of the file `path` of the working tree `root`, with the bytes of the
/// name kept as they are on systems that allow it.
fn worktree_file(root: &Path, path: &BStr) -> PathBuf {
    match gix_path::from_bstr(path) {
        Ok(native) => root.join(&*native),
        Err(_) => root.join(path.to_str_lossy().as_ref()),
    }
}

fn modified_seconds(meta: &Option<std::fs::Metadata>) -> Option<u64> {
    meta.as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
}

/// Whether the cached file information of `entry` still describes the file;
/// a file changed in the same second as the index was written or later is
/// never trusted, as in `git`.
fn stat_matches(
    entry: &gix::index::Entry,
    meta: &std::fs::Metadata,
    racy_after: Option<u64>,
) -> bool {
    let Some(modified) = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
    else {
        return false;
    };
    if racy_after.is_none_or(|index_time| modified.as_secs() >= index_time) {
        return false;
    }
    u32::try_from(meta.len()).ok() == Some(entry.stat.size)
        && u32::try_from(modified.as_secs()).ok() == Some(entry.stat.mtime.secs)
        && modified.subsec_nanos() == entry.stat.mtime.nsecs
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    const EXECUTE_BITS: u32 = 0o111;
    meta.is_file() && meta.permissions().mode() & EXECUTE_BITS != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Content of a symbolic link as Git stores it: the bytes of its target, with
/// `/` separators on Windows only.
fn link_content(target: &Path) -> Result<Vec<u8>> {
    let bytes = gix::path::into_bstr(target)
        .map_err(|failure| error::from_gix("read a link target", &failure))?;
    Ok(gix::path::to_unix_separators_on_windows(bytes)
        .into_owned()
        .into())
}

fn content_change(
    repo: &GitRepo,
    filters: &mut Filters<'_>,
    path: &BStr,
    full: &Path,
    meta: &std::fs::Metadata,
    expected: gix::ObjectId,
) -> Result<GitChange> {
    let content = if meta.file_type().is_symlink() {
        let target = std::fs::read_link(full).map_err(|failure| error::io(full, &failure))?;
        link_content(&target)?
    } else {
        let bytes = std::fs::read(full).map_err(|failure| error::io(full, &failure))?;
        filters.to_git(path, &bytes)?
    };
    let id = gix_object::compute_hash(repo.gix().object_hash(), gix::object::Kind::Blob, &content)
        .map_err(|failure| {
            Error::new(
                ErrorCode::Internal,
                format!("hashing {path} failed: {failure}"),
            )
        })?;
    Ok(if id == expected {
        GitChange::Unmodified
    } else {
        GitChange::Modified
    })
}

fn excludes<'repo>(
    repo: &'repo GitRepo,
    index: &gix::index::State,
) -> Result<gix::AttributeStack<'repo>> {
    repo.gix()
        .excludes(
            index,
            None,
            gix_worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
        )
        .map_err(|failure| error::from_gix("read the ignore rules", &failure))
}

fn is_excluded(excludes: &mut gix::AttributeStack<'_>, path: &BStr, is_dir: bool) -> Result<bool> {
    let mode = if is_dir {
        gix::index::entry::Mode::DIR
    } else {
        gix::index::entry::Mode::FILE
    };
    let platform = excludes
        .at_entry(path, Some(mode))
        .map_err(|failure| error::from_gix("read the ignore rules", &failure))?;
    Ok(platform.is_excluded())
}

/// Files below the working tree that are neither in the index nor ignored,
/// sorted; directories holding another repository are skipped.
fn untracked(
    repo: &GitRepo,
    state: &gix::index::State,
    filter: &PathFilter,
) -> Result<Vec<BString>> {
    let root = repo.root();
    let tracked: BTreeSet<BString> = state
        .entries()
        .iter()
        .map(|entry| entry.path(state).to_owned())
        .collect();
    let mut excludes = excludes(repo, state)?;
    let mut found = Vec::new();
    let mut pending = vec![BString::default()];
    while let Some(dir) = pending.pop() {
        let full = worktree_file(root, dir.as_bstr());
        let entries = std::fs::read_dir(&full).map_err(|failure| error::io(&full, &failure))?;
        for entry in entries {
            let entry = entry.map_err(|failure| error::io(&full, &failure))?;
            let name = entry.file_name();
            if name == DOT_GIT {
                continue;
            }
            let name = gix::path::into_bstr(Path::new(&name))
                .map_err(|failure| error::from_gix("read a file name", &failure))?;
            let path = join(dir.as_bstr(), name.as_ref());
            let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if tracked.contains(&path) {
                continue;
            }
            if is_dir {
                if descend(repo, &mut excludes, &tracked, filter, &path)? {
                    pending.push(path);
                }
            } else if filter.matches(path.as_bstr())
                && !is_excluded(&mut excludes, path.as_bstr(), false)?
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn descend(
    repo: &GitRepo,
    excludes: &mut gix::AttributeStack<'_>,
    tracked: &BTreeSet<BString>,
    filter: &PathFilter,
    dir: &BString,
) -> Result<bool> {
    if !filter.may_contain(dir.as_bstr()) {
        return Ok(false);
    }
    let mut prefix = dir.clone();
    prefix.push(b'/');
    if tracked
        .range(prefix.clone()..)
        .next()
        .is_some_and(|path| path.starts_with(&prefix))
    {
        return Ok(true);
    }
    if worktree_file(repo.root(), dir.as_bstr())
        .join(DOT_GIT)
        .exists()
    {
        return Ok(false);
    }
    Ok(!is_excluded(excludes, dir.as_bstr(), true)?)
}

/// Statistics of one file over the walked commits.
#[derive(Default)]
struct History {
    commits: u32,
    authors: HashSet<String>,
    last_change: i64,
}

fn file_history(repo: &GitRepo, cutoff: i64, limit: u32) -> Result<Vec<FileHistoryStat>> {
    let Some(head) = repo.head_id()? else {
        return Ok(Vec::new());
    };
    let walk_error = |failure: gix::Error| error::from_gix("walk the history", &failure);
    let walk = repo
        .gix()
        .rev_walk([head])
        .sorting(gix::revision::walk::Sorting::ByCommitTimeCutoff {
            order: gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
            seconds: cutoff,
        })
        .all()
        .map_err(walk_error)?;
    let mut files: BTreeMap<BString, History> = BTreeMap::new();
    for info in walk {
        let info = info.map_err(walk_error)?;
        let commit = repo
            .gix()
            .find_commit(info.id)
            .map_err(|failure| error::from_gix("read a commit", &failure))?;
        let parents: Vec<gix::ObjectId> = commit.parent_ids().map(gix::Id::detach).collect();
        if parents.len() > 1 {
            continue;
        }
        let time = commit.time().map(|time| time.seconds).unwrap_or_default();
        if time < cutoff {
            continue;
        }
        let author = commit
            .author()
            .map(|signature| signature.email.to_str_lossy().into_owned())
            .unwrap_or_default();
        let tree = commit.tree_id().map_err(walk_error)?.detach();
        let parent_tree = match parents.first() {
            Some(parent) => Some(
                repo.gix()
                    .find_commit(*parent)
                    .and_then(|parent| parent.tree_id())
                    .map_err(walk_error)?
                    .detach(),
            ),
            None => None,
        };
        let mut changed = Vec::new();
        changed_paths(repo, parent_tree, Some(tree), b"".as_bstr(), &mut changed)?;
        for path in changed {
            let history = files.entry(path).or_default();
            history.commits += 1;
            history.authors.insert(author.clone());
            history.last_change = history.last_change.max(time);
        }
    }
    let mut stats: Vec<FileHistoryStat> = files
        .into_iter()
        .map(|(path, history)| FileHistoryStat {
            path: path.to_str_lossy().into_owned(),
            commits: history.commits,
            authors: u32::try_from(history.authors.len()).unwrap_or(u32::MAX),
            last_change: Some(WallTime::from_unix_nanos(
                i128::from(history.last_change) * NANOS_PER_SECOND,
            )),
        })
        .collect();
    stats.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.path.cmp(&b.path)));
    stats.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(stats)
}

/// Files that differ between the trees `old` and `new`, with `prefix`
/// prepended; equal subtrees are not read.
fn changed_paths(
    repo: &GitRepo,
    old: Option<gix::ObjectId>,
    new: Option<gix::ObjectId>,
    prefix: &BStr,
    out: &mut Vec<BString>,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    let old_entries = tree_entries(repo, old)?;
    let new_entries = tree_entries(repo, new)?;
    let names: BTreeSet<&BString> = old_entries.keys().chain(new_entries.keys()).collect();
    for name in names {
        let before = old_entries.get(name);
        let after = new_entries.get(name);
        if before == after {
            continue;
        }
        let path = join(prefix, name.as_bstr());
        let subtree = |entry: Option<&(bool, u32, gix::ObjectId)>| {
            entry
                .filter(|(is_tree, _, _)| *is_tree)
                .map(|(_, _, id)| *id)
        };
        let is_file = |entry: Option<&(bool, u32, gix::ObjectId)>| {
            entry.is_some_and(|(is_tree, _, _)| !*is_tree)
        };
        if subtree(before).is_some() || subtree(after).is_some() {
            changed_paths(repo, subtree(before), subtree(after), path.as_bstr(), out)?;
        }
        if is_file(before) || is_file(after) {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{FixedClock, RepoBuilder, TestHost};

    fn queries() -> RepoQueries {
        RepoQueries::new(
            Arc::new(FixedClock::at_unix_seconds(
                RepoBuilder::EPOCH + SECONDS_PER_DAY,
            )),
            Arc::new(TestHost::new("/nonexistent-home")),
        )
    }

    fn fixture() -> crate::testutil::Fixture {
        RepoBuilder::new()
            .file(".gitignore", "*.log\nbuild/\n")
            .file("a.txt", "a\n")
            .file("src/b.txt", "b\n")
            .commit("one")
            .file("a.txt", "a2\n")
            .commit("two")
            .build()
    }

    fn status_of(statuses: &[GitPathStatus], path: &str) -> (GitChange, GitChange) {
        let status = statuses
            .iter()
            .find(|status| status.path == path)
            .expect("path listed");
        (status.index, status.worktree)
    }

    #[test]
    fn repo_root_is_found_from_a_subdirectory_and_absent_outside() {
        let fixture = fixture();
        let service = queries();
        assert_eq!(
            service.repo_root(&fixture.path().join("src")).unwrap(),
            Some(fixture.path().to_path_buf())
        );
        assert_eq!(service.repo_root(fixture.temp_dir()).unwrap(), None);
    }

    #[test]
    fn clean_repository_has_no_changes() {
        let fixture = fixture();
        assert!(queries().status(fixture.path(), &[]).unwrap().is_empty());
        assert!(
            queries()
                .changed_files(fixture.path(), None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn worktree_changes_deletions_and_untracked_files_are_reported() {
        let fixture = fixture();
        std::fs::write(fixture.path().join("a.txt"), "changed\n").unwrap();
        std::fs::remove_file(fixture.path().join("src/b.txt")).unwrap();
        std::fs::write(fixture.path().join("new.txt"), "n\n").unwrap();
        std::fs::write(fixture.path().join("debug.log"), "x\n").unwrap();
        std::fs::create_dir_all(fixture.path().join("build")).unwrap();
        std::fs::write(fixture.path().join("build/out.o"), "o").unwrap();
        let statuses = queries().status(fixture.path(), &[]).unwrap();
        assert_eq!(
            status_of(&statuses, "a.txt"),
            (GitChange::Unmodified, GitChange::Modified)
        );
        assert_eq!(
            status_of(&statuses, "src/b.txt"),
            (GitChange::Unmodified, GitChange::Deleted)
        );
        assert_eq!(
            status_of(&statuses, "new.txt"),
            (GitChange::Unmodified, GitChange::Untracked)
        );
        assert_eq!(statuses.len(), 3);
    }

    #[test]
    fn an_untracked_file_keeps_the_staged_deletion_of_its_path() {
        let fixture = fixture();
        let repo = GitRepo::open(fixture.path(), &crate::testutil::test_ctx()).unwrap();
        let mut index = repo.read_index().unwrap();
        index
            .state
            .remove_entries(|_, path, _| path == BStr::new("a.txt"));
        let journal = crate::testutil::RecordingJournal::new();
        let mut tx = journal.begin();
        crate::repo::index::write_index(tx.as_mut(), &mut index).unwrap();
        tx.commit().unwrap();
        let statuses = queries().status(fixture.path(), &[]).unwrap();
        assert_eq!(
            status_of(&statuses, "a.txt"),
            (GitChange::Deleted, GitChange::Untracked)
        );
    }

    #[test]
    fn requested_paths_include_unmodified_tracked_files() {
        let fixture = fixture();
        std::fs::write(fixture.path().join("a.txt"), "changed\n").unwrap();
        let statuses = queries()
            .status(
                fixture.path(),
                &["src/b.txt".to_owned(), "a.txt".to_owned()],
            )
            .unwrap();
        assert_eq!(statuses.len(), 2);
        assert_eq!(
            status_of(&statuses, "src/b.txt"),
            (GitChange::Unmodified, GitChange::Unmodified)
        );
        assert_eq!(status_of(&statuses, "a.txt").1, GitChange::Modified);
    }

    #[test]
    fn changes_against_an_older_revision_include_committed_files() {
        let fixture = fixture();
        let changed = queries()
            .changed_files(fixture.path(), Some("HEAD~1"))
            .unwrap();
        assert_eq!(changed, vec!["a.txt".to_owned()]);
    }

    #[test]
    fn line_ending_conversion_is_not_a_change() {
        let fixture = RepoBuilder::new()
            .config("core.autocrlf", "true")
            .file("a.txt", "a\n")
            .commit("one")
            .build();
        std::fs::write(fixture.path().join("a.txt"), "a\r\n").unwrap();
        assert!(
            queries()
                .changed_files(fixture.path(), None)
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn link_content_keeps_backslashes_and_non_utf8_bytes_of_the_target() {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(link_content(Path::new("a\\b/c")).unwrap(), b"a\\b/c");
        let target = std::ffi::OsStr::from_bytes(b"t\xffx");
        assert_eq!(link_content(Path::new(target)).unwrap(), b"t\xffx");
    }

    #[cfg(unix)]
    #[test]
    fn worktree_file_keeps_the_bytes_of_a_non_utf8_name() {
        use std::os::unix::ffi::OsStrExt;
        let path = worktree_file(Path::new("/r"), b"d\xffir/f".as_bstr());
        assert_eq!(path.as_os_str().as_bytes(), b"/r/d\xffir/f");
    }

    #[cfg(unix)]
    #[test]
    fn untracked_files_below_a_non_utf8_directory_are_listed() {
        use std::os::unix::ffi::OsStrExt;
        let fixture = fixture();
        let directory = fixture.path().join(std::ffi::OsStr::from_bytes(b"d\xffir"));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("f.txt"), "x\n").unwrap();
        let statuses = queries().status(fixture.path(), &[]).unwrap();
        assert_eq!(
            status_of(&statuses, "d\u{fffd}ir/f.txt"),
            (GitChange::Unmodified, GitChange::Untracked)
        );
    }

    #[test]
    fn tracked_files_list_the_index() {
        let fixture = fixture();
        assert_eq!(
            queries().tracked_files(fixture.path()).unwrap(),
            vec![
                ".gitignore".to_owned(),
                "a.txt".to_owned(),
                "src/b.txt".to_owned()
            ]
        );
    }

    #[test]
    fn ignore_rules_cover_files_and_directory_contents() {
        let fixture = fixture();
        let service = queries();
        assert!(service.is_ignored(fixture.path(), "x.log").unwrap());
        assert!(
            service
                .is_ignored(fixture.path(), "build/deep/out.o")
                .unwrap()
        );
        assert!(!service.is_ignored(fixture.path(), "a.txt").unwrap());
    }

    #[test]
    fn file_history_counts_commits_per_file() {
        let fixture = fixture();
        let stats = queries().file_history(fixture.path(), 30, 10).unwrap();
        assert_eq!(stats[0].path, "a.txt");
        assert_eq!(stats[0].commits, 2);
        assert_eq!(stats[0].authors, 1);
        assert_eq!(
            stats[0].last_change,
            Some(WallTime::from_unix_nanos(
                i128::from(RepoBuilder::EPOCH + RepoBuilder::STEP_SECONDS) * NANOS_PER_SECOND
            ))
        );
        assert_eq!(stats.len(), 3);
        assert_eq!(
            queries().file_history(fixture.path(), 30, 1).unwrap().len(),
            1
        );
    }

    #[test]
    fn changed_paths_descend_only_into_changed_subtrees() {
        let fixture = fixture();
        let repo = GitRepo::open(fixture.path(), &crate::testutil::test_ctx()).unwrap();
        let old = repo.resolve_rev("HEAD~1^{tree}").unwrap();
        let new = repo.resolve_rev("HEAD^{tree}").unwrap();
        let mut changed = Vec::new();
        changed_paths(&repo, Some(old), Some(new), b"".as_bstr(), &mut changed).unwrap();
        assert_eq!(changed, vec![BString::from("a.txt")]);
        let mut all = Vec::new();
        changed_paths(&repo, None, Some(new), b"".as_bstr(), &mut all).unwrap();
        assert_eq!(all.len(), 3);
    }
}
