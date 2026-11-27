// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `set` stage and the removal of document keys.
//!
//! `set` changes values at key paths of a structured document and keeps the
//! layout of everything else. The source form names the file (`set "p"
//! key: "a.b" value: 1`, or several keys with `values:` in one transaction);
//! the stage form sets the value of each `item` record read by `read …
//! key:`. [`RemoveItemsStage`] implements `remove` for `item` records
//! (`read "p" key: "a.b" |> remove`) with the same steps.
//!
//! The steps are the same for every target file:
//!
//! 1. the file is read through the file system of the context and its entity
//!    tag compared with `if_match` or with the tag of the item;
//! 2. the document is opened through the format registry and every change is
//!    applied by the document handle, whose guards refuse a change that would
//!    alter bytes outside the changed node or read back with another value;
//! 3. with `validate` other than `off`, the syntax of the new content is
//!    compared with the syntax of the old content and only a new error
//!    counts;
//! 4. the new content is written through the mutation transaction, with the
//!    content hash read in step 1 as the expected state.
//!
//! Nothing is written when any step fails for any key of any file.

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Location, Result, Stage};
use nurt_contracts::fs::{AbsPath, ContentHash, FileId, Follow};
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::record::{Effect, EffectStatus};
use nurt_contracts::report::{Change, ChangeAction, Notable, NotableEntry};
use nurt_contracts::services::{DocFormatId, IssueSeverity, SyntaxIssue};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, Event, ExecCtx, InputKinds,
    LogEvent, LogLevel, RecordStream, StageDescriptor, StageImpl, StageInput, StageOutput,
    StaticInfo,
};
use nurt_contracts::value::{Record, RecordKind, Size, Value};

use super::query::read_file;
use crate::edit::EditReport;
use crate::model::{NodeKind, SetOptions, key_not_found};
use crate::registry::{DocOptions, OpenDocument, Registry, open_with_path};
use crate::value::to_document_value;

/// Name of the command in the catalog.
pub const COMMAND: &str = "set";

/// Name of the removal command implemented for `item` records.
pub const REMOVE_COMMAND: &str = "remove";

/// Flag of a change whose new content has a syntax error the old content did
/// not have, written because `validate` is `warn`.
pub const VALIDATE_WARNING_FLAG: &str = "validate_warning";

const PATH: &str = "path";
const KEY: &str = "key";
const VALUE: &str = "value";
const VALUES: &str = "values";
const CREATE: &str = "create";
const FORMAT: &str = "format";
const VALIDATE: &str = "validate";
const IF_MATCH: &str = "if_match";
const AUTO: &str = "auto";

/// Largest number of existing keys offered for a missing key.
const MAX_KEY_CANDIDATES: usize = 5;

/// Content of a new property list before its first key is set.
const PLIST_SKELETON: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n</dict>\n</plist>\n";

/// Content of a new JSON document before its first key is set.
const JSON_SKELETON: &[u8] = b"{}\n";

/// Treatment of new syntax errors (`validate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Validate {
    Off,
    Warn,
    Reject,
}

/// One change of a document.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Set(KeyPath, Value),
    Remove(KeyPath),
}

impl Op {
    fn key(&self) -> &KeyPath {
        match self {
            Self::Set(key, _) | Self::Remove(key) => key,
        }
    }
}

/// Changes of one file.
#[derive(Debug, Clone)]
struct FileRequest {
    path: String,
    format: Option<String>,
    ops: Vec<Op>,
    expect: Option<ContentHash>,
}

/// Options shared by all files of one call.
#[derive(Debug, Clone, Copy)]
struct Options {
    create: bool,
    validate: Validate,
}

/// New content of one target, computed by the plan and written by apply.
#[derive(Debug)]
struct PlannedWrite {
    abs: AbsPath,
    bytes: Vec<u8>,
    expect: Expect,
    action: TargetAction,
}

