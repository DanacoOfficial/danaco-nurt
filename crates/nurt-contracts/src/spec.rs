// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Command and parameter specifications loaded from `catalog/*.toml`.
//!
//! The command catalog is the single source of command signatures for the
//! binder, the help system, the model description, the reference
//! documentation and the tool definitions. It is stored as one TOML file per
//! group (`catalog/core.toml` and one file per optional group) plus an index
//! (`catalog/_index.toml`). The file format is `nurt-catalog/1`, specified in
//! `spec/catalog.md`; its JSON schema is `spec/schemas/nurt-catalog-1.schema.json`.
//!
//! | Type | Content |
//! |---|---|
//! | [`CatalogFile`] | one catalog file: a group and its commands |
//! | [`CatalogIndex`] | the index of catalog files and the expected number of commands |
//! | [`StageSpec`] | the specification of one command |
//! | [`ParamSpec`] | the specification of one parameter |
//! | [`ParamType`] | a parsed type expression such as `path \| list<path>` |
//! | [`DefaultValue`] | a default value written as a literal of the language |
//! | [`IoSpec`] | a kind of stage input or output |
//! | [`FormSpec`] | an input-dependent form of a polymorphic command |
//!
//! The parser of type expressions is self-contained and reports typed errors
//! ([`TypeParseError`]). Consistency checks of a catalog ([`CatalogFile::check`],
//! [`CatalogIndex::check`], [`check_catalog`]) return a list of
//! [`CatalogIssue`] values instead of failing on the first problem.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::spec::{ParamType, ScalarType};
//!
//! let ty: ParamType = "text | regex | list<text | regex>".parse()?;
//! assert_eq!(ty.alternatives().len(), 3);
//! assert!(ty.accepts_list());
//! assert!(ty.contains_scalar(ScalarType::Regex));
//! assert_eq!(ty.to_string(), "text | regex | list<text | regex>");
//! # Ok::<(), nurt_contracts::spec::TypeParseError>(())
//! ```

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::kinds::{Category, EffectKind, Implementation, Reversibility};

/// Value of the `format` key of every catalog file.
pub const CATALOG_FORMAT: &str = "nurt-catalog/1";

/// Value of the `format` key of the catalog index.
pub const CATALOG_INDEX_FORMAT: &str = "nurt-catalog-index/1";

/// Identifier (`$id`) of the JSON schema of catalog files.
pub const CATALOG_SCHEMA_ID: &str =
    "https://nurt.danaco-group.pl/schema/nurt-catalog-1.schema.json";

/// Name of the group that holds the core commands.
pub const CORE_GROUP: &str = "core";

/// Maximum nesting depth of a type expression accepted by the parser.
pub const MAX_TYPE_DEPTH: usize = 16;

/// Smallest number of parameters in an exclusive group.
const MIN_EXCLUSIVE_GROUP_MEMBERS: usize = 2;

/// Call parameter that `default_from` may name besides the parameters of the
/// command.
const BASE_CALL_PARAMETER: &str = "base";

/// Record kind consumed by the stage form that an expanded source form binds
/// to.
const ENTRY_RECORD: &str = "entry";

fn is_false(value: &bool) -> bool {
    !*value
}

/// Returns `true` when `text` is a lowercase identifier: a lowercase ASCII
/// letter followed by lowercase ASCII letters, digits or underscores.
///
/// Parameter names, enumeration values, record kinds and group names are
/// identifiers.
///
/// # Examples
///
/// ```
/// use nurt_contracts::spec::is_identifier;
///
/// assert!(is_identifier("case_sensitive"));
/// assert!(is_identifier("sha256"));
/// assert!(!is_identifier("Sort"));
/// assert!(!is_identifier("2nd"));
/// assert!(!is_identifier(""));
/// ```
#[must_use]
pub fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        _ => false,
    }
}

/// Returns `true` when `text` is a valid command name: an identifier, or two
/// identifiers joined by a dot (`git.status`).
///
/// # Examples
///
/// ```
/// use nurt_contracts::spec::is_command_name;
///
/// assert!(is_command_name("replace"));
/// assert!(is_command_name("git.status"));
/// assert!(!is_command_name("git.stash.list"));
/// assert!(!is_command_name(".status"));
/// ```
#[must_use]
pub fn is_command_name(text: &str) -> bool {
    match text.split_once('.') {
        None => is_identifier(text),
        Some((prefix, rest)) => is_identifier(prefix) && is_identifier(rest),
    }
}

/// Scalar type of the language, a leaf of a [`ParamType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScalarType {
    /// Text (UTF-8).
    Text,
    /// Integer.
    Int,
    /// Number (integer or decimal).
    Number,
    /// Boolean.
    Bool,
    /// Path: a quoted text in a place of type `path`.
    Path,
    /// File name pattern.
    Glob,
    /// Regular expression `/…/flags`.
    Regex,
    /// Size in bytes (`10mb`).
    Size,
    /// Duration (`30s`).
    Duration,
    /// Share of a capacity, written as a quoted text such as `"10%"`.
    Percent,
    /// Point in time (UTC).
    Instant,
    /// Key path in a structured document (`"a.b[0]"`).
    Key,
    /// Raw bytes.
    Bytes,
    /// Range (`5`, `1..5`, `40..`, `..9`, `all`).
    Range,
    /// Expression evaluated on each input record (`filter`, `assert all:`).
    Expression,
    /// Nested pipeline (argument of `verify`, `write from:`).
    Pipeline,
    /// Name of a record field (`sort size`).
    Field,
    /// Pair `name: expression` that computes a new field (`select`).
    NamedExpression,
    /// Any value.
    Any,
}

impl ScalarType {
    /// Every scalar type, in the order of the type grammar.
    pub const ALL: &'static [ScalarType] = &[
        ScalarType::Text,
        ScalarType::Int,
        ScalarType::Number,
        ScalarType::Bool,
        ScalarType::Path,
        ScalarType::Glob,
        ScalarType::Regex,
        ScalarType::Size,
        ScalarType::Duration,
        ScalarType::Percent,
        ScalarType::Instant,
        ScalarType::Key,
        ScalarType::Bytes,
        ScalarType::Range,
        ScalarType::Expression,
        ScalarType::Pipeline,
        ScalarType::Field,
        ScalarType::NamedExpression,
        ScalarType::Any,
    ];

    /// Returns the keyword of the type in type expressions.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ScalarType::Text => "text",
            ScalarType::Int => "int",
            ScalarType::Number => "number",
            ScalarType::Bool => "bool",
            ScalarType::Path => "path",
            ScalarType::Glob => "glob",
            ScalarType::Regex => "regex",
            ScalarType::Size => "size",
            ScalarType::Duration => "duration",
            ScalarType::Percent => "percent",
            ScalarType::Instant => "instant",
            ScalarType::Key => "key",
            ScalarType::Bytes => "bytes",
            ScalarType::Range => "range",
            ScalarType::Expression => "expression",
            ScalarType::Pipeline => "pipeline",
            ScalarType::Field => "field",
            ScalarType::NamedExpression => "named_expression",
            ScalarType::Any => "any",
        }
    }

    /// Returns the scalar type with the given keyword.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::spec::ScalarType;
    ///
    /// assert_eq!(ScalarType::from_keyword("glob"), Some(ScalarType::Glob));
    /// assert_eq!(ScalarType::from_keyword("string"), None);
    /// ```
    #[must_use]
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|scalar| scalar.as_str() == keyword)
    }
}

