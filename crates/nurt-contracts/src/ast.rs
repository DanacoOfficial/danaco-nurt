// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Surface syntax tree `nurt-ast/1`.
//!
//! The parser turns program text of any syntax variant into a surface tree.
//! The surface tree does not depend on the command catalog: positional
//! arguments keep their order, literals keep their written kind (a path, a
//! glob, an instant and a key are `text` nodes) and no default value is added.
//! Binding a surface tree with a catalog produces the bound tree
//! [`crate::bound::BoundDocument`].
//!
//! Every node object carries its kind in the member `"t"`. Optional members
//! are omitted when absent. The normative definition of the JSON form is the
//! published schema `nurt-ast-1.schema.json`.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::ast::{
//!     AST_VERSION, Arg, AstDocument, Expr, Head, Pipeline, Program, Stage, Statement,
//! };
//!
//! let stage = Stage {
//!     name: "find".to_owned(),
//!     args: vec![Arg {
//!         name: None,
//!         value: Expr::Text { v: "/srv/app".to_owned(), span: None },
//!         span: None,
//!     }],
//!     span: None,
//! };
//! let document = AstDocument::new(Program {
//!     statements: vec![Statement::Pipeline(Pipeline {
//!         head: Head::Stage(stage),
//!         stages: Vec::new(),
//!         span: None,
//!     })],
//! });
//! let json = serde_json::to_value(&document)?;
//! assert_eq!(json["nurt"], AST_VERSION);
//! assert_eq!(json["program"]["statements"][0]["head"]["t"], "stage");
//! # Ok::<(), serde_json::Error>(())
//! ```

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// Name of the surface tree format.
pub const AST_FORMAT: &str = "nurt-ast/1";

/// Format version written into the member `"nurt"` of a surface tree by this
/// version of the contract.
pub const AST_VERSION: &str = "1.0";

/// Media type of a surface tree encoded as JSON.
pub const AST_MEDIA_TYPE_JSON: &str = "application/vnd.nurt.ast+json; v=1";

/// Media type of a surface tree encoded as deterministic CBOR.
pub const AST_MEDIA_TYPE_CBOR: &str = "application/vnd.nurt.ast+cbor; v=1";

/// Prefix of an `ast_hash` value.
pub const AST_HASH_PREFIX: &str = "sha256:";

/// Free-form metadata of a tree document (session, source text, comments).
///
/// Metadata is informative only and is excluded from `ast_hash`.
pub type Meta = serde_json::Map<String, serde_json::Value>;

/// Root of a surface tree document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AstDocument {
    /// Format version, `MAJOR.MINOR`; see [`AST_VERSION`].
    pub nurt: String,
    /// The program.
    pub program: Program,
    /// Free-form metadata; excluded from `ast_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl AstDocument {
    /// Creates a document of the current format version without metadata.
    #[must_use]
    pub fn new(program: Program) -> Self {
        Self {
            nurt: AST_VERSION.to_owned(),
            program,
            meta: None,
        }
    }
}

/// A program: statements executed in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Program {
    /// Statements in source order; at least one.
    pub statements: Vec<Statement>,
}

/// A statement of a program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Statement {
    /// `let name = pipeline`.
    Let(Let),
    /// A pipeline.
    Pipeline(Pipeline),
}

/// Named result: `let recent = find … |> filter …`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Let {
    /// Name of the result, `[a-z][a-z0-9_]*`; referenced as `@name`.
    pub name: String,
    /// Pipeline whose result is bound to the name.
    #[serde(
        serialize_with = "tagged::serialize_pipeline",
        deserialize_with = "tagged::deserialize_pipeline"
    )]
    #[schemars(with = "tagged::PipelineNode")]
    pub value: Pipeline,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// A pipeline: a head followed by stages joined with `|>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pipeline {
    /// First element: a stage or a reference to a named result.
    pub head: Head,
    /// Stages after the head, in order; possibly empty.
    #[serde(
        serialize_with = "tagged::serialize_stages",
        deserialize_with = "tagged::deserialize_stages"
    )]
    #[schemars(with = "Vec<tagged::StageNode>")]
    pub stages: Vec<Stage>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Head of a pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Head {
    /// A command.
    Stage(Stage),
    /// A named result, `@recent`.
    Ref(Ref),
}

