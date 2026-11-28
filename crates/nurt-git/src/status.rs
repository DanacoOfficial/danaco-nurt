// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::status
// Responsibility:   The `git.status` command.
// Dependencies:     bstr, gix, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `git.status` command.
//!
//! The entries follow `git status --porcelain=v2`: changed tracked paths and
//! paths with conflict stages in byte order of the path, then untracked
//! paths, then ignored paths. The index is compared with `HEAD` with rename
//! detection as configured by `status.renames` (by default the value of
//! `diff.renames`, which is on); the working tree is compared with the index
//! without rename detection. An untracked directory without tracked files is
//! one entry ending in `/` unless every file is requested.
//!
//! Index entries with the skip-worktree flag are not compared with the
//! working tree, and directory entries of a sparse index cover the files of
//! `HEAD` below them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path as StdPath, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{
    GitConflictKind, GitFileState, GitStatus as StatusEntry, GitStatusKind, Submodule,
};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Path, Record, Value};

use crate::diff::command::PathFilter;
use crate::diff::source::{
    Entry, MODE_EXECUTABLE, MODE_FILE, MODE_GITLINK, Reader, index_files, open_repository,
    read_error, resolve_tree, tree_files,
};
use crate::diff::{
    Driver, FileOptions, FilePair, RenameOptions, configured_lines, detect_renames, diff_contents,
};
use crate::repo::GitRepo;
use crate::repo::records::raw_path_bytes;

/// Default number of entries listed per repository.
pub const DEFAULT_LIMIT: u32 = 200;

/// Largest accepted number of entries per repository.
pub const MAX_LIMIT: u32 = 5000;

const TYPE_MASK: u32 = 0o170_000;
const MODE_TREE: u32 = 0o040_000;

/// Untracked files listed by `git.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Untracked {
    /// Untracked files, with directories that hold no tracked file collapsed
    /// into one entry.
    #[default]
    Normal,
    /// Every untracked file.
    All,
    /// No untracked file.
    No,
}

impl Untracked {
    /// Reads `normal`, `all` or `no`.
    ///
    /// # Errors
    ///
    /// `E_INVALID_VALUE` for any other text.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git::status::Untracked;
    ///
    /// assert_eq!(Untracked::parse("all")?, Untracked::All);
    /// assert!(Untracked::parse("some").is_err());
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self, Error> {
        match text {
            "normal" => Ok(Self::Normal),
            "all" => Ok(Self::All),
            "no" => Ok(Self::No),
            other => Err(Error::new(
                ErrorCode::InvalidValue,
                format!("untracked `{other}` is not known, use normal, all or no"),
            )
            .with_problem("unknown_value")
            .with_found(other)),
        }
    }
}

/// Parameters of a `git.status` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRequest {
    /// Repositories or paths inside them.
    pub repos: Vec<PathBuf>,
    /// Untracked files to list.
    pub untracked: Untracked,
    /// List ignored files.
    pub ignored: bool,
    /// Entries per repository.
    pub limit: u32,
    /// Count added and removed lines of tracked files.
    pub stat: bool,
    /// Glob patterns of paths to leave out.
    pub exclude: Vec<String>,
}

impl StatusRequest {
    /// A request for `repos` with the default parameters.
    #[must_use]
    pub fn new(repos: Vec<PathBuf>) -> Self {
        Self {
            repos,
            untracked: Untracked::Normal,
            ignored: false,
            limit: DEFAULT_LIMIT,
            stat: false,
            exclude: Vec::new(),
        }
    }

    /// Reads the bound arguments; `default_limit` applies when `limit` is
    /// not given.
    ///
    /// # Errors
    ///
    /// `E_MISSING_PARAM` without a repository, `E_INVALID_VALUE` for an
    /// unknown `untracked` value or a negative `limit`, and the type errors
    /// of the arguments.
    pub fn from_args(args: &BoundArgs, default_limit: u32) -> Result<Self, Error> {
        let repos = path_list(args.require("repo")?);
        if repos.is_empty() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`repo` is required, give a repository path",
            )
            .with_problem("missing"));
        }
        let mut request = Self::new(repos);
        request.untracked = match args.value("untracked") {
            Some(Value::Enum(text) | Value::Text(text)) => Untracked::parse(text)?,
            Some(other) => {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "untracked must be normal, all or no, not a {}",
                        other.type_name()
                    ),
                )
                .with_problem("unknown_value"));
            }
            None => Untracked::Normal,
        };
        request.ignored = args.bool("ignored")?.unwrap_or(false);
        request.limit = match args.int("limit")? {
            Some(value) => u32::try_from(value).map_err(|_| limit_error(value))?,
            None => default_limit,
        };
        request.stat = args.bool("stat")?.unwrap_or(false);
        request.exclude = text_list(args.value("exclude"));
        Ok(request)
    }

    /// Checks the parameters.
    ///
    /// # Errors
    ///
    /// `E_MISSING_PARAM` without a repository and `E_INVALID_VALUE` for a
    /// limit outside 1 to [`MAX_LIMIT`].
    pub fn validate(&self) -> Result<(), Error> {
        if self.repos.is_empty() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`repo` is required, give a repository path",
            )
            .with_problem("missing"));
        }
        if self.limit == 0 || self.limit > MAX_LIMIT {
            return Err(limit_error(i64::from(self.limit)));
        }
        Ok(())
    }

    fn options(&self) -> StatusOptions {
        StatusOptions {
            untracked: self.untracked,
            ignored: self.ignored,
            limit: usize::try_from(self.limit).ok(),
            stat: self.stat,
            exclude: self.exclude.clone(),
        }
    }
}