impl fmt::Display for ScalarType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsed type expression of a parameter.
///
/// The text form follows the type grammar of `spec/catalog.md`:
///
/// * Scalars are `text`, `int`, `number`, `bool`, `path`, `glob`, `regex`, `percent`,
///   `size`, `duration`, `instant`, `key`, `bytes`, `range`, `expression`,
///   `pipeline`, `field`, `named_expression` and `any`.
/// * The form `enum(a|b|c)` is one of the listed bare words.
/// * The forms `list<T>` and `map<T>` are lists and maps; bare `list` and
///   `map` mean `list<any>` and `map<any>`.
/// * The form `stream<R>` is the result of a named pipeline (`@name`) with
///   records of kind `R` or `any`.
/// * The form `A | B` is any of the alternatives.
///
/// The value is serialized as its canonical text ([`fmt::Display`]) and parsed
/// on deserialization, so an invalid type expression makes the catalog file
/// invalid.
///
/// # Examples
///
/// ```
/// use nurt_contracts::spec::ParamType;
///
/// let ty: ParamType = "list<enum(vcs|deps|build|cache)>".parse()?;
/// assert_eq!(ty.enum_values(), vec!["vcs", "deps", "build", "cache"]);
/// assert_eq!("list".parse::<ParamType>()?.to_string(), "list<any>");
/// assert!("list<".parse::<ParamType>().is_err());
/// # Ok::<(), nurt_contracts::spec::TypeParseError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParamType {
    /// Scalar type.
    Scalar(ScalarType),
    /// Enumeration of bare words, in the written order.
    Enum(Vec<String>),
    /// List of values of the element type.
    List(Box<ParamType>),
    /// Map from text keys to values of the value type.
    Map(Box<ParamType>),
    /// Result of a named pipeline with records of the given kind (`any` for
    /// any kind).
    Stream(String),
    /// Any of the alternatives; never nested and never a single alternative.
    Union(Vec<ParamType>),
}

impl ParamType {
    /// Parses a type expression.
    ///
    /// # Errors
    ///
    /// Returns a [`TypeParseError`] with the byte offset of the problem when
    /// the text is not a valid type expression.
    pub fn parse(text: &str) -> Result<Self, TypeParseError> {
        let mut parser = TypeParser::new(text);
        parser.skip_space();
        if parser.at_end() {
            return Err(TypeParseError::new(TypeParseErrorKind::Empty, 0));
        }
        let parsed = parser.parse_union(0)?;
        parser.skip_space();
        match parser.peek_char() {
            None => Ok(parsed),
            Some(found) => Err(TypeParseError::new(
                TypeParseErrorKind::UnexpectedChar {
                    found,
                    expected: Cow::Borrowed("end of type"),
                },
                parser.pos,
            )),
        }
    }

    /// Returns the alternatives of a union, or the type itself as a single
    /// alternative.
    #[must_use]
    pub fn alternatives(&self) -> &[ParamType] {
        match self {
            ParamType::Union(alternatives) => alternatives,
            other => std::slice::from_ref(other),
        }
    }

    /// Returns `true` when one of the alternatives is a list.
    #[must_use]
    pub fn accepts_list(&self) -> bool {
        self.alternatives()
            .iter()
            .any(|alternative| matches!(alternative, ParamType::List(_)))
    }

    /// Returns `true` when the scalar type occurs anywhere in the expression.
    #[must_use]
    pub fn contains_scalar(&self, scalar: ScalarType) -> bool {
        match self {
            ParamType::Scalar(own) => *own == scalar,
            ParamType::Enum(_) | ParamType::Stream(_) => false,
            ParamType::List(inner) | ParamType::Map(inner) => inner.contains_scalar(scalar),
            ParamType::Union(alternatives) => alternatives
                .iter()
                .any(|alternative| alternative.contains_scalar(scalar)),
        }
    }

    /// Returns every enumeration value that occurs in the expression, in the
    /// written order and without repetitions.
    #[must_use]
    pub fn enum_values(&self) -> Vec<&str> {
        let mut values = Vec::new();
        self.collect_enum_values(&mut values);
        values
    }

    fn collect_enum_values<'a>(&'a self, values: &mut Vec<&'a str>) {
        match self {
            ParamType::Enum(own) => {
                for value in own {
                    if !values.contains(&value.as_str()) {
                        values.push(value.as_str());
                    }
                }
            }
            ParamType::List(inner) | ParamType::Map(inner) => inner.collect_enum_values(values),
            ParamType::Union(alternatives) => {
                for alternative in alternatives {
                    alternative.collect_enum_values(values);
                }
            }
            ParamType::Scalar(_) | ParamType::Stream(_) => {}
        }
    }
}

impl fmt::Display for ParamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamType::Scalar(scalar) => f.write_str(scalar.as_str()),
            ParamType::Enum(values) => write!(f, "enum({})", values.join("|")),
            ParamType::List(inner) => write!(f, "list<{inner}>"),
            ParamType::Map(inner) => write!(f, "map<{inner}>"),
            ParamType::Stream(record) => write!(f, "stream<{record}>"),
            ParamType::Union(alternatives) => {
                for (index, alternative) in alternatives.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{alternative}")?;
                }
                Ok(())
            }
        }
    }
}

impl FromStr for ParamType {
    type Err = TypeParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl Serialize for ParamType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ParamType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ParamType {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ParamType")
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Type expression: scalars (text, int, number, bool, path, glob, regex, size, duration, percent, instant, key, bytes, range, expression, pipeline, field, named_expression, any), enum(a|b), list<T>, map<T>, stream<R> and alternatives A | B.",
            "type": "string",
            "minLength": 1
        })
    }
}

/// Kind of a problem found by the type expression parser.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum TypeParseErrorKind {
    /// The expression is empty.
    Empty,
    /// The expression ended where more input was expected.
    UnexpectedEnd {
        /// What the parser expected.
        expected: Cow<'static, str>,
    },
    /// A character that cannot appear at this place.
    UnexpectedChar {
        /// The character found.
        found: char,
        /// What the parser expected.
        expected: Cow<'static, str>,
    },
    /// A word that is not a type keyword.
    UnknownType {
        /// The word found.
        name: String,
    },
    /// An enumeration value that is not an identifier.
    InvalidEnumValue {
        /// The value found.
        value: String,
    },
    /// An enumeration value given twice.
    DuplicateEnumValue {
        /// The repeated value.
        value: String,
    },
    /// An alternative of a union given twice.
    DuplicateAlternative {
        /// The repeated alternative, in canonical form.
        alternative: String,
    },
    /// The expression is nested deeper than [`MAX_TYPE_DEPTH`].
    TooDeep,
}

/// Error of the type expression parser.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct TypeParseError {
    /// Kind of the problem.
    pub kind: TypeParseErrorKind,
    /// Byte offset of the problem in the expression.
    pub offset: usize,
}

impl TypeParseError {
    /// Creates an error of the given kind at the given byte offset.
    #[must_use]
    pub const fn new(kind: TypeParseErrorKind, offset: usize) -> Self {
        Self { kind, offset }
    }
}

impl fmt::Display for TypeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            TypeParseErrorKind::Empty => f.write_str("empty type expression")?,
            TypeParseErrorKind::UnexpectedEnd { expected } => {
                write!(f, "unexpected end of type expression, expected {expected}")?;
            }
            TypeParseErrorKind::UnexpectedChar { found, expected } => {
                write!(f, "unexpected character {found:?}, expected {expected}")?;
            }
            TypeParseErrorKind::UnknownType { name } => write!(f, "unknown type {name:?}")?,
            TypeParseErrorKind::InvalidEnumValue { value } => {
                write!(
                    f,
                    "enumeration value {value:?} is not a lowercase identifier"
                )?;
            }
            TypeParseErrorKind::DuplicateEnumValue { value } => {
                write!(f, "enumeration value {value:?} is given twice")?;
            }
            TypeParseErrorKind::DuplicateAlternative { alternative } => {
                write!(f, "alternative {alternative:?} is given twice")?;
            }
            TypeParseErrorKind::TooDeep => {
                write!(
                    f,
                    "type expression is nested deeper than {MAX_TYPE_DEPTH} levels"
                )?;
            }
        }
        write!(f, " at offset {}", self.offset)
    }
}

