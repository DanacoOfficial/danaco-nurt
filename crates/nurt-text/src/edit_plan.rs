// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-text::edit_plan
// Responsibility:   Planning of content changes: replacements, insertions and line deletions.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Planning of content changes: replacements, insertions and line deletions.
//!
//! [`plan`] computes the text that results from a list of [`Change`]s. Every
//! change is located in the text as it was before the edit: line numbers and
//! pattern matches of a change do not move because of the changes listed
//! before it. The located ranges are then checked against each other, and
//! overlapping ranges of different changes are rejected before any result is
//! produced, so a plan applies all changes or none.
//!
//! Two insertions at the same position keep the order of the list, and an
//! insertion at the position where a deletion or replacement starts comes
//! before it. Deleting lines `a..b` and inserting before line `a` therefore
//! replaces those lines.
//!
//! Inserted lines and replacements take the prevailing line ending of the
//! text. A text without a final line break keeps that form after a line
//! change at its end, unless `ensure_newline` is set.

use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;
use std::ops::Range as Span;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::Range;

use crate::fuzzy_match::{
    self, Adapter, LineIndex, Match, MatchError, MatchOptions, OnMissing, Selection, Tolerance,
    Whitespace,
};
use crate::template::{CaptureSource, Groups, Template, TemplateError};

/// Pattern of a replacement.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// Literal text matched with the tolerance of [`Whitespace`].
    Literal(String),
    /// Regular expression of the linear-time engine; always matched exactly.
    Regex(regex::Regex),
    /// Regular expression of the backtracking engine, for look-around and
    /// back-references; always matched exactly.
    Backtracking(fancy_regex::Regex),
}

impl Pattern {
    fn groups(&self) -> Groups {
        match self {
            Self::Literal(_) => Groups::default(),
            Self::Regex(regex) => Groups::from_names(regex.capture_names()),
            Self::Backtracking(regex) => Groups::from_names(regex.capture_names()),
        }
    }
}

/// Replacement of the matches of a pattern.
#[derive(Debug, Clone)]
pub struct ReplaceChange {
    /// Pattern to replace.
    pub pattern: Pattern,
    /// Replacement; a template for regular expressions when `template` is
    /// set, otherwise the text as it is.
    pub with: String,
    /// Whether `with` is a [`Template`] for regular expressions; literal
    /// patterns always insert `with` as it is.
    pub template: bool,
    /// Number of the only match to replace, from 1; `None` replaces all.
    pub occurrence: Option<NonZeroUsize>,
    /// Required number of matches, counted before `occurrence` applies.
    pub expect: Option<Range>,
    /// Tolerance of a literal pattern; `None` takes the value of the edit.
    pub whitespace: Option<Whitespace>,
    /// Handling of a pattern without matches; `None` takes the value of the
    /// edit.
    pub on_missing: Option<OnMissing>,
    /// Whether letters of a literal pattern match only in the same case.
    pub case_sensitive: bool,
    /// Whether a match must not continue a word on either side.
    pub whole_word: bool,
}

impl ReplaceChange {
    /// Replacement of all matches with the default options of `replace`.
    #[must_use]
    pub fn new(pattern: Pattern, with: impl Into<String>) -> Self {
        Self {
            pattern,
            with: with.into(),
            template: true,
            occurrence: None,
            expect: None,
            whitespace: None,
            on_missing: None,
            case_sensitive: true,
            whole_word: false,
        }
    }

    /// Replacement template checked against the groups of the pattern.
    ///
    /// # Errors
    ///
    /// Returns a [`TemplateError`] when `with` is not a valid template for a
    /// regular expression pattern with `template` set.
    pub fn template(&self) -> Result<Template, TemplateError> {
        if self.template && !matches!(self.pattern, Pattern::Literal(_)) {
            Template::parse(&self.with, &self.pattern.groups())
        } else {
            Ok(Template::literal(&self.with))
        }
    }
}

/// Place of an insertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Position {
    /// Before the line.
    Before,
    /// After the line.
    #[default]
    After,
    /// At the start of the text; the line is not used.
    Start,
    /// At the end of the text; the line is not used.
    End,
}

