// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `git.diff` command.
//!
//! Sides: without `from` and `to` the working tree is compared with the
//! index, or with `staged` the index with `HEAD`; `from` alone compares the
//! revision with the working tree, or with `staged` with the index; `from`
//! and `to` compare two revisions. Paths are relative to the repository root
//! and select files and directories literally. Renames are detected as
//! configured by `diff.renames` (on by default). The output is one `hunk`
//! record per hunk, or with `stat` one `change` record per file.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Component, Path as StdPath, PathBuf};

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{Change, ChangeAction, DiffUnit};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::CodeService;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Path, Record, Value};
use regex::bytes::Regex;

use super::canonical::{Driver, FileDiff, FileOptions, FilePair, configured_lines, diff_texts};
use super::hunk::{CANONICAL_CONTEXT, Layout};
use super::moved::{mark_moved, only_moved};
use super::rename::{RenameOptions, detect_renames};
use super::source::{
    Entry, Reader, index_files, open_repository, read_error, resolve_tree, tree_files,
};
use super::words::word_lines;

/// Largest accepted number of context lines.
pub const MAX_CONTEXT: u32 = 200;

const TYPE_MASK: u32 = 0o170_000;

/// File names of dependency lock files, flagged `lockfile` in `change`
/// records.
pub const LOCK_FILES: [&str; 12] = [
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "uv.lock",
    "go.sum",
    "composer.lock",
    "Gemfile.lock",
    "flake.lock",
];

/// Patterns of generated files, flagged `generated` in `change` records.
pub const GENERATED_PATTERNS: [&str; 12] = [
    "**/*.min.js",
    "**/*.min.css",
    "**/*.map",
    "**/*.snap",
    "**/*.pb.go",
    "**/*_pb2.py",
    "**/*.generated.*",
    "**/dist/**",
    "**/build/**",
    "**/node_modules/**",
    "**/vendor/**",
    "**/__snapshots__/**",
];

/// Parameters of a `git.diff` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRequest {
    /// Repository or a path inside it.
    pub repo: PathBuf,
    /// Old revision.
    pub from: Option<String>,
    /// New revision.
    pub to: Option<String>,
    /// Compare the index instead of the working tree.
    pub staged: bool,
    /// Files and directories, relative to the repository root.
    pub paths: Vec<String>,
    /// Context lines.
    pub context: u32,
    /// One `change` record per file instead of hunks.
    pub stat: bool,
    /// Extend hunks to whole functions.
    pub function_context: bool,
    /// Only changes within this code element of the new side.
    pub symbol: Option<String>,
    /// Lines as words in the porcelain form.
    pub word_diff: bool,
    /// Ignore white space.
    pub ignore_whitespace: bool,
    /// Leave out hunks that only move lines.
    pub ignore_moved: bool,
    /// Glob patterns of files to leave out.
    pub exclude: Vec<String>,
}

impl DiffRequest {
    /// A request for `repo` with the default parameters.
    #[must_use]
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            from: None,
            to: None,
            staged: false,
            paths: Vec::new(),
            context: CANONICAL_CONTEXT,
            stat: false,
            function_context: false,
            symbol: None,
            word_diff: false,
            ignore_whitespace: false,
            ignore_moved: false,
            exclude: Vec::new(),
        }
    }

    /// Reads the bound arguments.
    ///
    /// # Errors
    ///
    /// Fails when an argument has an unexpected type.
    pub fn from_args(args: &BoundArgs) -> Result<Self, Error> {
        let repo = path_value(args.require("repo")?).ok_or_else(|| missing("repo"))?;
        let mut request = Self::new(repo);
        request.from = args.text("from")?.map(str::to_owned);
        request.to = args.text("to")?.map(str::to_owned);
        request.staged = args.bool("staged")?.unwrap_or(false);
        request.paths = list_value(args.value("paths").or_else(|| args.value("path")));
        request.exclude = list_value(args.value("exclude"));
        if let Some(context) = args.int("context")? {
            request.context = u32::try_from(context).map_err(|_| context_error(context))?;
        }
        request.stat = args.bool("stat")?.unwrap_or(false);
        request.function_context = args.bool("function_context")?.unwrap_or(false);
        request.symbol = args.text("symbol")?.map(str::to_owned);
        request.word_diff = args.bool("word_diff")?.unwrap_or(false);
        request.ignore_whitespace = args.bool("ignore_whitespace")?.unwrap_or(false);
        request.ignore_moved = args.bool("ignore_moved")?.unwrap_or(false);
        Ok(request)
    }

    /// Checks the combination of parameters.
    ///
    /// # Errors
    ///
    /// `E_MISSING_PARAM` for `to` without `from` and for `symbol` without a
    /// path, `E_ARITY` for `symbol` with several paths, `E_ARG` with problem
    /// `param_conflict` for `staged` with `to`, and `E_INVALID_VALUE` for a
    /// context above [`MAX_CONTEXT`].
    pub fn validate(&self) -> Result<(), Error> {
        if self.to.is_some() && self.from.is_none() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                "`to` needs `from`, give the old revision as `from`",
            )
            .with_problem("missing_from"));
        }
        if self.staged && self.to.is_some() {
            return Err(Error::new(
                ErrorCode::Arg,
                "`staged` compares the index and cannot be combined with `to`, remove one of them",
            )
            .with_problem("param_conflict"));
        }
        if self.context > MAX_CONTEXT {
            return Err(context_error(i64::from(self.context)));
        }
        if self.symbol.is_some() {
            match self.paths.len() {
                0 => {
                    return Err(Error::new(
                        ErrorCode::MissingParam,
                        "`symbol` needs exactly one file in `paths`",
                    )
                    .with_problem("missing_path"));
                }
                1 => {}
                _ => {
                    return Err(Error::new(
                        ErrorCode::Arity,
                        "`symbol` needs exactly one file in `paths`",
                    )
                    .with_problem("too_many_paths"));
                }
            }
        }
        Ok(())
    }

    fn canonical_layout(&self) -> bool {
        self.context == CANONICAL_CONTEXT
            && !self.function_context
            && !self.ignore_whitespace
            && self.symbol.is_none()
    }
}

