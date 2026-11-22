// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
const STRING_KINDS: [&str; 10] = [
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

fn classify(node: Node<'_>, out: &mut Vec<ClassifiedSpan>) {
    if node.is_named() {
        if let Some(class) = class_of_kind(node.kind()) {
            push_with_holes(node, class, out);
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        classify(child, out);
    }
}

/// Adds the span of a literal or comment, leaving out its interpolations,
/// which are classified as code with their own comments and strings.
fn push_with_holes(node: Node<'_>, class: SpanClass, out: &mut Vec<ClassifiedSpan>) {
    let mut start = node.start_byte();
    let mut holes = Vec::new();
    if class == SpanClass::String {
        collect_interpolations(node, &mut holes);
    }
    for hole in holes {
        push(out, start, hole.start_byte(), class);
        classify_children(hole, out);
        start = hole.end_byte();
    }
    push(out, start, node.end_byte(), class);
}

fn classify_children(node: Node<'_>, out: &mut Vec<ClassifiedSpan>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        classify(child, out);
    }
}

fn collect_interpolations<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if is_interpolation(child.kind()) {
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

fn collect_tokens(node: Node<'_>, out: &mut Vec<SyntaxToken>) {
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
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_tokens(child, out);
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
/// let index = LineIndex::new("a\nżb".as_bytes());
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
        for kind in ["identifier", "call_expression", "string_content_x", "plain_scalar"] {
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

    #[test]
    fn line_index_reports_lines_and_indentation() {
        let index = LineIndex::new(b"a\r\n\tb c\nlast");
        assert_eq!(index.line_text(0), "a");
        assert_eq!(index.indentation(5), "\t");
        assert_eq!(index.position(100).line, 3);
        assert_eq!(index.span(3, 4).start.column, 1);
    }
}