impl Position {
    /// Value with the name used in the catalog: `before`, `after`, `start`
    /// or `end`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "before" => Some(Self::Before),
            "after" => Some(Self::After),
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            _ => None,
        }
    }

    /// Name used in the catalog.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::Start => "start",
            Self::End => "end",
        }
    }
}

/// Insertion of whole lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertChange {
    /// Lines to insert; a missing final line break is added.
    pub text: String,
    /// Line, from 1, that `before` and `after` refer to.
    pub line: Option<i64>,
    /// Place of the insertion.
    pub position: Position,
}

/// One change of an edit.
#[derive(Debug, Clone)]
pub enum Change {
    /// Replacement of pattern matches.
    Replace(ReplaceChange),
    /// Insertion of lines.
    Insert(InsertChange),
    /// Deletion of the lines in the range, from 1; open bounds reach the
    /// first or the last line.
    DeleteLines(Range),
}

/// Options that apply to every change of an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditOptions {
    /// Tolerance of literal patterns without their own value.
    pub whitespace: Whitespace,
    /// Handling of patterns without matches and without their own value.
    pub on_missing: OnMissing,
    /// Required total number of replacements made by `replace` changes.
    pub expect: Option<Range>,
    /// Whether a non-empty result ends with a line break.
    pub ensure_newline: bool,
}

impl Default for EditOptions {
    /// Defaults of the `edit` command: `whitespace: normalize`,
    /// `on_missing: skip`, no `expect`, no `ensure_newline`.
    fn default() -> Self {
        Self {
            whitespace: Whitespace::Normalize,
            on_missing: OnMissing::Skip,
            expect: None,
            ensure_newline: false,
        }
    }
}

/// Effect of one change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeOutcome {
    /// Replaced matches, inserted blocks (0 or 1) or deleted lines.
    pub count: usize,
    /// Replaced matches in the text before the edit; empty for other changes.
    pub matches: Vec<Match>,
}

/// Result of [`plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditPlan {
    /// Text after all changes.
    pub text: String,
    /// Effect of each change, in the order of the list.
    pub outcomes: Vec<ChangeOutcome>,
}

/// Change that cannot be planned. Changes are numbered from 0 in the order of
/// the list and shown from 1 in messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The pattern of a replacement failed to match or to select.
    Match {
        /// Index of the change.
        change: usize,
        /// The failure.
        source: MatchError,
    },
    /// The replacement template is not valid.
    Template {
        /// Index of the change.
        change: usize,
        /// The failure.
        source: TemplateError,
    },
    /// The backtracking engine exceeded its limit.
    BacktrackLimit {
        /// Index of the change.
        change: usize,
    },
    /// A line number lies outside the text.
    LineOutOfRange {
        /// Index of the change.
        change: usize,
        /// Line as given.
        line: i64,
        /// Number of lines of the text.
        lines: usize,
    },
    /// An insertion before or after a line names no line.
    MissingLine {
        /// Index of the change.
        change: usize,
        /// Position of the insertion.
        position: Position,
    },
    /// The ranges of two changes overlap.
    Conflict {
        /// Index of the earlier change in the list.
        first: usize,
        /// Index of the later change in the list.
        second: usize,
    },
    /// The total number of replacements is outside `expect` of the edit.
    TotalMismatch {
        /// Required total.
        expected: Range,
        /// Total of all `replace` changes.
        found: usize,
    },
}

