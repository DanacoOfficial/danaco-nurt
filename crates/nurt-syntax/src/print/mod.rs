// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::print
// Responsibility:   Canonical printer of surface trees.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Canonical printer of surface trees.
//!
//! For every tree `T` that the parser can produce, parsing the printed text
//! in the same variant gives `T` again, spans and metadata aside; printing
//! the parse of any program text gives its canonical text, the output of
//! `nurt fmt`. Trees without such a text form are listed at [`PrintError`];
//! literals and expressions are described at [`print_expr`].
//!
//! # Layout
//!
//! * Statements are separated by one line break and the text ends with a
//!   line break; there are no blank lines.
//! * A pipeline with stages after a head that is a stage with arguments is
//!   printed one stage per line: the head on the first line (after
//!   `let name = `), every further stage on its own line starting with `|> `.
//!   Any other pipeline is printed on one line, joined by ` |> `, when the
//!   line fits [`MAX_LINE_WIDTH`] columns, otherwise one stage per line.
//! * A sub-pipeline is always printed on one line.
//! * A list is printed one item per line, indented by two spaces per level
//!   and without a trailing comma, when it has at least two items and one of
//!   them is a list or a map, or when its one-line form would end beyond
//!   [`MAX_LINE_WIDTH`]. A map is broken under the second rule only.
//! * Columns count Unicode scalar values, not display width.
//!
//! # Variants
//!
//! | Variant | Stage | Sub-pipeline argument |
//! |---|---|---|
//! | A | `name arg name: value` | `(stage \|> stage)` |
//! | B | `name(arg, name: value)`, `name()` | `name(stage \|> stage)`; one stage as `name((stage))` |
//! | C | `name name=value` | `name=(stage \|> stage)` |
//!
//! Lists, maps, literals and expressions are alike in all variants; map
//! entries use `:` in variant C as well. Variant C has no positional
//! arguments: [`name_positional`] names them first.

mod expr;
mod layout;
mod literal;
mod names;

use std::fmt;

use nurt_contracts::ast::{AstDocument, Expr, Span};

use self::expr::{Ctx, Out, PREC_ANY, Printer};
pub(crate) use self::layout::Segment;
pub use self::names::{PositionalForm, PositionalNames, name_positional};
use crate::parse::Variant;

/// Width of a line, in Unicode scalar values, beyond which pipelines, lists
/// and maps are broken.
pub const MAX_LINE_WIDTH: usize = 100;

/// Deepest node nesting printed; equal to the nesting limit of the parser and
/// the decoders.
pub const MAX_NESTING_DEPTH: usize = 128;

/// Reason a tree has no text form that reads back as the same tree.
///
/// [`print`] still prints such trees; [`try_print`] rejects them:
///
/// * A structured `key` prints as the text of its key path and reads back
///   as `text`;
/// * A sub-pipeline of a reference only, or of one stage without arguments
///   in variants A and C, reads back as an expression;
/// * A regular expression that is empty, spans lines, has flags outside
///   `imsx` or out of order, ends with an odd number of `\`, has an odd
///   number of `\` before `/`, or starts with white space in variants A and
///   C;
/// * Names that are not identifiers or are reserved words, and command names
///   that are not identifiers joined by `.`;
/// * Negative or non-finite numbers, values outside `0..=2^53-1`, unary `-`
///   applied to a range, a spread outside a list, a field without a record
///   expression, a repeated map key and an empty program;
/// * Positional arguments in variant C, which [`print`] writes without a
///   name;
/// * Nesting deeper than [`MAX_NESTING_DEPTH`]; [`print`] writes `null` in
///   place of the deeper nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrintError {
    /// A positional argument in variant C, which has named arguments only.
    PositionalArgument {
        /// Source range of the argument.
        span: Option<Span>,
    },
    /// A positional argument for which [`PositionalNames`] has no name.
    UnnamedArgument {
        /// Command of the stage.
        command: String,
        /// Zero-based position among the positional arguments of the stage.
        index: usize,
        /// Source range of the argument.
        span: Option<Span>,
    },
    /// A node outside the image of the parser.
    Unprintable {
        /// Kind of the node.
        node: &'static str,
        /// What is wrong and how to correct it.
        problem: &'static str,
        /// Source range of the node.
        span: Option<Span>,
    },
    /// Nesting deeper than the limit.
    TooDeep {
        /// The limit, [`MAX_NESTING_DEPTH`].
        limit: usize,
    },
}

impl PrintError {
    /// Returns the source range of the offending node, when the tree has
    /// spans.
    #[must_use]
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::PositionalArgument { span }
            | Self::UnnamedArgument { span, .. }
            | Self::Unprintable { span, .. } => *span,
            Self::TooDeep { .. } => None,
        }
    }
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PositionalArgument { .. } => f.write_str(
                "variant C has no positional arguments; name every argument or print in variant A or B",
            ),
            Self::UnnamedArgument { command, index, .. } => write!(
                f,
                "`{command}` has no name for positional argument {index}; write it as `name: value`"
            ),
            Self::Unprintable { node, problem, .. } => write!(f, "`{node}` node {problem}"),
            Self::TooDeep { limit } => {
                write!(f, "the tree is nested deeper than {limit} levels; split the program")
            }
        }
    }
}