fn context_error(value: i64) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("context {value} is out of range, use 0 to {MAX_CONTEXT}"),
    )
    .with_problem("out_of_range")
}

fn missing(name: &str) -> Error {
    Error::new(ErrorCode::MissingParam, format!("`{name}` is required")).with_problem("missing")
}

fn path_value(value: &Value) -> Option<PathBuf> {
    match value {
        Value::Path(text) | Value::Text(text) => Some(PathBuf::from(text)),
        Value::List(items) => items.first().and_then(path_value),
        _ => None,
    }
}

fn list_value(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::List(items)) => items.iter().filter_map(text_of).collect(),
        Some(other) => text_of(other).into_iter().collect(),
        None => Vec::new(),
    }
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Path(text) | Value::Text(text) | Value::Glob(text) => Some(text.clone()),
        _ => None,
    }
}

/// Records and notes of a `git.diff` call.
#[derive(Debug, Clone, Default)]
pub struct DiffOutput {
    /// `hunk` records, or `change` records with `stat`.
    pub records: Vec<Record>,
    /// Unusual effects for the report.
    pub notes: Vec<NotableEntry>,
}

/// Literal path selection relative to the repository root.
#[derive(Debug, Clone, Default)]
pub(crate) struct PathFilter {
    prefixes: Vec<BString>,
    exclude: Vec<BString>,
}

impl PathFilter {
    /// Builds the selection for the repository with working tree `root`.
    ///
    /// # Errors
    ///
    /// `E_GIT_NOT_REPOSITORY` with problem `path_outside_repo` for a path
    /// outside the repository.
    pub(crate) fn new(
        root: Option<&StdPath>,
        paths: &[String],
        exclude: &[String],
    ) -> Result<Self, Error> {
        let mut prefixes = Vec::new();
        for path in paths {
            match normalize(root, path)? {
                Some(prefix) => prefixes.push(prefix),
                None => {
                    prefixes.clear();
                    break;
                }
            }
        }
        Ok(Self {
            prefixes,
            exclude: exclude
                .iter()
                .map(|pattern| BString::from(pattern.as_str()))
                .collect(),
        })
    }

    /// The path is selected.
    pub(crate) fn selects(&self, path: &BStr) -> bool {
        let included = self.prefixes.is_empty()
            || self.prefixes.iter().any(|prefix| {
                path == prefix.as_bstr()
                    || (path.starts_with(prefix.as_slice())
                        && path.get(prefix.len()) == Some(&b'/'))
            });
        included && !self.excluded(path)
    }

    /// The path matches an exclusion pattern.
    pub(crate) fn excluded(&self, path: &BStr) -> bool {
        self.exclude
            .iter()
            .any(|pattern| glob_match(pattern.as_bstr(), path))
    }

    /// The filter has exclusion patterns.
    pub(crate) fn has_exclusions(&self) -> bool {
        !self.exclude.is_empty()
    }
}

/// Matches a glob in the syntax of Git path patterns; a pattern without `/`
/// matches the file name in any directory.
pub(crate) fn glob_match(pattern: &BStr, path: &BStr) -> bool {
    let mode = gix::glob::wildmatch::Mode::NO_MATCH_SLASH_LITERAL;
    if pattern.contains(&b'/') {
        gix::glob::wildmatch(pattern, path, mode)
    } else {
        let name = path.rsplit_str("/").next().unwrap_or(path.as_bytes());
        gix::glob::wildmatch(pattern, name.as_bstr(), mode)
    }
}

