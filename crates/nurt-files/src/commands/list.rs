// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `list`: the entries of a directory, and the listing shared with `find`.
//!
//! Both commands walk one directory with [`Walker`] and produce `entry`
//! records with the listing fields `depth` and `relative`. This module reads
//! the parameters the commands share, selects and converts the walked
//! entries, computes the directory totals of `view: sizes`, orders the
//! records and reports the notable entries of the walk.
//!
//! | `view` | Order of the records |
//! |---|---|
//! | `flat` | by `sort` over all records; ties by path |
//! | `tree` | depth first; the entries of every directory by `sort` |
//! | `sizes` | as `tree`; directories carry `total_size` and `file_count` |
//!
//! The path order compares paths component by component, so a directory
//! comes right before its contents. It is the order of the walk.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use globset::GlobSet;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileId, FileKind, FileMeta, Follow, Fs, FsError, FsErrorKind};
use nurt_contracts::record::{Entry, EntryType};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, RenderHint, StageDescriptor,
    StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Size, Value};
use unicode_normalization::UnicodeNormalization;

use super::file_types::TypeFilter;
use crate::common::entry::{entry_record, link_type};
use crate::common::params::{WalkFilters, walk_filters};
use crate::common::path::{path_arg, path_field};
use crate::fuzzy::FuzzyQuery;
use crate::walk::{self, KindFilter, WalkItem, WalkOptions, WalkSummary, Walker};

/// Name of the `path` parameter.
pub(crate) const PATH: &str = "path";
/// Name of the `depth` parameter.
pub(crate) const DEPTH: &str = "depth";
/// Name of the `type` parameter.
pub(crate) const TYPE: &str = "type";
/// Name of the `sort` parameter.
pub(crate) const SORT: &str = "sort";
/// Name of the `order` parameter.
pub(crate) const ORDER: &str = "order";
/// Name of the `view` parameter.
pub(crate) const VIEW: &str = "view";

const COMMAND: &str = "list";
const DEFAULT_DEPTH: u32 = 1;
const MIN_DEPTH: u32 = 1;
const SORT_VALUES: [&str; 4] = ["name", "modified", "size", "type"];
const KIND_VALUES: [&str; 4] = ["file", "directory", "link", "any"];
const ORDER_VALUES: [&str; 2] = ["asc", "desc"];
const VIEW_VALUES: [&str; 3] = ["flat", "tree", "sizes"];

/// `relative` of the root entry of a listing.
const ROOT_RELATIVE: &str = ".";

/// Field filled only when the rest of the pipeline uses it.
const LINK_TARGET: &str = "link_target";

const SAMPLE_SEPARATOR: &str = ", ";
const LINK_LOOP_DETAIL: &str = "links not entered because they lead back to a walked directory";

const INVALID_UNICODE: &str = "invalid_unicode";
const WINDOWS_RESERVED_NAME: &str = "windows_reserved_name";
const WINDOWS_INVALID_CHARACTER: &str = "windows_invalid_character";
const WINDOWS_TRAILING_CHARACTER: &str = "windows_trailing_character";

/// Characters Windows does not allow in names, besides control characters.
const WINDOWS_FORBIDDEN: [char; 8] = ['<', '>', ':', '"', '\\', '|', '?', '*'];
/// Device names Windows reserves with any extension.
const WINDOWS_DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
/// Device names Windows reserves when followed by one digit from 1 to 9.
const WINDOWS_NUMBERED_DEVICES: [&str; 2] = ["COM", "LPT"];
/// Characters Windows removes from the end of a name.
const WINDOWS_TRAILING: [char; 2] = ['.', ' '];

/// Implementation of `list`.
pub(crate) struct List;

impl StageImpl for List {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        ListParams::read(args)?;
        Ok(static_info(args))
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for List {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let params = ListParams::read(args)?;
        let fs = ctx.require_fs()?;
        let root = actual_root(ctx, fs, root_param(args, ctx)?);
        require_directory(fs, &root.path)?;
        params.listing(root).run(ctx)
    }
}

/// Stages of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(List)]
}

/// Parameters of `list`.
struct ListParams {
    depth: u32,
    kind: KindFilter,
    filters: WalkFilters,
    sort: SortSpec,
    view: View,
}

impl ListParams {
    fn read(args: &BoundArgs) -> Result<Self> {
        let key = match choice(args, SORT, &SORT_VALUES)? {
            Some("modified") => SortKey::Modified,
            Some("size") => SortKey::Size,
            Some("type") => SortKey::Type,
            _ => SortKey::Name,
        };
        Ok(Self {
            depth: depth_param(args, MIN_DEPTH)?.unwrap_or(DEFAULT_DEPTH),
            kind: kind_param(args, KindFilter::Any)?,
            filters: walk_filters(args)?,
            sort: SortSpec {
                key,
                descending: descending(args)?,
            },
            view: View::read(args)?,
        })
    }

    fn listing(self, root: Root) -> Listing {
        let mut options = WalkOptions {
            min_depth: MIN_DEPTH,
            max_depth: Some(self.depth),
            kind: self.kind,
            ..WalkOptions::default()
        };
        self.filters.apply(&mut options);
        Listing {
            root,
            options,
            view: self.view,
            selection: Selection {
                min_depth: MIN_DEPTH,
                max_depth: Some(self.depth),
                kind: self.kind,
                globs: None,
                names: None,
                fuzzy: None,
            },
            sort: self.sort,
        }
    }
}

/// Value of the `view` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    /// One record per entry in the order of `sort`.
    Flat,
    /// Depth-first order for drawing a tree.
    Tree,
    /// Depth-first order with directory totals.
    Sizes,
}

impl View {
    /// Reads `view`; [`View::Flat`] without a value.
    pub(crate) fn read(args: &BoundArgs) -> Result<Self> {
        Ok(match choice(args, VIEW, &VIEW_VALUES)? {
            Some("tree") => Self::Tree,
            Some("sizes") => Self::Sizes,
            _ => Self::Flat,
        })
    }