fn limit_error(value: i64) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("limit {value} is out of range, use 1 to {MAX_LIMIT}"),
    )
    .with_problem("out_of_range")
}

fn path_list(value: &Value) -> Vec<PathBuf> {
    match value {
        Value::List(items) => items.iter().flat_map(path_list).collect(),
        Value::Path(text) | Value::Text(text) => vec![PathBuf::from(text)],
        _ => Vec::new(),
    }
}

fn text_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::List(items)) => items
            .iter()
            .flat_map(|item| text_list(Some(item)))
            .collect(),
        Some(Value::Path(text) | Value::Text(text) | Value::Glob(text)) => vec![text.clone()],
        _ => Vec::new(),
    }
}

/// Options of the status of one repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusOptions {
    /// Untracked files to list.
    pub untracked: Untracked,
    /// List ignored files.
    pub ignored: bool,
    /// Largest number of entries; `None` lists every entry.
    pub limit: Option<usize>,
    /// Count added and removed lines of tracked files against `HEAD`.
    pub stat: bool,
    /// Glob patterns of paths to leave out.
    pub exclude: Vec<String>,
}

/// Records and notes of a `git.status` call.
#[derive(Debug, Clone, Default)]
pub struct StatusOutput {
    /// `git_status` records of every repository in the order of the request.
    pub records: Vec<Record>,
    /// Unusual effects for the report.
    pub notes: Vec<NotableEntry>,
}

/// Runs a `git.status` call.
///
/// # Errors
///
/// Fails for invalid parameters, a path that is not inside a repository, a
/// bare repository and when the repository cannot be read.
pub fn run_status(request: &StatusRequest, ctx: &ExecCtx) -> Result<StatusOutput, Error> {
    request.validate()?;
    let options = request.options();
    let mut output = StatusOutput::default();
    for path in &request.repos {
        ctx.checkpoint()?;
        let opened = open_repository(path, ctx)?;
        output.notes.extend(opened.notes);
        let entries = repo_status(&opened.repo, &options, &mut output.notes)?;
        output.records.extend(entries.into_iter().map(Record::from));
    }
    Ok(output)
}

/// Status entries of one repository; notes are appended to `notes`.
///
/// # Errors
///
/// `E_GIT_NOT_REPOSITORY` with problem `bare_repository` for a repository
/// without a working tree, `E_GIT_NOT_REPOSITORY` with problem
/// `path_outside_repo` for an invalid exclusion, and `E_STAGE_FAILED` when
/// the index, an object or a file cannot be read.
pub fn repo_status(
    repo: &GitRepo,
    options: &StatusOptions,
    notes: &mut Vec<NotableEntry>,
) -> Result<Vec<StatusEntry>, Error> {
    let Some(workdir) = repo.workdir() else {
        return Err(bare_error(repo.root()));
    };
    let root = Path::from(path_text(workdir));
    let filter = PathFilter::new(Some(workdir), &[], &options.exclude)?;
    let mut reader = Reader::new(repo.gix())?;
    let mut tracked = tracked_changes(repo, &mut reader)?;
    let mut scan = scan_worktree(repo.gix(), options)?;
    let in_index = |path: &BString| {
        tracked
            .get(path.as_bstr())
            .is_some_and(|item| item.staged.is_some() || item.conflict.is_some())
    };
    scan.untracked.retain(|path| !in_index(path));
    scan.ignored.retain(|path| !in_index(path));
    for (path, change) in scan.changes {
        let item = tracked.entry(path).or_insert_with(Tracked::unchanged);
        item.worktree = change.state;
        item.worktree_mode = change.mode;
        item.submodule = change.submodule;
    }
    if options.stat {
        let (lines, _) = configured_lines(repo.gix());
        let file_options = FileOptions {
            lines,
            ..FileOptions::default()
        };
        for (path, item) in &mut tracked {
            item.lines = line_counts(&mut reader, path.as_bstr(), item, &file_options)?;
        }
    }
    let mut entries: Vec<StatusEntry> = tracked
        .iter()
        .filter(|(_, item)| item.is_changed())
        .map(|(path, item)| item.to_entry(path.as_bstr(), &root))
        .collect();
    entries.extend(listed(
        &scan.untracked,
        GitStatusKind::Untracked,
        GitFileState::Untracked,
        &root,
    ));
    entries.extend(listed(
        &scan.ignored,
        GitStatusKind::Ignored,
        GitFileState::Ignored,
        &root,
    ));
    let before = entries.len();
    entries.retain(|entry| !filter.excluded(entry.path.trim_end_matches('/').into()));
    push_count(
        notes,
        Notable::Excluded,
        before - entries.len(),
        "paths left out by `exclude`",
        &root,
    );
    if let Some(limit) = options.limit {
        let over = entries.len().saturating_sub(limit);
        entries.truncate(limit);
        push_count(
            notes,
            Notable::Excluded,
            over,
            &format!("entries beyond `limit: {limit}` left out"),
            &root,
        );
    }
    repository_notes(repo, &root, notes)?;
    Ok(entries)
}

