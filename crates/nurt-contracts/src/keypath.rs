// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Key paths into structured documents.
//!
//! A key path addresses values inside JSON, YAML, TOML, XML, INI, CSV,
//! property lists and similar documents. It is the value of the language type
//! `key`: a text literal in a parameter of type `key` is converted to a
//! [`KeyPath`] when the program is bound, so a malformed key is reported
//! before any command runs.
//!
//! The parser and the canonical printer in this module have no dependencies
//! beyond the standard library. The notation in short:
//!
//! | Written | Segments | Canonical |
//! |---|---|---|
//! | `scripts.test` | key `scripts`, key `test` | `scripts.test` |
//! | `deps[0].name` | key `deps`, index 0, key `name` | `deps[0].name` |
//! | `a[-1]` | key `a`, index -1 (from the end) | `a[-1]` |
//! | `a[*]`, `a.*` | key `a`, wildcard | `a[*]` |
//! | `a.**` | key `a`, recursive descent | `a.**` |
//! | `deps[+]` | key `deps`, append | `deps[+]` |
//! | `a[?name == "x"]` | key `a`, filter | `a[?name == "x"]` |
//! | `["a.b"].c` | key `a.b`, key `c` | `["a.b"].c` |
//! | `#1:spec.replicas` | document 1, key `spec`, key `replicas` | `#1:spec.replicas` |
//! | `$.a.b` (JSONPath) | key `a`, key `b` | `a.b` |
//!
//! The canonical form is the shortest unambiguous spelling; for every key
//! path `k`, `KeyPath::parse(&k.to_string()) == Ok(k)`.
//!
//! ```
//! use nurt_contracts::keypath::{KeyPath, KeySegment};
//!
//! let key = KeyPath::parse("deps[0].name")?;
//! assert_eq!(
//!     key.segments(),
//!     &[
//!         KeySegment::Key("deps".to_owned()),
//!         KeySegment::Index(0),
//!         KeySegment::Key("name".to_owned()),
//!     ]
//! );
//! assert_eq!(key.to_string(), "deps[0].name");
//! assert_eq!(KeyPath::parse("$.deps[0]['name']")?, key);
//! # Ok::<(), nurt_contracts::keypath::KeyPathError>(())
//! ```

use std::borrow::Cow;
use std::fmt::{self, Write as _};
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ast::{BinOp, Expr, KeySeg, UnOp, Wildcard};
use crate::error::{Error, ErrorCode, Stage};

/// Stable error code reported for a malformed key path.
pub const KEY_SYNTAX_ERROR_CODE: &str = ErrorCode::KeySyntax.as_str();

/// Reserved words of filter expressions; a field with one of these names is
/// written in brackets (`@["not"]`).
pub const FILTER_KEYWORDS: [&str; 6] = ["true", "false", "null", "and", "or", "not"];

/// Document of a multi-document file (YAML streams) addressed by a key path.
///
/// A key path without a selector addresses the first document; the selector
/// `#0:` is therefore never stored and never printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocSelector {
    /// Document with the given zero-based index (`#1:`).
    Index(u32),
    /// Every document of the stream (`#*:`).
    All,
}

/// One step of a key path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeySegment {
    /// Member of a map (`name`, `["name with . or ["]`).
    ///
    /// A name made of digits (`a.0`) is a map key `"0"`; when the node is a
    /// list the document format treats it as an index. In XML documents a
    /// name starting with `@` addresses an attribute and `#text` the text
    /// content; in other formats these are ordinary names (for example the
    /// JSON-LD member `@context`).
    Key(String),
    /// Element of a list (`[0]`); negative values count from the end
    /// (`[-1]` is the last element).
    Index(i64),
    /// Every member of a map or element of a list (`[*]`, `.*`).
    Wildcard,
    /// Any sequence of zero or more levels (`.**`); `a.**` therefore selects
    /// `a` and all of its descendants.
    Recursive,
    /// Position after the last element of a list (`[+]`); allowed only as
    /// the final segment.
    Append,
    /// Elements of a list or members of a map for which the expression holds
    /// (`[?name == "x"]`).
    Filter(FilterExpr),
}

/// Expression of a filter segment.
///
/// Fields are resolved relative to the element being tested; a field with an
/// empty path is the element itself (`@`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FilterExpr {
    /// Value at a path relative to the element (`name`, `meta.version`,
    /// `@["a b"]`); the empty path is the element itself (`@`).
    Field {
        /// Member names from the element downwards.
        path: Vec<String>,
    },
    /// Literal value.
    Literal {
        /// The value.
        value: FilterLiteral,
    },
    /// Comparison of two operands.
    Compare {
        /// Comparison operator.
        op: CompareOp,
        /// Left operand.
        left: Box<FilterExpr>,
        /// Right operand.
        right: Box<FilterExpr>,
    },
    /// Both operands hold (`and`, `&&`).
    And {
        /// Left operand.
        left: Box<FilterExpr>,
        /// Right operand.
        right: Box<FilterExpr>,
    },
    /// At least one operand holds (`or`, `||`).
    Or {
        /// Left operand.
        left: Box<FilterExpr>,
        /// Right operand.
        right: Box<FilterExpr>,
    },
    /// The operand does not hold (`not`, `!`).
    Not {
        /// Negated operand.
        operand: Box<FilterExpr>,
    },
}

/// Literal of a filter expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FilterLiteral {
    /// Text in double or single quotes.
    Text(String),
    /// Integer without fraction or exponent that fits 64 bits.
    Int(i64),
    /// Finite decimal number.
    Number(f64),
    /// `true` or `false`.
    Bool(bool),
    /// `null`.
    Null,
}

/// Comparison operator of a filter expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    /// Equal to (`==`).
    Eq,
    /// Not equal to (`!=`).
    Ne,
    /// Less than (`<`).
    Lt,
    /// Less than or equal to (`<=`).
    Le,
    /// Greater than (`>`).
    Gt,
    /// Greater than or equal to (`>=`).
    Ge,
}