/// A command invocation: `filter size > 1mb`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stage {
    /// Command name as written after correction: identifiers joined by `.`
    /// (`find`, `git.status`).
    pub name: String,
    /// Arguments in source order.
    pub args: Vec<Arg>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// An argument of a stage: positional (`20`) or named (`glob: "**/*.go"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Arg {
    /// Parameter name of a named argument; absent for a positional argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Argument value.
    pub value: Expr,
    /// Source range; covers the name and the value of a named argument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Reference to a named result: `@recent`, `@recent.path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Ref {
    /// Name of the result, without `@`.
    pub name: String,
    /// Field path after the name; omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// An expression node.
///
/// The parser produces `text` for every quoted string, including paths,
/// globs, instants and keys, and `ident` for every bare identifier; the
/// binder types them by their place in the command signature. The `key` node
/// is a structured alternative to a key written as text, accepted from
/// producers that build trees directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Expr {
    /// Reference to a named result.
    Ref(Ref),
    /// Text, `"abc"` or `r"abc"`; the value after escape processing.
    Text {
        /// Text value.
        v: String,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Bare identifier: an enumeration value or a field name (`desc`, `size`).
    Ident {
        /// Identifier, `[a-z][a-z0-9_]*`.
        v: String,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Regular expression, `/oldName\(/i`.
    Regex {
        /// Pattern; `\/` of the written form is stored as `/`.
        v: String,
        /// Flags from `imsx` in this order, possibly empty.
        flags: String,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Integer, `40`.
    Int {
        /// Value.
        v: i64,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Number with a fraction, `3.5`.
    Num {
        /// Value.
        v: f64,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Size, `10mb`, in bytes.
    Size {
        /// Bytes.
        v: u64,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Duration, `30s`, in milliseconds.
    Dur {
        /// Milliseconds.
        v: u64,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// `true` or `false`.
    Bool {
        /// Value.
        v: bool,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// `null`.
    Null {
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Structured key in a document, an alternative to a key written as text.
    Key {
        /// Key segments; at least one.
        segs: Vec<KeySeg>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// List, `["a", "b"]`.
    List {
        /// Elements in order.
        items: Vec<Expr>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Map, `{replace: "a", with: "b"}`.
    Map {
        /// Entries in source order.
        entries: Vec<MapEntry>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Spread of a list into the enclosing list, `...@recent.path`.
    Spread {
        /// Spread expression.
        x: Box<Expr>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Field access, `file.size`.
    Field {
        /// Field name.
        name: String,
        /// Expression whose field is read; absent for a field of the current
        /// record.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<Box<Expr>>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Function call, `ago(1h)`.
    Call {
        /// Function name.
        #[serde(rename = "fn")]
        function: String,
        /// Arguments in order.
        args: Vec<Expr>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Unary operation, `not hidden`.
    Un {
        /// Operator.
        op: UnOp,
        /// Operand.
        x: Box<Expr>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Binary operation, `size > 1mb`.
    Bin {
        /// Operator.
        op: BinOp,
        /// Left operand.
        l: Box<Expr>,
        /// Right operand.
        r: Box<Expr>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Inclusive integer range, `10..12`, `40..`, `..9`.
    Range {
        /// Lower bound; absent when open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lo: Option<i64>,
        /// Upper bound; absent when open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hi: Option<i64>,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
    /// Sub-pipeline, `(run […] |> assert exit_code: 0)`.
    Sub {
        /// The pipeline.
        #[serde(
            serialize_with = "tagged::serialize_pipeline",
            deserialize_with = "tagged::deserialize_pipeline"
        )]
        #[schemars(with = "tagged::PipelineNode")]
        p: Pipeline,
        /// Source range.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        span: Option<Span>,
    },
}

/// Entry of a map expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapEntry {
    /// Key: an identifier or text.
    pub k: String,
    /// Value.
    pub v: Expr,
}

/// Segment of a structured key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
#[non_exhaustive]
pub enum KeySeg {
    /// Member name, `scripts`.
    Name(String),
    /// List index, `0`; a negative index counts from the end.
    Index(i64),
    /// Wildcard segment.
    Wild {
        /// The wildcard.
        wild: Wildcard,
    },
    /// Filter over list elements, `[?name == "x"]`.
    Filter {
        /// Condition evaluated on each element.
        filter: Box<Expr>,
    },
}

/// Wildcard of a key segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub enum Wildcard {
    /// `*`: every member or element of one level.
    #[serde(rename = "*")]
    Level,
    /// `**`: every member or element at any depth.
    #[serde(rename = "**")]
    Deep,
    /// `+`: a new element appended at the end of a list.
    #[serde(rename = "+")]
    Append,
}

/// Binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub enum BinOp {
    /// Equality, `==`.
    #[serde(rename = "==")]
    Eq,
    /// Inequality, `!=`.
    #[serde(rename = "!=")]
    Ne,
    /// Less than, `<`.
    #[serde(rename = "<")]
    Lt,
    /// Less than or equal, `<=`.
    #[serde(rename = "<=")]
    Le,
    /// Greater than, `>`.
    #[serde(rename = ">")]
    Gt,
    /// Greater than or equal, `>=`.
    #[serde(rename = ">=")]
    Ge,
    /// Regular expression match, `~`.
    #[serde(rename = "~")]
    Match,
    /// Regular expression non-match, `!~`.
    #[serde(rename = "!~")]
    NotMatch,
    /// Membership in a list or range, `in`.
    #[serde(rename = "in")]
    In,
    /// Logical conjunction, `and`.
    #[serde(rename = "and")]
    And,
    /// Logical disjunction, `or`.
    #[serde(rename = "or")]
    Or,
    /// Addition or text concatenation, `+`.
    #[serde(rename = "+")]
    Add,
    /// Subtraction, `-`.
    #[serde(rename = "-")]
    Sub,
    /// Multiplication, `*`.
    #[serde(rename = "*")]
    Mul,
    /// Division, `/`.
    #[serde(rename = "/")]
    Div,
}

impl BinOp {
    /// Returns the operator as written in program text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Match => "~",
            Self::NotMatch => "!~",
            Self::In => "in",
            Self::And => "and",
            Self::Or => "or",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
        }
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UnOp {
    /// Logical negation, `not`.
    Not,
    /// Arithmetic negation, `-`.
    Neg,
}

impl UnOp {
    /// Returns the operator as written in program text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Not => "not",
            Self::Neg => "-",
        }
    }
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Source range of a node, written as `[line, column, end_line, end_column]`.
///
/// Lines and columns start at 1; columns count Unicode scalar values. The end
/// position is exclusive: it is the position directly after the last
/// character of the node. Spans are informative and excluded from `ast_hash`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::Span;
///
/// let span: Span = serde_json::from_str("[1, 1, 1, 5]")?;
/// assert_eq!(span.end_column, 5);
/// assert!(serde_json::from_str::<Span>("[2, 1, 1, 9]").is_err());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "[u32; 4]", into = "[u32; 4]")]
pub struct Span {
    /// Line of the first character.
    pub line: u32,
    /// Column of the first character.
    pub column: u32,
    /// Line of the end position.
    pub end_line: u32,
    /// Column of the end position.
    pub end_column: u32,
}

/// Error for a span whose positions start at 0 or whose end precedes its
/// start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanError(pub [u32; 4]);

impl fmt::Display for SpanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid span {:?}: positions start at 1 and the end must not precede the start",
            self.0
        )
    }
}

impl std::error::Error for SpanError {}

impl TryFrom<[u32; 4]> for Span {
    type Error = SpanError;

    fn try_from(raw: [u32; 4]) -> Result<Self, Self::Error> {
        let [line, column, end_line, end_column] = raw;
        let positive = raw.iter().all(|&position| position >= 1);
        let ordered = end_line > line || (end_line == line && end_column >= column);
        if positive && ordered {
            Ok(Self {
                line,
                column,
                end_line,
                end_column,
            })
        } else {
            Err(SpanError(raw))
        }
    }
}

impl From<Span> for [u32; 4] {
    fn from(span: Span) -> Self {
        [span.line, span.column, span.end_line, span.end_column]
    }
}

impl JsonSchema for Span {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Span")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "[line, column, end_line, end_column]; 1-based, end exclusive",
            "type": "array",
            "items": { "type": "integer", "minimum": 1 },
            "minItems": 4,
            "maxItems": 4
        })
    }
}

/// Returns the hash form of a tree document: the document without the member
/// `meta` of the root object and without the member `span` of every object.
///
/// Map entries are objects with the members `k` and `v`, so removing members
/// by name never touches user data. `ast_hash` is computed from the hash
/// form: `"sha256:"` ([`AST_HASH_PREFIX`]) followed by the lowercase hex
/// SHA-256 digest of the RFC 8785 (JCS) canonical JSON of the form, encoded
/// in UTF-8, taken from the JSON produced by serializing the typed tree. The
/// report and the journal use `ast_hash` of the bound tree.
///
/// # Examples
///
/// ```
/// use nurt_contracts::ast::strip_for_hash;
/// use serde_json::json;
///
/// let tree = json!({
///     "nurt": "1.0",
///     "meta": {"session": "s-1"},
///     "program": {"statements": [{
///         "t": "pipeline",
///         "head": {"t": "stage", "name": "count", "args": [], "span": [1, 1, 1, 6]},
///         "stages": [],
///         "span": [1, 1, 1, 6]
///     }]}
/// });
/// let expected = json!({
///     "nurt": "1.0",
///     "program": {"statements": [{
///         "t": "pipeline",
///         "head": {"t": "stage", "name": "count", "args": []},
///         "stages": []
///     }]}
/// });
/// assert_eq!(strip_for_hash(&tree), expected);
/// ```
#[must_use]
pub fn strip_for_hash(document: &serde_json::Value) -> serde_json::Value {
    match document {
        serde_json::Value::Object(members) => serde_json::Value::Object(
            members
                .iter()
                .filter(|(name, _)| name.as_str() != META_MEMBER && name.as_str() != SPAN_MEMBER)
                .map(|(name, value)| (name.clone(), strip_spans(value)))
                .collect(),
        ),
        other => strip_spans(other),
    }
}

/// Root member excluded from the hash form.
const META_MEMBER: &str = "meta";

/// Member excluded from every object of the hash form.
const SPAN_MEMBER: &str = "span";

fn strip_spans(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(members) => serde_json::Value::Object(
            members
                .iter()
                .filter(|(name, _)| name.as_str() != SPAN_MEMBER)
                .map(|(name, value)| (name.clone(), strip_spans(value)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(strip_spans).collect())
        }
        other => other.clone(),
    }
}

/// Serialization of pipelines and stages that appear outside a tagged
/// union but still carry their member `"t"`.
mod tagged {
    use schemars::JsonSchema;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{Pipeline, Stage};

    #[derive(Serialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    enum PipelineRef<'a> {
        Pipeline(&'a Pipeline),
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub(super) enum PipelineNode {
        Pipeline(Pipeline),
    }

    #[derive(Serialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    enum StageRef<'a> {
        Stage(&'a Stage),
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub(super) enum StageNode {
        Stage(Stage),
    }

    pub(super) fn serialize_pipeline<S: Serializer>(
        pipeline: &Pipeline,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        PipelineRef::Pipeline(pipeline).serialize(serializer)
    }

    pub(super) fn deserialize_pipeline<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Pipeline, D::Error> {
        let PipelineNode::Pipeline(pipeline) = PipelineNode::deserialize(deserializer)?;
        Ok(pipeline)
    }

    pub(super) fn serialize_stages<S: Serializer>(
        stages: &[Stage],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(stages.iter().map(StageRef::Stage))
    }

    pub(super) fn deserialize_stages<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Stage>, D::Error> {
        let nodes = Vec::<StageNode>::deserialize(deserializer)?;
        Ok(nodes
            .into_iter()
            .map(|StageNode::Stage(stage)| stage)
            .collect())
    }
}
