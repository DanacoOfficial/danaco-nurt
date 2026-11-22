// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Formatting of program text into its canonical form (`nurt fmt`).
//!
//! [`format`] parses a program, prints it with the canonical printer
//! ([`crate::print`]) and puts its comments back:
//!
//! * a comment on a line of its own stays on a line of its own before the
//!   statement or stage that follows it;
//! * a comment at the end of a line stays at the end of the line of the
//!   statement or stage that ended on that line in the source;
//! * when the canonical layout joins source lines, or a comment stood inside
//!   a statement where no line ends, the comment moves to a line of its own
//!   before the statement; comments keep their source order;
//! * a first line starting with `#!` stays the first line;
//! * blank lines are removed.
//!
//! Formatting is idempotent, keeps every comment and does not change the
//! tree: the parse of the result equals the parse of the input apart from
//! spans and metadata.

use std::collections::BTreeMap;

use nurt_contracts::ast::{AstDocument, Head, Span, Statement};
use nurt_contracts::error::At;
use serde::Deserialize;

use crate::error::SyntaxError;
use crate::parse::{self, ParseOptions, Variant};
use crate::print::{self, PositionalNames, PrintError, Segment};

/// Start of an interpreter line on the first line of a program file.
const SHEBANG: &str = "#!";

/// Formats program text of `variant` into its canonical text, keeping
/// comments.
///
/// Without comments the result equals [`print::print`] of the parsed tree.
///
/// # Errors
///
/// Returns the [`SyntaxError`] of the parser when the text is not a valid
/// program of `variant`, and `E_SYNTAX` in the rare case that the program
/// has no text form that reads back unchanged (for example a regular
/// expression starting with white space).
///
/// # Examples
///
/// ```
/// use nurt_syntax::fmt;
/// use nurt_syntax::parse::Variant;
///
/// let text = "# largest Go files\nfind \"/srv/app\"   glob: \"*.go\" |> first 0x14 # top\n";
/// assert_eq!(
///     fmt::format(text, Variant::A)?,
///     "# largest Go files\nfind \"/srv/app\" glob: \"*.go\"\n|> first 20 # top\n"
/// );
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub fn format(text: &str, variant: Variant) -> Result<String, SyntaxError> {
    convert(text, variant, variant, None)
}

/// Converts program text from the variant `from` to the canonical text of
/// the variant `to`, keeping comments as [`format`] does.
///
/// Variant C names every argument: when `to` is [`Variant::C`] and the
/// program has positional arguments, `names` supplies their parameter names
/// through [`print::name_positional`].
///
/// # Errors
///
/// Returns the [`SyntaxError`] of the parser when the text is not a valid
/// program of `from`, and `E_SYNTAX` when a positional argument has no name
/// for variant C or the tree has no text form in `to` that reads back
/// unchanged.
///
/// # Examples
///
/// ```
/// use nurt_syntax::fmt;
/// use nurt_syntax::parse::Variant;
///
/// let text = "find \"/srv/app\" glob: \"*.go\" |> count # files\n";
/// assert_eq!(
///     fmt::convert(text, Variant::A, Variant::B, None)?,
///     "find(\"/srv/app\", glob: \"*.go\")\n|> count() # files\n"
/// );
/// # Ok::<(), nurt_syntax::error::SyntaxError>(())
/// ```
pub fn convert(
    text: &str,
    from: Variant,
    to: Variant,
    names: Option<&dyn PositionalNames>,
) -> Result<String, SyntaxError> {
    let options = ParseOptions {
        keep_comments: true,
        ..ParseOptions::default()
    };
    let mut document = parse::parse(text, from, &options)?;
    if to == Variant::C
        && let Some(names) = names
    {
        document = print::name_positional(&document, names).map_err(print_error)?;
    }
    let (statements, issue) = print::layout(&document, to);
    if let Some(issue) = issue {
        return Err(print_error(issue));
    }
    let comments = comments(&document)?;
    Ok(assemble(text, &document, &statements, comments))
}

/// Converts a printer failure into the error of the formatter.
fn print_error(error: PrintError) -> SyntaxError {
    let problem = match error {
        PrintError::PositionalArgument { .. } => "positional_argument",
        PrintError::UnnamedArgument { .. } => "unnamed_argument",
        PrintError::TooDeep { .. } => "nesting_too_deep",
        _ => "no_text_form",
    };
    let converted = SyntaxError::syntax(error.to_string()).with_problem(problem);
    match error.span() {
        Some(span) => converted.with_at(At {
            line: Some(span.line),
            column: Some(span.column),
            length: (span.end_line == span.line).then(|| span.end_column - span.column),
            stage: None,
        }),
        None => converted,
    }
}

/// Comment of the source text as stored by the parser.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct Comment {
    span: Span,
    text: String,
}

fn comments(document: &AstDocument) -> Result<Vec<Comment>, SyntaxError> {
    let Some(traces) = document.meta.as_ref().and_then(|meta| meta.get("comments")) else {
        return Ok(Vec::new());
    };
    Vec::<Comment>::deserialize(traces).map_err(|error| {
        SyntaxError::syntax(format!(
            "the comments of the program cannot be read: {error}"
        ))
        .with_problem("invalid_comment_trace")
    })
}

/// Lines occupied in the source by one element of a statement: the head
/// (index 0, from the start of the statement) or a stage (index `k`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Element {
    statement: usize,
    index: usize,
    start: u32,
    end: u32,
}