impl CompareOp {
    /// Operator as written in a filter.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }
}

/// Kind of a key path syntax error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeyPathErrorKind {
    /// The text is empty.
    Empty,
    /// The text ends where a segment, operand or closing character is
    /// expected.
    UnexpectedEnd,
    /// A character that cannot appear at this position.
    UnexpectedCharacter,
    /// A name between dots is empty (`a..b`).
    EmptyName,
    /// The path starts with a dot (`.a`).
    LeadingDot,
    /// The path ends with a dot (`a.`).
    TrailingDot,
    /// A dot stands before a bracket (`a.[0]`).
    DotBeforeBracket,
    /// Brackets without content (`a[]`).
    EmptyBracket,
    /// A bracket holds an unquoted name (`a[b]`).
    UnquotedName,
    /// A name holds a character that requires quoting (`it's`).
    CharacterRequiresQuoting,
    /// A quoted name or text has no closing quote.
    UnterminatedString,
    /// An unknown or malformed escape sequence.
    InvalidEscape,
    /// An index does not fit 64 bits.
    IndexOutOfRange,
    /// A malformed or non-finite number.
    InvalidNumber,
    /// A filter without an expression (`a[?]`).
    EmptyFilter,
    /// A malformed filter expression.
    InvalidFilter,
    /// The append marker `[+]` is not the final segment.
    MisplacedAppend,
    /// A document selector is not followed by a path (`#1:`).
    MissingPath,
    /// A JSONPath construct outside the supported subset (slices, unions,
    /// functions, absolute references in filters).
    Unsupported,
}

/// Syntax error of a key path, with position and an optional correction.
///
/// The position refers to the key text itself; a caller reporting the error
/// for a program adds the position of the literal holding the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KeyPathError {
    /// Kind of the error.
    pub kind: KeyPathErrorKind,
    /// Byte offset in the key text where the error was detected.
    pub offset: usize,
    /// One-based column (in characters) of the offset.
    pub column: usize,
    /// Description in English.
    pub message: String,
    /// Corrected key text, when a single correction is evident.
    pub fix: Option<String>,
}

impl KeyPathError {
    /// Stable error code of every key path error ([`KEY_SYNTAX_ERROR_CODE`]).
    pub const fn code(&self) -> &'static str {
        KEY_SYNTAX_ERROR_CODE
    }
}

impl fmt::Display for KeyPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at column {}: {}",
            KEY_SYNTAX_ERROR_CODE, self.column, self.message
        )?;
        if let Some(fix) = &self.fix {
            write!(f, " (did you mean `{fix}`?)")?;
        }
        Ok(())
    }
}

impl std::error::Error for KeyPathError {}

/// Parsed key path: an optional document selector and a list of segments.
///
/// The empty list of segments addresses the document root and is written `$`.
/// Values are always valid: [`KeyPath::new`] and [`KeyPath::push`] reject an
/// append marker that is not final and non-finite numbers in filters.
/// Serialized as its canonical text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeyPath {
    document: Option<DocSelector>,
    segments: Vec<KeySegment>,
}

impl KeyPath {
    /// Parses a key path in the native notation or in the supported JSONPath
    /// subset.
    ///
    /// # Errors
    ///
    /// Returns [`KeyPathError`] with the kind, position and an evident
    /// correction when the text is not a valid key path.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::{KeyPath, KeyPathErrorKind};
    ///
    /// let error = KeyPath::parse("a.").unwrap_err();
    /// assert_eq!(error.kind, KeyPathErrorKind::TrailingDot);
    /// assert_eq!(error.fix.as_deref(), Some("a"));
    /// ```
    pub fn parse(text: &str) -> Result<Self, KeyPathError> {
        Parser::new(text).key_path()
    }

    /// Builds a key path from parts, checking the same rules as the parser.
    ///
    /// The selector `DocSelector::Index(0)` is normalized to no selector.
    ///
    /// # Errors
    ///
    /// Returns [`KeyPathErrorKind::MisplacedAppend`] when a segment follows
    /// [`KeySegment::Append`], and [`KeyPathErrorKind::InvalidNumber`] for a
    /// non-finite number in a filter.
    pub fn new(
        document: Option<DocSelector>,
        segments: Vec<KeySegment>,
    ) -> Result<Self, KeyPathError> {
        let mut key = Self {
            document: normalize_document(document),
            segments: Vec::new(),
        };
        for segment in segments {
            key.push(segment)?;
        }
        Ok(key)
    }

    /// Key path of the document root (`$`).
    pub const fn root() -> Self {
        Self {
            document: None,
            segments: Vec::new(),
        }
    }

    /// Document selector; `None` addresses the first document.
    pub const fn document(&self) -> Option<DocSelector> {
        self.document
    }

    /// Segments from the root downwards.
    pub fn segments(&self) -> &[KeySegment] {
        &self.segments
    }

    /// Returns `true` for the document root (no segments).
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns `true` when the path addresses no more than one node (only
    /// keys and indices).
    pub fn is_concrete(&self) -> bool {
        self.document != Some(DocSelector::All)
            && self
                .segments
                .iter()
                .all(|s| matches!(s, KeySegment::Key(_) | KeySegment::Index(_)))
    }

    /// Returns `true` when the final segment is the append marker `[+]`.
    pub fn ends_with_append(&self) -> bool {
        matches!(self.segments.last(), Some(KeySegment::Append))
    }

    /// Appends a segment, rejecting a segment after `[+]` and non-finite
    /// numbers in filters.
    ///
    /// # Errors
    ///
    /// Returns [`KeyPathErrorKind::MisplacedAppend`] when the path already
    /// ends with [`KeySegment::Append`], and
    /// [`KeyPathErrorKind::InvalidNumber`] for a non-finite number in a
    /// filter.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::{KeyPath, KeySegment};
    ///
    /// let mut key = KeyPath::root();
    /// key.push(KeySegment::Key("a.b".to_owned()))?;
    /// key.push(KeySegment::Index(2))?;
    /// assert_eq!(key.to_string(), r#"["a.b"][2]"#);
    /// # Ok::<(), nurt_contracts::keypath::KeyPathError>(())
    /// ```
    pub fn push(&mut self, segment: KeySegment) -> Result<(), KeyPathError> {
        if self.ends_with_append() {
            return Err(invalid_value(
                KeyPathErrorKind::MisplacedAppend,
                "the append marker [+] must be the final segment",
            ));
        }
        if let KeySegment::Filter(expr) = &segment {
            check_filter(expr)?;
        }
        self.segments.push(segment);
        Ok(())
    }

    /// Key path of the parent node; `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        let (_, init) = self.segments.split_last()?;
        Some(Self {
            document: self.document,
            segments: init.to_vec(),
        })
    }
}

