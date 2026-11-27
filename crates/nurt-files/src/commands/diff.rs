// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::diff
// Responsibility:   `diff`: line differences of files, directory trees and contents as `hunk`.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `diff`: line differences of files, directory trees and contents as `hunk`
//! records.
//!
//! The source form compares `from` (old) with `to` (new). Two files give the
//! hunks of their difference; two directories are compared recursively by
//! relative path, and a file that exists on one side only is reported as
//! `added` or `deleted`. A deleted file and an added file with the same
//! non-empty content are reported once as `renamed`. A file compared with a
//! directory is compared with the file of the same name in that directory.
//!
//! The stage form compares each input record (new) with `against` (old): an
//! `entry` record names a file, a `content` record carries the text itself.
//! `against` is a file, a directory (the file of the same name in it is
//! used, and an input directory is compared with it recursively) or a named
//! result whose records are matched to the input by `path`.
//!
//! Texts are compared after decoding, so files in different encodings are
//! compared by their characters and the difference of encodings is reported
//! as `encoding_changed`. Files with binary content are compared by content
//! hash and reported as one record with `binary: true`. The hunks equal those
//! of GNU `diff -u` for the same texts and context.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, FileKind, Follow, Fs};
use nurt_contracts::record::{Hunk as HunkRecord, HunkStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_text::diff::{Hunk, unified_diff};
use nurt_text::hash::{HashAlgorithm, Hasher};

use crate::common::etag::{Observed, observe};
use crate::common::notable::at_path;
use crate::common::params::EncodingParam;
use crate::common::path::{path_arg, path_field, record_path, resolve};
use crate::common::text::read_text;

/// Name of the command in the catalog.
const COMMAND: &str = "diff";

/// Name of the `from` parameter.
const FROM: &str = "from";

/// Name of the `to` parameter.
const TO: &str = "to";

/// Name of the `against` parameter.
const AGAINST: &str = "against";

/// Name of the `context` parameter.
const CONTEXT: &str = "context";

/// Name of the `ignore_whitespace` parameter.
const IGNORE_WHITESPACE: &str = "ignore_whitespace";

/// Context lines when `context` has no value, as in the catalog.
const DEFAULT_CONTEXT: usize = 3;

/// Field of `content` records that holds the text.
const TEXT_FIELD: &str = "text";

/// Field of `content` records that holds the encoding.
const ENCODING_FIELD: &str = "encoding";

/// Hexadecimal digits of the SHA-256 hash of the hunk lines kept in the
/// `fingerprint` field.
const FINGERPRINT_DIGITS: usize = 6;

/// Separator of the components of relative paths within compared trees.
const SEPARATOR: char = '/';

/// Separator of the values before and after in the `detail` of notable
/// entries.
const DETAIL_ARROW: &str = " -> ";

/// Stage of `diff`: source form with `from` and `to`, stage form on `entry`
/// and `content` records.
pub(crate) struct Diff;

impl StageImpl for Diff {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry, RecordKind::Content]),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Diff {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let settings = Settings::from_args(args)?;
        if input.is_none() {
            if args.is_given(AGAINST) {
                return Err(form_error(AGAINST, "the stage form"));
            }
            let from = required_path(args, FROM, ctx)?;
            let to = required_path(args, TO, ctx)?;
            let jobs = source_jobs(ctx, &from, &to)?;
            return Ok(StageOutput::records(job_stream(jobs, settings, ctx)));
        }
        for name in [FROM, TO] {
            if args.is_given(name) {
                return Err(form_error(name, "the source form"));
            }
        }
        let against = Against::from_args(args, ctx)?;
        input.map_records(move |stream| stage_stream(stream, against, settings, ctx))
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Diff)]
}

/// `hunk` record of one hunk of the difference of a file.
///
/// `number` is the position of the hunk within the file, from 1, and forms
/// the identifier `path#number`. The marker line of a missing final line feed
/// is not part of `lines`, which hold only lines with the prefixes ` `, `-`
/// and `+`.
pub(crate) fn hunk_record(
    hunk: &Hunk,
    number: usize,
    path: &AbsPath,
    old_path: Option<&AbsPath>,
    status: HunkStatus,
    ignore_whitespace: bool,
) -> HunkRecord {
    let lines: Vec<String> = hunk
        .lines
        .iter()
        .map(|line| {
            let mut text = String::with_capacity(line.text.len() + 1);
            text.push(line.kind.prefix());
            text.push_str(&line.text);
            text
        })
        .collect();
    HunkRecord {
        path: path_field(path),
        old_start: to_u64(hunk.old_start),
        old_len: to_u64(hunk.old_len),
        new_start: to_u64(hunk.new_start),
        new_len: to_u64(hunk.new_len),
        id: Some(hunk_id(path, number)),
        header: hunk.header.clone(),
        old_path: old_path.map(path_field),
        status: Some(status),
        ignore_whitespace: ignore_whitespace.then_some(true),
        fingerprint: Some(fingerprint(&lines)),
        lines,
        ..HunkRecord::default()
    }
}