    fn render(self) -> Option<RenderHint> {
        match self {
            Self::Flat => None,
            Self::Tree => Some(RenderHint::Tree),
            Self::Sizes => Some(RenderHint::Sizes),
        }
    }
}

/// Key of the `sort` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortKey {
    /// Path, component by component.
    Path,
    /// Name in NFC, by code points.
    Name,
    /// Modification time.
    Modified,
    /// Size; for directories with totals, the total size.
    Size,
    /// Type: directories, files, links, other entries.
    Type,
    /// Fuzzy rank, closest match first.
    Rank,
}

/// Order of the records: `sort` and `order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SortSpec {
    /// Sort key.
    pub(crate) key: SortKey,
    /// `order: desc`.
    pub(crate) descending: bool,
}

impl SortSpec {
    /// The order in which the walker delivers entries.
    pub(crate) const WALK_ORDER: Self = Self {
        key: SortKey::Path,
        descending: false,
    };

    fn direct(self, ordering: Ordering) -> Ordering {
        if self.descending {
            ordering.reverse()
        } else {
            ordering
        }
    }
}

/// Which walked entries become records.
///
/// The walker applies the depth range, the kind and the globs itself except
/// in `view: sizes`, which walks every entry to compute totals; the
/// selection repeats those checks, so it gives the same result in every
/// view.
pub(crate) struct Selection {
    /// Shallowest depth of a record.
    pub(crate) min_depth: u32,
    /// Deepest depth of a record; `None` is unlimited.
    pub(crate) max_depth: Option<u32>,
    /// Kind of a record.
    pub(crate) kind: KindFilter,
    /// Globs of `view: sizes`; `None` matches every entry.
    pub(crate) globs: Option<GlobSet>,
    /// File types of `file_type`, matched against the NFC name.
    pub(crate) names: Option<TypeFilter>,
    /// Query of `fuzzy`.
    pub(crate) fuzzy: Option<FuzzyQuery>,
}

impl Selection {
    fn walked(&self, entry: &walk::Entry) -> bool {
        let target = if entry.depth == 0 {
            Path::new(&entry.name)
        } else {
            entry.relative.as_path()
        };
        entry.depth >= self.min_depth
            && self.max_depth.is_none_or(|max| entry.depth <= max)
            && kind_matches(self.kind, entry)
            && walk::glob_matches(self.globs.as_ref(), target)
    }

    fn ranked(&self, mut record: Entry) -> Option<Entry> {
        if let Some(names) = &self.names
            && (record.kind != EntryType::File || !names.accepts(&record.name))
        {
            return None;
        }
        if let Some(query) = &self.fuzzy {
            record.rank = Some(f64::from(query.rank(&record.name)?));
        }
        Some(record)
    }
}

/// Root of a listing in its stored spelling.
pub(crate) struct Root {
    /// Path to walk.
    pub(crate) path: AbsPath,
    /// Stored name of the root when the given path differs from it in
    /// letter case.
    pub(crate) alias: Option<String>,
}

/// Root of a listing, with the spelling stored on the volume.
///
/// On a volume that ignores letter case or stores names decomposed, the
/// given path can differ from the stored one. The walk then starts at the
/// stored spelling, so that every `path` is canonical, and a notable entry
/// `case_alias` or `nfd_name` records the given path.
pub(crate) fn actual_root(ctx: &ExecCtx, fs: &dyn Fs, given: AbsPath) -> Root {
    let unchanged = |path: AbsPath| Root { path, alias: None };
    let foldable = fs
        .volume(&given)
        .is_ok_and(|volume| !volume.features.case_sensitive || volume.features.decomposes_names);
    if !foldable {
        return unchanged(given);
    }
    let Ok(actual) = fs.real_path(&given) else {
        return unchanged(given);
    };
    let given_text = given.to_language_string();
    let actual_text = actual.to_language_string();
    if given_text == actual_text {
        return unchanged(given);
    }
    let given_nfc: String = given_text.nfc().collect();
    let actual_nfc: String = actual_text.nfc().collect();
    let code = if given_nfc == actual_nfc {
        Notable::NfdName
    } else if given_nfc.to_lowercase() == actual_nfc.to_lowercase() {
        Notable::CaseAlias
    } else {
        return unchanged(given);
    };
    ctx.note(NotableEntry {
        path: Some(given_text),
        detail: Some(format!("stored as {actual_text}")),
        ..NotableEntry::new(code)
    });
    let alias = (code == Notable::CaseAlias)
        .then(|| {
            actual
                .file_name()
                .map(|name| name.to_string_lossy().nfc().collect())
        })
        .flatten();
    Root {
        path: actual,
        alias,
    }
}

/// Fails unless `path` is a directory or a link to one.
fn require_directory(fs: &dyn Fs, path: &AbsPath) -> Result<()> {
    let meta = fs.metadata(path, Follow::Follow)?;
    if meta.kind == FileKind::Directory {
        return Ok(());
    }
    let mut error = Error::from(FsError::new(
        FsErrorKind::NotADirectory,
        COMMAND,
        Some(path.as_path()),
    ));
    error.message = format!(
        "`{}` is not a directory: give a directory, or use `find` with `depth: 0` for one entry",
        path.to_language_string()
    );
    Err(error)
}

/// A walk with its selection and order.
pub(crate) struct Listing {
    /// Root of the walk.
    pub(crate) root: Root,
    /// Settings of the walker.
    pub(crate) options: WalkOptions,
    /// Value of `view`.
    pub(crate) view: View,
    /// Entries that become records.
    pub(crate) selection: Selection,
    /// Order of the records.
    pub(crate) sort: SortSpec,
}

