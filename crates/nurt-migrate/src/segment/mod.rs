// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Division of a script into comments and statements with tree-sitter.
//!
//! A statement is a top-level command, pipeline, assignment or list. Lists
//! joined with `&&` or `||`, and statements joined with `;` on one line, form
//! one statement with several parts, so that a rule spanning the whole line
//! (`cd D && CMD`) still applies. Loops, conditions, definitions, blocks and
//! dynamic evaluation are recognized by structure and never translated.

mod bash;
mod powershell;

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use tree_sitter::{Node, Parser};

use crate::language::ScriptLanguage;
use crate::report::Reason;

/// Line and column of a source position; both start at 1 and the column
/// counts Unicode scalar values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Position {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

/// Comment or statement of a script, in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) start: Position,
    pub(crate) end_line: u32,
    pub(crate) range: Range<usize>,
    pub(crate) kind: SegmentKind,
}

impl Segment {
    pub(crate) fn text<'s>(&self, source: &'s str) -> &'s str {
        source
            .get(self.range.clone())
            .unwrap_or_default()
            .trim_end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    /// Comment lines in program syntax, each starting with `#`.
    Comment(Vec<String>),
    /// Statement submitted for translation.
    Statement(Statement),
    /// Construct without an equivalent, recognized by structure.
    Construct(Reason),
    /// Statement containing a syntax error at the given position.
    Invalid(Position),
}

/// Parts of a statement and the operators between them; `joins` has one
/// element fewer than `parts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Statement {
    pub(crate) parts: Vec<Part>,
    pub(crate) joins: Vec<Join>,
}

/// Operator joining two parts of a statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Join {
    /// `&&`: the right part runs when the left part succeeds.
    And,
    /// `||`: the right part runs when the left part fails.
    Or,
    /// `;`: the right part runs after the left part in any case.
    Sequence,
}

/// One command, pipeline or assignment of a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Part {
    pub(crate) start: Position,
    pub(crate) range: Range<usize>,
    /// Variable assigned by the part, without `$`.
    pub(crate) variable: Option<String>,
    /// Target of a change of the current directory written as a literal.
    pub(crate) directory: Option<String>,
    /// The part contains a here-document.
    pub(crate) here_document: bool,
}

impl Part {
    pub(crate) fn text<'s>(&self, source: &'s str) -> &'s str {
        source
            .get(self.range.clone())
            .unwrap_or_default()
            .trim_end()
    }
}

/// Classification of one top-level node.
pub(crate) enum Analysis {
    Statement(Statement),
    Construct(Reason),
    Invalid(Position),
}

/// Divides a script into segments.
///
/// # Errors
///
/// Returns `E_INTERNAL` when the grammar cannot be loaded or the parser
/// returns no tree.
pub(crate) fn segment(source: &str, language: ScriptLanguage) -> Result<Vec<Segment>, Error> {
    let mut parser = Parser::new();
    let grammar = match language {
        ScriptLanguage::Bash => tree_sitter_bash::LANGUAGE,
        ScriptLanguage::PowerShell => tree_sitter_powershell::LANGUAGE,
    };
    parser.set_language(&grammar.into()).map_err(|error| {
        Error::new(
            ErrorCode::Internal,
            format!("cannot load the {language} grammar: {error}"),
        )
    })?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        Error::new(
            ErrorCode::Internal,
            format!("the {language} parser returned no tree"),
        )
    })?;
    let mut builder = Builder::new(source);
    match language {
        ScriptLanguage::Bash => bash::segment(&mut builder, tree.root_node()),
        ScriptLanguage::PowerShell => powershell::segment(&mut builder, tree.root_node()),
    }
    Ok(builder.segments)
}

/// Collects segments and joins statements separated by `;` on one line.
pub(crate) struct Builder<'s> {
    source: &'s str,
    segments: Vec<Segment>,
    after_separator: bool,
}

impl<'s> Builder<'s> {
    fn new(source: &'s str) -> Self {
        Self {
            source,
            segments: Vec::new(),
            after_separator: false,
        }
    }