impl EditError {
    /// Error code: the code of the source for [`EditError::Match`],
    /// `E_REGEX` for a template, `E_LIMIT` for the backtracking limit,
    /// `E_OUT_OF_RANGE` for a line outside the text, `E_INVALID_VALUE` for
    /// overlapping changes, `E_MISSING_PARAM` for a missing line and `E_ASSERT` for a
    /// total outside `expect`.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Match { source, .. } => source.code(),
            Self::Template { source, .. } => source.code(),
            Self::BacktrackLimit { .. } => ErrorCode::Limit,
            Self::LineOutOfRange { .. } => ErrorCode::OutOfRange,
            Self::Conflict { .. } => ErrorCode::InvalidValue,
            Self::MissingLine { .. } => ErrorCode::MissingParam,
            Self::TotalMismatch { .. } => ErrorCode::Assert,
        }
    }
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Match { change, source } => write!(f, "change {}: {source}", change + 1),
            Self::Template { change, source } => write!(f, "change {}: {source}", change + 1),
            Self::BacktrackLimit { change } => write!(
                f,
                "change {}: the regular expression exceeded the backtracking limit; simplify the pattern",
                change + 1
            ),
            Self::LineOutOfRange {
                change,
                line,
                lines,
            } => write!(
                f,
                "change {}: line {line} is outside the text of {lines} lines; give a line from 1 to {lines}",
                change + 1
            ),
            Self::MissingLine { change, position } => write!(
                f,
                "change {}: position `{}` requires `line`; give the line or use position `start` or `end`",
                change + 1,
                position.as_str()
            ),
            Self::Conflict { first, second } => write!(
                f,
                "changes {} and {} overlap; merge them into one change or adjust their ranges",
                first + 1,
                second + 1
            ),
            Self::TotalMismatch { expected, found } => write!(
                f,
                "the changes replace {found} matches, expected {expected}; adjust the patterns or `expect`"
            ),
        }
    }
}

impl Error for EditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Match { source, .. } => Some(source),
            Self::Template { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Computes the result of a list of changes on a text.
///
/// Every change is located in `text` as it is before the edit. A replacement
/// with zero matches and `on_missing: skip` has an outcome with `count` 0.
///
/// # Errors
///
/// Returns an [`EditError`] for the first change that cannot be located, for
/// two changes whose ranges overlap and for a total of replacements outside
/// `options.expect`. No partial result is produced.
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Range;
/// use nurt_text::edit_plan::{plan, Change, EditOptions, InsertChange, Pattern, Position, ReplaceChange};
///
/// let text = "a\nb\nc\nd\n";
/// let changes = [
///     Change::DeleteLines(Range::new(Some(2), Some(3)).unwrap()),
///     Change::Insert(InsertChange { text: "B".into(), line: Some(2), position: Position::Before }),
///     Change::Replace(ReplaceChange::new(Pattern::Literal("d".into()), "D")),
/// ];
/// let edit = plan(text, &changes, &EditOptions::default())?;
/// assert_eq!(edit.text, "a\nB\nD\n");
/// assert_eq!(edit.outcomes[0].count, 2);
/// # Ok::<(), nurt_text::edit_plan::EditError>(())
/// ```
pub fn plan(text: &str, changes: &[Change], options: &EditOptions) -> Result<EditPlan, EditError> {
    let planner = Planner {
        text,
        lines: LineIndex::new(text),
        line_ending: fuzzy_match::detect_line_ending(text),
        options,
    };
    let mut splices = Vec::new();
    let mut outcomes = Vec::with_capacity(changes.len());
    for (index, change) in changes.iter().enumerate() {
        let outcome = match change {
            Change::Replace(replace) => planner.replace(index, replace, &mut splices)?,
            Change::Insert(insert) => planner.insert(index, insert, &mut splices)?,
            Change::DeleteLines(range) => planner.delete_lines(index, *range, &mut splices)?,
        };
        outcomes.push(outcome);
    }
    if let Some(expected) = options.expect {
        let found = changes
            .iter()
            .zip(&outcomes)
            .filter(|(change, _)| matches!(change, Change::Replace(_)))
            .map(|(_, outcome)| outcome.count)
            .sum::<usize>();
        if !expected.contains(i64::try_from(found).unwrap_or(i64::MAX)) {
            return Err(EditError::TotalMismatch { expected, found });
        }
    }
    splices.sort_by_key(|splice| (splice.span.start, splice.span.end, splice.change));
    check_conflicts(&splices)?;
    Ok(EditPlan {
        text: planner.apply(&splices),
        outcomes,
    })
}

/// Located change: the span of the text before the edit and its new content.
struct Splice {
    span: Span<usize>,
    text: String,
    change: usize,
    lines: bool,
}

struct Planner<'a> {
    text: &'a str,
    lines: LineIndex,
    line_ending: &'static str,
    options: &'a EditOptions,
}

