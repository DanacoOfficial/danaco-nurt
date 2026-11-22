// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `edit` command, and the planning and writing of text changes shared
//! with `replace` and `insert`.
//!
//! `edit` applies a list of changes to one file, all or none. Every change is
//! located in the file as it was before the edit: line numbers and matches of
//! a later change do not move because of the changes listed before it. Two
//! changes whose ranges overlap are rejected before anything is written.
//!
//! A text command plans one [`FileEdit`] per file: the file as read and its
//! new text. [`effect_plan`] turns the edits into the planned `change`
//! records, with the difference, the counts of lines and replacements and the
//! notable effects, and [`apply_edits`] writes them through the transaction in
//! the encoding, byte order mark and line endings of each file, then checks
//! the result as `validate` requires.

use std::num::NonZeroUsize;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Mutation, MutationTx, TargetSpec};
use nurt_contracts::record::{self, Change, Effect, EffectStatus};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{self, Fields, Range, Record, Value};
use nurt_text::diff::unified_diff;
use nurt_text::edit_plan::{
    self, EditError, EditOptions, InsertChange, Pattern, Position, ReplaceChange,
};
use nurt_text::fuzzy_match::{Excerpt, MatchError, OnMissing, Whitespace};
use nurt_text::search::DEFAULT_BACKTRACK_LIMIT;
use nurt_text::template::TemplateError;
use serde_json::Value as Json;

use super::validate::{self, Validate};
use crate::common::etag::{Observed, check_if_match, parse_etag, stale_error};
use crate::common::mutation::applied_change;
use crate::common::notable::{at_path, preserved_text};
use crate::common::params::{self, EncodingParam, IfMatch};
use crate::common::path::{path_arg, path_field};
use crate::common::text::{TextFile, encode_preserving, read_text, text_change};

const COMMAND: &str = "edit";
const PATH: &str = "path";
const CHANGES: &str = "changes";

/// Name of the `whitespace` parameter and change key.
pub(crate) const WHITESPACE: &str = "whitespace";
/// Name of the `expect` parameter and change key.
pub(crate) const EXPECT: &str = "expect";
/// Name of the `occurrence` parameter and change key.
pub(crate) const OCCURRENCE: &str = "occurrence";
/// Name of the `whole_word` parameter and change key.
pub(crate) const WHOLE_WORD: &str = "whole_word";
/// Name of the `position` parameter and change key.
pub(crate) const POSITION: &str = "position";
/// Name of the `line` parameter and change key.
pub(crate) const LINE: &str = "line";
/// Name of the `with` parameter and change key.
pub(crate) const WITH: &str = "with";

const REPLACE: &str = "replace";
const INSERT: &str = "insert";
const DELETE_LINES: &str = "delete_lines";
const REPLACE_KEYS: [&str; 7] = [
    REPLACE,
    WITH,
    OCCURRENCE,
    WHOLE_WORD,
    WHITESPACE,
    params::ON_MISSING,
    EXPECT,
];
const INSERT_KEYS: [&str; 3] = [INSERT, LINE, POSITION];
const DELETE_KEYS: [&str; 1] = [DELETE_LINES];

const WHITESPACE_VALUES: [&str; 4] = ["exact", "trailing", "indent", "normalize"];
const POSITION_VALUES: [&str; 4] = ["before", "after", "start", "end"];
const ON_MISSING_VALUES: [&str; 2] = ["skip", "error"];

/// Lines of context around every hunk of the difference of a change.
const DIFF_CONTEXT: usize = 3;

/// The `edit` command.
pub(crate) struct Edit;

/// Stage implementations of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Edit)]
}

impl StageImpl for Edit {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if args.value(CHANGES).is_some() {
            changes_param(args, range_param(args, EXPECT)?)?;
        }
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

impl EffectStage for Edit {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let path = path_arg(args, PATH, ctx)?.ok_or_else(|| missing_param(COMMAND, PATH))?;
        let changes = changes_param(args, range_param(args, EXPECT)?)?;
        let options = EditOptions {
            whitespace: whitespace_param(args)?,
            on_missing: on_missing_param(args)?,
            expect: None,
            ensure_newline: false,
        };
        let validate = validate::validate_param(args)?;
        let if_match = params::if_match_param(args, ctx)?;
        let file = read_checked(ctx, &path, if_match.as_ref())?;
        let planned = edit_plan::plan(&file.text, &changes, &options)
            .map_err(|error| edit_error(&error, &path))?;

        let mut notable = Vec::new();
        let mut replacements: Option<u64> = None;
        for (index, (change, outcome)) in changes.iter().zip(&planned.outcomes).enumerate() {
            if !matches!(change, edit_plan::Change::Replace(_)) {
                continue;
            }
            replacements = Some(replacements.unwrap_or(0) + count_u64(outcome.count));
            if outcome.count == 0 {
                notable.push(NotableEntry {
                    detail: Some(format!("change {}", index + 1)),
                    ..at_path(Notable::NothingMatched, &path)
                });
            }
        }
        let edit = FileEdit {
            file,
            text: planned.text,
            replacements,
        };
        effect_plan(ctx, COMMAND, validate, vec![edit], notable)
    }

