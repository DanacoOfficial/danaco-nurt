// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selectors of `read`: line ranges, the last lines, the lines around the
//! matches of a pattern, Markdown sections, code symbols, document keys and
//! notebook cells.
//!
//! A line ends at a line feed; a carriage return before the line feed
//! belongs to the line ending. `search` counts lines the same way, so the
//! line numbers of its matches select the same lines here. Lines count from
//! 1 and ranges include both bounds.

use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::{Fields, Range, Value};
use nurt_notebook::CellRef;
use nurt_text::search::{Pattern, SearchOptions, Searcher};

/// Name of the `path` parameter and of the path key of a target.
pub(crate) const PATH: &str = "path";
/// Name of the `lines` parameter.
pub(crate) const LINES: &str = "lines";
/// Name of the `last` parameter.
pub(crate) const LAST: &str = "last";
/// Name of the `around` parameter.
pub(crate) const AROUND: &str = "around";
/// Name of the `context` parameter.
pub(crate) const CONTEXT: &str = "context";
/// Name of the `symbol` parameter.
pub(crate) const SYMBOL: &str = "symbol";
/// Name of the `section` parameter.
pub(crate) const SECTION: &str = "section";
/// Name of the `cell` parameter.
pub(crate) const CELL: &str = "cell";
/// Name of the `key` parameter.
pub(crate) const KEY: &str = "key";

/// Lines of context around the matches of `around` without `context`.
pub(crate) const DEFAULT_CONTEXT: u64 = 3;

/// Separator of the headings of a nested Markdown section.
const SECTION_SEPARATOR: &str = " > ";

/// Largest number of headings listed when a section is not found.
const MAX_LISTED_HEADINGS: usize = 20;

/// Deepest ATX heading level.
const MAX_HEADING_LEVEL: usize = 6;

/// Indentation up to which a line can still open a heading or a fence.
const MAX_BLOCK_INDENT: usize = 3;

/// Length of the shortest code fence.
const MIN_FENCE: usize = 3;

type Parse = fn(&str, &str, &Value) -> Result<Selector>;

/// Selector parameters, in the order of the catalog, with their parsers.
const SELECTORS: [(&str, Parse); 7] = [
    (LINES, parse_lines),
    (LAST, parse_last),
    (AROUND, parse_around),
    (SYMBOL, parse_symbol),
    (SECTION, parse_section),
    (CELL, parse_cell),
    (KEY, parse_key),
];

/// Keys of a map of `targets`.
const TARGET_KEYS: [&str; 7] = [PATH, LINES, AROUND, SYMBOL, KEY, SECTION, CELL];

/// What `read` takes from a file.
#[derive(Debug, Clone)]
pub(crate) enum Selector {
    /// The whole file.
    Whole,
    /// Line ranges in the order given.
    Lines(Vec<Range>),
    /// The last lines.
    Last(u64),
    /// The lines around the matches of a pattern.
    Around(Arc<Searcher>),
    /// A Markdown section, by its heading or by nested headings joined with
    /// ` > `.
    Section(String),
    /// The definition of a code symbol.
    Symbol(String),
    /// Values at a key path of a structured document.
    Key(KeyPath),
    /// A notebook cell.
    Cell(CellRef),
}

impl Selector {
    /// Selector given with the parameters of the call.
    ///
    /// # Errors
    ///
    /// Fails with `E_PARAM_CONFLICT` for more than one selector and with
    /// the errors of the selector values: `E_TYPE`, `E_INVALID_VALUE`,
    /// `E_REGEX` and `E_KEY_SYNTAX`.
    pub(crate) fn from_args(args: &BoundArgs) -> Result<Self> {
        let given: Vec<(&str, Parse, &Value)> = SELECTORS
            .iter()
            .filter_map(|&(name, parse)| match args.value(name) {
                None | Some(Value::Null) => None,
                Some(value) => Some((name, parse, value)),
            })
            .collect();
        from_given(args.command(), &given)
    }