fn elements(document: &AstDocument) -> (Vec<Element>, Vec<Option<(u32, u32)>>) {
    let mut elements = Vec::new();
    let mut extents = Vec::with_capacity(document.program.statements.len());
    for (statement, node) in document.program.statements.iter().enumerate() {
        let (span, pipeline) = match node {
            Statement::Let(binding) => (binding.span, &binding.value),
            Statement::Pipeline(pipeline) => (pipeline.span, pipeline),
            _ => {
                extents.push(None);
                continue;
            }
        };
        let head = match &pipeline.head {
            Head::Stage(stage) => stage.span,
            Head::Ref(reference) => reference.span,
            _ => None,
        };
        let spans = std::iter::once(head).chain(pipeline.stages.iter().map(|stage| stage.span));
        for (index, element) in spans.enumerate() {
            if let Some(element) = element {
                let start = if index == 0 {
                    span.map_or(element.line, |span| span.line)
                } else {
                    element.line
                };
                elements.push(Element {
                    statement,
                    index,
                    start,
                    end: element.end_line,
                });
            }
        }
        extents.push(span.map(|span| (span.line, span.end_line)));
    }
    (elements, extents)
}

/// Where a comment is written in the formatted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Anchor {
    /// On its own line before the statement.
    BeforeStatement(usize),
    /// On its own line before the segment that starts with the element.
    BeforeElement(usize, usize),
    /// At the end of the segment that ends with the element.
    EndOf(usize, usize),
    /// On its own line after the last statement.
    Trailing,
}

fn anchor(
    comment: &Comment,
    own_line: bool,
    elements: &[Element],
    extents: &[Option<(u32, u32)>],
) -> Anchor {
    let line = comment.span.line;
    let enclosing = extents
        .iter()
        .position(|extent| extent.is_some_and(|(start, end)| start <= line && line <= end));
    if !own_line && let Some(element) = elements.iter().rev().find(|element| element.end == line) {
        return Anchor::EndOf(element.statement, element.index);
    }
    if let Some(statement) = enclosing {
        return elements
            .iter()
            .find(|element| element.statement == statement && element.start > line)
            .map_or(Anchor::BeforeStatement(statement), |element| {
                Anchor::BeforeElement(statement, element.index)
            });
    }
    elements
        .iter()
        .find(|element| element.start > line)
        .map_or(Anchor::Trailing, |element| {
            Anchor::BeforeStatement(element.statement)
        })
}

/// Returns `true` when only white space precedes the comment on its line.
fn is_own_line(lines: &[&str], comment: &Comment) -> bool {
    let index = usize::try_from(comment.span.line)
        .unwrap_or(usize::MAX)
        .saturating_sub(1);
    let before = usize::try_from(comment.span.column)
        .unwrap_or(usize::MAX)
        .saturating_sub(1);
    lines
        .get(index)
        .is_none_or(|line| line.chars().take(before).all(char::is_whitespace))
}

fn comment_line(comment: &Comment) -> String {
    format!("#{}", comment.text.trim_end_matches('\r'))
}

/// Resolves every anchor against the printed segments: an end-of-line
/// comment needs a segment that ends with its element and a comment before
/// an element needs a segment that starts with it; otherwise the comment
/// moves before its statement.
fn resolve(anchor: Anchor, statements: &[Vec<Segment>], taken: &mut Vec<(usize, usize)>) -> Anchor {
    let segments = |statement: usize| statements.get(statement).map_or(&[][..], Vec::as_slice);
    match anchor {
        Anchor::EndOf(statement, index) => {
            let ends = segments(statement)
                .iter()
                .any(|segment| segment.last == index);
            if ends && !taken.contains(&(statement, index)) {
                taken.push((statement, index));
                anchor
            } else {
                Anchor::BeforeStatement(statement)
            }
        }
        Anchor::BeforeElement(statement, index) => {
            if segments(statement)
                .iter()
                .any(|segment| segment.first == index)
            {
                anchor
            } else {
                Anchor::BeforeStatement(statement)
            }
        }
        other => other,
    }
}

fn assemble(
    source: &str,
    document: &AstDocument,
    statements: &[Vec<Segment>],
    comments: Vec<Comment>,
) -> String {
    let lines: Vec<&str> = source.split('\n').collect();
    let shebang = source.starts_with(SHEBANG);
    let (elements, extents) = elements(document);
    let mut placed: BTreeMap<Anchor, Vec<String>> = BTreeMap::new();
    let mut taken = Vec::new();
    for comment in comments {
        if shebang && comment.span.line == 1 {
            continue;
        }
        let anchor = anchor(&comment, is_own_line(&lines, &comment), &elements, &extents);
        let anchor = resolve(anchor, statements, &mut taken);
        placed
            .entry(anchor)
            .or_default()
            .push(comment_line(&comment));
    }
    let mut out = String::new();
    if shebang {
        let first = lines.first().map_or("", |line| line.trim_end_matches('\r'));
        out.push_str(first);
        out.push('\n');
    }
    for (statement, segments) in statements.iter().enumerate() {
        push_lines(&mut out, placed.get(&Anchor::BeforeStatement(statement)));
        for segment in segments {
            if segment.first > 0 {
                push_lines(
                    &mut out,
                    placed.get(&Anchor::BeforeElement(statement, segment.first)),
                );
            }
            out.push_str(&segment.text);
            if let Some(comment) = placed
                .get(&Anchor::EndOf(statement, segment.last))
                .and_then(|found| found.first())
            {
                out.push(' ');
                out.push_str(comment);
            }
            out.push('\n');
        }
    }
    push_lines(&mut out, placed.get(&Anchor::Trailing));
    out
}

fn push_lines(out: &mut String, lines: Option<&Vec<String>>) {
    for line in lines.into_iter().flatten() {
        out.push_str(line);
        out.push('\n');
    }
}