    fn apply<'a>(
        &'a self,
        _args: &'a BoundArgs,
        plan: EffectPlan,
        tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        apply_edits(plan, tx, ctx)
    }
}

/// Reads `changes` into the changes of an edit; `expect` is the value of
/// `expect` of the command, the default of every replacement.
fn changes_param(args: &BoundArgs, expect: Option<Range>) -> Result<Vec<edit_plan::Change>> {
    let Value::List(items) = args.require(CHANGES)? else {
        return Err(type_error(COMMAND, CHANGES, "a list of maps"));
    };
    if items.is_empty() {
        return Err(invalid_value(
            CHANGES,
            "`changes` is empty: give at least one change".to_owned(),
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            Value::Map(map) => change_from_map(index, map, expect),
            _ => Err(invalid_value(
                CHANGES,
                format!(
                    "change {} is not a map: write it as `{{replace: \"old\", with: \"new\"}}`",
                    index + 1
                ),
            )),
        })
        .collect()
}

fn change_from_map(index: usize, map: &Fields, expect: Option<Range>) -> Result<edit_plan::Change> {
    let number = index + 1;
    let kinds: Vec<&str> = [REPLACE, INSERT, DELETE_LINES]
        .into_iter()
        .filter(|key| map.contains_key(*key))
        .collect();
    let [kind] = kinds.as_slice() else {
        return Err(invalid_value(
            CHANGES,
            format!("change {number} needs exactly one of `replace`, `insert` and `delete_lines`"),
        ));
    };
    let allowed: &[&str] = match *kind {
        REPLACE => &REPLACE_KEYS,
        INSERT => &INSERT_KEYS,
        _ => &DELETE_KEYS,
    };
    if let Some(unknown) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "change {number}: `{unknown}` does not apply to a `{kind}` change; use {}",
                allowed.join(", ")
            ),
        )
        .with_found(unknown.clone())
        .with_expected(serde_json::json!(allowed)));
    }
    let entry = |key: &str| map.get(key).filter(|value| !value.is_null());
    let key_name = |key: &str| format!("{CHANGES}[{number}].{key}");
    match *kind {
        REPLACE => {
            let pattern = match entry(REPLACE) {
                Some(Value::Text(text)) => Pattern::Literal(text.clone()),
                Some(Value::Regex(regex)) => regex_pattern(regex, true, false)?,
                _ => {
                    return Err(type_error(
                        COMMAND,
                        &key_name(REPLACE),
                        "a text or a regular expression",
                    ));
                }
            };
            let with = match entry(WITH) {
                Some(Value::Text(text)) => text.clone(),
                Some(_) => return Err(type_error(COMMAND, &key_name(WITH), "a text")),
                None => {
                    return Err(missing_param(COMMAND, &key_name(WITH)));
                }
            };
            let mut change = ReplaceChange::new(pattern, with);
            change.occurrence = occurrence_value(&key_name(OCCURRENCE), entry(OCCURRENCE))?;
            change.expect = range_value(&key_name(EXPECT), entry(EXPECT))?.or(expect);
            change.whitespace = entry(WHITESPACE)
                .map(|value| whitespace_value(&key_name(WHITESPACE), value))
                .transpose()?;
            change.on_missing = entry(params::ON_MISSING)
                .map(|value| on_missing_value(&key_name(params::ON_MISSING), value))
                .transpose()?;
            change.whole_word = match entry(WHOLE_WORD) {
                None => false,
                Some(Value::Bool(flag)) => *flag,
                Some(_) => return Err(type_error(COMMAND, &key_name(WHOLE_WORD), "a bool")),
            };
            Ok(edit_plan::Change::Replace(change))
        }
        INSERT => {
            let Some(Value::Text(text)) = entry(INSERT) else {
                return Err(type_error(COMMAND, &key_name(INSERT), "a text"));
            };
            let line = match entry(LINE) {
                None => None,
                Some(Value::Int(line)) => Some(*line),
                Some(_) => return Err(type_error(COMMAND, &key_name(LINE), "an int")),
            };
            let position = entry(POSITION)
                .map(|value| position_value(&key_name(POSITION), value))
                .transpose()?
                .unwrap_or_default();
            Ok(edit_plan::Change::Insert(InsertChange {
                text: text.clone(),
                line,
                position,
            }))
        }
        _ => range_value(&key_name(DELETE_LINES), entry(DELETE_LINES))?
            .map(edit_plan::Change::DeleteLines)
            .ok_or_else(|| type_error(COMMAND, &key_name(DELETE_LINES), "a range of lines")),
    }
}

/// A text file read for a change and its new text.
#[derive(Debug, Clone)]
pub(crate) struct FileEdit {
    /// The file as read.
    pub(crate) file: TextFile,
    /// New text of the file.
    pub(crate) text: String,
    /// Replacements made, for the commands that replace.
    pub(crate) replacements: Option<u64>,
}

/// Private state of a planned text change.
struct PlannedEdits {
    command: &'static str,
    validate: Validate,
    edits: Vec<FileEdit>,
}