    /// Path and selector of one map of `targets`.
    ///
    /// # Errors
    ///
    /// Fails with `E_UNKNOWN_FIELD` for a key other than `path`, `lines`,
    /// `around`, `symbol`, `key`, `section` and `cell`, with the code
    /// `E_MISSING_PARAM` without `path`, and as [`Selector::from_args`].
    pub(crate) fn from_target<'a>(command: &str, fields: &'a Fields) -> Result<(&'a Value, Self)> {
        if let Some(unknown) = fields
            .keys()
            .find(|key| !TARGET_KEYS.contains(&key.as_str()))
        {
            return Err(Error::new(
                ErrorCode::UnknownField,
                format!("a target of `{command}` has no key `{unknown}`"),
            )
            .with_found(unknown.as_str())
            .with_expected(serde_json::json!(TARGET_KEYS)));
        }
        let path = match fields.get(PATH) {
            None | Some(Value::Null) => {
                return Err(Error::new(
                    ErrorCode::MissingParam,
                    format!("every target of `{command}` needs a `path`"),
                ));
            }
            Some(path) => path,
        };
        let given: Vec<(&str, Parse, &Value)> = SELECTORS
            .iter()
            .filter_map(|&(name, parse)| match fields.get(name) {
                None | Some(Value::Null) => None,
                Some(value) => Some((name, parse, value)),
            })
            .collect();
        Ok((path, from_given(command, &given)?))
    }

    /// Returns `true` for the selectors that take lines of the text: `lines`,
    /// `last`, `around` and `section`.
    pub(crate) fn selects_lines(&self) -> bool {
        matches!(
            self,
            Self::Lines(_) | Self::Last(_) | Self::Around(_) | Self::Section(_)
        )
    }
}

fn from_given(command: &str, given: &[(&str, Parse, &Value)]) -> Result<Selector> {
    match given {
        [] => Ok(Selector::Whole),
        [(name, parse, value)] => parse(command, name, value),
        [(first, ..), (second, ..), ..] => Err(Error::new(
            ErrorCode::ParamConflict,
            format!(
                "`{command}` takes one selector, not both `{first}` and `{second}`: read once per selector or give `targets`"
            ),
        )),
    }
}

fn parse_lines(_command: &str, _name: &str, value: &Value) -> Result<Selector> {
    let ranges = match value {
        Value::List(items) if items.is_empty() => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "`lines` needs at least one range",
            ));
        }
        Value::List(items) => items.iter().map(line_range).collect::<Result<Vec<_>>>()?,
        other => vec![line_range(other)?],
    };
    Ok(Selector::Lines(ranges))
}

/// One line range: a range, a line number or a range written as text.
fn line_range(value: &Value) -> Result<Range> {
    let range = match value {
        Value::Range(range) => *range,
        Value::Int(line) => Range::single(*line),
        Value::Text(text) => parse_range_text(text)?,
        other => {
            return Err(Error::new(
                ErrorCode::Type,
                format!(
                    "`lines` expects a range such as `1..5`, not a {}",
                    other.type_name()
                ),
            ));
        }
    };
    for bound in [range.start(), range.end()].into_iter().flatten() {
        if bound < 1 {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "line numbers start at 1, not {bound}: use `last` for the lines at the end"
                ),
            )
            .with_found(range.to_string()));
        }
    }
    Ok(range)
}

fn parse_range_text(text: &str) -> Result<Range> {
    let written = text.trim();
    let invalid = || {
        Error::new(
            ErrorCode::InvalidValue,
            format!("`{written}` is not a line range: write `5`, `1..5`, `40..` or `..9`"),
        )
        .with_found(written)
    };
    if written == "all" {
        return Ok(Range::full());
    }
    let bound = |part: &str| -> Result<Option<i64>> {
        let part = part.trim();
        if part.is_empty() {
            Ok(None)
        } else {
            part.parse().map(Some).map_err(|_| invalid())
        }
    };
    match written.split_once("..") {
        Some((start, end)) => Range::new(bound(start)?, bound(end)?).map_err(|_| invalid()),
        None => bound(written)?.map(Range::single).ok_or_else(invalid),
    }
}