impl std::error::Error for TypeParseError {}

struct TypeParser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> TypeParser<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.text.len()
    }

    fn peek_char(&self) -> Option<char> {
        self.text[self.pos..].chars().next()
    }

    fn skip_space(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == ' ' || c == '\t' {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> &'a str {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        &self.text[start..self.pos]
    }

    fn unexpected(&self, expected: &'static str) -> TypeParseError {
        let kind = match self.peek_char() {
            None => TypeParseErrorKind::UnexpectedEnd {
                expected: Cow::Borrowed(expected),
            },
            Some(found) => TypeParseErrorKind::UnexpectedChar {
                found,
                expected: Cow::Borrowed(expected),
            },
        };
        TypeParseError::new(kind, self.pos)
    }

    fn expect(&mut self, wanted: char, expected: &'static str) -> Result<(), TypeParseError> {
        self.skip_space();
        if self.peek_char() == Some(wanted) {
            self.pos += wanted.len_utf8();
            Ok(())
        } else {
            Err(self.unexpected(expected))
        }
    }

    fn parse_union(&mut self, depth: usize) -> Result<ParamType, TypeParseError> {
        if depth > MAX_TYPE_DEPTH {
            return Err(TypeParseError::new(TypeParseErrorKind::TooDeep, self.pos));
        }
        let mut alternatives: Vec<ParamType> = Vec::new();
        loop {
            self.skip_space();
            let start = self.pos;
            let term = self.parse_term(depth)?;
            if alternatives.contains(&term) {
                let alternative = term.to_string();
                return Err(TypeParseError::new(
                    TypeParseErrorKind::DuplicateAlternative { alternative },
                    start,
                ));
            }
            alternatives.push(term);
            self.skip_space();
            if self.peek_char() == Some('|') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if alternatives.len() == 1 {
            Ok(alternatives.swap_remove(0))
        } else {
            Ok(ParamType::Union(alternatives))
        }
    }

    fn parse_term(&mut self, depth: usize) -> Result<ParamType, TypeParseError> {
        let start = self.pos;
        let word = self.word();
        match word {
            "" => Err(self.unexpected("a type")),
            "list" | "map" => {
                self.skip_space();
                let inner = if self.peek_char() == Some('<') {
                    self.pos += 1;
                    let inner = self.parse_union(depth + 1)?;
                    self.expect('>', "'>'")?;
                    inner
                } else {
                    ParamType::Scalar(ScalarType::Any)
                };
                if word == "list" {
                    Ok(ParamType::List(Box::new(inner)))
                } else {
                    Ok(ParamType::Map(Box::new(inner)))
                }
            }
            "stream" => self.parse_stream(),
            "enum" => self.parse_enum(),
            other => ScalarType::from_keyword(other)
                .map(ParamType::Scalar)
                .ok_or_else(|| {
                    TypeParseError::new(
                        TypeParseErrorKind::UnknownType {
                            name: other.to_owned(),
                        },
                        start,
                    )
                }),
        }
    }

    fn parse_stream(&mut self) -> Result<ParamType, TypeParseError> {
        self.expect('<', "'<'")?;
        self.skip_space();
        let start = self.pos;
        let record = self.word();
        if !is_identifier(record) {
            self.pos = start;
            return Err(self.unexpected("a record kind"));
        }
        self.expect('>', "'>'")?;
        Ok(ParamType::Stream(record.to_owned()))
    }

    fn parse_enum(&mut self) -> Result<ParamType, TypeParseError> {
        self.expect('(', "'('")?;
        let mut values: Vec<String> = Vec::new();
        loop {
            self.skip_space();
            let start = self.pos;
            let value = self.word();
            if value.is_empty() {
                return Err(self.unexpected("an enumeration value"));
            }
            if !is_identifier(value) {
                return Err(TypeParseError::new(
                    TypeParseErrorKind::InvalidEnumValue {
                        value: value.to_owned(),
                    },
                    start,
                ));
            }
            if values.iter().any(|known| known == value) {
                return Err(TypeParseError::new(
                    TypeParseErrorKind::DuplicateEnumValue {
                        value: value.to_owned(),
                    },
                    start,
                ));
            }
            values.push(value.to_owned());
            self.skip_space();
            match self.peek_char() {
                Some('|') => self.pos += 1,
                Some(')') => {
                    self.pos += 1;
                    return Ok(ParamType::Enum(values));
                }
                _ => return Err(self.unexpected("'|' or ')'")),
            }
        }
    }
}

/// Kind of records accepted or produced by a stage.
///
/// Text forms: `none` (the command starts a pipeline), `any` (records of any
/// kind), `same` (output only: the records of the input, unchanged in kind),
/// a scalar keyword such as `int` or `text` (a single value), or a record kind
/// such as `entry`, `match` or `effect`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::spec::{IoSpec, ScalarType};
///
/// assert_eq!("none".parse::<IoSpec>()?, IoSpec::None);
/// assert_eq!("int".parse::<IoSpec>()?, IoSpec::Value(ScalarType::Int));
/// assert_eq!("entry".parse::<IoSpec>()?, IoSpec::Record("entry".to_owned()));
/// assert!("Entry".parse::<IoSpec>().is_err());
/// # Ok::<(), nurt_contracts::spec::TypeParseError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IoSpec {
    /// No input: the command is a source.
    None,
    /// Records of any kind.
    Any,
    /// Output only: records of the same kind as the input.
    Same,
    /// A single value of a scalar type (`count` produces `int`).
    Value(ScalarType),
    /// Records of the named kind.
    Record(String),
}

impl IoSpec {
    /// Returns `true` for [`IoSpec::None`].
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, IoSpec::None)
    }
}

impl fmt::Display for IoSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoSpec::None => f.write_str("none"),
            IoSpec::Any => f.write_str("any"),
            IoSpec::Same => f.write_str("same"),
            IoSpec::Value(scalar) => f.write_str(scalar.as_str()),
            IoSpec::Record(kind) => f.write_str(kind),
        }
    }
}

impl FromStr for IoSpec {
    type Err = TypeParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "none" => Ok(IoSpec::None),
            "any" => Ok(IoSpec::Any),
            "same" => Ok(IoSpec::Same),
            "" => Err(TypeParseError::new(TypeParseErrorKind::Empty, 0)),
            other => match ScalarType::from_keyword(other) {
                Some(scalar) => Ok(IoSpec::Value(scalar)),
                None if is_identifier(other) => Ok(IoSpec::Record(other.to_owned())),
                None => Err(TypeParseError::new(
                    TypeParseErrorKind::UnknownType {
                        name: other.to_owned(),
                    },
                    0,
                )),
            },
        }
    }
}

impl Serialize for IoSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IoSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for IoSpec {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("IoSpec")
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Kind of stage input or output: none, any, same (output only), a scalar type keyword, or a record kind.",
            "type": "string",
            "pattern": "^[a-z][a-z0-9_]*$"
        })
    }
}

