// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Literal matching that tolerates white space differences.
//!
//! [`find`] searches for a literal pattern through the [`Tolerance`] levels
//! that a [`Whitespace`] setting allows, from the strictest, and stops at the
//! first level that yields a match. An exact occurrence therefore always takes
//! precedence over a reformatted one, and all matches returned by one call are
//! found at the same level.
//!
//! | Level | Comparison |
//! |---|---|
//! | `exact` | byte for byte |
//! | `line_ending` | a line break of the pattern also matches `\r\n` |
//! | `trailing` | whole lines, trailing white space ignored |
//! | `indent` | whole lines, leading and trailing white space ignored, same relative indentation |
//! | `whitespace` | every run of white space equals any other run; the relative indentation of whole lines is kept |
//!
//! [`select`] applies `occurrence`, `expect` and `on_missing` and describes a
//! failed selection with the nearest fragments or the candidate matches.
//! [`Adapter`] fits a replacement to the indentation and line endings of the
//! place it replaces.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range as Span;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Range;
use regex::{Regex, RegexBuilder};

/// Largest number of white-space separated tokens matched at the `whitespace`
/// level; longer patterns are not matched at that level.
const TOKENS_MAX: usize = 2000;
/// Number of candidates listed in a selection error.
const CANDIDATES_SHOWN: usize = 8;
/// Number of nearest fragments listed when nothing matches.
const NEAREST_SHOWN: usize = 3;
/// Characters of a line kept in an excerpt.
const EXCERPT_CHARS: usize = 80;
/// Shortest word, in characters, that counts when ranking nearest fragments.
const WORD_MIN_CHARS: usize = 2;
/// Width of the indentation unit of a text without indented lines.
const DEFAULT_INDENT_WIDTH: usize = 2;

/// Tolerance for white space differences between a literal pattern and the
/// text: the loosest [`Tolerance`] level that [`find`] may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whitespace {
    /// Byte-for-byte comparison; the replacement keeps its indentation.
    Exact,
    /// Line endings and trailing white space of whole lines may differ.
    Trailing,
    /// Indentation, line endings and trailing white space of whole lines may
    /// differ.
    Indent,
    /// Every run of white space matches any other run.
    Normalize,
}

impl Whitespace {
    /// Value with the name used in the catalog: `exact`, `trailing`, `indent`
    /// or `normalize`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "exact" => Some(Self::Exact),
            "trailing" => Some(Self::Trailing),
            "indent" => Some(Self::Indent),
            "normalize" => Some(Self::Normalize),
            _ => None,
        }
    }

    /// Name used in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Trailing => "trailing",
            Self::Indent => "indent",
            Self::Normalize => "normalize",
        }
    }

    /// Loosest level the setting allows.
    #[must_use]
    pub const fn loosest(self) -> Tolerance {
        match self {
            Self::Exact => Tolerance::Exact,
            Self::Trailing => Tolerance::Trailing,
            Self::Indent => Tolerance::Indent,
            Self::Normalize => Tolerance::Whitespace,
        }
    }
}

impl fmt::Display for Whitespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Level at which a match was found, ordered from the strictest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tolerance {
    /// The text contains the pattern byte for byte.
    Exact,
    /// The text differs from the pattern only in `\n` against `\r\n`.
    LineEnding,
    /// Whole lines differ only in trailing white space.
    Trailing,
    /// Whole lines differ only in leading and trailing white space.
    Indent,
    /// The text differs from the pattern in white space.
    Whitespace,
}

impl Tolerance {
    const LEVELS: [Self; 5] = [
        Self::Exact,
        Self::LineEnding,
        Self::Trailing,
        Self::Indent,
        Self::Whitespace,
    ];

    /// Name used in reports: `exact`, `line_ending`, `trailing`, `indent` or
    /// `whitespace`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::LineEnding => "line_ending",
            Self::Trailing => "trailing",
            Self::Indent => "indent",
            Self::Whitespace => "whitespace",
        }
    }
}

impl fmt::Display for Tolerance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Options of [`find`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchOptions {
    /// Tolerance for white space differences.
    pub whitespace: Whitespace,
    /// Whether letters match only in the same case.
    pub case_sensitive: bool,
    /// Whether a match must not continue a word on either side; a word
    /// consists of letters, digits and `_`.
    pub whole_word: bool,
}

impl MatchOptions {
    /// Options with the tolerance, case-sensitive and without the word rule.
    #[must_use]
    pub const fn new(whitespace: Whitespace) -> Self {
        Self {
            whitespace,
            case_sensitive: true,
            whole_word: false,
        }
    }
}

/// One occurrence of a pattern in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Byte offset of the first matched byte.
    pub start: usize,
    /// Byte offset directly after the match.
    pub end: usize,
    /// Line of `start`, from 1.
    pub line: usize,
    /// Column of `start`, from 1, in Unicode scalar values.
    pub column: usize,
    /// Level at which the match was found.
    pub tolerance: Tolerance,
    /// Whether the match consists of whole lines.
    pub whole_lines: bool,
}

/// Line of a text shown in an error: a candidate match or a nearest fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    /// Line, from 1.
    pub line: usize,
    /// Column of the match, from 1; `1` for a nearest fragment.
    pub column: usize,
    /// Line without surrounding white space, up to 80 characters.
    pub text: String,
}

/// Handling of a pattern without matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OnMissing {
    /// Zero matches is a valid result.
    #[default]
    Skip,
    /// Zero matches is an error.
    Error,
}

impl OnMissing {
    /// Value with the name used in the catalog: `skip` or `error`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "skip" => Some(Self::Skip),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// Name used in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Error => "error",
        }
    }
}

/// Choice among the matches of a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// Number of the only match to keep, from 1; `None` keeps all.
    pub occurrence: Option<NonZeroUsize>,
    /// Required number of matches, counted before `occurrence` applies.
    pub expect: Option<Range>,
    /// Handling of zero matches and of an `occurrence` above their number.
    pub on_missing: OnMissing,
}

/// Failure to find or select matches of a literal pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchError {
    /// The pattern is empty.
    EmptyPattern,
    /// The pattern is too large to compile for matching.
    PatternTooLarge,
    /// Nothing matched and `on_missing` is `error`.
    NoMatch {
        /// Lines closest to the first line of the pattern, best first.
        nearest: Vec<Excerpt>,
    },
    /// `occurrence` is above the number of matches and `on_missing` is
    /// `error`.
    OccurrenceMissing {
        /// Requested occurrence.
        occurrence: NonZeroUsize,
        /// Number of matches.
        found: usize,
        /// First matches, in text order.
        candidates: Vec<Excerpt>,
    },
    /// The number of matches is outside `expect`.
    CountMismatch {
        /// Required number of matches.
        expected: Range,
        /// Number of matches.
        found: usize,
        /// First matches, in text order.
        candidates: Vec<Excerpt>,
    },
}