fn parse_last(command: &str, name: &str, value: &Value) -> Result<Selector> {
    match value {
        Value::Int(count) if *count >= 1 => Ok(Selector::Last(count.unsigned_abs())),
        Value::Int(count) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` must be at least 1, not {count}"),
        )),
        _ => Err(type_error(command, name, "an int")),
    }
}

fn parse_around(command: &str, name: &str, value: &Value) -> Result<Selector> {
    let pattern = match value {
        Value::Text(text) if text.is_empty() => {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{name}` needs a text or a regular expression, not an empty text"),
            ));
        }
        Value::Text(text) => Pattern::literal(text.clone()),
        Value::Regex(regex) => Pattern::regex_with_flags(regex.pattern.clone(), regex.flags),
        _ => return Err(type_error(command, name, "a text or a regular expression")),
    };
    let searcher = Searcher::new(&[pattern], SearchOptions::new().max_line(0))
        .map_err(|error| Error::new(error.code(), error.to_string()))?;
    Ok(Selector::Around(Arc::new(searcher)))
}

fn parse_symbol(command: &str, name: &str, value: &Value) -> Result<Selector> {
    required_text(command, name, value).map(Selector::Symbol)
}

fn parse_section(command: &str, name: &str, value: &Value) -> Result<Selector> {
    required_text(command, name, value).map(Selector::Section)
}

fn parse_key(command: &str, name: &str, value: &Value) -> Result<Selector> {
    match value {
        Value::Key(text) | Value::Text(text) => {
            KeyPath::parse(text).map(Selector::Key).map_err(Error::from)
        }
        _ => Err(type_error(command, name, "a key path")),
    }
}

fn parse_cell(command: &str, name: &str, value: &Value) -> Result<Selector> {
    match value {
        Value::Int(index) if *index >= 0 => Ok(Selector::Cell(CellRef::Index(*index))),
        Value::Int(index) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("cells count from 0, not {index}"),
        )),
        Value::Text(id) if !id.is_empty() => Ok(Selector::Cell(CellRef::Id(id.clone()))),
        _ => Err(type_error(command, name, "a cell index or identifier")),
    }
}

fn required_text(command: &str, name: &str, value: &Value) -> Result<String> {
    match value {
        Value::Text(text) if !text.trim().is_empty() => Ok(text.clone()),
        Value::Text(_) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` cannot be empty"),
        )),
        _ => Err(type_error(command, name, "a text")),
    }
}

fn type_error(command: &str, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{command}` expects {expected} for `{name}`"),
    )
}

/// Consecutive lines `first..=last`, from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineSpan {
    /// First line.
    pub(crate) first: u64,
    /// Last line, not before `first`.
    pub(crate) last: u64,
}

impl LineSpan {
    /// Lines `first..=last`.
    pub(crate) const fn new(first: u64, last: u64) -> Self {
        Self { first, last }
    }

    /// Number of lines.
    pub(crate) const fn len(self) -> u64 {
        self.last - self.first + 1
    }

    /// The lines as a range value.
    pub(crate) fn range(self) -> Range {
        let bound = |line: u64| i64::try_from(line).unwrap_or(i64::MAX);
        Range::new(Some(bound(self.first)), Some(bound(self.last)))
            .unwrap_or_else(|_| Range::single(bound(self.first)))
    }

    const fn overlaps(self, other: Self) -> bool {
        self.first <= other.last && other.first <= self.last
    }

    fn union(self, other: Self) -> Self {
        Self::new(self.first.min(other.first), self.last.max(other.last))
    }
}

impl fmt::Display for LineSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.first, self.last)
    }
}

#[derive(Debug, Clone, Copy)]
struct Bounds {
    start: usize,
    content_end: usize,
    end: usize,
}

/// Lines of a text with their byte offsets.
#[derive(Debug)]
pub(crate) struct Lines<'t> {
    text: &'t str,
    bounds: Vec<Bounds>,
}

impl<'t> Lines<'t> {
    /// Splits `text` at line feeds.
    pub(crate) fn new(text: &'t str) -> Self {
        let mut bounds = Vec::new();
        let mut start = 0;
        for (feed, _) in text.match_indices('\n') {
            let content_end = if text[start..feed].ends_with('\r') {
                feed - 1
            } else {
                feed
            };
            bounds.push(Bounds {
                start,
                content_end,
                end: feed + 1,
            });
            start = feed + 1;
        }
        if start < text.len() {
            bounds.push(Bounds {
                start,
                content_end: text.len(),
                end: text.len(),
            });
        }
        Self { text, bounds }
    }