fn normalize_document(document: Option<DocSelector>) -> Option<DocSelector> {
    match document {
        Some(DocSelector::Index(0)) => None,
        other => other,
    }
}

fn invalid_value(kind: KeyPathErrorKind, message: &str) -> KeyPathError {
    KeyPathError {
        kind,
        offset: 0,
        column: 1,
        message: message.to_owned(),
        fix: None,
    }
}

fn check_filter(expr: &FilterExpr) -> Result<(), KeyPathError> {
    match expr {
        FilterExpr::Field { .. } => Ok(()),
        FilterExpr::Literal {
            value: FilterLiteral::Number(n),
        } if !n.is_finite() => Err(invalid_value(
            KeyPathErrorKind::InvalidNumber,
            "filter numbers must be finite",
        )),
        FilterExpr::Literal { .. } => Ok(()),
        FilterExpr::Compare { left, right, .. }
        | FilterExpr::And { left, right }
        | FilterExpr::Or { left, right } => {
            check_filter(left)?;
            check_filter(right)
        }
        FilterExpr::Not { operand } => check_filter(operand),
    }
}

impl FromStr for KeyPath {
    type Err = KeyPathError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl TryFrom<&str> for KeyPath {
    type Error = KeyPathError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        Self::parse(text)
    }
}

impl Serialize for KeyPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeyPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<'de, str>::deserialize(deserializer)?;
        Self::parse(&text).map_err(D::Error::custom)
    }
}

impl JsonSchema for KeyPath {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("KeyPath")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "description": "Key path into a structured document, in canonical notation."
        })
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.document {
            Some(DocSelector::Index(index)) => write!(f, "#{index}:")?,
            Some(DocSelector::All) => f.write_str("#*:")?,
            None => {}
        }
        if self.segments.is_empty() {
            return f.write_str("$");
        }
        for (position, segment) in self.segments.iter().enumerate() {
            let first = position == 0;
            match segment {
                KeySegment::Key(name) => {
                    if is_bare_name(name, first && self.document.is_none(), first) {
                        if !first {
                            f.write_char('.')?;
                        }
                        f.write_str(name)?;
                    } else {
                        f.write_char('[')?;
                        write_quoted(f, name)?;
                        f.write_char(']')?;
                    }
                }
                KeySegment::Index(index) => write!(f, "[{index}]")?,
                KeySegment::Wildcard => f.write_str("[*]")?,
                KeySegment::Recursive => f.write_str(if first { "**" } else { ".**" })?,
                KeySegment::Append => f.write_str("[+]")?,
                KeySegment::Filter(expr) => {
                    f.write_str("[?")?;
                    write_filter(f, expr, 0)?;
                    f.write_char(']')?;
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for FilterExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_filter(f, self, 0)
    }
}

/// `true` when `name` can be written without brackets at this position.
fn is_bare_name(name: &str, may_look_like_selector: bool, first: bool) -> bool {
    if name.is_empty() || name == "*" || name == "**" {
        return false;
    }
    if first && name == "$" {
        return false;
    }
    if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
        return false;
    }
    if name
        .chars()
        .any(|c| matches!(c, '.' | '[' | ']' | '"' | '\'') || c.is_control())
    {
        return false;
    }
    !(may_look_like_selector && doc_selector_length(name).is_some())
}

/// Length in bytes of a document selector (`#12:`, `#*:`) at the start of
/// `text`.
fn doc_selector_length(text: &str) -> Option<usize> {
    let body = text.strip_prefix('#')?;
    if body.starts_with("*:") {
        return Some("#*:".len());
    }
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0 && body.as_bytes().get(digits) == Some(&b':'))
        .then_some("#".len() + digits + ":".len())
}

