// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-files::commands::write
// Responsibility:   `write`: text, bytes or records written to one file, or a batch of files.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_text, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `write`: text, bytes or records written to one file, or a batch of files
//! written in one transaction.
//!
//! The plan step resolves the targets, reads what the new content depends on
//! (the encoding, byte order mark and line endings of an existing file, the
//! content named by `from`, the input records), encodes the content and
//! checks `if_match`; it changes nothing. Missing parent directories are
//! targets of their own, created before the file. The apply step declares
//! every target before the first change and applies the planned mutations
//! through the transaction, which checks again that no target changed after
//! the plan.
//!
//! Text written to a file of a known format is checked after the write as
//! `validate` requires: a syntax error that the previous content did not
//! have rolls the write back under `reject` and is reported as a warning
//! under `warn`. Bytes, sizes and registry values are not checked.
//!
//! | Mode | Missing file | Existing file |
//! |---|---|---|
//! | `create` | created | `E_EXISTS` |
//! | `overwrite` | created | content replaced in the encoding, byte order mark and line endings of the file |
//! | `append` | created | text added at the end in the encoding and line endings of the file |
//! | `prepend` | created | text added at the start; the file keeps its encoding and line endings |
//!
//! A new file is UTF-8 without a byte order mark, with the line endings of
//! the configuration key `text.new_file_line_ending`. Content identical to
//! the file is not written and is reported as `unchanged`.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{Read, SeekFrom};
use std::sync::Arc;

use nurt_contracts::config;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    AbsPath, Durability, Etag, FileKind, FileMeta, Follow, Fs, FsErrorKind, ProviderValue,
    RegistryKind, WriteHandle,
};
use nurt_contracts::journal::{
    Applied, Expect, FileOp, Mutation, MutationTx, Preserved, TargetAction, TargetPath, TargetSpec,
};
use nurt_contracts::record::Change;
use nurt_contracts::report::{Irreversible, IrreversibleEntry, Notable, NotableEntry};
use nurt_contracts::services::{
    DocFormatId, LineEnding, SerializeOptions, TextCodecService, TextEncoding,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, StageData,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Fields, Record, Size, Value};
use nurt_text::codec::{Codec, decode, encode};
use nurt_text::codec_service::line_ending;
use nurt_text::eol::{self, Terminator};
use serde_json::Value as Json;

use crate::commands::validate::{Validate, check_change, validate_param};
use crate::common::etag::{Observed, check_if_match, observe};
use crate::common::mutation::{applied_change, fs_target, planned_change};
use crate::common::notable::{at_path, preserved_text};
use crate::common::params::{
    EncodingParam, IfMatch, LineEndingParam, encoding_param, if_match_param, line_ending_param,
};
use crate::common::path::{path_arg_for_change, resolve, resolve_for_change};
use crate::common::text::{TextFile, read_text, record_line_ending, text_codec};
use crate::details::content::count_lines;
use crate::registry;

const COMMAND: &str = "write";

const PATH: &str = "path";
const TEXT: &str = "text";
const MODE: &str = "mode";
const FROM: &str = "from";
const BOM: &str = "bom";
const PARENTS: &str = "parents";
const ENSURE_NEWLINE: &str = "ensure_newline";
const FORMAT: &str = "format";
const FILES: &str = "files";
const VALUE_TYPE: &str = "value_type";
const FILE_MODE: &str = "file_mode";
const SIZE: &str = "size";
const DATA: &str = "data";
const OFFSET: &str = "offset";

const CONTENT_PARAMS: [&str; 4] = [TEXT, FROM, SIZE, DATA];
const ITEM_FIELDS: [&str; 3] = [PATH, TEXT, MODE];
const MODE_VALUES: [&str; 4] = ["create", "overwrite", "append", "prepend"];
const FORMAT_VALUES: [&str; 7] = ["auto", "json", "yaml", "toml", "csv", "tsv", "text"];
const VALUE_TYPES: [&str; 6] = [
    "string",
    "dword",
    "qword",
    "multi_string",
    "expand_string",
    "binary",
];

/// Permission bits of directories created for missing parents.
pub(crate) const DIRECTORY_MODE: u32 = 0o755;
const PERMISSION_BITS: u32 = 0o7777;
const OCTAL_RADIX: u32 = 8;
const OCTAL_PREFIX: &str = "0o";
const HEX_RADIX: u32 = 16;
const HEX_PREFIXES: [&str; 2] = ["0x", "0X"];

/// Files smaller than this are never reported as `size_shrunk`.
const SHRINK_MIN_BYTES: u64 = 1024;

/// Field of a record written as its line by `format: text`.
const TEXT_FIELD: &str = "text";
/// Column of scalar values written as a table.
const VALUE_COLUMN: &str = "value";
const CSV_DELIMITER: char = ',';
const TSV_DELIMITER: char = '\t';

/// Platform type code of `REG_BINARY`.
const REG_BINARY: u32 = 3;

/// Stage of `write`.
pub(crate) struct Write;

/// Stages of this module, for the registry of the engine.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Write)]
}

impl StageImpl for Write {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Any,
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Write {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let settings = Settings::read(args, ctx)?;
        if let Some(raw) = registry_path(args) {
            return plan_registry(args, raw, Planner::new(ctx, settings));
        }
        if present(args, VALUE_TYPE) {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "`value_type` applies only to `registry:` paths: remove it for files",
            ));
        }
        let if_match = if_match_param(args, ctx)?;
        let items = items(args, input, ctx)?;
        let mut planner = Planner::new(ctx, settings);
        for item in items {
            ctx.checkpoint()?;
            planner.item(item, if_match.as_ref())?;
        }
        Ok(planner.finish())
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let planned = plan.take_state::<Planned>().ok_or_else(|| {
            Error::new(
                ErrorCode::StageFailed,
                format!("the plan of `{COMMAND}` holds no planned changes"),
            )
        })?;
        let changes = apply_planned(tx, planned.ops, ctx)?;
        for check in &planned.checks {
            check_change(
                ctx,
                COMMAND,
                &check.path,
                check.before.as_bytes(),
                check.after.as_bytes(),
                planned.validate,
            )?;
        }
        Ok(EffectOutcome {
            changes,
            ..EffectOutcome::new(StageOutput::none())
        })
    }
}

/// One change planned by an effect stage of this crate: the declared
/// target, the work that performs the change and the `change` record shown
/// by a preview.
#[derive(Debug)]
pub(crate) struct PlannedOp {
    /// Declaration of the target.
    pub(crate) target: TargetSpec,
    /// Work of the apply step.
    pub(crate) work: Work,
    /// Planned `change` record; its encoding, line ending, byte order mark,
    /// link target and irreversibility fields are kept in the applied record.
    pub(crate) change: Change,
}

impl PlannedOp {
    /// Planned operation of a file operation.
    pub(crate) fn from_op(op: FileOp) -> Self {
        let change = planned_change(&op.target);
        Self {
            target: op.target,
            work: Work::Apply(op.mutation),
            change,
        }
    }
}

/// How the apply step changes a target.
#[derive(Debug)]
pub(crate) enum Work {
    /// One mutation applied to the declared target.
    Apply(Mutation),
    /// Content written to a staged file that is switched in afterwards.
    Staged(Staged),
}

/// Content of a staged file.
#[derive(Debug)]
pub(crate) struct Staged {
    /// Bytes written at `offset`.
    pub(crate) bytes: Vec<u8>,
    /// Offset of the bytes; the range before it is zero.
    pub(crate) offset: u64,
    /// Length set after writing, when any.
    pub(crate) length: Option<u64>,
    /// Permission bits of the new file, when given.
    pub(crate) mode: Option<u32>,
}

/// Takes the planned operations out of a plan.
///
/// # Errors
///
/// Fails with `E_STAGE_FAILED` when the plan was not made by the stage.
pub(crate) fn take_ops(plan: &mut EffectPlan, command: &str) -> Result<Vec<PlannedOp>> {
    plan.take_state::<Vec<PlannedOp>>().ok_or_else(|| {
        Error::new(
            ErrorCode::StageFailed,
            format!("the plan of `{command}` holds no planned changes"),
        )
    })
}

/// Declares every target in one call and applies the planned work in order;
/// returns the `change` record of every target.
///
/// # Errors
///
/// Fails with the errors of the transaction, for example `E_STALE` when a
/// target changed after the plan, and with `E_CANCELLED` or `E_TIMEOUT`
/// between targets; the executor then rolls the transaction back.
pub(crate) fn apply_planned(
    tx: &mut dyn MutationTx,
    ops: Vec<PlannedOp>,
    ctx: &ExecCtx,
) -> Result<Vec<Change>> {
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    let targets: Vec<TargetSpec> = ops.iter().map(|op| op.target.clone()).collect();
    let preserved = tx.preserve(&targets)?;
    ops.into_iter()
        .zip(preserved)
        .map(|(op, kept)| apply_one(tx, op, &kept, ctx))
        .collect()
}

fn apply_one(
    tx: &mut dyn MutationTx,
    op: PlannedOp,
    kept: &Preserved,
    ctx: &ExecCtx,
) -> Result<Change> {
    ctx.checkpoint()?;
    let applied = match op.work {
        Work::Apply(mutation) => tx.apply(kept.index, mutation)?,
        Work::Staged(staged) => apply_staged(tx, kept, &staged, ctx)?,
    };
    Ok(merge_change(
        applied_change(&op.target, kept, &applied),
        op.change,
    ))
}

fn merge_change(applied: Change, planned: Change) -> Change {
    Change {
        encoding_before: planned.encoding_before,
        encoding_after: planned.encoding_after,
        line_ending_before: planned.line_ending_before,
        line_ending_after: planned.line_ending_after,
        bom_before: planned.bom_before,
        bom_after: planned.bom_after,
        link_target: planned.link_target,
        irreversible: planned.irreversible.or(applied.irreversible),
        added: planned.added,
        removed: planned.removed,
        ..applied
    }
}

fn apply_staged(
    tx: &mut dyn MutationTx,
    kept: &Preserved,
    staged: &Staged,
    ctx: &ExecCtx,
) -> Result<Applied> {
    let mut handle = tx.stage(kept.index)?;
    fill_staged(handle.as_mut(), staged, ctx)?;
    drop(handle);
    tx.apply(kept.index, Mutation::CommitStaged)
}

fn fill_staged(handle: &mut dyn WriteHandle, staged: &Staged, ctx: &ExecCtx) -> Result<()> {
    let path = handle.path().clone();
    let failed = |error: std::io::Error| {
        Error::new(ErrorCode::Io, format!("writing `{path}` failed: {error}"))
    };
    if staged.offset > 0 {
        handle
            .seek(SeekFrom::Start(staged.offset))
            .map_err(&failed)?;
    }
    handle.write_all(&staged.bytes).map_err(&failed)?;
    if let Some(length) = staged.length {
        handle.set_length(length)?;
    }
    if let Some(mode) = staged.mode {
        let mut meta = ctx
            .require_fs()?
            .extended_metadata(&path, Follow::NoFollow)?;
        meta.basic.mode = mode;
        handle.apply_meta(&meta)?;
    }
    handle.sync(Durability::Full)?;
    Ok(())
}

/// Plans the creation of the missing parent directories of `path`, the
/// topmost first; directories already in `planned` count as present and
/// the new ones are added to it.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` when a parent is missing and `parents` is false,
/// with `E_INVALID_VALUE` when a parent is not a directory, and with the
/// errors of the file system.
pub(crate) fn plan_parents(
    fs: &dyn Fs,
    path: &AbsPath,
    parents: bool,
    planned: &mut BTreeSet<AbsPath>,
) -> Result<Vec<PlannedOp>> {
    let missing = missing_parents(fs, path, planned)?;
    if let Some(top) = missing.last()
        && !parents
    {
        return Err(Error::new(
            ErrorCode::NotFound,
            format!(
                "parent directory `{top}` of `{path}` does not exist: create it or set `parents: true`"
            ),
        )
        .with_detail("path", Json::from(top.to_language_string())));
    }
    Ok(missing
        .into_iter()
        .rev()
        .map(|dir| {
            planned.insert(dir.clone());
            PlannedOp::from_op(FileOp::create_dir(dir, DIRECTORY_MODE))
        })
        .collect())
}