/// Default value of a parameter, written as a literal of the language in
/// variant A (`1`, `true`, `"**"`, `asc`, `120s`, `10mb`, `[]`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct DefaultValue(String);

/// Lexical kind of a [`DefaultValue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DefaultKind {
    /// `true` or `false`.
    Bool,
    /// `null`.
    Null,
    /// Integer (`3`, `1_000`).
    Int,
    /// Decimal number (`3.5`).
    Number,
    /// Size with a unit (`10mb`).
    Size,
    /// Duration with units (`120s`, `1h30m`).
    Duration,
    /// Range (`1..5`, `40..`, `..9`).
    Range,
    /// Quoted or raw text.
    Text,
    /// Regular expression `/…/`.
    Regex,
    /// List literal `[…]`.
    List,
    /// Map literal `{…}`.
    Map,
    /// Bare word: an enumeration value or a named constant (`asc`, `all`).
    Word,
    /// Anything else.
    Other,
}

const DURATION_UNITS: &[&str] = &["ms", "s", "m", "h", "d", "w"];
const SIZE_UNITS: &[&str] = &["b", "kb", "mb", "gb", "tb", "kib", "mib", "gib", "tib"];

impl DefaultValue {
    /// Creates a default value from its literal text.
    #[must_use]
    pub fn new(literal: impl Into<String>) -> Self {
        Self(literal.into())
    }

    /// Returns the literal text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` when the literal is empty or only white space.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// Returns the lexical kind of the literal.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::spec::{DefaultKind, DefaultValue};
    ///
    /// assert_eq!(DefaultValue::new("120s").kind(), DefaultKind::Duration);
    /// assert_eq!(DefaultValue::new("10mb").kind(), DefaultKind::Size);
    /// assert_eq!(DefaultValue::new("\"**\"").kind(), DefaultKind::Text);
    /// assert_eq!(DefaultValue::new("asc").kind(), DefaultKind::Word);
    /// assert_eq!(DefaultValue::new("1..5").kind(), DefaultKind::Range);
    /// ```
    #[must_use]
    pub fn kind(&self) -> DefaultKind {
        classify_literal(self.0.trim())
    }
}

impl fmt::Display for DefaultValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn classify_literal(text: &str) -> DefaultKind {
    match text {
        "" => return DefaultKind::Other,
        "true" | "false" => return DefaultKind::Bool,
        "null" => return DefaultKind::Null,
        _ => {}
    }
    if text.starts_with('"') || text.starts_with("r\"") || text.starts_with("r#") {
        return DefaultKind::Text;
    }
    match text.chars().next() {
        Some('[') => return DefaultKind::List,
        Some('{') => return DefaultKind::Map,
        Some('/') => return DefaultKind::Regex,
        _ => {}
    }
    if text.contains("..") && text.split("..").all(|part| part.is_empty() || is_int(part)) {
        return DefaultKind::Range;
    }
    if is_int(text) {
        return DefaultKind::Int;
    }
    if is_decimal(text) {
        return DefaultKind::Number;
    }
    if let Some(kind) = unit_kind(text) {
        return kind;
    }
    if is_identifier(text) {
        DefaultKind::Word
    } else {
        DefaultKind::Other
    }
}

fn is_digits(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit()) && text.chars().all(|c| c.is_ascii_digit() || c == '_')
}

fn is_int(text: &str) -> bool {
    is_digits(text.strip_prefix('-').unwrap_or(text))
}

fn is_decimal(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    match unsigned.split_once('.') {
        Some((whole, fraction)) => is_digits(whole) && is_digits(fraction),
        None => false,
    }
}

fn unit_kind(text: &str) -> Option<DefaultKind> {
    let mut rest = text;
    let mut units: Vec<&str> = Vec::new();
    while !rest.is_empty() {
        let digits_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '_'))
            .unwrap_or(rest.len());
        if digits_end == 0 {
            return None;
        }
        let after = &rest[digits_end..];
        let unit_end = after
            .find(|c: char| !c.is_ascii_lowercase())
            .unwrap_or(after.len());
        if unit_end == 0 {
            return None;
        }
        units.push(&after[..unit_end]);
        rest = &after[unit_end..];
    }
    if units.is_empty() {
        None
    } else if units.iter().all(|unit| DURATION_UNITS.contains(unit)) {
        Some(DefaultKind::Duration)
    } else if units.len() == 1 && SIZE_UNITS.contains(&units[0]) {
        Some(DefaultKind::Size)
    } else {
        None
    }
}

/// Operating system family a command is limited to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Os {
    /// Linux.
    Linux,
    /// Apple macOS.
    Macos,
    /// Windows.
    Windows,
}

/// Specification of one parameter of a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// Parameter name (identifier).
    pub name: String,
    /// Type expression.
    #[serde(rename = "type")]
    pub ty: ParamType,
    /// Position among the positional parameters of the primary form,
    /// counted from 1; absent for named-only parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    /// The parameter is required in the primary form.
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// The positional parameter collects several arguments into a list.
    #[serde(default, skip_serializing_if = "is_false")]
    pub repeated: bool,
    /// The value may be an expression evaluated on each input record.
    #[serde(default, skip_serializing_if = "is_false")]
    pub per_record: bool,
    /// Default value as a literal of the language; absent means no value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<DefaultValue>,
    /// The default is "no limit".
    #[serde(default, skip_serializing_if = "is_false")]
    pub unbounded_default: bool,
    /// Parameter whose value is the default: another parameter of the
    /// command or the call parameter `base`. The value is converted to the
    /// type of this parameter, so a path becomes a one-element list. A
    /// required parameter with this key is missing only when its source has
    /// no value either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_from: Option<String>,
    /// Description of a default that is computed rather than literal (for
    /// example "the latest operation").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_note: Option<String>,
    /// Enumeration values of the type, repeated for readability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// One-sentence description.
    pub summary: String,
    /// Name of a group of mutually exclusive parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusive_group: Option<String>,
    /// A value of this parameter above the waiting threshold makes the call
    /// block; such stages are marked `waits` in the bound tree.
    #[serde(default, skip_serializing_if = "is_false")]
    pub waits: bool,
    /// The value is confidential (credentials in an address or a header); it
    /// is masked in the journal, the report and the printed bound tree.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

impl ParamSpec {
    /// Returns `true` when the parameter has a position.
    #[must_use]
    pub fn is_positional(&self) -> bool {
        self.position.is_some()
    }
}

/// Effect of one action of a command that differs from the command default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionEffect {
    /// Identifier of the action: a value of the `action` parameter, a value of
    /// another enumeration parameter (`method: post`), the name of a parameter
    /// whose use changes the effect (`save`), or the input record kind for
    /// commands whose effect depends on the kind of input (`job`); `note`
    /// states the condition when it is not obvious.
    pub action: String,
    /// Effect of the action.
    pub effect: EffectKind,
    /// Reversibility of the action.
    pub reversibility: Reversibility,
    /// Conditions of reversibility or other remarks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Use of a program or a system component outside the executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalUse {
    /// Kind of the external use.
    pub kind: Implementation,
    /// Names of the programs used (for example `launchctl`, `apt`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<String>,
    /// Parameter whose use triggers the external use; absent when the use is
    /// unconditional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Remark shown in the description and in the report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Input-dependent form of a polymorphic command.