/// A path relative to the root with `/` separators; `None` selects the whole
/// repository.
fn normalize(root: Option<&StdPath>, path: &str) -> Result<Option<BString>, Error> {
    let outside = || {
        Error::new(
            ErrorCode::GitNotRepository,
            format!(
                "`{path}` is outside the repository, give a path relative to the repository root"
            ),
        )
        .with_problem("path_outside_repo")
        .with_found(path)
    };
    let candidate = StdPath::new(path);
    let relative = if candidate.is_absolute() {
        let root = root.ok_or_else(outside)?;
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let resolved = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf());
        resolved
            .strip_prefix(&root)
            .map_err(|_| outside())?
            .to_path_buf()
    } else {
        candidate.to_path_buf()
    };
    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(outside());
            }
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    Ok(Some(BString::from(parts.join("/"))))
}

/// Old side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OldSide {
    Tree(ObjectId),
    Index,
}

/// New side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewSide {
    Tree(ObjectId),
    Index,
    Worktree,
}

fn sides(repo: &gix::Repository, request: &DiffRequest) -> Result<(OldSide, NewSide), Error> {
    let from = request
        .from
        .as_deref()
        .map(|rev| resolve_tree(repo, Some(rev)))
        .transpose()?;
    let to = request
        .to
        .as_deref()
        .map(|rev| resolve_tree(repo, Some(rev)))
        .transpose()?;
    let sides = match (from, to, request.staged) {
        (Some(old), Some(new), _) => (OldSide::Tree(old), NewSide::Tree(new)),
        (Some(old), None, true) => (OldSide::Tree(old), NewSide::Index),
        (Some(old), None, false) => (OldSide::Tree(old), NewSide::Worktree),
        (None, _, true) => (OldSide::Tree(resolve_tree(repo, None)?), NewSide::Index),
        (None, _, false) => (OldSide::Index, NewSide::Worktree),
    };
    if sides.1 == NewSide::Worktree && repo.workdir().is_none() {
        return Err(Error::new(
            ErrorCode::GitNotRepository,
            "the repository has no working tree, compare revisions with `from` and `to`",
        )
        .with_problem("bare_repository"));
    }
    Ok(sides)
}

/// State of a tracked path in the working tree relative to the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorktreeState {
    Removed,
    Changed,
}

/// Paths whose working tree file differs from the index, without
/// untracked files.
pub(crate) fn worktree_changes(
    repo: &gix::Repository,
) -> Result<BTreeMap<BString, WorktreeState>, Error> {
    use gix::status::index_worktree::Item;
    use gix::status::plumbing::index_as_worktree::{Change as WtChange, EntryStatus};
    let platform = repo
        .status(gix::progress::Discard)
        .map_err(|error| read_error("status", &error))?
        .untracked_files(gix::status::UntrackedFiles::None)
        .index_worktree_rewrites(None)
        .index_worktree_submodules(gix::status::Submodule::Given {
            ignore: gix::submodule::config::Ignore::All,
            check_dirty: false,
        });
    let iter = platform
        .into_index_worktree_iter(Vec::<BString>::new())
        .map_err(|error| read_error("status", &error))?;
    let mut changes = BTreeMap::new();
    for item in iter {
        let item = item.map_err(|error| read_error("status", &error))?;
        if let Item::Modification {
            rela_path, status, ..
        } = item
        {
            let state = match status {
                EntryStatus::Change(WtChange::Removed) => WorktreeState::Removed,
                EntryStatus::Change(WtChange::Type { .. } | WtChange::Modification { .. })
                | EntryStatus::IntentToAdd => WorktreeState::Changed,
                _ => continue,
            };
            changes.insert(rela_path, state);
        }
    }
    Ok(changes)
}

struct Sides {
    old: BTreeMap<BString, Entry>,
    new: BTreeMap<BString, Entry>,
    worktree_data: HashMap<BString, Vec<u8>>,
}

fn collect_sides(
    reader: &mut Reader<'_>,
    old: OldSide,
    new: NewSide,
    filter: &PathFilter,
) -> Result<Sides, Error> {
    let repo = reader.repo();
    let (index, conflicts) = index_files(reader.index());
    let unmerged: BTreeSet<BString> = conflicts.into_keys().collect();
    let mut old_files = match old {
        OldSide::Tree(tree) => tree_files(repo, tree)?,
        OldSide::Index => index.clone(),
    };
    let mut worktree_data = HashMap::new();
    let mut new_files = match new {
        NewSide::Tree(tree) => tree_files(repo, tree)?,
        NewSide::Index => index.clone(),
        NewSide::Worktree => {
            let changes = worktree_changes(repo)?;
            let mut files = BTreeMap::new();
            for (path, entry) in &index {
                if !filter.selects(path.as_bstr()) {
                    continue;
                }
                match changes.get(path) {
                    None => {
                        files.insert(path.clone(), *entry);
                    }
                    Some(WorktreeState::Removed) => {}
                    Some(WorktreeState::Changed) => {
                        if let Some((data, mode)) =
                            reader.worktree(path.as_bstr(), Some(entry.mode))?
                        {
                            files.insert(path.clone(), Entry { mode, id: None });
                            worktree_data.insert(path.clone(), data);
                        }
                    }
                }
            }
            files
        }
    };
    for map in [&mut old_files, &mut new_files] {
        map.retain(|path, _| filter.selects(path.as_bstr()) && !unmerged.contains(path));
    }
    Ok(Sides {
        old: old_files,
        new: new_files,
        worktree_data,
    })
}