fn missing_parents(
    fs: &dyn Fs,
    path: &AbsPath,
    planned: &BTreeSet<AbsPath>,
) -> Result<Vec<AbsPath>> {
    let mut missing = Vec::new();
    let mut current = path.parent();
    while let Some(dir) = current {
        if planned.contains(&dir) {
            break;
        }
        match fs.metadata(&dir, Follow::Follow) {
            Ok(meta) if meta.kind == FileKind::Directory => break,
            Ok(_) => {
                return Err(Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "`{dir}` is not a directory: the parents of a target must be directories"
                    ),
                ));
            }
            Err(error) if error.kind == FsErrorKind::NotFound => {
                current = dir.parent();
                missing.push(dir);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(missing)
}

/// Value of an enumeration parameter; `None` without a value.
///
/// # Errors
///
/// Fails with `E_UNKNOWN_VALUE` for a value outside `values` and with
/// `E_TYPE` for a value that is not an enumeration value.
pub(crate) fn choice<'a>(
    args: &'a BoundArgs,
    name: &str,
    values: &[&str],
) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) if values.contains(&text.as_str()) => {
            Ok(Some(text.as_str()))
        }
        Some(Value::Enum(text) | Value::Text(text)) => Err(Error::new(
            ErrorCode::UnknownValue,
            format!(
                "`{text}` is not a value of `{name}`: use one of {}",
                values.join(", ")
            ),
        )
        .with_found(text.as_str())
        .with_expected(serde_json::json!(values))),
        Some(_) => Err(type_error(
            args.command(),
            name,
            "a value of the enumeration",
        )),
    }
}

/// Returns `true` when the parameter has a value.
pub(crate) fn present(args: &BoundArgs, name: &str) -> bool {
    !matches!(
        args.get(name),
        None | Some(ArgValue::Absent | ArgValue::Value(Value::Null))
    )
}

/// `E_PARAM_CONFLICT` for two parameters that exclude each other.
pub(crate) fn conflict(command: &str, first: &str, second: &str) -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!("`{command}` takes `{first}` or `{second}`, not both"),
    )
}

/// `E_TYPE` for a parameter of another type.
pub(crate) fn type_error(command: &str, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{command}` expects {expected} for `{name}`"),
    )
}

/// Write mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Create,
    Overwrite,
    Append,
    Prepend,
}

impl Mode {
    fn parse(text: &str) -> Result<Self> {
        match text {
            "create" => Ok(Self::Create),
            "overwrite" => Ok(Self::Overwrite),
            "append" => Ok(Self::Append),
            "prepend" => Ok(Self::Prepend),
            other => Err(Error::new(
                ErrorCode::UnknownValue,
                format!(
                    "`{other}` is not a value of `mode`: use one of {}",
                    MODE_VALUES.join(", ")
                ),
            )
            .with_found(other)),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Overwrite => "overwrite",
            Self::Append => "append",
            Self::Prepend => "prepend",
        }
    }

    fn adds(self) -> bool {
        matches!(self, Self::Append | Self::Prepend)
    }
}

fn mode_param(args: &BoundArgs) -> Result<Option<Mode>> {
    choice(args, MODE, &MODE_VALUES)?
        .map(Mode::parse)
        .transpose()
}

fn required_mode(args: &BoundArgs, path: Option<&AbsPath>) -> Result<Mode> {
    mode_param(args)?.ok_or_else(|| missing_mode(path))
}

fn missing_mode(path: Option<&AbsPath>) -> Error {
    let error = Error::new(
        ErrorCode::MissingParam,
        "`write` needs `mode`: create, overwrite, append or prepend; there is no default",
    );
    let Some(path) = path else {
        return error;
    };
    let path = path.to_language_string();
    ["create", "overwrite", "append"]
        .into_iter()
        .fold(error, |error, mode| {
            error.with_fix(format!("write \"{path}\" mode: {mode}"))
        })
}

/// Serialization of written records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Auto,
    Json,
    Yaml,
    Toml,
    Csv,
    Tsv,
    Text,
}

impl Format {
    fn parse(args: &BoundArgs) -> Result<Self> {
        Ok(match choice(args, FORMAT, &FORMAT_VALUES)? {
            Some("json") => Self::Json,
            Some("yaml") => Self::Yaml,
            Some("toml") => Self::Toml,
            Some("csv") => Self::Csv,
            Some("tsv") => Self::Tsv,
            Some("text") => Self::Text,
            _ => Self::Auto,
        })
    }

    /// The format itself, or for `auto` the format of the extension of
    /// `path`, otherwise text.
    fn for_path(self, path: &AbsPath) -> Self {
        if self != Self::Auto {
            return self;
        }
        let extension = path
            .as_path()
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("json") => Self::Json,
            Some("yaml" | "yml") => Self::Yaml,
            Some("toml") => Self::Toml,
            Some("csv") => Self::Csv,
            Some("tsv") => Self::Tsv,
            _ => Self::Text,
        }
    }
}

/// Parameters that shape the written content.
struct Settings {
    encoding: EncodingParam,
    line_ending: LineEndingParam,
    bom: Option<bool>,
    parents: bool,
    ensure_newline: bool,
    file_mode: Option<u32>,
    new_line: Terminator,
    shrink_ratio: f64,
    validate: Validate,
}

impl Settings {
    fn read(args: &BoundArgs, ctx: &ExecCtx) -> Result<Self> {
        let new_line = match ctx.config().text.new_file_line_ending {
            config::LineEnding::Lf => Terminator::Lf,
            config::LineEnding::Crlf => Terminator::CrLf,
        };
        Ok(Self {
            encoding: encoding_param(args)?.unwrap_or(EncodingParam::Auto),
            line_ending: line_ending_param(args)?,
            bom: args.bool(BOM)?,
            parents: args.bool(PARENTS)?.unwrap_or(true),
            ensure_newline: args.bool(ENSURE_NEWLINE)?.unwrap_or(false),
            file_mode: args.text(FILE_MODE)?.map(parse_file_mode).transpose()?,
            new_line,
            shrink_ratio: ctx.config().report.notable.shrink_ratio,
            validate: validate_param(args)?,
        })
    }

    fn terminator(&self) -> Option<Terminator> {
        match self.line_ending {
            LineEndingParam::Convert(terminator) => Some(terminator),
            LineEndingParam::Keep => None,
        }
    }

    /// Form of a new file.
    fn new_form(&self) -> Form<'static> {
        let codec = match self.encoding {
            EncodingParam::Codec(codec) => codec,
            EncodingParam::Auto => Codec::Utf8,
        };
        Form {
            encoding: codec.text_encoding(self.bom.unwrap_or(false)),
            lines: Lines::Convert(self.terminator().unwrap_or(self.new_line)),
            fallback: self.new_line,
        }
    }

    /// Form of an existing text file, with the explicit parameters applied.
    fn kept_form<'t>(&self, file: &'t TextFile, appending: bool) -> Form<'t> {
        let encoding = match self.encoding {
            EncodingParam::Codec(codec) if codec.label() != file.encoding.label => {
                codec.text_encoding(self.bom.unwrap_or(file.encoding.bom) && codec.bom().is_some())
            }
            _ => TextEncoding {
                label: file.encoding.label.clone(),
                bom: self.bom.unwrap_or(file.encoding.bom),
            },
        };
        let lines = match (self.terminator(), appending) {
            (Some(terminator), _) => Lines::Convert(terminator),
            (None, true) => {
                Lines::Convert(terminator_of(file.line_ending).unwrap_or(self.new_line))
            }
            (None, false) => Lines::Preserve(&file.text),
        };
        Form {
            encoding,
            lines,
            fallback: self.new_line,
        }
    }
}

fn parse_file_mode(text: &str) -> Result<u32> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(OCTAL_PREFIX).unwrap_or(trimmed);
    u32::from_str_radix(digits, OCTAL_RADIX)
        .ok()
        .filter(|mode| !digits.is_empty() && *mode <= PERMISSION_BITS)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{text}` is not a permission mode: write octal digits such as \"0644\""),
            )
            .with_found(text)
        })
}

/// Encoding and line endings of written text.
struct Form<'t> {
    encoding: TextEncoding,
    lines: Lines<'t>,
    fallback: Terminator,
}

/// Line endings of written text.
#[derive(Clone, Copy)]
enum Lines<'t> {
    /// Every line ending becomes the terminator.
    Convert(Terminator),
    /// Line endings follow the original text ([`eol::preserve`]).
    Preserve(&'t str),
}

impl Form<'_> {
    fn terminator(&self) -> Terminator {
        match self.lines {
            Lines::Convert(terminator) => terminator,
            Lines::Preserve(original) => {
                terminator_of(eol::detect(original.as_bytes())).unwrap_or(self.fallback)
            }
        }
    }

    fn render(&self, text: &str, ensure_newline: bool) -> String {
        let mut out = match self.lines {
            Lines::Convert(terminator) => eol::convert(text, terminator).into_owned(),
            Lines::Preserve(original) => eol::preserve(original, text).into_owned(),
        };
        if ensure_newline && !out.is_empty() && !out.ends_with(['\n', '\r']) {
            out.push_str(self.terminator().as_str());
        }
        out
    }
}

fn terminator_of(ending: LineEnding) -> Option<Terminator> {
    match ending {
        LineEnding::Lf => Some(Terminator::Lf),
        LineEnding::CrLf => Some(Terminator::CrLf),
        LineEnding::Cr => Some(Terminator::Cr),
        _ => None,
    }
}

fn combined(before: LineEnding, added: LineEnding) -> LineEnding {
    match (before, added) {
        (LineEnding::None, added) => added,
        (before, LineEnding::None) => before,
        (before, added) if before == added => before,
        _ => LineEnding::Mixed,
    }
}

fn encode_text(text: &str, encoding: &TextEncoding, path: &AbsPath) -> Result<Vec<u8>> {
    encode(text, encoding).map_err(|error| error.into_error(Some(&path.to_language_string())))
}

fn byte_len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// Content of one target.
#[derive(Debug, PartialEq, Eq)]
enum Content {
    Text(String),
    Bytes(Vec<u8>),
    Data { bytes: Vec<u8>, offset: u64 },
    Size(u64),
}

/// One file to write.
#[derive(Debug)]
struct Item {
    path: AbsPath,
    mode: Mode,
    content: Content,
}

fn items(args: &BoundArgs, input: StageInput<'_>, ctx: &ExecCtx) -> Result<Vec<Item>> {
    let path = path_arg_for_change(args, PATH, ctx)?;
    let files = args
        .value(FILES)
        .filter(|value| !matches!(value, Value::Null));
    match (path, files) {
        (Some(_), Some(_)) => Err(conflict(COMMAND, PATH, FILES)),
        (Some(path), None) => {
            let mode = required_mode(args, Some(&path))?;
            let content = content(args, input, &path, ctx)?;
            Ok(vec![Item {
                path,
                mode,
                content,
            }])
        }
        (None, Some(files)) => batch(args, files, &input, ctx),
        (None, None) => Err(Error::new(
            ErrorCode::MissingParam,
            "`write` needs `path` or `files`: give the file to write",
        )),
    }
}

fn input_conflict(name: &str) -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!("`write` received input records and `{name}`: write either the input or `{name}`"),
    )
}

fn content(
    args: &BoundArgs,
    input: StageInput<'_>,
    path: &AbsPath,
    ctx: &ExecCtx,
) -> Result<Content> {
    let sources: Vec<&str> = CONTENT_PARAMS
        .into_iter()
        .filter(|name| present(args, name))
        .collect();
    if let [first, second, ..] = sources.as_slice() {
        return Err(conflict(COMMAND, first, second));
    }
    if !input.is_none()
        && let Some(first) = sources.first()
    {
        return Err(input_conflict(first));
    }
    if present(args, OFFSET) && sources.first() != Some(&DATA) {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            "`offset` needs `data`: give the bytes written at the offset",
        ));
    }
    let format = Format::parse(args)?.for_path(path);
    match sources.first().copied() {
        Some(TEXT) => text_value(args, ctx).map(Content::Text),
        Some(FROM) => from_content(args, format, ctx),
        Some(SIZE) => size_content(args),
        Some(DATA) => data_content(args),
        _ => data_of(input.data, format, ctx),
    }
}

fn text_value(args: &BoundArgs, ctx: &ExecCtx) -> Result<String> {
    match args.eval(TEXT, None, ctx)? {
        Some(Value::Text(text)) => Ok(text),
        None | Some(Value::Null) => Ok(String::new()),
        Some(_) => Err(type_error(COMMAND, TEXT, "a text")),
    }
}