    /// Number of lines; a text without a final line feed still ends with a
    /// line.
    pub(crate) fn total(&self) -> u64 {
        u64::try_from(self.bounds.len()).unwrap_or(u64::MAX)
    }

    fn bounds(&self, line: u64) -> Option<Bounds> {
        let index = usize::try_from(line.checked_sub(1)?).ok()?;
        self.bounds.get(index).copied()
    }

    /// Line `line` without its line ending; empty outside the text.
    pub(crate) fn content(&self, line: u64) -> &'t str {
        self.bounds(line)
            .map_or("", |bounds| &self.text[bounds.start..bounds.content_end])
    }

    /// Line ending of line `line`; empty for the last line of a text without
    /// a final line feed.
    pub(crate) fn ending(&self, line: u64) -> &'t str {
        self.bounds(line)
            .map_or("", |bounds| &self.text[bounds.content_end..bounds.end])
    }

    /// Text of the lines of `span` with their line endings.
    pub(crate) fn slice(&self, span: LineSpan) -> &'t str {
        match (self.bounds(span.first), self.bounds(span.last)) {
            (Some(first), Some(last)) => &self.text[first.start..last.end],
            _ => "",
        }
    }

    /// The whole text as a span; `None` for an empty text.
    pub(crate) fn whole(&self) -> Option<LineSpan> {
        (self.total() > 0).then(|| LineSpan::new(1, self.total()))
    }
}

/// Lines of `ranges` that exist in the text, in the order given; ranges
/// that overlap are merged into the place of the first of them.
pub(crate) fn range_spans(lines: &Lines<'_>, ranges: &[Range]) -> Vec<LineSpan> {
    let total = lines.total();
    let clipped = ranges.iter().filter_map(|range| {
        let first = range.start().map_or(1, |start| start.unsigned_abs());
        let last = range
            .end()
            .map_or(total, |end| end.unsigned_abs().min(total));
        (first <= last).then(|| LineSpan::new(first, last))
    });
    merge_overlapping(clipped)
}

fn merge_overlapping(spans: impl IntoIterator<Item = LineSpan>) -> Vec<LineSpan> {
    let mut merged: Vec<LineSpan> = Vec::new();
    for mut span in spans {
        let mut place = None;
        let mut index = 0;
        while index < merged.len() {
            if merged[index].overlaps(span) {
                span = span.union(merged.remove(index));
                place.get_or_insert(index);
            } else {
                index += 1;
            }
        }
        merged.insert(place.unwrap_or(merged.len()), span);
    }
    merged
}

/// The last `count` lines; the whole text when it is shorter.
pub(crate) fn last_spans(lines: &Lines<'_>, count: u64) -> Vec<LineSpan> {
    let total = lines.total();
    lines
        .whole()
        .map(|whole| LineSpan::new(total.saturating_sub(count).saturating_add(1), whole.last))
        .into_iter()
        .collect()
}