///
/// A command with forms binds by the kind of its input: the form whose
/// `input` contains the input kind applies. The first form is the primary
/// form; the `position` and `required` attributes of the parameters describe
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FormSpec {
    /// Name of the form (`source`, `stage`, …).
    pub name: String,
    /// Input kinds of the form.
    pub input: Vec<IoSpec>,
    /// Output kinds of the form; empty means the output of the command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<IoSpec>,
    /// Positional parameters of the form, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positional: Vec<String>,
    /// Required parameters of the form; a name of an exclusive group means
    /// "exactly one parameter of the group".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    /// Parameters accepted by the form; empty means every parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<String>,
    /// Parameter of a source form that names the entries the command works
    /// on. The binder expands the form to an `info` head whose `path` is the
    /// value of this parameter, followed by the command bound in its form for
    /// `entry` input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand: Option<String>,
    /// One-sentence description of the form.
    pub summary: String,
}

/// Specification of one command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StageSpec {
    /// Full command name (`replace`, `git.status`).
    pub name: String,
    /// Group of the command; empty in catalog files, where the group is
    /// given once per file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Category.
    pub category: Category,
    /// Reversibility of the default action.
    pub reversibility: Reversibility,
    /// Effect of the default action.
    pub effect: EffectKind,
    /// Implementation marker.
    pub implementation: Implementation,
    /// Accepted input kinds.
    pub input: Vec<IoSpec>,
    /// Produced output kinds.
    pub output: Vec<IoSpec>,
    /// One-sentence description.
    pub summary: String,
    /// Conditions of reversibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversibility_note: Option<String>,
    /// Actions whose effect differs from the command default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ActionEffect>,
    /// The command always blocks the call by waiting (`sleep`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub waits: bool,
    /// Operating systems the command is limited to; empty means all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub os: Vec<Os>,
    /// Use of an external program or system component beyond the
    /// implementation marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalUse>,
    /// Implementation used when the primary one is unavailable (syntax
    /// analysis when no language server is installed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Implementation>,
    /// Exclusive groups of which exactly one parameter must be given in the
    /// primary form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_groups: Vec<String>,
    /// Parameters.
    #[serde(rename = "param", default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamSpec>,
    /// Input-dependent forms; empty when the command has a single form.
    #[serde(rename = "form", default, skip_serializing_if = "Vec::is_empty")]
    pub forms: Vec<FormSpec>,
}