/// Plan of the effect of text changes.
///
/// Edits whose new text equals the text read are left out and reported as
/// `unchanged`. Every other edit gets a planned `change` record with its
/// difference, and the notable effects of its encoding and line endings.
/// The plan is a bulk operation when it changes more than one file.
///
/// # Errors
///
/// Fails with `E_ENCODING` when the encoding of a file cannot represent its
/// new text.
pub(crate) fn effect_plan(
    ctx: &ExecCtx,
    command: &'static str,
    validate: Validate,
    edits: Vec<FileEdit>,
    mut notable: Vec<NotableEntry>,
) -> Result<EffectPlan> {
    let report = &ctx.config().report;
    let threshold = u64::from(report.notable.per_file_repl);
    let max_hunks = usize::try_from(report.diff_max_hunks).unwrap_or(usize::MAX);
    let mut changes = Vec::with_capacity(edits.len());
    let mut kept = Vec::with_capacity(edits.len());
    for edit in edits {
        if edit.text == edit.file.text {
            notable.push(at_path(Notable::Unchanged, &edit.file.path));
            continue;
        }
        let bytes = encode_preserving(&edit.text, &edit.file)?;
        changes.push(planned_text_change(&edit, &bytes, max_hunks));
        notable.extend(preserved_text(&edit.file));
        if let Some(count) = edit.replacements
            && threshold > 0
            && count >= threshold
        {
            notable.push(NotableEntry {
                count: Some(count),
                ..at_path(Notable::ManyPerFile, &edit.file.path)
            });
        }
        kept.push(edit);
    }
    let mut plan = EffectPlan::new();
    plan.bulk = kept.len() > 1;
    plan.changes = changes;
    plan.notable = notable;
    Ok(plan.with_state(PlannedEdits {
        command,
        validate,
        edits: kept,
    }))
}

fn planned_text_change(edit: &FileEdit, bytes: &[u8], max_hunks: usize) -> Change {
    let hunks = unified_diff(&edit.file.text, &edit.text, DIFF_CONTEXT, false);
    let (added, removed) = hunks.iter().fold((0, 0), |(added, removed), hunk| {
        let (plus, minus) = hunk.counts();
        (added + plus, removed + minus)
    });
    let path = path_field(&edit.file.path);
    let diff = hunks
        .iter()
        .take(max_hunks)
        .map(|hunk| record::Hunk {
            path: path.clone(),
            old_start: count_u64(hunk.old_start),
            old_len: count_u64(hunk.old_len),
            new_start: count_u64(hunk.new_start),
            new_len: count_u64(hunk.new_len),
            lines: hunk.prefixed_lines(),
            header: hunk.header.clone(),
            ..record::Hunk::default()
        })
        .collect();
    Change {
        replacements: edit.replacements,
        added: Some(count_u64(added)),
        removed: Some(count_u64(removed)),
        diff: Some(diff),
        ..text_change(&edit.file, bytes)
    }
}

/// Writes the edits of a plan made by [`effect_plan`] and checks the result.
///
/// Every file is declared before the first write and must still have the
/// entity tag it had when it was read. After the writes the new content of
/// every file is validated ([`validate::check_change`]); the output is one
/// `effect` record with the applied changes.
///
/// # Errors
///
/// Fails with `E_STAGE_FAILED` for a plan made elsewhere, with the errors of
/// the transaction (`E_STALE` for a file changed since it was read) and with
/// `E_ROLLED_BACK` when the new content adds a syntax error and `validate`
/// is `reject`.
pub(crate) fn apply_edits<'a>(
    mut plan: EffectPlan,
    tx: &mut dyn MutationTx,
    ctx: &'a ExecCtx,
) -> Result<EffectOutcome<'a>> {
    let planned: PlannedEdits = plan.take_state().ok_or_else(|| {
        Error::new(
            ErrorCode::StageFailed,
            "the plan holds no text changes: plan the change with the same command",
        )
    })?;
    let mut changes = Vec::with_capacity(planned.edits.len());
    if !planned.edits.is_empty() {
        let targets: Vec<TargetSpec> = planned
            .edits
            .iter()
            .map(|edit| edit.file.target())
            .collect();
        let preserved = tx.preserve(&targets)?;
        let steps = planned
            .edits
            .iter()
            .zip(&preserved)
            .zip(&targets)
            .zip(plan.changes);
        for (((edit, kept), target), planned_change) in steps {
            ctx.checkpoint()?;
            let bytes = encode_preserving(&edit.text, &edit.file)?;
            let applied = tx.apply(kept.index, Mutation::WriteBytes(bytes))?;
            let journal = applied_change(target, kept, &applied);
            changes.push(Change {
                size_after: journal.size_after.or(planned_change.size_after),
                hash_before: journal.hash_before,
                hash_after: journal.hash_after,
                store: journal.store,
                irreversible: journal.irreversible,
                ..planned_change
            });
        }
        for edit in &planned.edits {
            validate::check_change(
                ctx,
                planned.command,
                &edit.file.path,
                edit.file.text.as_bytes(),
                edit.text.as_bytes(),
                planned.validate,
            )?;
        }
    }
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: planned.command.to_owned(),
        targets: Some(count_u64(changes.len())),
        changes: Some(changes.clone()),
        ..Effect::default()
    };
    let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(Record::from(
        effect,
    ))));
    outcome.changes = changes;
    Ok(outcome)
}

