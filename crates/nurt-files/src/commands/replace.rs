// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `replace` command: replacement of text in files, in input lines or
//! content, and of input matches.
//!
//! | Form | Input | Replaced |
//! |---|---|---|
//! | source | none | matches of `pattern` in the files of `in` |
//! | `lines` | `line`, `content` | matches of `pattern` inside the lines of the records |
//! | `matches` | `match` | the text of every input match |
//!
//! A literal pattern is matched with the tolerance of `whitespace`, and its
//! replacement takes the indentation and line endings of the place it
//! replaces. A regular expression is matched exactly, `^` and `$` match at
//! line boundaries, and without `multiline` a match does not span lines.
//! `with` is a template for regular expressions and input matches: `$1`,
//! `${1}` and `${name}` insert groups, `$$` inserts `$`.
//!
//! `expect` counts the matches of all files before `occurrence` applies;
//! `occurrence` selects one match in every file. Nothing is planned when a
//! count is outside `expect`, or when nothing matches and `on_missing` is
//! `error`; an occurrence that no file has fails with `E_OUT_OF_RANGE` when
//! `on_missing` is `error`.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::ops::{ControlFlow, Range as Span};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::services::{CodeService, LanguageId, SpanClass};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, RecordStream,
    StageDescriptor, StageImpl, StageInput, StaticInfo,
};
use nurt_contracts::value::{self, Range, RecordKind, RegexFlags, Value};
use nurt_text::codec::Codec;
use nurt_text::edit_plan::Pattern;
use nurt_text::fuzzy_match::{self, Adapter, MatchOptions, Whitespace};
use nurt_text::template::{CaptureSource, Groups, Template};
use serde_json::Value as Json;

use super::edit::{
    EXPECT, FileEdit, OCCURRENCE, WHOLE_WORD, WITH, apply_edits, check_record_etag, count_u64,
    effect_plan, invalid_value, match_error, missing_param, occurrence_param, range_param,
    regex_pattern, template_error, type_error, unknown_value, whitespace_param,
};
use super::validate::{self, Validate};
use crate::common::etag::{Observed, check_if_match};
use crate::common::notable::{at_path, counted};
use crate::common::params::{self, EncodingParam, IfMatch, OnMissing};
use crate::common::path::{path_list_arg, record_path};
use crate::common::text::{TextFile, read_text};
use crate::walk::{KindFilter, WalkItem, WalkOptions, Walker};

const COMMAND: &str = "replace";
const PATTERN: &str = "pattern";
const IN: &str = "in";
const GLOB: &str = "glob";
const MODE: &str = "mode";
const CASE_SENSITIVE: &str = "case_sensitive";
const MULTILINE: &str = "multiline";
const SCOPE: &str = "scope";
const CHECK_REFERENCES: &str = "check_references";
const TEMPLATE: &str = "template";
const BINARY: &str = "binary";
const FILE_TYPE: &str = "file_type";

const DEFAULT_GLOB: &str = "**";
const MODE_VALUES: [&str; 4] = ["auto", "literal", "regex", "syntax"];
const SCOPE_VALUES: [&str; 4] = ["any", "code", "comments", "strings"];
const BINARY_VALUES: [&str; 2] = ["skip", "text"];

/// Matches listed in an `expect` error.
const CANDIDATES_SHOWN: usize = 8;
/// Places listed in the detail of `references_left`.
const REFERENCES_SHOWN: usize = 5;
/// Distinct replaced texts searched for by `check_references` in one file.
const NAMES_MAX: usize = 16;
/// Characters of a line kept in a sample.
const EXCERPT_CHARS: usize = 80;

/// The `replace` command.
pub(crate) struct Replace;

/// Stage implementations of this module.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Replace)]
}

impl StageImpl for Replace {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![
                RecordKind::Match,
                RecordKind::Line,
                RecordKind::Content,
            ]),
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        static_info(args)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl EffectStage for Replace {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        let options = Options::from_args(args)?;
        let if_match = params::if_match_param(args, ctx)?;
        let targets = if input.is_none() {
            source_targets(args, ctx)?
        } else {
            record_targets(input.into_records()?, ctx)?
        };
        let mut run = Run::new(&options, targets.len());
        for target in &targets {
            ctx.checkpoint()?;
            run.file(ctx, target, if_match.as_ref())?;
        }
        run.finish(ctx)
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

/// Static facts of a call: the pattern and the template compile, and the
/// paths of `in` are listed.
fn static_info(args: &BoundArgs) -> Result<StaticInfo> {
    Options::from_args(args)?;
    let mut info = StaticInfo::default();
    match args.value(IN) {
        Some(Value::List(items)) => info
            .paths
            .extend(items.iter().filter_map(Value::as_str).map(str::to_owned)),
        Some(value) => info.paths.extend(value.as_str().map(str::to_owned)),
        None => {}
    }
    Ok(info)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Any,
    Code,
    Comments,
    Strings,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::Code => "code",
            Self::Comments => "comments",
            Self::Strings => "strings",
        }
    }

    /// Whether a match starting in a region of `class` (`None` outside
    /// comments and strings) is in the scope.
    fn accepts(self, class: Option<SpanClass>) -> bool {
        match self {
            Self::Any => true,
            Self::Code => matches!(class, None | Some(SpanClass::Code)),
            Self::Comments => matches!(class, Some(SpanClass::Comment)),
            Self::Strings => matches!(class, Some(SpanClass::String)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binary {
    Skip,
    Text,
}

/// Compiled `pattern`.
#[derive(Debug)]
enum Compiled {
    Literal(String),
    Regex(regex::Regex),
    Backtracking(fancy_regex::Regex),
    Syntax(String),
}

/// Parameters of one call.
#[derive(Debug)]
struct Options {
    pattern: Option<Compiled>,
    with: String,
    template: bool,
    occurrence: Option<NonZeroUsize>,
    expect: Option<Range>,
    on_missing: OnMissing,
    whitespace: Whitespace,
    case_sensitive: bool,
    whole_word: bool,
    multiline: bool,
    scope: Scope,
    check_references: bool,
    validate: Validate,
    binary: Binary,
}

impl Options {
    fn from_args(args: &BoundArgs) -> Result<Self> {
        let case_sensitive = args.bool(CASE_SENSITIVE)?.unwrap_or(true);
        let multiline = args.bool(MULTILINE)?.unwrap_or(false);
        let pattern = pattern_param(args, case_sensitive, multiline)?;
        let with = match args.require(WITH)? {
            Value::Text(text) => text.clone(),
            _ => return Err(type_error(COMMAND, WITH, "a text")),
        };
        let template = args.bool(TEMPLATE)?.unwrap_or(true);
        let options = Self {
            pattern,
            with,
            template,
            occurrence: occurrence_param(args)?,
            expect: range_param(args, EXPECT)?,
            on_missing: params::on_missing_param(args)?,
            whitespace: whitespace_param(args)?,
            case_sensitive,
            whole_word: args.bool(WHOLE_WORD)?.unwrap_or(false),
            multiline,
            scope: match choice(args, SCOPE, &SCOPE_VALUES)? {
                Some("code") => Scope::Code,
                Some("comments") => Scope::Comments,
                Some("strings") => Scope::Strings,
                _ => Scope::Any,
            },
            check_references: args.bool(CHECK_REFERENCES)?.unwrap_or(false),
            validate: validate::validate_param(args)?,
            binary: match choice(args, BINARY, &BINARY_VALUES)? {
                Some("text") => Binary::Text,
                _ => Binary::Skip,
            },
        };
        match &options.pattern {
            Some(Compiled::Regex(regex)) => {
                options.regex_template(Groups::from_names(regex.capture_names()))?;
            }
            Some(Compiled::Backtracking(regex)) => {
                options.regex_template(Groups::from_names(regex.capture_names()))?;
            }
            _ => {}
        }
        Ok(options)
    }

    fn regex_template(&self, groups: Groups) -> Result<Template> {
        if self.template {
            Template::parse(&self.with, &groups).map_err(|error| template_error(&error))
        } else {
            Ok(Template::literal(&self.with))
        }
    }

    /// Literal text of the pattern, for the nearest fragments of an error
    /// and for `check_references`.
    fn literal(&self) -> Option<&str> {
        match &self.pattern {
            Some(Compiled::Literal(text)) => Some(text),
            _ => None,
        }
    }
}

fn pattern_param(
    args: &BoundArgs,
    case_sensitive: bool,
    multiline: bool,
) -> Result<Option<Compiled>> {
    let mode = choice(args, MODE, &MODE_VALUES)?;
    let regex = |regex: &value::Regex| -> Result<Compiled> {
        Ok(match regex_pattern(regex, case_sensitive, multiline)? {
            Pattern::Regex(compiled) => Compiled::Regex(compiled),
            Pattern::Backtracking(compiled) => Compiled::Backtracking(compiled),
            Pattern::Literal(text) => Compiled::Literal(text),
        })
    };
    let compiled = match (args.value(PATTERN), mode) {
        (None | Some(Value::Null), _) => return Ok(None),
        (Some(Value::Text(text)), Some("regex")) => regex(&value::Regex {
            pattern: text.clone(),
            flags: RegexFlags::default(),
        })?,
        (Some(Value::Text(text)), Some("syntax")) => Compiled::Syntax(text.clone()),
        (Some(Value::Text(text)), _) => Compiled::Literal(text.clone()),
        (Some(Value::Regex(pattern)), Some("literal")) => {
            Compiled::Literal(pattern.pattern.clone())
        }
        (Some(Value::Regex(_)), Some("syntax")) => {
            return Err(invalid_value(
                PATTERN,
                "`mode: syntax` takes the pattern as text in the syntax of the language".to_owned(),
            ));
        }
        (Some(Value::Regex(pattern)), _) => regex(pattern)?,
        (Some(_), _) => {
            return Err(type_error(
                COMMAND,
                PATTERN,
                "a text or a regular expression",
            ));
        }
    };
    if let Compiled::Literal(text) | Compiled::Syntax(text) = &compiled
        && text.is_empty()
    {
        return Err(invalid_value(
            PATTERN,
            "`pattern` is empty: give the text to replace".to_owned(),
        ));
    }
    Ok(Some(compiled))
}

fn choice<'a>(args: &'a BoundArgs, name: &str, values: &[&str]) -> Result<Option<&'a str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Enum(text) | Value::Text(text)) if values.contains(&text.as_str()) => {
            Ok(Some(text.as_str()))
        }
        Some(Value::Enum(text) | Value::Text(text)) => Err(unknown_value(name, text, values)),
        Some(_) => Err(type_error(COMMAND, name, "a value of the enumeration")),
    }
}