/// The lines of every match of `searcher` with `context` lines before and
/// after; spans that overlap or touch are merged.
pub(crate) fn around_spans(lines: &Lines<'_>, searcher: &Searcher, context: u64) -> Vec<LineSpan> {
    let total = lines.total();
    let mut spans: Vec<LineSpan> = Vec::new();
    searcher.search_slice(lines.text.as_bytes(), |hit| {
        let span = LineSpan::new(
            hit.line().saturating_sub(context).max(1),
            hit.end_line().saturating_add(context).min(total),
        );
        match spans.last_mut() {
            Some(previous) if span.first <= previous.last.saturating_add(1) => {
                previous.last = previous.last.max(span.last);
            }
            _ => spans.push(span),
        }
        ControlFlow::Continue(())
    });
    spans
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Heading {
    line: u64,
    level: usize,
    title: String,
}

/// Lines of the Markdown section `path` of the text of `file`: the heading
/// and the lines up to the next heading of the same or a higher level.
///
/// The path is a heading, or headings of nested sections joined with
/// ` > `. Headings are compared without emphasis, code and link markers; an
/// exact match is preferred over one that ignores letter case.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for an empty heading in the path and with
/// the code `E_KEY_NOT_FOUND`, listing the headings available, when no
/// heading matches.
pub(crate) fn section_span(lines: &Lines<'_>, path: &str, file: &str) -> Result<LineSpan> {
    let headings = headings(lines);
    let mut scope = LineSpan::new(1, lines.total());
    let mut parent: Option<&Heading> = None;
    for part in path.split(SECTION_SEPARATOR) {
        let wanted = plain_heading(part);
        if wanted.is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!("`{path}` has an empty heading: separate headings with `>`"),
            ));
        }
        let candidates: Vec<&Heading> = headings
            .iter()
            .filter(|heading| {
                heading.line >= scope.first
                    && heading.line <= scope.last
                    && parent.is_none_or(|parent| {
                        heading.line > parent.line && heading.level > parent.level
                    })
            })
            .collect();
        let found = candidates
            .iter()
            .find(|heading| heading.title == wanted)
            .or_else(|| {
                candidates
                    .iter()
                    .find(|heading| heading.title.to_lowercase() == wanted.to_lowercase())
            })
            .copied()
            .ok_or_else(|| section_missing(file, part, &candidates))?;
        scope = section_of(found, &headings, lines.total());
        parent = Some(found);
    }
    Ok(scope)
}

fn section_missing(file: &str, part: &str, candidates: &[&Heading]) -> Error {
    let titles: Vec<&str> = candidates
        .iter()
        .take(MAX_LISTED_HEADINGS)
        .map(|heading| heading.title.as_str())
        .collect();
    Error::new(
        ErrorCode::KeyNotFound,
        format!("`{file}` has no Markdown section `{part}`: use a heading listed in `expected`"),
    )
    .with_found(part)
    .with_expected(serde_json::json!(titles))
}

fn section_of(heading: &Heading, headings: &[Heading], total: u64) -> LineSpan {
    let end = headings
        .iter()
        .find(|next| next.line > heading.line && next.level <= heading.level)
        .map_or(total, |next| next.line - 1);
    LineSpan::new(heading.line, end)
}