fn from_content(args: &BoundArgs, format: Format, ctx: &ExecCtx) -> Result<Content> {
    match args.get(FROM) {
        Some(ArgValue::Pipeline(pipeline)) => {
            let output = ctx.pipelines()?.run(pipeline, StageInput::none(), ctx)?;
            data_of(output.data, format, ctx)
        }
        Some(ArgValue::Value(Value::Path(text) | Value::Text(text))) => {
            file_content(&resolve(ctx, text)?, ctx)
        }
        Some(ArgValue::Value(value)) => value_content(value.clone(), format, ctx),
        _ => Err(type_error(
            COMMAND,
            FROM,
            "a pipeline, a named result or a path",
        )),
    }
}

fn size_content(args: &BoundArgs) -> Result<Content> {
    match args.value(SIZE) {
        Some(Value::Size(size)) => Ok(Content::Size(size.bytes())),
        Some(Value::Int(number)) => u64::try_from(*number)
            .map(Content::Size)
            .map_err(|_| negative(SIZE, *number)),
        _ => Err(type_error(COMMAND, SIZE, "a size")),
    }
}

fn data_content(args: &BoundArgs) -> Result<Content> {
    let bytes = match args.value(DATA) {
        Some(Value::Bytes(bytes)) => bytes.as_slice().to_vec(),
        _ => return Err(type_error(COMMAND, DATA, "bytes")),
    };
    match args.int(OFFSET)? {
        None => Ok(Content::Bytes(bytes)),
        Some(offset) => u64::try_from(offset)
            .map(|offset| Content::Data { bytes, offset })
            .map_err(|_| negative(OFFSET, offset)),
    }
}

fn negative(name: &str, value: i64) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{name}` cannot be negative: give 0 or more"),
    )
    .with_found(value.to_string())
}

fn data_of(data: StageData<'_>, format: Format, ctx: &ExecCtx) -> Result<Content> {
    match data {
        StageData::None => Ok(Content::Text(String::new())),
        StageData::Records(stream) => {
            let records = stream.collect_bounded(ctx.limits().max_records)?;
            serialize(
                Payload::Many(records.into_iter().map(Value::Record).collect()),
                format,
                ctx,
            )
        }
        StageData::Value(value) => value_content(value, format, ctx),
        _ => Err(Error::new(
            ErrorCode::Type,
            "`write` cannot write this input: pass records or a value",
        )),
    }
}

fn value_content(value: Value, format: Format, ctx: &ExecCtx) -> Result<Content> {
    match value {
        Value::Text(text) => Ok(Content::Text(text)),
        Value::Bytes(bytes) => Ok(Content::Bytes(bytes.into_vec())),
        Value::List(items) => serialize(Payload::Many(items), format, ctx),
        other => serialize(Payload::One(other), format, ctx),
    }
}

fn file_content(path: &AbsPath, ctx: &ExecCtx) -> Result<Content> {
    let bytes = read_bytes(ctx, path)?;
    match text_codec(ctx).detect(&bytes).encoding {
        Some(encoding) => decode(&bytes, &encoding)
            .map(Content::Text)
            .map_err(|error| error.into_error(Some(&path.to_language_string()))),
        None => Ok(Content::Bytes(bytes)),
    }
}

/// Reads a whole file within the text size limit.
fn read_bytes(ctx: &ExecCtx, path: &AbsPath) -> Result<Vec<u8>> {
    let limit = ctx.limits().max_file_bytes_text;
    let mut handle = ctx.require_fs()?.open_read(path)?;
    let size = handle.meta()?.size;
    let mut bytes = Vec::new();
    if size <= limit {
        Read::take(&mut handle, limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
            })?;
    }
    let read = size.max(byte_len(&bytes));
    if read > limit {
        return Err(Error::new(
            ErrorCode::Limit,
            format!("`{path}` has {read} bytes, more than the {limit} bytes `write` reads"),
        )
        .with_expected(serde_json::json!(limit))
        .with_actual(serde_json::json!(read)));
    }
    Ok(bytes)
}

/// Values to serialize: one value, or a list.
enum Payload {
    One(Value),
    Many(Vec<Value>),
}

impl Payload {
    fn items(&self) -> &[Value] {
        match self {
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(items) => items,
        }
    }

    fn into_items(self) -> Vec<Value> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(items) => items,
        }
    }
}

fn serialize(payload: Payload, format: Format, ctx: &ExecCtx) -> Result<Content> {
    match format {
        Format::Json => json_text(&payload).map(Content::Text),
        Format::Csv => Ok(Content::Text(table(payload.items(), CSV_DELIMITER))),
        Format::Tsv => Ok(Content::Text(table(payload.items(), TSV_DELIMITER))),
        Format::Yaml => documents(payload.into_items(), &DocFormatId::YAML, ctx),
        Format::Toml => documents(payload.into_items(), &DocFormatId::TOML, ctx),
        Format::Auto | Format::Text => Ok(Content::Text(lines_text(payload.items()))),
    }
}

fn json_text(payload: &Payload) -> Result<String> {
    let json = match payload {
        Payload::One(value) => value.to_plain_json(),
        Payload::Many(items) => Json::Array(items.iter().map(Value::to_plain_json).collect()),
    };
    let mut text = serde_json::to_string_pretty(&json).map_err(|error| {
        Error::new(
            ErrorCode::Format,
            format!("the records cannot be written as JSON: {error}"),
        )
    })?;
    text.push('\n');
    Ok(text)
}

fn fields_of(value: &Value) -> Option<&Fields> {
    match value {
        Value::Record(record) => Some(&record.fields),
        Value::Map(fields) => Some(fields),
        _ => None,
    }
}

fn plain_text(value: &Value) -> String {
    match value.to_plain_json() {
        Json::String(text) => text,
        Json::Null => String::new(),
        other => other.to_string(),
    }
}

fn fields_json(fields: &Fields) -> String {
    Json::Object(
        fields
            .iter()
            .map(|(name, value)| (name.clone(), value.to_plain_json()))
            .collect(),
    )
    .to_string()
}

/// Text of one value as a line: the `text` field of a record, the plain
/// form of a scalar, otherwise compact JSON.
fn line_of(value: &Value) -> String {
    match fields_of(value) {
        Some(fields) => match fields.get(TEXT_FIELD) {
            Some(Value::Text(text)) => text.clone(),
            _ => fields_json(fields),
        },
        None => plain_text(value),
    }
}

fn lines_text(items: &[Value]) -> String {
    items
        .iter()
        .map(|item| {
            let mut line = line_of(item);
            line.push('\n');
            line
        })
        .collect()
}

/// Rows of records under a header of every field name, in order of first
/// appearance; scalar values form the single column `value`.
fn table(items: &[Value], delimiter: char) -> String {
    let mut columns: Vec<String> = Vec::new();
    for fields in items.iter().filter_map(fields_of) {
        for name in fields.keys() {
            if !columns.contains(name) {
                columns.push(name.clone());
            }
        }
    }
    if columns.is_empty() {
        columns.push(VALUE_COLUMN.to_owned());
    }
    let mut out = row(columns.iter().map(String::as_str), delimiter);
    for item in items {
        let cells: Vec<String> = match fields_of(item) {
            Some(fields) => columns
                .iter()
                .map(|name| fields.get(name).map(plain_text).unwrap_or_default())
                .collect(),
            None => vec![plain_text(item)],
        };
        out.push_str(&row(cells.iter().map(String::as_str), delimiter));
    }
    out
}

fn row<'c>(cells: impl Iterator<Item = &'c str>, delimiter: char) -> String {
    let mut line = cells
        .map(|cell| cell_text(cell, delimiter))
        .collect::<Vec<_>>()
        .join(&delimiter.to_string());
    line.push('\n');
    line
}

/// A cell quoted as RFC 4180 requires for CSV; for TSV, tab, line breaks
/// and backslash escaped with a backslash.
fn cell_text(cell: &str, delimiter: char) -> String {
    if delimiter == TSV_DELIMITER {
        return cell
            .replace('\\', "\\\\")
            .replace('\t', "\\t")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
    }
    if cell.contains([delimiter, '"', '\n', '\r']) {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_owned()
    }
}

fn documents(items: Vec<Value>, format: &DocFormatId, ctx: &ExecCtx) -> Result<Content> {
    let records = items
        .into_iter()
        .map(|item| match item {
            Value::Record(record) => Ok(record),
            _ => Err(Error::new(
                ErrorCode::Type,
                format!("`format: {format}` writes records, not single values: use `format: json` or `format: text`"),
            )),
        })
        .collect::<Result<Vec<Record>>>()?;
    let service = ctx.doc().ok_or_else(|| {
        Error::new(
            ErrorCode::Unsupported,
            format!("`format: {format}` needs the documents group: load it or use `format: json`"),
        )
    })?;
    let bytes = service.serialize_records(&records, format, &SerializeOptions::default())?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => Content::Text(text),
        Err(error) => Content::Bytes(error.into_bytes()),
    })
}

fn batch(
    args: &BoundArgs,
    files: &Value,
    input: &StageInput<'_>,
    ctx: &ExecCtx,
) -> Result<Vec<Item>> {
    if let Some(name) = [TEXT, FROM, SIZE, DATA, OFFSET]
        .into_iter()
        .find(|name| present(args, name))
    {
        return Err(conflict(COMMAND, name, FILES));
    }
    if !input.is_none() {
        return Err(input_conflict(FILES));
    }
    let default = mode_param(args)?;
    let Value::List(entries) = files else {
        return Err(type_error(
            COMMAND,
            FILES,
            "a list of maps with `path`, `text` and `mode`",
        ));
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| batch_item(index + 1, entry, default, ctx))
        .collect()
}

fn item_error(number: usize, problem: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("item {number} of `files` {problem}: give a map with `path`, `text` and `mode`"),
    )
}

fn batch_item(number: usize, entry: &Value, default: Option<Mode>, ctx: &ExecCtx) -> Result<Item> {
    let fields = fields_of(entry).ok_or_else(|| item_error(number, "is not a map"))?;
    if let Some(name) = fields
        .keys()
        .find(|name| !ITEM_FIELDS.contains(&name.as_str()))
    {
        return Err(Error::new(
            ErrorCode::UnknownField,
            format!(
                "item {number} of `files` has the field `{name}`: use `path`, `text` and `mode`"
            ),
        )
        .with_found(name.as_str()));
    }
    let path = match fields.get(PATH) {
        Some(Value::Path(text) | Value::Text(text)) => resolve_for_change(ctx, text)?,
        Some(_) => return Err(item_error(number, "has a `path` that is not a path")),
        None => return Err(item_error(number, "has no `path`")),
    };
    let text = match fields.get(TEXT) {
        None | Some(Value::Null) => String::new(),
        Some(Value::Text(text)) => text.clone(),
        Some(_) => return Err(item_error(number, "has a `text` that is not a text")),
    };
    let mode = match fields.get(MODE) {
        None | Some(Value::Null) => default.ok_or_else(|| missing_mode(Some(&path)))?,
        Some(Value::Enum(text) | Value::Text(text)) => Mode::parse(text)?,
        Some(_) => return Err(item_error(number, "has a `mode` that is not a mode")),
    };
    Ok(Item {
        path,
        mode,
        content: Content::Text(text),
    })
}

/// A resolved file target: the path the change is made at, after a link in
/// the last component is followed, and its state as observed.
struct Target {
    path: AbsPath,
    observed: Observed,
}

/// The file at a target, as far as the change needs it.
enum Existing {
    None,
    Special,
    Binary(FileMeta),
    Text(TextFile),
}

fn exists_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Exists,
        format!("`{path}` already exists: use `mode: overwrite` to replace it or `mode: append` to add to it"),
    )
    .with_detail("path", Json::from(path.to_language_string()))
}

fn directory_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{path}` is a directory: give the path of a file"),
    )
}

fn special_error(path: &AbsPath, name: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!(
            "`{path}` is a special file, which `{name}` cannot change: write text or bytes to it"
        ),
    )
}

fn mode_conflict(name: &str, mode: Mode) -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!(
            "`{name}` cannot be used with `mode: {}`: use `mode: overwrite`",
            mode.as_str()
        ),
    )
}

fn existing_spec(path: &AbsPath, action: TargetAction, meta: &FileMeta, etag: Etag) -> TargetSpec {
    TargetSpec {
        size_hint: Some(meta.size),
        ..fs_target(path, action, Expect::Etag(etag))
    }
}

fn new_work(bytes: Vec<u8>, file_mode: Option<u32>) -> Work {
    match file_mode {
        None => Work::Apply(Mutation::WriteBytes(bytes)),
        Some(mode) => Work::Staged(Staged {
            bytes,
            offset: 0,
            length: None,
            mode: Some(mode),
        }),
    }
}