fn text_list(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let item = |value: &Value| match value {
        Value::Glob(text) | Value::Text(text) | Value::Enum(text) => Ok(text.clone()),
        _ => Err(type_error(COMMAND, name, "a text or a list of texts")),
    };
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(item).collect(),
        Some(value) => item(value).map(|text| vec![text]),
    }
}

/// How a file was named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Listed in `in`.
    Named,
    /// Found in a directory of `in`.
    Walked,
    /// Named by an input record.
    Record,
}

/// Lines of a file that a pattern may match, from 1, inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Region {
    Whole,
    Lines(Vec<(u64, u64)>),
}

impl Region {
    fn add(&mut self, first: u64, last: u64) {
        if let Self::Lines(ranges) = self {
            ranges.push((first, last));
        }
    }

    fn covers(&self, line: u64) -> bool {
        match self {
            Self::Whole => true,
            Self::Lines(ranges) => ranges
                .iter()
                .any(|(first, last)| (*first..=*last).contains(&line)),
        }
    }

    /// Hits whose every line lies in the region.
    fn keep(&self, hits: Vec<Hit>) -> Vec<Hit> {
        if *self == Self::Whole {
            return hits;
        }
        hits.into_iter()
            .filter(|hit| (hit.line..=hit.end_line).all(|line| self.covers(count_u64(line))))
            .collect()
    }
}

/// What is replaced in a file.
#[derive(Debug)]
enum Work {
    /// Matches of the pattern inside the region.
    Pattern(Region),
    /// The input matches.
    Matches(Vec<record::Match>),
}

#[derive(Debug)]
struct Target {
    path: AbsPath,
    origin: Origin,
    etag: Option<String>,
    work: Work,
}

/// Files of the source form: files listed in `in`, and the files in the
/// directories of `in` that match `glob` and `file_type`, each once.
fn source_targets(args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Target>> {
    let roots = path_list_arg(args, IN, ctx)?;
    if roots.is_empty() {
        return Err(missing_param(COMMAND, IN));
    }
    let mut globs = text_list(args, GLOB)?;
    if globs.is_empty() {
        globs.push(DEFAULT_GLOB.to_owned());
    }
    let file_types = text_list(args, FILE_TYPE)?;
    let filters = params::walk_filters(args)?;
    let fs = ctx.require_fs()?;
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    let mut add = |path: AbsPath, origin: Origin| {
        if seen.insert(path.clone()) {
            targets.push(Target {
                path,
                origin,
                etag: None,
                work: Work::Pattern(Region::Whole),
            });
        }
    };
    for root in &roots {
        let meta = fs.metadata(root, Follow::Follow)?;
        if !matches!(meta.kind, FileKind::Directory) {
            add(root.clone(), Origin::Named);
            continue;
        }
        let mut options = WalkOptions {
            globs: globs.clone(),
            kind: KindFilter::File,
            ..WalkOptions::default()
        };
        filters.apply(&mut options);
        let mut found = Vec::new();
        Walker::new(options)?.run(
            fs,
            std::slice::from_ref(root),
            &|| ctx.checkpoint(),
            |item| {
                if let WalkItem::Entry(entry) = item {
                    found.push(entry.path);
                }
                ControlFlow::Continue(())
            },
        )?;
        for path in found {
            if has_file_type(&path, &file_types) {
                add(path, Origin::Walked);
            }
        }
    }
    Ok(targets)
}

/// Whether the extension of the file is one of `types`; every file matches
/// an empty list.
fn has_file_type(path: &AbsPath, types: &[String]) -> bool {
    if types.is_empty() {
        return true;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| {
            types
                .iter()
                .any(|kind| kind.eq_ignore_ascii_case(extension))
        })
}

/// Files of the stage forms, in the order of their first record.
fn record_targets(stream: RecordStream<'_>, ctx: &ExecCtx) -> Result<Vec<Target>> {
    let mut targets: Vec<Target> = Vec::new();
    let mut index: HashMap<AbsPath, usize> = HashMap::new();
    for item in stream {
        let record = item?;
        ctx.checkpoint()?;
        let kind = record.kind;
        let path = record_path(&record, ctx)?;
        let slot = *index.entry(path.clone()).or_insert_with(|| {
            targets.push(Target {
                path,
                origin: Origin::Record,
                etag: None,
                work: match kind {
                    RecordKind::Match => Work::Matches(Vec::new()),
                    _ => Work::Pattern(Region::Lines(Vec::new())),
                },
            });
            targets.len() - 1
        });
        let target = &mut targets[slot];
        match (kind, &mut target.work) {
            (RecordKind::Match, Work::Matches(matches)) => {
                let found = record::Match::try_from(record)?;
                target.etag.get_or_insert_with(|| found.etag.clone());
                matches.push(found);
            }
            (RecordKind::Line, Work::Pattern(region)) => {
                let line = record::Line::try_from(record)?;
                target.etag.get_or_insert_with(|| line.etag.clone());
                region.add(line.no, line.no);
            }
            (RecordKind::Content, Work::Pattern(region)) => {
                let content = record::Content::try_from(record)?;
                target.etag.get_or_insert_with(|| content.etag.clone());
                let ranges: Vec<Range> = content
                    .ranges
                    .clone()
                    .or_else(|| content.range.map(|range| vec![range]))
                    .unwrap_or_default();
                if ranges.is_empty() {
                    *region = Region::Whole;
                }
                for range in ranges {
                    let first = range.start().and_then(|line| u64::try_from(line).ok());
                    let last = range.end().and_then(|line| u64::try_from(line).ok());
                    region.add(first.unwrap_or(1), last.unwrap_or(u64::MAX));
                }
            }
            (RecordKind::Match | RecordKind::Line | RecordKind::Content, _) => {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "`{COMMAND}` received `match` and `line` or `content` records for `{}`: replace matches and lines in separate calls",
                        target.path
                    ),
                ));
            }
            (other, _) => {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "`{COMMAND}` does not accept `{other}` records: pass `match`, `line` or `content` records"
                    ),
                ));
            }
        }
    }
    Ok(targets)
}

/// A located match and its replacement; offsets in the decoded text, lines
/// and columns from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    start: usize,
    end: usize,
    line: usize,
    end_line: usize,
    column: usize,
    replacement: String,
}

/// Start offsets of the lines of a text.
struct LineStarts {
    starts: Vec<usize>,
}