/// Reads a text file and checks the tag `if_match` expects of it.
///
/// # Errors
///
/// Fails as [`read_text`] (`E_NOT_FOUND`, `E_BINARY`, `E_ENCODING`,
/// `E_LIMIT`) and with `E_STALE` when the file does not have the expected
/// tag.
pub(crate) fn read_checked(
    ctx: &ExecCtx,
    path: &AbsPath,
    if_match: Option<&IfMatch>,
) -> Result<TextFile> {
    let file = read_text(ctx, path, EncodingParam::Auto)?;
    if let Some(if_match) = if_match {
        check_if_match(
            path,
            &Observed::Present(file.meta.clone()),
            if_match.expected(path),
        )?;
    }
    Ok(file)
}

/// Checks the entity tag carried by an input record against the file as
/// read; an empty tag is not checked.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for a tag that is not an entity tag and with
/// `E_STALE` when the file changed since the record was produced.
pub(crate) fn check_record_etag(file: &TextFile, etag: &str) -> Result<()> {
    if etag.is_empty() {
        return Ok(());
    }
    let expected = parse_etag(etag)?;
    let actual = file.etag();
    if actual.matches(&expected) {
        Ok(())
    } else {
        Err(stale_error(&file.path, &expected, Some(&actual)))
    }
}

/// Compiles a regular expression of the language for replacing.
///
/// `^` and `$` match at line boundaries and before `\r\n`. A pattern the
/// linear-time engine rejects, for example one with look-around or
/// back-references, is compiled by the backtracking engine.
///
/// # Errors
///
/// Fails with `E_REGEX` when neither engine accepts the pattern and with
/// `E_LIMIT` when it compiles to a program too large.
pub(crate) fn regex_pattern(
    regex: &value::Regex,
    case_sensitive: bool,
    dot_all: bool,
) -> Result<Pattern> {
    let flags = regex.flags;
    let case_insensitive = flags.case_insensitive || !case_sensitive;
    let dot_all = flags.dot_matches_new_line || dot_all;
    let linear = regex::RegexBuilder::new(&regex.pattern)
        .case_insensitive(case_insensitive)
        .multi_line(true)
        .crlf(true)
        .dot_matches_new_line(dot_all)
        .ignore_whitespace(flags.extended)
        .build();
    match linear {
        Ok(compiled) => Ok(Pattern::Regex(compiled)),
        Err(regex::Error::Syntax(message)) => fancy_regex::RegexBuilder::new(&regex.pattern)
            .case_insensitive(case_insensitive)
            .multi_line(true)
            .crlf(true)
            .dot_matches_new_line(dot_all)
            .ignore_whitespace(flags.extended)
            .backtrack_limit(DEFAULT_BACKTRACK_LIMIT)
            .build()
            .map(Pattern::Backtracking)
            .map_err(|_| regex_error(regex, &message)),
        Err(error) => Err(Error::new(
            ErrorCode::Limit,
            format!("`{regex}` compiles to a program too large: {error}; simplify the pattern"),
        )),
    }
}

fn regex_error(regex: &value::Regex, message: &str) -> Error {
    let reason = message
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map_or(message, str::trim);
    let reason = reason.strip_prefix("error: ").unwrap_or(reason);
    Error::new(
        ErrorCode::Regex,
        format!("`{regex}` is not a valid regular expression: {reason}"),
    )
    .with_found(regex.to_string())
}

/// Reads `whitespace`; `normalize` without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value and with
/// `E_UNKNOWN_VALUE` for an unknown value.
pub(crate) fn whitespace_param(args: &BoundArgs) -> Result<Whitespace> {
    match args.value(WHITESPACE) {
        None | Some(Value::Null) => Ok(Whitespace::Normalize),
        Some(value) => whitespace_value(WHITESPACE, value),
    }
}

fn whitespace_value(name: &str, value: &Value) -> Result<Whitespace> {
    let text = choice_text(name, value)?;
    Whitespace::from_name(text).ok_or_else(|| unknown_value(name, text, &WHITESPACE_VALUES))
}

/// Reads `on_missing`; `skip` without a value.
///
/// # Errors
///
/// Fails as [`params::on_missing_param`].
pub(crate) fn on_missing_param(args: &BoundArgs) -> Result<OnMissing> {
    Ok(match params::on_missing_param(args)? {
        params::OnMissing::Skip => OnMissing::Skip,
        params::OnMissing::Error => OnMissing::Error,
    })
}

fn on_missing_value(name: &str, value: &Value) -> Result<OnMissing> {
    let text = choice_text(name, value)?;
    OnMissing::from_name(text).ok_or_else(|| unknown_value(name, text, &ON_MISSING_VALUES))
}