impl Listing {
    /// Walks the root and returns the ordered `entry` records.
    ///
    /// Entries refused for lack of permission, unreadable entries, link
    /// loops, decomposed names and names that are not portable are reported
    /// with [`ExecCtx::note`].
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for a pattern that is not valid, with
    /// the code of the file system error when the root cannot be read, with
    /// `E_LIMIT` when the records exceed the record limit of the context,
    /// and with `E_CANCELLED` or `E_TIMEOUT` from the checkpoint.
    pub(crate) fn run<'a>(self, ctx: &'a ExecCtx) -> Result<StageOutput<'a>> {
        let fs = ctx.require_fs()?;
        let walker = Walker::new(self.walk_options(ctx))?;
        let early_stop = self.view == View::Flat && self.sort == SortSpec::WALK_ORDER;
        let mut collector = Collector {
            fs,
            selection: &self.selection,
            totals: (self.view == View::Sizes).then(Totals::default),
            items: Vec::new(),
            max_records: ctx.limits().max_records,
            stop_at: if early_stop { ctx.demand().limit } else { None },
            link_target: ctx.demand().fields.wants(LINK_TARGET),
            root_alias: self.root.alias.clone(),
            notes: Notes::default(),
            failure: None,
        };
        let summary = walker.run(
            fs,
            std::slice::from_ref(&self.root.path),
            &|| ctx.checkpoint(),
            |item| collector.accept(item),
        )?;
        let (items, notes) = collector.finish()?;
        report(ctx, &summary, &notes);
        let records = order(items, self.view, self.sort)
            .into_iter()
            .map(|item| Record::from(item.record))
            .collect();
        let output = StageOutput::records(RecordStream::from_records(records));
        Ok(match self.view.render() {
            Some(hint) => output.with_render(hint),
            None => output,
        })
    }

    fn walk_options(&self, ctx: &ExecCtx) -> WalkOptions {
        let mut options = self.options.clone();
        options.metadata = true;
        options.threads = usize::try_from(ctx.limits().threads).unwrap_or(1).max(1);
        if self.view == View::Sizes {
            options.min_depth = 0;
            options.max_depth = None;
            options.kind = KindFilter::Any;
            options.globs.clear();
        }
        options
    }
}

/// Selected entry with the path used for ordering.
struct Item {
    record: Entry,
    relative: PathBuf,
}

/// Paths of a kind of notable entry: the number and the first paths.
#[derive(Default)]
struct Sample {
    count: u64,
    paths: Vec<AbsPath>,
}

impl Sample {
    fn push(&mut self, path: AbsPath) {
        self.count = self.count.saturating_add(1);
        if self.paths.len() < walk::SAMPLE_LIMIT {
            self.paths.push(path);
        }
    }
}

/// Notable entries collected during a walk, besides those of
/// [`WalkSummary`].
#[derive(Default)]
struct Notes {
    unreadable: Sample,
    nfd: Sample,
    portability: Sample,
}

/// Consumer of the walk.
struct Collector<'c> {
    fs: &'c dyn Fs,
    selection: &'c Selection,
    totals: Option<Totals>,
    items: Vec<Item>,
    max_records: u64,
    stop_at: Option<u64>,
    link_target: bool,
    root_alias: Option<String>,
    notes: Notes,
    failure: Option<Error>,
}

impl Collector<'_> {
    fn accept(&mut self, item: WalkItem) -> ControlFlow<()> {
        match item {
            WalkItem::Entry(entry) => self.entry(entry),
            WalkItem::Unreadable(unreadable) => {
                if unreadable.error.kind != FsErrorKind::PermissionDenied {
                    self.notes.unreadable.push(unreadable.path);
                }
                ControlFlow::Continue(())
            }
        }
    }

    fn entry(&mut self, mut entry: walk::Entry) -> ControlFlow<()> {
        let stored = entry.meta.take();
        let meta = match stored {
            Some(meta) => meta,
            None => match self.fs.metadata(&entry.path, follow_of(&entry)) {
                Ok(meta) => meta,
                Err(_) => {
                    self.notes.unreadable.push(entry.path);
                    return ControlFlow::Continue(());
                }
            },
        };
        let record = self
            .selection
            .walked(&entry)
            .then(|| self.record(&entry, &meta))
            .and_then(|record| self.selection.ranked(record));
        let output = record.as_ref().map(|_| self.items.len());
        if let Some(totals) = self.totals.as_mut() {
            totals.visit(&entry, &meta, output, &mut self.items);
        }
        match record {
            Some(record) => self.keep(entry, record),
            None => ControlFlow::Continue(()),
        }
    }

    fn record(&self, entry: &walk::Entry, meta: &FileMeta) -> Entry {
        let mut record = entry_record(&entry.path, meta);
        if is_link(entry.kind) {
            record.link_type = link_type(entry.kind);
            if self.link_target {
                record.link_target = link_target(self.fs, &entry.path);
            }
        }
        record.depth = Some(u64::from(entry.depth));
        record.relative = Some(relative_text(entry));
        record.portability = portability(&entry.name);
        if entry.depth == 0 {
            record.case_alias.clone_from(&self.root_alias);
        }
        record
    }

    fn keep(&mut self, entry: walk::Entry, record: Entry) -> ControlFlow<()> {
        let held = u64::try_from(self.items.len()).unwrap_or(u64::MAX);
        if held >= self.max_records {
            self.failure = Some(limit_error(self.max_records));
            return ControlFlow::Break(());
        }
        if record.raw_name.is_some() && entry.name.to_str().is_some() {
            self.notes.nfd.push(entry.path.clone());
        }
        if record.portability.is_some() {
            self.notes.portability.push(entry.path.clone());
        }
        self.items.push(Item {
            record,
            relative: entry.relative,
        });
        let held = held.saturating_add(1);
        if self.stop_at.is_some_and(|limit| held >= limit) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn finish(mut self) -> Result<(Vec<Item>, Notes)> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if let Some(totals) = self.totals.as_mut() {
            totals.close(0, &mut self.items);
        }
        Ok((self.items, self.notes))
    }
}

fn limit_error(max: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "the listing holds more than {max} entries: narrow it with `depth`, `glob` or `exclude`"
        ),
    )
    .with_actual(serde_json::json!(max))
}

fn follow_of(entry: &walk::Entry) -> Follow {
    if entry.target_kind.is_some() {
        Follow::Follow
    } else {
        Follow::NoFollow
    }
}

fn is_link(kind: FileKind) -> bool {
    matches!(kind, FileKind::Symlink | FileKind::Junction)
}