fn hunk_id(path: &AbsPath, number: usize) -> String {
    format!("{}#{number}", path.to_language_string())
}

/// First [`FINGERPRINT_DIGITS`] hexadecimal digits of the SHA-256 hash of the
/// hunk lines, each followed by a line feed.
fn fingerprint(lines: &[String]) -> String {
    let mut hasher = Hasher::new(HashAlgorithm::Sha256);
    for line in lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    let mut hex = hasher.finalize().to_hex();
    hex.truncate(FINGERPRINT_DIGITS);
    hex
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// `context` and `ignore_whitespace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Settings {
    context: usize,
    ignore_whitespace: bool,
}

impl Settings {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        let context = match args.int(CONTEXT)? {
            None => DEFAULT_CONTEXT,
            Some(count) => usize::try_from(count).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{CONTEXT}` must be zero or more, not {count}"),
                )
                .with_found(count.to_string())
            })?,
        };
        Ok(Self {
            context,
            ignore_whitespace: args.bool(IGNORE_WHITESPACE)?.unwrap_or(false),
        })
    }
}

fn required_path(args: &BoundArgs, name: &str, ctx: &ExecCtx) -> Result<AbsPath> {
    path_arg(args, name, ctx)?.ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            format!(
                "`{COMMAND}` needs `{FROM}` and `{TO}`: give the old and the new file or directory"
            ),
        )
    })
}

fn form_error(name: &str, form: &str) -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!("`{name}` belongs to {form} of `{COMMAND}`: remove `{name}`"),
    )
}

/// Origin of one side of a comparison.
#[derive(Debug, Clone)]
enum Source {
    /// The file does not exist on this side.
    Absent,
    /// A file read when the comparison runs.
    File(AbsPath),
    /// Text of a `content` record.
    Text {
        text: String,
        encoding: Option<String>,
    },
}

/// Paths of the records of one comparison.
#[derive(Debug, Clone)]
struct Labels {
    /// File after the change, or the deleted file.
    path: AbsPath,
    /// File before the change, when it differs from `path`.
    old_path: Option<AbsPath>,
}

impl Labels {
    fn new(path: AbsPath, old_path: Option<AbsPath>) -> Self {
        let old_path = old_path.filter(|old| *old != path);
        Self { path, old_path }
    }
}

/// One comparison that produces the records of one file.
#[derive(Debug, Clone)]
enum Job {
    /// Compare the content of two sides.
    Compare {
        old: Source,
        new: Source,
        labels: Labels,
    },
    /// A file moved without a change of content.
    Renamed { from: AbsPath, to: AbsPath },
}

impl Job {
    fn records(self, settings: Settings, ctx: &ExecCtx) -> Result<Vec<Record>> {
        match self {
            Self::Compare { old, new, labels } => {
                let old = load(ctx, old)?;
                let new = load(ctx, new)?;
                Ok(compare(&labels, &old, &new, settings, ctx))
            }
            Self::Renamed { from, to } => {
                let record = HunkRecord {
                    id: Some(hunk_id(&to, 1)),
                    path: path_field(&to),
                    old_path: Some(path_field(&from)),
                    status: Some(HunkStatus::Renamed),
                    ..HunkRecord::default()
                };
                Ok(vec![record.into()])
            }
        }
    }
}

/// Content of one side after reading.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Side {
    Absent,
    Text {
        text: String,
        encoding: Option<String>,
    },
    Binary(ContentHash),
}

impl Side {
    fn text(&self) -> &str {
        match self {
            Self::Text { text, .. } => text,
            Self::Absent | Self::Binary(_) => "",
        }
    }

    fn encoding(&self) -> Option<&str> {
        match self {
            Self::Text { encoding, .. } => encoding.as_deref(),
            Self::Absent | Self::Binary(_) => None,
        }
    }
}

fn load(ctx: &ExecCtx, source: Source) -> Result<Side> {
    match source {
        Source::Absent => Ok(Side::Absent),
        Source::Text { text, encoding } => Ok(Side::Text { text, encoding }),
        Source::File(path) => match read_text(ctx, &path, EncodingParam::Auto) {
            Ok(file) => Ok(Side::Text {
                text: file.text,
                encoding: Some(file.encoding.label),
            }),
            Err(error) if error.code() == ErrorCode::Binary => {
                Ok(Side::Binary(ctx.require_fs()?.content_hash(&path)?))
            }
            Err(error) => Err(error),
        },
    }
}

/// Records of the difference of two sides.
fn compare(
    labels: &Labels,
    old: &Side,
    new: &Side,
    settings: Settings,
    ctx: &ExecCtx,
) -> Vec<Record> {
    let status = match (old, new) {
        (Side::Absent, Side::Absent) => return Vec::new(),
        (Side::Absent, _) => HunkStatus::Added,
        (_, Side::Absent) => HunkStatus::Deleted,
        _ => HunkStatus::Modified,
    };
    match (old, new) {
        (Side::Binary(before), Side::Binary(after)) if before == after => Vec::new(),
        (Side::Binary(_), _) | (_, Side::Binary(_)) => {
            let status = if status == HunkStatus::Modified {
                HunkStatus::Binary
            } else {
                status
            };
            let record = HunkRecord {
                id: Some(hunk_id(&labels.path, 1)),
                path: path_field(&labels.path),
                old_path: labels.old_path.as_ref().map(path_field),
                status: Some(status),
                binary: Some(true),
                ..HunkRecord::default()
            };
            vec![record.into()]
        }
        _ => {
            if let (Some(before), Some(after)) = (old.encoding(), new.encoding())
                && before != after
            {
                ctx.note(NotableEntry {
                    detail: Some(format!("{before}{DETAIL_ARROW}{after}")),
                    ..at_path(Notable::EncodingChanged, &labels.path)
                });
            }
            let hunks = unified_diff(
                old.text(),
                new.text(),
                settings.context,
                settings.ignore_whitespace,
            );
            if hunks.is_empty() && status != HunkStatus::Modified {
                let record = HunkRecord {
                    id: Some(hunk_id(&labels.path, 1)),
                    path: path_field(&labels.path),
                    old_path: labels.old_path.as_ref().map(path_field),
                    status: Some(status),
                    ..HunkRecord::default()
                };
                return vec![record.into()];
            }
            hunks
                .iter()
                .enumerate()
                .map(|(index, hunk)| {
                    hunk_record(
                        hunk,
                        index + 1,
                        &labels.path,
                        labels.old_path.as_ref(),
                        status,
                        settings.ignore_whitespace,
                    )
                    .into()
                })
                .collect()
        }
    }
}

/// Lazy stream of the records of the jobs, in order.
fn job_stream(jobs: Vec<Job>, settings: Settings, ctx: &ExecCtx) -> RecordStream<'_> {
    let records = jobs
        .into_iter()
        .flat_map(move |job| flattened(job.records(settings, ctx)));
    RecordStream::new(records).checked(ctx)
}

fn flattened(result: Result<Vec<Record>>) -> Vec<Result<Record>> {
    match result {
        Ok(records) => records.into_iter().map(Ok).collect(),
        Err(error) => vec![Err(error)],
    }
}

/// Jobs of the source form.
fn source_jobs(ctx: &ExecCtx, from: &AbsPath, to: &AbsPath) -> Result<Vec<Job>> {
    let fs = ctx.require_fs()?;
    let from_kind = fs.metadata(from, Follow::Follow)?.kind;
    let to_kind = fs.metadata(to, Follow::Follow)?.kind;
    let (old, new) = match (from_kind, to_kind) {
        (FileKind::Directory, FileKind::Directory) => return tree_jobs(ctx, fs, from, to),
        (FileKind::File, FileKind::File) => (from.clone(), to.clone()),
        (FileKind::Directory, FileKind::File) => (same_name_in(from, to)?, to.clone()),
        (FileKind::File, FileKind::Directory) => (from.clone(), same_name_in(to, from)?),
        (FileKind::File | FileKind::Directory, _) => return Err(not_comparable_error(to)),
        _ => return Err(not_comparable_error(from)),
    };
    if fs.metadata(&old, Follow::Follow)?.kind != FileKind::File {
        return Err(not_comparable_error(&old));
    }
    if fs.metadata(&new, Follow::Follow)?.kind != FileKind::File {
        return Err(not_comparable_error(&new));
    }
    Ok(vec![Job::Compare {
        old: Source::File(old.clone()),
        new: Source::File(new.clone()),
        labels: Labels::new(new, Some(old)),
    }])
}

/// The file in `dir` with the name of `file`, as GNU `diff` compares a file
/// with a directory.
fn same_name_in(dir: &AbsPath, file: &AbsPath) -> Result<AbsPath> {
    let name = file.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`{file}` has no file name to look up in `{dir}`: give two files"),
        )
    })?;
    Ok(dir.join_name(name)?)
}

fn not_comparable_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` is neither a regular file nor a directory: compare files or directories"),
    )
    .with_problem("not_a_file")
    .with_found(path.to_language_string())
    .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

/// Jobs of the recursive comparison of two directory trees, in byte order of
/// the relative paths.
fn tree_jobs(ctx: &ExecCtx, fs: &dyn Fs, from: &AbsPath, to: &AbsPath) -> Result<Vec<Job>> {
    let old_files = tree_files(ctx, fs, from)?;
    let new_files = tree_files(ctx, fs, to)?;
    let renamed = renames(ctx, fs, from, to, &old_files, &new_files)?;
    let moved_away: BTreeSet<&String> = renamed.values().collect();
    let names: BTreeSet<&String> = old_files.keys().chain(new_files.keys()).collect();
    let mut jobs = Vec::with_capacity(names.len());
    for relative in names {
        let in_old = old_files.contains_key(relative);
        let in_new = new_files.contains_key(relative);
        if let Some(source) = renamed.get(relative) {
            jobs.push(Job::Renamed {
                from: join_relative(from, source)?,
                to: join_relative(to, relative)?,
            });
            continue;
        }
        if !in_new && moved_away.contains(relative) {
            continue;
        }
        let old_path = join_relative(from, relative)?;
        let new_path = join_relative(to, relative)?;
        let old = if in_old {
            Source::File(old_path.clone())
        } else {
            Source::Absent
        };
        let new = if in_new {
            Source::File(new_path.clone())
        } else {
            Source::Absent
        };
        let labels = if in_new {
            Labels::new(new_path, in_old.then_some(old_path))
        } else {
            Labels::new(old_path, None)
        };
        jobs.push(Job::Compare { old, new, labels });
    }
    Ok(jobs)
}

/// Files that exist only in the new tree and have the content of a file that
/// exists only in the old tree: new relative path to old relative path.
/// Empty files are never paired.
fn renames(
    ctx: &ExecCtx,
    fs: &dyn Fs,
    from: &AbsPath,
    to: &AbsPath,
    old_files: &BTreeMap<String, u64>,
    new_files: &BTreeMap<String, u64>,
) -> Result<BTreeMap<String, String>> {
    let only = |files: &BTreeMap<String, u64>, other: &BTreeMap<String, u64>| -> Vec<String> {
        files
            .iter()
            .filter(|(name, size)| **size > 0 && !other.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect()
    };
    let deleted = only(old_files, new_files);
    let added = only(new_files, old_files);
    let mut renamed = BTreeMap::new();
    if deleted.is_empty() || added.is_empty() {
        return Ok(renamed);
    }
    let mut by_hash: BTreeMap<ContentHash, Vec<String>> = BTreeMap::new();
    for name in deleted {
        ctx.checkpoint()?;
        let hash = fs.content_hash(&join_relative(from, &name)?)?;
        by_hash.entry(hash).or_default().push(name);
    }
    for name in added {
        ctx.checkpoint()?;
        let hash = fs.content_hash(&join_relative(to, &name)?)?;
        if let Some(candidates) = by_hash.get_mut(&hash)
            && !candidates.is_empty()
        {
            renamed.insert(name, candidates.remove(0));
        }
    }
    Ok(renamed)
}

/// Regular files below `root`, by relative path with `/` separators, with
/// their sizes. Links to regular files count as files; links to directories
/// are not followed.
fn tree_files(ctx: &ExecCtx, fs: &dyn Fs, root: &AbsPath) -> Result<BTreeMap<String, u64>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![(root.clone(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        ctx.checkpoint()?;
        for item in fs.read_dir(&dir)? {
            let entry = item?;
            let Some(name) = entry.name.to_str() else {
                ctx.note(NotableEntry {
                    detail: Some("an entry name is not valid UTF-8 and was skipped".to_owned()),
                    ..at_path(Notable::Portability, &dir)
                });
                continue;
            };
            let path = dir.join_name(name)?;
            let relative = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}{SEPARATOR}{name}")
            };
            let meta = fs.metadata(&path, Follow::NoFollow)?;
            match meta.kind {
                FileKind::Directory => pending.push((path, relative)),
                FileKind::File => {
                    files.insert(relative, meta.size);
                }
                FileKind::Symlink | FileKind::Junction => {
                    if let Ok(target) = fs.metadata(&path, Follow::Follow)
                        && target.kind == FileKind::File
                    {
                        files.insert(relative, target.size);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(files)
}

/// Joins a relative path with `/` separators to `root`, one name at a time.
pub(crate) fn join_relative(root: &AbsPath, relative: &str) -> Result<AbsPath> {
    relative
        .split(SEPARATOR)
        .try_fold(root.clone(), |path, name| Ok(path.join_name(name)?))
}

/// Old side of the stage form.
#[derive(Debug, Clone)]
enum Against {
    /// A file compared with every input.
    File(AbsPath),
    /// A directory: each input file is compared with the file of the same
    /// name, each input directory with the whole tree.
    Directory(AbsPath),
    /// Records of a named result, matched to the input by `path`; a single
    /// record is compared with every input.
    Records(Vec<Record>),
}

impl Against {
    fn from_args(args: &BoundArgs, ctx: &ExecCtx) -> Result<Self> {
        match args.value(AGAINST) {
            None | Some(Value::Null) => Err(Error::new(
                ErrorCode::MissingParam,
                format!(
                    "`{COMMAND}` needs `{AGAINST}` in the stage form: give the file or named result to compare with"
                ),
            )),
            Some(Value::Path(text) | Value::Text(text)) => {
                let path = resolve(ctx, text)?;
                match ctx.require_fs()?.metadata(&path, Follow::Follow)?.kind {
                    FileKind::Directory => Ok(Self::Directory(path)),
                    FileKind::File => Ok(Self::File(path)),
                    _ => Err(not_comparable_error(&path)),
                }
            }
            Some(Value::Record(record)) => Ok(Self::Records(vec![record.clone()])),
            Some(Value::List(items)) => items
                .iter()
                .map(|item| match item {
                    Value::Record(record) => Ok(record.clone()),
                    _ => Err(against_type_error()),
                })
                .collect::<Result<Vec<_>>>()
                .map(Self::Records),
            Some(_) => Err(against_type_error()),
        }
    }

    /// Old side for the input file or content at `path`, with its path.
    fn old_side(&self, path: &AbsPath, ctx: &ExecCtx) -> Result<(Source, Option<AbsPath>)> {
        match self {
            Self::File(file) => Ok((Source::File(file.clone()), Some(file.clone()))),
            Self::Directory(dir) => {
                let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                    return Ok((Source::Absent, None));
                };
                let file = dir.join_name(name)?;
                match observe(ctx.require_fs()?, &file)? {
                    Observed::Absent => Ok((Source::Absent, None)),
                    Observed::Present(..) => Ok((Source::File(file.clone()), Some(file))),
                }
            }
            Self::Records(records) => {
                let matching = records
                    .iter()
                    .find(|record| record_path(record, ctx).is_ok_and(|found| found == *path));
                let single = (records.len() == 1).then(|| records.first()).flatten();
                let Some(record) = matching.or(single) else {
                    return Ok((Source::Absent, None));
                };
                let found = record_path(record, ctx)?;
                if record.kind == RecordKind::Content {
                    Ok((content_source(record)?, Some(found)))
                } else {
                    Ok((Source::File(found.clone()), Some(found)))
                }
            }
        }
    }
}

fn against_type_error() -> Error {
    Error::new(
        ErrorCode::Type,
        format!(
            "`{COMMAND}` expects a path or a named result of entry or content records for `{AGAINST}`"
        ),
    )
}

fn content_source(record: &Record) -> Result<Source> {
    let Some(Value::Text(text)) = record.get(TEXT_FIELD) else {
        return Err(Error::new(
            ErrorCode::Type,
            format!(
                "a `content` record has no `{TEXT_FIELD}` field: read the file with `read` first"
            ),
        ));
    };
    let encoding = match record.get(ENCODING_FIELD) {
        Some(Value::Text(label) | Value::Enum(label)) => Some(label.clone()),
        _ => None,
    };
    Ok(Source::Text {
        text: text.clone(),
        encoding,
    })
}

/// Lazy stream of the records of the stage form.
fn stage_stream<'a>(
    input: RecordStream<'a>,
    against: Against,
    settings: Settings,
    ctx: &'a ExecCtx,
) -> RecordStream<'a> {
    let records = input.flat_map(move |item| {
        let jobs = item.and_then(|record| record_jobs(&record, &against, ctx));
        flattened(jobs.and_then(|jobs| {
            let mut records = Vec::new();
            for job in jobs {
                records.extend(job.records(settings, ctx)?);
            }
            Ok(records)
        }))
    });
    RecordStream::new(records).checked(ctx)
}

/// Jobs for one input record: none for an entry that is neither a file nor a
/// directory compared with a directory.
fn record_jobs(record: &Record, against: &Against, ctx: &ExecCtx) -> Result<Vec<Job>> {
    let path = record_path(record, ctx)?;
    let new = match record.kind {
        RecordKind::Content => content_source(record)?,
        RecordKind::Entry => {
            let fs = ctx.require_fs()?;
            match fs.metadata(&path, Follow::Follow)?.kind {
                FileKind::File => Source::File(path.clone()),
                FileKind::Directory => {
                    return match against {
                        Against::Directory(dir) => tree_jobs(ctx, fs, dir, &path),
                        Against::File(_) | Against::Records(_) => Ok(Vec::new()),
                    };
                }
                _ => return Ok(Vec::new()),
            }
        }
        other => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`{COMMAND}` compares `entry` and `content` records, not `{}` records",
                    other.as_str()
                ),
            ));
        }
    };
    let (old, old_path) = against.old_side(&path, ctx)?;
    Ok(vec![Job::Compare {
        old,
        new,
        labels: Labels::new(path, old_path),
    }])
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    use super::*;

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn language(text: &str) -> String {
        abs(text).to_language_string()
    }

    fn with_files(files: &[(&str, &[u8])]) -> TestContext {
        let test = TestContext::new();
        for (path, content) in files {
            test.fs().write_file(&abs(path), content).unwrap();
        }
        test
    }

    fn args(params: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        for (name, value) in params {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn path(text: &str) -> Value {
        Value::Path(language(text))
    }

    fn files(from: &str, to: &str, params: &[(&str, Value)]) -> BoundArgs {
        let mut all = vec![(FROM, path(from)), (TO, path(to))];
        all.extend(params.iter().cloned());
        args(&all)
    }

    fn run(ctx: &ExecCtx, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<HunkRecord>> {
        let output = Diff.execute(args, input, ctx)?;
        let StageData::Records(stream) = output.data else {
            panic!("expected records");
        };
        stream
            .map(|item| item.map(|record| HunkRecord::try_from(record).unwrap()))
            .collect()
    }

    fn entries(paths: &[&str]) -> StageInput<'static> {
        let records = paths
            .iter()
            .map(|item| Record::new(RecordKind::Entry).with("path", path(item)))
            .collect();
        StageInput::records(RecordStream::from_records(records))
    }

    fn content(at: &str, text: &str) -> Record {
        Record::new(RecordKind::Content)
            .with("path", path(at))
            .with(TEXT_FIELD, Value::Text(text.to_owned()))
    }

    fn statuses(records: &[HunkRecord]) -> Vec<(String, HunkStatus)> {
        records
            .iter()
            .map(|record| {
                (
                    record.path.as_str().to_owned(),
                    record.status.unwrap_or_default(),
                )
            })
            .collect()
    }

    #[test]
    fn descriptor_registers_source_and_stage_forms() {
        let descriptor = Diff.descriptor();
        assert_eq!(descriptor.command, "diff");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Entry));
        assert!(descriptor.input.accepts(RecordKind::Content));
        assert!(!descriptor.input.accepts(RecordKind::Match));
        assert!(Diff.as_read().is_some());
        assert_eq!(stages().len(), 1);
    }

    #[test]
    fn two_files_give_unified_hunks() {
        let test = with_files(&[
            ("/data/old.txt", b"a\nb\nc\n"),
            ("/data/new.txt", b"a\nB\nc\n"),
        ]);
        let records = run(
            &test.ctx(),
            &files("/data/old.txt", "/data/new.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(records.len(), 1);
        let hunk = &records[0];
        assert_eq!(hunk.lines, [" a", "-b", "+B", " c"]);
        assert_eq!(
            (hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len),
            (1, 3, 1, 3)
        );
        assert_eq!(hunk.path.as_str(), language("/data/new.txt"));
        assert_eq!(
            hunk.old_path.as_ref().map(|old| old.as_str().to_owned()),
            Some(language("/data/old.txt"))
        );
        assert_eq!(hunk.status, Some(HunkStatus::Modified));
        assert_eq!(
            hunk.id.as_deref(),
            Some(format!("{}#1", language("/data/new.txt")).as_str())
        );
        assert_eq!(hunk.fingerprint.as_deref().map(str::len), Some(6));
        assert_eq!(hunk.ignore_whitespace, None);
    }

    #[test]
    fn multibyte_text_is_compared_by_characters() {
        let test = with_files(&[
            (
                "/data/old.txt",
                "g\u{119}\u{15b}\n\u{17c}\u{f3}\u{142}\u{107}\n".as_bytes(),
            ),
            (
                "/data/new.txt",
                "g\u{119}\u{15b}\n\u{17c}\u{f3}\u{142}w\n".as_bytes(),
            ),
        ]);
        let records = run(
            &test.ctx(),
            &files("/data/old.txt", "/data/new.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].lines,
            [
                " g\u{119}\u{15b}",
                "-\u{17c}\u{f3}\u{142}\u{107}",
                "+\u{17c}\u{f3}\u{142}w"
            ]
        );
    }

    #[test]
    fn equal_files_give_no_records() {
        let test = with_files(&[("/data/a.txt", b"same\n"), ("/data/b.txt", b"same\n")]);
        let records = run(
            &test.ctx(),
            &files("/data/a.txt", "/data/b.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn context_controls_the_lines_around_changes() {
        let old = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = b"1\n2\n3\n4\nfive\n6\n7\n8\n9\n";
        let test = with_files(&[("/data/a.txt", old), ("/data/b.txt", new)]);
        let records = run(
            &test.ctx(),
            &files("/data/a.txt", "/data/b.txt", &[(CONTEXT, Value::Int(0))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(records[0].lines, ["-5", "+five"]);
        let wide = run(
            &test.ctx(),
            &files("/data/a.txt", "/data/b.txt", &[(CONTEXT, Value::Int(1))]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(wide[0].lines, [" 4", "-5", "+five", " 6"]);
    }

    #[test]
    fn negative_context_is_rejected() {
        let test = with_files(&[("/data/a.txt", b"a\n"), ("/data/b.txt", b"b\n")]);
        let error = Diff
            .execute(
                &files("/data/a.txt", "/data/b.txt", &[(CONTEXT, Value::Int(-1))]),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn whitespace_changes_are_ignored_on_request() {
        let test = with_files(&[
            ("/data/a.txt", b"x = 1\ny\n"),
            ("/data/b.txt", b"x  =  1\r\ny\n"),
        ]);
        let ignoring = files(
            "/data/a.txt",
            "/data/b.txt",
            &[(IGNORE_WHITESPACE, Value::Bool(true))],
        );
        assert!(
            run(&test.ctx(), &ignoring, StageInput::none())
                .unwrap()
                .is_empty()
        );
        let strict = run(
            &test.ctx(),
            &files("/data/a.txt", "/data/b.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(strict.len(), 1);
    }

    #[test]
    fn missing_final_line_feed_is_not_a_hunk_line() {
        let test = with_files(&[("/data/a.txt", b"a\n"), ("/data/b.txt", b"b")]);
        let records = run(
            &test.ctx(),
            &files("/data/a.txt", "/data/b.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(records[0].lines, ["-a", "+b"]);
        assert_eq!(records[0].first_invalid_line(), None);
    }

    #[test]
    fn directories_are_compared_recursively() {
        let test = with_files(&[
            ("/old/kept.txt", b"same\n"),
            ("/old/sub/changed.txt", b"one\ntwo\n"),
            ("/old/gone.txt", b"bye\n"),
            ("/old/moved.txt", b"moved content\n"),
            ("/new/kept.txt", b"same\n"),
            ("/new/sub/changed.txt", b"one\n2\n"),
            ("/new/added.txt", b"hello\n"),
            ("/new/sub/renamed.txt", b"moved content\n"),
        ]);
        let records = run(&test.ctx(), &files("/old", "/new", &[]), StageInput::none()).unwrap();
        assert_eq!(
            statuses(&records),
            [
                (language("/new/added.txt"), HunkStatus::Added),
                (language("/old/gone.txt"), HunkStatus::Deleted),
                (language("/new/sub/changed.txt"), HunkStatus::Modified),
                (language("/new/sub/renamed.txt"), HunkStatus::Renamed),
            ]
        );
        assert_eq!(records[0].lines, ["+hello"]);
        assert_eq!(records[0].old_path, None);
        assert_eq!(records[1].lines, ["-bye"]);
        assert_eq!(
            records[3]
                .old_path
                .as_ref()
                .map(|old| old.as_str().to_owned()),
            Some(language("/old/moved.txt"))
        );
        assert!(records[3].lines.is_empty());
    }

    #[test]
    fn empty_files_are_not_paired_as_renames() {
        let test = with_files(&[("/old/a.txt", b""), ("/new/b.txt", b"")]);
        test.fs().create_dir_all(&abs("/new")).unwrap();
        let records = run(&test.ctx(), &files("/old", "/new", &[]), StageInput::none()).unwrap();
        assert_eq!(
            statuses(&records),
            [
                (language("/old/a.txt"), HunkStatus::Deleted),
                (language("/new/b.txt"), HunkStatus::Added),
            ]
        );
        assert!(records.iter().all(|record| record.lines.is_empty()));
    }

    #[test]
    fn binary_files_are_compared_by_content_hash() {
        let test = with_files(&[
            ("/data/a.bin", b"\x7fELF\x00\x01\x02\x00"),
            ("/data/b.bin", b"\x7fELF\x00\x01\x03\x00"),
            ("/data/c.bin", b"\x7fELF\x00\x01\x02\x00"),
        ]);
        let changed = run(
            &test.ctx(),
            &files("/data/a.bin", "/data/b.bin", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].status, Some(HunkStatus::Binary));
        assert_eq!(changed[0].binary, Some(true));
        assert!(changed[0].lines.is_empty());
        let same = run(
            &test.ctx(),
            &files("/data/a.bin", "/data/c.bin", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert!(same.is_empty());
    }

    #[test]
    fn different_encodings_are_compared_as_text() {
        let mut wide = vec![0xff, 0xfe];
        wide.extend("caf\u{e9}\n".encode_utf16().flat_map(u16::to_le_bytes));
        let test = with_files(&[
            ("/data/utf8.txt", "caf\u{e9}\n".as_bytes()),
            ("/data/utf16.txt", &wide),
        ]);
        let ctx = test.ctx();
        let records = run(
            &ctx,
            &files("/data/utf8.txt", "/data/utf16.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert!(records.is_empty());
        let notes = ctx.take_notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].code, Notable::EncodingChanged);
        assert_eq!(notes[0].detail.as_deref(), Some("utf-8 -> utf-16le"));
    }

    #[test]
    fn a_file_is_compared_with_the_same_name_in_a_directory() {
        let test = with_files(&[("/old/a.txt", b"1\n"), ("/new/a.txt", b"2\n")]);
        let records = run(
            &test.ctx(),
            &files("/old", "/new/a.txt", &[]),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(records[0].lines, ["-1", "+2"]);
        assert_eq!(
            records[0]
                .old_path
                .as_ref()
                .map(|old| old.as_str().to_owned()),
            Some(language("/old/a.txt"))
        );
    }

    #[test]
    fn missing_files_fail_with_not_found() {
        let test = with_files(&[("/data/a.txt", b"a\n")]);
        let error = Diff
            .execute(
                &files("/data/a.txt", "/data/none.txt", &[]),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn source_form_needs_both_paths_and_no_against() {
        let test = with_files(&[("/data/a.txt", b"a\n")]);
        let only_from = args(&[(FROM, path("/data/a.txt"))]);
        let error = Diff
            .execute(&only_from, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        let with_against = files(
            "/data/a.txt",
            "/data/a.txt",
            &[(AGAINST, path("/data/a.txt"))],
        );
        let error = Diff
            .execute(&with_against, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
    }

    #[test]
    fn stage_form_compares_entries_with_a_file() {
        let test = with_files(&[("/data/base.txt", b"a\nb\n"), ("/data/work.txt", b"a\nc\n")]);
        let given = args(&[(AGAINST, path("/data/base.txt"))]);
        let records = run(&test.ctx(), &given, entries(&["/data/work.txt"])).unwrap();
        assert_eq!(records[0].lines, [" a", "-b", "+c"]);
        assert_eq!(records[0].path.as_str(), language("/data/work.txt"));
        assert_eq!(
            records[0]
                .old_path
                .as_ref()
                .map(|old| old.as_str().to_owned()),
            Some(language("/data/base.txt"))
        );
    }

    #[test]
    fn stage_form_matches_files_by_name_in_a_directory() {
        let test = with_files(&[
            ("/base/a.txt", b"1\n"),
            ("/work/a.txt", b"2\n"),
            ("/work/new.txt", b"n\n"),
        ]);
        let given = args(&[(AGAINST, path("/base"))]);
        let records = run(
            &test.ctx(),
            &given,
            entries(&["/work/a.txt", "/work/new.txt"]),
        )
        .unwrap();
        assert_eq!(
            statuses(&records),
            [
                (language("/work/a.txt"), HunkStatus::Modified),
                (language("/work/new.txt"), HunkStatus::Added),
            ]
        );
        let tree = run(&test.ctx(), &given, entries(&["/work"])).unwrap();
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn stage_form_compares_contents_with_a_named_result() {
        let test = TestContext::new();
        let named = Value::List(vec![
            Value::Record(content("/data/a.txt", "one\ntwo\n")),
            Value::Record(content("/data/b.txt", "x\n")),
        ]);
        let given = args(&[(AGAINST, named)]);
        let input = StageInput::records(RecordStream::from_records(vec![
            content("/data/a.txt", "one\n2\n"),
            content("/data/b.txt", "x\n"),
        ]));
        let records = run(&test.ctx(), &given, input).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].lines, [" one", "-two", "+2"]);
        assert_eq!(records[0].path.as_str(), language("/data/a.txt"));
        assert_eq!(records[0].old_path, None);
    }

    #[test]
    fn stage_form_rejects_source_parameters_and_bad_against() {
        let test = with_files(&[("/data/a.txt", b"a\n")]);
        let with_from = args(&[(AGAINST, path("/data/a.txt")), (FROM, path("/data/a.txt"))]);
        let error = Diff
            .execute(&with_from, entries(&["/data/a.txt"]), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
        let missing = Diff
            .execute(&args(&[]), entries(&["/data/a.txt"]), &test.ctx())
            .unwrap_err();
        assert_eq!(missing.code(), ErrorCode::MissingParam);
        let wrong = Diff
            .execute(
                &args(&[(AGAINST, Value::Int(3))]),
                entries(&["/data/a.txt"]),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(wrong.code(), ErrorCode::Type);
        let absent = Diff
            .execute(
                &args(&[(AGAINST, path("/data/none.txt"))]),
                entries(&["/data/a.txt"]),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(absent.code(), ErrorCode::NotFound);
    }

    #[test]
    fn hunk_records_number_each_hunk_of_a_file() {
        let old: String = (1..=20).map(|line| format!("{line}\n")).collect();
        let new: String = (1..=20)
            .map(|line| match line {
                2 => "two\n".to_owned(),
                19 => "nineteen\n".to_owned(),
                other => format!("{other}\n"),
            })
            .collect();
        let hunks = unified_diff(&old, &new, 1, false);
        let path = abs("/data/a.txt");
        let records: Vec<HunkRecord> = hunks
            .iter()
            .enumerate()
            .map(|(index, hunk)| {
                hunk_record(hunk, index + 1, &path, None, HunkStatus::Modified, true)
            })
            .collect();
        assert_eq!(records.len(), 2);
        assert!(records[1].id.as_deref().unwrap().ends_with("#2"));
        assert_eq!(records[1].ignore_whitespace, Some(true));
        assert_ne!(records[0].fingerprint, records[1].fingerprint);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