/// Fills the text fields of a `change` record: encodings, line endings and
/// byte order marks before and after, and the size after.
fn describe_text(
    change: &mut Change,
    before: Option<&TextFile>,
    encoding: &TextEncoding,
    written: &[u8],
) {
    if let Some(file) = before {
        change.encoding_before = Some(file.encoding.label.clone());
        change.line_ending_before = Some(record_line_ending(file.line_ending));
        change.bom_before = Some(file.encoding.bom);
    }
    change.encoding_after = Some(encoding.label.clone());
    change.line_ending_after = Some(record_line_ending(line_ending(written, encoding)));
    change.bom_after = Some(encoding.bom);
    change.size_after = Some(Size::from_bytes(byte_len(written)));
}

/// Text of a file before and after a write, checked as `validate` requires
/// once the write is applied.
#[derive(Debug)]
struct TextCheck {
    path: AbsPath,
    before: String,
    after: String,
}

/// State of a planned `write`: the planned operations and the text checks.
#[derive(Debug)]
struct Planned {
    ops: Vec<PlannedOp>,
    checks: Vec<TextCheck>,
    validate: Validate,
}

/// Collects the planned operations and the notable and irreversible effects
/// of one `write`.
struct Planner<'c> {
    ctx: &'c ExecCtx,
    settings: Settings,
    ops: Vec<PlannedOp>,
    checks: Vec<TextCheck>,
    notable: Vec<NotableEntry>,
    irreversible: Vec<IrreversibleEntry>,
    dirs: BTreeSet<AbsPath>,
    files: BTreeSet<AbsPath>,
}

impl<'c> Planner<'c> {
    fn new(ctx: &'c ExecCtx, settings: Settings) -> Self {
        Self {
            ctx,
            settings,
            ops: Vec::new(),
            checks: Vec::new(),
            notable: Vec::new(),
            irreversible: Vec::new(),
            dirs: BTreeSet::new(),
            files: BTreeSet::new(),
        }
    }

    fn finish(self) -> EffectPlan {
        let mut plan = EffectPlan::new();
        plan.changes = self.ops.iter().map(|op| op.change.clone()).collect();
        plan.notable = self.notable;
        plan.irreversible = self.irreversible;
        plan.bulk = self.ops.len() > 1;
        plan.with_state(Planned {
            ops: self.ops,
            checks: self.checks,
            validate: self.settings.validate,
        })
    }

    fn push(&mut self, target: TargetSpec, work: Work, change: Change) {
        self.ops.push(PlannedOp {
            target,
            work,
            change,
        });
    }

    fn note(&mut self, code: Notable, path: &AbsPath, detail: Option<String>) {
        self.notable.push(NotableEntry {
            detail,
            ..at_path(code, path)
        });
    }

    /// Keeps the text of a written file for the check of `validate`.
    fn check(&mut self, path: &AbsPath, before: &str, after: String) {
        if self.settings.validate != Validate::Off {
            self.checks.push(TextCheck {
                path: path.clone(),
                before: before.to_owned(),
                after,
            });
        }
    }

    fn claim(&mut self, path: &AbsPath) -> Result<()> {
        if self.files.insert(path.clone()) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::ParamConflict,
                format!("`{path}` is written twice: give every file once"),
            ))
        }
    }

    fn item(&mut self, item: Item, if_match: Option<&IfMatch>) -> Result<()> {
        let ctx = self.ctx;
        let fs = ctx.require_fs()?;
        self.claim(&item.path)?;
        let observed = observe(fs, &item.path)?;
        let expected = if_match.and_then(|tags| tags.expected(&item.path));
        check_if_match(&item.path, &observed, expected)?;
        let target = self.target(fs, &item, observed)?;
        if !target.observed.exists() {
            let parents = plan_parents(fs, &target.path, self.settings.parents, &mut self.dirs)?;
            self.ops.extend(parents);
        }
        match item.content {
            Content::Text(text) => self.text(&target, item.mode, &text),
            Content::Bytes(bytes) => self.bytes(&target, item.mode, bytes),
            Content::Data { bytes, offset } => self.data(&target, item.mode, bytes, offset),
            Content::Size(length) => self.size(&target, item.mode, length),
        }
    }

    fn target(&mut self, fs: &dyn Fs, item: &Item, observed: Observed) -> Result<Target> {
        match observed.meta().map(|meta| meta.kind) {
            None => Ok(Target {
                path: item.path.clone(),
                observed,
            }),
            Some(_) if item.mode == Mode::Create => Err(exists_error(&item.path)),
            Some(FileKind::Directory) => Err(directory_error(&item.path)),
            Some(FileKind::Symlink | FileKind::Junction) => self.follow(fs, &item.path),
            Some(_) => Ok(Target {
                path: item.path.clone(),
                observed,
            }),
        }
    }

    /// Follows a link to the file it names; the change is made there and the
    /// link stays a link.
    fn follow(&mut self, fs: &dyn Fs, link: &AbsPath) -> Result<Target> {
        let real = fs.real_path(link).map_err(|error| match error.kind {
            FsErrorKind::NotFound => Error::new(
                ErrorCode::NotFound,
                format!("link `{link}` points to a missing entry: create the target first"),
            ),
            _ => error.into(),
        })?;
        let observed = observe(fs, &real)?;
        if observed
            .meta()
            .is_some_and(|meta| meta.kind == FileKind::Directory)
        {
            return Err(directory_error(link));
        }
        if link.parent().is_none_or(|dir| !real.starts_with(&dir)) {
            self.note(
                Notable::SymlinkFollowed,
                link,
                Some(real.to_language_string()),
            );
        }
        if real != *link {
            self.claim(&real)?;
        }
        Ok(Target {
            path: real,
            observed,
        })
    }

    fn existing(&self, target: &Target, mode: Mode) -> Result<Existing> {
        let Some(meta) = target.observed.meta() else {
            return Ok(Existing::None);
        };
        if meta.kind == FileKind::Special {
            return Ok(Existing::Special);
        }
        let encoding = if mode == Mode::Overwrite {
            EncodingParam::Auto
        } else {
            self.settings.encoding
        };
        match read_text(self.ctx, &target.path, encoding) {
            Ok(file) => Ok(Existing::Text(file)),
            Err(error)
                if matches!(
                    error.code(),
                    ErrorCode::Binary | ErrorCode::Limit | ErrorCode::Encoding
                ) =>
            {
                Ok(Existing::Binary(meta.clone()))
            }
            Err(error) => Err(error),
        }
    }

    fn new_bytes_of(&self, text: &str, path: &AbsPath) -> Result<Vec<u8>> {
        let form = self.settings.new_form();
        let rendered = form.render(text, self.settings.ensure_newline);
        encode_text(&rendered, &form.encoding, path)
    }

    fn text(&mut self, target: &Target, mode: Mode, text: &str) -> Result<()> {
        match self.existing(target, mode)? {
            Existing::None => self.new_text(&target.path, text),
            Existing::Special => {
                let bytes = self.new_bytes_of(text, &target.path)?;
                self.special(&target.path, mode, bytes)
            }
            Existing::Binary(meta) => {
                let bytes = self.new_bytes_of(text, &target.path)?;
                self.existing_bytes(&target.path, &meta, mode, bytes)
            }
            Existing::Text(file) => match mode {
                Mode::Append => self.append_text(&file, text),
                Mode::Prepend => self.rewrite_text(&file, &format!("{text}{}", file.text), false),
                Mode::Create | Mode::Overwrite => self.rewrite_text(&file, text, true),
            },
        }
    }

    fn new_text(&mut self, path: &AbsPath, text: &str) -> Result<()> {
        let form = self.settings.new_form();
        let rendered = form.render(text, self.settings.ensure_newline);
        let bytes = encode_text(&rendered, &form.encoding, path)?;
        let target = fs_target(path, TargetAction::Create, Expect::Absent);
        let mut change = planned_change(&target);
        describe_text(&mut change, None, &form.encoding, &bytes);
        change.added = Some(count_lines(&rendered));
        self.push(target, new_work(bytes, self.settings.file_mode), change);
        self.check(path, "", rendered);
        Ok(())
    }

    fn rewrite_text(&mut self, file: &TextFile, text: &str, overwrite: bool) -> Result<()> {
        let form = self.settings.kept_form(file, false);
        let rendered = form.render(text, self.settings.ensure_newline);
        let bytes = encode_text(&rendered, &form.encoding, &file.path)?;
        if self.same_content(&file.path, file.size, &bytes)? {
            self.note(Notable::Unchanged, &file.path, None);
            return Ok(());
        }
        self.preserved_notes(file);
        self.change_notes(file, &form);
        self.replaced(&file.path, file.size, byte_len(&bytes), overwrite);
        let target = existing_spec(&file.path, TargetAction::Modify, &file.meta, file.etag());
        let mut change = planned_change(&target);
        describe_text(&mut change, Some(file), &form.encoding, &bytes);
        self.push(target, Work::Apply(Mutation::WriteBytes(bytes)), change);
        self.check(&file.path, &file.text, rendered);
        Ok(())
    }

    fn append_text(&mut self, file: &TextFile, text: &str) -> Result<()> {
        let form = self.settings.kept_form(file, true);
        let rendered = form.render(text, self.settings.ensure_newline);
        if rendered.is_empty() {
            self.note(Notable::Unchanged, &file.path, None);
            return Ok(());
        }
        let encoding = TextEncoding {
            label: form.encoding.label.clone(),
            bom: form.encoding.bom && file.size == 0,
        };
        let bytes = encode_text(&rendered, &encoding, &file.path)?;
        self.preserved_notes(file);
        let target = existing_spec(&file.path, TargetAction::Append, &file.meta, file.etag());
        let mut change = planned_change(&target);
        describe_text(&mut change, Some(file), &encoding, &bytes);
        let added = line_ending(&bytes, &encoding);
        change.line_ending_after = Some(record_line_ending(combined(file.line_ending, added)));
        change.bom_after = Some(file.encoding.bom || encoding.bom);
        change.size_after = Some(Size::from_bytes(file.size.saturating_add(byte_len(&bytes))));
        change.added = Some(count_lines(&rendered));
        self.push(target, Work::Apply(Mutation::AppendBytes(bytes)), change);
        self.check(&file.path, &file.text, format!("{}{rendered}", file.text));
        Ok(())
    }

    /// `encoding_preserved` and `line_ending_preserved` for the parts of the
    /// form taken from the file.
    fn preserved_notes(&mut self, file: &TextFile) {
        let auto = self.settings.encoding == EncodingParam::Auto;
        let keep = self.settings.line_ending == LineEndingParam::Keep;
        self.notable
            .extend(preserved_text(file).into_iter().filter(|entry| {
                if entry.code == Notable::EncodingPreserved {
                    auto
                } else {
                    keep
                }
            }));
    }

    /// `encoding_changed`, `bom_changed` and `line_ending_changed` for a file
    /// rewritten in another form.
    fn change_notes(&mut self, file: &TextFile, form: &Form<'_>) {
        if form.encoding.label != file.encoding.label {
            let detail = format!("{} -> {}", file.encoding.label, form.encoding.label);
            self.note(Notable::EncodingChanged, &file.path, Some(detail));
        }
        if form.encoding.bom != file.encoding.bom {
            self.note(Notable::BomChanged, &file.path, None);
        }
        let changed = self.settings.terminator().is_some_and(|terminator| {
            file.line_ending != LineEnding::None
                && terminator_of(file.line_ending) != Some(terminator)
        });
        if changed {
            self.note(Notable::LineEndingChanged, &file.path, None);
        }
    }

    /// `overwrote_existing` for `mode: overwrite`, and the size notes.
    fn replaced(&mut self, path: &AbsPath, before: u64, after: u64, overwrite: bool) {
        if overwrite {
            self.note(Notable::OverwroteExisting, path, None);
        }
        self.size_notes(path, before, after);
    }

    fn size_notes(&mut self, path: &AbsPath, before: u64, after: u64) {
        if after == 0 && before > 0 {
            self.note(Notable::FileEmptied, path, None);
        } else if before >= SHRINK_MIN_BYTES
            && after < before
            && ((before - after) as f64) >= (before as f64) * self.settings.shrink_ratio
        {
            self.note(
                Notable::SizeShrunk,
                path,
                Some(format!("{before} -> {after} bytes")),
            );
        }
    }

    fn same_content(&self, path: &AbsPath, size: u64, bytes: &[u8]) -> Result<bool> {
        if byte_len(bytes) != size || size > self.ctx.limits().max_file_bytes_text {
            return Ok(false);
        }
        Ok(read_bytes(self.ctx, path)? == bytes)
    }

    fn bytes(&mut self, target: &Target, mode: Mode, bytes: Vec<u8>) -> Result<()> {
        match target.observed.meta() {
            None => {
                self.new_file(&target.path, bytes);
                Ok(())
            }
            Some(meta) if meta.kind == FileKind::Special => self.special(&target.path, mode, bytes),
            Some(meta) => self.existing_bytes(&target.path, meta, mode, bytes),
        }
    }

    fn new_file(&mut self, path: &AbsPath, bytes: Vec<u8>) {
        let target = fs_target(path, TargetAction::Create, Expect::Absent);
        let mut change = planned_change(&target);
        change.size_after = Some(Size::from_bytes(byte_len(&bytes)));
        self.push(target, new_work(bytes, self.settings.file_mode), change);
    }

    fn existing_bytes(
        &mut self,
        path: &AbsPath,
        meta: &FileMeta,
        mode: Mode,
        bytes: Vec<u8>,
    ) -> Result<()> {
        match mode {
            Mode::Append => {
                self.append_bytes(path, meta, bytes);
                Ok(())
            }
            Mode::Prepend => {
                let mut whole = bytes;
                whole.extend(read_bytes(self.ctx, path)?);
                self.replace_bytes(path, meta, whole, false)
            }
            Mode::Create | Mode::Overwrite => self.replace_bytes(path, meta, bytes, true),
        }
    }

    fn append_bytes(&mut self, path: &AbsPath, meta: &FileMeta, bytes: Vec<u8>) {
        if bytes.is_empty() {
            self.note(Notable::Unchanged, path, None);
            return;
        }
        let target = existing_spec(path, TargetAction::Append, meta, meta.etag());
        let mut change = planned_change(&target);
        change.size_after = Some(Size::from_bytes(meta.size.saturating_add(byte_len(&bytes))));
        self.push(target, Work::Apply(Mutation::AppendBytes(bytes)), change);
    }

    fn replace_bytes(
        &mut self,
        path: &AbsPath,
        meta: &FileMeta,
        bytes: Vec<u8>,
        overwrite: bool,
    ) -> Result<()> {
        if self.same_content(path, meta.size, &bytes)? {
            self.note(Notable::Unchanged, path, None);
            return Ok(());
        }
        self.replaced(path, meta.size, byte_len(&bytes), overwrite);
        let target = existing_spec(path, TargetAction::Modify, meta, meta.etag());
        let mut change = planned_change(&target);
        change.size_after = Some(Size::from_bytes(byte_len(&bytes)));
        self.push(target, Work::Apply(Mutation::WriteBytes(bytes)), change);
        Ok(())
    }

    /// Bytes written to a device, a pipe or a socket: appended without
    /// truncation and never undoable.
    fn special(&mut self, path: &AbsPath, mode: Mode, bytes: Vec<u8>) -> Result<()> {
        if mode == Mode::Prepend {
            return Err(special_error(path, "mode: prepend"));
        }
        let target = fs_target(path, TargetAction::Append, Expect::Unchecked);
        let mut change = planned_change(&target);
        change.irreversible = Some(Irreversible::SpecialFile);
        self.note(Notable::SpecialFile, path, None);
        self.irreversible.push(IrreversibleEntry {
            path: Some(path.to_language_string()),
            raw_path: path.raw_bytes(),
            ..IrreversibleEntry::new(Irreversible::SpecialFile)
        });
        self.push(target, Work::Apply(Mutation::AppendBytes(bytes)), change);
        Ok(())
    }

    fn data(&mut self, target: &Target, mode: Mode, bytes: Vec<u8>, offset: u64) -> Result<()> {
        if mode.adds() {
            return Err(mode_conflict(OFFSET, mode));
        }
        let end = offset.saturating_add(byte_len(&bytes));
        match target.observed.meta() {
            None => {
                let spec = fs_target(&target.path, TargetAction::Create, Expect::Absent);
                let mut change = planned_change(&spec);
                change.size_after = Some(Size::from_bytes(end));
                let staged = Staged {
                    bytes,
                    offset,
                    length: None,
                    mode: self.settings.file_mode,
                };
                self.push(spec, Work::Staged(staged), change);
                Ok(())
            }
            Some(meta) if meta.kind == FileKind::Special => {
                Err(special_error(&target.path, OFFSET))
            }
            Some(meta) => {
                let spec = existing_spec(&target.path, TargetAction::Modify, meta, meta.etag());
                let mut change = planned_change(&spec);
                change.size_after = Some(Size::from_bytes(end.max(meta.size)));
                self.push(
                    spec,
                    Work::Apply(Mutation::WriteAt { offset, bytes }),
                    change,
                );
                Ok(())
            }
        }
    }

    fn size(&mut self, target: &Target, mode: Mode, length: u64) -> Result<()> {
        if mode.adds() {
            return Err(mode_conflict(SIZE, mode));
        }
        match target.observed.meta() {
            None => {
                let spec = fs_target(&target.path, TargetAction::Create, Expect::Absent);
                let mut change = planned_change(&spec);
                change.size_after = Some(Size::from_bytes(length));
                let staged = Staged {
                    bytes: Vec::new(),
                    offset: 0,
                    length: Some(length),
                    mode: self.settings.file_mode,
                };
                self.push(spec, Work::Staged(staged), change);
                Ok(())
            }
            Some(meta) if meta.kind == FileKind::Special => Err(special_error(&target.path, SIZE)),
            Some(meta) if meta.size == length => {
                self.note(Notable::Unchanged, &target.path, None);
                Ok(())
            }
            Some(meta) => {
                self.size_notes(&target.path, meta.size, length);
                let spec = existing_spec(&target.path, TargetAction::Truncate, meta, meta.etag());
                let mut change = planned_change(&spec);
                change.size_after = Some(Size::from_bytes(length));
                self.push(spec, Work::Apply(Mutation::SetLength(length)), change);
                Ok(())
            }
        }
    }
}