impl StageSpec {
    /// Returns the parameter with the given name.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|param| param.name == name)
    }

    /// Returns the positional parameters of the primary form, ordered by
    /// position.
    #[must_use]
    pub fn positional(&self) -> Vec<&ParamSpec> {
        let mut positional: Vec<&ParamSpec> = self
            .params
            .iter()
            .filter(|param| param.is_positional())
            .collect();
        positional.sort_by_key(|param| param.position);
        positional
    }

    /// Returns the exclusive groups declared by the parameters, with their
    /// members in declaration order.
    #[must_use]
    pub fn exclusive_groups(&self) -> BTreeMap<&str, Vec<&str>> {
        let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for param in &self.params {
            if let Some(group) = &param.exclusive_group {
                groups
                    .entry(group.as_str())
                    .or_default()
                    .push(param.name.as_str());
            }
        }
        groups
    }

    /// Returns the form that applies to the given input kind.
    ///
    /// For a command without forms this is always `None`; the command itself
    /// is the only form.
    #[must_use]
    pub fn form_for(&self, input: &IoSpec) -> Option<&FormSpec> {
        self.forms.iter().find(|form| form.input.contains(input))
    }

    /// Checks the consistency of the specification.
    ///
    /// The returned list is empty when the specification is consistent.
    #[must_use]
    pub fn check(&self) -> Vec<CatalogIssue> {
        let mut issues = Vec::new();
        self.check_header(&mut issues);
        self.check_params(&mut issues);
        self.check_positions(&mut issues);
        self.check_groups(&mut issues);
        self.check_forms(&mut issues);
        self.check_effects(&mut issues);
        issues
    }

    fn issue(&self, message: impl Into<String>) -> CatalogIssue {
        CatalogIssue::new(Some(self.name.clone()), None, message)
    }

    fn param_issue(&self, param: &str, message: impl Into<String>) -> CatalogIssue {
        CatalogIssue::new(Some(self.name.clone()), Some(param.to_owned()), message)
    }

    fn check_header(&self, issues: &mut Vec<CatalogIssue>) {
        if !is_command_name(&self.name) {
            issues.push(self.issue("command name is not an identifier or a dotted pair"));
        }
        if !is_sentence(&self.summary) {
            issues.push(self.issue("summary must be one non-empty sentence ending with '.'"));
        }
        if self.input.is_empty() {
            issues.push(self.issue("input must list at least one kind"));
        }
        if self.output.is_empty() {
            issues.push(self.issue("output must list at least one kind"));
        }
        if self.input.contains(&IoSpec::Same) {
            issues.push(self.issue("input cannot be 'same'"));
        }
        if self
            .reversibility_note
            .as_deref()
            .is_some_and(|note| note.trim().is_empty())
        {
            issues.push(self.issue("reversibility_note is empty"));
        }
        if self.fallback == Some(self.implementation) {
            issues.push(self.issue("fallback repeats the implementation"));
        }
        let when = self
            .external
            .as_ref()
            .and_then(|external| external.when.as_deref());
        if let Some(when) = when
            && self.param(when).is_none()
        {
            issues.push(self.issue(format!("external.when names unknown parameter {when}")));
        }
    }

    fn check_params(&self, issues: &mut Vec<CatalogIssue>) {
        let mut names = HashSet::new();
        for param in &self.params {
            if !names.insert(param.name.as_str()) {
                issues.push(self.param_issue(&param.name, "duplicate parameter name"));
            }
            self.check_param(param, issues);
        }
    }

    fn check_param(&self, param: &ParamSpec, issues: &mut Vec<CatalogIssue>) {
        let mut problems: Vec<String> = Vec::new();
        if !is_identifier(&param.name) {
            problems.push("parameter name is not an identifier".to_owned());
        }
        if param.summary.trim().is_empty() {
            problems.push("summary is empty".to_owned());
        }
        if param.default.as_ref().is_some_and(DefaultValue::is_blank) {
            problems.push("default is empty".to_owned());
        }
        if param.default.is_some() && param.unbounded_default {
            problems.push("default and unbounded_default are both given".to_owned());
        }
        if param.required && (param.default.is_some() || param.unbounded_default) {
            problems.push("a required parameter cannot have a default".to_owned());
        }
        if param.repeated && !param.ty.accepts_list() {
            problems.push("a repeated parameter must accept a list".to_owned());
        }
        if param.repeated && !param.is_positional() {
            problems.push("a repeated parameter must be positional".to_owned());
        }
        if let Some(source) = param.default_from.as_deref() {
            if source == param.name {
                problems.push("default_from names the parameter itself".to_owned());
            } else if source != BASE_CALL_PARAMETER && self.param(source).is_none() {
                problems.push(format!("default_from names unknown parameter {source}"));
            }
            if param.default.is_some() || param.unbounded_default {
                problems.push("default_from and a default are both given".to_owned());
            }
        }
        if param
            .default_note
            .as_deref()
            .is_some_and(|note| note.trim().is_empty())
        {
            problems.push("default_note is empty".to_owned());
        }
        if param
            .exclusive_group
            .as_deref()
            .is_some_and(|group| !is_identifier(group))
        {
            problems.push("exclusive_group is not an identifier".to_owned());
        }
        check_values(param, &mut problems);
        for problem in problems {
            issues.push(self.param_issue(&param.name, problem));
        }
    }

    fn check_positions(&self, issues: &mut Vec<CatalogIssue>) {
        let mut positions: Vec<u32> = self
            .params
            .iter()
            .filter_map(|param| param.position)
            .collect();
        positions.sort_unstable();
        let expected: Vec<u32> = (1..).take(positions.len()).collect();
        if positions != expected {
            issues.push(self.issue(format!(
                "positions {positions:?} do not form the sequence 1..{}",
                positions.len()
            )));
        }
        let repeated = self.params.iter().filter(|param| param.repeated).count();
        if repeated > 1 {
            issues.push(self.issue("no more than one parameter can be repeated"));
        }
    }

    fn check_groups(&self, issues: &mut Vec<CatalogIssue>) {
        let groups = self.exclusive_groups();
        for (group, members) in &groups {
            if members.len() < MIN_EXCLUSIVE_GROUP_MEMBERS {
                issues.push(self.issue(format!("exclusive group {group} has a single member")));
            }
            if self.param(group).is_some() {
                issues.push(self.issue(format!("exclusive group {group} has a parameter's name")));
            }
        }
        for group in &self.required_groups {
            if !groups.contains_key(group.as_str()) {
                issues
                    .push(self.issue(format!("required group {group} is not an exclusive group")));
            }
        }
    }

    fn check_effects(&self, issues: &mut Vec<CatalogIssue>) {
        let mut actions = HashSet::new();
        for effect in &self.effects {
            if !is_identifier(&effect.action) {
                issues.push(self.issue(format!(
                    "effect action {:?} is not an identifier",
                    effect.action
                )));
            }
            if !actions.insert(effect.action.as_str()) {
                issues.push(self.issue(format!("effect action {} is given twice", effect.action)));
            }
            if effect
                .note
                .as_deref()
                .is_some_and(|note| note.trim().is_empty())
            {
                issues.push(self.issue(format!("effect note of {} is empty", effect.action)));
            }
        }
    }

    fn check_forms(&self, issues: &mut Vec<CatalogIssue>) {
        let Some(primary) = self.forms.first() else {
            return;
        };
        let mut names = HashSet::new();
        let mut inputs: Vec<&IoSpec> = Vec::new();
        for form in &self.forms {
            if !names.insert(form.name.as_str()) {
                issues.push(self.issue(format!("form {} is given twice", form.name)));
            }
            for input in &form.input {
                if inputs.contains(&input) {
                    issues.push(self.issue(format!("input {input} belongs to several forms")));
                }
                inputs.push(input);
            }
            self.check_form(form, issues);
        }
        for input in &self.input {
            if !inputs.contains(&input) {
                issues.push(self.issue(format!("input {input} has no form")));
            }
        }
        self.check_primary_form(primary, issues);
    }

    fn check_form(&self, form: &FormSpec, issues: &mut Vec<CatalogIssue>) {
        let groups = self.exclusive_groups();
        let context = format!("form {}", form.name);
        if form.input.is_empty() {
            issues.push(self.issue(format!("{context}: input is empty")));
        }
        if !is_sentence(&form.summary) {
            issues.push(self.issue(format!("{context}: summary must be one sentence")));
        }
        for input in &form.input {
            if !self.input.contains(input) {
                issues.push(self.issue(format!("{context}: input {input} is not a command input")));
            }
        }
        for output in &form.output {
            if !self.output.contains(output) {
                issues.push(self.issue(format!(
                    "{context}: output {output} is not a command output"
                )));
            }
        }
        for name in form.params.iter().chain(&form.positional) {
            if self.param(name).is_none() {
                issues.push(self.issue(format!("{context}: unknown parameter {name}")));
            }
        }
        let accepts = |name: &str| form.params.is_empty() || form.params.iter().any(|p| p == name);
        for name in &form.positional {
            if !accepts(name) {
                issues.push(self.issue(format!("{context}: positional {name} is not accepted")));
            }
        }
        for name in &form.required {
            let known = self.param(name).is_some() || groups.contains_key(name.as_str());
            if !known {
                issues.push(self.issue(format!("{context}: unknown required name {name}")));
            }
        }
        if let Some(name) = form.expand.as_deref() {
            self.check_expansion(form, name, issues);
        }
    }

    fn check_expansion(&self, form: &FormSpec, name: &str, issues: &mut Vec<CatalogIssue>) {
        let context = format!("form {}", form.name);
        if !form.input.iter().all(IoSpec::is_none) {
            issues.push(self.issue(format!("{context}: expand requires a source form")));
        }
        let entry = IoSpec::Record(ENTRY_RECORD.to_owned());
        let Some(stage) = self.forms.iter().find(|other| other.input.contains(&entry)) else {
            issues.push(self.issue(format!("{context}: expand requires a form for entry input")));
            return;
        };
        match self.param(name) {
            Some(param) if param.ty.contains_scalar(ScalarType::Path) => {}
            Some(_) => issues.push(self.issue(format!("{context}: expand {name} is not a path"))),
            None => {
                issues.push(self.issue(format!("{context}: expand names unknown parameter {name}")))
            }
        }
        if stage.params.is_empty() || stage.params.iter().any(|param| param == name) {
            issues.push(self.issue(format!(
                "{context}: expand {name} is also a parameter of form {}",
                stage.name
            )));
        }
    }

    fn check_primary_form(&self, primary: &FormSpec, issues: &mut Vec<CatalogIssue>) {
        let positional: Vec<&str> = self
            .positional()
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        let form_positional: Vec<&str> = primary.positional.iter().map(String::as_str).collect();
        if positional != form_positional {
            issues.push(self.issue(format!(
                "primary form {} lists positional {form_positional:?}, parameters give {positional:?}",
                primary.name
            )));
        }
        let mut required: Vec<&str> = self
            .params
            .iter()
            .filter(|param| param.required)
            .map(|param| param.name.as_str())
            .chain(self.required_groups.iter().map(String::as_str))
            .collect();
        let mut form_required: Vec<&str> = primary.required.iter().map(String::as_str).collect();
        required.sort_unstable();
        form_required.sort_unstable();
        if required != form_required {
            issues.push(self.issue(format!(
                "primary form {} requires {form_required:?}, parameters require {required:?}",
                primary.name
            )));
        }
    }
}

fn check_values(param: &ParamSpec, problems: &mut Vec<String>) {
    let enum_values = param.ty.enum_values();
    let listed: Vec<&str> = param.values.iter().map(String::as_str).collect();
    if enum_values != listed {
        problems.push(format!(
            "values {listed:?} do not repeat the enumeration values {enum_values:?} of the type"
        ));
    }
    if let Some(default) = &param.default
        && default.kind() == DefaultKind::Word
    {
        let word = default.as_str().trim();
        let allowed = listed.contains(&word)
            || (word == "all" && param.ty.contains_scalar(ScalarType::Range));
        if !allowed {
            problems.push(format!("default {word} is not a value of the type"));
        }
    }
}

fn is_sentence(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.ends_with('.') && !trimmed.contains('\n')
}

/// One catalog file (`catalog/<group>.toml`): a group and its commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogFile {
    /// Format identifier, always `nurt-catalog/1`.
    pub format: String,
    /// Group name: `core` or the name of an optional group.
    pub group: String,
    /// One-sentence description of the group.
    pub summary: String,
    /// Commands of the group.
    #[serde(rename = "command", default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<StageSpec>,
}

