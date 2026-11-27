// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `convert`: explicit change of the encoding, the line endings or the byte
//! order mark of text files.
//!
//! The plan step reads every file, detects its encoding and line endings and
//! encodes the converted text, so a character the target encoding cannot
//! represent fails with `E_ENCODING`, with its line and column, before
//! anything changes. A file already in the requested form is reported as
//! `unchanged` and not written. The apply step reads each file again, checks
//! that it still has the entity tag seen by the plan and writes the new
//! content through the transaction, which repeats the check atomically.
//!
//! Without `bom`, a file that keeps its encoding keeps its byte order mark;
//! a file converted to UTF-16 receives one and a file converted to any other
//! encoding is written without one.

use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Etag, FileKind, Follow};
use nurt_contracts::journal::{Expect, FileOp, MutationTx};
use nurt_contracts::record::{Change, Effect, EffectStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::TextEncoding;
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Size};
use nurt_text::codec::{Codec, CodecError, encode};
use nurt_text::codec_service::line_ending;

use crate::common::etag::stale_error;
use crate::common::mutation::{apply_changes, planned_change};
use crate::common::notable::{at_path, counted};
use crate::common::params::{EncodingParam, LineEndingParam, encoding_param, line_ending_param};
use crate::common::path::{path_list_arg_for_change, record_path};
use crate::common::text::{TextFile, read_text, record_line_ending};

/// Name of the command in the catalog.
const COMMAND: &str = "convert";

/// Name of the `path` parameter.
const PATH: &str = "path";

/// Name of the `bom` parameter.
const BOM: &str = "bom";

/// Separator of the values before and after in the `detail` of notable
/// entries.
const DETAIL_ARROW: &str = " -> ";

/// `detail` of [`Notable::BomChanged`] for an added byte order mark.
const BOM_ADDED: &str = "added";

/// `detail` of [`Notable::BomChanged`] for a removed byte order mark.
const BOM_REMOVED: &str = "removed";

/// Stage of `convert`: source form with `path`, stage form on `entry`
/// records.
pub(crate) struct Convert;

impl StageImpl for Convert {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Convert {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let settings = Settings::from_args(args)?;
        let sources = sources(args, input, ctx)?;
        let mut plan = EffectPlan::new();
        if sources.is_empty() {
            plan.notable
                .push(NotableEntry::new(Notable::NothingMatched));
        }
        let mut files = Vec::new();
        let mut binary_skipped = 0_u64;
        for source in sources {
            ctx.checkpoint()?;
            let file = match read_source(ctx, &source)? {
                Source::Text(file) => *file,
                Source::Binary => {
                    binary_skipped += 1;
                    continue;
                }
                Source::NotFile => continue,
            };
            let Some(converted) = settings.convert(&file)? else {
                plan.notable.push(at_path(Notable::Unchanged, &file.path));
                continue;
            };
            let change = file_change(&file, &converted);
            plan.notable.extend(change_notables(&change, &file.path));
            plan.changes.push(change.clone());
            files.push(PlannedFile {
                path: file.path.clone(),
                etag: file.etag(),
                size: file.size,
                change,
            });
        }
        if binary_skipped > 0 {
            plan.notable
                .push(counted(Notable::BinarySkipped, binary_skipped));
        }
        plan.bulk = files.len() > 1;
        Ok(plan.with_state(PlanState { settings, files }))
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        mut plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let state = plan.take_state::<PlanState>().ok_or_else(|| {
            Error::new(
                ErrorCode::Internal,
                format!("the plan of `{COMMAND}` has no state: plan the conversion again"),
            )
        })?;
        let mut ops = Vec::with_capacity(state.files.len());
        for planned in &state.files {
            ctx.checkpoint()?;
            ops.push(state.settings.reconvert(ctx, planned)?);
        }
        let changes: Vec<Change> = if ops.is_empty() {
            Vec::new()
        } else {
            apply_changes(tx, ops)?
                .into_iter()
                .zip(&state.files)
                .map(|(applied, planned)| merged_change(applied, &planned.change))
                .collect()
        };
        let effect = Effect {
            status: EffectStatus::Ok,
            kind: COMMAND.to_owned(),
            op: tx.op().map(|op| op.0),
            targets: Some(u64::try_from(changes.len()).unwrap_or(u64::MAX)),
            changes: Some(changes.clone()),
            notable: (!plan.notable.is_empty()).then(|| {
                plan.notable
                    .iter()
                    .map(|entry| entry.code.as_str().to_owned())
                    .collect()
            }),
            ..Effect::default()
        };
        let output = StageOutput::records(RecordStream::once(Record::from(effect)));
        Ok(EffectOutcome {
            changes,
            ..EffectOutcome::new(output)
        })
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Convert)]
}