/// Reads `position`; `after` without a value.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an enumeration value and with
/// `E_UNKNOWN_VALUE` for an unknown value.
pub(crate) fn position_param(args: &BoundArgs) -> Result<Position> {
    match args.value(POSITION) {
        None | Some(Value::Null) => Ok(Position::default()),
        Some(value) => position_value(POSITION, value),
    }
}

fn position_value(name: &str, value: &Value) -> Result<Position> {
    let text = choice_text(name, value)?;
    Position::from_name(text).ok_or_else(|| unknown_value(name, text, &POSITION_VALUES))
}

fn choice_text<'a>(name: &str, value: &'a Value) -> Result<&'a str> {
    match value {
        Value::Enum(text) | Value::Text(text) => Ok(text.as_str()),
        _ => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects a value of the enumeration"),
        )),
    }
}

/// Reads a range parameter; an integer is the range of that number alone.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value of another type.
pub(crate) fn range_param(args: &BoundArgs, name: &str) -> Result<Option<Range>> {
    range_value(name, args.value(name))
}

fn range_value(name: &str, value: Option<&Value>) -> Result<Option<Range>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Range(range)) => Ok(Some(*range)),
        Some(Value::Int(number)) => Ok(Some(Range::single(*number))),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects a range such as `1` or `1..3`"),
        )),
    }
}

/// Reads `occurrence`, counted from 1.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not an integer and with
/// `E_INVALID_VALUE` for a number below 1.
pub(crate) fn occurrence_param(args: &BoundArgs) -> Result<Option<NonZeroUsize>> {
    occurrence_value(OCCURRENCE, args.value(OCCURRENCE))
}

fn occurrence_value(name: &str, value: Option<&Value>) -> Result<Option<NonZeroUsize>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(number)) => usize::try_from(*number)
            .ok()
            .and_then(NonZeroUsize::new)
            .map(Some)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`{name}` is {number}: occurrences are counted from 1"),
                )
                .with_found(number.to_string())
            }),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!("`{name}` expects an int"),
        )),
    }
}

/// Error of a change that cannot be planned in `path`, with the code of the
/// planner.
#[must_use]
pub(crate) fn edit_error(error: &EditError, path: &AbsPath) -> Error {
    let base = Error::new(error.code(), format!("`{path}`: {error}"))
        .with_detail("path", Json::from(path.to_language_string()));
    match error {
        EditError::Match { source, .. } => with_match_details(base, source),
        EditError::Template { source, .. } => with_template_details(base, source),
        EditError::TotalMismatch { expected, found } => base
            .with_expected(Json::from(expected.to_string()))
            .with_actual(Json::from(*found)),
        EditError::LineOutOfRange { line, lines, .. } => base
            .with_detail("lines", Json::from(*lines))
            .with_actual(Json::from(*line)),
        _ => base,
    }
}

/// Error of a literal pattern that cannot be matched or selected in `path`.
#[must_use]
pub(crate) fn match_error(error: &MatchError, path: &AbsPath) -> Error {
    with_match_details(
        Error::new(error.code(), format!("`{path}`: {error}"))
            .with_detail("path", Json::from(path.to_language_string())),
        error,
    )
}

/// Error of a replacement template that is not valid.
#[must_use]
pub(crate) fn template_error(error: &TemplateError) -> Error {
    with_template_details(
        Error::new(
            error.code(),
            format!("`{WITH}` is not a valid template: {error}"),
        ),
        error,
    )
}

fn with_template_details(error: Error, source: &TemplateError) -> Error {
    error.with_detail("column", Json::from(source.column()))
}

fn with_match_details(error: Error, source: &MatchError) -> Error {
    let (excerpts, expected, found): (&[Excerpt], Option<String>, Option<usize>) = match source {
        MatchError::NoMatch { nearest } => (nearest, None, Some(0)),
        MatchError::OccurrenceMissing {
            occurrence,
            found,
            candidates,
        } => (candidates, Some(occurrence.to_string()), Some(*found)),
        MatchError::CountMismatch {
            expected,
            found,
            candidates,
        } => (candidates, Some(expected.to_string()), Some(*found)),
        _ => (&[], None, None),
    };
    let mut error = error;
    if let Some(expected) = expected {
        error = error.with_expected(Json::from(expected));
    }
    if let Some(found) = found {
        error = error.with_actual(Json::from(found));
    }
    for excerpt in excerpts {
        error = error.with_sample(format!(
            "{}:{}: {}",
            excerpt.line, excerpt.column, excerpt.text
        ));
    }
    error
}

