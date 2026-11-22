// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The stages `parse` and `split`.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::Row;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, StageDescriptor, StageImpl, StageInput,
    StageOutput, StaticInfo,
};
use nurt_contracts::value::{Fields, Path, Record, RecordKind, Regex, Value};

use super::common::{
    Description, NUMBER_FIELD, PATH_FIELD, Parts, descriptor, invalid_value, kind_error,
    known_value, missing_param, type_error, unknown_field,
};

const PATTERN: &str = "pattern";
const DELIMITER: &str = "delimiter";
const NAMES: &str = "names";
const TEXT_FIELD: &str = "text";
const LINE_FIELD: &str = "line";

/// Compiles a regular expression value with its flags.
fn compile(args: &BoundArgs, name: &str, regex: &Regex) -> Result<regex::Regex> {
    regex::RegexBuilder::new(&regex.pattern)
        .case_insensitive(regex.flags.case_insensitive)
        .multi_line(regex.flags.multi_line)
        .dot_matches_new_line(regex.flags.dot_matches_new_line)
        .ignore_whitespace(regex.flags.extended)
        .build()
        .map_err(|error| {
            let reason = error.to_string();
            let reason = reason.lines().last().unwrap_or_default().trim().to_owned();
            Error::new(
                ErrorCode::Regex,
                format!(
                    "`{}` cannot compile `{name}`: {}",
                    args.command(),
                    reason.trim_start_matches("error: ")
                ),
            )
            .with_found(regex.to_string())
        })
}

/// Text of a record read by `parse` and `split`, with its path and number.
struct Source<'r> {
    path: Path,
    no: u64,
    text: &'r str,
}

fn number_of(record: &Record) -> u64 {
    record
        .get(NUMBER_FIELD)
        .and_then(Value::as_int)
        .and_then(|number| u64::try_from(number).ok())
        .unwrap_or_default()
}

fn source<'r>(record: &'r Record, text_field: &str) -> Result<Source<'r>> {
    let text = record
        .get(text_field)
        .and_then(Value::as_str)
        .ok_or_else(|| unknown_field(record, text_field))?;
    let path = record
        .get(PATH_FIELD)
        .and_then(Value::as_str)
        .map(Path::from)
        .unwrap_or_default();
    Ok(Source {
        path,
        no: number_of(record),
        text,
    })
}

fn row(path: Path, no: u64, cells: Fields) -> Record {
    Record::from(Row {
        path,
        no,
        cells,
        ..Row::default()
    })
}

const PARSE: &str = "parse";
const PARSE_KINDS: [&str; 3] = ["line", "match", "content"];

/// Compiled pattern of `parse` with the names of its groups.
struct Pattern {
    regex: regex::Regex,
    names: Vec<String>,
}

impl Pattern {
    fn of(args: &BoundArgs) -> Result<Self> {
        let regex = match known_value(args, PATTERN) {
            None => return Err(missing_param(args, PATTERN)),
            Some(Value::Regex(regex)) => compile(args, PATTERN, regex)?,
            Some(Value::Text(text)) => compile(
                args,
                PATTERN,
                &Regex::new(text, "").map_err(|error| invalid_value(error.to_string(), text))?,
            )?,
            Some(other) => return Err(type_error(args, PATTERN, &["regex"], other.type_name())),
        };
        let names: Vec<String> = regex.capture_names().flatten().map(str::to_owned).collect();
        if names.is_empty() {
            return Err(invalid_value(
                "`parse` needs named groups in `pattern`, written as (?<name>...)",
                regex.as_str(),
            ));
        }
        Ok(Self { regex, names })
    }

    fn cells(&self, captures: &regex::Captures<'_>) -> Fields {
        self.names
            .iter()
            .map(|name| {
                let value = captures
                    .name(name)
                    .map_or(Value::Null, |found| Value::from(found.as_str()));
                (name.clone(), value)
            })
            .collect()
    }

    /// Rows of one record: one per match for content, the first match for a
    /// line or a search match; nothing when the pattern does not match.
    fn rows(&self, record: &Record) -> Result<Vec<Record>> {
        match record.kind {
            RecordKind::Line => Ok(self.first_row(source(record, TEXT_FIELD)?)),
            RecordKind::Match => Ok(self.first_row(source(record, LINE_FIELD)?)),
            RecordKind::Content => {
                let source = source(record, TEXT_FIELD)?;
                Ok(self
                    .regex
                    .captures_iter(source.text)
                    .map(|captures| {
                        let start = captures.get(0).map_or(0, |found| found.start());
                        let line = source.text.get(..start).unwrap_or_default().matches('\n');
                        let no = u64::try_from(line.count())
                            .unwrap_or(u64::MAX)
                            .saturating_add(1);
                        row(source.path.clone(), no, self.cells(&captures))
                    })
                    .collect())
            }
            _ => Err(kind_error(PARSE, record, &PARSE_KINDS)),
        }
    }

