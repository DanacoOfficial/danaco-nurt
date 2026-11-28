// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-code::spans
// Responsibility:   Classification of source spans as code, comments or strings.
// Dependencies:     nurt_contracts, tree_sitter
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Classification of source spans as code, comments or strings.
//!
//! Comments and string literals are recognized by the node kinds of the
//! syntax tree, so the classification follows the grammar of every language:
//! nested block comments, raw and multi-line strings, heredocs and
//! documentation comments are one span each. Interpolations inside string
//! literals (`${x}` in a JavaScript template, `{x}` in a Python f-string,
//! `#{x}` in Ruby) are code, and the comments and strings inside them are
//! classified again.
//!
//! ```
//! use nurt_code::spans::classify_spans;
//! use nurt_contracts::services::SpanClass;
//!
//! let source = b"let a = \"x\"; // note\n";
//! let mut parser = tree_sitter::Parser::new();
//! parser.set_language(&tree_sitter_rust::LANGUAGE.into())?;
//! let tree = parser.parse(source, None).ok_or("no tree")?;
//! let spans = classify_spans(&tree);
//! assert_eq!(spans.len(), 2);
//! assert_eq!(spans[0].class, SpanClass::String);
//! assert_eq!(&source[spans[1].bytes.start as usize..spans[1].bytes.end as usize], b"// note");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, SpanClass, SyntaxToken, TextPosition, TextSpan,
};
use tree_sitter::{Node, Tree};

/// Node kinds of string literals that the suffix rules do not cover.
const STRING_KINDS: [&str; 11] = [
    "shell_command_expression",
    "rune_literal",
    "character",
    "block_scalar",
    "heredoc",
    "heredoc_body",
    "nowdoc",
    "text_block",
    "attvalue",
    "string_value",
    "quoted_attribute_value",
];

/// Suffixes of node kinds of string literals.
const STRING_SUFFIXES: [&str; 6] = [
    "string",
    "string_literal",
    "string_characters",
    "char_literal",
    "character_literal",
    "quote_scalar",
];

/// Lexical class of a node kind; `None` for code.
#[must_use]
pub fn class_of_kind(kind: &str) -> Option<SpanClass> {
    let kind = kind.to_ascii_lowercase();
    if kind.contains("comment") {
        return Some(SpanClass::Comment);
    }
    let string = STRING_KINDS.contains(&kind.as_str())
        || STRING_SUFFIXES.iter().any(|suffix| kind.ends_with(suffix))
        || kind == "interpolated_string_expression";
    string.then_some(SpanClass::String)
}

/// Returns `true` for a node inside a string literal that holds code.
fn is_interpolation(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    (kind.contains("interpolat") && !kind.contains("string"))
        || kind.contains("substitution")
        || kind == "expansion"
        || kind == "simple_expansion"
}

/// Comments and string literals of a syntax tree, in source order and
/// without overlap; regions not listed are code.
#[must_use]
pub fn classify_spans(tree: &Tree) -> Vec<ClassifiedSpan> {
    let mut spans = Vec::new();
    classify(tree.root_node(), &mut spans);
    spans
}

/// Work of the classification: a node to visit, or a region of a literal or
/// comment to add.
enum Step<'t> {
    Visit(Node<'t>),
    Emit(usize, usize, SpanClass),
}

/// Classifies the subtree of `root` with an explicit stack, so that the depth
/// of the tree is not limited by the stack of the thread.
fn classify(root: Node<'_>, out: &mut Vec<ClassifiedSpan>) {
    let mut pending = vec![Step::Visit(root)];
    while let Some(step) = pending.pop() {
        let node = match step {
            Step::Emit(start, end, class) => {
                push(out, start, end, class);
                continue;
            }
            Step::Visit(node) => node,
        };
        let mut steps = Vec::new();
        match node
            .is_named()
            .then(|| class_of_kind(node.kind()))
            .flatten()
        {
            Some(class) => with_holes(node, class, &mut steps),
            None => {
                let mut cursor = node.walk();
                steps.extend(node.children(&mut cursor).map(Step::Visit));
            }
        }
        pending.extend(steps.into_iter().rev());
    }
}

/// Steps in source order that add the span of a literal or comment, leaving
/// out its interpolations, which are classified as code with their own
/// comments and strings.
fn with_holes<'t>(node: Node<'t>, class: SpanClass, steps: &mut Vec<Step<'t>>) {
    let mut start = node.start_byte();
    let mut holes = Vec::new();
    if class == SpanClass::String {
        collect_interpolations(node, &mut holes);
    }
    for hole in holes {
        steps.push(Step::Emit(start, hole.start_byte(), class));
        if is_interpolation(hole.kind()) {
            let mut cursor = hole.walk();
            steps.extend(hole.children(&mut cursor).map(Step::Visit));
        } else {
            steps.push(Step::Visit(hole));
        }
        start = hole.end_byte();
    }
    steps.push(Step::Emit(start, node.end_byte(), class));
}