/// `E_TYPE` for a parameter of `command` with a value of another type.
#[must_use]
pub(crate) fn type_error(command: &str, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{command}` expects {expected} for `{name}`"),
    )
}

/// `E_MISSING_PARAM` for a required parameter of `command` without a value.
#[must_use]
pub(crate) fn missing_param(command: &str, name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        format!("`{command}` needs a value for `{name}`"),
    )
}

/// `E_INVALID_VALUE` for a parameter whose value cannot be used.
#[must_use]
pub(crate) fn invalid_value(name: &str, message: String) -> Error {
    Error::new(ErrorCode::InvalidValue, message).with_detail("param", Json::from(name))
}

/// `E_UNKNOWN_VALUE` for an enumeration value outside `expected`.
#[must_use]
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

/// A count as the value of a record field.
#[must_use]
pub(crate) fn count_u64(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Values shared by the tests of the text commands.

    use nurt_contracts::fs::AbsPath;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    /// Absolute path of the virtual file system for a path written with `/`.
    pub(crate) fn abs(text: &str) -> AbsPath {
        test_path(text).expect("test paths are absolute")
    }

    /// Test context whose file system holds one file.
    pub(crate) fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs()
            .write_file(&path, content)
            .expect("the virtual file system accepts the file");
        (test, path)
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::RegexFlags;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;
    use nurt_testkit::journal::JournalCall;

    use super::*;
    use crate::commands::edit::fixture::with_file;
    use crate::common::etag::etag_text;

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn map(entries: &[(&str, Value)]) -> Value {
        Value::Map(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        )
    }

    fn args(path: &AbsPath, changes: Vec<Value>) -> BoundArgs {
        BoundArgs::new(COMMAND)
            .with(
                PATH,
                ArgValue::Value(Value::Path(path.to_language_string())),
                false,
            )
            .with(CHANGES, ArgValue::Value(Value::List(changes)), false)
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Vec<Change>> {
        let ctx = test.ctx();
        let plan = Edit.plan(args, StageInput::none(), &ctx)?;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = Edit.apply(args, plan, tx.as_mut(), &ctx)?;
        tx.commit()?;
        Ok(outcome.changes)
    }

    #[test]
    fn changes_are_read_from_maps() {
        let path = crate::commands::edit::fixture::abs("/data/a.txt");
        let given = args(
            &path,
            vec![
                map(&[
                    (REPLACE, text("a")),
                    (WITH, text("b")),
                    (OCCURRENCE, Value::Int(2)),
                    (WHOLE_WORD, Value::Bool(true)),
                    (WHITESPACE, Value::Enum("exact".to_owned())),
                    (params::ON_MISSING, Value::Enum("error".to_owned())),
                ]),
                map(&[
                    (INSERT, text("x")),
                    (LINE, Value::Int(3)),
                    (POSITION, Value::Enum("before".to_owned())),
                ]),
                map(&[(DELETE_LINES, Value::Int(4))]),
            ],
        );
        let changes = changes_param(&given, Some(Range::single(1))).unwrap();
        let edit_plan::Change::Replace(replace) = &changes[0] else {
            panic!("expected a replacement");
        };
        assert_eq!(replace.occurrence, NonZeroUsize::new(2));
        assert!(replace.whole_word);
        assert_eq!(replace.whitespace, Some(Whitespace::Exact));
        assert_eq!(replace.on_missing, Some(OnMissing::Error));
        assert_eq!(replace.expect, Some(Range::single(1)));
        assert!(matches!(
            &changes[1],
            edit_plan::Change::Insert(InsertChange {
                line: Some(3),
                position: Position::Before,
                ..
            })
        ));
        assert!(matches!(
            &changes[2],
            edit_plan::Change::DeleteLines(range) if *range == Range::single(4)
        ));
    }

    #[test]
    fn malformed_changes_are_rejected() {
        let path = crate::commands::edit::fixture::abs("/data/a.txt");
        let cases = [
            (vec![], ErrorCode::InvalidValue),
            (vec![text("a")], ErrorCode::InvalidValue),
            (vec![map(&[(WITH, text("b"))])], ErrorCode::InvalidValue),
            (
                vec![map(&[(REPLACE, text("a")), (INSERT, text("b"))])],
                ErrorCode::InvalidValue,
            ),
            (
                vec![map(&[(REPLACE, text("a")), (LINE, Value::Int(1))])],
                ErrorCode::InvalidValue,
            ),
            (vec![map(&[(REPLACE, text("a"))])], ErrorCode::MissingParam),
            (
                vec![map(&[(REPLACE, Value::Int(1)), (WITH, text("b"))])],
                ErrorCode::Type,
            ),
            (
                vec![map(&[(REPLACE, text("a")), (WITH, Value::Int(1))])],
                ErrorCode::Type,
            ),
            (
                vec![map(&[
                    (REPLACE, text("a")),
                    (WITH, text("b")),
                    (OCCURRENCE, Value::Int(0)),
                ])],
                ErrorCode::InvalidValue,
            ),
            (
                vec![map(&[
                    (REPLACE, text("a")),
                    (WITH, text("b")),
                    (WHITESPACE, text("loose")),
                ])],
                ErrorCode::UnknownValue,
            ),
            (
                vec![map(&[
                    (REPLACE, text("a")),
                    (WITH, text("b")),
                    (WHOLE_WORD, text("yes")),
                ])],
                ErrorCode::Type,
            ),
            (
                vec![map(&[(INSERT, text("a")), (LINE, text("3"))])],
                ErrorCode::Type,
            ),
            (
                vec![map(&[(INSERT, text("a")), (POSITION, text("middle"))])],
                ErrorCode::UnknownValue,
            ),
            (vec![map(&[(INSERT, Value::Int(1))])], ErrorCode::Type),
            (vec![map(&[(DELETE_LINES, text("1"))])], ErrorCode::Type),
        ];
        for (changes, code) in cases {
            let error = changes_param(&args(&path, changes.clone()), None).unwrap_err();
            assert_eq!(error.code(), code, "{changes:?}");
        }
        let not_list = BoundArgs::new(COMMAND).with(CHANGES, ArgValue::Value(text("x")), false);
        assert_eq!(
            changes_param(&not_list, None).unwrap_err().code(),
            ErrorCode::Type
        );
    }

    #[test]
    fn edit_applies_every_change_against_the_text_before_the_edit() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\nthree\nfour\n");
        let changes = run(
            &test,
            &args(
                &path,
                vec![
                    map(&[(
                        DELETE_LINES,
                        Value::Range(Range::new(Some(2), Some(3)).unwrap()),
                    )]),
                    map(&[
                        (INSERT, text("middle")),
                        (LINE, Value::Int(2)),
                        (POSITION, Value::Enum("before".to_owned())),
                    ]),
                    map(&[(REPLACE, text("four")), (WITH, text("4"))]),
                ],
            ),
        )
        .unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"one\nmiddle\n4\n");
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].replacements, Some(1));
        assert_eq!(changes[0].added, Some(2));
        assert_eq!(changes[0].removed, Some(3));
        assert!(changes[0].hash_after.is_some());
    }

    #[test]
    fn overlapping_changes_change_nothing() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\nthree\n");
        let error = run(
            &test,
            &args(
                &path,
                vec![
                    map(&[(
                        DELETE_LINES,
                        Value::Range(Range::new(Some(1), Some(2)).unwrap()),
                    )]),
                    map(&[(REPLACE, text("two")), (WITH, text("2"))]),
                ],
            ),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert!(error.message.contains("overlap"));
        assert_eq!(test.fs().read_file(&path).unwrap(), b"one\ntwo\nthree\n");
        assert!(
            test.journal()
                .calls()
                .iter()
                .all(|call| !matches!(call, JournalCall::Preserve { .. }))
        );
    }

    #[test]
    fn lines_and_occurrences_outside_the_file_are_out_of_range() {
        let (test, path) = with_file("/data/a.txt", b"one\ntwo\n");
        let cases = [
            map(&[(
                DELETE_LINES,
                Value::Range(Range::new(Some(2), Some(5)).unwrap()),
            )]),
            map(&[(INSERT, text("x")), (LINE, Value::Int(9))]),
            map(&[
                (INSERT, text("x")),
                (LINE, Value::Int(0)),
                (POSITION, Value::Enum("before".to_owned())),
            ]),
            map(&[
                (REPLACE, text("o")),
                (WITH, text("0")),
                (OCCURRENCE, Value::Int(5)),
                (params::ON_MISSING, Value::Enum("error".to_owned())),
            ]),
        ];
        for change in cases {
            let error = run(&test, &args(&path, vec![change.clone()])).unwrap_err();
            assert_eq!(error.code(), ErrorCode::OutOfRange, "{change:?}");
        }
        let line = run(
            &test,
            &args(
                &path,
                vec![map(&[(INSERT, text("x")), (LINE, Value::Int(9))])],
            ),
        )
        .unwrap_err();
        assert_eq!(line.actual, Some(Json::from(9)));
        assert_eq!(test.fs().read_file(&path).unwrap(), b"one\ntwo\n");
        assert!(
            test.journal()
                .calls()
                .iter()
                .all(|call| !matches!(call, JournalCall::Preserve { .. }))
        );
    }

    #[test]
    fn the_expect_of_the_command_applies_to_every_replacement() {
        let (test, path) = with_file("/data/a.txt", b"a a\nb\n");
        let given = args(
            &path,
            vec![
                map(&[(REPLACE, text("a")), (WITH, text("x"))]),
                map(&[(REPLACE, text("b")), (WITH, text("y"))]),
            ],
        )
        .with(EXPECT, ArgValue::Value(Value::Int(1)), false);
        let error = run(&test, &given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Assert);
        assert_eq!(error.actual, Some(Json::from(2)));
        assert_eq!(error.sample.len(), 2);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"a a\nb\n");
    }

    #[test]
    fn unchanged_text_writes_nothing() {
        let (test, path) = with_file("/data/a.txt", b"same\n");
        let ctx = test.ctx();
        let given = args(
            &path,
            vec![map(&[(REPLACE, text("same")), (WITH, text("same"))])],
        );
        let plan = Edit.plan(&given, StageInput::none(), &ctx).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.notable[0].code, Notable::Unchanged);
        let missing = args(
            &path,
            vec![map(&[(REPLACE, text("other")), (WITH, text("x"))])],
        );
        let plan = Edit.plan(&missing, StageInput::none(), &ctx).unwrap();
        assert_eq!(plan.notable[0].code, Notable::NothingMatched);
        assert_eq!(plan.notable[0].detail.as_deref(), Some("change 1"));
    }

    #[test]
    fn if_match_with_another_tag_fails_with_stale() {
        let (test, path) = with_file("/data/a.txt", b"old\n");
        let ctx = test.ctx();
        let file = read_text(&ctx, &path, EncodingParam::Auto).unwrap();
        let mut tag = file.etag();
        tag.size += 1;
        let given = args(
            &path,
            vec![map(&[(REPLACE, text("old")), (WITH, text("new"))])],
        )
        .with(
            params::IF_MATCH,
            ArgValue::Value(text(&etag_text(&tag))),
            false,
        );
        let error = Edit.plan(&given, StageInput::none(), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        let current = args(
            &path,
            vec![map(&[(REPLACE, text("old")), (WITH, text("new"))])],
        )
        .with(
            params::IF_MATCH,
            ArgValue::Value(text(&etag_text(&file.etag()))),
            false,
        );
        run(&test, &current).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"new\n");
    }

    #[test]
    fn record_etags_are_checked_against_the_file() {
        let (test, path) = with_file("/data/a.txt", b"x\n");
        let file = read_text(&test.ctx(), &path, EncodingParam::Auto).unwrap();
        check_record_etag(&file, "").unwrap();
        check_record_etag(&file, &etag_text(&file.etag())).unwrap();
        let mut other = file.etag();
        other.modified_ns += 1;
        assert_eq!(
            check_record_etag(&file, &etag_text(&other))
                .unwrap_err()
                .code(),
            ErrorCode::Stale
        );
        assert_eq!(
            check_record_etag(&file, "nonsense").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn regular_expressions_use_the_backtracking_engine_when_needed() {
        let linear = value::Regex::new(r"(\w+)=\d", "").unwrap();
        assert!(matches!(
            regex_pattern(&linear, true, false).unwrap(),
            Pattern::Regex(_)
        ));
        let look_ahead = value::Regex::new(r"foo(?=bar)", "").unwrap();
        assert!(matches!(
            regex_pattern(&look_ahead, true, false).unwrap(),
            Pattern::Backtracking(_)
        ));
        let broken = value::Regex::new("a(", "").unwrap();
        let error = regex_pattern(&broken, true, false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Regex);
        assert!(error.message.contains("not a valid regular expression"));
        let insensitive = value::Regex {
            pattern: "^abc$".to_owned(),
            flags: RegexFlags::default(),
        };
        let Pattern::Regex(compiled) = regex_pattern(&insensitive, false, false).unwrap() else {
            panic!("expected the linear engine");
        };
        assert!(compiled.is_match("x\r\nABC\r\n"));
    }

    #[test]
    fn scalar_parameters_have_catalog_defaults() {
        let empty = BoundArgs::new(COMMAND);
        assert_eq!(whitespace_param(&empty).unwrap(), Whitespace::Normalize);
        assert_eq!(on_missing_param(&empty).unwrap(), OnMissing::Skip);
        assert_eq!(position_param(&empty).unwrap(), Position::After);
        assert_eq!(occurrence_param(&empty).unwrap(), None);
        assert_eq!(range_param(&empty, EXPECT).unwrap(), None);
        let given = BoundArgs::new(COMMAND)
            .with(WHITESPACE, ArgValue::Value(Value::Bool(true)), false)
            .with(OCCURRENCE, ArgValue::Value(text("1")), false)
            .with(EXPECT, ArgValue::Value(text("1")), false)
            .with(
                POSITION,
                ArgValue::Value(Value::Enum("end".to_owned())),
                false,
            );
        assert_eq!(
            whitespace_param(&given).unwrap_err().code(),
            ErrorCode::Type
        );
        assert_eq!(
            occurrence_param(&given).unwrap_err().code(),
            ErrorCode::Type
        );
        assert_eq!(
            range_param(&given, EXPECT).unwrap_err().code(),
            ErrorCode::Type
        );
        assert_eq!(position_param(&given).unwrap(), Position::End);
    }

    #[test]
    fn match_errors_carry_samples_and_counts() {
        let path = crate::commands::edit::fixture::abs("/data/a.txt");
        let error = match_error(
            &MatchError::CountMismatch {
                expected: Range::single(1),
                found: 2,
                candidates: vec![Excerpt {
                    line: 3,
                    column: 5,
                    text: "let a = 1;".to_owned(),
                }],
            },
            &path,
        );
        assert_eq!(error.code(), ErrorCode::Assert);
        assert_eq!(error.sample, ["3:5: let a = 1;"]);
        assert_eq!(error.expected, Some(Json::from("1")));
        assert_eq!(error.actual, Some(Json::from(2)));
        let empty = match_error(&MatchError::EmptyPattern, &path);
        assert_eq!(empty.code(), ErrorCode::InvalidValue);
        assert!(empty.sample.is_empty());
        let beyond = match_error(
            &MatchError::OccurrenceMissing {
                occurrence: NonZeroUsize::new(3).unwrap(),
                found: 2,
                candidates: Vec::new(),
            },
            &path,
        );
        assert_eq!(beyond.code(), ErrorCode::OutOfRange);
        assert_eq!(beyond.expected, Some(Json::from("3")));
        assert_eq!(beyond.actual, Some(Json::from(2)));
    }
}