/// Implementation of the `set` command.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::StageImpl;
/// use nurt_doc::commands::set::SetStage;
/// use nurt_doc::registry::Registry;
///
/// let stage = SetStage::new(Arc::new(Registry::new()));
/// assert_eq!(stage.descriptor().command, "set");
/// assert!(stage.as_effect().is_some());
/// ```
#[derive(Debug, Clone)]
pub struct SetStage {
    registry: Arc<Registry>,
}

impl SetStage {
    /// Stage that opens documents with the formats of `registry`.
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }
}

impl StageImpl for SetStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Item]),
        }
    }

    /// Checks the key paths, the `values` map and the form of `if_match`
    /// before anything is read.
    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let bind = |error: Error| error.with_stage(Stage::Bind);
        if args.value(KEY).is_some() || args.value(VALUES).is_some() {
            source_ops(args).map_err(bind)?;
        }
        if_match(args).map_err(bind)?;
        let mut info = StaticInfo::default();
        if let Some(path) = args.value(PATH).and_then(Value::as_str) {
            info.paths.push(path.to_owned());
        }
        Ok(info)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for SetStage {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let options = Options {
            create: args.bool(CREATE)?.unwrap_or(true),
            validate: validate_mode(args)?,
        };
        let requests = if input.is_none() {
            vec![source_request(args)?]
        } else {
            let expect = if_match(args)?;
            item_requests(input, ctx, expect, |record| {
                let value = args.eval(VALUE, Some(record), ctx)?.ok_or_else(|| {
                    Error::new(
                        ErrorCode::MissingParam,
                        "`set` after `read … key:` needs the new value, for example set 8080",
                    )
                })?;
                Ok(Some(value))
            })?
        };
        plan_requests(&self.registry, requests, options, ctx)
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        apply_plan(COMMAND, plan, tx)
    }
}

/// Implementation of `remove` for `item` records: removes the keys read by
/// `read … key:` from their documents.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use nurt_contracts::stage::StageImpl;
/// use nurt_contracts::value::RecordKind;
/// use nurt_doc::commands::set::RemoveItemsStage;
/// use nurt_doc::registry::Registry;
///
/// let stage = RemoveItemsStage::new(Arc::new(Registry::new()));
/// let descriptor = stage.descriptor();
/// assert_eq!(descriptor.command, "remove");
/// assert!(descriptor.input.accepts(RecordKind::Item));
/// ```
#[derive(Debug, Clone)]
pub struct RemoveItemsStage {
    registry: Arc<Registry>,
}

impl RemoveItemsStage {
    /// Stage that opens documents with the formats of `registry`.
    #[must_use]
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }
}

impl StageImpl for RemoveItemsStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: REMOVE_COMMAND.to_owned(),
            source: false,
            input: InputKinds::Only(vec![RecordKind::Item]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for RemoveItemsStage {
    fn plan<'a>(
        &'a self,
        _args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let options = Options {
            create: false,
            validate: Validate::Reject,
        };
        let requests = item_requests(input, ctx, None, |_| Ok(None))?;
        plan_requests(&self.registry, requests, options, ctx)
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        apply_plan(REMOVE_COMMAND, plan, tx)
    }
}

fn validate_mode(args: &BoundArgs) -> Result<Validate> {
    match args.value(VALIDATE).and_then(Value::as_str) {
        None | Some("reject") => Ok(Validate::Reject),
        Some("warn") => Ok(Validate::Warn),
        Some("off") => Ok(Validate::Off),
        Some(other) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`validate` is off, warn or reject, not `{other}`"),
        )
        .with_found(other)),
    }
}

fn if_match(args: &BoundArgs) -> Result<Option<ContentHash>> {
    args.text(IF_MATCH)?.map(parse_etag).transpose()
}

fn parse_etag(text: &str) -> Result<ContentHash> {
    ContentHash::parse(text).map_err(|_| {
        Error::new(
            ErrorCode::InvalidValue,
            "`if_match` expects an etag of the form sha256:<64 hexadecimal digits>, as in the etag field of records",
        )
        .with_found(text)
    })
}