fn write_quoted(f: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    f.write_char('"')?;
    for c in text.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if c.is_control() => write!(f, "\\u{{{:x}}}", u32::from(c))?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_NOT: u8 = 3;
const PREC_COMPARE: u8 = 4;
const PREC_ATOM: u8 = 5;

const fn precedence(expr: &FilterExpr) -> u8 {
    match expr {
        FilterExpr::Or { .. } => PREC_OR,
        FilterExpr::And { .. } => PREC_AND,
        FilterExpr::Not { .. } => PREC_NOT,
        FilterExpr::Compare { .. } => PREC_COMPARE,
        FilterExpr::Field { .. } | FilterExpr::Literal { .. } => PREC_ATOM,
    }
}

fn write_filter(f: &mut fmt::Formatter<'_>, expr: &FilterExpr, min: u8) -> fmt::Result {
    if precedence(expr) < min {
        f.write_char('(')?;
        write_filter(f, expr, 0)?;
        return f.write_char(')');
    }
    match expr {
        FilterExpr::Or { left, right } => {
            write_filter(f, left, PREC_OR)?;
            f.write_str(" or ")?;
            write_filter(f, right, PREC_AND)
        }
        FilterExpr::And { left, right } => {
            write_filter(f, left, PREC_AND)?;
            f.write_str(" and ")?;
            write_filter(f, right, PREC_NOT)
        }
        FilterExpr::Not { operand } => {
            f.write_str("not ")?;
            let min = if matches!(**operand, FilterExpr::Not { .. }) {
                PREC_NOT
            } else {
                PREC_ATOM
            };
            write_filter(f, operand, min)
        }
        FilterExpr::Compare { op, left, right } => {
            write_filter(f, left, PREC_ATOM)?;
            write!(f, " {} ", op.as_str())?;
            write_filter(f, right, PREC_ATOM)
        }
        FilterExpr::Field { path } => write_field(f, path),
        FilterExpr::Literal { value } => write_literal(f, value),
    }
}

fn write_field(f: &mut fmt::Formatter<'_>, path: &[String]) -> fmt::Result {
    let Some((head, tail)) = path.split_first() else {
        return f.write_char('@');
    };
    if is_filter_name(head) {
        f.write_str(head)?;
    } else {
        f.write_str("@[")?;
        write_quoted(f, head)?;
        f.write_char(']')?;
    }
    for name in tail {
        if is_filter_name(name) {
            f.write_char('.')?;
            f.write_str(name)?;
        } else {
            f.write_char('[')?;
            write_quoted(f, name)?;
            f.write_char(']')?;
        }
    }
    Ok(())
}

fn write_literal(f: &mut fmt::Formatter<'_>, value: &FilterLiteral) -> fmt::Result {
    match value {
        FilterLiteral::Text(text) => write_quoted(f, text),
        FilterLiteral::Int(n) => write!(f, "{n}"),
        FilterLiteral::Number(n) => write!(f, "{n:?}"),
        FilterLiteral::Bool(b) => write!(f, "{b}"),
        FilterLiteral::Null => f.write_str("null"),
    }
}

fn is_filter_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_filter_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '$')
}

/// `true` when `name` can be written as a bare field in a filter.
fn is_filter_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_filter_name_start)
        && chars.all(is_filter_name_char)
        && !FILTER_KEYWORDS.contains(&name)
}

type Parsed<T> = Result<T, KeyPathError>;

/// Largest number of hexadecimal digits in a `\u{…}` escape.
const MAX_BRACED_HEX_DIGITS: usize = 6;

/// Number of hexadecimal digits in a `\uXXXX` escape.
const UTF16_ESCAPE_DIGITS: usize = 4;

/// UTF-16 high surrogates, the first unit of a pair.
const HIGH_SURROGATES: std::ops::Range<u32> = 0xD800..0xDC00;

/// UTF-16 low surrogates, the second unit of a pair.
const LOW_SURROGATES: std::ops::Range<u32> = 0xDC00..0xE000;

/// First code point outside the Basic Multilingual Plane, encoded by a
/// surrogate pair.
const SUPPLEMENTARY_PLANE_START: u32 = 0x10000;

/// Bits of a code point carried by the low surrogate of a pair.
const LOW_SURROGATE_BITS: u32 = 10;