/// Requested form of the converted files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Settings {
    /// Target encoding; `None` keeps the encoding of each file.
    encoding: Option<Codec>,
    /// Target line endings.
    line_ending: LineEndingParam,
    /// Byte order mark; `None` applies the rule of the module.
    bom: Option<bool>,
}

impl Settings {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        let encoding = match encoding_param(args)? {
            Some(EncodingParam::Codec(codec)) => Some(codec),
            Some(EncodingParam::Auto) | None => None,
        };
        let settings = Self {
            encoding,
            line_ending: line_ending_param(args)?,
            bom: args.bool(BOM)?,
        };
        if settings.encoding.is_none()
            && settings.line_ending == LineEndingParam::Keep
            && settings.bom.is_none()
        {
            return Err(Error::new(
                ErrorCode::MissingParam,
                format!(
                    "`{COMMAND}` needs `encoding`, `line_ending` or `{BOM}`: give the form to convert to"
                ),
            ));
        }
        if let (Some(codec), Some(true)) = (settings.encoding, settings.bom)
            && codec.bom().is_none()
        {
            return Err(CodecError::BomNotSupported { codec }.into_error(None));
        }
        Ok(settings)
    }

    /// Encoding and byte order mark of the converted file.
    fn target(&self, file: &TextFile) -> Result<TextEncoding> {
        let source = Codec::from_encoding(&file.encoding)
            .map_err(|error| error.into_error(Some(&file.path.to_language_string())))?;
        let codec = self.encoding.unwrap_or(source);
        let bom = match self.bom {
            Some(bom) => bom,
            None if codec == source => file.encoding.bom,
            None => matches!(codec, Codec::Utf16Le | Codec::Utf16Be),
        };
        if bom && codec.bom().is_none() {
            return Err(CodecError::BomNotSupported { codec }
                .into_error(Some(&file.path.to_language_string())));
        }
        Ok(codec.text_encoding(bom))
    }

    /// Converted content of a file; `None` when the file already has the
    /// requested form.
    ///
    /// Fails with `E_ENCODING` at the first character the target encoding
    /// cannot represent.
    fn convert(&self, file: &TextFile) -> Result<Option<Converted>> {
        let encoding = self.target(file)?;
        let text = self.line_ending.apply(&file.text);
        if encoding == file.encoding && text == file.text {
            return Ok(None);
        }
        let bytes = encode(&text, &encoding)
            .map_err(|error| error.into_error(Some(&file.path.to_language_string())))?;
        Ok(Some(Converted { bytes, encoding }))
    }

    /// Reads a planned file again and builds its write; fails with
    /// `E_STALE` when the file no longer has the tag seen by the plan.
    fn reconvert(&self, ctx: &ExecCtx, planned: &PlannedFile) -> Result<FileOp> {
        let file = read_text(ctx, &planned.path, EncodingParam::Auto)?;
        let current = file.etag();
        if !current.matches(&planned.etag) {
            return Err(stale_error(&planned.path, &planned.etag, Some(&current)));
        }
        let Some(converted) = self.convert(&file)? else {
            return Err(stale_error(&planned.path, &planned.etag, Some(&current)));
        };
        Ok(FileOp::write(planned.path.clone(), converted.bytes)
            .expect(Expect::Etag(planned.etag.clone()))
            .size_hint(planned.size))
    }
}

/// Converted content of one file.
struct Converted {
    bytes: Vec<u8>,
    encoding: TextEncoding,
}

/// File planned for conversion; the content is produced again in the apply
/// step.
struct PlannedFile {
    path: AbsPath,
    etag: Etag,
    size: u64,
    change: Change,
}

/// Private state of the plan.
struct PlanState {
    settings: Settings,
    files: Vec<PlannedFile>,
}

/// File named by the program or by an input record.
struct Target {
    path: AbsPath,
    from_record: bool,
}

