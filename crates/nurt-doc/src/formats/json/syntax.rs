// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::json::syntax
// Responsibility:   Syntax tree of a JSON or JSONC document with byte spans.
// Dependencies:     jsonc_parser
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Syntax tree of a JSON or JSONC document with byte spans.
//!
//! `jsonc-parser` builds the abstract syntax tree; this module copies it
//! into an arena with spans relative to the document bytes and checks every
//! gap between tokens. The gap check enforces what the parser options do
//! not: whitespace limited to the four characters of RFC 8259, string and
//! number lexemes of the RFC, and the exact separators between entries.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use jsonc_parser::ast;
use jsonc_parser::common::Ranged;
use jsonc_parser::{CollectOptions, ParseOptions};

use super::Dialect;
use super::layout::Style;
use super::lexeme::{self, LexemeError};

/// Greatest nesting depth of objects and arrays; deeper documents are
/// rejected before parsing, so that neither the parser nor the reference
/// parser exhausts the stack; `serde_json` accepts the same depth.
pub(crate) const MAX_NESTING_DEPTH: usize = 127;

/// Index of a node in [`Tree::nodes`].
pub(crate) type NodeId = usize;

/// Half-open byte range of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Span {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl Span {
    pub(crate) const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// Member name of an object entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Key {
    /// Decoded name.
    pub(crate) text: String,
    /// Span of the name, quotes included.
    pub(crate) span: Span,
}

/// Member of an object or element of an array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Member name; `None` for array elements.
    pub(crate) key: Option<Key>,
    /// Value node.
    pub(crate) value: NodeId,
    /// Offset of the comma that follows the value, including a trailing
    /// comma after the last entry.
    pub(crate) comma: Option<usize>,
}

impl Entry {
    /// Offset where the entry starts: its key, or its value for elements.
    pub(crate) fn start(&self, tree: &Tree) -> usize {
        self.key
            .as_ref()
            .map_or(tree.nodes[self.value].span.start, |key| key.span.start)
    }
}

/// Kind and content of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Null,
    Bool(bool),
    /// Number; the lexeme is the source text of the node span.
    Number,
    /// String with its decoded text.
    String(String),
    Object(Vec<Entry>),
    Array(Vec<Entry>),
}

/// Value node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Node {
    /// Span of the value; for objects and arrays it covers the brackets.
    pub(crate) span: Span,
    pub(crate) kind: NodeKind,
    /// Object or array containing the node; `None` for the root.
    pub(crate) parent: Option<NodeId>,
}

impl Node {
    /// Entries of an object or an array.
    pub(crate) fn entries(&self) -> Option<&[Entry]> {
        match &self.kind {
            NodeKind::Object(entries) | NodeKind::Array(entries) => Some(entries),
            _ => None,
        }
    }
}

/// Parsed document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tree {
    pub(crate) nodes: Vec<Node>,
    pub(crate) root: NodeId,
    /// Spans of all comments in document order.
    pub(crate) comments: Vec<Span>,
    /// Offsets of the line feeds in document order, so that the question
    /// whether a span holds a line break takes a search, not a scan of the
    /// span.
    pub(crate) newlines: Vec<usize>,
    /// More line breaks of the document are `\r\n` than `\n` alone.
    pub(crate) crlf_majority: bool,
    /// Styles of containers, computed when an edit first asks for them.
    pub(crate) styles: StyleCache,
}

/// Styles of the containers of one tree. The cache belongs to the tree and
/// is empty in a copy of it, so a style never outlives the text it was read
/// from.
#[derive(Debug, Default)]
pub(crate) struct StyleCache(Mutex<HashMap<NodeId, Style>>);

impl StyleCache {
    /// The style of `container`, computed by `compute` the first time.
    pub(crate) fn get_or_compute(
        &self,
        container: NodeId,
        compute: impl FnOnce() -> Style,
    ) -> Style {
        let mut styles = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        styles.entry(container).or_insert_with(compute).clone()
    }
}

impl Clone for StyleCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for StyleCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for StyleCache {}

/// Syntax error with the byte offset in the document where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub(crate) offset: usize,
    pub(crate) message: String,
}

impl SyntaxError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