impl CatalogFile {
    /// Sets the group of every command that does not name one to the group of
    /// the file.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::spec::CatalogFile;
    ///
    /// let json = serde_json::json!({
    ///     "format": "nurt-catalog/1",
    ///     "group": "core",
    ///     "summary": "Core commands.",
    ///     "command": [{
    ///         "name": "count",
    ///         "category": "transform",
    ///         "reversibility": "read",
    ///         "effect": "read",
    ///         "implementation": "builtin",
    ///         "input": ["any"],
    ///         "output": ["int"],
    ///         "summary": "Count the input records."
    ///     }]
    /// });
    /// let mut file: CatalogFile = serde_json::from_value(json)?;
    /// file.resolve_groups();
    /// assert_eq!(file.commands[0].group, "core");
    /// assert!(file.check().is_empty());
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    pub fn resolve_groups(&mut self) {
        for command in &mut self.commands {
            if command.group.is_empty() {
                command.group.clone_from(&self.group);
            }
        }
    }

    /// Returns the command with the given name.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<&StageSpec> {
        self.commands.iter().find(|command| command.name == name)
    }

    /// Checks the consistency of the file and of every command in it.
    ///
    /// The returned list is empty when the file is consistent.
    #[must_use]
    pub fn check(&self) -> Vec<CatalogIssue> {
        let mut issues = Vec::new();
        if self.format != CATALOG_FORMAT {
            issues.push(CatalogIssue::new(
                None,
                None,
                format!("{}: format must be {CATALOG_FORMAT}", self.group),
            ));
        }
        if !is_identifier(&self.group) {
            issues.push(CatalogIssue::new(None, None, "group is not an identifier"));
        }
        if !is_sentence(&self.summary) {
            issues.push(CatalogIssue::new(
                None,
                None,
                format!("{}: summary must be one sentence", self.group),
            ));
        }
        let mut names = HashSet::new();
        for command in &self.commands {
            if !names.insert(command.name.as_str()) {
                issues.push(command.issue("duplicate command name"));
            }
            if !command.group.is_empty() && command.group != self.group {
                issues.push(
                    command.issue(format!("group differs from the file group {}", self.group)),
                );
            }
            issues.extend(command.check());
        }
        issues
    }
}

/// Entry of the catalog index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogIndexEntry {
    /// Group name.
    pub name: String,
    /// File name relative to the catalog directory (`core.toml`).
    pub file: String,
    /// One-sentence description of the group.
    pub summary: String,
    /// Number of commands in the file.
    pub commands: u32,
}

/// Index of the catalog files (`catalog/_index.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogIndex {
    /// Format identifier, always `nurt-catalog-index/1`.
    pub format: String,
    /// Expected number of commands in all files together.
    pub total: u32,
    /// Catalog files, core first.
    #[serde(rename = "file", default)]
    pub files: Vec<CatalogIndexEntry>,
}

impl CatalogIndex {
    /// Returns the entry of the given group.
    #[must_use]
    pub fn entry(&self, group: &str) -> Option<&CatalogIndexEntry> {
        self.files.iter().find(|entry| entry.name == group)
    }

    /// Returns the sum of the command counts of all entries.
    #[must_use]
    pub fn listed_total(&self) -> u64 {
        self.files
            .iter()
            .map(|entry| u64::from(entry.commands))
            .sum()
    }

    /// Checks the consistency of the index.
    ///
    /// The returned list is empty when the index is consistent.
    #[must_use]
    pub fn check(&self) -> Vec<CatalogIssue> {
        let mut issues = Vec::new();
        let issue = |message: String| CatalogIssue::new(None, None, message);
        if self.format != CATALOG_INDEX_FORMAT {
            issues.push(issue(format!(
                "index format must be {CATALOG_INDEX_FORMAT}"
            )));
        }
        if self
            .files
            .first()
            .is_none_or(|entry| entry.name != CORE_GROUP)
        {
            issues.push(issue(format!("the first index entry must be {CORE_GROUP}")));
        }
        let mut names = HashSet::new();
        for entry in &self.files {
            if !names.insert(entry.name.as_str()) {
                issues.push(issue(format!("group {} is listed twice", entry.name)));
            }
            if !is_identifier(&entry.name) {
                issues.push(issue(format!("group {} is not an identifier", entry.name)));
            }
            if entry.file != format!("{}.toml", entry.name) {
                issues.push(issue(format!(
                    "group {} must be stored in {}.toml",
                    entry.name, entry.name
                )));
            }
            if !is_sentence(&entry.summary) {
                issues.push(issue(format!(
                    "group {}: summary must be one sentence",
                    entry.name
                )));
            }
        }
        if self.listed_total() != u64::from(self.total) {
            issues.push(issue(format!(
                "entries list {} commands, total is {}",
                self.listed_total(),
                self.total
            )));
        }
        issues
    }
}

/// Checks a set of catalog files against the index.
///
/// The set may be incomplete: every file present must be listed with its
/// command count, and command names must be unique across files. When every
/// listed file is present, the number of commands must equal the index total.
#[must_use]
pub fn check_catalog(index: &CatalogIndex, files: &[CatalogFile]) -> Vec<CatalogIssue> {
    let mut issues = index.check();
    let mut groups = HashSet::new();
    let mut names = HashSet::new();
    let mut present: u64 = 0;
    for file in files {
        issues.extend(file.check());
        if !groups.insert(file.group.as_str()) {
            issues.push(CatalogIssue::new(
                None,
                None,
                format!("group {} is loaded twice", file.group),
            ));
        }
        let count = u64::try_from(file.commands.len()).unwrap_or(u64::MAX);
        present = present.saturating_add(count);
        match index.entry(&file.group) {
            None => issues.push(CatalogIssue::new(
                None,
                None,
                format!("group {} is not listed in the index", file.group),
            )),
            Some(entry) if u64::from(entry.commands) != count => issues.push(CatalogIssue::new(
                None,
                None,
                format!(
                    "group {} has {count} commands, the index lists {}",
                    file.group, entry.commands
                ),
            )),
            Some(_) => {}
        }
        for command in &file.commands {
            if !names.insert(command.name.as_str()) {
                issues.push(command.issue("command name is used in several groups"));
            }
        }
    }
    let complete = index
        .files
        .iter()
        .all(|entry| groups.contains(entry.name.as_str()));
    if complete && present != u64::from(index.total) {
        issues.push(CatalogIssue::new(
            None,
            None,
            format!(
                "the catalog has {present} commands, the index expects {}",
                index.total
            ),
        ));
    }
    issues
}

/// A consistency problem found in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CatalogIssue {
    /// Command concerned, when the problem belongs to a command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Parameter concerned, when the problem belongs to a parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// Description of the problem.
    pub message: String,
}

impl CatalogIssue {
    /// Creates an issue.
    #[must_use]
    pub fn new(command: Option<String>, param: Option<String>, message: impl Into<String>) -> Self {
        Self {
            command,
            param,
            message: message.into(),
        }
    }
}