fn kind_matches(filter: KindFilter, entry: &walk::Entry) -> bool {
    let effective = entry.effective_kind();
    match filter {
        KindFilter::File => effective == FileKind::File,
        KindFilter::Directory => effective == FileKind::Directory,
        KindFilter::Link => is_link(entry.kind) && entry.target_kind.is_none(),
        KindFilter::Any => true,
    }
}

/// Target of a link as an absolute path; a relative target is resolved
/// against the directory of the link.
fn link_target(fs: &dyn Fs, path: &AbsPath) -> Option<nurt_contracts::value::Path> {
    let target = fs.read_link(path).ok()?;
    let absolute = if target.is_absolute() {
        target
    } else {
        path.parent()?.as_path().join(target)
    };
    AbsPath::new(absolute)
        .ok()
        .map(|target| path_field(&target))
}

fn relative_text(entry: &walk::Entry) -> String {
    if entry.depth == 0 {
        ROOT_RELATIVE.to_owned()
    } else {
        entry.relative_string().nfc().collect()
    }
}

/// Portability problems of a stored name; `None` when the name is valid on
/// every supported system.
fn portability(name: &OsStr) -> Option<Vec<String>> {
    let Some(text) = name.to_str() else {
        return Some(vec![INVALID_UNICODE.to_owned()]);
    };
    if text.is_empty() {
        return None;
    }
    let mut problems = Vec::new();
    if windows_reserved(text) {
        problems.push(WINDOWS_RESERVED_NAME.to_owned());
    }
    if text
        .chars()
        .any(|c| c < ' ' || WINDOWS_FORBIDDEN.contains(&c))
    {
        problems.push(WINDOWS_INVALID_CHARACTER.to_owned());
    }
    if text.ends_with(WINDOWS_TRAILING) {
        problems.push(WINDOWS_TRAILING_CHARACTER.to_owned());
    }
    (!problems.is_empty()).then_some(problems)
}

/// Returns `true` for a device name Windows reserves, with or without an
/// extension, such as `NUL` or `com1.txt`.
fn windows_reserved(name: &str) -> bool {
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    WINDOWS_DEVICES.contains(&base.as_str())
        || WINDOWS_NUMBERED_DEVICES.iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|digit| {
                digit.len() == 1 && digit.bytes().all(|byte| (b'1'..=b'9').contains(&byte))
            })
        })
}

/// Directory totals of `view: sizes`.
///
/// Entries arrive depth first, so the open directories form a stack: an
/// entry at depth `d` closes every open directory at depth `d` or deeper.
/// A closed directory passes its totals to its parent. A file with several
/// hard links is counted once, at its first occurrence.
#[derive(Default)]
struct Totals {
    open: Vec<OpenDir>,
    seen: HashSet<FileId>,
}

struct OpenDir {
    depth: u32,
    size: u64,
    files: u64,
    output: Option<usize>,
}

impl Totals {
    fn visit(
        &mut self,
        entry: &walk::Entry,
        meta: &FileMeta,
        output: Option<usize>,
        items: &mut [Item],
    ) {
        self.close(entry.depth, items);
        if entry.is_dir() {
            self.open.push(OpenDir {
                depth: entry.depth,
                size: 0,
                files: 0,
                output,
            });
            return;
        }
        if meta.links > 1 && !self.seen.insert(meta.file_id) {
            return;
        }
        if let Some(parent) = self.open.last_mut() {
            parent.size = parent.size.saturating_add(meta.size);
            if entry.effective_kind() == FileKind::File {
                parent.files = parent.files.saturating_add(1);
            }
        }
    }

    fn close(&mut self, depth: u32, items: &mut [Item]) {
        while self.open.last().is_some_and(|dir| dir.depth >= depth) {
            let Some(dir) = self.open.pop() else {
                break;
            };
            if let Some(item) = dir.output.and_then(|index| items.get_mut(index)) {
                item.record.total_size = Some(Size::from_bytes(dir.size));
                item.record.file_count = Some(dir.files);
            }
            if let Some(parent) = self.open.last_mut() {
                parent.size = parent.size.saturating_add(dir.size);
                parent.files = parent.files.saturating_add(dir.files);
            }
        }
    }
}

fn order(mut items: Vec<Item>, view: View, sort: SortSpec) -> Vec<Item> {
    if view == View::Flat {
        items.sort_by(|a, b| {
            sort.direct(
                compare_key(sort.key, &a.record, &b.record)
                    .then_with(|| a.relative.cmp(&b.relative)),
            )
        });
        return items;
    }
    tree_order(items, sort)
}

fn compare_key(key: SortKey, a: &Entry, b: &Entry) -> Ordering {
    match key {
        SortKey::Path => Ordering::Equal,
        SortKey::Name => a.name.cmp(&b.name),
        SortKey::Modified => a.modified.cmp(&b.modified),
        SortKey::Size => listed_size(a).cmp(&listed_size(b)),
        SortKey::Type => type_rank(a.kind).cmp(&type_rank(b.kind)),
        SortKey::Rank => b
            .rank
            .unwrap_or_default()
            .total_cmp(&a.rank.unwrap_or_default()),
    }
}

fn listed_size(entry: &Entry) -> Size {
    entry.total_size.unwrap_or(entry.size)
}

fn type_rank(kind: EntryType) -> u8 {
    match kind {
        EntryType::Directory => 0,
        EntryType::File => 1,
        EntryType::Link => 2,
        _ => 3,
    }
}

/// Node of the path tree used by the depth-first order.
#[derive(Default)]
struct Node {
    item: Option<usize>,
    name: OsString,
    children: Vec<usize>,
}