    pub(crate) fn source(&self) -> &'s str {
        self.source
    }

    /// Adds a comment; the `#!` line at the start of the script is dropped.
    pub(crate) fn comment(&mut self, node: Node<'_>, lines: Vec<String>) {
        self.after_separator = false;
        if node.start_byte() == 0 && self.source.starts_with("#!") {
            return;
        }
        self.push(node, SegmentKind::Comment(lines));
    }

    /// Records a `;` between two statements.
    pub(crate) fn separator(&mut self) {
        self.after_separator = true;
    }

    /// Extends the last statement with a trailing background operator `&`.
    pub(crate) fn background(&mut self, node: Node<'_>) {
        self.after_separator = false;
        let Some(last) = self.segments.last_mut() else {
            return;
        };
        last.range.end = node.end_byte();
        if let SegmentKind::Statement(statement) = &mut last.kind
            && let Some(part) = statement.parts.last_mut()
        {
            part.range.end = node.end_byte();
        }
    }

    /// Adds a statement, joining it to the previous one after `;` on the
    /// same line.
    pub(crate) fn statement(&mut self, node: Node<'_>, analysis: Analysis) {
        let joined = std::mem::take(&mut self.after_separator);
        let kind = match analysis {
            Analysis::Statement(statement) => {
                if joined && self.join(node, &statement) {
                    return;
                }
                SegmentKind::Statement(statement)
            }
            Analysis::Construct(reason) => SegmentKind::Construct(reason),
            Analysis::Invalid(at) => {
                if self.absorb_error(node, at) {
                    return;
                }
                SegmentKind::Invalid(at)
            }
        };
        self.push(node, kind);
    }

    /// Makes the previous statement invalid when a syntax error starts on its
    /// last line; the parser closes a statement before an unterminated token.
    fn absorb_error(&mut self, node: Node<'_>, at: Position) -> bool {
        let start = self.position(node);
        let Some(last) = self.segments.last_mut() else {
            return false;
        };
        if matches!(last.kind, SegmentKind::Comment(_)) || last.end_line != start.line {
            return false;
        }
        last.kind = SegmentKind::Invalid(at);
        last.range.end = node.end_byte();
        last.end_line = end_line(node);
        true
    }

    fn join(&mut self, node: Node<'_>, next: &Statement) -> bool {
        let start = self.position(node);
        let Some(last) = self.segments.last_mut() else {
            return false;
        };
        let SegmentKind::Statement(statement) = &mut last.kind else {
            return false;
        };
        if last.end_line != start.line {
            return false;
        }
        statement.joins.push(Join::Sequence);
        statement.parts.extend(next.parts.iter().cloned());
        statement.joins.extend(next.joins.iter().copied());
        last.range.end = node.end_byte();
        last.end_line = end_line(node);
        true
    }

    fn push(&mut self, node: Node<'_>, kind: SegmentKind) {
        let segment = Segment {
            start: self.position(node),
            end_line: end_line(node),
            range: node.start_byte()..node.end_byte(),
            kind,
        };
        self.segments.push(segment);
    }

    pub(crate) fn position(&self, node: Node<'_>) -> Position {
        position(self.source, node)
    }

    /// Builds a part from a node, with the text trimmed at the end.
    pub(crate) fn part(&self, node: Node<'_>) -> Part {
        let text = node_text(self.source, node);
        Part {
            start: self.position(node),
            range: node.start_byte()..node.start_byte() + text.len(),
            variable: None,
            directory: None,
            here_document: false,
        }
    }

    /// Classifies a statement that contains a syntax error.
    pub(crate) fn invalid(&self, node: Node<'_>) -> Analysis {
        let at = first_error(node, &|_| false).unwrap_or(node);
        Analysis::Invalid(self.position(at))
    }

    /// Returns the classification of a statement with a syntax error other
    /// than the errors accepted by `tolerated`.
    pub(crate) fn syntax_error(
        &self,
        node: Node<'_>,
        tolerated: &impl Fn(Node<'_>) -> bool,
    ) -> Option<Analysis> {
        let at = first_error(node, tolerated)?;
        Some(Analysis::Invalid(self.position(at)))
    }
}

/// Position of the start of a node.
pub(crate) fn position(source: &str, node: Node<'_>) -> Position {
    let point = node.start_position();
    let start = node.start_byte();
    let column = start
        .checked_sub(point.column)
        .and_then(|line_start| source.get(line_start..start))
        .map_or(point.column, |prefix| prefix.chars().count());
    Position {
        line: to_u32(point.row.saturating_add(1)),
        column: to_u32(column.saturating_add(1)),
    }
}

/// Line of the last character of a node.
fn end_line(node: Node<'_>) -> u32 {
    let start = node.start_position();
    let end = node.end_position();
    let row = if end.column == 0 && end.row > start.row {
        end.row
    } else {
        end.row.saturating_add(1)
    };
    to_u32(row)
}

/// Text of a node without trailing white space.
pub(crate) fn node_text<'s>(source: &'s str, node: Node<'_>) -> &'s str {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or_default()
        .trim_end()
}

/// First node in source order that is an error or a missing token and is
/// not accepted by `tolerated`.
fn first_error<'t>(node: Node<'t>, tolerated: &impl Fn(Node<'_>) -> bool) -> Option<Node<'t>> {
    if node.is_error() || node.is_missing() {
        return (!tolerated(node)).then_some(node);
    }
    if !node.has_error() {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| first_error(child, tolerated))
}

/// Comment lines of a `#` comment.
pub(crate) fn line_comment(text: &str) -> Vec<String> {
    vec![text.trim_end().to_owned()]
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Returns the first construct without an equivalent found in a subtree, in
/// source order, as decided by a classifier of single nodes.
///
/// Subtrees for which `descend` returns `false` are not visited.
pub(crate) fn find_construct(
    node: Node<'_>,
    classify: &impl Fn(Node<'_>) -> Option<Reason>,
    descend: &impl Fn(Node<'_>) -> bool,
) -> Option<Reason> {
    if let Some(reason) = classify(node) {
        return Some(reason);
    }
    if !descend(node) {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| find_construct(child, classify, descend))
}

/// Returns `true` when any node of a subtree has the given kind.
pub(crate) fn contains_kind(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| contains_kind(child, kind))
}

#[cfg(test)]
mod differential;