/// Paths of the source form or of the input records, each path once, in
/// order.
fn sources(args: &BoundArgs, input: StageInput<'_>, ctx: &ExecCtx) -> Result<Vec<Target>> {
    let mut targets: Vec<Target> = Vec::new();
    let mut push = |path: AbsPath, from_record: bool| {
        if !targets.iter().any(|target| target.path == path) {
            targets.push(Target { path, from_record });
        }
    };
    if input.is_none() {
        let paths = path_list_arg_for_change(args, PATH, ctx)?;
        if paths.is_empty() {
            return Err(Error::new(
                ErrorCode::MissingParam,
                format!(
                    "`{COMMAND}` needs a path: give the files to convert or pass entry records"
                ),
            ));
        }
        for path in paths {
            push(path, false);
        }
        return Ok(targets);
    }
    if args.is_given(PATH) {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            format!(
                "`{PATH}` cannot be given when `{COMMAND}` receives records: remove `{PATH}` or start the pipeline with `{COMMAND}`"
            ),
        ));
    }
    for record in input
        .into_records()?
        .collect_bounded(ctx.limits().max_records)?
    {
        push(record_path(&record, ctx)?, true);
    }
    Ok(targets)
}

/// Outcome of reading one target.
enum Source {
    /// A text file to convert.
    Text(Box<TextFile>),
    /// A file of an input record with binary content, skipped.
    Binary,
    /// An entry of an input record that is not a regular file, skipped.
    NotFile,
}

/// Reads a target as text. A target named in the source form must be a text
/// file; entries of input records that are not regular files or have binary
/// content are skipped.
fn read_source(ctx: &ExecCtx, target: &Target) -> Result<Source> {
    let kind = ctx
        .require_fs()?
        .metadata(&target.path, Follow::Follow)?
        .kind;
    if kind != FileKind::File {
        if target.from_record {
            return Ok(Source::NotFile);
        }
        return Err(not_a_file_error(&target.path, kind));
    }
    match read_text(ctx, &target.path, EncodingParam::Auto) {
        Ok(file) => Ok(Source::Text(Box::new(file))),
        Err(error) if target.from_record && error.code() == ErrorCode::Binary => Ok(Source::Binary),
        Err(error) => Err(error),
    }
}

/// Planned `change` record of a converted file.
fn file_change(file: &TextFile, converted: &Converted) -> Change {
    let written = u64::try_from(converted.bytes.len()).unwrap_or(u64::MAX);
    Change {
        size_after: Some(Size::from_bytes(written)),
        encoding_before: Some(file.encoding.label.clone()),
        encoding_after: Some(converted.encoding.label.clone()),
        line_ending_before: Some(record_line_ending(file.line_ending)),
        line_ending_after: Some(record_line_ending(line_ending(
            &converted.bytes,
            &converted.encoding,
        ))),
        bom_before: Some(file.encoding.bom),
        bom_after: Some(converted.encoding.bom),
        ..planned_change(&file.target())
    }
}

/// Applied `change` record with the text details of the plan.
fn merged_change(applied: Change, planned: &Change) -> Change {
    Change {
        encoding_before: planned.encoding_before.clone(),
        encoding_after: planned.encoding_after.clone(),
        line_ending_before: planned.line_ending_before,
        line_ending_after: planned.line_ending_after,
        bom_before: planned.bom_before,
        bom_after: planned.bom_after,
        ..applied
    }
}

/// `encoding_changed`, `line_ending_changed` and `bom_changed` entries of a
/// planned change.
fn change_notables(change: &Change, path: &AbsPath) -> Vec<NotableEntry> {
    let mut entries = Vec::new();
    if let (Some(before), Some(after)) = (&change.encoding_before, &change.encoding_after)
        && before != after
    {
        entries.push(NotableEntry {
            detail: Some(format!("{before}{DETAIL_ARROW}{after}")),
            ..at_path(Notable::EncodingChanged, path)
        });
    }
    if let (Some(before), Some(after)) = (change.line_ending_before, change.line_ending_after)
        && before != after
    {
        entries.push(NotableEntry {
            detail: Some(format!("{before}{DETAIL_ARROW}{after}")),
            ..at_path(Notable::LineEndingChanged, path)
        });
    }
    if let (Some(before), Some(after)) = (change.bom_before, change.bom_after)
        && before != after
    {
        let detail = if after { BOM_ADDED } else { BOM_REMOVED };
        entries.push(NotableEntry {
            detail: Some(detail.to_owned()),
            ..at_path(Notable::BomChanged, path)
        });
    }
    entries
}