impl MatchError {
    /// Error code: `E_INVALID_VALUE` for an empty pattern, `E_LIMIT` for a
    /// pattern too large, `E_NO_MATCH` for a missing match,
    /// `E_OUT_OF_RANGE` for an occurrence above the number of matches and
    /// `E_ASSERT` for a number of matches outside `expect`.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::EmptyPattern => ErrorCode::InvalidValue,
            Self::PatternTooLarge => ErrorCode::Limit,
            Self::NoMatch { .. } => ErrorCode::NoMatch,
            Self::OccurrenceMissing { .. } => ErrorCode::OutOfRange,
            Self::CountMismatch { .. } => ErrorCode::Assert,
        }
    }
}

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPattern => f.write_str("the pattern is empty; give the text to replace"),
            Self::PatternTooLarge => f.write_str(
                "the pattern is too large to match; split the change into smaller patterns",
            ),
            Self::NoMatch { nearest } => match nearest.first() {
                Some(best) => write!(
                    f,
                    "the pattern does not occur in the text; the nearest fragment is on line {}",
                    best.line
                ),
                None => f.write_str(
                    "the pattern does not occur in the text; read the current text and give an existing fragment",
                ),
            },
            Self::OccurrenceMissing {
                occurrence, found, ..
            } => write!(
                f,
                "occurrence {occurrence} does not exist, the pattern occurs {}; give an occurrence from 1 to {found}",
                times(*found)
            ),
            Self::CountMismatch {
                expected, found, ..
            } => write!(
                f,
                "the pattern occurs {}, expected {expected}; extend the pattern, set `occurrence` or change `expect`",
                times(*found)
            ),
        }
    }
}

impl Error for MatchError {}

fn times(count: usize) -> String {
    match count {
        0 => "0 times".to_owned(),
        1 => "once".to_owned(),
        _ => format!("{count} times"),
    }
}

/// Finds the occurrences of a literal pattern.
///
/// The search goes through the [`Tolerance`] levels allowed by
/// `options.whitespace` and returns the non-overlapping matches of the first
/// level that has any, in text order. Matches that break the word rule of
/// `options.whole_word` do not count at their level.
///
/// # Errors
///
/// Returns [`MatchError::EmptyPattern`] for an empty pattern and
/// [`MatchError::PatternTooLarge`] when the pattern cannot be compiled for
/// matching.
///
/// # Examples
///
/// ```
/// use nurt_text::fuzzy_match::{find, MatchOptions, Tolerance, Whitespace};
///
/// let text = "fn main() {\r\n\tlet a = 1;\r\n}\r\n";
/// let found = find(text, "    let a = 1;", &MatchOptions::new(Whitespace::Normalize))?;
/// assert_eq!(found.len(), 1);
/// assert_eq!(found[0].line, 2);
/// assert_eq!(found[0].tolerance, Tolerance::Indent);
/// # Ok::<(), nurt_text::fuzzy_match::MatchError>(())
/// ```
pub fn find(text: &str, pattern: &str, options: &MatchOptions) -> Result<Vec<Match>, MatchError> {
    if pattern.is_empty() {
        return Err(MatchError::EmptyPattern);
    }
    let lines = LineIndex::new(text);
    let unified = pattern.replace("\r\n", "\n");
    let loosest = options.whitespace.loosest();
    for level in Tolerance::LEVELS
        .into_iter()
        .filter(|level| *level <= loosest)
    {
        let spans = level_spans(text, &lines, pattern, &unified, level, options)?;
        if !spans.is_empty() {
            return Ok(spans
                .into_iter()
                .map(|span| lines.match_at(text, span, level))
                .collect());
        }
    }
    Ok(Vec::new())
}

/// Span found at one level, with whether it consists of whole lines.
struct Found {
    span: Span<usize>,
    whole_lines: bool,
}

fn level_spans(
    text: &str,
    lines: &LineIndex,
    pattern: &str,
    unified: &str,
    level: Tolerance,
    options: &MatchOptions,
) -> Result<Vec<Found>, MatchError> {
    let case = options.case_sensitive;
    let found = match level {
        Tolerance::Exact => exact_spans(text, pattern, case)?,
        Tolerance::LineEnding => line_ending_spans(text, unified, case)?,
        Tolerance::Trailing | Tolerance::Indent => window_spans(text, lines, unified, level, case),
        Tolerance::Whitespace => {
            let window = keep_words(
                text,
                window_spans(text, lines, unified, level, case),
                options,
            );
            if !window.is_empty() {
                return Ok(window);
            }
            token_spans(text, lines, unified, case)?
        }
    };
    Ok(keep_words(text, found, options))
}

fn keep_words(text: &str, found: Vec<Found>, options: &MatchOptions) -> Vec<Found> {
    if !options.whole_word {
        return found;
    }
    found
        .into_iter()
        .filter(|item| at_word_boundaries(text, item.span.clone()))
        .collect()
}

fn compile(source: &str, case_sensitive: bool) -> Result<Regex, MatchError> {
    RegexBuilder::new(source)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|_| MatchError::PatternTooLarge)
}

fn regex_spans(regex: &Regex, text: &str) -> Vec<Found> {
    regex
        .find_iter(text)
        .map(|found| Found {
            span: found.range(),
            whole_lines: false,
        })
        .collect()
}

fn exact_spans(text: &str, pattern: &str, case_sensitive: bool) -> Result<Vec<Found>, MatchError> {
    if case_sensitive {
        return Ok(text
            .match_indices(pattern)
            .map(|(start, found)| Found {
                span: start..start + found.len(),
                whole_lines: false,
            })
            .collect());
    }
    Ok(regex_spans(&compile(&regex::escape(pattern), false)?, text))
}

fn line_ending_spans(
    text: &str,
    unified: &str,
    case_sensitive: bool,
) -> Result<Vec<Found>, MatchError> {
    if !unified.contains('\n') {
        return Ok(Vec::new());
    }
    let source = unified
        .split('\n')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\r?\n");
    Ok(regex_spans(&compile(&source, case_sensitive)?, text))
}

/// Matches windows of whole lines that equal the pattern lines after the
/// normalization of the level.
fn window_spans(
    text: &str,
    lines: &LineIndex,
    unified: &str,
    level: Tolerance,
    case_sensitive: bool,
) -> Vec<Found> {
    let wanted_raw = split_lines(unified);
    let wanted: Vec<String> = wanted_raw
        .iter()
        .map(|line| normalize_line(line, level, case_sensitive))
        .collect();
    if wanted.iter().all(String::is_empty) {
        return Vec::new();
    }
    let text_raw = split_lines(text);
    let normalized: Vec<String> = text_raw
        .iter()
        .map(|line| normalize_line(line, level, case_sensitive))
        .collect();
    let size = wanted.len();
    let mut found = Vec::new();
    let mut row = 0;
    while row + size <= normalized.len() {
        let rows = row..row + size;
        let equal = normalized[rows.clone()] == wanted[..];
        if equal && (level == Tolerance::Trailing || same_shape(&wanted_raw, &text_raw[rows])) {
            found.push(Found {
                span: lines.whole_lines(text, row, size, unified.ends_with('\n')),
                whole_lines: true,
            });
            row += size;
        } else {
            row += 1;
        }
    }
    found
}