/// Parses UTF-8 text without a byte order mark in the given dialect.
pub(crate) fn parse(text: &str, dialect: Dialect) -> Result<Tree, SyntaxError> {
    check_depth(text.as_bytes())?;
    let jsonc = dialect == Dialect::Jsonc;
    let options = ParseOptions {
        allow_comments: jsonc,
        allow_loose_object_property_names: false,
        allow_trailing_commas: jsonc,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    };
    let parsed = jsonc_parser::parse_to_ast(text, &CollectOptions::default(), &options)
        .map_err(|error| SyntaxError::new(error.range().start, parser_message(error.kind())))?;
    let Some(value) = parsed.value else {
        return Err(SyntaxError::new(
            text.len(),
            "document is empty; a JSON document holds one value",
        ));
    };
    let mut builder = Builder {
        text,
        dialect,
        nodes: Vec::new(),
        comments: Vec::new(),
    };
    builder.gap(0, value.start(), &[])?;
    let root = builder.value(&value)?;
    builder.gap(value.end(), text.len(), &[])?;
    let newlines: Vec<usize> = text
        .bytes()
        .enumerate()
        .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
        .collect();
    let crlf = newlines
        .iter()
        .filter(|offset| **offset > 0 && text.as_bytes()[**offset - 1] == b'\r')
        .count();
    Ok(Tree {
        nodes: builder.nodes,
        root,
        comments: builder.comments,
        crlf_majority: crlf > newlines.len() - crlf,
        newlines,
        styles: StyleCache::default(),
    })
}

fn parser_message(kind: &jsonc_parser::errors::ParseErrorKind) -> String {
    use jsonc_parser::errors::ParseErrorKind;
    match kind {
        ParseErrorKind::CommentsNotAllowed => {
            "comments are not allowed in JSON; read the file with format: jsonc".to_owned()
        }
        ParseErrorKind::TrailingCommasNotAllowed => {
            "trailing commas are not allowed in JSON; remove the comma or read the file with format: jsonc"
                .to_owned()
        }
        other => lowercase_first(&other.to_string()),
    }
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first
            .to_lowercase()
            .chain(chars)
            .collect::<String>()
            .trim_end_matches('.')
            .to_owned()
    })
}