fn registry_path(args: &BoundArgs) -> Option<&str> {
    match args.value(PATH) {
        Some(Value::Path(text) | Value::Text(text))
            if registry::is_registry_path(text.as_str()) =>
        {
            Some(text.as_str())
        }
        _ => None,
    }
}

fn plan_registry(args: &BoundArgs, raw: &str, mut planner: Planner<'_>) -> Result<EffectPlan> {
    let registry = registry::access(planner.ctx, raw)?;
    if let Some(name) = [FROM, SIZE, OFFSET, FILES]
        .into_iter()
        .find(|name| present(args, name))
    {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            format!("`{name}` cannot be used with a registry path"),
        ));
    }
    let (action, expect) = match required_mode(args, None)? {
        Mode::Create => (TargetAction::Create, Expect::Absent),
        Mode::Overwrite => (TargetAction::Modify, Expect::Unchecked),
        other => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "`mode: {}` is not available for registry values: use `create` or `overwrite`",
                    other.as_str()
                ),
            ));
        }
    };
    let target = TargetSpec {
        path: TargetPath::Provider(registry.address(raw, RegistryKind::Value)?.key),
        action,
        expect,
        size_hint: None,
    };
    let value = registry_value(args, planner.ctx)?;
    let change = planned_change(&target);
    planner.push(target, Work::Apply(Mutation::ProviderSet(value)), change);
    Ok(planner.finish())
}

fn registry_value(args: &BoundArgs, ctx: &ExecCtx) -> Result<ProviderValue> {
    let kind = choice(args, VALUE_TYPE, &VALUE_TYPES)?.unwrap_or("string");
    if kind == "binary" {
        return match args.value(DATA) {
            Some(Value::Bytes(bytes)) => Ok(ProviderValue::Binary {
                raw_type: REG_BINARY,
                bytes: bytes.as_slice().to_vec(),
            }),
            _ => Err(Error::new(
                ErrorCode::MissingParam,
                "a `binary` registry value needs `data`: give its bytes",
            )),
        };
    }
    if present(args, DATA) {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            format!("`data` is written only to `binary` registry values, not `{kind}`: use `text`"),
        ));
    }
    let text = text_value(args, ctx)?;
    match kind {
        "dword" => registry_number(&text).and_then(|number| {
            u32::try_from(number)
                .map(ProviderValue::U32)
                .map_err(|_| number_error(&text))
        }),
        "qword" => registry_number(&text).map(ProviderValue::U64),
        "multi_string" => Ok(ProviderValue::TextList(
            text.lines().map(str::to_owned).collect(),
        )),
        "expand_string" => Ok(ProviderValue::ExpandableText(text)),
        _ => Ok(ProviderValue::Text(text)),
    }
}

fn registry_number(text: &str) -> Result<u64> {
    let trimmed = text.trim();
    let parsed = match HEX_PREFIXES
        .into_iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
    {
        Some(hex) => u64::from_str_radix(hex, HEX_RADIX),
        None => trimmed.parse(),
    };
    parsed.map_err(|_| number_error(text))
}