impl LineStarts {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|start| *start <= offset)
    }

    fn start_of(&self, line: usize) -> Option<usize> {
        self.starts.get(line.checked_sub(1)?).copied()
    }

    /// Offset of the column, from 1 in Unicode scalar values, of the line;
    /// the column after the last character is the end of the line.
    fn offset(&self, text: &str, line: usize, column: usize) -> Option<usize> {
        let start = self.start_of(line)?;
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let wanted = column.checked_sub(1)?;
        let row = &text[start..end];
        row.char_indices()
            .map(|(at, _)| start + at)
            .chain(std::iter::once(end))
            .nth(wanted)
    }

    fn hit(&self, text: &str, span: Span<usize>, replacement: String) -> Hit {
        let line = self.line_of(span.start);
        let line_start = self.start_of(line).unwrap_or(0);
        let last = if span.end > span.start {
            span.end - 1
        } else {
            span.start
        };
        Hit {
            line,
            end_line: self.line_of(last),
            column: text[line_start..span.start].chars().count() + 1,
            start: span.start,
            end: span.end,
            replacement,
        }
    }

    fn excerpt(&self, text: &str, line: usize) -> String {
        let start = self.start_of(line).unwrap_or(0);
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        text[start..end]
            .trim()
            .chars()
            .take(EXCERPT_CHARS)
            .collect()
    }
}

/// Prevailing line ending of a text: `\r\n` when more than half of the line
/// breaks are `\r\n`, otherwise `\n`.
fn line_ending_of(text: &str) -> &'static str {
    let breaks = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if crlf * 2 > breaks { "\r\n" } else { "\n" }
}

fn to_line_ending(text: &str, ending: &str) -> String {
    let unified = text.replace("\r\n", "\n");
    if ending == "\n" {
        unified
    } else {
        unified.replace('\n', ending)
    }
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn at_word_boundaries(text: &str, span: &Span<usize>) -> bool {
    let before = text[..span.start].chars().next_back();
    let after = text[span.end..].chars().next();
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

fn expand<C: CaptureSource>(template: &Template, captures: &C, ending: &str) -> String {
    let mut out = String::new();
    template.expand_into(captures, &mut out);
    to_line_ending(&out, ending)
}

/// Matches of the pattern in a file with their replacements, in text order.
fn locate(
    ctx: &ExecCtx,
    file: &TextFile,
    pattern: &Compiled,
    options: &Options,
) -> Result<Vec<Hit>> {
    let text = file.text.as_str();
    let lines = LineStarts::new(text);
    let ending = line_ending_of(text);
    // An empty match inside a `\r\n` line ending would split the ending.
    let wanted = |span: &Span<usize>| {
        !(span.is_empty()
            && text[..span.start].ends_with('\r')
            && text[span.start..].starts_with('\n'))
            && (!options.whole_word || at_word_boundaries(text, span))
            && (options.multiline || !text[span.clone()].contains('\n'))
    };
    let mut hits = Vec::new();
    match pattern {
        Compiled::Literal(literal) => {
            let matching = MatchOptions {
                whitespace: options.whitespace,
                case_sensitive: options.case_sensitive,
                whole_word: options.whole_word,
            };
            let found = fuzzy_match::find(text, literal, &matching)
                .map_err(|error| match_error(&error, &file.path))?;
            let adapter = Adapter::new(text, literal, options.whitespace);
            for found in &found {
                let replacement = adapter.adapt(found, &options.with);
                hits.push(lines.hit(text, found.start..found.end, replacement));
            }
        }
        Compiled::Regex(regex) => {
            let template = options.regex_template(Groups::from_names(regex.capture_names()))?;
            for captures in regex.captures_iter(text) {
                let Some(whole) = captures.get(0) else {
                    continue;
                };
                let span = whole.start()..whole.end();
                if wanted(&span) {
                    let replacement = expand(&template, &captures, ending);
                    hits.push(lines.hit(text, span, replacement));
                }
            }
        }
        Compiled::Backtracking(regex) => {
            let template = options.regex_template(Groups::from_names(regex.capture_names()))?;
            for captures in regex.captures_iter(text) {
                let captures = captures.map_err(|_| backtrack_error(&file.path))?;
                let Some(whole) = captures.get(0) else {
                    continue;
                };
                let span = whole.start()..whole.end();
                if wanted(&span) {
                    let replacement = expand(&template, &captures, ending);
                    hits.push(lines.hit(text, span, replacement));
                }
            }
        }
        Compiled::Syntax(source) => {
            let (code, language) = grammar(
                ctx,
                &file.path,
                text.as_bytes(),
                "`mode: syntax`",
                "use `mode: literal` or `mode: regex`",
            )?;
            let compiled = code.compile_pattern(source, &language)?;
            for found in code.syntax_search(text.as_bytes(), compiled.as_ref())? {
                let (Ok(start), Ok(end)) = (
                    usize::try_from(found.span.bytes.start),
                    usize::try_from(found.span.bytes.end),
                ) else {
                    continue;
                };
                if start > end || text.get(start..end).is_none() {
                    continue;
                }
                let replacement =
                    to_line_ending(&code.render_template(&found, &options.with)?, ending);
                hits.push(lines.hit(text, start..end, replacement));
            }
        }
    }
    Ok(hits)
}

fn backtrack_error(path: &AbsPath) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "the regular expression exceeded the backtracking limit in `{path}`: simplify the pattern"
        ),
    )
}

/// Code service and language of a file, for `what`, which needs the syntax
/// of the file.
fn grammar<'c>(
    ctx: &'c ExecCtx,
    path: &AbsPath,
    bytes: &[u8],
    what: &str,
    fix: &str,
) -> Result<(&'c dyn CodeService, LanguageId)> {
    let unsupported = || {
        Error::new(
            ErrorCode::UnsupportedLanguage,
            format!("{what} needs a grammar for `{path}` and none applies: {fix}"),
        )
        .with_detail("path", Json::from(path.to_language_string()))
    };
    let code = ctx.code().ok_or_else(unsupported)?;
    let language = code
        .language_of(Some(path.as_path()), bytes)
        .ok_or_else(unsupported)?;
    Ok((code, language))
}

/// Hits that start in the part of the source code that `scope` selects.
fn in_scope(ctx: &ExecCtx, file: &TextFile, hits: Vec<Hit>, scope: Scope) -> Result<Vec<Hit>> {
    if scope == Scope::Any || hits.is_empty() {
        return Ok(hits);
    }
    let bytes = file.text.as_bytes();
    let (code, language) = grammar(
        ctx,
        &file.path,
        bytes,
        &format!("`scope: {}`", scope.as_str()),
        "use `scope: any`",
    )?;
    let spans = code.classify_spans(bytes, &language)?;
    Ok(hits
        .into_iter()
        .filter(|hit| {
            let offset = count_u64(hit.start);
            let class = spans
                .iter()
                .find(|span| span.bytes.contains(offset))
                .map(|span| span.class);
            scope.accepts(class)
        })
        .collect())
}

/// Groups of an input match: group 0 is the matched text, the numbered
/// groups follow, then the named groups in their order.
///
/// The `groups` of a `match` record already start with the whole match, as
/// `search` writes them, so `$n` is `groups[n]` and the whole match is
/// taken from `text`.
struct RecordCaptures<'r> {
    whole: &'r str,
    numbered: Vec<Option<&'r str>>,
    named: Vec<(&'r str, Option<&'r str>)>,
}

impl<'r> RecordCaptures<'r> {
    fn new(found: &'r record::Match) -> Self {
        Self {
            whole: &found.text,
            numbered: found
                .groups
                .iter()
                .flatten()
                .skip(1)
                .map(Option::as_deref)
                .collect(),
            named: found
                .named
                .iter()
                .flat_map(|named| named.iter())
                .map(|(name, text)| (name.as_str(), text.as_deref()))
                .collect(),
        }
    }

    fn groups(&self) -> Groups {
        Groups::from_names(
            std::iter::once(None)
                .chain(self.numbered.iter().map(|_| None))
                .chain(self.named.iter().map(|(name, _)| Some(*name))),
        )
    }
}

impl CaptureSource for RecordCaptures<'_> {
    fn group(&self, index: usize) -> Option<&str> {
        let numbered = self.numbered.len();
        match index {
            0 => Some(self.whole),
            _ if index <= numbered => self.numbered.get(index - 1).copied().flatten(),
            _ => self
                .named
                .get(index - numbered - 1)
                .and_then(|(_, text)| *text),
        }
    }
}