/// Matches the white-space separated tokens of the pattern separated by any
/// white space in the text.
fn token_spans(
    text: &str,
    lines: &LineIndex,
    unified: &str,
    case_sensitive: bool,
) -> Result<Vec<Found>, MatchError> {
    let tokens: Vec<&str> = unified.split_whitespace().collect();
    if tokens.is_empty() || tokens.len() > TOKENS_MAX {
        return Ok(Vec::new());
    }
    let source = tokens
        .iter()
        .map(|token| regex::escape(token))
        .collect::<Vec<_>>()
        .join(r"\s+");
    let shape = split_lines(trim_blank_edges(unified));
    let found = regex_spans(&compile(&source, case_sensitive)?, text)
        .into_iter()
        .filter(|item| {
            if shape.len() < 2 {
                return true;
            }
            let covered = split_lines(&text[lines.line_start_of(item.span.start)..item.span.end]);
            covered.len() != shape.len() || same_shape(&shape, &covered)
        })
        .collect();
    Ok(found)
}

/// Text without leading blank lines and trailing white space.
fn trim_blank_edges(text: &str) -> &str {
    let mut rest = text;
    while let Some(newline) = rest.find('\n') {
        if !rest[..newline]
            .bytes()
            .all(|byte| byte == b' ' || byte == b'\t')
        {
            break;
        }
        rest = &rest[newline + 1..];
    }
    rest.trim_end()
}

fn normalize_line(line: &str, level: Tolerance, case_sensitive: bool) -> String {
    let normalized = match level {
        Tolerance::Trailing => line.trim_end().to_owned(),
        Tolerance::Indent => line.trim().to_owned(),
        _ => line.split_whitespace().collect::<Vec<_>>().join(" "),
    };
    if case_sensitive {
        normalized
    } else {
        normalized.to_lowercase()
    }
}

/// Whether the non-blank lines of the pattern and of the text have the same
/// relative indentation: each line is deeper, shallower or level with the
/// first and the previous non-blank line in both. Without this rule a
/// tolerant match could move a line to another nesting level, which changes
/// the meaning of Python or YAML while keeping the syntax valid.
fn same_shape(wanted: &[&str], found: &[&str]) -> bool {
    let depth = |lines: &[&str], row: usize, other: usize| -> isize {
        let width = |at: usize| lines.get(at).map_or(0, |line| indent_of(line).len());
        direction(width(row), width(other))
    };
    let mut first: Option<usize> = None;
    let mut previous = 0;
    for (row, wanted_line) in wanted.iter().enumerate() {
        let found_line = found.get(row).copied().unwrap_or_default();
        if wanted_line.trim().is_empty() || found_line.trim().is_empty() {
            continue;
        }
        match first {
            Some(first) => {
                if depth(wanted, row, first) != depth(found, row, first)
                    || depth(wanted, row, previous) != depth(found, row, previous)
                {
                    return false;
                }
            }
            None => first = Some(row),
        }
        previous = row;
    }
    true
}

fn direction(left: usize, right: usize) -> isize {
    match left.cmp(&right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether the span neither starts inside a word nor ends inside a word.
pub(crate) fn at_word_boundaries(text: &str, span: Span<usize>) -> bool {
    let joined = |left: Option<char>, right: Option<char>| matches!((left, right), (Some(left), Some(right)) if is_word(left) && is_word(right));
    let inner = &text[span.clone()];
    !joined(text[..span.start].chars().next_back(), inner.chars().next())
        && !joined(inner.chars().next_back(), text[span.end..].chars().next())
}

/// Applies a [`Selection`] to the matches of a pattern.
///
/// `expect` is checked first, against the number of all matches; then zero
/// matches give an empty result or [`MatchError::NoMatch`], and `occurrence`
/// keeps a single match. `pattern` ranks the nearest fragments of
/// [`MatchError::NoMatch`]; an empty text omits them.
///
/// # Errors
///
/// Returns [`MatchError::CountMismatch`] when the number of matches is outside
/// `expect`, and with `on_missing: error` [`MatchError::NoMatch`] for zero
/// matches or [`MatchError::OccurrenceMissing`] for an `occurrence` above
/// their number.
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Range;
/// use nurt_text::fuzzy_match::{find, select, MatchError, MatchOptions, Selection, Whitespace};
///
/// let text = "a = 1\nb = 1\n";
/// let found = find(text, "= 1", &MatchOptions::new(Whitespace::Exact))?;
/// let unique = Selection { expect: Some(Range::single(1)), ..Selection::default() };
/// let error = select(text, "= 1", found, &unique).unwrap_err();
/// assert!(matches!(error, MatchError::CountMismatch { found: 2, .. }));
/// # Ok::<(), MatchError>(())
/// ```
pub fn select(
    text: &str,
    pattern: &str,
    matches: Vec<Match>,
    selection: &Selection,
) -> Result<Vec<Match>, MatchError> {
    let kept = selected(text, pattern, &matches, selection)?;
    Ok(matches
        .into_iter()
        .skip(kept.start)
        .take(kept.len())
        .collect())
}

/// Range of the matches kept by a selection; see [`select`].
pub(crate) fn selected(
    text: &str,
    pattern: &str,
    matches: &[Match],
    selection: &Selection,
) -> Result<Span<usize>, MatchError> {
    let found = matches.len();
    if let Some(expected) = selection.expect
        && !expected.contains(i64::try_from(found).unwrap_or(i64::MAX))
    {
        if found == 0 && selection.on_missing == OnMissing::Error {
            return Err(MatchError::NoMatch {
                nearest: nearest(text, pattern),
            });
        }
        return Err(MatchError::CountMismatch {
            expected,
            found,
            candidates: candidates(text, matches),
        });
    }
    if found == 0 {
        return match selection.on_missing {
            OnMissing::Skip => Ok(0..0),
            OnMissing::Error => Err(MatchError::NoMatch {
                nearest: nearest(text, pattern),
            }),
        };
    }
    let Some(occurrence) = selection.occurrence else {
        return Ok(0..found);
    };
    let index = occurrence.get() - 1;
    if index < found {
        return Ok(index..index + 1);
    }
    match selection.on_missing {
        OnMissing::Skip => Ok(0..0),
        OnMissing::Error => Err(MatchError::OccurrenceMissing {
            occurrence,
            found,
            candidates: candidates(text, matches),
        }),
    }
}

fn candidates(text: &str, matches: &[Match]) -> Vec<Excerpt> {
    matches
        .iter()
        .take(CANDIDATES_SHOWN)
        .map(|found| {
            let start = text[..found.start].rfind('\n').map_or(0, |at| at + 1);
            let line = text[start..].split('\n').next().unwrap_or_default();
            Excerpt {
                line: found.line,
                column: found.column,
                text: excerpt(line),
            }
        })
        .collect()
}

fn excerpt(line: &str) -> String {
    line.trim().chars().take(EXCERPT_CHARS).collect()
}

fn words(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|character: char| !is_word(character))
        .filter(|word| word.chars().count() >= WORD_MIN_CHARS)
        .map(str::to_lowercase)
}

/// Lines of the text closest to the first non-blank line of the pattern.
///
/// The score of a line is the number of its words that occur in that pattern
/// line, compared without case; a line qualifies when it reaches half of the
/// distinct pattern words. Up to three lines are returned, best first, and
/// equal scores keep text order.
///
/// # Examples
///
/// ```
/// use nurt_text::fuzzy_match::nearest;
///
/// let text = "let total = price * count;\nreturn total;\n";
/// let found = nearest(text, "let total = price * amount;");
/// assert_eq!(found[0].line, 1);
/// ```
#[must_use]
pub fn nearest(text: &str, pattern: &str) -> Vec<Excerpt> {
    let Some(first) = split_lines(pattern)
        .into_iter()
        .find(|line| !line.trim().is_empty())
    else {
        return Vec::new();
    };
    let wanted: HashSet<String> = words(first).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, usize, &str)> = split_lines(text)
        .into_iter()
        .enumerate()
        .filter_map(|(row, line)| {
            let score = words(line).filter(|word| wanted.contains(word)).count();
            (score > 0 && score * 2 >= wanted.len()).then_some((score, row, line))
        })
        .collect();
    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    scored
        .into_iter()
        .take(NEAREST_SHOWN)
        .map(|(_, row, line)| Excerpt {
            line: row + 1,
            column: 1,
            text: excerpt(line),
        })
        .collect()
}