impl Planner<'_> {
    fn replace(
        &self,
        index: usize,
        change: &ReplaceChange,
        splices: &mut Vec<Splice>,
    ) -> Result<ChangeOutcome, EditError> {
        let selection = Selection {
            occurrence: change.occurrence,
            expect: change.expect,
            on_missing: change.on_missing.unwrap_or(self.options.on_missing),
        };
        let template = change.template().map_err(|source| EditError::Template {
            change: index,
            source,
        })?;
        let (matches, replacements) = match &change.pattern {
            Pattern::Literal(pattern) => {
                self.literal_matches(index, change, pattern, &change.with)?
            }
            Pattern::Regex(regex) => self.regex_matches(change, &template, regex),
            Pattern::Backtracking(regex) => {
                self.backtracking_matches(index, change, &template, regex)?
            }
        };
        let pattern = match &change.pattern {
            Pattern::Literal(pattern) => pattern.as_str(),
            Pattern::Regex(_) | Pattern::Backtracking(_) => "",
        };
        let kept =
            fuzzy_match::selected(self.text, pattern, &matches, &selection).map_err(|source| {
                EditError::Match {
                    change: index,
                    source,
                }
            })?;
        let chosen: Vec<Match> = matches[kept.clone()].to_vec();
        for (found, replacement) in chosen.iter().zip(&replacements[kept]) {
            splices.push(Splice {
                span: found.start..found.end,
                text: replacement.clone(),
                change: index,
                lines: false,
            });
        }
        Ok(ChangeOutcome {
            count: chosen.len(),
            matches: chosen,
        })
    }

    fn literal_matches(
        &self,
        index: usize,
        change: &ReplaceChange,
        pattern: &str,
        with: &str,
    ) -> Result<(Vec<Match>, Vec<String>), EditError> {
        let whitespace = change.whitespace.unwrap_or(self.options.whitespace);
        let options = MatchOptions {
            whitespace,
            case_sensitive: change.case_sensitive,
            whole_word: change.whole_word,
        };
        let matches =
            fuzzy_match::find(self.text, pattern, &options).map_err(|source| EditError::Match {
                change: index,
                source,
            })?;
        let adapter = Adapter::new(self.text, pattern, whitespace);
        let replacements = matches
            .iter()
            .map(|found| adapter.adapt(found, with))
            .collect();
        Ok((matches, replacements))
    }

    fn regex_matches(
        &self,
        change: &ReplaceChange,
        template: &Template,
        regex: &regex::Regex,
    ) -> (Vec<Match>, Vec<String>) {
        let mut located = Located::default();
        if template.is_literal() {
            for found in regex.find_iter(self.text) {
                located.push(self, change, found.range(), || {
                    expand(template, &NoCaptures)
                });
            }
        } else {
            for captures in regex.captures_iter(self.text) {
                let span = captures.get(0).map_or(0..0, |found| found.range());
                located.push(self, change, span, || expand(template, &captures));
            }
        }
        located.into_parts()
    }

    fn backtracking_matches(
        &self,
        index: usize,
        change: &ReplaceChange,
        template: &Template,
        regex: &fancy_regex::Regex,
    ) -> Result<(Vec<Match>, Vec<String>), EditError> {
        let limit = |_| EditError::BacktrackLimit { change: index };
        let mut located = Located::default();
        for captures in regex.captures_iter(self.text) {
            let captures = captures.map_err(limit)?;
            let span = captures.get(0).map_or(0..0, |found| found.range());
            located.push(self, change, span, || expand(template, &captures));
        }
        Ok(located.into_parts())
    }

    fn insert(
        &self,
        index: usize,
        change: &InsertChange,
        splices: &mut Vec<Splice>,
    ) -> Result<ChangeOutcome, EditError> {
        let count = self.lines.line_count();
        let offset = match change.position {
            Position::Start => 0,
            Position::End => self.text.len(),
            Position::Before | Position::After => {
                let line = change.line.ok_or(EditError::MissingLine {
                    change: index,
                    position: change.position,
                })?;
                let before = change.position == Position::Before;
                let last = if before { count + 1 } else { count };
                let row = row_of(line, last).ok_or(EditError::LineOutOfRange {
                    change: index,
                    line,
                    lines: count,
                })?;
                self.lines.row_start(if before { row - 1 } else { row })
            }
        };
        if change.text.is_empty() {
            return Ok(ChangeOutcome::default());
        }
        let mut lines = fuzzy_match::to_line_ending(&change.text, self.line_ending);
        if !lines.ends_with('\n') {
            lines.push_str(self.line_ending);
        }
        splices.push(Splice {
            span: offset..offset,
            text: lines,
            change: index,
            lines: true,
        });
        Ok(ChangeOutcome {
            count: 1,
            matches: Vec::new(),
        })
    }

    fn delete_lines(
        &self,
        index: usize,
        range: Range,
        splices: &mut Vec<Splice>,
    ) -> Result<ChangeOutcome, EditError> {
        let count = self.lines.line_count();
        let first = range.start().unwrap_or(1);
        let last = range
            .end()
            .unwrap_or_else(|| i64::try_from(count).unwrap_or(i64::MAX));
        let row = |line: i64| {
            row_of(line, count).ok_or(EditError::LineOutOfRange {
                change: index,
                line,
                lines: count,
            })
        };
        let (first_row, last_row) = (row(first)?, row(last)?);
        splices.push(Splice {
            span: self.lines.row_start(first_row - 1)..self.lines.row_start(last_row),
            text: String::new(),
            change: index,
            lines: true,
        });
        Ok(ChangeOutcome {
            count: last_row - first_row + 1,
            matches: Vec::new(),
        })
    }

    fn apply(&self, splices: &[Splice]) -> String {
        let text = self.text;
        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;
        let mut lines_at_end = false;
        for splice in splices {
            output.push_str(&text[cursor..splice.span.start]);
            if splice.lines
                && !splice.text.is_empty()
                && !output.is_empty()
                && !output.ends_with('\n')
            {
                output.push_str(self.line_ending);
            }
            output.push_str(&splice.text);
            cursor = splice.span.end;
            lines_at_end |= splice.lines && splice.span.end == text.len();
        }
        output.push_str(&text[cursor..]);
        let open = !text.is_empty() && !text.ends_with('\n');
        if self.options.ensure_newline {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push_str(self.line_ending);
            }
        } else if open && lines_at_end {
            let terminator = if output.ends_with("\r\n") {
                2
            } else {
                usize::from(output.ends_with('\n'))
            };
            output.truncate(output.len() - terminator);
        }
        output
    }
}