fn bare_error(root: &StdPath) -> Error {
    Error::new(
        ErrorCode::GitNotRepository,
        format!(
            "`{}` is a bare repository without a working tree, compare revisions with git.diff `from` and `to`",
            root.display()
        ),
    )
    .with_problem("bare_repository")
}

/// The path with `/` separators, resolved when it exists.
fn path_text(path: &StdPath) -> String {
    let resolved = gix::path::realpath(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy().replace('\\', "/");
    match text.strip_suffix('/') {
        Some(stripped) if !stripped.is_empty() => stripped.to_owned(),
        _ => text,
    }
}

fn push_count(
    notes: &mut Vec<NotableEntry>,
    code: Notable,
    count: usize,
    detail: &str,
    root: &Path,
) {
    if count == 0 {
        return;
    }
    let mut note = NotableEntry::new(code);
    note.count = Some(u64::try_from(count).unwrap_or(u64::MAX));
    note.detail = Some(detail.to_owned());
    note.path = Some(root.as_str().to_owned());
    notes.push(note);
}

fn repository_notes(
    repo: &GitRepo,
    root: &Path,
    notes: &mut Vec<NotableEntry>,
) -> Result<(), Error> {
    if repo.head_ref()?.is_none() && repo.head_id()?.is_some() {
        let mut note = NotableEntry::new(Notable::DetachedHead);
        note.path = Some(root.as_str().to_owned());
        notes.push(note);
    }
    if repo.config_bool("core.sparseCheckout") == Some(true) {
        let mut note = NotableEntry::new(Notable::SparseCheckout);
        note.path = Some(root.as_str().to_owned());
        note.detail = Some(
            "paths outside the sparse checkout are not compared with the working tree".to_owned(),
        );
        notes.push(note);
    }
    Ok(())
}

/// State of a tracked path, or of a path with conflict stages.
#[derive(Debug, Clone, PartialEq)]
struct Tracked {
    index: GitFileState,
    worktree: GitFileState,
    head: Option<Entry>,
    staged: Option<Entry>,
    renamed_from: Option<BString>,
    similarity: Option<u32>,
    worktree_mode: Option<u32>,
    submodule: Option<Submodule>,
    conflict: Option<GitConflictKind>,
    lines: (Option<u64>, Option<u64>),
}

impl Tracked {
    fn unchanged() -> Self {
        Self {
            index: GitFileState::Unmodified,
            worktree: GitFileState::Unmodified,
            head: None,
            staged: None,
            renamed_from: None,
            similarity: None,
            worktree_mode: None,
            submodule: None,
            conflict: None,
            lines: (None, None),
        }
    }

    fn is_changed(&self) -> bool {
        self.conflict.is_some()
            || self.index != GitFileState::Unmodified
            || self.worktree != GitFileState::Unmodified
    }

    fn mode_after(&self) -> Option<u32> {
        match self.worktree {
            GitFileState::Deleted => None,
            _ => self.worktree_mode.or(self.staged.map(|entry| entry.mode)),
        }
    }

    fn to_entry(&self, path: &BStr, root: &Path) -> StatusEntry {
        let kind = if self.conflict.is_some() {
            GitStatusKind::Conflict
        } else {
            GitStatusKind::Tracked
        };
        let before = self.head.map(|entry| entry.mode);
        let after = self.mode_after();
        let modes_differ = before != after && before.is_some() && after.is_some();
        StatusEntry {
            path: path.to_str_lossy().into_owned(),
            raw_path: raw_path_bytes(path),
            index: self.index,
            worktree: self.worktree,
            kind,
            repo: root.clone(),
            renamed_from: self
                .renamed_from
                .as_ref()
                .map(|old| old.to_str_lossy().into_owned()),
            raw_renamed_from: self
                .renamed_from
                .as_ref()
                .and_then(|old| raw_path_bytes(old.as_bstr())),
            conflict: self.conflict,
            submodule: self.submodule.clone(),
            mode_before: before.filter(|_| modes_differ).map(mode_octal),
            mode_after: after.filter(|_| modes_differ).map(mode_octal),
            added: self.lines.0,
            removed: self.lines.1,
            similarity: self.similarity.map(u64::from),
        }
    }
}

fn mode_octal(mode: u32) -> String {
    format!("{mode:06o}")
}

fn listed(
    paths: &[BString],
    kind: GitStatusKind,
    state: GitFileState,
    root: &Path,
) -> Vec<StatusEntry> {
    paths
        .iter()
        .map(|path| StatusEntry {
            path: path.to_str_lossy().into_owned(),
            raw_path: raw_path_bytes(path.as_bstr()),
            index: state,
            worktree: state,
            kind,
            repo: root.clone(),
            ..StatusEntry::default()
        })
        .collect()
}

/// Paths whose index differs from `HEAD`, renames included, and the paths
/// with conflict stages.
fn tracked_changes(
    repo: &GitRepo,
    reader: &mut Reader<'_>,
) -> Result<BTreeMap<BString, Tracked>, Error> {
    let gix_repo = repo.gix();
    let mut head = tree_files(gix_repo, resolve_tree(gix_repo, None)?)?;
    let (mut index, conflicts) = index_files(reader.index());
    let sparse: Vec<BString> = index
        .iter()
        .filter(|(_, entry)| entry.mode & TYPE_MASK == MODE_TREE)
        .map(|(path, _)| path.clone())
        .collect();
    for directory in &sparse {
        index.remove(directory);
        head.retain(|path, _| !is_below(path.as_bstr(), directory.as_bstr()));
    }
    let mut tracked = BTreeMap::new();
    for (path, entry) in &index {
        let mut item = Tracked::unchanged();
        item.staged = Some(*entry);
        item.head = head.get(path).copied();
        tracked.insert(path.clone(), item);
    }
    let mut pairs = Vec::new();
    let paths: BTreeSet<&BString> = head.keys().chain(index.keys()).collect();
    for path in paths {
        if conflicts.contains_key(path) {
            continue;
        }
        let old = head.get(path).copied();
        let new = index.get(path).copied();
        if old != new {
            pairs.push(FilePair {
                path: path.clone(),
                old_path: None,
                old,
                new,
                similarity: None,
                copied: false,
            });
        }
    }
    if let Some(options) = rename_options(repo) {
        let mut load = |path: &BStr, entry: &Entry| reader.content(path, entry);
        pairs = detect_renames(pairs, &options, &mut load)?;
    }
    for pair in pairs {
        apply_pair(&mut tracked, pair);
    }
    for (path, stages) in conflicts {
        let mut item = Tracked::unchanged();
        item.index = GitFileState::Unmerged;
        item.worktree = GitFileState::Unmerged;
        item.conflict = Some(conflict_kind(&stages));
        item.head = head.get(&path).copied();
        tracked.insert(path, item);
    }
    Ok(tracked)
}

fn is_below(path: &BStr, directory: &BStr) -> bool {
    let directory = directory.strip_suffix(b"/").unwrap_or(directory);
    path.starts_with(directory) && path.get(directory.len()) == Some(&b'/')
}

fn rename_options(repo: &GitRepo) -> Option<RenameOptions> {
    let setting = repo
        .config_string("status.renames")
        .or_else(|| repo.config_string("diff.renames"));
    RenameOptions::from_config(setting.as_deref())
}

fn apply_pair(tracked: &mut BTreeMap<BString, Tracked>, pair: FilePair) {
    let state = match (&pair.old, &pair.new) {
        (_, Some(_)) if pair.old_path.is_some() => {
            if pair.copied {
                GitFileState::Copied
            } else {
                GitFileState::Renamed
            }
        }
        (None, Some(_)) => GitFileState::Added,
        (Some(_), None) => GitFileState::Deleted,
        (Some(old), Some(new)) if old.mode & TYPE_MASK != new.mode & TYPE_MASK => {
            GitFileState::TypeChanged
        }
        (Some(_), Some(_)) => GitFileState::Modified,
        (None, None) => return,
    };
    let item = tracked
        .entry(pair.path.clone())
        .or_insert_with(Tracked::unchanged);
    item.index = state;
    item.head = pair.old;
    if pair.old_path.is_some() {
        item.renamed_from = pair.old_path;
        item.similarity = pair.similarity;
    }
}

/// Kind of a conflict from the stages present: base, ours, theirs.
fn conflict_kind(stages: &[Option<Entry>; 3]) -> GitConflictKind {
    match (
        stages[0].is_some(),
        stages[1].is_some(),
        stages[2].is_some(),
    ) {
        (true, false, false) => GitConflictKind::BothDeleted,
        (false, true, false) => GitConflictKind::AddedByUs,
        (true, true, false) => GitConflictKind::DeletedByThem,
        (false, false, true) => GitConflictKind::AddedByThem,
        (true, false, true) => GitConflictKind::DeletedByUs,
        (false, true, true) => GitConflictKind::BothAdded,
        _ => GitConflictKind::BothModified,
    }
}

/// Change of a tracked path in the working tree.
#[derive(Debug, Clone, PartialEq)]
struct WorktreeChange {
    state: GitFileState,
    mode: Option<u32>,
    submodule: Option<Submodule>,
}

/// Result of the walk of the working tree.
#[derive(Debug, Default)]
struct Scan {
    changes: BTreeMap<BString, WorktreeChange>,
    untracked: Vec<BString>,
    ignored: Vec<BString>,
}

fn scan_worktree(repo: &gix::Repository, options: &StatusOptions) -> Result<Scan, Error> {
    use gix::dir::walk::EmissionMode;
    use gix::status::index_worktree::Item;
    let walk = options.untracked != Untracked::No || options.ignored;
    let (untracked, emission) = match (walk, options.untracked) {
        (false, _) => (
            gix::status::UntrackedFiles::None,
            EmissionMode::CollapseDirectory,
        ),
        (true, Untracked::All) => (gix::status::UntrackedFiles::Files, EmissionMode::Matching),
        (true, _) => (
            gix::status::UntrackedFiles::Collapsed,
            EmissionMode::CollapseDirectory,
        ),
    };
    let ignored = options.ignored.then_some(emission);
    let platform = repo
        .status(gix::progress::Discard)
        .map_err(|error| read_error("status", &error))?
        .untracked_files(untracked)
        .dirwalk_options(|walk| walk.emit_ignored(ignored).emit_empty_directories(false))
        .index_worktree_rewrites(None)
        .index_worktree_submodules(gix::status::Submodule::AsConfigured { check_dirty: true });
    let iter = platform
        .into_index_worktree_iter(Vec::<BString>::new())
        .map_err(|error| read_error("status", &error))?;
    let mut scan = Scan::default();
    for item in iter {
        match item.map_err(|error| read_error("status", &error))? {
            Item::Modification {
                rela_path,
                status,
                entry,
                ..
            } => {
                if let Some(change) = worktree_change(status, entry.mode.bits()) {
                    scan.changes.insert(rela_path, change);
                }
            }
            Item::DirectoryContents { entry, .. } => directory_entry(&mut scan, entry, options),
            Item::Rewrite { .. } => {}
        }
    }
    scan.untracked.sort();
    scan.ignored.sort();
    Ok(scan)
}

fn worktree_change(
    status: gix::status::plumbing::index_as_worktree::EntryStatus<(), gix::submodule::Status>,
    index_mode: u32,
) -> Option<WorktreeChange> {
    use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
    let change = |state, mode| WorktreeChange {
        state,
        mode,
        submodule: None,
    };
    match status {
        EntryStatus::Change(Change::Removed) => Some(change(GitFileState::Deleted, None)),
        EntryStatus::Change(Change::Type { worktree_mode }) => Some(change(
            GitFileState::TypeChanged,
            Some(worktree_mode.bits()),
        )),
        EntryStatus::Change(Change::Modification {
            executable_bit_changed,
            ..
        }) => Some(change(
            GitFileState::Modified,
            executable_bit_changed.then_some(toggle_executable(index_mode)),
        )),
        EntryStatus::Change(Change::SubmoduleModification(status)) => {
            let summary = submodule_summary(&status);
            (summary.commit_changed || summary.modified || summary.untracked).then_some(
                WorktreeChange {
                    state: GitFileState::Modified,
                    mode: None,
                    submodule: Some(summary),
                },
            )
        }
        EntryStatus::IntentToAdd => Some(change(GitFileState::Added, None)),
        EntryStatus::Conflict { .. } | EntryStatus::NeedsUpdate(_) => None,
    }
}

fn toggle_executable(mode: u32) -> u32 {
    if mode == MODE_EXECUTABLE {
        MODE_FILE
    } else {
        MODE_EXECUTABLE
    }
}

fn submodule_summary(status: &gix::submodule::Status) -> Submodule {
    use gix::status::index_worktree::Item as WorktreeItem;
    let commit_changed = matches!(
        (status.index_id, status.checked_out_head_id),
        (Some(recorded), Some(checked_out)) if recorded != checked_out
    );
    let mut summary = Submodule {
        commit_changed,
        modified: false,
        untracked: false,
    };
    for item in status.changes.iter().flatten() {
        match item {
            gix::status::Item::IndexWorktree(WorktreeItem::DirectoryContents { entry, .. }) => {
                if entry.status == gix::dir::entry::Status::Untracked {
                    summary.untracked = true;
                }
            }
            _ => summary.modified = true,
        }
    }
    summary
}

fn directory_entry(scan: &mut Scan, entry: gix::dir::Entry, options: &StatusOptions) {
    use gix::dir::entry::{Kind, Status};
    let mut path = entry.rela_path;
    if matches!(entry.disk_kind, Some(Kind::Directory | Kind::Repository)) {
        path.push(b'/');
    }
    match entry.status {
        Status::Untracked if options.untracked != Untracked::No => scan.untracked.push(path),
        Status::Ignored(_) if options.ignored => scan.ignored.push(path),
        _ => {}
    }
}

/// Added and removed lines of a tracked path from `HEAD` to the working
/// tree; `None` for binary content and submodules.
fn line_counts(
    reader: &mut Reader<'_>,
    path: &BStr,
    item: &Tracked,
    options: &FileOptions,
) -> Result<(Option<u64>, Option<u64>), Error> {
    let is_gitlink = |entry: &Entry| entry.mode == MODE_GITLINK;
    if item.conflict.is_some()
        || item.head.as_ref().is_some_and(is_gitlink)
        || item.staged.as_ref().is_some_and(is_gitlink)
    {
        return Ok((None, None));
    }
    let old_path = item.renamed_from.as_ref().map_or(path, |old| old.as_bstr());
    let old = match &item.head {
        Some(entry) => reader.content(old_path, entry)?,
        None => Vec::new(),
    };
    let new = match item.worktree {
        GitFileState::Deleted => Vec::new(),
        GitFileState::Unmodified => match &item.staged {
            Some(entry) => reader.content(path, entry)?,
            None => Vec::new(),
        },
        _ => reader
            .worktree(path, item.staged.map(|entry| entry.mode))?
            .map(|(data, _)| data)
            .unwrap_or_default(),
    };
    let pair = FilePair {
        path: path.to_owned(),
        old_path: None,
        old: item.head,
        new: item.staged,
        similarity: None,
        copied: false,
    };
    let driver = Driver::of(reader, path);
    let diff = diff_contents(pair, &old, &new, options, &driver);
    Ok((diff.added, diff.removed))
}

/// The `git.status` stage.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitStatus;

impl StageImpl for GitStatus {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "git.status".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo, Error> {
        StatusRequest::from_args(args, ctx.config().git.status.limit)?.validate()?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for GitStatus {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>, Error> {
        let request = StatusRequest::from_args(args, ctx.config().git.status.limit)?;
        let output = run_status(&request, ctx)?;
        for note in output.notes {
            ctx.note(note);
        }
        Ok(StageOutput::records(
            RecordStream::from_records(output.records).checked(ctx),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::fixture::Fixture;
    use crate::testutil::test_ctx;

    fn status_with(
        fixture: &Fixture,
        options: &StatusOptions,
    ) -> (Vec<StatusEntry>, Vec<NotableEntry>) {
        let ctx = test_ctx();
        let opened = open_repository(fixture.root(), &ctx).unwrap();
        let mut notes = Vec::new();
        let entries = repo_status(&opened.repo, options, &mut notes).unwrap();
        (entries, notes)
    }

    fn status(fixture: &Fixture) -> Vec<StatusEntry> {
        status_with(fixture, &StatusOptions::default()).0
    }

    fn summary(entries: &[StatusEntry]) -> Vec<(String, GitFileState, GitFileState)> {
        entries
            .iter()
            .map(|entry| (entry.path.clone(), entry.index, entry.worktree))
            .collect()
    }

    /// Replaces the index entries of `path` with conflict stages holding
    /// `x\n`.
    fn conflict(fixture: &Fixture, path: &str, stages: &[u32]) {
        let repo = fixture.repo();
        let mut index = repo.open_index().unwrap();
        let blob = repo.write_blob(b"x\n").unwrap().detach();
        index.remove_entries(|_, entry_path, _| entry_path == path);
        for stage in stages {
            index.dangerously_push_entry(
                gix::index::entry::Stat::default(),
                blob,
                gix::index::entry::Flags::from_bits_retain(stage << 12),
                gix::index::entry::Mode::FILE,
                path.into(),
            );
        }
        index.sort_entries();
        fixture.save_index(&mut index);
    }

    // APFS rejects a name that is not valid UTF-8, so the setup cannot be made there.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn a_path_that_is_not_valid_unicode_keeps_its_bytes_beside_the_display_form() {
        use std::os::unix::ffi::OsStrExt;
        let fixture = Fixture::new();
        fixture.commit(&[("tracked.txt", "one\n")]);
        std::fs::write(
            fixture
                .root()
                .join(std::ffi::OsStr::from_bytes(b"un\xfftracked.txt")),
            "x\n",
        )
        .unwrap();
        let entries = status(&fixture);
        let found = entries
            .iter()
            .find(|entry| entry.path.contains('\u{fffd}'))
            .expect("the untracked file is listed");
        assert_eq!(found.path, "un\u{fffd}tracked.txt");
        assert_eq!(
            found.raw_path.as_ref().map(|raw| raw.as_slice().to_vec()),
            Some(b"un\xfftracked.txt".to_vec())
        );
        assert!(
            entries
                .iter()
                .filter(|entry| !entry.path.contains('\u{fffd}'))
                .all(|entry| entry.raw_path.is_none() && entry.raw_renamed_from.is_none())
        );
    }

    #[test]
    fn parses_untracked_values_and_checks_limit() {
        assert_eq!(Untracked::parse("normal").unwrap(), Untracked::Normal);
        assert_eq!(Untracked::parse("no").unwrap(), Untracked::No);
        assert_eq!(
            Untracked::parse("x").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let mut request = StatusRequest::new(vec![PathBuf::from("/tmp")]);
        assert!(request.validate().is_ok());
        request.limit = 0;
        assert_eq!(
            request.validate().unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        request.limit = MAX_LIMIT + 1;
        assert!(request.validate().is_err());
        assert_eq!(
            StatusRequest::new(Vec::new())
                .validate()
                .unwrap_err()
                .code(),
            ErrorCode::MissingParam
        );
    }

    #[test]
    fn clean_repository_has_no_entries() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n"), ("dir/b.txt", "b\n")]);
        assert!(status(&fixture).is_empty());
    }

    #[test]
    fn reports_index_and_worktree_states_in_path_order() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
        fixture.stage(&[("a.txt", "staged\n"), ("d.txt", "new\n")]);
        fixture.write("a.txt", "worktree\n");
        fixture.remove("b.txt");
        fixture.write("new/e.txt", "e\n");
        fixture.write("new/f.txt", "f\n");
        let entries = status(&fixture);
        assert_eq!(
            summary(&entries),
            [
                (
                    "a.txt".to_owned(),
                    GitFileState::Modified,
                    GitFileState::Modified
                ),
                (
                    "b.txt".to_owned(),
                    GitFileState::Unmodified,
                    GitFileState::Deleted
                ),
                (
                    "d.txt".to_owned(),
                    GitFileState::Added,
                    GitFileState::Deleted
                ),
                (
                    "new/".to_owned(),
                    GitFileState::Untracked,
                    GitFileState::Untracked
                ),
            ]
        );
        assert_eq!(entries[3].kind, GitStatusKind::Untracked);
        assert!(
            entries[0]
                .repo
                .as_str()
                .ends_with(&path_text(fixture.root()))
        );
    }

    #[test]
    fn untracked_all_lists_files_and_no_hides_them() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n")]);
        fixture.write("new/e.txt", "e\n");
        fixture.write("new/deep/f.txt", "f\n");
        let all = StatusOptions {
            untracked: Untracked::All,
            ..StatusOptions::default()
        };
        let paths: Vec<_> = status_with(&fixture, &all)
            .0
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        assert_eq!(paths, ["new/deep/f.txt", "new/e.txt"]);
        let none = StatusOptions {
            untracked: Untracked::No,
            ..StatusOptions::default()
        };
        assert!(status_with(&fixture, &none).0.is_empty());
    }

    #[test]
    fn ignored_files_follow_untracked_files() {
        let fixture = Fixture::new();
        fixture.commit(&[(".gitignore", "*.log\nbuild/\n"), ("a.txt", "a\n")]);
        fixture.write("run.log", "x\n");
        fixture.write("build/out.bin", "x\n");
        fixture.write("notes.txt", "n\n");
        let plain: Vec<_> = status(&fixture)
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        assert_eq!(plain, ["notes.txt"]);
        let with_ignored = StatusOptions {
            ignored: true,
            ..StatusOptions::default()
        };
        let entries = status_with(&fixture, &with_ignored).0;
        let listed: Vec<_> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind))
            .collect();
        assert_eq!(
            listed,
            [
                ("notes.txt", GitStatusKind::Untracked),
                ("build/", GitStatusKind::Ignored),
                ("run.log", GitStatusKind::Ignored),
            ]
        );
    }

    #[test]
    fn file_removed_from_the_index_and_kept_on_disk_is_also_untracked() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
        let repo = fixture.repo();
        let mut index = repo.open_index().unwrap();
        index.remove_entries(|_, path, _| path == "a.txt");
        fixture.save_index(&mut index);
        assert_eq!(
            summary(&status(&fixture)),
            [
                (
                    "a.txt".to_owned(),
                    GitFileState::Deleted,
                    GitFileState::Unmodified
                ),
                (
                    "a.txt".to_owned(),
                    GitFileState::Untracked,
                    GitFileState::Untracked
                ),
            ]
        );
    }

    #[test]
    fn conflict_stages_map_to_conflict_kinds() {
        let fixture = Fixture::new();
        fixture.commit(&[
            ("both.txt", "a\n"),
            ("ours.txt", "a\n"),
            ("gone.txt", "a\n"),
            ("keep.txt", "k\n"),
        ]);
        conflict(&fixture, "both.txt", &[1, 2, 3]);
        conflict(&fixture, "ours.txt", &[2]);
        conflict(&fixture, "gone.txt", &[1, 3]);
        let entries = status(&fixture);
        let kinds: Vec<_> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind, entry.conflict))
            .collect();
        assert_eq!(
            kinds,
            [
                (
                    "both.txt",
                    GitStatusKind::Conflict,
                    Some(GitConflictKind::BothModified)
                ),
                (
                    "gone.txt",
                    GitStatusKind::Conflict,
                    Some(GitConflictKind::DeletedByUs)
                ),
                (
                    "ours.txt",
                    GitStatusKind::Conflict,
                    Some(GitConflictKind::AddedByUs)
                ),
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.index == GitFileState::Unmerged)
        );
    }

    #[test]
    fn every_stage_combination_has_a_kind() {
        let some = Some(Entry {
            mode: MODE_FILE,
            id: None,
        });
        let cases = [
            ([some, None, None], GitConflictKind::BothDeleted),
            ([None, some, None], GitConflictKind::AddedByUs),
            ([some, some, None], GitConflictKind::DeletedByThem),
            ([None, None, some], GitConflictKind::AddedByThem),
            ([some, None, some], GitConflictKind::DeletedByUs),
            ([None, some, some], GitConflictKind::BothAdded),
            ([some, some, some], GitConflictKind::BothModified),
        ];
        for (stages, kind) in cases {
            assert_eq!(conflict_kind(&stages), kind);
        }
    }

    #[test]
    fn staged_rename_reports_old_path_and_similarity() {
        let fixture = Fixture::new();
        let content = "one\ntwo\nthree\nfour\nfive\n";
        fixture.commit(&[("old.txt", content)]);
        let repo = fixture.repo();
        let mut index = repo.open_index().unwrap();
        let position = index.entry_index_by_path("old.txt".into()).unwrap();
        let entry = index.entries()[position].clone();
        index.remove_entries(|_, path, _| path == "old.txt");
        index.dangerously_push_entry(
            entry.stat,
            entry.id,
            entry.flags,
            entry.mode,
            "new.txt".into(),
        );
        index.sort_entries();
        fixture.save_index(&mut index);
        std::fs::rename(
            fixture.root().join("old.txt"),
            fixture.root().join("new.txt"),
        )
        .unwrap();
        let entries = status(&fixture);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "new.txt");
        assert_eq!(entries[0].index, GitFileState::Renamed);
        assert_eq!(entries[0].worktree, GitFileState::Unmodified);
        assert_eq!(entries[0].renamed_from.as_deref(), Some("old.txt"));
        assert_eq!(entries[0].similarity, Some(100));
    }

    #[test]
    fn stat_counts_lines_from_head_to_worktree() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "1\n2\n3\n"), ("bin.dat", "a")]);
        fixture.stage(&[("a.txt", "1\n2\n3\n4\n")]);
        fixture.write("a.txt", "1\nTWO\n3\n4\n5\n");
        fixture.write_bytes("bin.dat", b"\0\x01");
        let options = StatusOptions {
            stat: true,
            ..StatusOptions::default()
        };
        let entries = status_with(&fixture, &options).0;
        let counts: Vec<_> = entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry.added, entry.removed))
            .collect();
        assert_eq!(
            counts,
            [("a.txt", Some(3), Some(1)), ("bin.dat", None, None)]
        );
    }

    #[test]
    fn exclude_and_limit_are_reported() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n")]);
        for name in ["x.tmp", "y.tmp", "b.txt", "c.txt", "d.txt"] {
            fixture.write(name, "n\n");
        }
        let options = StatusOptions {
            exclude: vec!["*.tmp".to_owned()],
            limit: Some(2),
            ..StatusOptions::default()
        };
        let (entries, notes) = status_with(&fixture, &options);
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths, ["b.txt", "c.txt"]);
        let counts: Vec<_> = notes
            .iter()
            .filter(|note| note.code == Notable::Excluded)
            .map(|note| note.count)
            .collect();
        assert_eq!(counts, [Some(2), Some(1)]);
    }

    #[cfg(unix)]
    #[test]
    fn executable_bit_change_reports_modes() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        fixture.commit(&[("run.sh", "echo\n")]);
        let path = fixture.root().join("run.sh");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let entries = status(&fixture);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].worktree, GitFileState::Modified);
        assert_eq!(entries[0].mode_before.as_deref(), Some("100644"));
        assert_eq!(entries[0].mode_after.as_deref(), Some("100755"));
    }

    #[test]
    fn unborn_branch_lists_staged_and_untracked_files() {
        let fixture = Fixture::new();
        fixture.stage(&[("a.txt", "a\n")]);
        fixture.write("a.txt", "a\n");
        fixture.write("b.txt", "b\n");
        let entries = status(&fixture);
        assert_eq!(
            summary(&entries),
            [
                (
                    "a.txt".to_owned(),
                    GitFileState::Added,
                    GitFileState::Unmodified
                ),
                (
                    "b.txt".to_owned(),
                    GitFileState::Untracked,
                    GitFileState::Untracked
                ),
            ]
        );
    }

    #[test]
    fn detached_head_is_noted() {
        let fixture = Fixture::new();
        let commit = fixture.commit(&[("a.txt", "a\n")]);
        std::fs::write(fixture.root().join(".git/HEAD"), format!("{commit}\n")).unwrap();
        let (entries, notes) = status_with(&fixture, &StatusOptions::default());
        assert!(entries.is_empty());
        assert!(notes.iter().any(|note| note.code == Notable::DetachedHead));
    }

    #[test]
    fn bare_repository_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        crate::testutil::init_bare_isolated(dir.path()).unwrap();
        let ctx = test_ctx();
        let opened = open_repository(dir.path(), &ctx).unwrap();
        let error =
            repo_status(&opened.repo, &StatusOptions::default(), &mut Vec::new()).unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("bare_repository"));
    }

    #[test]
    fn run_status_covers_every_repository() {
        let first = Fixture::new();
        first.commit(&[("a.txt", "a\n")]);
        first.write("a.txt", "A\n");
        let second = Fixture::new();
        second.commit(&[("b.txt", "b\n")]);
        second.write("c.txt", "c\n");
        let request = StatusRequest::new(vec![
            first.root().to_path_buf(),
            second.root().to_path_buf(),
        ]);
        let output = run_status(&request, &test_ctx()).unwrap();
        let entries: Vec<StatusEntry> = output
            .records
            .into_iter()
            .map(|record| StatusEntry::try_from(record).unwrap())
            .collect();
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "c.txt"]);
        assert_ne!(entries[0].repo, entries[1].repo);
    }

    #[test]
    fn sparse_directory_covers_head_files() {
        assert!(is_below("dir/a.txt".into(), "dir/".into()));
        assert!(is_below("dir/sub/a.txt".into(), "dir".into()));
        assert!(!is_below("directory/a.txt".into(), "dir/".into()));
        assert!(!is_below("dir".into(), "dir/".into()));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