/// Fits replacements to the indentation and line endings of one text.
///
/// Line endings of a replacement always follow the text: `\r\n` when the majority of
/// line breaks of the text are `\r\n`, otherwise `\n`. Indentation is
/// adapted unless the tolerance is [`Whitespace::Exact`]:
///
/// - a match of whole lines found at the `indent` or `whitespace` level moves
///   the replacement from the indentation of the pattern to the indentation
///   of the matched lines, and converts relative indentation from the unit of
///   the pattern (tab or a number of spaces) to the unit of the text;
/// - a match that starts after the indentation of its line gives that
///   indentation to the following lines of a single-line pattern's
///   replacement;
/// - a multi-line pattern matched exactly already carries the indentation of
///   the text, so its replacement is kept.
///
/// # Examples
///
/// ```
/// use nurt_text::fuzzy_match::{find, Adapter, MatchOptions, Whitespace};
///
/// let text = "if ready {\n\t\tstart();\n}\n";
/// let pattern = "start();";
/// let found = find(text, pattern, &MatchOptions::new(Whitespace::Normalize))?;
/// let adapter = Adapter::new(text, pattern, Whitespace::Normalize);
/// assert_eq!(adapter.adapt(&found[0], "prepare();\nstart();"), "prepare();\n\t\tstart();");
/// # Ok::<(), nurt_text::fuzzy_match::MatchError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Adapter<'t> {
    text: &'t str,
    pattern: String,
    whitespace: Whitespace,
    line_ending: &'static str,
    unit: String,
}

impl<'t> Adapter<'t> {
    /// Adapter for the matches of `pattern` in `text`.
    #[must_use]
    pub fn new(text: &'t str, pattern: &str, whitespace: Whitespace) -> Self {
        Self {
            text,
            pattern: pattern.replace("\r\n", "\n"),
            whitespace,
            line_ending: detect_line_ending(text),
            unit: indent_unit(text),
        }
    }

    /// Replacement for the match in the style of the text.
    #[must_use]
    pub fn adapt(&self, found: &Match, replacement: &str) -> String {
        let unified = replacement.replace("\r\n", "\n");
        let exact_block = matches!(found.tolerance, Tolerance::Exact | Tolerance::LineEnding)
            && self.pattern.trim_end().contains('\n');
        let shaped = if self.whitespace == Whitespace::Exact {
            unified
        } else if found.whole_lines && found.tolerance != Tolerance::Trailing {
            self.reindent(&unified, &self.text[found.start..found.end])
        } else if found.whole_lines || exact_block {
            unified
        } else {
            self.hanging_indent(&unified, found.start)
        };
        to_line_ending(&shaped, self.line_ending)
    }