/// Rejects documents nested deeper than [`MAX_NESTING_DEPTH`] without
/// recursion; strings and comments are skipped.
fn check_depth(bytes: &[u8]) -> Result<(), SyntaxError> {
    let mut depth = 0usize;
    let mut pos = 0;
    while let Some(&byte) = bytes.get(pos) {
        match byte {
            b'"' => pos = skip_string(bytes, pos),
            b'/' if bytes.get(pos + 1) == Some(&b'/') => pos = skip_line_comment(bytes, pos),
            b'/' if bytes.get(pos + 1) == Some(&b'*') => pos = skip_block_comment(bytes, pos),
            b'[' | b'{' => {
                depth += 1;
                if depth > MAX_NESTING_DEPTH {
                    return Err(SyntaxError::new(
                        pos,
                        format!("nesting deeper than {MAX_NESTING_DEPTH} levels is not supported"),
                    ));
                }
                pos += 1;
            }
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    Ok(())
}

fn skip_string(bytes: &[u8], mut pos: usize) -> usize {
    pos += 1;
    while let Some(&byte) = bytes.get(pos) {
        match byte {
            b'\\' => pos += 2,
            b'"' => return pos + 1,
            _ => pos += 1,
        }
    }
    pos
}

/// Offset where the line comment starting at `pos` ends: before its line
/// break, a `\r\n` pair included.
fn skip_line_comment(bytes: &[u8], pos: usize) -> usize {
    match bytes[pos..].iter().position(|&byte| byte == b'\n') {
        Some(offset) if offset > 2 && bytes[pos + offset - 1] == b'\r' => pos + offset - 1,
        Some(offset) => pos + offset,
        None => bytes.len(),
    }
}

fn skip_block_comment(bytes: &[u8], pos: usize) -> usize {
    bytes[pos + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")
        .map_or(bytes.len(), |offset| pos + 2 + offset + 2)
}

struct Builder<'t> {
    text: &'t str,
    dialect: Dialect,
    nodes: Vec<Node>,
    comments: Vec<Span>,
}

impl Builder<'_> {
    /// Adds a node after its children and links the children to it.
    fn push(&mut self, span: Span, kind: NodeKind) -> NodeId {
        let id = self.nodes.len();
        if let NodeKind::Object(entries) | NodeKind::Array(entries) = &kind {
            for entry in entries {
                self.nodes[entry.value].parent = Some(id);
            }
        }
        self.nodes.push(Node {
            span,
            kind,
            parent: None,
        });
        id
    }

    fn value(&mut self, value: &ast::Value<'_>) -> Result<NodeId, SyntaxError> {
        let span = Span::new(value.start(), value.end());
        let kind = match value {
            ast::Value::NullKeyword(_) => NodeKind::Null,
            ast::Value::BooleanLit(lit) => NodeKind::Bool(lit.value),
            ast::Value::NumberLit(lit) => {
                lexeme::validate_number(lit.value).map_err(|error| lexeme_error(span, &error))?;
                NodeKind::Number
            }
            ast::Value::StringLit(lit) => NodeKind::String(self.string(lit)?),
            ast::Value::Object(object) => NodeKind::Object(self.object(object)?),
            ast::Value::Array(array) => NodeKind::Array(self.array(array)?),
        };
        Ok(self.push(span, kind))
    }

    fn string(&self, lit: &ast::StringLit<'_>) -> Result<String, SyntaxError> {
        let span = Span::new(lit.start(), lit.end());
        lexeme::validate_string(&self.text[span.start..span.end])
            .map_err(|error| lexeme_error(span, &error))?;
        Ok(lit.value.clone().into_owned())
    }

    fn object(&mut self, object: &ast::Object<'_>) -> Result<Vec<Entry>, SyntaxError> {
        let mut entries = Vec::with_capacity(object.properties.len());
        let mut cursor = object.start() + 1;
        for prop in &object.properties {
            let ast::ObjectPropName::String(name) = &prop.name else {
                return Err(SyntaxError::new(
                    prop.name.start(),
                    "object member names must be strings in double quotes",
                ));
            };
            let separator = if entries.is_empty() {
                &[][..]
            } else {
                &b","[..]
            };
            let comma = self.gap(cursor, name.start(), separator)?;
            set_previous_comma(&mut entries, comma);
            let key = Key {
                text: self.string(name)?,
                span: Span::new(name.start(), name.end()),
            };
            self.gap(name.end(), prop.value.start(), b":")?;
            let value = self.value(&prop.value)?;
            entries.push(Entry {
                key: Some(key),
                value,
                comma: None,
            });
            cursor = prop.value.end();
        }
        self.close(&mut entries, cursor, object.end() - 1)?;
        Ok(entries)
    }

    fn array(&mut self, array: &ast::Array<'_>) -> Result<Vec<Entry>, SyntaxError> {
        let mut entries = Vec::with_capacity(array.elements.len());
        let mut cursor = array.start() + 1;
        for element in &array.elements {
            let separator = if entries.is_empty() {
                &[][..]
            } else {
                &b","[..]
            };
            let comma = self.gap(cursor, element.start(), separator)?;
            set_previous_comma(&mut entries, comma);
            let value = self.value(element)?;
            entries.push(Entry {
                key: None,
                value,
                comma: None,
            });
            cursor = element.end();
        }
        self.close(&mut entries, cursor, array.end() - 1)?;
        Ok(entries)
    }

    /// Checks the gap before the closing bracket and records a trailing
    /// comma.
    fn close(
        &mut self,
        entries: &mut [Entry],
        cursor: usize,
        closing: usize,
    ) -> Result<(), SyntaxError> {
        let trailing_allowed = self.dialect == Dialect::Jsonc && !entries.is_empty();
        let comma = self.gap_with(cursor, closing, |found| {
            found.is_empty() || (trailing_allowed && found == b",")
        })?;
        set_previous_comma(entries, comma);
        Ok(())
    }

    fn gap(
        &mut self,
        start: usize,
        end: usize,
        expected: &[u8],
    ) -> Result<Option<usize>, SyntaxError> {
        self.gap_with(start, end, |found| found == expected)
    }

    /// Scans the trivia and punctuation between two tokens and returns the
    /// offset of the comma in it, if any.
    fn gap_with(
        &mut self,
        start: usize,
        end: usize,
        accept: impl Fn(&[u8]) -> bool,
    ) -> Result<Option<usize>, SyntaxError> {
        let bytes = self.text.as_bytes();
        let mut punctuation = Vec::new();
        let mut comma = None;
        let mut pos = start;
        while pos < end {
            match bytes[pos] {
                b' ' | b'\t' | b'\n' | b'\r' => pos += 1,
                b'/' if self.dialect == Dialect::Jsonc
                    && matches!(bytes.get(pos + 1), Some(b'/' | b'*')) =>
                {
                    let comment_end = if bytes[pos + 1] == b'/' {
                        skip_line_comment(bytes, pos)
                    } else {
                        skip_block_comment(bytes, pos)
                    };
                    self.comments.push(Span::new(pos, comment_end));
                    pos = comment_end;
                }
                byte @ (b',' | b':') => {
                    if byte == b',' {
                        comma = Some(pos);
                    }
                    punctuation.push(byte);
                    pos += 1;
                }
                _ => return Err(SyntaxError::new(pos, unexpected_message(self.text, pos))),
            }
        }
        if accept(&punctuation) {
            Ok(comma)
        } else {
            Err(SyntaxError::new(
                start,
                "unexpected or missing separator between values",
            ))
        }
    }
}

fn set_previous_comma(entries: &mut [Entry], comma: Option<usize>) {
    if let Some(last) = entries.last_mut() {
        last.comma = comma;
    }
}

fn unexpected_message(text: &str, pos: usize) -> String {
    let found = text[pos..].chars().next().unwrap_or(' ');
    if found.is_whitespace() {
        format!(
            "whitespace U+{:04X} is not allowed in JSON; use space, tab or line breaks",
            u32::from(found)
        )
    } else {
        format!("unexpected character '{found}'")
    }
}

fn lexeme_error(span: Span, error: &LexemeError) -> SyntaxError {
    SyntaxError::new(span.start + error.offset, error.message)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