fn parse_key(text: &str) -> Result<KeyPath> {
    Ok(KeyPath::parse(text)?)
}

/// Changes of the source form: `key` with `value`, then the members of
/// `values` in their order.
fn source_ops(args: &BoundArgs) -> Result<Vec<Op>> {
    let mut ops = Vec::new();
    if let Some(key) = args.value(KEY).and_then(Value::as_str) {
        let value = args.value(VALUE).cloned().ok_or_else(|| {
            Error::new(
                ErrorCode::MissingParam,
                format!("`set` with key `{key}` needs `value`; add value: <new value>"),
            )
        })?;
        ops.push(Op::Set(parse_key(key)?, value));
    }
    match args.value(VALUES) {
        None => {}
        Some(Value::Map(fields)) => {
            for (key, value) in fields {
                ops.push(Op::Set(parse_key(key)?, value.clone()));
            }
        }
        Some(other) => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`values` is a map of key paths to values, not {}; for example values: {{\"a.b\": 1}}",
                    other.type_name()
                ),
            ));
        }
    }
    if ops.is_empty() {
        return Err(Error::new(
            ErrorCode::MissingParam,
            "`set` needs key and value, or values; for example set \"config.json\" key: \"port\" value: 8080",
        ));
    }
    Ok(ops)
}

fn source_request(args: &BoundArgs) -> Result<FileRequest> {
    let path = args.value(PATH).and_then(Value::as_str).ok_or_else(|| {
        Error::new(
            ErrorCode::MissingParam,
            "`set` without input records needs the document path, for example set \"config.json\" key: \"port\" value: 8080",
        )
    })?;
    let format = args
        .value(FORMAT)
        .and_then(Value::as_str)
        .filter(|format| *format != AUTO)
        .map(str::to_owned);
    Ok(FileRequest {
        path: path.to_owned(),
        format,
        ops: source_ops(args)?,
        expect: if_match(args)?,
    })
}

fn text_field<'r>(record: &'r Record, name: &str) -> Option<&'r str> {
    record.get(name).and_then(Value::as_str)
}

/// Changes of the stage form, grouped by file in the order of first
/// appearance; `value_of` gives the new value of an item, or `None` to
/// remove its key.
fn item_requests<'a>(
    input: StageInput<'a>,
    ctx: &ExecCtx,
    if_match: Option<ContentHash>,
    value_of: impl Fn(&Record) -> Result<Option<Value>>,
) -> Result<Vec<FileRequest>> {
    let mut requests: Vec<FileRequest> = Vec::new();
    let mut index: HashMap<FileKey, usize> = HashMap::new();
    for record in input.into_records()? {
        ctx.checkpoint()?;
        let record = record?;
        if record.kind != RecordKind::Item {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "the input of this stage is item records from read … key:, not {}",
                    record.kind.as_str()
                ),
            ));
        }
        let path = text_field(&record, PATH).unwrap_or_default().to_owned();
        let key = parse_key(text_field(&record, KEY).unwrap_or_default())?;
        let op = match value_of(&record)? {
            Some(value) => Op::Set(key, value),
            None => Op::Remove(key),
        };
        let expect = match if_match {
            Some(expect) => Some(expect),
            None => text_field(&record, "etag").and_then(|etag| ContentHash::parse(etag).ok()),
        };
        let slot = *index.entry(group_key(ctx, &path)).or_insert_with(|| {
            requests.push(FileRequest {
                path,
                format: text_field(&record, FORMAT).map(str::to_owned),
                ops: Vec::new(),
                expect,
            });
            requests.len() - 1
        });
        if let Some(request) = requests.get_mut(slot) {
            request.ops.push(op);
        }
    }
    Ok(requests)
}

/// Identity of the file an item changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FileKey {
    /// Device and file number of an existing file, so that two names of one
    /// file (a symbolic link, a hard link, a path through a link to a
    /// directory) are one file.
    Identity(FileId),
    /// Path after lexical normalisation, for a file that does not exist yet.
    Path(PathBuf),
}