    fn reindent(&self, replacement: &str, matched: &str) -> String {
        let pattern_base = first_indent(&self.pattern);
        let text_base = first_indent(matched);
        let pattern_unit = indent_unit(&format!("{}\n{replacement}", self.pattern));
        if pattern_base == text_base && pattern_unit == self.unit {
            return replacement.to_owned();
        }
        replacement
            .split('\n')
            .map(|line| {
                if line.trim().is_empty() {
                    return line.to_owned();
                }
                let lead = indent_of(line);
                let relative = lead.strip_prefix(pattern_base).unwrap_or_default();
                let converted = convert_unit(relative, &pattern_unit, &self.unit);
                format!("{text_base}{converted}{}", &line[lead.len()..])
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn hanging_indent(&self, replacement: &str, start: usize) -> String {
        let line_start = self.text[..start].rfind('\n').map_or(0, |at| at + 1);
        let hanging = &self.text[line_start..start];
        if hanging.is_empty() || !hanging.trim().is_empty() {
            return replacement.to_owned();
        }
        replacement
            .split('\n')
            .enumerate()
            .map(|(row, line)| {
                if row == 0 || line.trim().is_empty() {
                    line.to_owned()
                } else {
                    format!("{hanging}{line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn indent_of(line: &str) -> &str {
    let rest = line.trim_start_matches([' ', '\t']);
    &line[..line.len() - rest.len()]
}

fn first_indent(text: &str) -> &str {
    split_lines(text)
        .into_iter()
        .find(|line| !line.trim().is_empty())
        .map_or("", indent_of)
}

/// Indentation unit of a text: a tab when tab-indented lines are at least as
/// frequent as the commonest increase of space indentation, otherwise
/// that increase in spaces.
fn indent_unit(text: &str) -> String {
    let mut steps: Vec<(usize, usize)> = Vec::new();
    let mut tabs = 0;
    let mut previous = 0;
    for line in split_lines(text) {
        if line.trim().is_empty() {
            continue;
        }
        let lead = indent_of(line);
        if lead.starts_with('\t') {
            tabs += 1;
        }
        if !lead.contains('\t') && lead.len() > previous {
            let step = lead.len() - previous;
            match steps.iter_mut().find(|(known, _)| *known == step) {
                Some((_, count)) => *count += 1,
                None => steps.push((step, 1)),
            }
        }
        previous = lead.len();
    }
    let best = steps.into_iter().fold(
        None,
        |best: Option<(usize, usize)>, (step, count)| match best {
            Some((_, top)) if top >= count => best,
            _ => Some((step, count)),
        },
    );
    if tabs > 0 && tabs >= best.map_or(0, |(_, count)| count) {
        "\t".to_owned()
    } else {
        " ".repeat(best.map_or(DEFAULT_INDENT_WIDTH, |(step, _)| step))
    }
}

/// Relative indentation in the unit `to` when it consists of whole units
/// `from`; any other indentation is kept.
fn convert_unit(relative: &str, from: &str, to: &str) -> String {
    if from == to
        || relative.is_empty()
        || from.is_empty()
        || !relative.len().is_multiple_of(from.len())
    {
        return relative.to_owned();
    }
    let units = relative.len() / from.len();
    if relative == from.repeat(units) {
        to.repeat(units)
    } else {
        relative.to_owned()
    }
}

/// Lines of a text without their terminators; a final line break does not
/// open an empty line.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// Prevailing line ending of a text: `\r\n` when it terminates more lines
/// than a bare `\n`, otherwise `\n`.
pub(crate) fn detect_line_ending(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf { "\r\n" } else { "\n" }
}

/// Text with every line break written as `line_ending`.
pub(crate) fn to_line_ending(text: &str, line_ending: &str) -> String {
    let unified = text.replace("\r\n", "\n");
    if line_ending == "\n" {
        unified
    } else {
        unified.replace('\n', line_ending)
    }
}

/// Byte offsets of the line starts of a text.
pub(crate) struct LineIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub(crate) fn new(text: &str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        Self {
            starts,
            len: text.len(),
        }
    }

    /// Number of lines; a final line break does not open a new line.
    pub(crate) fn line_count(&self) -> usize {
        match self.starts.last() {
            _ if self.len == 0 => 0,
            Some(last) if *last == self.len => self.starts.len() - 1,
            _ => self.starts.len(),
        }
    }

    /// Byte offset of the start of the line `row`, counted from 0; rows past
    /// the last line start at the end of the text.
    pub(crate) fn row_start(&self, row: usize) -> usize {
        self.starts.get(row).copied().unwrap_or(self.len)
    }

    /// Byte offset of the start of the line that contains `offset`.
    pub(crate) fn line_start_of(&self, offset: usize) -> usize {
        let row = self.starts.partition_point(|start| *start <= offset);
        self.row_start(row.saturating_sub(1))
    }

    /// Span of `size` whole lines from the line `row`, counted from 0, with
    /// or without the line break of the last line.
    fn whole_lines(&self, text: &str, row: usize, size: usize, terminated: bool) -> Span<usize> {
        let start = self.row_start(row);
        let next = self.row_start(row + size);
        if terminated {
            return start..next;
        }
        let covered = &text[start..next];
        let terminator = if covered.ends_with("\r\n") {
            2
        } else {
            usize::from(covered.ends_with('\n'))
        };
        start..next - terminator
    }

    fn match_at(&self, text: &str, found: Found, tolerance: Tolerance) -> Match {
        let mut located = self.locate(text, found.span, tolerance);
        located.whole_lines = found.whole_lines;
        located
    }

    /// Match covering the span, with its line and column.
    pub(crate) fn locate(&self, text: &str, span: Span<usize>, tolerance: Tolerance) -> Match {
        let row = self.starts.partition_point(|start| *start <= span.start);
        let line_start = self.row_start(row.saturating_sub(1));
        Match {
            start: span.start,
            end: span.end,
            line: row,
            column: text[line_start..span.start].chars().count() + 1,
            tolerance,
            whole_lines: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str, pattern: &str, whitespace: Whitespace) -> Vec<Match> {
        find(text, pattern, &MatchOptions::new(whitespace)).unwrap()
    }

    fn spans(text: &str, pattern: &str, whitespace: Whitespace) -> Vec<(usize, usize, Tolerance)> {
        found(text, pattern, whitespace)
            .into_iter()
            .map(|item| (item.start, item.end, item.tolerance))
            .collect()
    }

    fn replace_once(
        text: &str,
        pattern: &str,
        replacement: &str,
        whitespace: Whitespace,
    ) -> String {
        let matches = found(text, pattern, whitespace);
        assert_eq!(
            matches.len(),
            1,
            "expected one match of {pattern:?} in {text:?}"
        );
        let adapter = Adapter::new(text, pattern, whitespace);
        let item = &matches[0];
        format!(
            "{}{}{}",
            &text[..item.start],
            adapter.adapt(item, replacement),
            &text[item.end..]
        )
    }

    #[test]
    fn parses_and_names_whitespace_values() {
        for name in ["exact", "trailing", "indent", "normalize"] {
            let value = Whitespace::from_name(name).unwrap();
            assert_eq!(value.as_str(), name);
            assert_eq!(value.to_string(), name);
        }
        assert_eq!(Whitespace::from_name("loose"), None);
    }

    #[test]
    fn parses_and_names_on_missing_values() {
        assert_eq!(OnMissing::from_name("skip"), Some(OnMissing::Skip));
        assert_eq!(OnMissing::from_name("error"), Some(OnMissing::Error));
        assert_eq!(OnMissing::from_name("ignore"), None);
        assert_eq!(OnMissing::Error.as_str(), "error");
        assert_eq!(OnMissing::default().as_str(), "skip");
    }

    #[test]
    fn names_tolerance_levels() {
        let names: Vec<String> = Tolerance::LEVELS.iter().map(ToString::to_string).collect();
        assert_eq!(
            names,
            ["exact", "line_ending", "trailing", "indent", "whitespace"]
        );
    }

    #[test]
    fn rejects_empty_pattern() {
        let error = find("abc", "", &MatchOptions::new(Whitespace::Normalize)).unwrap_err();
        assert_eq!(error, MatchError::EmptyPattern);
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn finds_exact_non_overlapping_matches() {
        assert_eq!(
            spans("aaaa", "aa", Whitespace::Exact),
            [(0, 2, Tolerance::Exact), (2, 4, Tolerance::Exact)]
        );
    }

    #[test]
    fn exact_match_wins_over_tolerant_match() {
        let text = "x  = 1\nx = 1\n";
        assert_eq!(
            spans(text, "x = 1", Whitespace::Normalize),
            [(7, 12, Tolerance::Exact)]
        );
    }

    #[test]
    fn matches_lf_pattern_in_crlf_text() {
        let text = "one\r\ntwo\r\nthree\r\n";
        assert_eq!(
            spans(text, "one\ntwo", Whitespace::Trailing),
            [(0, 8, Tolerance::LineEnding)]
        );
    }

    #[test]
    fn matches_crlf_pattern_in_lf_text() {
        assert_eq!(
            spans("one\ntwo\n", "one\r\ntwo\r\n", Whitespace::Trailing),
            [(0, 8, Tolerance::LineEnding)]
        );
    }

    #[test]
    fn exact_setting_does_not_tolerate_line_endings() {
        assert!(found("one\r\ntwo", "one\ntwo", Whitespace::Exact).is_empty());
    }

    #[test]
    fn trailing_level_ignores_trailing_white_space_of_lines() {
        let text = "alpha  \nbeta\t\n";
        assert_eq!(
            spans(text, "alpha\nbeta", Whitespace::Trailing),
            [(0, 13, Tolerance::Trailing)]
        );
    }

    #[test]
    fn trailing_setting_does_not_tolerate_indentation() {
        assert!(found("    alpha\n", "alpha\n", Whitespace::Trailing).len() == 1);
        assert!(found("    alpha beta\n", "alpha  beta", Whitespace::Trailing).is_empty());
    }

    #[test]
    fn indent_level_matches_tabs_against_spaces() {
        let text = "fn f() {\n\tif x {\n\t\ty();\n\t}\n}\n";
        let matches = found(
            text,
            "    if x {\n        y();\n    }\n",
            Whitespace::Indent,
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].tolerance, Tolerance::Indent);
        assert_eq!(matches[0].line, 2);
        assert_eq!(
            &text[matches[0].start..matches[0].end],
            "\tif x {\n\t\ty();\n\t}\n"
        );
    }

    #[test]
    fn indent_level_rejects_different_nesting() {
        let text = "if a:\n    b()\nc()\n";
        assert!(found(text, "if a:\nb()\n    c()\n", Whitespace::Indent).is_empty());
    }

    #[test]
    fn indent_level_tolerates_blank_lines_with_spaces() {
        let text = "  a\n    \n  b\n";
        assert_eq!(found(text, "a\n\nb", Whitespace::Indent).len(), 1);
    }

    #[test]
    fn whole_line_match_without_final_break_excludes_terminator() {
        let text = "  a\r\n  b\r\nc";
        let matches = found(text, "a\nb", Whitespace::Indent);
        assert_eq!(&text[matches[0].start..matches[0].end], "  a\r\n  b");
    }

    #[test]
    fn whitespace_level_collapses_inner_runs() {
        let text = "let  x =   1;\n";
        assert_eq!(
            spans(text, "let x = 1;", Whitespace::Normalize),
            [(0, 13, Tolerance::Whitespace)]
        );
    }

    #[test]
    fn whitespace_level_matches_tokens_inside_lines() {
        let text = "call(a,   b) + 1";
        assert_eq!(
            spans(text, "a, b", Whitespace::Normalize),
            [(5, 11, Tolerance::Whitespace)]
        );
    }

    #[test]
    fn whitespace_level_matches_tokens_across_lines() {
        let text = "x = f(a,\n      b)\n";
        let matches = found(text, "f(a, b)", Whitespace::Normalize);
        assert_eq!(&text[matches[0].start..matches[0].end], "f(a,\n      b)");
    }

    #[test]
    fn whitespace_level_keeps_relative_indentation_of_tokens() {
        let text = "if a:\n    b\n";
        assert!(found(text, "if  a:\n  b", Whitespace::Normalize).len() == 1);
        let flat = "if a:\nb\n";
        assert!(found(flat, "if  a:\n  b", Whitespace::Normalize).is_empty());
    }

    #[test]
    fn white_space_pattern_matches_nothing_tolerantly() {
        assert!(found("a b", "   ", Whitespace::Normalize).is_empty());
        assert_eq!(found("a   b", "   ", Whitespace::Exact).len(), 1);
    }

    #[test]
    fn indent_setting_does_not_collapse_inner_runs() {
        assert!(found("a  b\n", "a b", Whitespace::Indent).is_empty());
    }

    #[test]
    fn too_many_tokens_are_not_matched_loosely() {
        let tokens = |count: usize, separator: &str| vec!["t"; count].join(separator);
        let limit = found(
            &tokens(TOKENS_MAX, "\n"),
            &tokens(TOKENS_MAX, " "),
            Whitespace::Normalize,
        );
        assert_eq!(limit.len(), 1);
        let above = tokens(TOKENS_MAX + 1, " ");
        assert!(found(&tokens(TOKENS_MAX + 1, "\n"), &above, Whitespace::Normalize).is_empty());
    }

    #[test]
    fn matches_without_case_when_requested() {
        let options = MatchOptions {
            case_sensitive: false,
            ..MatchOptions::new(Whitespace::Normalize)
        };
        let exact = find("Hello World", "hello world", &options).unwrap();
        assert_eq!(exact[0].tolerance, Tolerance::Exact);
        let crlf = find("A\r\nB", "a\nb", &options).unwrap();
        assert_eq!(crlf[0].tolerance, Tolerance::LineEnding);
        let indented = find("  A\n  B\n", "a\nb", &options).unwrap();
        assert_eq!(indented[0].tolerance, Tolerance::Indent);
    }

    #[test]
    fn whole_word_skips_parts_of_longer_words() {
        let options = MatchOptions {
            whole_word: true,
            ..MatchOptions::new(Whitespace::Exact)
        };
        let matches = find("count counter recount count_x count", "count", &options).unwrap();
        let starts: Vec<usize> = matches.iter().map(|item| item.start).collect();
        assert_eq!(starts, [0, 30]);
    }

    #[test]
    fn whole_word_falls_back_to_tolerant_level() {
        let options = MatchOptions {
            whole_word: true,
            ..MatchOptions::new(Whitespace::Normalize)
        };
        let matches = find("ab cx\na  b\n", "a b", &options).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 2);
    }

    #[test]
    fn whole_word_applies_to_token_level() {
        let options = MatchOptions {
            whole_word: true,
            ..MatchOptions::new(Whitespace::Normalize)
        };
        assert!(find("xa  by", "a b", &options).unwrap().is_empty());
    }

    #[test]
    fn whole_word_accepts_punctuation_edges() {
        assert!(at_word_boundaries("(x)", 0..3));
        assert!(at_word_boundaries("a(x)b", 1..4));
        assert!(!at_word_boundaries("ax", 1..2));
        assert!(at_word_boundaries("ab", 1..1));
    }

    #[test]
    fn reports_line_and_column_in_characters() {
        let matches = found("café\n  naïve", "naïve", Whitespace::Exact);
        assert_eq!((matches[0].line, matches[0].column), (2, 3));
    }

    #[test]
    fn select_keeps_all_matches_by_default() {
        let text = "x x x";
        let matches = found(text, "x", Whitespace::Exact);
        assert_eq!(
            select(text, "x", matches, &Selection::default())
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn select_keeps_requested_occurrence() {
        let text = "x x x";
        let matches = found(text, "x", Whitespace::Exact);
        let selection = Selection {
            occurrence: NonZeroUsize::new(2),
            ..Selection::default()
        };
        let kept = select(text, "x", matches, &selection).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].start, 2);
    }

    #[test]
    fn select_reports_missing_occurrence_with_candidates() {
        let text = "x\n  x";
        let matches = found(text, "x", Whitespace::Exact);
        let selection = Selection {
            occurrence: NonZeroUsize::new(3),
            on_missing: OnMissing::Error,
            ..Selection::default()
        };
        let error = select(text, "x", matches, &selection).unwrap_err();
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        let MatchError::OccurrenceMissing {
            found, candidates, ..
        } = &error
        else {
            panic!("unexpected error {error:?}");
        };
        assert_eq!(*found, 2);
        assert_eq!(
            candidates[1],
            Excerpt {
                line: 2,
                column: 3,
                text: "x".to_owned()
            }
        );
        assert_eq!(
            error.to_string(),
            "occurrence 3 does not exist, the pattern occurs 2 times; give an occurrence from 1 to 2"
        );
    }

    #[test]
    fn select_skips_missing_occurrence_when_allowed() {
        let text = "x";
        let matches = found(text, "x", Whitespace::Exact);
        let selection = Selection {
            occurrence: NonZeroUsize::new(2),
            ..Selection::default()
        };
        assert!(select(text, "x", matches, &selection).unwrap().is_empty());
    }

    #[test]
    fn select_reports_ambiguity_against_expect() {
        let text = "a = 1\nb = 1\n";
        let matches = found(text, "= 1", Whitespace::Exact);
        let selection = Selection {
            expect: Some(Range::single(1)),
            ..Selection::default()
        };
        let error = select(text, "= 1", matches, &selection).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Assert);
        let MatchError::CountMismatch { candidates, .. } = &error else {
            panic!("unexpected error {error:?}");
        };
        let lines: Vec<usize> = candidates.iter().map(|item| item.line).collect();
        assert_eq!(lines, [1, 2]);
        assert_eq!(
            error.to_string(),
            "the pattern occurs 2 times, expected 1; extend the pattern, set `occurrence` or change `expect`"
        );
    }

    #[test]
    fn select_checks_expect_before_occurrence() {
        let text = "x x x";
        let matches = found(text, "x", Whitespace::Exact);
        let selection = Selection {
            occurrence: NonZeroUsize::new(1),
            expect: Some(Range::new(Some(2), Some(3)).unwrap()),
            on_missing: OnMissing::Skip,
        };
        assert_eq!(select(text, "x", matches, &selection).unwrap().len(), 1);
    }

    #[test]
    fn select_limits_candidates() {
        let text = "x ".repeat(CANDIDATES_SHOWN + 4);
        let matches = found(&text, "x", Whitespace::Exact);
        let selection = Selection {
            expect: Some(Range::single(1)),
            ..Selection::default()
        };
        let MatchError::CountMismatch {
            found, candidates, ..
        } = select(&text, "x", matches, &selection).unwrap_err()
        else {
            panic!("expected a count mismatch");
        };
        assert_eq!(found, CANDIDATES_SHOWN + 4);
        assert_eq!(candidates.len(), CANDIDATES_SHOWN);
    }

    #[test]
    fn select_returns_nothing_for_missing_pattern_when_skipping() {
        assert!(
            select("abc", "x", Vec::new(), &Selection::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn select_reports_missing_pattern_with_nearest_fragments() {
        let text = "fn load(path: &str) {}\nfn save(path: &str) {}\nfn other() {}\n";
        let selection = Selection {
            on_missing: OnMissing::Error,
            ..Selection::default()
        };
        let error = select(text, "fn load(file: &str)", Vec::new(), &selection).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NoMatch);
        let MatchError::NoMatch { nearest } = &error else {
            panic!("unexpected error {error:?}");
        };
        let lines: Vec<usize> = nearest.iter().map(|item| item.line).collect();
        assert_eq!(lines, [1, 2]);
        assert_eq!(nearest[0].text, "fn load(path: &str) {}");
        assert_eq!(
            error.to_string(),
            "the pattern does not occur in the text; the nearest fragment is on line 1"
        );
    }

    #[test]
    fn missing_pattern_with_zero_expected_is_valid_when_skipping() {
        let selection = Selection {
            expect: Some(Range::new(Some(0), Some(1)).unwrap()),
            ..Selection::default()
        };
        assert!(
            select("abc", "x", Vec::new(), &selection)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn on_missing_error_requires_a_match_even_when_expect_allows_zero() {
        let selection = Selection {
            expect: Some(Range::new(Some(0), Some(1)).unwrap()),
            on_missing: OnMissing::Error,
            ..Selection::default()
        };
        let error = select("abc", "x", Vec::new(), &selection).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NoMatch);
    }

    #[test]
    fn missing_pattern_violating_expect_reports_no_match_when_required() {
        let selection = Selection {
            expect: Some(Range::single(1)),
            on_missing: OnMissing::Error,
            ..Selection::default()
        };
        let error = select("abc", "zzz", Vec::new(), &selection).unwrap_err();
        assert_eq!(
            error,
            MatchError::NoMatch {
                nearest: Vec::new()
            }
        );
        assert_eq!(
            error.to_string(),
            "the pattern does not occur in the text; read the current text and give an existing fragment"
        );
    }

    #[test]
    fn missing_pattern_violating_expect_reports_count_when_skipping() {
        let selection = Selection {
            expect: Some(Range::single(1)),
            ..Selection::default()
        };
        let error = select("abc", "zzz", Vec::new(), &selection).unwrap_err();
        assert_eq!(
            error.to_string().split(';').next(),
            Some("the pattern occurs 0 times, expected 1")
        );
    }

    #[test]
    fn single_match_against_expect_is_described_as_once() {
        let matches = found("x", "x", Whitespace::Exact);
        let selection = Selection {
            expect: Some(Range::single(2)),
            ..Selection::default()
        };
        let error = select("x", "x", matches, &selection).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("the pattern occurs once, expected 2")
        );
    }

    #[test]
    fn nearest_ignores_short_words_and_blank_pattern_lines() {
        assert!(nearest("a b c\n", "a b").is_empty());
        assert!(nearest("text\n", "\n  \n").is_empty());
        let found = nearest("alpha beta\nbeta\n", "\nalpha beta gamma delta");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 1);
    }

    #[test]
    fn nearest_limits_and_orders_fragments() {
        let text = "one two\nthree\none two three\none two\none two\n";
        let lines: Vec<usize> = nearest(text, "one two three")
            .iter()
            .map(|item| item.line)
            .collect();
        assert_eq!(lines, [3, 1, 4]);
    }

    #[test]
    fn excerpt_is_trimmed_and_limited() {
        let line = format!("   {}   ", "x".repeat(EXCERPT_CHARS + 10));
        assert_eq!(excerpt(&line).chars().count(), EXCERPT_CHARS);
    }

    #[test]
    fn describes_compile_errors() {
        assert_eq!(MatchError::PatternTooLarge.code(), ErrorCode::Limit);
        assert_eq!(
            MatchError::PatternTooLarge.to_string(),
            "the pattern is too large to match; split the change into smaller patterns"
        );
        assert_eq!(
            MatchError::EmptyPattern.to_string(),
            "the pattern is empty; give the text to replace"
        );
    }

    #[test]
    fn adapts_replacement_to_crlf_text() {
        let text = "a\r\nold\r\nb\r\n";
        assert_eq!(
            replace_once(text, "old", "new\nline", Whitespace::Normalize),
            "a\r\nnew\r\nline\r\nb\r\n"
        );
    }

    #[test]
    fn keeps_lf_in_lf_text() {
        assert_eq!(
            replace_once("a\nold\n", "old", "x\r\ny", Whitespace::Normalize),
            "a\nx\ny\n"
        );
    }

    #[test]
    fn reindents_replacement_of_whole_lines_to_text_indentation() {
        let text = "def f():\n    if x:\n        y()\n    return 1\n";
        let result = replace_once(
            text,
            "if x:\n    y()\n",
            "if x:\n    y()\nelse:\n    z()\n",
            Whitespace::Normalize,
        );
        assert_eq!(
            result,
            "def f():\n    if x:\n        y()\n    else:\n        z()\n    return 1\n"
        );
    }

    #[test]
    fn converts_indentation_unit_to_tabs() {
        let text = "fn f() {\n\tif x {\n\t\ty();\n\t}\n}\n";
        let result = replace_once(
            text,
            "    if x {\n        y();\n    }\n",
            "    if x {\n        y();\n        z();\n    }\n",
            Whitespace::Normalize,
        );
        assert_eq!(result, "fn f() {\n\tif x {\n\t\ty();\n\t\tz();\n\t}\n}\n");
    }

    #[test]
    fn keeps_blank_lines_and_unaligned_indentation_when_reindenting() {
        let text = "\tif x {\n\t\ty();\n\t}\n";
        let result = replace_once(
            text,
            "  if x {\n    y();\n  }\n",
            "  if x {\n\n   odd();\n z();\n  }\n",
            Whitespace::Normalize,
        );
        assert_eq!(result, "\tif x {\n\n\t odd();\n\tz();\n\t}\n");
    }

    #[test]
    fn keeps_replacement_when_indentation_already_matches() {
        let text = "  a\n  b\n";
        assert_eq!(
            replace_once(text, "  a \n  b", "  c\n  d", Whitespace::Normalize),
            "  c\n  d\n"
        );
    }

    #[test]
    fn trailing_match_keeps_replacement_indentation() {
        let text = "    alpha   \n";
        assert_eq!(
            replace_once(
                text,
                "    alpha \n",
                "    beta\n    gamma\n",
                Whitespace::Trailing
            ),
            "    beta\n    gamma\n"
        );
    }

    #[test]
    fn hanging_indent_continues_following_lines() {
        let text = "{\n    call();\n}\n";
        assert_eq!(
            replace_once(
                text,
                "call();",
                "first();\nsecond();",
                Whitespace::Normalize
            ),
            "{\n    first();\n    second();\n}\n"
        );
    }

    #[test]
    fn no_hanging_indent_after_code_on_the_line() {
        let text = "x = call();\n";
        assert_eq!(
            replace_once(text, "call();", "a +\nb;", Whitespace::Normalize),
            "x = a +\nb;\n"
        );
    }

    #[test]
    fn exact_multiline_match_keeps_replacement() {
        let text = "  a\n  b\n";
        assert_eq!(
            replace_once(text, "a\n  b", "c\n  d", Whitespace::Normalize),
            "  c\n  d\n"
        );
    }

    #[test]
    fn exact_setting_inserts_replacement_verbatim_except_line_endings() {
        let text = "{\r\n    call();\r\n}\r\n";
        assert_eq!(
            replace_once(text, "call();", "first();\nsecond();", Whitespace::Exact),
            "{\r\n    first();\r\nsecond();\r\n}\r\n"
        );
    }

    #[test]
    fn detects_indentation_units() {
        assert_eq!(indent_unit("a\n    b\n        c\n"), "    ");
        assert_eq!(indent_unit("a\n\tb\n\t\tc\n"), "\t");
        assert_eq!(indent_unit("a\nb\n"), "  ");
        assert_eq!(indent_unit("a\n  b\n    c\n      d\n\n"), "  ");
        assert_eq!(indent_unit("a\n  b\na\n    b\na\n    b\n"), "    ");
    }

    #[test]
    fn converts_only_whole_units() {
        assert_eq!(convert_unit("        ", "    ", "\t"), "\t\t");
        assert_eq!(convert_unit("      ", "    ", "\t"), "      ");
        assert_eq!(convert_unit("  \t", "  ", "\t"), "  \t");
        assert_eq!(convert_unit("", "  ", "\t"), "");
    }

    #[test]
    fn detects_prevailing_line_ending() {
        assert_eq!(detect_line_ending("a\r\nb\r\nc\n"), "\r\n");
        assert_eq!(detect_line_ending("a\r\nb\nc\n"), "\n");
        assert_eq!(detect_line_ending("abc"), "\n");
    }

    #[test]
    fn splits_lines_without_terminators() {
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("\n"), [""]);
        assert_eq!(split_lines("a\r\nb"), ["a", "b"]);
        assert_eq!(split_lines("a\n\nb\n"), ["a", "", "b"]);
    }

    #[test]
    fn indexes_lines() {
        let index = LineIndex::new("a\nb\n");
        assert_eq!(index.line_count(), 2);
        assert_eq!(index.row_start(1), 2);
        assert_eq!(index.row_start(5), 4);
        assert_eq!(index.line_start_of(3), 2);
        assert_eq!(LineIndex::new("").line_count(), 0);
        assert_eq!(LineIndex::new("a\nb").line_count(), 2);
    }

    #[test]
    fn trims_blank_edges() {
        assert_eq!(trim_blank_edges(" \n\t\n a\n b \n"), " a\n b");
        assert_eq!(trim_blank_edges("a"), "a");
    }

    #[test]
    fn compares_shapes_of_line_lists() {
        assert!(same_shape(&["a", "  b", "c"], &["x", "\tb", "c"]));
        assert!(!same_shape(&["a", "  b"], &["a", "b"]));
        assert!(!same_shape(
            &["a", "    b", "  c"],
            &["a", "    b", "      c"]
        ));
        assert!(same_shape(&["a", "", "b"], &["a", "  ", "b"]));
    }
}