/// Matches of a regular expression with their expanded replacements.
#[derive(Default)]
struct Located {
    matches: Vec<Match>,
    replacements: Vec<String>,
}

impl Located {
    fn push(
        &mut self,
        planner: &Planner<'_>,
        change: &ReplaceChange,
        span: Span<usize>,
        replacement: impl FnOnce() -> String,
    ) {
        if change.whole_word && !fuzzy_match::at_word_boundaries(planner.text, span.clone()) {
            return;
        }
        self.matches
            .push(planner.lines.locate(planner.text, span, Tolerance::Exact));
        self.replacements.push(fuzzy_match::to_line_ending(
            &replacement(),
            planner.line_ending,
        ));
    }

    fn into_parts(self) -> (Vec<Match>, Vec<String>) {
        (self.matches, self.replacements)
    }
}

fn expand(template: &Template, captures: &dyn CaptureSource) -> String {
    let mut out = String::new();
    template.expand_into(captures, &mut out);
    out
}

/// Capture source of a template without group references.
struct NoCaptures;

impl CaptureSource for NoCaptures {
    fn group(&self, _index: usize) -> Option<&str> {
        None
    }
}

/// Line number as a row from 1 when it lies in `1..=last`.
fn row_of(line: i64, last: usize) -> Option<usize> {
    usize::try_from(line)
        .ok()
        .filter(|row| (1..=last).contains(row))
}

/// Rejects a splice that starts inside the span of an earlier one; an
/// insertion conflicts only when it lies strictly inside another span.
fn check_conflicts(splices: &[Splice]) -> Result<(), EditError> {
    let mut reach: Option<&Splice> = None;
    for splice in splices {
        if let Some(held) = reach {
            let inside = if splice.span.is_empty() {
                held.span.start < splice.span.start && splice.span.start < held.span.end
            } else {
                splice.span.start < held.span.end
            };
            if inside {
                return Err(EditError::Conflict {
                    first: held.change.min(splice.change),
                    second: held.change.max(splice.change),
                });
            }
        }
        if reach.is_none_or(|held| splice.span.end > held.span.end) {
            reach = Some(splice);
        }
    }
    Ok(())
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