/// Kinds of the nodes of PHP that hold interpolated strings; every named
/// child other than the text and the escape sequences is an expression.
const PHP_INTERPOLATED: [&str; 3] = [
    "encapsed_string",
    "heredoc_body",
    "shell_command_expression",
];

/// Kinds of the children of [`PHP_INTERPOLATED`] nodes that are text.
const PHP_TEXT: [&str; 2] = ["string_content", "escape_sequence"];

/// Returns `true` for a child of an interpolated PHP string that is code.
fn is_php_interpolation(parent: Node<'_>, child: Node<'_>) -> bool {
    child.is_named()
        && PHP_INTERPOLATED.contains(&parent.kind())
        && !PHP_TEXT.contains(&child.kind())
}

fn collect_interpolations<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_interpolation(child.kind()) || is_php_interpolation(node, child) {
            out.push(child);
        } else if child.child_count() > 0 {
            collect_interpolations(child, out);
        }
    }
}

fn push(out: &mut Vec<ClassifiedSpan>, start: usize, end: usize, class: SpanClass) {
    if end > start {
        out.push(ClassifiedSpan {
            bytes: ByteRange::new(to_u64(start), to_u64(end)),
            class,
        });
    }
}

/// Lexical class of the byte at `offset`.
#[must_use]
pub fn class_at(spans: &[ClassifiedSpan], offset: u64) -> SpanClass {
    let index = spans.partition_point(|span| span.bytes.end <= offset);
    spans
        .get(index)
        .filter(|span| span.bytes.contains(offset))
        .map_or(SpanClass::Code, |span| span.class)
}

/// Tokens of a syntax tree in source order, without whitespace.
///
/// Leaves of the tree are tokens; a comment or a string literal is one token
/// with the kind of its node. Zero-width nodes inserted by error recovery
/// are left out.
#[must_use]
pub fn tokens(tree: &Tree) -> Vec<SyntaxToken> {
    let mut out = Vec::new();
    collect_tokens(tree.root_node(), &mut out);
    out
}

/// Collects the tokens of the subtree of `root` with an explicit stack.
fn collect_tokens(root: Node<'_>, out: &mut Vec<SyntaxToken>) {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        let class = if node.is_named() {
            class_of_kind(node.kind())
        } else {
            None
        };
        if class.is_some() || node.child_count() == 0 {
            if node.end_byte() > node.start_byte() {
                out.push(SyntaxToken {
                    bytes: ByteRange::new(to_u64(node.start_byte()), to_u64(node.end_byte())),
                    kind: node.kind().to_owned(),
                    class: class.unwrap_or(SpanClass::Code),
                });
            }
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        pending.extend(children.into_iter().rev());
    }
}

/// Byte offset as `u64`.
pub(crate) fn to_u64(offset: usize) -> u64 {
    u64::try_from(offset).unwrap_or(u64::MAX)
}

/// Line starts of a text, for converting byte offsets to positions with
/// one-based lines and one-based columns counted in Unicode scalar values.
///
/// ```
/// use nurt_code::spans::LineIndex;
///
/// let index = LineIndex::new("a\néb".as_bytes());
/// let position = index.position(4);
/// assert_eq!((position.line, position.column), (2, 2));
/// ```
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    source: &'a [u8],
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// Index of the lines of `source`.
    #[must_use]
    pub fn new(source: &'a [u8]) -> Self {
        let starts = std::iter::once(0)
            .chain(
                source
                    .iter()
                    .enumerate()
                    .filter(|(_, byte)| **byte == b'\n')
                    .map(|(index, _)| index + 1),
            )
            .collect();
        Self { source, starts }
    }

    /// Position of a byte offset; offsets past the end map to the end.
    #[must_use]
    pub fn position(&self, offset: usize) -> TextPosition {
        let offset = offset.min(self.source.len());
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let start = self.starts[line];
        let column = count_chars(&self.source[start..offset]) + 1;
        TextPosition {
            line: u32::try_from(line + 1).unwrap_or(u32::MAX),
            column: u32::try_from(column).unwrap_or(u32::MAX),
        }
    }

    /// Span of the bytes `start..end`.
    #[must_use]
    pub fn span(&self, start: usize, end: usize) -> TextSpan {
        TextSpan {
            bytes: ByteRange::new(to_u64(start), to_u64(end)),
            start: self.position(start),
            end: self.position(end),
        }
    }

    /// Text of the line holding `offset`, without the line ending.
    #[must_use]
    pub fn line_text(&self, offset: usize) -> String {
        let offset = offset.min(self.source.len());
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let start = self.starts[line];
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.source.len(), |next| next - 1);
        let text = String::from_utf8_lossy(&self.source[start..end]);
        text.trim_end_matches('\r').to_owned()
    }

    /// Leading whitespace of the line holding `offset`.
    #[must_use]
    pub fn indentation(&self, offset: usize) -> String {
        self.line_text(offset)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }
}