impl fmt::Display for CatalogIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.command, &self.param) {
            (Some(command), Some(param)) => write!(f, "{command}.{param}: {}", self.message),
            (Some(command), None) => write!(f, "{command}: {}", self.message),
            (None, _) => f.write_str(&self.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kind(text: &str) -> DefaultKind {
        DefaultValue::new(text).kind()
    }

    fn spec_with(params: serde_json::Value) -> StageSpec {
        serde_json::from_value(json!({
            "name": "cmd",
            "category": "transform",
            "reversibility": "read",
            "effect": "read",
            "implementation": "builtin",
            "input": ["any"],
            "output": ["int"],
            "summary": "Count the records.",
            "param": params
        }))
        .unwrap()
    }

    fn messages(spec: &StageSpec) -> Vec<String> {
        spec.check()
            .into_iter()
            .map(|issue| issue.message)
            .collect()
    }

    #[test]
    fn default_durations_accept_every_unit_including_weeks() {
        for text in ["250ms", "30s", "5m", "2h", "7d", "1w", "2w3d", "1h30m"] {
            assert_eq!(kind(text), DefaultKind::Duration, "{text}");
        }
    }

    #[test]
    fn default_sizes_accept_decimal_and_binary_units() {
        let sizes = [
            "1b", "10kb", "10mb", "3gb", "2tb", "1kib", "256mib", "1gib", "1.5tib", "1_000kb",
        ];
        for text in sizes {
            assert_eq!(kind(text), DefaultKind::Size, "{text}");
        }
        for text in ["1h30mb", "5x", "10 mb", "1w1mb"] {
            assert_eq!(kind(text), DefaultKind::Other, "{text}");
        }
    }

    #[test]
    fn default_literals_are_classified_by_their_first_characters() {
        let cases = [
            ("true", DefaultKind::Bool),
            ("null", DefaultKind::Null),
            ("-3", DefaultKind::Int),
            ("1_000", DefaultKind::Int),
            ("3.5", DefaultKind::Number),
            ("1..5", DefaultKind::Range),
            ("..9", DefaultKind::Range),
            ("40..", DefaultKind::Range),
            ("\"**\"", DefaultKind::Text),
            ("[]", DefaultKind::List),
            ("{}", DefaultKind::Map),
            ("/x/", DefaultKind::Regex),
            ("asc", DefaultKind::Word),
            ("", DefaultKind::Other),
        ];
        for (text, expected) in cases {
            assert_eq!(kind(text), expected, "{text:?}");
        }
    }

    #[test]
    fn type_expressions_print_in_canonical_form_and_read_back() {
        let texts = [
            "text | regex | list<text | regex>",
            "stream<entry>",
            "enum(asc|desc)",
            "map<int>",
            "list<list<enum(a|b)>>",
        ];
        for text in texts {
            let parsed: ParamType = text.parse().unwrap();
            assert_eq!(parsed.to_string(), text);
        }
        assert_eq!(
            "list".parse::<ParamType>().unwrap().to_string(),
            "list<any>"
        );
        assert_eq!(
            "  int  |\tbool ".parse::<ParamType>().unwrap().to_string(),
            "int | bool"
        );
    }

    #[test]
    fn type_expression_errors_carry_kind_and_byte_offset() {
        let cases = [
            ("", TypeParseErrorKind::Empty, 0),
            ("   ", TypeParseErrorKind::Empty, 0),
            (
                "bogus",
                TypeParseErrorKind::UnknownType {
                    name: "bogus".to_owned(),
                },
                0,
            ),
            (
                "text | text",
                TypeParseErrorKind::DuplicateAlternative {
                    alternative: "text".to_owned(),
                },
                7,
            ),
            (
                "enum(a|a)",
                TypeParseErrorKind::DuplicateEnumValue {
                    value: "a".to_owned(),
                },
                7,
            ),
            (
                "enum(1a)",
                TypeParseErrorKind::InvalidEnumValue {
                    value: "1a".to_owned(),
                },
                5,
            ),
        ];
        for (text, kind, offset) in cases {
            let error = text.parse::<ParamType>().unwrap_err();
            assert_eq!(error, TypeParseError::new(kind, offset), "{text:?}");
        }
        let error = "text | \u{17c}".parse::<ParamType>().unwrap_err();
        assert_eq!(error.offset, 7);
        assert!(matches!(
            error.kind,
            TypeParseErrorKind::UnexpectedChar {
                found: '\u{17c}',
                ..
            }
        ));
        let error = "list<".parse::<ParamType>().unwrap_err();
        assert!(matches!(
            error.kind,
            TypeParseErrorKind::UnexpectedEnd { .. }
        ));
        let error = "text text".parse::<ParamType>().unwrap_err();
        assert!(matches!(
            error.kind,
            TypeParseErrorKind::UnexpectedChar { .. }
        ));
    }

    #[test]
    fn type_expression_nesting_is_limited() {
        let nested = |depth: usize| format!("{}text{}", "list<".repeat(depth), ">".repeat(depth));
        assert!(nested(MAX_TYPE_DEPTH).parse::<ParamType>().is_ok());
        let error = nested(MAX_TYPE_DEPTH + 1).parse::<ParamType>().unwrap_err();
        assert_eq!(error.kind, TypeParseErrorKind::TooDeep);
    }

    #[test]
    fn io_specs_read_keywords_scalars_and_record_kinds() {
        assert_eq!("none".parse::<IoSpec>().unwrap(), IoSpec::None);
        assert_eq!("same".parse::<IoSpec>().unwrap(), IoSpec::Same);
        assert_eq!(
            "duration".parse::<IoSpec>().unwrap(),
            IoSpec::Value(ScalarType::Duration)
        );
        assert_eq!(
            "test_run".parse::<IoSpec>().unwrap().to_string(),
            "test_run"
        );
        for text in ["", "Entry", "a-b", "1x"] {
            assert!(text.parse::<IoSpec>().is_err(), "{text:?}");
        }
    }

    #[test]
    fn check_reports_gaps_in_positions_and_duplicate_parameters() {
        let spec = spec_with(json!([
            {"name": "a", "type": "text", "position": 1, "summary": "A."},
            {"name": "b", "type": "text", "position": 3, "summary": "B."},
            {"name": "b", "type": "text", "summary": "B again."}
        ]));
        let found = messages(&spec);
        assert!(
            found.iter().any(|m| m.contains("do not form the sequence")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|m| m.contains("duplicate parameter name")),
            "{found:?}"
        );
    }

    #[test]
    fn check_validates_enumeration_values_and_word_defaults() {
        let consistent = spec_with(json!([
            {"name": "order", "type": "enum(asc|desc)", "values": ["asc", "desc"],
             "default": "asc", "summary": "Order."},
            {"name": "limit", "type": "range", "default": "all", "summary": "Limit."}
        ]));
        assert!(consistent.check().is_empty(), "{:?}", messages(&consistent));
        let wrong_default = spec_with(json!([
            {"name": "order", "type": "enum(asc|desc)", "values": ["asc", "desc"],
             "default": "up", "summary": "Order."}
        ]));
        assert!(
            messages(&wrong_default)
                .iter()
                .any(|m| m.contains("is not a value of the type"))
        );
        let missing_values = spec_with(json!([
            {"name": "order", "type": "enum(asc|desc)", "summary": "Order."}
        ]));
        assert!(
            messages(&missing_values)
                .iter()
                .any(|m| m.contains("do not repeat"))
        );
    }

    #[test]
    fn check_validates_exclusive_groups_and_repeated_parameters() {
        let spec = spec_with(json!([
            {"name": "a", "type": "text", "exclusive_group": "pick", "summary": "A."},
            {"name": "b", "type": "text", "repeated": true, "summary": "B."}
        ]));
        let found = messages(&spec);
        assert!(
            found.iter().any(|m| m.contains("has a single member")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|m| m.contains("must accept a list")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|m| m.contains("must be positional")),
            "{found:?}"
        );
    }

    #[test]
    fn index_check_compares_the_listed_total_with_the_declared_total() {
        let index = CatalogIndex {
            format: CATALOG_INDEX_FORMAT.to_owned(),
            total: 2,
            files: vec![CatalogIndexEntry {
                name: CORE_GROUP.to_owned(),
                file: "core.toml".to_owned(),
                summary: "Core commands.".to_owned(),
                commands: 1,
            }],
        };
        let found: Vec<String> = index
            .check()
            .into_iter()
            .map(|issue| issue.message)
            .collect();
        assert_eq!(found, ["entries list 1 commands, total is 2"]);
        assert_eq!(index.listed_total(), 1);
        assert!(index.entry("missing").is_none());
    }
}