/// Key that groups the items of one file: the identity of the file the path
/// reaches through symbolic links, otherwise the path after lexical
/// normalisation (`.` and `..` components), so that `/d/a.json` and
/// `/d/./a.json` are one file even before it exists. A path that cannot be
/// normalised groups by its text and fails later when planned.
fn group_key(ctx: &ExecCtx, path: &str) -> FileKey {
    let lexical = AbsPath::new(path).ok();
    let identity = lexical
        .as_ref()
        .zip(ctx.fs())
        .and_then(|(abs, fs)| fs.metadata(abs, Follow::Follow).ok())
        .map(|meta| meta.file_id);
    match identity {
        Some(id) => FileKey::Identity(id),
        None => FileKey::Path(
            lexical.map_or_else(|| PathBuf::from(path), |abs| abs.as_path().to_path_buf()),
        ),
    }
}

/// Fails with `E_KEY_CONFLICT` when one key path of a file is equal to
/// another or lies inside it, because the second change would apply to the
/// result of the first.
fn check_disjoint(path: &str, ops: &[Op]) -> Result<()> {
    for (position, op) in ops.iter().enumerate() {
        for other in ops.iter().skip(position + 1) {
            if let Some((outer, inner)) = nested(op.key(), other.key()) {
                return Err(Error::new(
                    ErrorCode::KeyConflict,
                    format!(
                        "keys `{outer}` and `{inner}` of {path} overlap; set the value of `{outer}` once or change them in separate calls"
                    ),
                )
                .with_found("overlap")
                .with_detail("key", serde_json::Value::from(inner.to_string())));
            }
        }
    }
    Ok(())
}

fn nested<'k>(a: &'k KeyPath, b: &'k KeyPath) -> Option<(&'k KeyPath, &'k KeyPath)> {
    if a.document() != b.document() || appends(a) || appends(b) {
        return None;
    }
    let (short, long) = if a.segments().len() <= b.segments().len() {
        (a, b)
    } else {
        (b, a)
    };
    long.segments()
        .starts_with(short.segments())
        .then_some((short, long))
}

fn appends(key: &KeyPath) -> bool {
    key.ends_with_append()
}

fn plan_requests(
    registry: &Registry,
    requests: Vec<FileRequest>,
    options: Options,
    ctx: &ExecCtx,
) -> Result<EffectPlan> {
    let mut plan = EffectPlan::new();
    let mut writes = Vec::new();
    for request in requests {
        ctx.checkpoint()?;
        let path = request.path.clone();
        let planned = plan_file(registry, request, options, ctx)
            .map_err(|error| locate(error.with_stage(Stage::Plan), &path))?;
        match planned {
            Planned::Unchanged => {
                let mut entry = NotableEntry::new(Notable::Unchanged);
                entry.path = Some(path);
                plan.notable.push(entry);
            }
            Planned::Write(file) => {
                let PlannedFile {
                    write,
                    change,
                    notable,
                } = *file;
                plan.changes.push(change);
                plan.notable.extend(notable);
                writes.push(write);
            }
        }
    }
    plan.bulk = writes.len() > 1;
    Ok(plan.with_state(writes))
}

/// Planned change of one file.
struct PlannedFile {
    write: PlannedWrite,
    change: Change,
    notable: Vec<NotableEntry>,
}

enum Planned {
    Unchanged,
    Write(Box<PlannedFile>),
}