fn headings(lines: &Lines<'_>) -> Vec<Heading> {
    let mut found = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut underline_of_previous = false;
    for line in 1..=lines.total() {
        let text = lines.content(line);
        if let Some(open) = fence {
            if closes_fence(text, open) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = opens_fence(text) {
            fence = Some(open);
            continue;
        }
        if underline_of_previous {
            underline_of_previous = false;
            continue;
        }
        if let Some((level, title)) = atx_heading(text) {
            found.push(Heading { line, level, title });
        } else if !text.trim().is_empty()
            && let Some(level) = setext_level(lines.content(line + 1))
        {
            found.push(Heading {
                line,
                level,
                title: plain_heading(text),
            });
            underline_of_previous = true;
        }
    }
    found
}

fn block_text(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    (line.len() - rest.len() <= MAX_BLOCK_INDENT).then_some(rest)
}

fn opens_fence(line: &str) -> Option<(char, usize)> {
    let rest = block_text(line)?;
    let marker = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let length = rest.chars().take_while(|c| *c == marker).count();
    (length >= MIN_FENCE).then_some((marker, length))
}

fn closes_fence(line: &str, (marker, length): (char, usize)) -> bool {
    block_text(line).is_some_and(|rest| {
        let run = rest.chars().take_while(|c| *c == marker).count();
        run >= length && rest[run * marker.len_utf8()..].trim().is_empty()
    })
}

fn atx_heading(line: &str) -> Option<(usize, String)> {
    let rest = block_text(line)?;
    let level = rest.bytes().take_while(|byte| *byte == b'#').count();
    if level == 0 || level > MAX_HEADING_LEVEL {
        return None;
    }
    let after = &rest[level..];
    if !(after.is_empty() || after.starts_with([' ', '\t'])) {
        return None;
    }
    let mut title = after.trim();
    let open = title.trim_end_matches('#');
    if open.len() < title.len() && (open.is_empty() || open.ends_with([' ', '\t'])) {
        title = open.trim_end();
    }
    Some((level, plain_heading(title)))
}

fn setext_level(line: &str) -> Option<usize> {
    let rest = block_text(line)?.trim_end();
    if !rest.is_empty() && rest.bytes().all(|byte| byte == b'=') {
        Some(1)
    } else if !rest.is_empty() && rest.bytes().all(|byte| byte == b'-') {
        Some(2)
    } else {
        None
    }
}

/// Heading text without emphasis, code and strike-through markers and with
/// links and images reduced to their labels; white space is collapsed.
fn plain_heading(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '*' | '_' | '`' | '~' | '[' => {}
            '!' if chars.peek() == Some(&'[') => {}
            ']' if chars.peek() == Some(&'(') => {
                for skipped in chars.by_ref() {
                    if skipped == ')' {
                        break;
                    }
                }
            }
            ']' => {}
            other => plain.push(other),
        }
    }
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Regex;

    use super::*;

    fn args(name: &str, value: Value) -> BoundArgs {
        BoundArgs::new("read").with(name, ArgValue::Value(value), false)
    }

    fn range(start: i64, end: i64) -> Range {
        Range::new(Some(start), Some(end)).unwrap()
    }

    fn spans(list: &[(u64, u64)]) -> Vec<LineSpan> {
        list.iter()
            .map(|&(first, last)| LineSpan::new(first, last))
            .collect()
    }

    fn numbered(count: u64) -> String {
        (1..=count).map(|line| format!("line {line}\n")).collect()
    }

    fn around(lines: &Lines<'_>, value: Value, context: u64) -> Vec<LineSpan> {
        match Selector::from_args(&args(AROUND, value)).unwrap() {
            Selector::Around(searcher) => around_spans(lines, &searcher, context),
            other => panic!("unexpected selector {other:?}"),
        }
    }

    #[test]
    fn lines_split_at_line_feeds_and_keep_endings() {
        let lines = Lines::new("a\r\nb\n\nc");
        assert_eq!(lines.total(), 4);
        assert_eq!(lines.content(1), "a");
        assert_eq!(lines.ending(1), "\r\n");
        assert_eq!(lines.content(3), "");
        assert_eq!(lines.content(4), "c");
        assert_eq!(lines.ending(4), "");
        assert_eq!(lines.slice(LineSpan::new(2, 4)), "b\n\nc");
        assert_eq!(lines.content(9), "");
    }

    #[test]
    fn empty_text_has_no_lines() {
        let lines = Lines::new("");
        assert_eq!(lines.total(), 0);
        assert_eq!(lines.whole(), None);
        assert!(range_spans(&lines, &[range(1, 5)]).is_empty());
        assert!(last_spans(&lines, 3).is_empty());
    }

    #[test]
    fn final_line_without_line_feed_counts() {
        assert_eq!(Lines::new("a\nb").total(), 2);
        assert_eq!(Lines::new("a\nb\n").total(), 2);
        assert_eq!(Lines::new("\n").total(), 1);
    }

    #[test]
    fn ranges_are_clipped_to_the_text() {
        let text = numbered(10);
        let lines = Lines::new(&text);
        assert_eq!(range_spans(&lines, &[range(8, 20)]), spans(&[(8, 10)]));
        assert!(range_spans(&lines, &[range(11, 20)]).is_empty());
        assert_eq!(
            range_spans(&lines, &[Range::from_start(9)]),
            spans(&[(9, 10)])
        );
        assert_eq!(range_spans(&lines, &[Range::up_to(2)]), spans(&[(1, 2)]));
        assert_eq!(range_spans(&lines, &[Range::full()]), spans(&[(1, 10)]));
    }

    #[test]
    fn overlapping_ranges_merge_in_the_order_given() {
        let text = numbered(30);
        let lines = Lines::new(&text);
        let merged = range_spans(&lines, &[range(20, 25), range(1, 3), range(22, 28)]);
        assert_eq!(merged, spans(&[(20, 28), (1, 3)]));
        let chained = range_spans(&lines, &[range(1, 3), range(10, 12), range(2, 11)]);
        assert_eq!(chained, spans(&[(1, 12)]));
        let touching = range_spans(&lines, &[range(1, 3), range(4, 5)]);
        assert_eq!(touching, spans(&[(1, 3), (4, 5)]));
    }

    #[test]
    fn last_lines_cover_the_whole_text_when_it_is_shorter() {
        let text = numbered(5);
        let lines = Lines::new(&text);
        assert_eq!(last_spans(&lines, 2), spans(&[(4, 5)]));
        assert_eq!(last_spans(&lines, 50), spans(&[(1, 5)]));
    }

    #[test]
    fn matches_get_context_and_close_spans_merge() {
        let text = "a\nhit\nb\nc\nd\ne\nf\ng\nh\nhit\ni\nhit\nj\n";
        let lines = Lines::new(text);
        let found = around(&lines, Value::Text("hit".to_owned()), 1);
        assert_eq!(found, spans(&[(1, 3), (9, 13)]));
        let separate = around(&lines, Value::Text("hit".to_owned()), 0);
        assert_eq!(separate, spans(&[(2, 2), (10, 10), (12, 12)]));
        let wide = around(&lines, Value::Text("hit".to_owned()), 3);
        assert_eq!(wide, spans(&[(1, 5), (7, 13)]));
    }

    #[test]
    fn regular_expressions_select_matches() {
        let text = "fn a() {}\nlet x = 1;\nfn b() {}\n";
        let lines = Lines::new(text);
        let regex = Value::Regex(Regex::new(r"^fn b", "").unwrap());
        assert_eq!(around(&lines, regex, 0), spans(&[(3, 3)]));
        assert!(around(&lines, Value::Text("absent".to_owned()), 2).is_empty());
    }

    #[test]
    fn invalid_regular_expressions_are_rejected() {
        let regex = Value::Regex(Regex::new("a(b", "").unwrap());
        let error = Selector::from_args(&args(AROUND, regex)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Regex);
    }

    #[test]
    fn line_values_accept_ranges_numbers_and_texts() {
        let read = |value: Value| match Selector::from_args(&args(LINES, value)).unwrap() {
            Selector::Lines(ranges) => ranges,
            other => panic!("unexpected selector {other:?}"),
        };
        assert_eq!(read(Value::Int(4)), [Range::single(4)]);
        assert_eq!(read(Value::Text("2..7".to_owned())), [range(2, 7)]);
        assert_eq!(
            read(Value::Text("40..".to_owned())),
            [Range::from_start(40)]
        );
        assert_eq!(read(Value::Text("..9".to_owned())), [Range::up_to(9)]);
        assert_eq!(read(Value::Text("all".to_owned())), [Range::full()]);
        assert_eq!(
            read(Value::List(vec![Value::Range(range(1, 2)), Value::Int(9)])),
            [range(1, 2), Range::single(9)]
        );
    }

    #[test]
    fn invalid_line_values_are_rejected() {
        let error = |value: Value| Selector::from_args(&args(LINES, value)).unwrap_err().code();
        assert_eq!(error(Value::Int(0)), ErrorCode::InvalidValue);
        assert_eq!(
            error(Value::Text("5..2".to_owned())),
            ErrorCode::InvalidValue
        );
        assert_eq!(error(Value::Text("x".to_owned())), ErrorCode::InvalidValue);
        assert_eq!(error(Value::List(Vec::new())), ErrorCode::InvalidValue);
        assert_eq!(error(Value::Bool(true)), ErrorCode::Type);
    }

    #[test]
    fn other_selector_values_are_checked() {
        let error =
            |name: &str, value: Value| Selector::from_args(&args(name, value)).unwrap_err().code();
        assert_eq!(error(LAST, Value::Int(0)), ErrorCode::InvalidValue);
        assert_eq!(error(LAST, Value::Text("3".to_owned())), ErrorCode::Type);
        assert_eq!(
            error(AROUND, Value::Text(String::new())),
            ErrorCode::InvalidValue
        );
        assert_eq!(
            error(SECTION, Value::Text(" ".to_owned())),
            ErrorCode::InvalidValue
        );
        assert_eq!(error(CELL, Value::Int(-1)), ErrorCode::InvalidValue);
        assert_eq!(
            error(KEY, Value::Key("a.".to_owned())),
            ErrorCode::KeySyntax
        );
        assert!(matches!(
            Selector::from_args(&args(CELL, Value::Text("intro".to_owned()))).unwrap(),
            Selector::Cell(CellRef::Id(id)) if id == "intro"
        ));
        assert!(matches!(
            Selector::from_args(&BoundArgs::new("read")).unwrap(),
            Selector::Whole
        ));
    }

    #[test]
    fn two_selectors_conflict() {
        let given = args(LINES, Value::Int(1)).with(LAST, ArgValue::Value(Value::Int(2)), false);
        let error = Selector::from_args(&given).unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
        assert!(error.message.contains("`lines` and `last`"));
    }

    #[test]
    fn targets_need_a_path_and_known_keys() {
        let mut fields = Fields::new();
        fields.insert(LINES.to_owned(), Value::Int(3));
        let missing = Selector::from_target("read", &fields).unwrap_err();
        assert_eq!(missing.code(), ErrorCode::MissingParam);
        fields.insert(PATH.to_owned(), Value::Text("a.txt".to_owned()));
        let (path, selector) = Selector::from_target("read", &fields).unwrap();
        assert_eq!(path, &Value::Text("a.txt".to_owned()));
        assert!(matches!(selector, Selector::Lines(_)));
        assert!(selector.selects_lines());
        fields.insert("offset".to_owned(), Value::Int(1));
        let unknown = Selector::from_target("read", &fields).unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::UnknownField);
        assert_eq!(unknown.found.as_deref(), Some("offset"));
    }

    const GUIDE: &str = "\
# Guide
intro
## Install
step one
### Linux
apt
## Usage
run it
Notes
-----
more
```
# not a heading
```
# Appendix
end
";

    fn section(path: &str) -> Result<LineSpan> {
        section_span(&Lines::new(GUIDE), path, "guide.md")
    }

    #[test]
    fn sections_end_before_a_heading_of_the_same_level() {
        assert_eq!(section("Install").unwrap(), LineSpan::new(3, 6));
        assert_eq!(section("Linux").unwrap(), LineSpan::new(5, 6));
        assert_eq!(section("Guide").unwrap(), LineSpan::new(1, 14));
        assert_eq!(section("Appendix").unwrap(), LineSpan::new(15, 16));
    }

    #[test]
    fn setext_headings_and_fenced_code_are_recognised() {
        assert_eq!(section("Notes").unwrap(), LineSpan::new(9, 14));
        assert!(section("not a heading").is_err());
    }

    #[test]
    fn nested_sections_are_found_by_path() {
        assert_eq!(section("Install > Linux").unwrap(), LineSpan::new(5, 6));
        assert_eq!(section("guide > usage").unwrap(), LineSpan::new(7, 8));
        assert!(section("Usage > Linux").is_err());
        assert_eq!(
            section("Install >  ").unwrap_err().code(),
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn missing_sections_list_the_headings() {
        let error = section("Uninstall").unwrap_err();
        assert_eq!(error.code(), ErrorCode::KeyNotFound);
        let expected = error.expected.clone().unwrap();
        assert_eq!(expected[0], "Guide");
        assert!(expected.as_array().unwrap().iter().any(|t| t == "Notes"));
    }

    #[test]
    fn heading_markers_are_ignored_when_matching() {
        let text = "## The **`read`** [command](https://example.com) ##\nbody\n";
        let lines = Lines::new(text);
        assert_eq!(
            section_span(&lines, "The read command", "a.md").unwrap(),
            LineSpan::new(1, 2)
        );
        assert_eq!(plain_heading("  a   _b_ ![img](x.png) "), "a b img");
        assert_eq!(atx_heading("#hashtag"), None);
        assert_eq!(atx_heading("####### seven"), None);
        assert_eq!(atx_heading("    # code"), None);
        assert_eq!(atx_heading("# C# ##"), Some((1, "C#".to_owned())));
    }

    #[test]
    fn spans_convert_to_ranges() {
        assert_eq!(LineSpan::new(4, 9).range(), range(4, 9));
        assert_eq!(LineSpan::new(4, 9).len(), 6);
        assert_eq!(LineSpan::new(4, 9).to_string(), "4..9");
    }
}