    fn first_row(&self, source: Source<'_>) -> Vec<Record> {
        self.regex
            .captures(source.text)
            .map(|captures| row(source.path, source.no, self.cells(&captures)))
            .into_iter()
            .collect()
    }
}

/// `parse pattern`: extracts fields from text with the named groups of a
/// regular expression.
///
/// Each named group becomes a cell of a `row` record, as text, or `null`
/// when the group did not take part in the match. A `line` record gives one
/// row for the first match in its `text`; a `match` record one row for the
/// first match in its `line`; a `content` record one row per match, numbered
/// by the line where the match starts. Records the pattern does not match
/// give no row. The row keeps the `path` of its input record. A pattern
/// without named groups fails with `E_INVALID_VALUE`, one that does not
/// compile with `E_REGEX`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Parse;

impl StageImpl for Parse {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(
            PARSE,
            InputKinds::Only(vec![
                RecordKind::Line,
                RecordKind::Match,
                RecordKind::Content,
            ]),
        )
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Pattern::of(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Parse {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let pattern = Pattern::of(args)?;
        let parts = Parts::of(input, ctx)?;
        let records = parts.records.flat_map(move |item| {
            match item.and_then(|record| pattern.rows(&record)) {
                Ok(rows) => rows.into_iter().map(Ok).collect::<Vec<_>>(),
                Err(error) => vec![Err(error)],
            }
        });
        Ok(Description {
            header: parts.header,
            ..Description::default()
        }
        .stream(super::common::fused(records)))
    }
}

const SPLIT: &str = "split";
const COLUMN_PREFIX: &str = "c";

/// Delimiter of `split`.
enum Delimiter {
    Text(String),
    Pattern(regex::Regex),
}

impl Delimiter {
    fn of(args: &BoundArgs) -> Result<Self> {
        match known_value(args, DELIMITER) {
            None => Err(missing_param(args, DELIMITER)),
            Some(Value::Text(text)) if text.is_empty() => Err(invalid_value(
                "`split` needs a delimiter of at least one character",
                "\"\"",
            )),
            Some(Value::Text(text)) => Ok(Self::Text(text.clone())),
            Some(Value::Regex(regex)) => compile(args, DELIMITER, regex).map(Self::Pattern),
            Some(other) => Err(type_error(
                args,
                DELIMITER,
                &["text", "regex"],
                other.type_name(),
            )),
        }
    }

    fn split<'t>(&self, text: &'t str) -> Vec<&'t str> {
        match self {
            Self::Text(delimiter) => text.split(delimiter.as_str()).collect(),
            Self::Pattern(regex) => regex.split(text).collect(),
        }
    }
}

fn column_names(args: &BoundArgs) -> Result<Vec<String>> {
    let name = |item: &Value| match item {
        Value::Text(text) => Ok(text.clone()),
        other => Err(type_error(args, NAMES, &["list<text>"], other.type_name())),
    };
    match known_value(args, NAMES) {
        None => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(name).collect(),
        Some(other) => name(other).map(|text| vec![text]),
    }
}

fn split_row(delimiter: &Delimiter, names: &[String], record: &Record) -> Result<Record> {
    if record.kind != RecordKind::Line {
        return Err(kind_error(SPLIT, record, &["line"]));
    }
    let source = source(record, TEXT_FIELD)?;
    let parts = delimiter.split(source.text);
    let width = parts.len().max(names.len());
    let cells: Fields = (0..width)
        .map(|index| {
            let name = names
                .get(index)
                .cloned()
                .unwrap_or_else(|| format!("{COLUMN_PREFIX}{}", index + 1));
            let value = parts
                .get(index)
                .map_or(Value::Null, |part| Value::from(*part));
            (name, value)
        })
        .collect();
    Ok(row(source.path, source.no, cells))
}

/// `split delimiter names`: splits `line` records into the cells of `row`
/// records at a delimiter.
///
/// The delimiter is a text, matched literally, or a regular expression.
/// Cells are text and are named by `names` in order; cells beyond the names
/// are named `c<n>` by their position from 1, and names beyond the cells
/// get `null`. Without `names` the cells are `c1`, `c2`, … The row keeps the
/// `path` and the line number of its input record.
#[derive(Debug, Clone, Copy, Default)]
pub struct Split;

impl StageImpl for Split {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(SPLIT, InputKinds::Only(vec![RecordKind::Line]))
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        Delimiter::of(args)?;
        column_names(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Split {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let delimiter = Delimiter::of(args)?;
        let names = column_names(args)?;
        let parts = Parts::of(input, ctx)?;
        let records = parts
            .records
            .map(move |item| item.and_then(|record| split_row(&delimiter, &names, &record)));
        Ok(Description {
            header: parts.header,
            ..Description::default()
        }
        .stream(super::common::fused(records)))
    }
}