fn plan_file(
    registry: &Registry,
    request: FileRequest,
    options: Options,
    ctx: &ExecCtx,
) -> Result<Planned> {
    check_disjoint(&request.path, &request.ops)?;
    let abs = AbsPath::new(request.path.as_str())?;
    let existing = match read_file(ctx, &request.path) {
        Ok(content) => Some(content),
        Err(error) if error.code == ErrorCode::NotFound => None,
        Err(error) => return Err(error),
    };
    let current = existing
        .as_ref()
        .and_then(|content| ContentHash::parse(&content.etag).ok());
    if let Some(expected) = &request.expect
        && current.as_ref() != Some(expected)
    {
        return Err(stale(&request.path, expected));
    }
    let creating = existing.is_none();
    if creating && !(options.create && request.ops.iter().all(|op| matches!(op, Op::Set(..)))) {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!(
                "{} does not exist; check the path or set create: true",
                request.path
            ),
        ));
    }
    let before = existing.map(|content| content.bytes);
    let format = registry.resolve_format(
        request.format.as_deref(),
        Some(FsPath::new(&request.path)),
        before.as_deref().unwrap_or_default(),
    )?;
    let doc_options = DocOptions::from_config(&ctx.config().documents);
    let (initial, mut document) = open_document(
        registry,
        &format,
        FsPath::new(&request.path),
        before.as_deref(),
        &request.ops,
        &doc_options,
    )?;
    let report = apply_ops(&mut document, &request.ops, options)?;
    let after = document.bytes();
    if !creating && after == initial {
        return Ok(Planned::Unchanged);
    }
    let mut flags = report.flags.clone();
    if options.validate != Validate::Off {
        let issues_before = syntax_issues(registry, &format, &initial, &doc_options);
        let issues_after = syntax_issues(registry, &format, &after, &doc_options);
        if let Some(issue) = new_error(&issues_before, &issues_after) {
            if options.validate == Validate::Reject {
                return Err(validate_error(&request.path, issue));
            }
            ctx.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!(
                    "{}: the change introduces a syntax error: {}",
                    request.path, issue.message
                ),
                source: Some(COMMAND.to_owned()),
            }));
            flags.push(VALIDATE_WARNING_FLAG.to_owned());
        }
    }
    let change = Change {
        size_before: (!creating).then(|| byte_count(&initial)),
        size_after: Some(byte_count(&after)),
        hash_before: current.as_ref().map(ToString::to_string),
        replacements: Some(report.replacements),
        flags: (!flags.is_empty()).then_some(flags),
        ..Change::new(
            request.path.clone(),
            if creating {
                ChangeAction::Create
            } else {
                ChangeAction::Modify
            },
        )
    };
    let notable = report
        .notable
        .into_iter()
        .map(|mut entry| {
            entry.path.get_or_insert_with(|| request.path.clone());
            entry
        })
        .collect();
    let write = PlannedWrite {
        abs,
        bytes: after,
        expect: current.map_or(Expect::Absent, Expect::Hash),
        action: if creating {
            TargetAction::Create
        } else {
            TargetAction::Modify
        },
    };
    Ok(Planned::Write(Box::new(PlannedFile {
        write,
        change,
        notable,
    })))
}