/// Depth-first order: every directory before its contents, the entries of
/// one directory by `sort`. Directories missing from the records (filtered
/// out by `type` or `glob`) keep their place, ordered by name after the
/// entries that are records.
fn tree_order(items: Vec<Item>, sort: SortSpec) -> Vec<Item> {
    let mut nodes = vec![Node::default()];
    let mut lookup: HashMap<(usize, OsString), usize> = HashMap::new();
    for (position, item) in items.iter().enumerate() {
        let mut node = 0;
        for component in item.relative.components() {
            let Component::Normal(part) = component else {
                continue;
            };
            let key = (node, part.to_os_string());
            node = if let Some(&child) = lookup.get(&key) {
                child
            } else {
                let child = nodes.len();
                nodes.push(Node {
                    item: None,
                    name: part.to_os_string(),
                    children: Vec::new(),
                });
                if let Some(parent) = nodes.get_mut(node) {
                    parent.children.push(child);
                }
                lookup.insert(key, child);
                child
            };
        }
        if let Some(target) = nodes.get_mut(node) {
            target.item = Some(position);
        }
    }
    let mut sequence = Vec::with_capacity(items.len());
    let mut stack = vec![0];
    while let Some(node) = stack.pop() {
        let Some(current) = nodes.get_mut(node) else {
            continue;
        };
        let mut children = std::mem::take(&mut current.children);
        if let Some(position) = current.item {
            sequence.push(position);
        }
        children.sort_by(|&a, &b| sibling_order(&nodes, &items, sort, a, b));
        stack.extend(children.into_iter().rev());
    }
    let mut slots: Vec<Option<Item>> = items.into_iter().map(Some).collect();
    sequence
        .into_iter()
        .filter_map(|position| slots.get_mut(position).and_then(Option::take))
        .collect()
}

fn sibling_order(nodes: &[Node], items: &[Item], sort: SortSpec, a: usize, b: usize) -> Ordering {
    let (Some(left), Some(right)) = (nodes.get(a), nodes.get(b)) else {
        return Ordering::Equal;
    };
    let record = |node: &Node| {
        node.item
            .and_then(|position| items.get(position))
            .map(|item| &item.record)
    };
    match (record(left), record(right)) {
        (Some(x), Some(y)) => {
            sort.direct(compare_key(sort.key, x, y).then_with(|| left.name.cmp(&right.name)))
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => sort.direct(left.name.cmp(&right.name)),
    }
}

/// Reports the notable entries of a walk.
fn report(ctx: &ExecCtx, summary: &WalkSummary, notes: &Notes) {
    if summary.permission_denied > 0 {
        ctx.note(sampled(
            Notable::PermissionDeniedSkipped,
            summary.permission_denied,
            &summary.permission_denied_sample,
            None,
        ));
    }
    if summary.link_loops > 0 {
        ctx.note(sampled(
            Notable::LinkLoopSkipped,
            summary.link_loops,
            &summary.link_loop_sample,
            Some(LINK_LOOP_DETAIL),
        ));
    }
    let collected = [
        (Notable::SourceUnavailable, &notes.unreadable),
        (Notable::NfdName, &notes.nfd),
        (Notable::Portability, &notes.portability),
    ];
    for (code, sample) in collected {
        if sample.count > 0 {
            ctx.note(sampled(code, sample.count, &sample.paths, None));
        }
    }
}

fn sampled(code: Notable, count: u64, paths: &[AbsPath], what: Option<&str>) -> NotableEntry {
    let listed = paths
        .iter()
        .map(AbsPath::to_language_string)
        .collect::<Vec<_>>()
        .join(SAMPLE_SEPARATOR);
    let detail = match what {
        Some(what) => format!("{what}; sample: {listed}"),
        None => format!("sample: {listed}"),
    };
    NotableEntry {
        path: paths.first().map(AbsPath::to_language_string),
        count: Some(count),
        detail: Some(detail),
        ..NotableEntry::new(code)
    }
}

/// The directory of `path`, resolved as by
/// [`resolve`](crate::common::path::resolve).
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` without a value and as
/// [`path_arg`].
pub(crate) fn root_param(args: &BoundArgs, ctx: &ExecCtx) -> Result<AbsPath> {
    path_arg(args, PATH, ctx)?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!("`{}` needs a directory for `{PATH}`", args.command()),
        )
    })
}

/// Static facts of a listing: the path it reads.
pub(crate) fn static_info(args: &BoundArgs) -> StaticInfo {
    let paths = match args.value(PATH) {
        Some(Value::Path(text) | Value::Text(text)) => vec![text.clone()],
        _ => Vec::new(),
    };
    StaticInfo {
        paths,
        ..StaticInfo::default()
    }
}

/// Value of an enumeration parameter; `None` without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value and with
/// `E_UNKNOWN_VALUE` for a value not in `values`.
pub(crate) fn choice<'a>(
    args: &'a BoundArgs,
    name: &str,
    values: &[&str],
) -> Result<Option<&'a str>> {
    let text = match args.value(name) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) => text.as_str(),
        Some(_) => {
            return Err(type_error(
                args.command(),
                name,
                "a value of the enumeration",
            ));
        }
    };
    if values.contains(&text) {
        Ok(Some(text))
    } else {
        Err(unknown_value(name, text, values))
    }
}

/// `order: desc`.
///
/// # Errors
///
/// As [`choice`].
pub(crate) fn descending(args: &BoundArgs) -> Result<bool> {
    Ok(choice(args, ORDER, &ORDER_VALUES)? == Some("desc"))
}

/// Value of `type`; `default` without a value.
///
/// # Errors
///
/// As [`choice`].
pub(crate) fn kind_param(args: &BoundArgs, default: KindFilter) -> Result<KindFilter> {
    Ok(match choice(args, TYPE, &KIND_VALUES)? {
        Some("file") => KindFilter::File,
        Some("directory") => KindFilter::Directory,
        Some("link") => KindFilter::Link,
        Some("any") => KindFilter::Any,
        _ => default,
    })
}

/// Value of `depth`; `None` without a value, which is unlimited.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an integer and with
/// `E_INVALID_VALUE` for a depth below `minimum`.
pub(crate) fn depth_param(args: &BoundArgs, minimum: u32) -> Result<Option<u32>> {
    match args.value(DEPTH) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(depth)) if *depth < i64::from(minimum) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{DEPTH}` must be at least {minimum}, not {depth}"),
        )
        .with_found(depth.to_string())
        .with_expected(serde_json::json!(minimum))),
        Some(Value::Int(depth)) => Ok(Some(u32::try_from(*depth).unwrap_or(u32::MAX))),
        Some(_) => Err(type_error(args.command(), DEPTH, "an integer")),
    }
}