/// Hits of the input matches of one file, in text order.
fn input_hits(file: &TextFile, records: &[record::Match], options: &Options) -> Result<Vec<Hit>> {
    let text = file.text.as_str();
    let lines = LineStarts::new(text);
    let ending = line_ending_of(text);
    let mut hits = Vec::with_capacity(records.len());
    for found in records {
        let start = usize::try_from(found.no)
            .ok()
            .zip(usize::try_from(found.column).ok())
            .and_then(|(line, column)| lines.offset(text, line, column))
            .filter(|start| text[*start..].starts_with(found.text.as_str()))
            .ok_or_else(|| moved_match(&file.path, found))?;
        let captures = RecordCaptures::new(found);
        let template = if options.template {
            Template::parse(&options.with, &captures.groups())
                .map_err(|error| template_error(&error))?
        } else {
            Template::literal(&options.with)
        };
        let replacement = expand(&template, &captures, ending);
        hits.push(lines.hit(text, start..start + found.text.len(), replacement));
    }
    hits.sort_by_key(|hit| hit.start);
    hits.dedup_by_key(|hit| hit.start);
    Ok(hits)
}

fn moved_match(path: &AbsPath, found: &record::Match) -> Error {
    Error::new(
        ErrorCode::Stale,
        format!(
            "the match at line {} column {} is no longer in `{path}`: search the file again",
            found.no, found.column
        ),
    )
    .with_detail("path", Json::from(path.to_language_string()))
    .with_found(found.text.clone())
}

/// Replaces the hits in order and returns the new text, the spans of the
/// replacements in it and the number of replacements; a hit that overlaps
/// an earlier one is left out.
fn splice(text: &str, hits: &[Hit]) -> (String, Vec<Span<usize>>, usize) {
    let mut out = String::with_capacity(text.len());
    let mut spans = Vec::with_capacity(hits.len());
    let mut cursor = 0;
    for hit in hits {
        if hit.start < cursor {
            continue;
        }
        out.push_str(&text[cursor..hit.start]);
        let start = out.len();
        out.push_str(&hit.replacement);
        spans.push(start..out.len());
        cursor = hit.end;
    }
    out.push_str(&text[cursor..]);
    let count = spans.len();
    (out, spans, count)
}

fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && characters.all(is_word)
}

/// Lines, from 1, of the occurrences of `names` in `text` outside the
/// replacements: symbolic references of the code service when it knows the
/// language and the name is an identifier, whole words otherwise.
fn left_references(
    ctx: &ExecCtx,
    path: &AbsPath,
    text: &str,
    replaced: &[Span<usize>],
    names: &[String],
) -> Vec<usize> {
    let lines = LineStarts::new(text);
    let syntax = ctx.code().and_then(|code| {
        code.language_of(Some(path.as_path()), text.as_bytes())
            .map(|language| (code, language))
    });
    let words = MatchOptions {
        whitespace: Whitespace::Exact,
        case_sensitive: true,
        whole_word: true,
    };
    let mut found = Vec::new();
    for name in names {
        let symbolic = syntax
            .as_ref()
            .filter(|_| is_identifier(name))
            .and_then(|(code, language)| code.references(text.as_bytes(), language, name).ok());
        let starts: Vec<usize> = match symbolic {
            Some(references) => references
                .iter()
                .filter_map(|reference| usize::try_from(reference.span.bytes.start).ok())
                .collect(),
            None => fuzzy_match::find(text, name, &words)
                .map(|matches| matches.iter().map(|found| found.start).collect())
                .unwrap_or_default(),
        };
        found.extend(
            starts
                .into_iter()
                .filter(|start| !replaced.iter().any(|span| span.contains(start)))
                .map(|start| lines.line_of(start)),
        );
    }
    found.sort_unstable();
    found
}