fn byte_count(bytes: &[u8]) -> Size {
    Size::from_bytes(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
}

fn stale(path: &str, expected: &ContentHash) -> Error {
    Error::new(
        ErrorCode::Stale,
        format!("{path} changed since it was read; read it again and repeat the change"),
    )
    .with_expected(serde_json::Value::from(expected.to_string()))
}

fn locate(error: Error, path: &str) -> Error {
    if error.location.is_some() {
        return error;
    }
    error.with_location(Location {
        path: path.to_owned(),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        byte: None,
    })
}

/// Opens the document, or a new empty document of the format when the file
/// does not exist; returns the bytes the document was opened from.
fn open_document(
    registry: &Registry,
    format: &DocFormatId,
    path: &FsPath,
    bytes: Option<&[u8]>,
    ops: &[Op],
    options: &DocOptions,
) -> Result<(Vec<u8>, OpenDocument)> {
    let handler = registry.get(format).ok_or_else(|| {
        Error::new(
            ErrorCode::UnknownValue,
            format!("no reader for {format} documents is available"),
        )
    })?;
    if let Some(bytes) = bytes {
        return Ok((
            bytes.to_vec(),
            open_with_path(handler, Some(path), bytes, options)?,
        ));
    }
    let mut last = None;
    for initial in initial_contents(format, ops) {
        match open_with_path(handler, Some(path), &initial, options) {
            Ok(document) => return Ok((initial, document)),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| {
        Error::new(
            ErrorCode::UnknownValue,
            format!("a new {format} document cannot be created; create the file first"),
        )
    }))
}

/// Contents a new document may start from, tried in order: an empty file,
/// then the smallest document of the format that has a root map.
fn initial_contents(format: &DocFormatId, ops: &[Op]) -> Vec<Vec<u8>> {
    let mut contents = vec![Vec::new()];
    if *format == DocFormatId::JSON || *format == DocFormatId::JSONC {
        contents.push(JSON_SKELETON.to_vec());
    } else if *format == DocFormatId::PLIST {
        contents.push(PLIST_SKELETON.to_vec());
    } else if *format == DocFormatId::XML {
        let root = ops
            .first()
            .and_then(|op| match op.key().segments().first() {
                Some(KeySegment::Key(name)) if !name.starts_with(['@', '#']) => Some(name.clone()),
                _ => None,
            });
        if let Some(root) = root {
            contents.push(format!("<{root}/>\n").into_bytes());
        }
    }
    contents
}

fn apply_ops(document: &mut OpenDocument, ops: &[Op], options: Options) -> Result<EditReport> {
    let set_options = SetOptions {
        create: options.create,
    };
    let mut report = EditReport::default();
    for op in ops {
        let step = match op {
            Op::Set(key, value) => {
                document
                    .handle
                    .set(key, &to_document_value(value.clone()), &set_options)
            }
            Op::Remove(key) => document.handle.remove(key),
        };
        match step {
            Ok(edit) => report.absorb(edit),
            Err(error) => return Err(with_closest_keys(error, document, op.key())),
        }
    }
    Ok(report)
}

/// An `E_KEY_NOT_FOUND` error without candidates gets the existing keys of
/// the document closest to `key`, by edit distance of their text.
fn with_closest_keys(error: Error, document: &OpenDocument, key: &KeyPath) -> Error {
    if error.code != ErrorCode::KeyNotFound || !error.candidates.is_empty() {
        return error;
    }
    let wanted = key.to_string();
    let tree = document.handle.tree();
    let semantics = document.handle.semantics();
    let mut keys: Vec<(usize, String, KeyPath)> = tree
        .iter()
        .filter(|(_, node)| node.kind != NodeKind::Comment)
        .map(|(id, _)| tree.path_of(id, semantics))
        .filter(|path| !path.segments().is_empty())
        .map(|path| {
            let text = path.to_string();
            (distance(&wanted, &text), text, path)
        })
        .collect();
    keys.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    keys.dedup_by(|a, b| a.1 == b.1);
    let closest: Vec<KeyPath> = keys
        .into_iter()
        .take(MAX_KEY_CANDIDATES)
        .map(|(_, _, path)| path)
        .collect();
    key_not_found(key, &closest)
}

/// Number of character insertions, deletions and substitutions that turn
/// `from` into `to`.
fn distance(from: &str, to: &str) -> usize {
    let to: Vec<char> = to.chars().collect();
    let mut row: Vec<usize> = (0..=to.len()).collect();
    for (i, left) in from.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, right) in to.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if left == *right {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[j])
            };
            diagonal = above;
        }
    }
    row[to.len()]
}

fn syntax_issues(
    registry: &Registry,
    format: &DocFormatId,
    bytes: &[u8],
    options: &DocOptions,
) -> Vec<SyntaxIssue> {
    registry
        .get(format)
        .map(|handler| handler.check_syntax(bytes, options))
        .unwrap_or_default()
}