fn pairs_of(sides: &Sides) -> Vec<FilePair> {
    let paths: BTreeSet<&BString> = sides.old.keys().chain(sides.new.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| {
            let old = sides.old.get(path).copied();
            let new = sides.new.get(path).copied();
            let unchanged =
                old.is_some() && old == new && new.is_some_and(|entry| entry.id.is_some());
            (!unchanged).then(|| FilePair {
                path: path.clone(),
                old_path: None,
                old,
                new,
                similarity: None,
                copied: false,
            })
        })
        .collect()
}

fn load_side(
    reader: &mut Reader<'_>,
    data: &HashMap<BString, Vec<u8>>,
    path: &BStr,
    entry: &Entry,
) -> Result<Vec<u8>, Error> {
    if entry.id.is_none()
        && let Some(content) = data.get(path)
    {
        return Ok(content.clone());
    }
    reader.content(path, entry)
}

/// Runs a `git.diff` call.
///
/// # Errors
///
/// Fails for an invalid parameter combination, a path outside the
/// repository, an unknown revision, an unknown symbol and when the
/// repository cannot be read.
pub fn run_diff(request: &DiffRequest, ctx: &ExecCtx) -> Result<DiffOutput, Error> {
    request.validate()?;
    let opened = open_repository(&request.repo, ctx)?;
    let repo = opened.repo.gix();
    let mut notes = opened.notes;
    let (old_side, new_side) = sides(repo, request)?;
    let filter = PathFilter::new(repo.workdir(), &request.paths, &request.exclude)?;
    let mut reader = Reader::new(repo)?;
    let sides_data = collect_sides(
        &mut reader,
        old_side,
        new_side,
        &PathFilter::new(repo.workdir(), &request.paths, &[])?,
    )?;
    let mut pairs = pairs_of(&sides_data);
    if filter.has_exclusions() {
        let before = pairs.len();
        pairs.retain(|pair| !filter.excluded(pair.path.as_bstr()));
        let excluded = before - pairs.len();
        if excluded > 0 {
            let mut note = NotableEntry::new(Notable::Excluded);
            note.count = Some(excluded as u64);
            note.detail = Some("files left out by `exclude`".to_owned());
            notes.push(note);
        }
    }
    let rename_setting = repo
        .config_snapshot()
        .string("diff.renames")
        .map(|value| value.to_str_lossy().into_owned());
    if let Some(options) = RenameOptions::from_config(rename_setting.as_deref()) {
        let data = &sides_data.worktree_data;
        let mut load = |path: &BStr, entry: &Entry| load_side(&mut reader, data, path, entry);
        pairs = detect_renames(pairs, &options, &mut load)?;
    }
    if !request.stat {
        pairs = split_type_changes(pairs);
    }
    let (lines, substituted) = configured_lines(repo);
    if substituted {
        let mut note = NotableEntry::new(Notable::SemanticFallback);
        note.detail =
            Some("diff.algorithm patience is computed with the histogram algorithm".to_owned());
        notes.push(note);
    }
    let canonical = request.from.is_none() && request.to.is_none() && request.canonical_layout();
    // The canonical difference numbers hunks without `diff.interHunkContext`.
    let inter_hunk_context = if canonical {
        0
    } else {
        repo.config_snapshot()
            .integer("diff.interHunkContext")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    };
    let mut options = FileOptions {
        lines,
        layout: Layout {
            context: request.context,
            inter_hunk_context,
            function_context: request.function_context,
        },
    };
    options.lines.ignore_whitespace = request.ignore_whitespace;
    let mut files = Vec::new();
    let mut words = Vec::new();
    for pair in pairs {
        let path = pair.path.clone();
        let driver = Driver::of(&mut reader, path.as_bstr());
        note_driver(&driver, &path, &mut notes);
        let old_path = pair.old_path.clone().unwrap_or_else(|| path.clone());
        let old = match &pair.old {
            Some(entry) => load_side(
                &mut reader,
                &sides_data.worktree_data,
                old_path.as_bstr(),
                entry,
            )?,
            None => Vec::new(),
        };
        let new = match &pair.new {
            Some(entry) => load_side(
                &mut reader,
                &sides_data.worktree_data,
                path.as_bstr(),
                entry,
            )?,
            None => Vec::new(),
        };
        let mut file_options = options;
        let keep = match &request.symbol {
            Some(name) => {
                symbol_lines(ctx.code(), &path, &new, name, &mut file_options, &mut notes)?
            }
            None => None,
        };
        let diff = diff_texts(pair, &old, &new, &file_options, &driver, keep.as_ref());
        if diff.is_empty() {
            continue;
        }
        words.push(driver.word_regex.clone());
        files.push(diff);
    }
    mark_moved(&mut files);
    let records = if request.stat {
        files.iter().map(change_record).collect()
    } else {
        hunk_records(&files, &words, request, canonical, new_side)?
    };
    Ok(DiffOutput { records, notes })
}