fn not_a_file_error(path: &AbsPath, kind: FileKind) -> Error {
    let message = if kind == FileKind::Directory {
        format!("`{path}` is a directory: convert its files with `find` and `|> {COMMAND}`")
    } else {
        format!("`{path}` is not a regular file: give a text file")
    };
    Error::new(ErrorCode::InvalidValue, message)
        .with_problem("not_a_file")
        .with_found(path.to_language_string())
        .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::Config;
    use nurt_contracts::journal::{JournalStore, TargetIndex, TargetPath};
    use nurt_contracts::record::{ChangeAction, LineEnding};
    use nurt_contracts::stage::{ArgValue, StageData};
    use nurt_contracts::value::Value;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::{begin_request, racy_time_ns};
    use nurt_testkit::journal::JournalCall;
    use nurt_testkit::vfs::test_path;

    use super::*;

    /// Windows-1250 text with Polish letters and CRLF line endings.
    const CP1250_CRLF: &[u8] = b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\r\nkoniec\r\n";

    /// The same text in UTF-8 with LF line endings.
    const UTF8_LF: &[u8] =
        b"Zielona \xc5\x82\xc4\x85ka i \xc5\xbc\xc3\xb3\xc5\x82ty \xc5\x9bwierszcz\nkoniec\n";

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_files(files: &[(&str, &[u8])]) -> TestContext {
        let test = TestContext::new();
        for (path, content) in files {
            test.fs().write_file(&abs(path), content).unwrap();
        }
        test
    }

    /// Arguments: paths, then `(name, value)` pairs.
    fn args(paths: &[&str], params: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        if !paths.is_empty() {
            let list = paths
                .iter()
                .map(|path| Value::Path(abs(path).to_language_string()))
                .collect();
            args.insert(PATH, ArgValue::Value(Value::List(list)), false);
        }
        for (name, value) in params {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn choice(value: &str) -> Value {
        Value::Enum(value.to_owned())
    }

    fn entries(paths: &[&str]) -> StageInput<'static> {
        let records = paths
            .iter()
            .map(|path| {
                Record::new(RecordKind::Entry)
                    .with("path", Value::Path(abs(path).to_language_string()))
            })
            .collect();
        StageInput::records(RecordStream::from_records(records))
    }

    #[derive(Debug)]
    struct Run {
        planned: Vec<Change>,
        notable: Vec<NotableEntry>,
        changes: Vec<Change>,
        effect: Effect,
    }

    fn execute(
        ctx: &ExecCtx,
        test: &TestContext,
        args: &BoundArgs,
        input: StageInput<'_>,
    ) -> Result<Run> {
        let plan = Convert.plan(args, input, ctx)?;
        let planned = plan.changes.clone();
        let notable = plan.notable.clone();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = Convert.apply(args, plan, tx.as_mut(), ctx)?;
        let StageData::Records(stream) = outcome.output.data else {
            panic!("expected an effect record");
        };
        let records: Vec<Record> = stream.collect::<Result<_>>()?;
        tx.commit()?;
        Ok(Run {
            planned,
            notable,
            changes: outcome.changes,
            effect: Effect::try_from(records[0].clone()).unwrap(),
        })
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Run> {
        execute(&test.ctx(), test, args, StageInput::none())
    }

    fn codes(entries: &[NotableEntry]) -> Vec<Notable> {
        entries.iter().map(|entry| entry.code).collect()
    }

    fn declared(test: &TestContext) -> Vec<TargetPath> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Preserve { targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn applied_ok(test: &TestContext) -> Vec<TargetIndex> {
        test.journal()
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                JournalCall::Apply {
                    index, ok: true, ..
                } => Some(index),
                _ => None,
            })
            .collect()
    }

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut bytes = if bom { vec![0xff, 0xfe] } else { Vec::new() };
        bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        bytes
    }

    #[test]
    fn descriptor_registers_an_effect_stage() {
        let descriptor = Convert.descriptor();
        assert_eq!(descriptor.command, "convert");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Entry));
        assert!(Convert.as_effect().is_some());
        assert!(Convert.as_read().is_none());
        assert_eq!(stages().len(), 1);
    }

    #[test]
    fn legacy_crlf_file_becomes_utf8_with_lf() {
        let test = with_files(&[("/data/legacy.txt", CP1250_CRLF)]);
        let given = args(
            &["/data/legacy.txt"],
            &[("encoding", text("utf-8")), ("line_ending", choice("lf"))],
        );
        let run = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/legacy.txt")).unwrap(),
            UTF8_LF
        );
        assert_eq!(
            codes(&run.notable),
            [Notable::EncodingChanged, Notable::LineEndingChanged]
        );
        assert_eq!(
            run.notable[0].detail.as_deref(),
            Some("windows-1250 -> utf-8")
        );
        assert_eq!(run.notable[1].detail.as_deref(), Some("crlf -> lf"));
    }

    #[test]
    fn converted_file_reports_its_change_and_effect() {
        let test = with_files(&[("/data/legacy.txt", CP1250_CRLF)]);
        let given = args(
            &["/data/legacy.txt"],
            &[("encoding", text("utf-8")), ("line_ending", choice("lf"))],
        );
        let run = run(&test, &given).unwrap();
        let change = &run.changes[0];
        assert_eq!(change.action, ChangeAction::Modify);
        assert_eq!(change.encoding_before.as_deref(), Some("windows-1250"));
        assert_eq!(change.encoding_after.as_deref(), Some("utf-8"));
        assert_eq!(change.line_ending_before, Some(LineEnding::Crlf));
        assert_eq!(change.line_ending_after, Some(LineEnding::Lf));
        assert_eq!(change.bom_after, Some(false));
        assert_eq!(change.size_before, Some(Size::from_bytes(40)));
        assert_eq!(change.size_after, Some(Size::from_bytes(44)));
        assert_eq!(run.planned[0].size_after, change.size_after);
        assert_eq!(run.effect.kind, "convert");
        assert_eq!(run.effect.status, EffectStatus::Ok);
        assert_eq!(run.effect.targets, Some(1));
        assert_eq!(
            run.effect.notable,
            Some(vec![
                "encoding_changed".to_owned(),
                "line_ending_changed".to_owned()
            ])
        );
    }

    #[test]
    fn line_endings_alone_keep_the_encoding() {
        let test = with_files(&[("/data/legacy.txt", CP1250_CRLF)]);
        let given = args(&["/data/legacy.txt"], &[("line_ending", choice("lf"))]);
        let run = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/legacy.txt")).unwrap(),
            b"Zielona \xb3\xb9ka i \xbf\xf3\xb3ty \x9cwierszcz\nkoniec\n"
        );
        assert_eq!(codes(&run.notable), [Notable::LineEndingChanged]);
        assert_eq!(
            run.changes[0].encoding_after.as_deref(),
            Some("windows-1250")
        );
    }

    #[test]
    fn conversion_to_utf16_adds_a_byte_order_mark() {
        let test = with_files(&[("/data/a.txt", b"a\nb")]);
        let given = args(&["/data/a.txt"], &[("encoding", text("utf-16le"))]);
        let run = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt")).unwrap(),
            utf16le("a\nb", true)
        );
        assert_eq!(
            codes(&run.notable),
            [Notable::EncodingChanged, Notable::BomChanged]
        );
        assert_eq!(run.notable[1].detail.as_deref(), Some("added"));
    }

    #[test]
    fn utf16_without_a_byte_order_mark_on_request() {
        let test = with_files(&[("/data/a.txt", b"a\r\n")]);
        let given = args(
            &["/data/a.txt"],
            &[("encoding", text("utf-16le")), (BOM, Value::Bool(false))],
        );
        run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt")).unwrap(),
            utf16le("a\r\n", false)
        );
    }

    #[test]
    fn byte_order_mark_is_removed_and_kept_otherwise() {
        let test = with_files(&[
            ("/data/marked.csv", b"\xef\xbb\xbfa;b\r\n"),
            ("/data/kept.csv", b"\xef\xbb\xbfa;b\r\n"),
        ]);
        let removed = run(
            &test,
            &args(&["/data/marked.csv"], &[(BOM, Value::Bool(false))]),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/marked.csv")).unwrap(),
            b"a;b\r\n"
        );
        assert_eq!(codes(&removed.notable), [Notable::BomChanged]);
        assert_eq!(removed.notable[0].detail.as_deref(), Some("removed"));

        run(
            &test,
            &args(&["/data/kept.csv"], &[("line_ending", choice("lf"))]),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/kept.csv")).unwrap(),
            b"\xef\xbb\xbfa;b\n"
        );
    }

    #[test]
    fn utf16_file_converts_to_a_legacy_encoding_without_a_mark() {
        let text = "\u{17c}\u{f3}\u{142}w\r\n";
        let test = with_files(&[("/data/wide.txt", &utf16le(text, true))]);
        let given = args(&["/data/wide.txt"], &[("encoding", text_value("cp1250"))]);
        let run = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/wide.txt")).unwrap(),
            b"\xbf\xf3\xb3w\r\n"
        );
        assert_eq!(run.changes[0].bom_before, Some(true));
        assert_eq!(run.changes[0].bom_after, Some(false));
    }

    fn text_value(value: &str) -> Value {
        text(value)
    }

    #[test]
    fn mixed_line_endings_are_unified() {
        let test = with_files(&[("/data/mixed.txt", b"a\r\nb\nc\rd")]);
        let given = args(&["/data/mixed.txt"], &[("line_ending", choice("crlf"))]);
        let run = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/mixed.txt")).unwrap(),
            b"a\r\nb\r\nc\r\nd"
        );
        assert_eq!(run.changes[0].line_ending_before, Some(LineEnding::Mixed));
        assert_eq!(run.changes[0].line_ending_after, Some(LineEnding::Crlf));
        assert_eq!(run.notable[0].detail.as_deref(), Some("mixed -> crlf"));
    }

    #[test]
    fn missing_final_line_ending_stays_missing() {
        let test = with_files(&[("/data/a.txt", b"one\ntwo")]);
        run(
            &test,
            &args(&["/data/a.txt"], &[("line_ending", choice("cr"))]),
        )
        .unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt")).unwrap(),
            b"one\rtwo"
        );
    }

    #[test]
    fn files_in_the_requested_form_are_not_written() {
        let test = with_files(&[("/data/a.txt", b"a\nb\n"), ("/data/empty.txt", b"")]);
        let given = args(
            &["/data/a.txt", "/data/empty.txt"],
            &[("encoding", text("utf-8")), ("line_ending", choice("lf"))],
        );
        let run = run(&test, &given).unwrap();
        assert!(run.changes.is_empty());
        assert!(run.planned.is_empty());
        assert_eq!(
            codes(&run.notable),
            [Notable::Unchanged, Notable::Unchanged]
        );
        assert_eq!(run.effect.targets, Some(0));
        assert!(declared(&test).is_empty());
        assert!(applied_ok(&test).is_empty());
    }

    #[test]
    fn every_mutation_goes_through_the_transaction() {
        let test = with_files(&[
            ("/data/a.txt", b"a\r\n"),
            ("/data/b.txt", b"b\r\n"),
            ("/data/c.txt", b"c\n"),
        ]);
        let given = args(
            &["/data/a.txt", "/data/b.txt", "/data/c.txt"],
            &[("line_ending", choice("lf"))],
        );
        let ctx = test.ctx();
        let plan = Convert.plan(&given, StageInput::none(), &ctx).unwrap();
        assert!(plan.bulk);
        assert_eq!(plan.changes.len(), 2);
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = Convert.apply(&given, plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        let targets = declared(&test);
        assert_eq!(
            targets,
            [
                TargetPath::Fs(abs("/data/a.txt")),
                TargetPath::Fs(abs("/data/b.txt"))
            ]
        );
        assert_eq!(applied_ok(&test).len(), targets.len());
        assert_eq!(outcome.changes.len(), targets.len());
        assert_eq!(outcome.changes[0].store.as_deref(), Some("none"));
        assert_eq!(test.fs().read_file(&abs("/data/c.txt")).unwrap(), b"c\n");
    }

    #[test]
    fn a_same_size_rewrite_that_kept_the_modification_time_fails_with_stale() {
        let test = with_files(&[("/data/a.txt", b"a\r\n")]);
        let path = abs("/data/a.txt");
        let racy = racy_time_ns();
        test.fs().set_modified(&path, racy).unwrap();
        let given = args(&["/data/a.txt"], &[("line_ending", choice("lf"))]);
        let ctx = test.ctx();
        let plan = Convert.plan(&given, StageInput::none(), &ctx).unwrap();
        test.fs().write_file(&path, b"b\r\n").unwrap();
        test.fs().set_modified(&path, racy).unwrap();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = Convert.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"b\r\n");
    }

    #[test]
    fn a_file_changed_after_the_plan_fails_with_stale() {
        let test = with_files(&[("/data/a.txt", b"a\r\n")]);
        let given = args(&["/data/a.txt"], &[("line_ending", choice("lf"))]);
        let ctx = test.ctx();
        let plan = Convert.plan(&given, StageInput::none(), &ctx).unwrap();
        test.fs()
            .write_file(&abs("/data/a.txt"), b"changed\r\n")
            .unwrap();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = Convert.apply(&given, plan, tx.as_mut(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt")).unwrap(),
            b"changed\r\n"
        );
        assert!(applied_ok(&test).is_empty());
    }

    #[test]
    fn unrepresentable_characters_fail_before_any_change() {
        let test = with_files(&[
            ("/data/ok.txt", b"plain\n"),
            ("/data/arrows.txt", "a\nb \u{2192} c\n".as_bytes()),
        ]);
        let given = args(
            &["/data/ok.txt", "/data/arrows.txt"],
            &[("encoding", text("windows-1250"))],
        );
        let error = run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
        assert_eq!(error.problem.as_deref(), Some("unrepresentable_character"));
        let location = error.location.clone().unwrap();
        assert_eq!(location.path, abs("/data/arrows.txt").to_language_string());
        assert_eq!(location.line, Some(2));
        assert_eq!(location.column, Some(3));
        assert_eq!(error.fix.len(), 1);
        assert!(test.journal().calls().is_empty());
        assert_eq!(
            test.fs().read_file(&abs("/data/ok.txt")).unwrap(),
            b"plain\n"
        );
    }

    #[test]
    fn byte_order_mark_for_a_single_byte_encoding_is_rejected() {
        let test = with_files(&[("/data/a.txt", b"a\n")]);
        let given = args(
            &["/data/a.txt"],
            &[("encoding", text("windows-1250")), (BOM, Value::Bool(true))],
        );
        let error = Convert
            .plan(&given, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);

        let legacy = with_files(&[("/data/legacy.txt", CP1250_CRLF)]);
        let only_bom = args(&["/data/legacy.txt"], &[(BOM, Value::Bool(true))]);
        let error = Convert
            .plan(&only_bom, StageInput::none(), &legacy.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("bom_not_supported"));
    }

    #[test]
    fn a_conversion_needs_a_target_form() {
        let test = with_files(&[("/data/a.txt", b"a\n")]);
        let given = args(&["/data/a.txt"], &[("line_ending", choice("keep"))]);
        let error = Convert
            .plan(&given, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn unknown_encodings_and_values_are_rejected() {
        let test = with_files(&[("/data/a.txt", b"a\n")]);
        let unknown = args(&["/data/a.txt"], &[("encoding", text("koi8-r"))]);
        let error = Convert
            .plan(&unknown, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        let ending = args(&["/data/a.txt"], &[("line_ending", choice("dos"))]);
        let error = Convert
            .plan(&ending, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
    }

    #[test]
    fn source_form_needs_a_path_and_rejects_directories() {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/data/dir")).unwrap();
        let no_path = args(&[], &[("line_ending", choice("lf"))]);
        let error = Convert
            .plan(&no_path, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        let directory = args(&["/data/dir"], &[("line_ending", choice("lf"))]);
        let error = Convert
            .plan(&directory, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("not_a_file"));
    }

    #[test]
    fn source_form_rejects_binary_files() {
        let test = with_files(&[("/data/image.bin", b"\x7fELF\x00\x01\x02\x00")]);
        let given = args(&["/data/image.bin"], &[("line_ending", choice("lf"))]);
        let error = Convert
            .plan(&given, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Binary);
    }

    #[test]
    fn stage_form_skips_directories_and_binary_files() {
        let test = with_files(&[
            ("/data/dir/a.txt", b"a\r\n"),
            ("/data/dir/image.bin", b"\x7fELF\x00\x01\x02\x00"),
        ]);
        let given = args(&[], &[("line_ending", choice("lf"))]);
        let input = entries(&["/data/dir", "/data/dir/a.txt", "/data/dir/image.bin"]);
        let run = execute(&test.ctx(), &test, &given, input).unwrap();
        assert_eq!(run.changes.len(), 1);
        assert_eq!(
            test.fs().read_file(&abs("/data/dir/a.txt")).unwrap(),
            b"a\n"
        );
        let skipped = run
            .notable
            .iter()
            .find(|entry| entry.code == Notable::BinarySkipped)
            .unwrap();
        assert_eq!(skipped.count, Some(1));
    }

    #[test]
    fn stage_form_rejects_a_path_and_reports_empty_input() {
        let test = with_files(&[("/data/a.txt", b"a\r\n")]);
        let with_path = args(&["/data/a.txt"], &[("line_ending", choice("lf"))]);
        let error = Convert
            .plan(&with_path, entries(&["/data/a.txt"]), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);

        let given = args(&[], &[("line_ending", choice("lf"))]);
        let plan = Convert.plan(&given, entries(&[]), &test.ctx()).unwrap();
        assert_eq!(codes(&plan.notable), [Notable::NothingMatched]);
        assert!(plan.changes.is_empty());
    }

    #[test]
    fn repeated_paths_are_converted_once() {
        let test = with_files(&[("/data/a.txt", b"a\r\n")]);
        let given = args(
            &["/data/a.txt", "/data/a.txt"],
            &[("line_ending", choice("lf"))],
        );
        let run = run(&test, &given).unwrap();
        assert_eq!(run.changes.len(), 1);
        assert_eq!(declared(&test).len(), 1);
    }

    #[test]
    fn dos_code_page_text_becomes_utf8() {
        let text = "\u{179}r\u{f3}d\u{142}o: \u{17c}\u{f3}\u{142}w, g\u{119}\u{15b} i \u{142}\u{105}ka.\r\n";
        let bytes = encode(text, &Codec::Ibm852.text_encoding(false)).unwrap();
        let test = with_files(&[("/data/dir.txt", &bytes)]);
        let mut config = Config::default();
        config.text.legacy_encodings = vec!["ibm852".to_owned()];
        let ctx = test.builder().config(Arc::new(config)).build();
        let given = args(&["/data/dir.txt"], &[("encoding", text_value("utf-8"))]);
        let run = execute(&ctx, &test, &given, StageInput::none()).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/dir.txt")).unwrap(),
            text.as_bytes()
        );
        assert_eq!(run.changes[0].encoding_before.as_deref(), Some("ibm852"));
        assert_eq!(run.changes[0].line_ending_after, Some(LineEnding::Crlf));
    }

    #[test]
    fn apply_without_the_state_of_the_plan_fails() {
        let test = TestContext::new();
        let given = args(&["/data/a.txt"], &[("line_ending", choice("lf"))]);
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let error = Convert
            .apply(&given, EffectPlan::new(), tx.as_mut(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
    }

    #[test]
    fn utf16_big_endian_text_with_a_mark_becomes_utf8_without_one() {
        let mut wide = vec![0xfe, 0xff];
        wide.extend("a\u{142}\n".encode_utf16().flat_map(u16::to_be_bytes));
        let test = with_files(&[("/data/wide.txt", &wide)]);
        let given = args(&["/data/wide.txt"], &[("encoding", text("utf-8"))]);
        let run = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&abs("/data/wide.txt")).unwrap(),
            "a\u{142}\n".as_bytes()
        );
        assert_eq!(run.changes[0].bom_before, Some(true));
        assert_eq!(run.changes[0].bom_after, Some(false));
        assert_eq!(run.changes[0].encoding_before.as_deref(), Some("utf-16be"));
    }

    #[test]
    fn the_same_encoding_keeps_the_byte_order_mark_and_changes_nothing() {
        let test = with_files(&[("/data/a.txt", b"\xef\xbb\xbfa\n")]);
        let given = args(&["/data/a.txt"], &[("encoding", text("utf-8"))]);
        let run = run(&test, &given).unwrap();
        assert!(run.changes.is_empty());
        assert_eq!(codes(&run.notable), [Notable::Unchanged]);
        assert_eq!(
            test.fs().read_file(&abs("/data/a.txt")).unwrap(),
            b"\xef\xbb\xbfa\n"
        );
    }
}