/// Replaced texts searched for by `check_references`: the literal pattern
/// and the distinct texts of the hits, single lines only.
fn reference_names(options: &Options, text: &str, hits: &[Hit]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let candidates = options
        .literal()
        .into_iter()
        .chain(hits.iter().map(|hit| &text[hit.start..hit.end]));
    for candidate in candidates {
        let name = candidate.trim();
        if names.len() < NAMES_MAX
            && !name.is_empty()
            && !name.contains('\n')
            && !names.iter().any(|known| known == name)
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// State of one call while its files are planned.
struct Run<'o> {
    options: &'o Options,
    targets: usize,
    found: usize,
    replaced: usize,
    files: u64,
    /// Path of the file read last; the path a call with one target names
    /// when nothing matched.
    last_read: Option<AbsPath>,
    binary_skipped: u64,
    candidates: Vec<String>,
    references: Vec<String>,
    references_count: u64,
    nearest: Vec<String>,
    edits: Vec<FileEdit>,
    notable: Vec<NotableEntry>,
}

impl<'o> Run<'o> {
    fn new(options: &'o Options, targets: usize) -> Self {
        Self {
            options,
            targets,
            found: 0,
            replaced: 0,
            files: 0,
            last_read: None,
            binary_skipped: 0,
            candidates: Vec::new(),
            references: Vec::new(),
            references_count: 0,
            nearest: Vec::new(),
            edits: Vec::new(),
            notable: Vec::new(),
        }
    }

    /// Reads a target as text; `None` for a binary file that is skipped.
    fn read(&mut self, ctx: &ExecCtx, target: &Target) -> Result<Option<TextFile>> {
        match read_text(ctx, &target.path, EncodingParam::Auto) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.code() == ErrorCode::Binary => {
                match (self.options.binary, target.origin) {
                    (Binary::Text, _) => {
                        read_text(ctx, &target.path, EncodingParam::Codec(Codec::Windows1252))
                            .map(Some)
                    }
                    (Binary::Skip, Origin::Named) => {
                        self.notable
                            .push(at_path(Notable::BinaryTarget, &target.path));
                        Ok(None)
                    }
                    (Binary::Skip, Origin::Walked) => {
                        self.binary_skipped += 1;
                        Ok(None)
                    }
                    (Binary::Skip, Origin::Record) => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn file(&mut self, ctx: &ExecCtx, target: &Target, if_match: Option<&IfMatch>) -> Result<()> {
        let Some(file) = self.read(ctx, target)? else {
            return Ok(());
        };
        self.files += 1;
        self.last_read = Some(file.path.clone());
        if let Some(if_match) = if_match {
            check_if_match(
                &target.path,
                &Observed::Present(file.meta.clone()),
                if_match.expected(&target.path),
            )?;
        }
        if let Some(etag) = &target.etag {
            check_record_etag(&file, etag)?;
        }
        let hits = match &target.work {
            Work::Pattern(region) => {
                let pattern = self
                    .options
                    .pattern
                    .as_ref()
                    .ok_or_else(|| missing_param(COMMAND, PATTERN))?;
                let hits = region.keep(locate(ctx, &file, pattern, self.options)?);
                in_scope(ctx, &file, hits, self.options.scope)?
            }
            Work::Matches(records) => input_hits(&file, records, self.options)?,
        };
        self.found += hits.len();
        if self.options.on_missing == OnMissing::Error
            && self.nearest.is_empty()
            && hits.is_empty()
            && let Some(literal) = self.options.literal()
        {
            self.nearest = fuzzy_match::nearest(&file.text, literal)
                .iter()
                .map(|excerpt| format!("{}:{}: {}", file.path, excerpt.line, excerpt.text))
                .collect();
        }
        let lines = LineStarts::new(&file.text);
        for hit in hits
            .iter()
            .take(CANDIDATES_SHOWN.saturating_sub(self.candidates.len()))
        {
            self.candidates.push(format!(
                "{}:{}:{}: {}",
                file.path,
                hit.line,
                hit.column,
                lines.excerpt(&file.text, hit.line)
            ));
        }
        let selected: Vec<Hit> = match self.options.occurrence {
            None => hits,
            Some(occurrence) => hits
                .into_iter()
                .skip(occurrence.get() - 1)
                .take(1)
                .collect(),
        };
        let names = if self.options.check_references {
            reference_names(self.options, &file.text, &selected)
        } else {
            Vec::new()
        };
        if selected.is_empty() {
            self.count_references(ctx, &file.path, &file.text, &[], &names);
            return Ok(());
        }
        let (text, spans, count) = splice(&file.text, &selected);
        self.replaced += count;
        self.count_references(ctx, &file.path, &text, &spans, &names);
        self.edits.push(FileEdit {
            file,
            text,
            replacements: Some(count_u64(count)),
        });
        Ok(())
    }

    fn count_references(
        &mut self,
        ctx: &ExecCtx,
        path: &AbsPath,
        text: &str,
        replaced: &[Span<usize>],
        names: &[String],
    ) {
        if names.is_empty() {
            return;
        }
        let lines = left_references(ctx, path, text, replaced, names);
        self.references_count += count_u64(lines.len());
        for line in lines {
            if self.references.len() < REFERENCES_SHOWN {
                self.references.push(format!("{path}:{line}"));
            }
        }
    }

    fn finish(mut self, ctx: &ExecCtx) -> Result<EffectPlan> {
        let options = self.options;
        if self.found == 0 && options.on_missing == OnMissing::Error {
            return Err(self.no_match());
        }
        if let Some(expected) = options.expect
            && !expected.contains(i64::try_from(self.found).unwrap_or(i64::MAX))
        {
            let mut error = Error::new(
                ErrorCode::Assert,
                format!(
                    "the pattern occurs {} times, expected {expected}: extend the pattern, set `{OCCURRENCE}` or change `{EXPECT}`",
                    self.found
                ),
            )
            .with_expected(Json::from(expected.to_string()))
            .with_actual(Json::from(self.found));
            for candidate in &self.candidates {
                error = error.with_sample(candidate.clone());
            }
            return Err(error);
        }
        if self.replaced == 0 {
            if options.on_missing == OnMissing::Error {
                return Err(self.occurrence_missing());
            }
            self.notable.push(match &self.last_read {
                Some(path) if self.targets == 1 => at_path(Notable::NothingMatched, path),
                _ => counted(Notable::NothingMatched, self.files),
            });
        }
        if self.binary_skipped > 0 {
            self.notable
                .push(counted(Notable::BinarySkipped, self.binary_skipped));
        }
        if self.references_count > 0 {
            self.notable.push(NotableEntry {
                count: Some(self.references_count),
                detail: Some(self.references.join(", ")),
                ..NotableEntry::new(Notable::ReferencesLeft)
            });
        }
        effect_plan(ctx, COMMAND, options.validate, self.edits, self.notable)
    }

    fn no_match(&self) -> Error {
        let message = if self.targets == 0 {
            format!(
                "nothing reached `{COMMAND}`: the input is empty or `in` holds no matching files"
            )
        } else {
            format!(
                "the pattern does not occur in {} of `{IN}`: read the current text and give an existing fragment",
                files_text(self.files)
            )
        };
        let mut error = Error::new(ErrorCode::NoMatch, message).with_actual(Json::from(0));
        for nearest in &self.nearest {
            error = error.with_sample(nearest.clone());
        }
        error
    }

    fn occurrence_missing(&self) -> Error {
        let occurrence = self.options.occurrence.map_or(1, NonZeroUsize::get);
        let mut error = Error::new(
            ErrorCode::OutOfRange,
            format!(
                "occurrence {occurrence} does not exist in any file, the pattern occurs {} times: give a smaller `{OCCURRENCE}`",
                self.found
            ),
        )
        .with_expected(Json::from(occurrence))
        .with_actual(Json::from(self.found));
        for candidate in &self.candidates {
            error = error.with_sample(candidate.clone());
        }
        error
    }
}

fn files_text(count: u64) -> String {
    if count == 1 {
        "the file".to_owned()
    } else {
        format!("the {count} files")
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::path::Path;

    use nurt_contracts::journal::JournalStore;
    use nurt_contracts::services::{
        ByteRange, ClassifiedSpan, FunctionMetrics, LanguageInfo, ReferenceRole, Services,
        SymbolInfo, SymbolReference, SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken,
        TextPosition, TextSpan,
    };
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Record;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::fixtures::begin_request;

    use super::*;
    use crate::commands::edit::fixture::{abs, with_file};

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    fn args(pattern: Value, with: &str, extra: &[(&str, Value)]) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND)
            .with(PATTERN, ArgValue::Value(pattern), false)
            .with(WITH, ArgValue::Value(text(with)), false);
        for (name, value) in extra {
            args.insert(*name, ArgValue::Value(value.clone()), false);
        }
        args
    }

    fn in_paths(paths: &[&AbsPath]) -> (&'static str, Value) {
        (
            IN,
            Value::List(
                paths
                    .iter()
                    .map(|path| Value::Path(path.to_language_string()))
                    .collect(),
            ),
        )
    }

    fn regex(pattern: &str) -> Value {
        Value::Regex(value::Regex::new(pattern, "").unwrap())
    }

    fn plan(test: &TestContext, args: &BoundArgs, input: StageInput<'_>) -> Result<EffectPlan> {
        let ctx = test.ctx();
        Replace.plan(args, input, &ctx)
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<EffectOutcome<'static>> {
        let ctx = test.ctx();
        let plan = Replace.plan(args, StageInput::none(), &ctx)?;
        let mut tx = test.journal().begin(begin_request(COMMAND));
        let outcome = apply_edits(plan, tx.as_mut(), &ctx)?;
        tx.commit()?;
        Ok(EffectOutcome {
            output: nurt_contracts::stage::StageOutput::none(),
            changes: outcome.changes,
            notable: outcome.notable,
            irreversible: outcome.irreversible,
            detached: outcome.detached,
        })
    }

    /// Code service for `.rs` files: `//` comments to the end of the line,
    /// `"…"` strings, references and structural matches of whole words.
    struct WordCode;

    #[derive(Debug)]
    struct WordPattern {
        language: LanguageId,
        source: String,
    }

    impl SyntaxPattern for WordPattern {
        fn language(&self) -> &LanguageId {
            &self.language
        }

        fn source(&self) -> &str {
            &self.source
        }

        fn metavariables(&self) -> Vec<String> {
            Vec::new()
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn span(start: usize, end: usize) -> TextSpan {
        let position = TextPosition { line: 1, column: 1 };
        TextSpan {
            bytes: ByteRange::new(start as u64, end as u64),
            start: position,
            end: position,
        }
    }

    fn words(text: &str, name: &str) -> Vec<usize> {
        fuzzy_match::find(
            text,
            name,
            &MatchOptions {
                whitespace: Whitespace::Exact,
                case_sensitive: true,
                whole_word: true,
            },
        )
        .unwrap()
        .iter()
        .map(|found| found.start)
        .collect()
    }

    fn unsupported<T>() -> Result<T> {
        Err(Error::new(
            ErrorCode::UnsupportedLanguage,
            "not in the test grammar",
        ))
    }

    impl CodeService for WordCode {
        fn languages(&self) -> Vec<LanguageInfo> {
            Vec::new()
        }

        fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
            path.filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
                .and_then(|_| LanguageId::new("rust").ok())
        }

        fn outline(&self, _: &[u8], _: &LanguageId, _: Option<u32>) -> Result<Vec<SymbolInfo>> {
            unsupported()
        }

        fn find_symbol(&self, _: &[u8], _: &LanguageId, _: &str) -> Result<Vec<SymbolInfo>> {
            unsupported()
        }

        fn enclosing(&self, _: &[u8], _: &LanguageId, _: u64) -> Result<Option<SymbolInfo>> {
            unsupported()
        }

        fn classify_spans(&self, bytes: &[u8], _: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
            let text = std::str::from_utf8(bytes).unwrap_or_default();
            let mut spans = Vec::new();
            let mut at = 0;
            while at < text.len() {
                let rest = &text[at..];
                if rest.starts_with("//") {
                    let end = rest.find('\n').map_or(text.len(), |offset| at + offset);
                    spans.push(ClassifiedSpan {
                        bytes: ByteRange::new(at as u64, end as u64),
                        class: SpanClass::Comment,
                    });
                    at = end;
                } else if let Some(quoted) = rest.strip_prefix('"') {
                    let end = quoted
                        .find('"')
                        .map_or(text.len(), |offset| at + offset + 2);
                    spans.push(ClassifiedSpan {
                        bytes: ByteRange::new(at as u64, end as u64),
                        class: SpanClass::String,
                    });
                    at = end;
                } else {
                    at += rest.chars().next().map_or(1, char::len_utf8);
                }
            }
            Ok(spans)
        }

        fn check_syntax(&self, _: &[u8], _: &LanguageId) -> Result<Vec<SyntaxIssue>> {
            Ok(Vec::new())
        }

        fn compile_pattern(
            &self,
            pattern: &str,
            language: &LanguageId,
        ) -> Result<Box<dyn SyntaxPattern>> {
            Ok(Box::new(WordPattern {
                language: language.clone(),
                source: pattern.to_owned(),
            }))
        }

        fn syntax_search(
            &self,
            bytes: &[u8],
            pattern: &dyn SyntaxPattern,
        ) -> Result<Vec<SyntaxMatch>> {
            let text = std::str::from_utf8(bytes).unwrap_or_default();
            Ok(words(text, pattern.source())
                .into_iter()
                .map(|start| SyntaxMatch {
                    span: span(start, start + pattern.source().len()),
                    node_kind: "identifier".to_owned(),
                    text: pattern.source().to_owned(),
                    captures: Vec::new(),
                })
                .collect())
        }

        fn render_template(&self, found: &SyntaxMatch, template: &str) -> Result<String> {
            Ok(template.replace("$X", &found.text))
        }

        fn references(
            &self,
            bytes: &[u8],
            _: &LanguageId,
            name: &str,
        ) -> Result<Vec<SymbolReference>> {
            let text = std::str::from_utf8(bytes).unwrap_or_default();
            Ok(words(text, name)
                .into_iter()
                .map(|start| SymbolReference {
                    span: span(start, start + name.len()),
                    role: ReferenceRole::Usage,
                    class: SpanClass::Code,
                    container: None,
                    line_text: String::new(),
                })
                .collect())
        }

        fn function_metrics(&self, _: &[u8], _: &LanguageId) -> Result<Vec<FunctionMetrics>> {
            unsupported()
        }

        fn tokens(&self, _: &[u8], _: &LanguageId) -> Result<Vec<SyntaxToken>> {
            unsupported()
        }
    }

    fn with_grammar() -> TestContext {
        TestContext::new().with_services(Services::new().with_code(Arc::new(WordCode)))
    }

    #[test]
    fn literal_replacement_keeps_the_style_of_the_file() {
        let (test, path) = with_file("/data/a.txt", b"fn main() {\r\n\tlet a = 1;\r\n}\r\n");
        let given = args(
            text("    let a = 1;"),
            "    let a = 2;\n    let b = 3;",
            &[in_paths(&[&path])],
        );
        let outcome = run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"fn main() {\r\n\tlet a = 2;\r\n\tlet b = 3;\r\n}\r\n"
        );
        assert_eq!(outcome.changes[0].replacements, Some(1));
    }

    #[test]
    fn regular_expressions_expand_groups_and_respect_lines() {
        let (test, path) = with_file("/data/a.txt", b"a=1\nb=22\nc=x\n");
        let given = args(
            regex(r"(?<key>\w)=(\d+)"),
            "${key}:$2$$",
            &[in_paths(&[&path])],
        );
        run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"a:1$\nb:22$\nc=x\n");

        let (test, path) = with_file("/data/b.txt", b"x\ny\n");
        let single = args(regex(r"x\sy"), "z", &[in_paths(&[&path])]);
        let plan = plan(&test, &single, StageInput::none()).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.notable[0].code, Notable::NothingMatched);
        let across = args(
            regex(r"x\sy"),
            "z",
            &[in_paths(&[&path]), (MULTILINE, Value::Bool(true))],
        );
        run(&test, &across).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"z\n");
    }

    #[test]
    fn literal_mode_and_template_false_insert_text_as_written() {
        let (test, path) = with_file("/data/a.txt", b"a.b a+b\n");
        let literal = args(
            regex("a.b"),
            "$1",
            &[
                in_paths(&[&path]),
                (MODE, Value::Enum("literal".to_owned())),
            ],
        );
        run(&test, &literal).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"$1 a+b\n");
        let plain = args(
            text(r"a\+b"),
            "$0",
            &[
                in_paths(&[&path]),
                (MODE, Value::Enum("regex".to_owned())),
                (TEMPLATE, Value::Bool(false)),
            ],
        );
        run(&test, &plain).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"$1 $0\n");
    }

    #[test]
    fn invalid_templates_and_patterns_fail_before_reading() {
        let broken_template = args(regex(r"(\w)"), "$2", &[]);
        let error = Options::from_args(&broken_template).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Regex);
        let broken_regex = args(regex("a("), "b", &[]);
        assert_eq!(
            Options::from_args(&broken_regex).unwrap_err().code(),
            ErrorCode::Regex
        );
        let empty = args(text(""), "b", &[]);
        assert_eq!(
            Options::from_args(&empty).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let syntax_regex = args(regex("a"), "b", &[(MODE, Value::Enum("syntax".to_owned()))]);
        assert_eq!(
            Options::from_args(&syntax_regex).unwrap_err().code(),
            ErrorCode::InvalidValue
        );
        let wrong = args(Value::Int(1), "b", &[]);
        assert_eq!(
            Options::from_args(&wrong).unwrap_err().code(),
            ErrorCode::Type
        );
        let unknown = args(text("a"), "b", &[(SCOPE, Value::Enum("docs".to_owned()))]);
        assert_eq!(
            Options::from_args(&unknown).unwrap_err().code(),
            ErrorCode::UnknownValue
        );
        let no_with = BoundArgs::new(COMMAND).with(PATTERN, ArgValue::Value(text("a")), false);
        assert_eq!(
            Options::from_args(&no_with).unwrap_err().code(),
            ErrorCode::MissingParam
        );
    }

    #[test]
    fn occurrence_selects_one_match_in_every_file() {
        let (test, first) = with_file("/data/a.txt", b"x x x\n");
        let second = abs("/data/b.txt");
        test.fs().write_file(&second, b"x\n").unwrap();
        let given = args(
            text("x"),
            "y",
            &[in_paths(&[&first, &second]), (OCCURRENCE, Value::Int(2))],
        );
        let outcome = run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&first).unwrap(), b"x y x\n");
        assert_eq!(test.fs().read_file(&second).unwrap(), b"x\n");
        assert_eq!(outcome.changes.len(), 1);
    }

    #[test]
    fn expect_counts_matches_of_all_files() {
        let (test, first) = with_file("/data/a.txt", b"x\n");
        let second = abs("/data/b.txt");
        test.fs().write_file(&second, b"x x\n").unwrap();
        let given = args(
            text("x"),
            "y",
            &[in_paths(&[&first, &second]), (EXPECT, Value::Int(2))],
        );
        let error = plan(&test, &given, StageInput::none()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Assert);
        assert_eq!(error.actual, Some(Json::from(3)));
        assert_eq!(error.sample.len(), 3);
        let exact = args(
            text("x"),
            "y",
            &[
                in_paths(&[&first, &second]),
                (EXPECT, Value::Range(Range::new(Some(3), None).unwrap())),
            ],
        );
        run(&test, &exact).unwrap();
        assert_eq!(test.fs().read_file(&second).unwrap(), b"y y\n");
    }

    #[test]
    fn on_missing_error_reports_the_nearest_fragments() {
        let (test, path) = with_file("/data/a.txt", b"let total = price * count;\n");
        let given = args(
            text("let total = price * amount;"),
            "x",
            &[
                in_paths(&[&path]),
                (params::ON_MISSING, Value::Enum("error".to_owned())),
            ],
        );
        let error = plan(&test, &given, StageInput::none()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NoMatch);
        assert_eq!(error.sample.len(), 1);
        assert!(error.sample[0].ends_with(":1: let total = price * count;"));
        let beyond = args(
            text("total"),
            "x",
            &[
                in_paths(&[&path]),
                (OCCURRENCE, Value::Int(2)),
                (params::ON_MISSING, Value::Enum("error".to_owned())),
            ],
        );
        let error = plan(&test, &beyond, StageInput::none()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        assert!(error.message.contains("occurrence 2"));
        assert_eq!(error.expected, Some(Json::from(2)));
        assert_eq!(error.actual, Some(Json::from(1)));
    }

    #[test]
    fn directories_are_walked_with_glob_and_file_type() {
        let test = TestContext::new();
        for (path, content) in [
            ("/src/a.rs", "old\n"),
            ("/src/b.txt", "old\n"),
            ("/src/deep/c.rs", "old\n"),
            ("/src/.hidden.rs", "old\n"),
        ] {
            test.fs()
                .write_file(&abs(path), content.as_bytes())
                .unwrap();
        }
        test.fs()
            .write_file(&abs("/src/blob.rs"), b"old\x00\x01\x02")
            .unwrap();
        let root = abs("/src");
        let given = args(
            text("old"),
            "new",
            &[
                in_paths(&[&root]),
                (FILE_TYPE, Value::List(vec![text("RS")])),
                (params::HIDDEN, Value::Bool(false)),
            ],
        );
        let outcome = run(&test, &given).unwrap();
        assert_eq!(outcome.changes.len(), 2);
        assert_eq!(test.fs().read_file(&abs("/src/a.rs")).unwrap(), b"new\n");
        assert_eq!(
            test.fs().read_file(&abs("/src/deep/c.rs")).unwrap(),
            b"new\n"
        );
        assert_eq!(test.fs().read_file(&abs("/src/b.txt")).unwrap(), b"old\n");
        assert_eq!(
            test.fs().read_file(&abs("/src/.hidden.rs")).unwrap(),
            b"old\n"
        );
        let globbed = args(
            text("old"),
            "new",
            &[in_paths(&[&root]), (GLOB, Value::Glob("*.txt".to_owned()))],
        );
        let plan = plan(&test, &globbed, StageInput::none()).unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert!(plan.changes[0].path.as_str().ends_with("b.txt"));
    }

    #[test]
    fn binary_files_are_skipped_or_treated_as_text() {
        let (test, path) = with_file("/data/blob.bin", b"\x7fELF\x00old\x00");
        let given = args(text("old"), "new", &[in_paths(&[&path])]);
        let plan = plan(&test, &given, StageInput::none()).unwrap();
        assert!(plan.changes.is_empty());
        assert!(
            plan.notable
                .iter()
                .any(|entry| entry.code == Notable::BinaryTarget)
        );
        let as_text = args(
            text("old"),
            "new",
            &[in_paths(&[&path]), (BINARY, Value::Enum("text".to_owned()))],
        );
        run(&test, &as_text).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"\x7fELF\x00new\x00");
    }

    #[test]
    fn scope_limits_matches_to_comments_strings_or_code() {
        let test = with_grammar();
        let path = abs("/src/main.rs");
        let source = "let old = 1; // old value\nprint(\"old\");\n";
        for (scope, expected) in [
            ("code", "let new = 1; // old value\nprint(\"old\");\n"),
            ("comments", "let old = 1; // new value\nprint(\"old\");\n"),
            ("strings", "let old = 1; // old value\nprint(\"new\");\n"),
        ] {
            test.fs().write_file(&path, source.as_bytes()).unwrap();
            let given = args(
                text("old"),
                "new",
                &[in_paths(&[&path]), (SCOPE, Value::Enum(scope.to_owned()))],
            );
            run(&test, &given).unwrap();
            assert_eq!(
                test.fs().read_file(&path).unwrap(),
                expected.as_bytes(),
                "{scope}"
            );
        }
        let plain = abs("/src/notes.txt");
        test.fs().write_file(&plain, b"old\n").unwrap();
        let unknown = args(
            text("old"),
            "new",
            &[in_paths(&[&plain]), (SCOPE, Value::Enum("code".to_owned()))],
        );
        let error = plan(&test, &unknown, StageInput::none()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    }

    #[test]
    fn check_references_reports_occurrences_left() {
        let test = with_grammar();
        let path = abs("/src/main.rs");
        test.fs()
            .write_file(&path, b"fn old() {}\nold();\n// old\n")
            .unwrap();
        let given = args(
            text("old"),
            "new",
            &[
                in_paths(&[&path]),
                (OCCURRENCE, Value::Int(1)),
                (CHECK_REFERENCES, Value::Bool(true)),
            ],
        );
        let plan = plan(&test, &given, StageInput::none()).unwrap();
        let left = plan
            .notable
            .iter()
            .find(|entry| entry.code == Notable::ReferencesLeft)
            .unwrap();
        assert_eq!(left.count, Some(2));
        assert_eq!(
            left.detail.as_deref(),
            Some(format!("{path}:2, {path}:3").as_str())
        );

        let (plain, text_path) = with_file("/data/a.txt", b"old old\n");
        let textual = args(
            text("old"),
            "older",
            &[
                in_paths(&[&text_path]),
                (CHECK_REFERENCES, Value::Bool(true)),
            ],
        );
        let plan = self::plan(&plain, &textual, StageInput::none()).unwrap();
        assert!(
            plan.notable
                .iter()
                .all(|entry| entry.code != Notable::ReferencesLeft)
        );
    }

    #[test]
    fn syntax_mode_replaces_structural_matches() {
        let test = with_grammar();
        let path = abs("/src/main.rs");
        test.fs()
            .write_file(&path, b"call(x);\nrecall(x);\n")
            .unwrap();
        let given = args(
            text("call"),
            "wrapped($X)",
            &[in_paths(&[&path]), (MODE, Value::Enum("syntax".to_owned()))],
        );
        run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"wrapped(call)(x);\nrecall(x);\n"
        );
        let (bare, other) = with_file("/data/a.txt", b"call\n");
        let unsupported = args(
            text("call"),
            "x",
            &[
                in_paths(&[&other]),
                (MODE, Value::Enum("syntax".to_owned())),
            ],
        );
        let error = self::plan(&bare, &unsupported, StageInput::none()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    }

    fn line_record(path: &AbsPath, no: u64, line: &str) -> Record {
        Record::from(record::Line {
            path: value::Path::new(path.to_language_string()),
            no,
            text: line.to_owned(),
            ..record::Line::default()
        })
    }

    #[test]
    fn line_records_limit_the_replacement_to_their_lines() {
        let (test, path) = with_file("/data/a.txt", b"a\na\na\n");
        let given = args(text("a"), "b", &[]);
        let records = vec![line_record(&path, 2, "a"), line_record(&path, 3, "a")];
        let ctx = test.ctx();
        let plan = Replace
            .plan(
                &given,
                StageInput::records(RecordStream::from_records(records)),
                &ctx,
            )
            .unwrap();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        apply_edits(plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"a\nb\nb\n");
    }

    #[test]
    fn content_records_with_a_range_limit_the_replacement() {
        let (test, path) = with_file("/data/a.txt", b"a\na\na\na\n");
        let record = Record::from(record::Content {
            path: value::Path::new(path.to_language_string()),
            range: Some(Range::new(Some(2), Some(3)).unwrap()),
            ..record::Content::default()
        });
        let given = args(text("a"), "b", &[]);
        let plan = plan(
            &test,
            &given,
            StageInput::records(RecordStream::once(record)),
        )
        .unwrap();
        assert_eq!(plan.changes[0].replacements, Some(2));
        let whole = Record::from(record::Content {
            path: value::Path::new(path.to_language_string()),
            ..record::Content::default()
        });
        let plan = self::plan(
            &test,
            &given,
            StageInput::records(RecordStream::once(whole)),
        )
        .unwrap();
        assert_eq!(plan.changes[0].replacements, Some(4));
    }

    #[test]
    fn match_records_are_replaced_with_their_groups() {
        let (test, path) = with_file("/data/a.txt", b"key=1\nname=ann\n");
        let found = record::Match {
            path: value::Path::new(path.to_language_string()),
            no: 2,
            column: 1,
            start: 6,
            end: 14,
            text: "name=ann".to_owned(),
            groups: Some(vec![
                Some("name=ann".to_owned()),
                Some("name".to_owned()),
                None,
            ]),
            named: Some(
                [("value".to_owned(), Some("ann".to_owned()))]
                    .into_iter()
                    .collect(),
            ),
            ..record::Match::default()
        };
        let given =
            BoundArgs::new(COMMAND).with(WITH, ArgValue::Value(text("$1:${value}$2")), false);
        let plan = plan(
            &test,
            &given,
            StageInput::records(RecordStream::once(Record::from(found.clone()))),
        )
        .unwrap();
        let ctx = test.ctx();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        apply_edits(plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"key=1\nname:ann\n");

        let moved = record::Match {
            text: "other".to_owned(),
            ..found
        };
        let error = self::plan(
            &test,
            &given,
            StageInput::records(RecordStream::once(Record::from(moved))),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
    }

    #[test]
    fn records_of_other_kinds_are_rejected() {
        let (test, path) = with_file("/data/a.txt", b"a\n");
        let entry =
            Record::new(RecordKind::Row).with("path", Value::Path(path.to_language_string()));
        let given = args(text("a"), "b", &[]);
        let error = plan(
            &test,
            &given,
            StageInput::records(RecordStream::once(entry)),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
        let mixed = vec![
            line_record(&path, 1, "a"),
            Record::from(record::Match {
                path: value::Path::new(path.to_language_string()),
                no: 1,
                column: 1,
                text: "a".to_owned(),
                ..record::Match::default()
            }),
        ];
        let error = plan(
            &test,
            &given,
            StageInput::records(RecordStream::from_records(mixed)),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }

    #[test]
    fn line_positions_map_to_offsets() {
        let text = "ab\nc\u{105}d\n";
        let lines = LineStarts::new(text);
        assert_eq!(lines.offset(text, 1, 1), Some(0));
        assert_eq!(lines.offset(text, 2, 2), Some(4));
        assert_eq!(lines.offset(text, 2, 3), Some(6));
        assert_eq!(lines.offset(text, 2, 4), Some(7));
        assert_eq!(lines.offset(text, 2, 5), None);
        assert_eq!(lines.offset(text, 9, 1), None);
        assert_eq!(lines.offset(text, 1, 0), None);
        assert_eq!(lines.line_of(3), 2);
        let hit = lines.hit(text, 0..4, String::new());
        assert_eq!((hit.line, hit.end_line, hit.column), (1, 2, 1));
    }

    #[test]
    fn helpers_follow_the_text_style() {
        assert_eq!(line_ending_of("a\r\nb\r\nc\n"), "\r\n");
        assert_eq!(line_ending_of("a\nb\r\n"), "\n");
        assert_eq!(to_line_ending("a\nb\r\n", "\r\n"), "a\r\nb\r\n");
        assert!(at_word_boundaries("a foo b", &(2..5)));
        assert!(!at_word_boundaries("afoo b", &(1..4)));
        assert!(is_identifier("old_name"));
        assert!(!is_identifier("old name"));
        assert!(!is_identifier("1st"));
        assert!(has_file_type(&abs("/a/b.Go"), &["go".to_owned()]));
        assert!(!has_file_type(&abs("/a/Makefile"), &["go".to_owned()]));
        assert!(has_file_type(&abs("/a/Makefile"), &[]));
        let (out, spans, count) = splice(
            "abcdef",
            &[
                Hit {
                    start: 1,
                    end: 3,
                    line: 1,
                    end_line: 1,
                    column: 2,
                    replacement: "X".to_owned(),
                },
                Hit {
                    start: 2,
                    end: 4,
                    line: 1,
                    end_line: 1,
                    column: 3,
                    replacement: "Y".to_owned(),
                },
            ],
        );
        assert_eq!((out.as_str(), count), ("aXdef", 1));
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0], 1..2);
    }

    #[test]
    fn static_check_reports_paths_and_rejects_bad_patterns() {
        let given = args(text("a"), "b", &[(IN, Value::Path("/data".to_owned()))]);
        assert_eq!(static_info(&given).unwrap().paths, ["/data"]);
        let listed = args(
            text("a"),
            "b",
            &[(IN, Value::List(vec![text("/x"), text("/y")]))],
        );
        assert_eq!(static_info(&listed).unwrap().paths, ["/x", "/y"]);
        let broken = args(regex("("), "b", &[]);
        assert_eq!(static_info(&broken).unwrap_err().code(), ErrorCode::Regex);
    }

    #[test]
    fn nearest_fragments_are_collected_only_when_a_missing_pattern_is_an_error() {
        let (test, path) = with_file("/data/a.txt", b"let total = price * count;\n");
        let ctx = test.ctx();
        let target = Target {
            path: path.clone(),
            origin: Origin::Named,
            etag: None,
            work: Work::Pattern(Region::Whole),
        };
        let skipping = args(
            text("let total = price * amount;"),
            "x",
            &[in_paths(&[&path])],
        );
        let options = Options::from_args(&skipping).unwrap();
        let mut run = Run::new(&options, 1);
        run.file(&ctx, &target, None).unwrap();
        assert!(run.nearest.is_empty());

        let failing = args(
            text("let total = price * amount;"),
            "x",
            &[
                in_paths(&[&path]),
                (params::ON_MISSING, Value::Enum("error".to_owned())),
            ],
        );
        let options = Options::from_args(&failing).unwrap();
        let mut run = Run::new(&options, 1);
        run.file(&ctx, &target, None).unwrap();
        assert_eq!(run.nearest.len(), 1);
    }

    #[test]
    fn line_anchors_match_before_carriage_returns_and_replacements_keep_crlf() {
        let (test, path) = with_file("/data/a.txt", b"foo\r\nbar foo\r\n");
        let given = args(regex("foo$"), "baz\nqux", &[in_paths(&[&path])]);
        run(&test, &given).unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            b"baz\r\nqux\r\nbar baz\r\nqux\r\n"
        );
    }

    #[test]
    fn empty_regular_expression_matches_are_replaced_at_every_position() {
        let (test, path) = with_file("/data/a.txt", b"ab");
        let given = args(regex("x*"), "-", &[in_paths(&[&path])]);
        let outcome = run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"-a-b-");
        assert_eq!(outcome.changes[0].replacements, Some(3));
    }

    #[test]
    fn an_empty_match_never_splits_a_crlf_line_ending() {
        let (test, path) = with_file("/data/a.txt", b"a\r\nb\r\n");
        let given = args(regex("x*"), "-", &[in_paths(&[&path])]);
        run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"-a-\r\n-b-\r\n-");
    }

    #[test]
    fn a_search_without_a_match_in_one_file_names_the_file() {
        let (test, path) = with_file("/data/a.txt", b"alpha\n");
        let given = args(text("missing"), "x", &[in_paths(&[&path])]);
        let plan = plan(&test, &given, StageInput::none()).unwrap();
        assert_eq!(plan.notable[0].code, Notable::NothingMatched);
        assert_eq!(
            plan.notable[0].path.as_deref(),
            Some(path.to_language_string().as_str())
        );
        assert_eq!(plan.notable[0].count, None);
    }

    #[test]
    fn literal_replacements_insert_dollar_signs_and_backslashes_as_written() {
        let (test, path) = with_file("/data/a.txt", b"a\n");
        let given = args(text("a"), r"$1\d$$", &[in_paths(&[&path])]);
        run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"$1\\d$$\n");
    }

    #[test]
    fn look_ahead_patterns_use_the_backtracking_engine() {
        let (test, path) = with_file("/data/a.txt", b"foobar foobaz\n");
        let given = args(regex("foo(?=bar)"), "X", &[in_paths(&[&path])]);
        run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"Xbar foobaz\n");
    }

    #[test]
    fn whole_word_and_case_insensitive_options_apply_to_regular_expressions() {
        let (test, path) = with_file("/data/a.txt", b"Cat concat cat\n");
        let given = args(
            regex("cat"),
            "dog",
            &[
                in_paths(&[&path]),
                (WHOLE_WORD, Value::Bool(true)),
                (CASE_SENSITIVE, Value::Bool(false)),
            ],
        );
        run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"dog concat dog\n");
    }

    #[test]
    fn if_match_with_another_tag_fails_with_stale_before_any_change() {
        let (test, path) = with_file("/data/a.txt", b"old\n");
        let ctx = test.ctx();
        let file = read_text(&ctx, &path, EncodingParam::Auto).unwrap();
        let mut tag = file.etag();
        tag.size += 1;
        let given = args(
            text("old"),
            "new",
            &[
                in_paths(&[&path]),
                (
                    params::IF_MATCH,
                    Value::Text(crate::common::etag::etag_text(&tag)),
                ),
            ],
        );
        let error = plan(&test, &given, StageInput::none()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Stale);
        assert_eq!(test.fs().read_file(&path).unwrap(), b"old\n");
    }

    #[test]
    fn replacement_keeps_the_byte_order_mark_and_a_missing_final_line_break() {
        let (test, path) = with_file("/data/a.txt", b"\xef\xbb\xbfold line");
        let given = args(text("old"), "new", &[in_paths(&[&path])]);
        run(&test, &given).unwrap();
        assert_eq!(test.fs().read_file(&path).unwrap(), b"\xef\xbb\xbfnew line");
    }

    #[test]
    fn match_records_locate_columns_in_characters_not_bytes() {
        let (test, path) = with_file("/data/a.txt", "\u{17c}\u{f3}\u{142}w x\n".as_bytes());
        let found = record::Match {
            path: value::Path::new(path.to_language_string()),
            no: 1,
            column: 6,
            start: 8,
            end: 9,
            text: "x".to_owned(),
            ..record::Match::default()
        };
        let given = BoundArgs::new(COMMAND).with(WITH, ArgValue::Value(text("y")), false);
        let plan = plan(
            &test,
            &given,
            StageInput::records(RecordStream::once(Record::from(found))),
        )
        .unwrap();
        let ctx = test.ctx();
        let mut tx = test.journal().begin(begin_request(COMMAND));
        apply_edits(plan, tx.as_mut(), &ctx).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            test.fs().read_file(&path).unwrap(),
            "\u{17c}\u{f3}\u{142}w y\n".as_bytes()
        );
    }
}