/// Git writes a change of the object type, such as a file replaced by a
/// symbolic link, as the deletion of the old entry followed by the creation
/// of the new one; the statistics count it as one changed file.
fn split_type_changes(pairs: Vec<FilePair>) -> Vec<FilePair> {
    let mut out = Vec::with_capacity(pairs.len());
    for pair in pairs {
        match (pair.old, pair.new) {
            (Some(old), Some(new))
                if pair.old_path.is_none() && old.mode & TYPE_MASK != new.mode & TYPE_MASK =>
            {
                out.push(FilePair {
                    new: None,
                    ..pair.clone()
                });
                out.push(FilePair { old: None, ..pair });
            }
            _ => out.push(pair),
        }
    }
    out
}

fn note_driver(driver: &Driver, path: &BString, notes: &mut Vec<NotableEntry>) {
    if driver.textconv_skipped {
        let mut note = NotableEntry::new(Notable::FilterNotApplied);
        note.path = Some(path.to_str_lossy().into_owned());
        note.detail = Some("textconv program not run, the stored content is compared".to_owned());
        notes.push(note);
    }
    if let Some(pattern) = &driver.invalid_pattern {
        let mut note = NotableEntry::new(Notable::SemanticFallback);
        note.path = Some(path.to_str_lossy().into_owned());
        note.detail = Some(format!(
            "invalid function pattern `{pattern}`, the default rule is used"
        ));
        notes.push(note);
    }
}

/// Lines of the new side that hold `name`, or `None` with the function
/// context switched on when no code service knows the language.
fn symbol_lines(
    code: Option<&dyn CodeService>,
    path: &BString,
    new: &[u8],
    name: &str,
    options: &mut FileOptions,
    notes: &mut Vec<NotableEntry>,
) -> Result<Option<Range<usize>>, Error> {
    let file = StdPath::new(path.to_str_lossy().as_ref()).to_path_buf();
    let language = code.and_then(|service| {
        service
            .language_of(Some(&file), new)
            .map(|language| (service, language))
    });
    let Some((service, language)) = language else {
        options.layout.function_context = true;
        let mut note = NotableEntry::new(Notable::SemanticFallback);
        note.path = Some(path.to_str_lossy().into_owned());
        note.detail = Some(format!(
            "no code service for `{name}`, whole functions are shown"
        ));
        notes.push(note);
        return Ok(None);
    };
    let found = service.find_symbol(new, &language, name)?;
    let symbol = found.first().ok_or_else(|| {
        Error::new(
            ErrorCode::SymbolNotFound,
            format!("`{name}` is not defined in the new version of `{path}`, check the name"),
        )
        .with_problem("not_found")
        .with_found(name)
    })?;
    let first = symbol.span.start.line.saturating_sub(1) as usize;
    let last = symbol.span.end.line as usize;
    Ok(Some(first..last))
}

fn hunk_records(
    files: &[FileDiff],
    words: &[Option<String>],
    request: &DiffRequest,
    canonical: bool,
    new_side: NewSide,
) -> Result<Vec<Record>, Error> {
    let mut sections: HashMap<&BString, usize> = HashMap::new();
    for file in files {
        *sections.entry(&file.pair.path).or_default() += 1;
    }
    let mut records = Vec::new();
    for (file, word_pattern) in files.iter().zip(words) {
        let pattern = match (request.word_diff, word_pattern) {
            (true, Some(text)) => Some(Regex::new(text).map_err(|error| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("invalid word pattern `{text}`: {error}"),
                )
                .with_problem("invalid_pattern")
            })?),
            _ => None,
        };
        if file.hunks.is_empty() {
            records.push(header_record(file));
            continue;
        }
        for hunk in &file.hunks {
            if request.ignore_moved && only_moved(hunk) {
                continue;
            }
            let with_id = canonical
                && file.pair.old_path.is_none()
                && sections.get(&file.pair.path) == Some(&1);
            let mut record = hunk.to_hunk_record(with_id);
            if canonical {
                record.staged = Some(new_side == NewSide::Index);
            }
            if request.word_diff {
                record.lines = word_lines(hunk, pattern.as_ref())
                    .into_iter()
                    .map(|line| line.to_str_lossy().into_owned())
                    .collect();
                record.unit = Some(DiffUnit::Words);
            }
            if request.ignore_whitespace {
                record.ignore_whitespace = Some(true);
            }
            records.push(Record::from(record));
        }
    }
    Ok(records)
}