/// `E_TYPE` for a parameter value of another type.
pub(crate) fn type_error(command: &str, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{command}` expects {expected} for `{name}`"),
    )
}

/// `E_UNKNOWN_VALUE` listing the accepted values.
pub(crate) fn unknown_value(name: &str, found: &str, expected: &[&str]) -> Error {
    Error::new(
        ErrorCode::UnknownValue,
        format!(
            "`{found}` is not a value of `{name}`: use one of {}",
            expected.join(", ")
        ),
    )
    .with_found(found)
    .with_expected(serde_json::json!(expected))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nurt_contracts::fs::{FsWrite, ScopePurpose, TxScope};
    use nurt_contracts::journal::OpId;
    use nurt_contracts::record::LinkType;
    use nurt_contracts::stage::{ArgValue, Limits};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::{Fault, VolumeConfig, test_path};

    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).expect("test paths are absolute")
    }

    struct Run {
        entries: Vec<Entry>,
        notes: Vec<NotableEntry>,
        render: Option<RenderHint>,
    }

    fn args(path: &str, given: &[(&str, Value)]) -> BoundArgs {
        given.iter().fold(
            BoundArgs::new(COMMAND).with(
                PATH,
                ArgValue::Value(Value::Path(abs(path).to_language_string())),
                false,
            ),
            |args, (name, value)| args.with(*name, ArgValue::Value(value.clone()), false),
        )
    }

    fn choice_value(text: &str) -> Value {
        Value::Enum(text.to_owned())
    }

    fn run_with(ctx: &ExecCtx, args: &BoundArgs) -> Result<Run> {
        let output = List.execute(args, StageInput::none(), ctx)?;
        let render = output.render;
        let entries = StageInput::from(output)
            .into_records()?
            .map(|item| item.and_then(|record| Entry::try_from(record).map_err(Error::from)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Run {
            entries,
            notes: ctx.take_notes(),
            render,
        })
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        run_with(&test.ctx(), args)
    }

    fn relatives(run: &Run) -> Vec<String> {
        run.entries
            .iter()
            .map(|entry| entry.relative.clone().unwrap_or_default())
            .collect()
    }

    fn tree() -> TestContext {
        let test = TestContext::new();
        let fs = test.fs();
        fs.create_dir_all(&abs("/app/src/bin")).unwrap();
        fs.create_dir_all(&abs("/app/docs")).unwrap();
        fs.write_file(&abs("/app/src/lib.rs"), b"pub fn a() {}\n")
            .unwrap();
        fs.write_file(&abs("/app/src/bin/main.rs"), b"fn main() {}\n")
            .unwrap();
        fs.write_file(&abs("/app/docs/guide.md"), &[b'x'; 100])
            .unwrap();
        fs.write_file(&abs("/app/README.md"), b"# app\n").unwrap();
        fs.write_file(&abs("/app/.env"), b"KEY=1\n").unwrap();
        test
    }

    #[test]
    fn lists_direct_children_by_name() {
        let test = tree();
        let listed = run(&test, &args("/app", &[])).unwrap();
        assert_eq!(relatives(&listed), [".env", "README.md", "docs", "src"]);
        assert!(listed.entries.iter().all(|entry| entry.depth == Some(1)));
        assert_eq!(listed.render, None);
        let docs = &listed.entries[2];
        assert_eq!(docs.kind, EntryType::Directory);
        assert_eq!(docs.path.as_str(), abs("/app/docs").to_language_string());
        assert!(docs.etag.is_some());
        assert!(listed.notes.is_empty());
    }

    #[test]
    fn depth_reaches_nested_entries_in_path_order() {
        let test = tree();
        let listed = run(
            &test,
            &args(
                "/app",
                &[(DEPTH, Value::Int(3)), (SORT, choice_value("name"))],
            ),
        )
        .unwrap();
        assert_eq!(
            relatives(&listed),
            [
                ".env",
                "README.md",
                "src/bin",
                "docs",
                "docs/guide.md",
                "src/lib.rs",
                "src/bin/main.rs",
                "src"
            ]
        );
        let tree_view = run(
            &test,
            &args(
                "/app",
                &[(DEPTH, Value::Int(3)), (VIEW, choice_value("tree"))],
            ),
        )
        .unwrap();
        assert_eq!(
            relatives(&tree_view),
            [
                ".env",
                "README.md",
                "docs",
                "docs/guide.md",
                "src",
                "src/bin",
                "src/bin/main.rs",
                "src/lib.rs"
            ]
        );
        assert_eq!(tree_view.render, Some(RenderHint::Tree));
        assert_eq!(tree_view.entries[6].depth, Some(3));
    }

    #[test]
    fn hidden_entries_can_be_left_out() {
        let test = tree();
        let listed = run(&test, &args("/app", &[("hidden", Value::Bool(false))])).unwrap();
        assert_eq!(relatives(&listed), ["README.md", "docs", "src"]);
    }

    #[test]
    fn type_selects_the_kind_of_entries() {
        let test = tree();
        let files = run(&test, &args("/app", &[(TYPE, choice_value("file"))])).unwrap();
        assert_eq!(relatives(&files), [".env", "README.md"]);
        let directories = run(&test, &args("/app", &[(TYPE, choice_value("directory"))])).unwrap();
        assert_eq!(relatives(&directories), ["docs", "src"]);
    }

    #[test]
    fn sort_by_size_and_type_with_order() {
        let test = tree();
        let by_size = run(
            &test,
            &args(
                "/app",
                &[
                    (TYPE, choice_value("file")),
                    (SORT, choice_value("size")),
                    (ORDER, choice_value("desc")),
                ],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&by_size), ["README.md", ".env"]);
        let by_type = run(&test, &args("/app", &[(SORT, choice_value("type"))])).unwrap();
        assert_eq!(relatives(&by_type), ["docs", "src", ".env", "README.md"]);
    }

    #[test]
    fn sort_by_modification_time() {
        let test = TestContext::new();
        let fs = test.fs();
        fs.write_file(&abs("/logs/b.log"), b"1").unwrap();
        test.clock().advance(Duration::from_secs(60));
        fs.write_file(&abs("/logs/a.log"), b"2").unwrap();
        test.clock().advance(Duration::from_secs(60));
        fs.write_file(&abs("/logs/c.log"), b"3").unwrap();
        let listed = run(
            &test,
            &args(
                "/logs",
                &[
                    (SORT, choice_value("modified")),
                    (ORDER, choice_value("desc")),
                ],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&listed), ["c.log", "a.log", "b.log"]);
    }

    #[test]
    fn sizes_view_totals_whole_subtrees() {
        let test = tree();
        let listed = run(&test, &args("/app", &[(VIEW, choice_value("sizes"))])).unwrap();
        assert_eq!(listed.render, Some(RenderHint::Sizes));
        assert_eq!(relatives(&listed), [".env", "README.md", "docs", "src"]);
        let src = &listed.entries[3];
        assert_eq!(src.total_size, Some(Size::from_bytes(14 + 13)));
        assert_eq!(src.file_count, Some(2));
        assert_eq!(listed.entries[2].total_size, Some(Size::from_bytes(100)));
        assert_eq!(listed.entries[0].total_size, None);
        let largest = run(
            &test,
            &args(
                "/app",
                &[
                    (VIEW, choice_value("sizes")),
                    (SORT, choice_value("size")),
                    (ORDER, choice_value("desc")),
                ],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&largest), ["docs", "src", "README.md", ".env"]);
    }

    // The virtual file system needs a transaction scope to create a hard
    // link; no journal takes part in this test.
    #[allow(clippy::disallowed_methods)]
    #[test]
    fn sizes_view_counts_hard_links_once() {
        let test = TestContext::new();
        let fs = test.fs();
        fs.write_file(&abs("/data/one/a.bin"), &[0; 40]).unwrap();
        fs.create_dir_all(&abs("/data/two")).unwrap();
        let scope = TxScope::new_for_journal(OpId(1), ScopePurpose::Operation);
        fs.hard_link(&scope, &abs("/data/one/a.bin"), &abs("/data/two/a.bin"))
            .unwrap();
        let listed = run(&test, &args("/data", &[(VIEW, choice_value("sizes"))])).unwrap();
        let totals: Vec<_> = listed
            .entries
            .iter()
            .map(|entry| (entry.total_size, entry.file_count))
            .collect();
        assert_eq!(
            totals,
            [
                (Some(Size::from_bytes(40)), Some(1)),
                (Some(Size::from_bytes(0)), Some(0))
            ]
        );
    }

    #[test]
    fn tree_order_keeps_missing_directories_in_place() {
        let test = tree();
        let listed = run(
            &test,
            &args(
                "/app",
                &[
                    (DEPTH, Value::Int(3)),
                    (TYPE, choice_value("file")),
                    (VIEW, choice_value("tree")),
                    (SORT, choice_value("size")),
                ],
            ),
        )
        .unwrap();
        assert_eq!(
            relatives(&listed),
            [
                ".env",
                "README.md",
                "docs/guide.md",
                "src/lib.rs",
                "src/bin/main.rs"
            ]
        );
    }

    #[test]
    fn links_carry_their_kind_and_target() {
        let test = tree();
        test.fs()
            .symlink(abs("/app/src").as_path(), &abs("/app/code"))
            .unwrap();
        let listed = run(&test, &args("/app", &[(TYPE, choice_value("link"))])).unwrap();
        assert_eq!(relatives(&listed), ["code"]);
        let link = &listed.entries[0];
        assert_eq!(link.kind, EntryType::Link);
        assert_eq!(link.link_type, Some(LinkType::Symbolic));
        assert_eq!(
            link.link_target
                .as_ref()
                .map(|target| target.as_str().to_owned()),
            Some(abs("/app/src").to_language_string())
        );
    }

    #[test]
    fn unreadable_directories_are_reported_as_notable() {
        let test = tree();
        test.fs().inject(
            Fault::error(&abs("/app/src"), "read_dir", FsErrorKind::PermissionDenied).persistent(),
        );
        let listed = run(&test, &args("/app", &[(DEPTH, Value::Int(2))])).unwrap();
        assert!(relatives(&listed).contains(&"docs/guide.md".to_owned()));
        assert!(!relatives(&listed).contains(&"src/lib.rs".to_owned()));
        let note = listed
            .notes
            .iter()
            .find(|note| note.code == Notable::PermissionDeniedSkipped)
            .unwrap();
        assert_eq!(note.count, Some(1));
        assert_eq!(note.path, Some(abs("/app/src").to_language_string()));
        assert!(note.detail.as_deref().unwrap().contains("sample: "));
    }

    #[test]
    fn decomposed_names_are_normalised_and_noted() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/cafe\u{301}.txt"), b"")
            .unwrap();
        let listed = run(&test, &args("/data", &[])).unwrap();
        assert_eq!(listed.entries[0].name, "caf\u{e9}.txt");
        assert!(listed.entries[0].raw_name.is_some());
        assert_eq!(listed.notes.len(), 1);
        assert_eq!(listed.notes[0].code, Notable::NfdName);
        assert_eq!(listed.notes[0].count, Some(1));
    }

    #[test]
    fn exclusions_match_decomposed_names_in_composed_form() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/cafe\u{301}.txt"), b"")
            .unwrap();
        test.fs().write_file(&abs("/data/tea.txt"), b"").unwrap();
        let listed = run(
            &test,
            &args(
                "/data",
                &[(
                    "exclude",
                    Value::List(vec![Value::Glob("caf\u{e9}.txt".to_owned())]),
                )],
            ),
        )
        .unwrap();
        assert_eq!(relatives(&listed), ["tea.txt"]);
    }

    #[test]
    fn names_that_are_not_portable_are_flagged() {
        let test = TestContext::new();
        test.fs().write_file(&abs("/data/con.txt"), b"").unwrap();
        test.fs().write_file(&abs("/data/ok.txt"), b"").unwrap();
        let listed = run(&test, &args("/data", &[])).unwrap();
        assert_eq!(
            listed.entries[0].portability,
            Some(vec![WINDOWS_RESERVED_NAME.to_owned()])
        );
        assert_eq!(listed.entries[1].portability, None);
        assert_eq!(listed.notes[0].code, Notable::Portability);
    }

    #[test]
    fn portability_problems_are_detected() {
        let check = |name: &str| portability(OsStr::new(name)).unwrap_or_default();
        assert_eq!(check("NUL"), [WINDOWS_RESERVED_NAME]);
        assert_eq!(check("lpt9.log"), [WINDOWS_RESERVED_NAME]);
        assert!(check("com10").is_empty());
        assert!(check("console").is_empty());
        assert_eq!(check("a?b"), [WINDOWS_INVALID_CHARACTER]);
        assert_eq!(check("tab\there"), [WINDOWS_INVALID_CHARACTER]);
        assert_eq!(check("notes."), [WINDOWS_TRAILING_CHARACTER]);
        assert_eq!(
            check("aux "),
            [WINDOWS_RESERVED_NAME, WINDOWS_TRAILING_CHARACTER]
        );
        assert!(check("report.final.txt").is_empty());
        assert_eq!(portability(OsStr::new("")), None);
    }

    #[cfg(unix)]
    #[test]
    fn names_that_are_not_unicode_keep_their_bytes() {
        use std::os::unix::ffi::OsStrExt;

        let test = TestContext::new();
        let name = OsStr::from_bytes(b"data\xff.bin");
        let path = AbsPath::new(abs("/raw").as_path().join(name)).unwrap();
        test.fs().write_file(&path, b"").unwrap();
        let listed = run(&test, &args("/raw", &[])).unwrap();
        let entry = &listed.entries[0];
        assert_eq!(
            entry.raw_name.as_ref().map(|raw| raw.as_slice().to_vec()),
            Some(b"data\xff.bin".to_vec())
        );
        assert_eq!(entry.portability, Some(vec![INVALID_UNICODE.to_owned()]));
    }

    #[test]
    fn case_alias_of_the_root_is_noted_on_volumes_that_ignore_case() {
        let test = TestContext::new();
        let mut config = VolumeConfig::default();
        config.features.case_sensitive = false;
        test.fs().add_volume(&abs("/ci"), config).unwrap();
        test.fs().write_file(&abs("/ci/Docs/a.md"), b"a").unwrap();
        let listed = run(&test, &args("/ci/docs", &[])).unwrap();
        assert_eq!(
            listed.entries[0].path.as_str(),
            abs("/ci/Docs/a.md").to_language_string()
        );
        let note = &listed.notes[0];
        assert_eq!(note.code, Notable::CaseAlias);
        assert_eq!(note.path, Some(abs("/ci/docs").to_language_string()));
    }

    #[test]
    fn root_must_be_an_existing_directory() {
        let test = tree();
        let file = run(&test, &args("/app/README.md", &[])).err().unwrap();
        assert!(file.message.contains("is not a directory"));
        let missing = run(&test, &args("/nowhere", &[])).err().unwrap();
        assert_eq!(missing.code(), ErrorCode::NotFound);
        let without = BoundArgs::new(COMMAND);
        let error = List
            .execute(&without, StageInput::none(), &test.ctx())
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn relative_root_without_base_is_rejected() {
        let test = tree();
        let given = BoundArgs::new(COMMAND).with(
            PATH,
            ArgValue::Value(Value::Path("app".to_owned())),
            false,
        );
        let error = List
            .execute(&given, StageInput::none(), &test.ctx())
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::PathRelative);
    }

    #[test]
    fn parameter_values_are_checked() {
        let test = tree();
        let unknown = run(&test, &args("/app", &[(SORT, choice_value("rank"))]))
            .err()
            .unwrap();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
        assert_eq!(unknown.found.as_deref(), Some("rank"));
        let shallow = run(&test, &args("/app", &[(DEPTH, Value::Int(0))]))
            .err()
            .unwrap();
        assert_eq!(shallow.code(), ErrorCode::InvalidValue);
        let wrong = run(&test, &args("/app", &[(DEPTH, Value::Bool(true))]))
            .err()
            .unwrap();
        assert_eq!(wrong.code(), ErrorCode::Type);
        let view = run(&test, &args("/app", &[(VIEW, Value::Int(1))]))
            .err()
            .unwrap();
        assert_eq!(view.code(), ErrorCode::Type);
    }

    #[test]
    fn record_limit_ends_the_listing() {
        let test = tree();
        let ctx = test
            .builder()
            .limits(Limits {
                max_records: 2,
                ..Limits::default()
            })
            .build();
        let error = run_with(&ctx, &args("/app", &[])).err().unwrap();
        assert_eq!(error.code(), ErrorCode::Limit);
    }

    #[test]
    fn descriptor_names_a_source_stage() {
        let descriptor = List.descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert!(descriptor.source);
        assert_eq!(descriptor.input, InputKinds::None);
        assert!(List.as_read().is_some());
        let info = static_info(&args("/app", &[]));
        assert_eq!(info.paths, [abs("/app").to_language_string()]);
        assert!(static_info(&BoundArgs::new(COMMAND)).paths.is_empty());
    }

    #[test]
    fn walked_entries_match_the_kind_filter() {
        let entry = |kind, target_kind| walk::Entry {
            path: abs("/a/b"),
            relative: PathBuf::from("b"),
            name: "b".into(),
            depth: 1,
            root: 0,
            kind,
            target_kind,
            meta: None,
        };
        let resolved = entry(FileKind::Symlink, Some(FileKind::File));
        assert!(kind_matches(KindFilter::File, &resolved));
        assert!(!kind_matches(KindFilter::Link, &resolved));
        let dangling = entry(FileKind::Symlink, None);
        assert!(kind_matches(KindFilter::Link, &dangling));
        assert!(!kind_matches(KindFilter::Directory, &dangling));
        assert!(kind_matches(
            KindFilter::Any,
            &entry(FileKind::Special, None)
        ));
    }
}