/// Deepest nesting of parentheses and negations accepted in a filter.
pub const MAX_FILTER_DEPTH: usize = 64;

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    const fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            depth: 0,
        }
    }

    fn enter(&mut self) -> Parsed<()> {
        self.depth += 1;
        if self.depth > MAX_FILTER_DEPTH {
            return Err(self.error(
                KeyPathErrorKind::InvalidFilter,
                "the filter is nested too deeply",
            ));
        }
        Ok(())
    }

    fn rest(&self) -> &'a str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_second(&self) -> Option<char> {
        self.rest().chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn eat_keyword(&mut self, word: &str) -> bool {
        let rest = self.rest();
        let follows = rest.get(word.len()..).and_then(|r| r.chars().next());
        if rest.starts_with(word) && !follows.is_some_and(is_filter_name_char) {
            self.pos += word.len();
            true
        } else {
            false
        }
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn error_at(&self, offset: usize, kind: KeyPathErrorKind, message: String) -> KeyPathError {
        let column = self.text.get(..offset).map_or(1, |p| p.chars().count() + 1);
        KeyPathError {
            kind,
            offset,
            column,
            message,
            fix: None,
        }
    }

    fn error(&self, kind: KeyPathErrorKind, message: impl Into<String>) -> KeyPathError {
        self.error_at(self.pos, kind, message.into())
    }

    fn found(&self) -> String {
        self.peek()
            .map_or_else(|| "the end of the key".to_owned(), |c| format!("`{c}`"))
    }

    /// Text with `text[start..end]` replaced by `replacement`.
    fn splice(&self, start: usize, end: usize, replacement: &str) -> Option<String> {
        Some(format!(
            "{}{}{}",
            self.text.get(..start)?,
            replacement,
            self.text.get(end..)?
        ))
    }

    fn expect(&mut self, c: char, what: &str) -> Parsed<()> {
        if self.peek() == Some(c) {
            self.bump();
            return Ok(());
        }
        let kind = if self.peek().is_none() {
            KeyPathErrorKind::UnexpectedEnd
        } else {
            KeyPathErrorKind::UnexpectedCharacter
        };
        Err(self.error(kind, format!("expected {what}, found {}", self.found())))
    }

    fn key_path(mut self) -> Parsed<KeyPath> {
        if self.text.is_empty() {
            return Err(self.error(KeyPathErrorKind::Empty, "a key path must not be empty"));
        }
        let document = self.document_selector()?;
        let segments = if self.at_json_path_root() {
            self.bump();
            self.json_path_segments()?
        } else if self.peek().is_none() {
            let mut error = self.error(
                KeyPathErrorKind::MissingPath,
                "a document selector must be followed by a key path; `$` is the document root",
            );
            error.fix = Some(format!("{}$", self.text));
            return Err(error);
        } else {
            self.native_segments()?
        };
        Ok(KeyPath {
            document: normalize_document(document),
            segments,
        })
    }

    fn document_selector(&mut self) -> Parsed<Option<DocSelector>> {
        let Some(length) = doc_selector_length(self.rest()) else {
            return Ok(None);
        };
        let body = self.rest().get(1..length - 1).unwrap_or_default();
        let selector = if body == "*" {
            DocSelector::All
        } else {
            let index = body.parse::<u32>().map_err(|_| {
                self.error(
                    KeyPathErrorKind::IndexOutOfRange,
                    "the document index is too large",
                )
            })?;
            DocSelector::Index(index)
        };
        self.pos += length;
        Ok(Some(selector))
    }

    fn at_json_path_root(&self) -> bool {
        self.peek() == Some('$') && matches!(self.peek_second(), None | Some('.' | '['))
    }

    fn native_segments(&mut self) -> Parsed<Vec<KeySegment>> {
        let mut segments = Vec::new();
        match self.peek() {
            Some('.') => return Err(self.leading_dot()),
            Some('[') => self.push_segment(&mut segments, Self::bracket)?,
            _ => self.push_segment(&mut segments, Self::name_segment)?,
        }
        while let Some(c) = self.peek() {
            match c {
                '.' => {
                    self.check_after_dot()?;
                    self.push_segment(&mut segments, Self::name_segment)?;
                }
                '[' => self.push_segment(&mut segments, Self::bracket)?,
                _ => return Err(self.missing_separator()),
            }
        }
        Ok(segments)
    }

    fn push_segment(
        &mut self,
        segments: &mut Vec<KeySegment>,
        parse: fn(&mut Self) -> Parsed<KeySegment>,
    ) -> Parsed<()> {
        let start = self.pos;
        if matches!(segments.last(), Some(KeySegment::Append)) {
            return Err(self.error_at(
                start,
                KeyPathErrorKind::MisplacedAppend,
                "the append marker [+] must be the final segment".to_owned(),
            ));
        }
        let segment = parse(self)?;
        segments.push(segment);
        Ok(())
    }

    fn leading_dot(&self) -> KeyPathError {
        let mut error = self.error(
            KeyPathErrorKind::LeadingDot,
            "a key path does not start with `.`",
        );
        error.fix = self
            .splice(self.pos, self.pos + 1, "")
            .filter(|f| !f.is_empty());
        error
    }

    /// Consumes the dot of `.name` and rejects `a.`, `a..b` and `a.[0]`.
    fn check_after_dot(&mut self) -> Parsed<()> {
        let dot = self.pos;
        self.bump();
        match self.peek() {
            None => {
                let mut error = self.error_at(
                    dot,
                    KeyPathErrorKind::TrailingDot,
                    "a key path does not end with `.`".to_owned(),
                );
                error.fix = self.splice(dot, dot + 1, "").filter(|f| !f.is_empty());
                Err(error)
            }
            Some('.') => {
                let mut error = self.error(
                    KeyPathErrorKind::EmptyName,
                    "empty name between dots; recursive descent is written `.**`",
                );
                error.fix = self.splice(dot, dot + 2, ".**.");
                Err(error)
            }
            Some('[') => {
                let mut error = self.error_at(
                    dot,
                    KeyPathErrorKind::DotBeforeBracket,
                    "a bracket follows the previous segment without a dot".to_owned(),
                );
                error.fix = self.splice(dot, dot + 1, "");
                Err(error)
            }
            Some(_) => Ok(()),
        }
    }

    fn missing_separator(&self) -> KeyPathError {
        let mut error = self.error(
            KeyPathErrorKind::UnexpectedCharacter,
            format!(
                "expected `.` or `[` after a bracket, found {}",
                self.found()
            ),
        );
        error.fix = self.splice(self.pos, self.pos, ".");
        error
    }

    /// Bare name, `*` or `**`, up to the next `.` or `[`.
    fn name_segment(&mut self) -> Parsed<KeySegment> {
        let start = self.pos;
        let end = self
            .rest()
            .find(['.', '['])
            .map_or(self.text.len(), |i| start + i);
        let name = self.text.get(start..end).unwrap_or_default();
        if let Some(i) = name.find([']', '"', '\'']) {
            let mut error = self.error_at(
                start + i,
                KeyPathErrorKind::CharacterRequiresQuoting,
                format!(
                    "names containing `{}` are written in quotes: [\"...\"]",
                    name.get(i..=i).unwrap_or_default()
                ),
            );
            let before = self.text.get(..start).unwrap_or_default();
            let from = if before.ends_with('.') && !before.ends_with("..") {
                start - 1
            } else {
                start
            };
            error.fix = self.splice(from, end, &format!("[{}]", quote(name)));
            return Err(error);
        }
        if name.is_empty() {
            return Err(self.error(
                KeyPathErrorKind::EmptyName,
                format!("expected a name, found {}", self.found()),
            ));
        }
        self.pos = end;
        Ok(match name {
            "*" => KeySegment::Wildcard,
            "**" => KeySegment::Recursive,
            _ => KeySegment::Key(name.to_owned()),
        })
    }

    /// `[0]`, `[-1]`, `[*]`, `[+]`, `["name"]`, `['name']`, `[?filter]`.
    fn bracket(&mut self) -> Parsed<KeySegment> {
        let open = self.pos;
        self.bump();
        self.skip_whitespace();
        let segment = match self.peek() {
            None => {
                return Err(self.error_at(
                    open,
                    KeyPathErrorKind::UnexpectedEnd,
                    "the bracket is not closed".to_owned(),
                ));
            }
            Some(']') => {
                let mut error =
                    self.error(KeyPathErrorKind::EmptyBracket, "brackets must not be empty");
                error.fix = self.splice(open, self.pos + 1, "[*]");
                return Err(error);
            }
            Some('"' | '\'') => KeySegment::Key(self.quoted()?),
            Some('*') => self.bracket_wildcard()?,
            Some('+') => {
                self.bump();
                KeySegment::Append
            }
            Some('?') => {
                self.bump();
                KeySegment::Filter(self.filter()?)
            }
            Some(c) if c == '-' || c.is_ascii_digit() => KeySegment::Index(self.index()?),
            Some(':') => return Err(self.unsupported("slices")),
            Some(_) => return Err(self.unquoted_name(open)),
        };
        self.skip_whitespace();
        self.close_bracket()?;
        Ok(segment)
    }

    fn bracket_wildcard(&mut self) -> Parsed<KeySegment> {
        self.bump();
        Ok(if self.eat("*") {
            KeySegment::Recursive
        } else {
            KeySegment::Wildcard
        })
    }

    fn close_bracket(&mut self) -> Parsed<()> {
        match self.peek() {
            Some(']') => {
                self.bump();
                Ok(())
            }
            Some(',') => Err(self.unsupported("unions")),
            Some(':') => Err(self.unsupported("slices")),
            _ => self.expect(']', "`]`"),
        }
    }

    fn unsupported(&self, what: &str) -> KeyPathError {
        self.error(
            KeyPathErrorKind::Unsupported,
            format!("JSONPath {what} are not supported in key paths"),
        )
    }

    fn unquoted_name(&self, open: usize) -> KeyPathError {
        let mut error = self.error(
            KeyPathErrorKind::UnquotedName,
            "names in brackets are written in quotes",
        );
        if let Some(length) = self.rest().find(']') {
            let name = self.rest().get(..length).unwrap_or_default().trim_end();
            error.fix = self.splice(open, self.pos + length + 1, &format!("[{}]", quote(name)));
        }
        error
    }

    fn index(&mut self) -> Parsed<i64> {
        let start = self.pos;
        self.eat("-");
        let digits = self.rest().bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return Err(self.error(
                KeyPathErrorKind::UnexpectedCharacter,
                format!("expected digits of an index, found {}", self.found()),
            ));
        }
        self.pos += digits;
        let text = self.text.get(start..self.pos).unwrap_or_default();
        let value = text.parse::<i64>().map_err(|_| {
            self.error_at(
                start,
                KeyPathErrorKind::IndexOutOfRange,
                format!("index `{text}` does not fit 64 bits"),
            )
        })?;
        Ok(value)
    }

    /// Quoted text with escapes; the opening quote is at the current position.
    fn quoted(&mut self) -> Parsed<String> {
        let open = self.pos;
        let Some(quote_char) = self.bump() else {
            return Err(self.error(KeyPathErrorKind::UnexpectedEnd, "expected a quote"));
        };
        let mut out = String::new();
        loop {
            match self.bump() {
                None => {
                    return Err(self.error_at(
                        open,
                        KeyPathErrorKind::UnterminatedString,
                        format!("the quoted text is not closed with `{quote_char}`"),
                    ));
                }
                Some(c) if c == quote_char => return Ok(out),
                Some('\\') => out.push(self.escape()?),
                Some(c) => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Parsed<char> {
        let start = self.pos - 1;
        let c = match self.bump() {
            Some('"') => '"',
            Some('\'') => '\'',
            Some('\\') => '\\',
            Some('/') => '/',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            Some('b') => '\u{8}',
            Some('f') => '\u{c}',
            Some('u') => return self.unicode_escape(start),
            _ => {
                return Err(self.error_at(
                    start,
                    KeyPathErrorKind::InvalidEscape,
                    "unknown escape sequence".to_owned(),
                ));
            }
        };
        Ok(c)
    }

    /// `\u{1F600}` or `\uXXXX` (with surrogate pairs, as in JSON).
    fn unicode_escape(&mut self, start: usize) -> Parsed<char> {
        let invalid = |p: &Self| {
            p.error_at(
                start,
                KeyPathErrorKind::InvalidEscape,
                "malformed `\\u` escape".to_owned(),
            )
        };
        if self.eat("{") {
            let length = self.rest().find('}').ok_or_else(|| invalid(self))?;
            let hex = self.rest().get(..length).unwrap_or_default();
            let code = (1..=MAX_BRACED_HEX_DIGITS)
                .contains(&hex.len())
                .then(|| u32::from_str_radix(hex, 16).ok())
                .flatten()
                .and_then(char::from_u32)
                .ok_or_else(|| invalid(self))?;
            self.pos += length + 1;
            return Ok(code);
        }
        let high = self.hex4().ok_or_else(|| invalid(self))?;
        if !HIGH_SURROGATES.contains(&high) {
            return char::from_u32(high).ok_or_else(|| invalid(self));
        }
        if !self.eat("\\u") {
            return Err(invalid(self));
        }
        let low = self
            .hex4()
            .filter(|l| LOW_SURROGATES.contains(l))
            .ok_or_else(|| invalid(self))?;
        char::from_u32(
            SUPPLEMENTARY_PLANE_START
                + ((high - HIGH_SURROGATES.start) << LOW_SURROGATE_BITS)
                + (low - LOW_SURROGATES.start),
        )
        .ok_or_else(|| invalid(self))
    }

    fn hex4(&mut self) -> Option<u32> {
        let hex = self.rest().get(..UTF16_ESCAPE_DIGITS)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.pos += UTF16_ESCAPE_DIGITS;
        u32::from_str_radix(hex, 16).ok()
    }

    fn json_path_segments(&mut self) -> Parsed<Vec<KeySegment>> {
        let mut segments = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                '.' if self.peek_second() == Some('.') => {
                    self.pos += 2;
                    self.push_segment(&mut segments, |_| Ok(KeySegment::Recursive))?;
                    match self.peek() {
                        Some('[') => self.push_segment(&mut segments, Self::bracket)?,
                        _ => self.push_segment(&mut segments, Self::name_segment)?,
                    }
                }
                '.' => {
                    self.check_after_dot()?;
                    self.push_segment(&mut segments, Self::name_segment)?;
                }
                '[' => self.push_segment(&mut segments, Self::bracket)?,
                _ => return Err(self.missing_separator()),
            }
        }
        Ok(segments)
    }

    fn filter(&mut self) -> Parsed<FilterExpr> {
        self.skip_whitespace();
        if matches!(self.peek(), None | Some(']')) {
            return Err(self.error(
                KeyPathErrorKind::EmptyFilter,
                "a filter needs an expression",
            ));
        }
        self.or_expr()
    }

    fn or_expr(&mut self) -> Parsed<FilterExpr> {
        let mut left = self.and_expr()?;
        loop {
            self.skip_whitespace();
            if !(self.eat("||") || self.eat_keyword("or")) {
                return Ok(left);
            }
            let right = self.and_expr()?;
            left = FilterExpr::Or {
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn and_expr(&mut self) -> Parsed<FilterExpr> {
        let mut left = self.not_expr()?;
        loop {
            self.skip_whitespace();
            if !(self.eat("&&") || self.eat_keyword("and")) {
                return Ok(left);
            }
            let right = self.not_expr()?;
            left = FilterExpr::And {
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn not_expr(&mut self) -> Parsed<FilterExpr> {
        self.skip_whitespace();
        let negated = if self.peek() == Some('!') && self.peek_second() != Some('=') {
            self.bump();
            true
        } else {
            self.eat_keyword("not")
        };
        if negated {
            self.enter()?;
            let operand = self.not_expr()?;
            self.depth -= 1;
            return Ok(FilterExpr::Not {
                operand: Box::new(operand),
            });
        }
        self.compare()
    }

    fn compare(&mut self) -> Parsed<FilterExpr> {
        let left = self.operand()?;
        self.skip_whitespace();
        let Some(op) = self.compare_op()? else {
            return Ok(left);
        };
        let right = self.operand()?;
        self.skip_whitespace();
        let chained = self.pos;
        if self.compare_op()?.is_some() {
            return Err(self.error_at(
                chained,
                KeyPathErrorKind::InvalidFilter,
                "comparisons cannot be chained; join them with `and`".to_owned(),
            ));
        }
        Ok(FilterExpr::Compare {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn compare_op(&mut self) -> Parsed<Option<CompareOp>> {
        const OPS: [(&str, CompareOp); 6] = [
            ("==", CompareOp::Eq),
            ("!=", CompareOp::Ne),
            ("<=", CompareOp::Le),
            (">=", CompareOp::Ge),
            ("<", CompareOp::Lt),
            (">", CompareOp::Gt),
        ];
        for (token, op) in OPS {
            if self.eat(token) {
                return Ok(Some(op));
            }
        }
        if self.peek() == Some('=') {
            let mut error = self.error(KeyPathErrorKind::InvalidFilter, "equality is written `==`");
            error.fix = self.splice(self.pos, self.pos + 1, "==");
            return Err(error);
        }
        Ok(None)
    }

    fn operand(&mut self) -> Parsed<FilterExpr> {
        self.skip_whitespace();
        match self.peek() {
            Some('(') => {
                self.bump();
                self.enter()?;
                let inner = self.or_expr()?;
                self.depth -= 1;
                self.skip_whitespace();
                self.expect(')', "`)`")?;
                Ok(inner)
            }
            Some('"' | '\'') => Ok(literal(FilterLiteral::Text(self.quoted()?))),
            Some(c) if c == '-' || c.is_ascii_digit() => Ok(literal(self.number()?)),
            Some('@') => {
                self.bump();
                self.field_tail(Vec::new())
            }
            Some('$') => Err(self.unsupported("absolute references in filters")),
            Some(c) if is_filter_name_start(c) => self.word_operand(),
            _ => Err(self.error(
                KeyPathErrorKind::InvalidFilter,
                format!("expected a field or a literal, found {}", self.found()),
            )),
        }
    }

    fn word_operand(&mut self) -> Parsed<FilterExpr> {
        let start = self.pos;
        let word = self.filter_name();
        match word.as_str() {
            "true" => Ok(literal(FilterLiteral::Bool(true))),
            "false" => Ok(literal(FilterLiteral::Bool(false))),
            "null" => Ok(literal(FilterLiteral::Null)),
            "and" | "or" | "not" => Err(self.error_at(
                start,
                KeyPathErrorKind::InvalidFilter,
                format!("expected an operand, found the keyword `{word}`"),
            )),
            _ if self.peek() == Some('(') => Err(self.unsupported("functions")),
            _ => self.field_tail(vec![word]),
        }
    }

    fn filter_name(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(is_filter_name_char) {
            self.bump();
        }
        self.text
            .get(start..self.pos)
            .unwrap_or_default()
            .to_owned()
    }

    /// Continues a field with `.name` and `["name"]` steps.
    fn field_tail(&mut self, mut path: Vec<String>) -> Parsed<FilterExpr> {
        loop {
            match self.peek() {
                Some('.') => {
                    self.bump();
                    if !self.peek().is_some_and(is_filter_name_start) {
                        return Err(self.error(
                            KeyPathErrorKind::InvalidFilter,
                            format!("expected a field name, found {}", self.found()),
                        ));
                    }
                    path.push(self.filter_name());
                }
                Some('[') => {
                    self.bump();
                    self.skip_whitespace();
                    if !matches!(self.peek(), Some('"' | '\'')) {
                        return Err(self.error(
                            KeyPathErrorKind::InvalidFilter,
                            "fields in filters are written as names or quoted names",
                        ));
                    }
                    path.push(self.quoted()?);
                    self.skip_whitespace();
                    self.expect(']', "`]`")?;
                }
                _ => return Ok(FilterExpr::Field { path }),
            }
        }
    }

    fn number(&mut self) -> Parsed<FilterLiteral> {
        let start = self.pos;
        self.eat("-");
        let integer_digits = self.digits();
        let mut is_integer = true;
        if self.peek() == Some('.') && self.peek_second().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.digits();
            is_integer = false;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if self.digits() == 0 {
                return Err(self.error(KeyPathErrorKind::InvalidNumber, "malformed exponent"));
            }
            is_integer = false;
        }
        let text = self.text.get(start..self.pos).unwrap_or_default();
        if integer_digits == 0 {
            return Err(self.error_at(
                start,
                KeyPathErrorKind::InvalidNumber,
                format!("malformed number `{text}`"),
            ));
        }
        if is_integer && let Ok(n) = text.parse::<i64>() {
            return Ok(FilterLiteral::Int(n));
        }
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(FilterLiteral::Number(n)),
            _ => Err(self.error_at(
                start,
                KeyPathErrorKind::InvalidNumber,
                format!("number `{text}` is out of range"),
            )),
        }
    }

    fn digits(&mut self) -> usize {
        let count = self.rest().bytes().take_while(u8::is_ascii_digit).count();
        self.pos += count;
        count
    }
}

const fn literal(value: FilterLiteral) -> FilterExpr {
    FilterExpr::Literal { value }
}

/// Canonical quoted spelling of `text` (`"…"` with escapes).
fn quote(text: &str) -> String {
    struct Quoted<'a>(&'a str);
    impl fmt::Display for Quoted<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write_quoted(f, self.0)
        }
    }
    Quoted(text).to_string()
}

impl From<KeyPathError> for Error {
    /// Error `E_KEY_SYNTAX` of the bind stage; the column in the key text is
    /// in the detail `key_column` and the corrected key, when known, in
    /// `key_fix`.
    fn from(error: KeyPathError) -> Self {
        let mut converted = Self::new(ErrorCode::KeySyntax, error.message)
            .with_stage(Stage::Bind)
            .with_detail("key_column", serde_json::Value::from(error.column));
        if let Some(fix) = error.fix {
            converted = converted.with_detail("key_fix", serde_json::Value::String(fix));
        }
        converted
    }
}

impl KeyPath {
    /// Converts the structured key of the surface syntax tree.
    ///
    /// Filter expressions may use fields, identifiers (fields of the
    /// element), text, numbers, `true`, `false`, `null`, the comparisons
    /// `==`, `!=`, `<`, `<=`, `>`, `>=` and `and`, `or`, `not`; other
    /// expressions fail with [`KeyPathErrorKind::Unsupported`].
    ///
    /// # Errors
    ///
    /// Returns [`KeyPathErrorKind::Empty`] for an empty list of segments,
    /// [`KeyPathErrorKind::Unsupported`] for an expression outside the list
    /// above, and the errors of [`KeyPath::push`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::ast::{KeySeg, Wildcard};
    /// use nurt_contracts::keypath::KeyPath;
    ///
    /// let segs = [KeySeg::Name("deps".to_owned()), KeySeg::Wild { wild: Wildcard::Append }];
    /// assert_eq!(KeyPath::from_key_segs(&segs)?.to_string(), "deps[+]");
    /// # Ok::<(), nurt_contracts::keypath::KeyPathError>(())
    /// ```
    pub fn from_key_segs(segs: &[KeySeg]) -> Result<Self, KeyPathError> {
        if segs.is_empty() {
            return Err(invalid_value(
                KeyPathErrorKind::Empty,
                "a structured key needs a segment",
            ));
        }
        let mut key = Self::root();
        for seg in segs {
            let segment = match seg {
                KeySeg::Name(name) => KeySegment::Key(name.clone()),
                KeySeg::Index(index) => KeySegment::Index(*index),
                KeySeg::Wild {
                    wild: Wildcard::Level,
                } => KeySegment::Wildcard,
                KeySeg::Wild {
                    wild: Wildcard::Deep,
                } => KeySegment::Recursive,
                KeySeg::Wild {
                    wild: Wildcard::Append,
                } => KeySegment::Append,
                KeySeg::Filter { filter } => KeySegment::Filter(filter_from_expr(filter)?),
            };
            key.push(segment)?;
        }
        Ok(key)
    }
}

fn unsupported_expr(what: &str) -> KeyPathError {
    invalid_value(
        KeyPathErrorKind::Unsupported,
        &format!("{what} is not supported in key filters"),
    )
}

fn filter_from_expr(expr: &Expr) -> Result<FilterExpr, KeyPathError> {
    let boxed = |e: &Expr| filter_from_expr(e).map(Box::new);
    Ok(match expr {
        Expr::Ident { v, .. } => FilterExpr::Field {
            path: vec![v.clone()],
        },
        Expr::Field { name, of, .. } => {
            let mut path = match of.as_deref().map(filter_from_expr).transpose()? {
                None => Vec::new(),
                Some(FilterExpr::Field { path }) => path,
                Some(_) => return Err(unsupported_expr("a field of a non-field expression")),
            };
            path.push(name.clone());
            FilterExpr::Field { path }
        }
        Expr::Text { v, .. } => literal(FilterLiteral::Text(v.clone())),
        Expr::Int { v, .. } => literal(FilterLiteral::Int(*v)),
        Expr::Num { v, .. } => literal(FilterLiteral::Number(*v)),
        Expr::Bool { v, .. } => literal(FilterLiteral::Bool(*v)),
        Expr::Null { .. } => literal(FilterLiteral::Null),
        Expr::Un {
            op: UnOp::Not, x, ..
        } => FilterExpr::Not { operand: boxed(x)? },
        Expr::Bin { op, l, r, .. } => binary_from_expr(*op, boxed(l)?, boxed(r)?)?,
        _ => return Err(unsupported_expr("this expression")),
    })
}

fn binary_from_expr(
    op: BinOp,
    left: Box<FilterExpr>,
    right: Box<FilterExpr>,
) -> Result<FilterExpr, KeyPathError> {
    let compare = match op {
        BinOp::Eq => CompareOp::Eq,
        BinOp::Ne => CompareOp::Ne,
        BinOp::Lt => CompareOp::Lt,
        BinOp::Le => CompareOp::Le,
        BinOp::Gt => CompareOp::Gt,
        BinOp::Ge => CompareOp::Ge,
        BinOp::And => return Ok(FilterExpr::And { left, right }),
        BinOp::Or => return Ok(FilterExpr::Or { left, right }),
        _ => return Err(unsupported_expr("this operator")),
    };
    Ok(FilterExpr::Compare {
        op: compare,
        left,
        right,
    })
}