/// First error of `after` that `before` does not have. Issues are compared
/// by kind, because positions and messages move with the change; an error
/// is new when its kind occurs more often after the change.
fn new_error<'i>(before: &[SyntaxIssue], after: &'i [SyntaxIssue]) -> Option<&'i SyntaxIssue> {
    let count = |issues: &[SyntaxIssue], kind: &Option<String>| {
        issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Error && issue.kind == *kind)
            .count()
    };
    after
        .iter()
        .filter(|issue| issue.severity == IssueSeverity::Error)
        .find(|issue| count(after, &issue.kind) > count(before, &issue.kind))
}

fn validate_error(path: &str, issue: &SyntaxIssue) -> Error {
    Error::new(
        ErrorCode::DocParse,
        format!(
            "the change would introduce a syntax error at line {}, column {}: {}; nothing was written, correct the value or set validate: warn",
            issue.span.start.line, issue.span.start.column, issue.message
        ),
    )
    .with_problem("validate")
    .with_location(Location {
        path: path.to_owned(),
        line: Some(u64::from(issue.span.start.line)),
        column: Some(u64::from(issue.span.start.column)),
        end_line: None,
        end_column: None,
        byte: Some(issue.span.bytes.start),
    })
}

/// Writes the planned contents through the transaction: every target is
/// declared and preserved before the first write.
fn apply_plan<'a>(
    command: &str,
    mut plan: EffectPlan,
    tx: &mut dyn MutationTx,
) -> Result<EffectOutcome<'a>> {
    let writes: Vec<PlannedWrite> = plan.take_state().ok_or_else(|| {
        Error::new(
            ErrorCode::StageFailed,
            format!("the plan of `{command}` does not hold the new contents"),
        )
    })?;
    let specs: Vec<TargetSpec> = writes
        .iter()
        .map(|write| TargetSpec {
            path: TargetPath::Fs(write.abs.clone()),
            action: write.action.clone(),
            expect: write.expect.clone(),
            size_hint: None,
        })
        .collect();
    let preserved = tx.preserve(&specs)?;
    for (write, preserved) in writes.into_iter().zip(preserved) {
        tx.apply(preserved.index, Mutation::WriteBytes(write.bytes))?;
    }
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: command.to_owned(),
        op: tx.op().map(|op| op.0),
        targets: Some(byte_count_of(plan.changes.len())),
        changes: Some(plan.changes.clone()),
        notable: Some(plan.notable.iter().map(notable_code).collect()),
        ..Effect::default()
    };
    let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(Record::from(
        effect,
    ))));
    outcome.changes = plan.changes;
    outcome.notable = plan.notable;
    Ok(outcome)
}