/// A record without lines for a file whose change has no hunk: a change of
/// the mode, an empty file, or a rename without content change.
fn header_record(file: &FileDiff) -> Record {
    let mut hunk = super::hunk::Hunk::binary_placeholder(file.pair.path.clone());
    hunk.binary = false;
    hunk.number = 0;
    hunk.status = file.pair.status();
    hunk.old_path = file.pair.old_path.clone();
    hunk.old_mode = file.old_mode();
    hunk.new_mode = file.new_mode();
    hunk.to_record(false)
}

fn change_record(file: &FileDiff) -> Record {
    let pair = &file.pair;
    let action = match (&pair.old, &pair.new) {
        (None, _) => ChangeAction::Create,
        (_, None) => ChangeAction::Delete,
        _ if pair.old_path.is_some() && !pair.copied => ChangeAction::Rename,
        _ if pair.old_path.is_some() => ChangeAction::Create,
        _ => ChangeAction::Modify,
    };
    let mut flags = Vec::new();
    if file.binary {
        flags.push("binary".to_owned());
    }
    if is_lock_file(pair.path.as_bstr()) {
        flags.push("lockfile".to_owned());
    }
    if is_generated(pair.path.as_bstr()) {
        flags.push("generated".to_owned());
    }
    let change = Change {
        path: Path::from(pair.path.to_str_lossy().into_owned()),
        action,
        from: pair
            .old_path
            .as_ref()
            .map(|old| Path::from(old.to_str_lossy().into_owned())),
        added: file.added,
        removed: file.removed,
        flags: (!flags.is_empty()).then_some(flags),
        ..Change::default()
    };
    let mut record = Record::from(change);
    if let Some(similarity) = pair.similarity {
        record.set("similarity", Value::from(u64::from(similarity)));
    }
    record
}

/// The file is a dependency lock file.
#[must_use]
pub fn is_lock_file(path: &BStr) -> bool {
    let name = path.rsplit_str("/").next().unwrap_or(path.as_bytes());
    LOCK_FILES.iter().any(|lock| name == lock.as_bytes())
}

/// The file matches a pattern of generated files.
#[must_use]
pub fn is_generated(path: &BStr) -> bool {
    let rooted = BString::from(format!("x/{path}"));
    GENERATED_PATTERNS.iter().any(|pattern| {
        gix::glob::wildmatch(
            pattern.as_bytes().as_bstr(),
            rooted.as_bstr(),
            gix::glob::wildmatch::Mode::NO_MATCH_SLASH_LITERAL,
        )
    })
}

/// The `git.diff` stage.
#[derive(Debug, Clone, Copy, Default)]
pub struct GitDiff;