impl std::error::Error for PrintError {}

/// Prints a surface tree as canonical program text of `variant`, ending with
/// a line break.
///
/// The tree metadata, including comments, is not printed; `nurt fmt` keeps
/// comments through [`crate::fmt::format`]. A tree listed at [`PrintError`]
/// is printed as well, but its text does not read back as the same tree;
/// [`try_print`] detects such trees.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::{Arg, AstDocument, Expr, Head, Pipeline, Program, Stage, Statement};
/// use nurt_syntax::parse::Variant;
/// use nurt_syntax::print;
///
/// let path = Arg { name: None, value: Expr::Text { v: "/srv/app".to_owned(), span: None }, span: None };
/// let find = Stage { name: "find".to_owned(), args: vec![path], span: None };
/// let count = Stage { name: "count".to_owned(), args: Vec::new(), span: None };
/// let pipeline = Pipeline { head: Head::Stage(find), stages: vec![count], span: None };
/// let document = AstDocument::new(Program { statements: vec![Statement::Pipeline(pipeline)] });
/// assert_eq!(print::print(&document, Variant::A), "find \"/srv/app\"\n|> count\n");
/// assert_eq!(print::print(&document, Variant::B), "find(\"/srv/app\")\n|> count()\n");
/// ```
#[must_use]
pub fn print(document: &AstDocument, variant: Variant) -> String {
    render(document, variant).0
}

/// Prints a surface tree like [`print`], rejecting trees whose text would not
/// read back as the same tree.
///
/// # Errors
///
/// Returns the first [`PrintError`] in source order: a node outside the
/// parser image, a positional argument in variant C or nesting deeper than
/// [`MAX_NESTING_DEPTH`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::{Arg, AstDocument, Expr, Head, Pipeline, Program, Stage, Statement};
/// use nurt_syntax::parse::Variant;
/// use nurt_syntax::print::{self, PrintError};
///
/// let first = Stage {
///     name: "first".to_owned(),
///     args: vec![Arg { name: None, value: Expr::Int { v: 20, span: None }, span: None }],
///     span: None,
/// };
/// let document = AstDocument::new(Program {
///     statements: vec![Statement::Pipeline(Pipeline {
///         head: Head::Stage(first),
///         stages: Vec::new(),
///         span: None,
///     })],
/// });
/// assert_eq!(print::try_print(&document, Variant::A)?, "first 20\n");
/// assert!(matches!(
///     print::try_print(&document, Variant::C),
///     Err(PrintError::PositionalArgument { .. })
/// ));
/// # Ok::<(), PrintError>(())
/// ```
pub fn try_print(document: &AstDocument, variant: Variant) -> Result<String, PrintError> {
    match render(document, variant) {
        (text, None) => Ok(text),
        (_, Some(error)) => Err(error),
    }
}

/// Prints one expression in its canonical form, for messages, candidates and
/// help texts; a long list or map may span several lines.
///
/// Text is written `"…"` with the escapes `\\ \" \n \r \t \0` and `\u{hex}`
/// for other control characters (Unicode category Cc), or raw as `r#"…"#`
/// with the fewest `#` when it contains `"` or `\` and no control character
/// other than tab and line feed. Integers are decimal; numbers use the
/// shortest decimal that reads back to the same value, with a fraction and
/// without an exponent; sizes and durations use the shortest exact spelling
/// (`1mb`, `1.5kb`, `1m30s`, `1d`). Parentheses appear where precedence or
/// associativity requires them and where a token would merge with its
/// neighbour: around an argument of variant A that starts with `not` or `..`
/// after another token or ends with `..` before another argument, and
/// around a left operand that ends with `..`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Expr;
/// use nurt_syntax::parse::Variant;
/// use nurt_syntax::print;
///
/// let size = Expr::Size { v: 1_500, span: None };
/// assert_eq!(print::print_expr(&size, Variant::A), "1.5kb");
/// ```
#[must_use]
pub fn print_expr(expr: &Expr, variant: Variant) -> String {
    let mut printer = Printer::new(variant);
    let mut out = Out::default();
    printer.expr(expr, PREC_ANY, Ctx::TOP, &mut out);
    out.text
}

/// Lays out a document statement by statement, for formatting with
/// comments; returns the first reason the text does not read back, if any.
pub(crate) fn layout(
    document: &AstDocument,
    variant: Variant,
) -> (Vec<Vec<Segment>>, Option<PrintError>) {
    let mut printer = Printer::new(variant);
    let statements = printer.program(&document.program);
    (statements, printer.finish())
}

fn render(document: &AstDocument, variant: Variant) -> (String, Option<PrintError>) {
    let (statements, issue) = layout(document, variant);
    let mut text = String::new();
    for segment in statements.iter().flatten() {
        text.push_str(&segment.text);
        text.push('\n');
    }
    (text, issue)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