/// Number of Unicode scalar values in UTF-8 bytes; bytes of an invalid
/// sequence count one each.
fn count_chars(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| (**byte & 0xC0) != 0x80).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_of_comments_and_strings_are_recognized() {
        for kind in [
            "line_comment",
            "block_comment",
            "Comment",
            "string_literal",
            "raw_string_literal",
            "template_string",
            "interpreted_string_literal",
            "double_quote_scalar",
            "heredoc_body",
            "char_literal",
        ] {
            assert!(class_of_kind(kind).is_some(), "{kind}");
        }
        for kind in [
            "identifier",
            "call_expression",
            "string_content_x",
            "plain_scalar",
        ] {
            assert!(class_of_kind(kind).is_none(), "{kind}");
        }
    }

    #[test]
    fn class_at_finds_the_span_of_an_offset() {
        let spans = [
            ClassifiedSpan {
                bytes: ByteRange::new(2, 4),
                class: SpanClass::String,
            },
            ClassifiedSpan {
                bytes: ByteRange::new(6, 9),
                class: SpanClass::Comment,
            },
        ];
        assert_eq!(class_at(&spans, 0), SpanClass::Code);
        assert_eq!(class_at(&spans, 3), SpanClass::String);
        assert_eq!(class_at(&spans, 4), SpanClass::Code);
        assert_eq!(class_at(&spans, 8), SpanClass::Comment);
    }

    /// Depth of the nesting that would overflow the stack of a test thread
    /// in a walk by recursion.
    const DEEP: usize = 300_000;

    fn parse(language: tree_sitter::Language, source: &[u8]) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn deeply_nested_source_is_classified_without_recursion() {
        let source = format!("{}1{}", "[".repeat(DEEP), "]".repeat(DEEP));
        let tree = parse(tree_sitter_json::LANGUAGE.into(), source.as_bytes());
        assert!(classify_spans(&tree).is_empty());
        let tokens = tokens(&tree);
        assert_eq!(tokens.len(), 2 * DEEP + 1);
        assert_eq!(tokens[DEEP].kind, "number");
        assert_eq!(
            tokens[DEEP].bytes,
            ByteRange::new(DEEP as u64, DEEP as u64 + 1)
        );
    }

    #[test]
    fn comments_and_strings_are_found_in_source_order() {
        let source = b"let a = \"x\"; // note\nlet b = 'c'; /* block */\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), source);
        let spans = classify_spans(&tree);
        let found: Vec<(SpanClass, &[u8])> = spans
            .iter()
            .map(|span| {
                let range = span.bytes.start as usize..span.bytes.end as usize;
                (span.class, &source[range])
            })
            .collect();
        assert_eq!(
            found,
            [
                (SpanClass::String, &b"\"x\""[..]),
                (SpanClass::Comment, &b"// note"[..]),
                (SpanClass::String, &b"'c'"[..]),
                (SpanClass::Comment, &b"/* block */"[..]),
            ]
        );
    }

    #[test]
    fn interpolations_of_a_template_string_are_code_with_their_own_strings() {
        let source = b"const a = `x ${f(\"in\")} y`;\n";
        let tree = parse(tree_sitter_javascript::LANGUAGE.into(), source);
        let spans = classify_spans(&tree);
        let text: Vec<&[u8]> = spans
            .iter()
            .map(|span| &source[span.bytes.start as usize..span.bytes.end as usize])
            .collect();
        assert_eq!(text, [&b"`x "[..], &b"\"in\""[..], &b" y`"[..]]);
        assert_eq!(spans[0].class, SpanClass::String);
        assert_eq!(class_at(&spans, 14), SpanClass::Code);
    }

    /// Texts of the strings and comments of PHP source.
    fn php_spans(source: &str) -> Vec<(SpanClass, String)> {
        let tree = parse(tree_sitter_php::LANGUAGE_PHP.into(), source.as_bytes());
        classify_spans(&tree)
            .iter()
            .map(|span| {
                let range = span.bytes.start as usize..span.bytes.end as usize;
                (span.class, source[range].to_owned())
            })
            .collect()
    }

    #[test]
    fn php_variables_and_expressions_in_double_quoted_strings_are_code() {
        let source = "<?php\n$a = \"x $b y {$c['k']} z $d->e\"; // done\n";
        let spans = php_spans(source);
        let strings: Vec<&str> = spans
            .iter()
            .filter(|(class, _)| *class == SpanClass::String)
            .map(|(_, text)| text.as_str())
            .collect();
        assert_eq!(strings[0], "\"x ");
        assert!(strings.contains(&" y {"), "{strings:?}");
        assert!(strings.contains(&"'k'"), "{strings:?}");
        assert!(strings.contains(&"} z "), "{strings:?}");
        for variable in ["$b", "$c", "$d"] {
            assert!(
                strings.iter().all(|text| !text.contains(variable)),
                "{variable} in {strings:?}"
            );
        }
        assert!(spans.contains(&(SpanClass::Comment, "// done".to_owned())));
    }

    #[test]
    fn php_heredoc_interpolations_are_code_and_nowdoc_is_text() {
        let source = "<?php\n$a = <<<EOT\nhello $name and {$obj->x}\nEOT;\n$b = <<<'RAW'\nno $interpolation\nRAW;\n";
        let spans = php_spans(source);
        let start_of = |needle: &str| source.find(needle).unwrap();
        let class_of = |needle: &str| {
            let at = start_of(needle) as u64;
            let tree = parse(tree_sitter_php::LANGUAGE_PHP.into(), source.as_bytes());
            class_at(&classify_spans(&tree), at)
        };
        assert_eq!(class_of("hello"), SpanClass::String);
        assert_eq!(class_of("$name"), SpanClass::Code);
        assert_eq!(class_of("$obj"), SpanClass::Code);
        assert_eq!(class_of("no $interpolation"), SpanClass::String);
        assert_eq!(class_of("$interpolation"), SpanClass::String);
        assert!(spans.iter().all(|(_, text)| !text.contains("$name")));
    }

    #[test]
    fn php_shell_command_text_is_a_string_with_code_holes() {
        let source = "<?php\n$o = `ls $dir`;\n";
        let spans = php_spans(source);
        assert!(
            spans
                .iter()
                .any(|(class, text)| *class == SpanClass::String && text.starts_with("`ls "))
        );
        assert!(spans.iter().all(|(_, text)| !text.contains("$dir")));
    }

    #[test]
    fn line_index_reports_lines_and_indentation() {
        let index = LineIndex::new(b"a\r\n\tb c\nlast");
        assert_eq!(index.line_text(0), "a");
        assert_eq!(index.indentation(5), "\t");
        assert_eq!(index.position(100).line, 3);
        assert_eq!(index.span(3, 4).start.column, 1);
    }

    #[test]
    fn line_index_counts_scalar_values_in_multibyte_lines_with_crlf() {
        let source = "é\nλx\r\n".as_bytes();
        let index = LineIndex::new(source);
        let at = |offset| {
            let position = index.position(offset);
            (position.line, position.column)
        };
        assert_eq!(at(0), (1, 1));
        assert_eq!(at(2), (1, 2));
        assert_eq!(at(3), (2, 1));
        assert_eq!(at(5), (2, 2));
        assert_eq!(at(6), (2, 3));
        assert_eq!(at(8), (3, 1));
        assert_eq!(at(99), (3, 1));
        assert_eq!(index.line_text(0), "é");
        assert_eq!(index.line_text(4), "λx");
        assert_eq!(index.line_text(8), "");
        let span = index.span(3, 6);
        assert_eq!((span.start.column, span.end.column), (1, 3));
    }

    #[test]
    fn line_index_of_empty_text_has_one_empty_line() {
        let index = LineIndex::new(b"");
        let position = index.position(10);
        assert_eq!((position.line, position.column), (1, 1));
        assert_eq!(index.line_text(10), "");
        assert_eq!(index.indentation(0), "");
    }

    #[test]
    fn indentation_is_the_leading_spaces_and_tabs_of_the_line() {
        let index = LineIndex::new(b"  \tx\n\ty\nz\n   \n");
        assert_eq!(index.indentation(0), "  \t");
        assert_eq!(index.indentation(6), "\t");
        assert_eq!(index.indentation(9), "");
        assert_eq!(index.indentation(11), "   ");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