impl StageImpl for GitDiff {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "git.diff".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo, Error> {
        DiffRequest::from_args(args)?.validate()?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for GitDiff {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>, Error> {
        let request = DiffRequest::from_args(args)?;
        ctx.checkpoint()?;
        let output = run_diff(&request, ctx)?;
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

    fn ctx() -> ExecCtx {
        crate::testutil::test_ctx()
    }

    fn request(fixture: &Fixture) -> DiffRequest {
        DiffRequest::new(fixture.root())
    }

    fn texts(record: &Record, field: &str) -> Vec<String> {
        match record.get(field) {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|item| match item {
                    Value::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn text(record: &Record, field: &str) -> Option<String> {
        match record.get(field) {
            Some(Value::Text(text) | Value::Path(text) | Value::Enum(text)) => Some(text.clone()),
            _ => None,
        }
    }

    #[test]
    fn validates_parameter_combinations() {
        let mut bad = DiffRequest::new("/tmp");
        bad.to = Some("HEAD".to_owned());
        assert_eq!(bad.validate().unwrap_err().code(), ErrorCode::MissingParam);
        bad.from = Some("HEAD~1".to_owned());
        bad.staged = true;
        assert_eq!(
            bad.validate().unwrap_err().data().problem.as_deref(),
            Some("param_conflict")
        );
        let mut context = DiffRequest::new("/tmp");
        context.context = MAX_CONTEXT + 1;
        assert_eq!(
            context.validate().unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let mut symbol = DiffRequest::new("/tmp");
        symbol.symbol = Some("beta".to_owned());
        assert_eq!(
            symbol.validate().unwrap_err().code(),
            ErrorCode::MissingParam
        );
        symbol.paths = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(symbol.validate().unwrap_err().code(), ErrorCode::Arity);
    }

    #[test]
    fn worktree_difference_has_canonical_identifiers() {
        let fixture = Fixture::new();
        fixture.commit(&[("src/a.ts", "export function alpha() {\n  return 1;\n}\n")]);
        fixture.write("src/a.ts", "export function alpha() {\n  return 10;\n}\n");
        let output = run_diff(&request(&fixture), &ctx()).unwrap();
        assert_eq!(output.records.len(), 1);
        let record = &output.records[0];
        assert_eq!(
            texts(record, "lines"),
            [
                " export function alpha() {",
                "-  return 1;",
                "+  return 10;",
                " }"
            ]
        );
        assert_eq!(text(record, "id").as_deref(), Some("src/a.ts#1"));
        assert!(text(record, "fingerprint").is_some_and(|value| value.len() == 6));
    }

    #[test]
    fn staged_and_revision_sides() {
        let fixture = Fixture::new();
        let first = fixture.commit(&[("a.txt", "1\n"), ("README.md", "r\n")]);
        let second = fixture.commit(&[("a.txt", "2\n"), ("README.md", "r\nmore\nlines\n")]);
        fixture.stage(&[("a.txt", "3\n")]);
        let mut staged = request(&fixture);
        staged.staged = true;
        let output = run_diff(&staged, &ctx()).unwrap();
        assert_eq!(texts(&output.records[0], "lines"), ["-2", "+3"]);
        let mut revisions = request(&fixture);
        revisions.from = Some(first.to_string());
        revisions.to = Some(second.to_string());
        revisions.stat = true;
        let output = run_diff(&revisions, &ctx()).unwrap();
        let summary: Vec<_> = output
            .records
            .iter()
            .map(|record| {
                (
                    text(record, "path").unwrap(),
                    record.get("added").cloned(),
                    record.get("removed").cloned(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "README.md".to_owned(),
                    Some(Value::from(2_u64)),
                    Some(Value::from(0_u64))
                ),
                (
                    "a.txt".to_owned(),
                    Some(Value::from(1_u64)),
                    Some(Value::from(1_u64))
                ),
            ]
        );
    }

    #[test]
    fn paths_select_files_and_reject_outside_paths() {
        let fixture = Fixture::new();
        fixture.commit(&[("src/a.txt", "a\n"), ("b.txt", "b\n")]);
        fixture.write("src/a.txt", "A\n");
        fixture.write("b.txt", "B\n");
        let mut only_src = request(&fixture);
        only_src.paths = vec!["src".to_owned()];
        let output = run_diff(&only_src, &ctx()).unwrap();
        assert_eq!(output.records.len(), 1);
        let mut outside = request(&fixture);
        outside.paths = vec!["/etc/hosts".to_owned()];
        let error = run_diff(&outside, &ctx()).unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("path_outside_repo"));
        let mut parent = request(&fixture);
        parent.paths = vec!["../x".to_owned()];
        assert!(run_diff(&parent, &ctx()).is_err());
    }

    #[test]
    fn reports_unknown_revision() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n")]);
        let mut unknown = request(&fixture);
        unknown.from = Some("no-such-branch".to_owned());
        assert_eq!(
            run_diff(&unknown, &ctx())
                .unwrap_err()
                .data()
                .problem
                .as_deref(),
            Some("not_found")
        );
        let mut option_like = request(&fixture);
        option_like.from = Some("--output=x".to_owned());
        assert_eq!(
            run_diff(&option_like, &ctx())
                .unwrap_err()
                .data()
                .problem
                .as_deref(),
            Some("invalid_syntax")
        );
    }

    #[test]
    fn detects_staged_rename() {
        let fixture = Fixture::new();
        let content = "one\ntwo\nthree\nfour\nfive\n";
        fixture.commit(&[("old.txt", content)]);
        let repo = fixture.repo();
        let mut index = repo
            .index_from_tree(&repo.head_tree_id().unwrap().detach())
            .unwrap();
        let entry_index = index.entry_index_by_path("old.txt".into()).unwrap();
        let entry = index.entries()[entry_index].clone();
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
        let mut staged = request(&fixture);
        staged.staged = true;
        let output = run_diff(&staged, &ctx()).unwrap();
        assert_eq!(output.records.len(), 1);
        assert_eq!(text(&output.records[0], "path").as_deref(), Some("new.txt"));
        assert_eq!(
            text(&output.records[0], "old_path").as_deref(),
            Some("old.txt")
        );
        assert_eq!(
            text(&output.records[0], "status").as_deref(),
            Some("renamed")
        );
    }

    #[test]
    fn word_difference_and_whitespace_options() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "return 1;\n  keep\n")]);
        fixture.write("a.txt", "return 10;\nkeep\n");
        let mut words = request(&fixture);
        words.word_diff = true;
        let output = run_diff(&words, &ctx()).unwrap();
        assert_eq!(
            texts(&output.records[0], "lines"),
            [" return ", "-1;", "+10;", "~", " keep", "~"]
        );
        assert_eq!(text(&output.records[0], "unit").as_deref(), Some("words"));
        fixture.write("a.txt", "return  1;\nkeep\n");
        let mut whitespace = request(&fixture);
        whitespace.ignore_whitespace = true;
        assert!(run_diff(&whitespace, &ctx()).unwrap().records.is_empty());
    }

    #[test]
    fn symbol_without_code_service_falls_back_to_function_context() {
        let fixture = Fixture::new();
        fixture.commit(&[(
            "a.c",
            "int alpha(void)\n{\n  return 1;\n}\n\nint beta(void)\n{\n  return 2;\n}\n",
        )]);
        fixture.write(
            "a.c",
            "int alpha(void)\n{\n  return 1;\n}\n\nint beta(void)\n{\n  return 20;\n}\n",
        );
        let mut symbol = request(&fixture);
        symbol.paths = vec!["a.c".to_owned()];
        symbol.symbol = Some("beta".to_owned());
        let output = run_diff(&symbol, &ctx()).unwrap();
        assert!(
            output
                .notes
                .iter()
                .any(|note| note.code == Notable::SemanticFallback)
        );
        assert_eq!(
            texts(&output.records[0], "lines")[..2],
            [" ", " int beta(void)"]
        );
    }

    #[test]
    fn stat_flags_lock_generated_and_binary_files() {
        let fixture = Fixture::new();
        fixture.commit(&[
            ("bun.lock", "a\n"),
            ("dist/app.js", "x\n"),
            ("logo.bin", "a"),
        ]);
        fixture.write("bun.lock", "a\nb\nc\n");
        fixture.write("dist/app.js", "y\n");
        fixture.write_bytes("logo.bin", b"\0\x01");
        let mut stat = request(&fixture);
        stat.stat = true;
        let output = run_diff(&stat, &ctx()).unwrap();
        let flags: Vec<_> = output
            .records
            .iter()
            .map(|record| texts(record, "flags"))
            .collect();
        assert_eq!(
            flags,
            [
                vec!["lockfile".to_owned()],
                vec!["generated".to_owned()],
                vec!["binary".to_owned()]
            ]
        );
        let mut excluded = request(&fixture);
        excluded.exclude = vec!["*.lock".to_owned()];
        let output = run_diff(&excluded, &ctx()).unwrap();
        assert_eq!(output.records.len(), 2);
        assert!(
            output
                .notes
                .iter()
                .any(|note| note.code == Notable::Excluded && note.count == Some(1))
        );
    }

    #[test]
    fn canonical_hunks_ignore_the_inter_hunk_context_of_the_configuration() {
        use std::io::Write;

        let fixture = Fixture::new();
        let old: String = (1..=30).map(|number| format!("l{number}\n")).collect();
        fixture.commit(&[("a.txt", old.as_str())]);
        let new = old.replace("l2\n", "L2\n").replace("l10\n", "L10\n");
        fixture.write("a.txt", &new);
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(fixture.root().join(".git/config"))
            .unwrap();
        config
            .write_all(b"[diff]\n\tinterHunkContext = 5\n")
            .unwrap();
        let canonical = run_diff(&request(&fixture), &ctx()).unwrap();
        assert_eq!(canonical.records.len(), 2);
        assert_eq!(
            text(&canonical.records[1], "id").as_deref(),
            Some("a.txt#2")
        );
        let mut other = request(&fixture);
        other.ignore_whitespace = true;
        let joined = run_diff(&other, &ctx()).unwrap();
        assert_eq!(joined.records.len(), 1);
        assert_eq!(text(&joined.records[0], "id"), None);
    }

    #[test]
    fn mode_change_yields_record_without_lines() {
        let fixture = Fixture::new();
        fixture.commit(&[("run.sh", "echo\n")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = fixture.root().join("run.sh");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let output = run_diff(&request(&fixture), &ctx()).unwrap();
            assert_eq!(output.records.len(), 1);
            assert!(texts(&output.records[0], "lines").is_empty());
            assert_eq!(
                text(&output.records[0], "new_mode").as_deref(),
                Some("100755")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn type_change_is_a_deletion_and_an_addition() {
        let fixture = Fixture::new();
        fixture.commit(&[("a.txt", "a\n"), ("x", "x\n")]);
        fixture.remove("x");
        std::os::unix::fs::symlink("a.txt", fixture.root().join("x")).unwrap();
        let output = run_diff(&request(&fixture), &ctx()).unwrap();
        let summary: Vec<_> = output
            .records
            .iter()
            .map(|record| {
                (
                    text(record, "status"),
                    texts(record, "lines"),
                    text(record, "id"),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (Some("deleted".to_owned()), vec!["-x".to_owned()], None),
                (
                    Some("added".to_owned()),
                    vec!["+a.txt".to_owned(), NO_NEWLINE_LINE.to_owned()],
                    None
                ),
            ]
        );
        let mut stat = request(&fixture);
        stat.stat = true;
        assert_eq!(run_diff(&stat, &ctx()).unwrap().records.len(), 1);
    }

    const NO_NEWLINE_LINE: &str = "\\ No newline at end of file";

    #[test]
    fn matches_glob_patterns() {
        assert!(glob_match("*.lock".into(), "dir/bun.lock".into()));
        assert!(glob_match(
            "**/secrets.*".into(),
            "config/secrets.json".into()
        ));
        assert!(!glob_match("src/*.ts".into(), "src/deep/a.ts".into()));
        assert!(is_generated("web/dist/app.js".into()));
        assert!(is_generated("app.min.js".into()));
        assert!(!is_generated("src/app.js".into()));
        assert!(is_lock_file("sub/Cargo.lock".into()));
    }
}