fn byte_count_of(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

fn notable_code(entry: &NotableEntry) -> String {
    serde_json::to_value(entry.code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::services::{ByteRange, TextPosition, TextSpan};
    use nurt_testkit::ctx::TestContext;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    fn issue(kind: &str, severity: IssueSeverity) -> SyntaxIssue {
        let position = TextPosition { line: 1, column: 1 };
        SyntaxIssue {
            severity,
            message: format!("{kind} problem"),
            span: TextSpan {
                bytes: ByteRange::new(0, 0),
                start: position,
                end: position,
            },
            kind: Some(kind.to_owned()),
        }
    }

    #[test]
    fn paths_of_one_file_share_a_group_key() {
        let test = TestContext::new();
        let ctx = test.ctx();
        assert_eq!(group_key(&ctx, "/d/a.json"), group_key(&ctx, "/d/./a.json"));
        assert_eq!(
            group_key(&ctx, "/d/a.json"),
            group_key(&ctx, "/d/x/../a.json")
        );
        assert_ne!(group_key(&ctx, "/d/a.json"), group_key(&ctx, "/d/b.json"));
        assert_eq!(
            group_key(&ctx, "rel/a.json"),
            FileKey::Path(PathBuf::from("rel/a.json"))
        );
    }

    #[test]
    fn names_of_one_existing_file_share_a_group_key_whatever_the_link() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let fs = test.fs();
        fs.create_dir_all(&AbsPath::for_test("/d")).unwrap();
        fs.write_file(&AbsPath::for_test("/d/a.json"), b"{}")
            .unwrap();
        fs.write_file(&AbsPath::for_test("/d/b.json"), b"{}")
            .unwrap();
        fs.symlink("/d/a.json", &AbsPath::for_test("/d/link.json"))
            .unwrap();
        fs.symlink("/d", &AbsPath::for_test("/dir-link")).unwrap();
        let direct = group_key(&ctx, "/d/a.json");
        assert!(matches!(direct, FileKey::Identity(_)));
        assert_eq!(direct, group_key(&ctx, "/d/link.json"));
        assert_eq!(direct, group_key(&ctx, "/dir-link/a.json"));
        assert_ne!(direct, group_key(&ctx, "/d/b.json"));
    }

    #[test]
    fn nested_keys_conflict() {
        let ops = [
            Op::Set(key("a"), Value::Int(1)),
            Op::Set(key("a.b"), Value::Int(2)),
        ];
        let error = check_disjoint("/data/c.json", &ops).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        assert!(error.message.contains("`a` and `a.b`"), "{}", error.message);
        let same = [Op::Remove(key("x[0]")), Op::Remove(key("x[0]"))];
        assert!(check_disjoint("/data/c.json", &same).is_err());
    }

    #[test]
    fn sibling_and_appended_keys_do_not_conflict() {
        let ops = [
            Op::Set(key("a.b"), Value::Int(1)),
            Op::Set(key("a.c"), Value::Int(2)),
            Op::Set(key("a.list[+]"), Value::Int(3)),
            Op::Set(key("a.list[+]"), Value::Int(4)),
            Op::Set(key("#1:a.b"), Value::Int(5)),
        ];
        assert!(check_disjoint("/data/c.yaml", &ops).is_ok());
    }

    #[test]
    fn only_new_errors_count() {
        let before = [issue("unexpected", IssueSeverity::Error)];
        let same = [
            issue("unexpected", IssueSeverity::Error),
            issue("duplicate_key", IssueSeverity::Warning),
        ];
        assert!(new_error(&before, &same).is_none());
        let more = [
            issue("unexpected", IssueSeverity::Error),
            issue("missing", IssueSeverity::Error),
        ];
        assert_eq!(
            new_error(&before, &more)
                .and_then(|issue| issue.kind.clone())
                .as_deref(),
            Some("missing")
        );
    }

    #[test]
    fn validate_error_points_into_the_file() {
        let error = validate_error("/data/c.json", &issue("missing", IssueSeverity::Error));
        assert_eq!(error.code, ErrorCode::DocParse);
        assert_eq!(error.problem.as_deref(), Some("validate"));
        assert_eq!(
            error
                .location
                .as_ref()
                .map(|location| location.path.as_str()),
            Some("/data/c.json")
        );
    }

    #[test]
    fn new_documents_start_from_the_smallest_root() {
        let xml = initial_contents(
            &DocFormatId::XML,
            &[Op::Set(key("config.port"), Value::Int(1))],
        );
        assert_eq!(xml.last().map(Vec::as_slice), Some(&b"<config/>\n"[..]));
        let json = initial_contents(&DocFormatId::JSON, &[]);
        assert_eq!(json, [Vec::new(), JSON_SKELETON.to_vec()]);
        assert_eq!(
            initial_contents(&DocFormatId::TOML, &[]),
            [Vec::<u8>::new()]
        );
    }

    #[test]
    fn etags_must_be_content_hashes() {
        let error = parse_etag("W/\"abc\"").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidValue);
        let hash = format!("sha256:{}", "0".repeat(64));
        assert!(parse_etag(&hash).is_ok());
    }

    #[test]
    fn edit_distance_counts_character_changes() {
        assert_eq!(distance("prot", "port"), 2);
        assert_eq!(distance("port", "port"), 0);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("name", "names"), 1);
    }

    #[test]
    fn notable_codes_use_report_names() {
        assert_eq!(
            notable_code(&NotableEntry::new(Notable::KeysCreated)),
            "keys_created"
        );
    }
}