fn number_error(text: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{text}` is not a number for this registry value: write a decimal or 0x hexadecimal number"),
    )
    .with_found(text)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use nurt_contracts::fs::{ContentHash, Fs};
    use nurt_contracts::journal::{JournalStore, RollbackReason};
    use nurt_contracts::keypath::KeyPath;
    use nurt_contracts::record::{ChangeAction, LineEnding as RecordLineEnding};
    use nurt_contracts::services::{
        ByteRange, DocFormatInfo, DocItem, DocService, IssueSeverity, Services, SyntaxIssue,
        TextPosition, TextSpan,
    };
    use nurt_contracts::stage::{Event, LogLevel, RecordStream};
    use nurt_contracts::value::{Bytes, RecordKind};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::host::TestHost;
    use nurt_testkit::journal::JournalCall;

    use super::*;
    use crate::commands::validate::VALIDATE;
    use crate::common::etag::etag_text;

    /// Windows-1250 text with Polish letters and CRLF line endings.
    const CP1250_CRLF: &[u8] = b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n";

    fn abs(text: &str) -> AbsPath {
        nurt_testkit::vfs::test_path(text).unwrap()
    }

    fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs().write_file(&path, content).unwrap();
        (test, path)
    }

    #[derive(Debug)]
    struct Run {
        changes: Vec<Change>,
        planned: Vec<Change>,
        notable: Vec<Notable>,
        irreversible: Vec<Irreversible>,
        bulk: bool,
    }

    fn path_value(path: &str) -> Value {
        Value::Path(abs(path).to_language_string())
    }

    fn set(args: BoundArgs, name: &str, value: Value) -> BoundArgs {
        args.with(name, ArgValue::Value(value), false)
    }

    fn args(path: &str, mode: &str) -> BoundArgs {
        let args = set(BoundArgs::new(COMMAND), PATH, path_value(path));
        set(args, MODE, Value::Enum(mode.to_owned()))
    }

    fn with_text(path: &str, mode: &str, text: &str) -> BoundArgs {
        set(args(path, mode), TEXT, Value::Text(text.to_owned()))
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        run_with(test, &test.ctx(), args, StageInput::none())
    }

    fn run_with(
        test: &TestContext,
        ctx: &ExecCtx,
        args: &BoundArgs,
        input: StageInput<'_>,
    ) -> Result<Run> {
        let plan = Write.plan(args, input, ctx)?;
        let planned = plan.changes.clone();
        let notable = plan.notable.iter().map(|entry| entry.code).collect();
        let irreversible = plan.irreversible.iter().map(|entry| entry.reason).collect();
        let bulk = plan.bulk;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let changes = Write.apply(args, plan, tx.as_mut(), ctx)?.changes;
        tx.commit()?;
        Ok(Run {
            changes,
            planned,
            notable,
            irreversible,
            bulk,
        })
    }

    fn read(test: &TestContext, path: &str) -> Vec<u8> {
        test.fs().read_file(&abs(path)).unwrap()
    }

    fn assert_every_change_declared(test: &TestContext) {
        let calls = test.journal().calls();
        let declared: usize = calls
            .iter()
            .map(|call| match call {
                JournalCall::Preserve { targets, .. } => targets.len(),
                _ => 0,
            })
            .sum();
        let applied = calls
            .iter()
            .filter(|call| matches!(call, JournalCall::Apply { ok: true, .. }))
            .count();
        assert!(declared > 0);
        assert_eq!(declared, applied);
    }

    fn mutations(test: &TestContext) -> Vec<&'static str> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Apply { mutation, .. } => Some(mutation),
                _ => None,
            })
            .collect()
    }

    fn records(kind: RecordKind, rows: &[&[(&str, Value)]]) -> Vec<Record> {
        rows.iter()
            .map(|fields| {
                fields
                    .iter()
                    .fold(Record::new(kind), |record, (name, value)| {
                        record.with(name, value.clone())
                    })
            })
            .collect()
    }

    fn text_records(lines: &[&str]) -> Vec<Record> {
        lines
            .iter()
            .map(|line| {
                Record::new(RecordKind::Line).with(TEXT_FIELD, Value::Text((*line).to_owned()))
            })
            .collect()
    }

    fn data_dir() -> TestContext {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/data")).unwrap();
        test
    }

    #[test]
    fn create_writes_a_new_utf8_file_with_line_feeds() {
        let test = data_dir();
        let run = run(
            &test,
            &with_text("/data/new.txt", "create", "one\r\ntwo\r\n"),
        )
        .unwrap();
        assert_eq!(read(&test, "/data/new.txt"), b"one\ntwo\n");
        assert_eq!(run.changes.len(), 1);
        let change = &run.changes[0];
        assert_eq!(change.action, ChangeAction::Create);
        assert_eq!(change.size_after, Some(Size::from_bytes(8)));
        assert_eq!(change.encoding_after.as_deref(), Some("utf-8"));
        assert_eq!(change.bom_after, Some(false));
        assert_eq!(change.line_ending_after, Some(RecordLineEnding::Lf));
        assert!(!run.bulk);
        assert_every_change_declared(&test);
    }

    #[test]
    fn create_fails_on_an_existing_file_without_changes() {
        let (test, path) = with_file("/data/a.txt", b"old");
        let error = run(&test, &with_text("/data/a.txt", "create", "new")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"old");
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn missing_mode_fails_with_ready_programs() {
        let test = data_dir();
        let given = set(
            set(BoundArgs::new(COMMAND), PATH, path_value("/data/a.txt")),
            TEXT,
            Value::Text("x".to_owned()),
        );
        let error = run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        assert_eq!(error.fix.len(), 3);
        assert!(error.fix[0].ends_with("mode: create"));
    }

    #[test]
    fn overwrite_keeps_crlf_and_the_byte_order_mark() {
        let (test, _) = with_file("/data/a.txt", b"\xef\xbb\xbfold\r\nlines\r\n");
        let run = run(&test, &with_text("/data/a.txt", "overwrite", "new\nfile\n")).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"\xef\xbb\xbfnew\r\nfile\r\n");
        assert!(run.notable.contains(&Notable::OverwroteExisting));
        assert!(run.notable.contains(&Notable::LineEndingPreserved));
        let change = &run.changes[0];
        assert_eq!(change.action, ChangeAction::Modify);
        assert_eq!(change.bom_before, Some(true));
        assert_eq!(change.bom_after, Some(true));
        assert_eq!(change.line_ending_after, Some(RecordLineEnding::Crlf));
        assert_eq!(change.size_before, Some(Size::from_bytes(15)));
        assert_every_change_declared(&test);
    }

    #[test]
    fn overwrite_rejects_text_the_legacy_encoding_cannot_represent() {
        let (test, _) = with_file("/data/legacy.txt", CP1250_CRLF);
        let error = run(
            &test,
            &with_text("/data/legacy.txt", "overwrite", "\u{4e2d}\n"),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
        assert!(error.fix[0].starts_with("convert "));
        assert_eq!(read(&test, "/data/legacy.txt"), CP1250_CRLF);
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn overwrite_keeps_a_legacy_encoding() {
        let (test, _) = with_file("/data/legacy.txt", CP1250_CRLF);
        let written = with_text("/data/legacy.txt", "overwrite", "\u{17c}\u{f3}\u{142}w\n");
        let run = run(&test, &written).unwrap();
        assert_eq!(read(&test, "/data/legacy.txt"), b"\xbf\xf3\xb3w\r\n");
        assert!(run.notable.contains(&Notable::EncodingPreserved));
        assert_eq!(
            run.changes[0].encoding_after.as_deref(),
            Some("windows-1250")
        );
    }

    #[test]
    fn overwrite_reports_shrunk_and_emptied_files() {
        let (test, _) = with_file("/data/big.txt", &[b'x'; 2048]);
        let shrunk = run(&test, &with_text("/data/big.txt", "overwrite", "short")).unwrap();
        assert!(shrunk.notable.contains(&Notable::SizeShrunk));
        assert!(!shrunk.notable.contains(&Notable::FileEmptied));
        let emptied = run(&test, &with_text("/data/big.txt", "overwrite", "")).unwrap();
        assert!(emptied.notable.contains(&Notable::FileEmptied));
        assert!(read(&test, "/data/big.txt").is_empty());
    }

    #[test]
    fn size_shrunk_follows_the_share_by_which_the_file_shrinks() {
        let (test, _) = with_file("/data/big.txt", &[b'x'; 2048]);
        let mut config = config::Config::default();
        config.report.notable.shrink_ratio = 0.8;
        let ctx = test.builder().config(Arc::new(config)).build();
        let half = "y".repeat(1024);
        let halved = run_with(
            &test,
            &ctx,
            &with_text("/data/big.txt", "overwrite", &half),
            StageInput::none(),
        )
        .unwrap();
        assert!(!halved.notable.contains(&Notable::SizeShrunk));
        let tenth = run_with(
            &test,
            &ctx,
            &with_text("/data/big.txt", "overwrite", &"z".repeat(100)),
            StageInput::none(),
        )
        .unwrap();
        assert!(tenth.notable.contains(&Notable::SizeShrunk));
    }

    #[test]
    fn growing_a_file_is_not_reported_as_shrunk() {
        let (test, _) = with_file("/data/big.txt", &[b'x'; 2048]);
        let grown = "y".repeat(4096);
        let run = run(&test, &with_text("/data/big.txt", "overwrite", &grown)).unwrap();
        assert!(!run.notable.contains(&Notable::SizeShrunk));
        assert!(!run.notable.contains(&Notable::FileEmptied));
    }

    #[test]
    fn identical_content_is_unchanged_and_declares_no_target() {
        let (test, _) = with_file("/data/a.txt", b"same\n");
        let run = run(&test, &with_text("/data/a.txt", "overwrite", "same\n")).unwrap();
        assert!(run.changes.is_empty());
        assert_eq!(run.notable, [Notable::Unchanged]);
        assert!(
            !test
                .journal()
                .calls()
                .iter()
                .any(|call| matches!(call, JournalCall::Preserve { .. }))
        );
    }

    #[test]
    fn append_adds_text_in_the_form_of_the_file() {
        let (test, _) = with_file("/data/a.txt", b"\xef\xbb\xbfa\r\n");
        let run = run(&test, &with_text("/data/a.txt", "append", "b\n")).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"\xef\xbb\xbfa\r\nb\r\n");
        assert_eq!(mutations(&test), ["append_bytes"]);
        let change = &run.changes[0];
        assert_eq!(change.size_before, Some(Size::from_bytes(6)));
        assert_eq!(change.size_after, Some(Size::from_bytes(9)));
        assert_eq!(change.bom_after, Some(true));
        assert_eq!(change.line_ending_after, Some(RecordLineEnding::Crlf));
        assert!(!run.notable.contains(&Notable::OverwroteExisting));
    }

    #[test]
    fn append_and_prepend_create_missing_files() {
        let test = data_dir();
        run(&test, &with_text("/data/a.txt", "append", "a\n")).unwrap();
        run(&test, &with_text("/data/b.txt", "prepend", "b\n")).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"a\n");
        assert_eq!(read(&test, "/data/b.txt"), b"b\n");
        assert_every_change_declared(&test);
    }

    #[test]
    fn prepend_puts_text_before_the_content() {
        let (test, _) = with_file("/data/a.txt", b"body\r\n");
        let run = run(&test, &with_text("/data/a.txt", "prepend", "head\n")).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"head\r\nbody\r\n");
        assert!(!run.notable.contains(&Notable::OverwroteExisting));
        assert_eq!(mutations(&test), ["write_bytes"]);
    }

    #[test]
    fn missing_parents_are_created_before_the_file() {
        let test = TestContext::new();
        let run = run(&test, &with_text("/data/a/b/c.txt", "create", "x")).unwrap();
        let actions: Vec<ChangeAction> = run.changes.iter().map(|change| change.action).collect();
        assert_eq!(actions, [ChangeAction::Create; 4]);
        assert_eq!(
            run.changes[0].path.as_str(),
            abs("/data").to_language_string()
        );
        assert!(run.bulk);
        assert_eq!(read(&test, "/data/a/b/c.txt"), b"x");
        assert_every_change_declared(&test);

        let without = set(
            with_text("/other/d/e.txt", "create", "x"),
            PARENTS,
            Value::Bool(false),
        );
        let error = super::tests::run(&test, &without).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
        assert!(!test.fs().exists(&abs("/other")));
    }

    #[test]
    fn directories_are_not_written() {
        let test = data_dir();
        let error = run(&test, &with_text("/data", "overwrite", "x")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn links_are_followed_to_the_file_they_name() {
        let (test, real) = with_file("/data/real.txt", b"old\n");
        test.fs().create_dir_all(&abs("/other")).unwrap();
        let link = abs("/other/link.txt");
        test.fs()
            .symlink(real.as_path().to_path_buf(), &link)
            .unwrap();
        let run = run(&test, &with_text("/other/link.txt", "overwrite", "new\n")).unwrap();
        assert_eq!(test.fs().read_file(&real).unwrap(), b"new\n");
        let kind = test.fs().metadata(&link, Follow::NoFollow).unwrap().kind;
        assert_eq!(kind, FileKind::Symlink);
        assert!(run.notable.contains(&Notable::SymlinkFollowed));
        assert_eq!(run.changes[0].path.as_str(), real.to_language_string());
    }

    #[test]
    fn explicit_encoding_and_byte_order_mark_are_applied() {
        let test = data_dir();
        let legacy = set(
            with_text("/data/legacy.txt", "create", "\u{142}\n"),
            "encoding",
            Value::Text("windows-1250".to_owned()),
        );
        run(&test, &legacy).unwrap();
        assert_eq!(read(&test, "/data/legacy.txt"), b"\xb3\n");

        let marked = set(
            with_text("/data/bom.txt", "create", "a\n"),
            BOM,
            Value::Bool(true),
        );
        run(&test, &marked).unwrap();
        assert_eq!(read(&test, "/data/bom.txt"), b"\xef\xbb\xbfa\n");

        test.fs()
            .write_file(&abs("/data/utf8.txt"), b"x\n")
            .unwrap();
        let converted = set(
            with_text("/data/utf8.txt", "overwrite", "\u{142}\n"),
            "encoding",
            Value::Text("cp1250".to_owned()),
        );
        let run = run(&test, &converted).unwrap();
        assert_eq!(read(&test, "/data/utf8.txt"), b"\xb3\n");
        assert!(run.notable.contains(&Notable::EncodingChanged));
    }

    #[test]
    fn explicit_line_endings_convert_the_text() {
        let test = data_dir();
        let crlf = set(
            with_text("/data/new.txt", "create", "a\nb"),
            "line_ending",
            Value::Enum("crlf".to_owned()),
        );
        run(&test, &crlf).unwrap();
        assert_eq!(read(&test, "/data/new.txt"), b"a\r\nb");

        test.fs()
            .write_file(&abs("/data/lf.txt"), b"x\ny\n")
            .unwrap();
        let converted = set(
            with_text("/data/lf.txt", "overwrite", "x\nz\n"),
            "line_ending",
            Value::Enum("crlf".to_owned()),
        );
        let run = run(&test, &converted).unwrap();
        assert_eq!(read(&test, "/data/lf.txt"), b"x\r\nz\r\n");
        assert!(run.notable.contains(&Notable::LineEndingChanged));
    }

    #[test]
    fn ensure_newline_ends_the_file_with_its_line_ending() {
        let (test, _) = with_file("/data/a.txt", b"x\r\n");
        let crlf = set(
            with_text("/data/a.txt", "overwrite", "a\r\nb"),
            ENSURE_NEWLINE,
            Value::Bool(true),
        );
        run(&test, &crlf).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"a\r\nb\r\n");

        let new = set(
            with_text("/data/b.txt", "create", "a"),
            ENSURE_NEWLINE,
            Value::Bool(true),
        );
        run(&test, &new).unwrap();
        assert_eq!(read(&test, "/data/b.txt"), b"a\n");

        let empty = set(
            with_text("/data/c.txt", "create", ""),
            ENSURE_NEWLINE,
            Value::Bool(true),
        );
        run(&test, &empty).unwrap();
        assert!(read(&test, "/data/c.txt").is_empty());
    }

    #[test]
    fn file_mode_sets_the_permissions_of_a_new_file() {
        let test = data_dir();
        let given = set(
            with_text("/data/secret.txt", "create", "s"),
            FILE_MODE,
            Value::Text("0640".to_owned()),
        );
        run(&test, &given).unwrap();
        let meta = test
            .fs()
            .metadata(&abs("/data/secret.txt"), Follow::NoFollow)
            .unwrap();
        assert_eq!(meta.mode & PERMISSION_BITS, 0o640);
        assert_eq!(read(&test, "/data/secret.txt"), b"s");
        assert_eq!(mutations(&test), ["commit_staged"]);
        assert_every_change_declared(&test);

        for invalid in ["0999", "77777", "rw-r--r--", ""] {
            let given = set(
                with_text("/data/x.txt", "create", "s"),
                FILE_MODE,
                Value::Text(invalid.to_owned()),
            );
            assert_eq!(
                run(&test, &given).unwrap_err().code(),
                ErrorCode::InvalidValue,
                "{invalid}"
            );
        }
    }

    #[test]
    fn overwrite_keeps_the_permissions_of_the_file() {
        let (test, path) = with_file("/data/run.sh", b"echo old\n");
        test.fs().set_mode(&path, 0o755).unwrap();
        run(&test, &with_text("/data/run.sh", "overwrite", "echo new\n")).unwrap();
        let meta = test.fs().metadata(&path, Follow::NoFollow).unwrap();
        assert_eq!(meta.mode & PERMISSION_BITS, 0o755);
    }

    #[test]
    fn size_truncates_extends_and_creates_files() {
        let (test, _) = with_file("/data/a.bin", b"0123456789");
        let size = |path: &str, bytes: u64, mode: &str| {
            set(args(path, mode), SIZE, Value::Size(Size::from_bytes(bytes)))
        };
        let shrunk = run(&test, &size("/data/a.bin", 4, "overwrite")).unwrap();
        assert_eq!(read(&test, "/data/a.bin"), b"0123");
        assert_eq!(shrunk.changes[0].action, ChangeAction::Truncate);
        run(&test, &size("/data/a.bin", 6, "overwrite")).unwrap();
        assert_eq!(read(&test, "/data/a.bin"), b"0123\0\0");
        let same = run(&test, &size("/data/a.bin", 6, "overwrite")).unwrap();
        assert_eq!(same.notable, [Notable::Unchanged]);
        run(&test, &size("/data/new.bin", 3, "create")).unwrap();
        assert_eq!(read(&test, "/data/new.bin"), [0; 3]);
        let emptied = run(&test, &size("/data/a.bin", 0, "overwrite")).unwrap();
        assert!(emptied.notable.contains(&Notable::FileEmptied));
        let error = run(&test, &size("/data/a.bin", 1, "append")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
        assert_every_change_declared(&test);
    }

    #[test]
    fn data_is_written_in_place_at_an_offset() {
        let (test, _) = with_file("/data/a.bin", b"abcdef");
        let at = |path: &str, mode: &str, offset: Option<i64>| {
            let given = set(
                args(path, mode),
                DATA,
                Value::Bytes(Bytes::new(b"XY".to_vec())),
            );
            match offset {
                Some(offset) => set(given, OFFSET, Value::Int(offset)),
                None => given,
            }
        };
        let run = run(&test, &at("/data/a.bin", "overwrite", Some(2))).unwrap();
        assert_eq!(read(&test, "/data/a.bin"), b"abXYef");
        assert_eq!(run.changes[0].size_after, Some(Size::from_bytes(6)));
        assert_eq!(mutations(&test), ["write_at"]);

        super::tests::run(&test, &at("/data/b.bin", "create", Some(2))).unwrap();
        assert_eq!(read(&test, "/data/b.bin"), b"\0\0XY");
        super::tests::run(&test, &at("/data/c.bin", "create", None)).unwrap();
        assert_eq!(read(&test, "/data/c.bin"), b"XY");

        let append = super::tests::run(&test, &at("/data/a.bin", "append", Some(1))).unwrap_err();
        assert_eq!(append.code(), ErrorCode::ParamConflict);
        let negative =
            super::tests::run(&test, &at("/data/a.bin", "overwrite", Some(-1))).unwrap_err();
        assert_eq!(negative.code(), ErrorCode::InvalidValue);
        let alone = set(args("/data/a.bin", "overwrite"), OFFSET, Value::Int(1));
        assert_eq!(
            super::tests::run(&test, &alone).unwrap_err().code(),
            ErrorCode::ParamConflict
        );
        assert_every_change_declared(&test);
    }

    #[test]
    fn input_records_are_written_in_the_format_of_the_extension() {
        let test = data_dir();
        let rows = records(
            RecordKind::Row,
            &[
                &[
                    ("name", Value::Text("alpha".to_owned())),
                    ("size", Value::Int(1)),
                ],
                &[
                    ("name", Value::Text("b,c".to_owned())),
                    ("size", Value::Int(2)),
                ],
            ],
        );
        let write = |path: &str| {
            let input = StageInput::records(RecordStream::from_records(rows.clone()));
            run_with(&test, &test.ctx(), &args(path, "create"), input).unwrap();
            read(&test, path)
        };
        let json: Json = serde_json::from_slice(&write("/data/rows.json")).unwrap();
        assert_eq!(
            json,
            serde_json::json!([{"name": "alpha", "size": 1}, {"name": "b,c", "size": 2}])
        );
        assert_eq!(write("/data/rows.csv"), b"name,size\nalpha,1\n\"b,c\",2\n");
        assert_eq!(write("/data/rows.tsv"), b"name\tsize\nalpha\t1\nb,c\t2\n");

        let input = StageInput::records(RecordStream::from_records(text_records(&["b", "a"])));
        run_with(
            &test,
            &test.ctx(),
            &args("/data/lines.txt", "create"),
            input,
        )
        .unwrap();
        assert_eq!(read(&test, "/data/lines.txt"), b"b\na\n");
    }

    #[test]
    fn records_and_text_together_are_rejected() {
        let test = data_dir();
        let input = StageInput::records(RecordStream::from_records(text_records(&["a"])));
        let error = run_with(
            &test,
            &test.ctx(),
            &with_text("/data/a.txt", "create", "x"),
            input,
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
    }

    #[test]
    fn yaml_needs_the_documents_group() {
        let test = data_dir();
        let input = StageInput::records(RecordStream::from_records(text_records(&["a"])));
        let error =
            run_with(&test, &test.ctx(), &args("/data/a.yaml", "create"), input).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Unsupported);
        assert!(!test.fs().exists(&abs("/data/a.yaml")));
    }

    #[test]
    fn from_writes_a_file_or_a_named_result() {
        let (test, _) = with_file("/data/source.txt", b"copied\r\n");
        let from_path = set(
            args("/data/copy.txt", "create"),
            FROM,
            path_value("/data/source.txt"),
        );
        run(&test, &from_path).unwrap();
        assert_eq!(read(&test, "/data/copy.txt"), b"copied\n");

        let named = Value::List(
            text_records(&["x", "y"])
                .into_iter()
                .map(Value::Record)
                .collect(),
        );
        let from_value = set(args("/data/named.txt", "create"), FROM, named);
        run(&test, &from_value).unwrap();
        assert_eq!(read(&test, "/data/named.txt"), b"x\ny\n");

        let both = set(from_value, TEXT, Value::Text("z".to_owned()));
        assert_eq!(
            run(&test, &both).unwrap_err().code(),
            ErrorCode::ParamConflict
        );
    }

    fn batch_entry(path: &str, text: &str, mode: Option<&str>) -> Value {
        let mut fields = Fields::new();
        fields.insert(PATH.to_owned(), path_value(path));
        fields.insert(TEXT.to_owned(), Value::Text(text.to_owned()));
        if let Some(mode) = mode {
            fields.insert(MODE.to_owned(), Value::Enum(mode.to_owned()));
        }
        Value::Map(fields)
    }

    fn batch_args(entries: Vec<Value>, mode: &str) -> BoundArgs {
        set(
            set(BoundArgs::new(COMMAND), FILES, Value::List(entries)),
            MODE,
            Value::Enum(mode.to_owned()),
        )
    }

    #[test]
    fn a_batch_is_written_in_one_transaction() {
        let (test, _) = with_file("/data/log.txt", b"one\n");
        let given = batch_args(
            vec![
                batch_entry("/data/a.txt", "a\n", None),
                batch_entry("/data/log.txt", "two\n", Some("append")),
            ],
            "create",
        );
        let run = run(&test, &given).unwrap();
        assert!(run.bulk);
        assert_eq!(run.changes.len(), 2);
        assert_eq!(read(&test, "/data/a.txt"), b"a\n");
        assert_eq!(read(&test, "/data/log.txt"), b"one\ntwo\n");
        let preserve_calls = test
            .journal()
            .calls()
            .iter()
            .filter(|call| matches!(call, JournalCall::Preserve { .. }))
            .count();
        assert_eq!(preserve_calls, 1);
        assert_every_change_declared(&test);
    }

    #[test]
    fn a_failing_batch_item_changes_nothing() {
        let (test, _) = with_file("/data/exists.txt", b"keep");
        let given = batch_args(
            vec![
                batch_entry("/data/first.txt", "1", None),
                batch_entry("/data/exists.txt", "2", None),
                batch_entry("/data/third.txt", "3", None),
            ],
            "create",
        );
        let error = run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Exists);
        assert!(!test.fs().exists(&abs("/data/first.txt")));
        assert_eq!(read(&test, "/data/exists.txt"), b"keep");
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn batch_items_are_checked() {
        let test = data_dir();
        let twice = batch_args(
            vec![
                batch_entry("/data/a.txt", "1", None),
                batch_entry("/data/a.txt", "2", None),
            ],
            "create",
        );
        assert_eq!(
            run(&test, &twice).unwrap_err().code(),
            ErrorCode::ParamConflict
        );

        let mut fields = Fields::new();
        fields.insert(PATH.to_owned(), path_value("/data/b.txt"));
        fields.insert("encoding".to_owned(), Value::Text("utf-8".to_owned()));
        let unknown = batch_args(vec![Value::Map(fields)], "create");
        assert_eq!(
            run(&test, &unknown).unwrap_err().code(),
            ErrorCode::UnknownField
        );

        let with_path = set(
            batch_args(vec![batch_entry("/data/c.txt", "1", None)], "create"),
            PATH,
            path_value("/data/d.txt"),
        );
        assert_eq!(
            run(&test, &with_path).unwrap_err().code(),
            ErrorCode::ParamConflict
        );

        let not_map = batch_args(vec![Value::Int(1)], "create");
        assert_eq!(run(&test, &not_map).unwrap_err().code(), ErrorCode::Type);
        assert!(test.journal().calls().is_empty());
    }

    #[test]
    fn if_match_with_an_old_etag_fails_with_stale_without_changes() {
        let (test, path) = with_file("/data/a.txt", b"first\n");
        let old = observe(test.fs().as_ref(), &path).unwrap().etag().unwrap();
        test.fs().write_file(&path, b"second\n").unwrap();
        let stale = set(
            with_text("/data/a.txt", "overwrite", "third\n"),
            "if_match",
            Value::Text(etag_text(&old)),
        );
        let error = run(&test, &stale).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"second\n");
        assert!(test.journal().calls().is_empty());

        let current = observe(test.fs().as_ref(), &path).unwrap().etag().unwrap();
        let fresh = set(
            with_text("/data/a.txt", "overwrite", "third\n"),
            "if_match",
            Value::Text(etag_text(&current)),
        );
        run(&test, &fresh).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"third\n");
    }

    #[test]
    fn a_file_changed_after_the_plan_is_not_written() {
        let (test, path) = with_file("/data/a.txt", b"planned\n");
        let ctx = test.ctx();
        let given = with_text("/data/a.txt", "overwrite", "mine\n");
        let plan = Write.plan(&given, StageInput::none(), &ctx).unwrap();
        test.clock().advance(Duration::from_secs(1));
        test.fs().write_file(&path, b"theirs\n").unwrap();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = Write.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"theirs\n");
    }

    #[test]
    fn preview_plans_changes_without_writing() {
        let (test, path) = with_file("/data/a.txt", b"old\n");
        let ctx = test.ctx();
        let given = with_text("/data/a.txt", "overwrite", "new text\n");
        let plan = Write.plan(&given, StageInput::none(), &ctx).unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].action, ChangeAction::Modify);
        assert_eq!(plan.changes[0].size_before, Some(Size::from_bytes(4)));
        assert_eq!(plan.changes[0].size_after, Some(Size::from_bytes(9)));
        assert_eq!(test.fs().read_file(&path).unwrap(), b"old\n");
        assert!(test.journal().calls().is_empty());
    }

    /// Document service that knows `.json` files and reports one syntax
    /// error when the braces are unbalanced.
    struct BraceChecker;

    impl DocService for BraceChecker {
        fn formats(&self) -> Vec<DocFormatInfo> {
            Vec::new()
        }

        fn detect_format(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
            path.filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .map(|_| DocFormatId::JSON)
        }

        fn read_keys(
            &self,
            _bytes: &[u8],
            _format: &DocFormatId,
            _key: &KeyPath,
            _depth: Option<u32>,
        ) -> Result<Vec<DocItem>> {
            Ok(Vec::new())
        }

        fn check_syntax(&self, bytes: &[u8], _format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
            let open = bytes.iter().filter(|byte| **byte == b'{').count();
            let close = bytes.iter().filter(|byte| **byte == b'}').count();
            if open == close {
                return Ok(Vec::new());
            }
            let position = TextPosition { line: 1, column: 1 };
            Ok(vec![SyntaxIssue {
                severity: IssueSeverity::Error,
                message: "unbalanced braces".to_owned(),
                span: TextSpan {
                    bytes: ByteRange::new(0, 0),
                    start: position,
                    end: position,
                },
                kind: Some("unbalanced".to_owned()),
            }])
        }

        fn serialize_records(
            &self,
            _records: &[Record],
            _format: &DocFormatId,
            _options: &SerializeOptions,
        ) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    fn checked() -> TestContext {
        let test =
            TestContext::new().with_services(Services::new().with_doc(Arc::new(BraceChecker)));
        test.fs().create_dir_all(&abs("/data")).unwrap();
        test
    }

    fn validated(path: &str, mode: &str, text: &str, validate: &str) -> BoundArgs {
        set(
            with_text(path, mode, text),
            VALIDATE,
            Value::Enum(validate.to_owned()),
        )
    }

    #[test]
    fn validate_takes_off_warn_and_reject() {
        let test = checked();
        let unknown = validated("/data/a.json", "create", "{}", "strict");
        assert_eq!(
            run(&test, &unknown).unwrap_err().code(),
            ErrorCode::UnknownValue
        );
        assert!(test.journal().calls().is_empty());
        run(&test, &validated("/data/a.json", "create", "{", "off")).unwrap();
        assert_eq!(read(&test, "/data/a.json"), b"{");
        run(&test, &validated("/data/b.json", "create", "{}", "reject")).unwrap();
        assert_eq!(read(&test, "/data/b.json"), b"{}");
    }

    #[test]
    fn a_new_file_with_a_syntax_error_is_rolled_back() {
        let test = checked();
        let ctx = test.ctx();
        let given = with_text("/data/new.json", "create", "{\"a\": 1\n");
        let plan = Write.plan(&given, StageInput::none(), &ctx).unwrap();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = Write.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
        tx.rollback(RollbackReason::ValidateFailed).unwrap();
        assert_eq!(error.code(), ErrorCode::RolledBack);
        assert_eq!(error.problem.as_deref(), Some("validate_failed"));
        assert_every_change_declared(&test);
        assert!(test.journal().calls().iter().any(|call| matches!(
            call,
            JournalCall::Rollback {
                reason: Some(RollbackReason::ValidateFailed),
                ..
            }
        )));
    }

    #[test]
    fn warn_keeps_the_write_and_reports_the_new_error() {
        let test = checked();
        test.fs().write_file(&abs("/data/a.json"), b"{}\n").unwrap();
        run(&test, &validated("/data/a.json", "append", "{\n", "warn")).unwrap();
        assert_eq!(read(&test, "/data/a.json"), b"{}\n{\n");
        let events = test.events().take();
        let warning = events
            .iter()
            .find_map(|event| match event {
                Event::Log(log) => Some(log),
                _ => None,
            })
            .unwrap();
        assert_eq!(warning.level, LogLevel::Warn);
        assert_eq!(warning.source.as_deref(), Some(COMMAND));
        assert!(warning.message.contains("1 syntax error"));
    }

    #[test]
    fn bytes_and_unknown_formats_are_not_validated() {
        let test = checked();
        let bytes = set(
            args("/data/a.json", "create"),
            DATA,
            Value::Bytes(Bytes::new(b"{".to_vec())),
        );
        run(&test, &bytes).unwrap();
        assert_eq!(read(&test, "/data/a.json"), b"{");
        run(&test, &with_text("/data/a.txt", "create", "{")).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"{");
    }

    fn registry_args(value_type: &str, text: &str) -> BoundArgs {
        let given = set(
            BoundArgs::new(COMMAND),
            PATH,
            Value::Text("registry:/HKCU/Software/App/Interval".to_owned()),
        );
        let given = set(given, MODE, Value::Enum("overwrite".to_owned()));
        let given = set(given, VALUE_TYPE, Value::Enum(value_type.to_owned()));
        set(given, TEXT, Value::Text(text.to_owned()))
    }

    #[test]
    fn registry_paths_exist_only_on_windows() {
        let linux = TestContext::with_host(TestHost::linux());
        let error = Write
            .plan(
                &registry_args("dword", "16"),
                StageInput::none(),
                &linux.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::OsOnly);

        let windows = TestContext::with_host(TestHost::windows());
        let ctx = windows
            .builder()
            .extensions(crate::registry::testing::MemRegistry::new(&[], &[]).extensions())
            .build();
        let mut plan = Write
            .plan(&registry_args("dword", "0x10"), StageInput::none(), &ctx)
            .unwrap();
        assert_eq!(
            plan.changes[0].path.as_str(),
            "registry:/HKCU/Software/App/Interval"
        );
        let planned = plan.take_state::<Planned>().unwrap();
        assert!(planned.checks.is_empty());
        assert!(matches!(
            &planned.ops[0].work,
            Work::Apply(Mutation::ProviderSet(ProviderValue::U32(16)))
        ));
        let lines = Write
            .plan(
                &registry_args("multi_string", "a\nb"),
                StageInput::none(),
                &ctx,
            )
            .unwrap();
        assert_eq!(lines.changes.len(), 1);
        let too_large = Write
            .plan(
                &registry_args("dword", "4294967296"),
                StageInput::none(),
                &ctx,
            )
            .unwrap_err();
        assert_eq!(too_large.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn value_type_applies_only_to_registry_paths() {
        let test = data_dir();
        let given = set(
            with_text("/data/a.txt", "create", "x"),
            VALUE_TYPE,
            Value::Enum("dword".to_owned()),
        );
        assert_eq!(
            run(&test, &given).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn special_files_are_appended_and_marked_irreversible() {
        let test = TestContext::new();
        let ctx = test.ctx();
        let settings = Settings::read(&BoundArgs::new(COMMAND), &ctx).unwrap();
        let mut planner = Planner::new(&ctx, settings);
        let device = abs("/dev/sink");
        planner
            .special(&device, Mode::Overwrite, b"x".to_vec())
            .unwrap();
        let error = planner
            .special(&device, Mode::Prepend, b"x".to_vec())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let plan = planner.finish();
        assert_eq!(plan.notable[0].code, Notable::SpecialFile);
        assert_eq!(plan.irreversible[0].reason, Irreversible::SpecialFile);
        assert_eq!(
            plan.changes[0].irreversible,
            Some(Irreversible::SpecialFile)
        );
    }

    #[test]
    fn applied_changes_keep_planned_text_fields() {
        let (test, _) = with_file("/data/a.txt", b"a\r\n");
        let run = run(&test, &with_text("/data/a.txt", "overwrite", "b\n")).unwrap();
        assert_eq!(
            run.planned[0].line_ending_after,
            run.changes[0].line_ending_after
        );
        assert_eq!(run.changes[0].encoding_before.as_deref(), Some("utf-8"));
        let hash = test.fs().content_hash(&abs("/data/a.txt")).unwrap();
        assert_eq!(
            run.changes[0].hash_after,
            Some(ContentHash::to_string(&hash))
        );
        assert!(run.irreversible.is_empty());
    }

    #[test]
    fn tables_quote_and_escape_cells() {
        assert_eq!(cell_text("a\"b", CSV_DELIMITER), "\"a\"\"b\"");
        assert_eq!(cell_text("line\nbreak", CSV_DELIMITER), "\"line\nbreak\"");
        assert_eq!(cell_text("plain", CSV_DELIMITER), "plain");
        assert_eq!(cell_text("a\tb\\c\n", TSV_DELIMITER), "a\\tb\\\\c\\n");
        let scalars = table(&[Value::Int(1), Value::Text("x".to_owned())], CSV_DELIMITER);
        assert_eq!(scalars, "value\n1\nx\n");
    }

    #[test]
    fn lines_use_the_text_field_or_compact_json() {
        let record = Record::new(RecordKind::Row).with("n", Value::Int(1));
        assert_eq!(line_of(&Value::Record(record)), "{\"n\":1}");
        assert_eq!(line_of(&Value::Bool(true)), "true");
        assert_eq!(line_of(&Value::Null), "");
    }

    #[test]
    fn auto_format_follows_the_extension() {
        let cases = [
            ("/data/a.JSON", Format::Json),
            ("/data/a.yml", Format::Yaml),
            ("/data/a.toml", Format::Toml),
            ("/data/a.csv", Format::Csv),
            ("/data/a.tsv", Format::Tsv),
            ("/data/a.md", Format::Text),
            ("/data/Makefile", Format::Text),
        ];
        for (path, format) in cases {
            assert_eq!(Format::Auto.for_path(&abs(path)), format, "{path}");
        }
        assert_eq!(Format::Csv.for_path(&abs("/data/a.json")), Format::Csv);
    }

    #[test]
    fn stage_is_registered_as_an_effect() {
        let stages = stages();
        assert_eq!(stages.len(), 1);
        let descriptor = stages[0].descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert!(descriptor.source);
        assert_eq!(descriptor.input, InputKinds::Any);
        assert!(stages[0].as_effect().is_some());
        assert!(stages[0].as_read().is_none());
    }

    #[test]
    fn append_keeps_the_legacy_encoding_and_line_endings_of_the_file() {
        let (test, _) = with_file("/data/legacy.txt", CP1250_CRLF);
        let given = with_text("/data/legacy.txt", "append", "\u{17c}\u{f3}\u{142}w\n");
        run(&test, &given).unwrap();
        let mut expected = CP1250_CRLF.to_vec();
        expected.extend_from_slice(b"\xbf\xf3\xb3w\r\n");
        assert_eq!(read(&test, "/data/legacy.txt"), expected);
    }

    #[test]
    fn prepend_keeps_the_byte_order_mark_in_front() {
        let (test, _) = with_file("/data/a.txt", b"\xef\xbb\xbfbody\n");
        run(&test, &with_text("/data/a.txt", "prepend", "head\n")).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"\xef\xbb\xbfhead\nbody\n");
    }

    #[test]
    fn append_with_ensure_newline_ends_the_added_text_with_the_line_ending() {
        let (test, _) = with_file("/data/a.txt", b"a\r\n");
        let given = set(
            with_text("/data/a.txt", "append", "b"),
            ENSURE_NEWLINE,
            Value::Bool(true),
        );
        run(&test, &given).unwrap();
        assert_eq!(read(&test, "/data/a.txt"), b"a\r\nb\r\n");
    }

    #[test]
    fn unknown_modes_and_negative_sizes_are_rejected_before_any_change() {
        let (test, _) = with_file("/data/a.bin", b"abc");
        let unknown = with_text("/data/a.bin", "upsert", "x");
        assert_eq!(
            run(&test, &unknown).unwrap_err().code(),
            ErrorCode::UnknownValue
        );
        let negative = set(args("/data/a.bin", "overwrite"), SIZE, Value::Int(-1));
        assert_eq!(
            run(&test, &negative).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        assert_eq!(read(&test, "/data/a.bin"), b"abc");
        assert!(test.journal().calls().is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
