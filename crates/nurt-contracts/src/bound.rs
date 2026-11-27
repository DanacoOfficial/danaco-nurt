// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Bound syntax tree `nurt-bound/1`.
//!
//! Binding a surface tree ([`crate::ast::AstDocument`]) with the command
//! catalog produces a bound tree. The bound tree is a separate document with
//! its own schema, `nurt-bound-1.schema.json`. Compared with the surface tree
//! it records:
//!
//! * the catalog revision (`catalog_rev`) and the correction tables revision
//!   (`corr_rev`) used to produce it;
//! * the canonical command name of every stage, its effect kind (`effect`),
//!   its execution phase (`phase`) and whether it blocks the call by waiting
//!   (`waits`);
//! * every parameter of the command signature by name, in signature order,
//!   with every default value written explicitly and marked `default: true`;
//! * literals cast to the type of their place (`path`, `glob`, `instant`,
//!   `key`, `enum`, `range`) and bare identifiers resolved to enumeration
//!   values or record fields;
//! * short source forms expanded to their single canonical pipeline.
//!
//! The journal and the report store the bound tree, so a later change of a
//! default value never changes the meaning of a recorded program. Value nodes
//! of the bound tree carry no source range.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::bound::{
//!     BOUND_VERSION, BoundArg, BoundDocument, BoundExpr, BoundHead, BoundPipeline,
//!     BoundProgram, BoundStage, BoundStatement, Phase,
//! };
//! use nurt_contracts::kinds::EffectKind;
//!
//! let stage = BoundStage {
//!     name: "ports".to_owned(),
//!     effect: EffectKind::Read,
//!     phase: Phase::Source,
//!     waits: false,
//!     args: vec![
//!         BoundArg::given("state", BoundExpr::Enum { v: "listen".to_owned() }),
//!         BoundArg::from_default("protocol", BoundExpr::Enum { v: "any".to_owned() }),
//!     ],
//!     span: None,
//! };
//! let document = BoundDocument::new(
//!     "cat-1",
//!     "corr-1",
//!     BoundProgram {
//!         statements: vec![BoundStatement::Pipeline(BoundPipeline {
//!             head: BoundHead::Stage(stage),
//!             stages: Vec::new(),
//!             span: None,
//!         })],
//!     },
//! );
//! let json = serde_json::to_value(&document)?;
//! assert_eq!(json["bound"], BOUND_VERSION);
//! assert_eq!(json["program"]["statements"][0]["head"]["effect"], "read");
//! assert_eq!(json["program"]["statements"][0]["head"]["args"][1]["default"], true);
//! # Ok::<(), serde_json::Error>(())
//! ```

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ast::{AST_VERSION, BinOp, Meta, Span, UnOp};
use crate::kinds::EffectKind;

/// Name of the bound tree format.
pub const BOUND_FORMAT: &str = "nurt-bound/1";

/// Bound tree version written into the member `"bound"` by this version of
/// the contract.
pub const BOUND_VERSION: &str = "1.0";

/// Media type of a bound tree encoded as JSON.
pub const BOUND_MEDIA_TYPE_JSON: &str = "application/vnd.nurt.bound+json; v=1";

/// Media type of a bound tree encoded as deterministic CBOR.
pub const BOUND_MEDIA_TYPE_CBOR: &str = "application/vnd.nurt.bound+cbor; v=1";

/// Node kinds of the bound tree: the admissible values of the member `"t"`.
///
/// A reader reports node kinds outside this list with `E_AST_VERSION`.
pub const BOUND_KINDS: [&str; 26] = [
    "let", "pipeline", "stage", "ref", "null", "bool", "int", "num", "text", "path", "glob", "key",
    "enum", "instant", "size", "dur", "regex", "list", "map", "range", "sub", "field", "call",
    "un", "bin", "spread",
];

/// Root of a bound tree document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundDocument {
    /// Version of the surface tree format the program was bound from,
    /// `MAJOR.MINOR`; see [`AST_VERSION`].
    pub nurt: String,
    /// Bound tree version, `MAJOR.MINOR`; see [`BOUND_VERSION`].
    pub bound: String,
    /// Revision of the command catalog used for binding.
    pub catalog_rev: String,
    /// Revision of the correction tables applied before binding.
    pub corr_rev: String,
    /// The bound program.
    pub program: BoundProgram,
    /// Free-form metadata; excluded from `ast_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

impl BoundDocument {
    /// Creates a document of the current format versions without metadata.
    #[must_use]
    pub fn new(
        catalog_rev: impl Into<String>,
        corr_rev: impl Into<String>,
        program: BoundProgram,
    ) -> Self {
        Self {
            nurt: AST_VERSION.to_owned(),
            bound: BOUND_VERSION.to_owned(),
            catalog_rev: catalog_rev.into(),
            corr_rev: corr_rev.into(),
            program,
            meta: None,
        }
    }
}

/// A bound program: statements executed in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundProgram {
    /// Statements in source order; at least one.
    pub statements: Vec<BoundStatement>,
}

/// A statement of a bound program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundStatement {
    /// `let name = pipeline`.
    Let(BoundLet),
    /// A pipeline.
    Pipeline(BoundPipeline),
}

/// Bound named result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundLet {
    /// Name of the result, `[a-z][a-z0-9_]*`.
    pub name: String,
    /// Pipeline whose result is bound to the name.
    #[serde(
        serialize_with = "tagged::serialize_pipeline",
        deserialize_with = "tagged::deserialize_pipeline"
    )]
    #[schemars(with = "tagged::PipelineNode")]
    pub value: BoundPipeline,
    /// Source range of the surface statement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundPipeline {
    /// First element: a stage or a reference to a named result.
    pub head: BoundHead,
    /// Stages after the head, in order; possibly empty.
    #[serde(
        serialize_with = "tagged::serialize_stages",
        deserialize_with = "tagged::deserialize_stages"
    )]
    #[schemars(with = "Vec<tagged::StageNode>")]
    pub stages: Vec<BoundStage>,
    /// Source range of the surface pipeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Head of a bound pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundHead {
    /// A command.
    Stage(BoundStage),
    /// A named result.
    Ref(BoundRef),
}

/// Names of the call parameters that can be written on a stage; they follow
/// the parameters of the signature in [`BoundStage::args`].
pub const STAGE_CALL_PARAMETERS: [&str; 2] = ["privilege", "as_user"];

/// Bound command invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundStage {
    /// Canonical command name from the catalog, after correction: one
    /// identifier or a group and an identifier joined by `.`.
    pub name: String,
    /// Effect kind of the stage, taken from the catalog.
    pub effect: EffectKind,
    /// Execution phase of the stage.
    pub phase: Phase,
    /// `true` when the stage blocks the call by waiting (for example `sleep`
    /// or `run` with a long `wait`); omitted when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub waits: bool,
    /// Every parameter of the command signature, in signature order. The
    /// pipeline checked by `verify` is the value of its parameter `check`.
    pub args: Vec<BoundArg>,
    /// Source range of the surface stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl BoundStage {
    /// Text of the call parameter `name` (`privilege` or `as_user`) written on
    /// the stage, when it was written.
    #[must_use]
    pub fn call_parameter(&self, name: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|arg| arg.name == name && !arg.default)
            .and_then(|arg| match &arg.value {
                BoundExpr::Text { v } => Some(v.as_str()),
                _ => None,
            })
    }
}

/// Execution phase of a bound stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Phase {
    /// The head stage that produces records.
    Source,
    /// A transformation of records before the effect.
    Transform,
    /// A check before the effect; failure changes nothing.
    Check,
    /// The single change stage of the pipeline.
    Mutate,
    /// The single program-running stage of the pipeline.
    Run,
    /// A check or transformation of the records produced by the effect; a
    /// failed check is reported without rolling the effect back.
    After,
    /// The `verify` stage; failure rolls the effect back.
    Verify,
}

impl Phase {
    /// Returns `true` for the phases of the effect stage.
    #[must_use]
    pub fn is_effect(self) -> bool {
        matches!(self, Self::Mutate | Self::Run)
    }
}

/// A bound argument: one parameter of the signature with its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundArg {
    /// Parameter name.
    pub name: String,
    /// Value cast to the parameter type; `null` when the parameter has no
    /// value.
    pub value: BoundExpr,
    /// `true` when the binder inserted the value from the catalog default;
    /// omitted when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub default: bool,
    /// Source range of the surface argument; absent for defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

impl BoundArg {
    /// Creates an argument whose value was written in the program.
    #[must_use]
    pub fn given(name: impl Into<String>, value: BoundExpr) -> Self {
        Self {
            name: name.into(),
            value,
            default: false,
            span: None,
        }
    }

    /// Creates an argument whose value the binder took from the catalog
    /// default.
    #[must_use]
    pub fn from_default(name: impl Into<String>, value: BoundExpr) -> Self {
        Self {
            name: name.into(),
            value,
            default: true,
            span: None,
        }
    }
}

/// Reference to a named result in a bound tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundRef {
    /// Name of the result, without `@`.
    pub name: String,
    /// Field path after the name; omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

/// A typed value or expression of a bound tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum BoundExpr {
    /// No value.
    Null,
    /// Boolean.
    Bool {
        /// Value.
        v: bool,
    },
    /// Integer.
    Int {
        /// Value.
        v: i64,
    },
    /// Number with a fraction.
    Num {
        /// Value.
        v: f64,
    },
    /// Text.
    Text {
        /// Value.
        v: String,
    },
    /// Absolute path in canonical form.
    Path {
        /// Path.
        v: String,
    },
    /// Glob pattern.
    Glob {
        /// Pattern.
        v: String,
    },
    /// Key in a structured document.
    Key {
        /// Key in canonical text form.
        v: String,
    },
    /// Enumeration value.
    Enum {
        /// Value.
        v: String,
    },
    /// Point in time, RFC 3339.
    Instant {
        /// Instant text.
        v: String,
    },
    /// Size in bytes.
    Size {
        /// Bytes.
        v: u64,
    },
    /// Duration in milliseconds.
    Dur {
        /// Milliseconds.
        v: u64,
    },
    /// Regular expression.
    Regex {
        /// Pattern.
        v: String,
        /// Flags from `imsx` in this order, possibly empty.
        flags: String,
    },
    /// List.
    List {
        /// Elements in order.
        items: Vec<BoundExpr>,
    },
    /// Map.
    Map {
        /// Entries in source order.
        entries: Vec<BoundMapEntry>,
    },
    /// Inclusive integer range; a single integer `n` binds to `n..n`.
    Range {
        /// Lower bound; absent when open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lo: Option<i64>,
        /// Upper bound; absent when open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hi: Option<i64>,
    },
    /// Reference to a named result.
    Ref(BoundRef),
    /// Sub-pipeline value: the `check` of `verify`, `schedule.create action:`.
    Sub {
        /// The pipeline.
        #[serde(
            serialize_with = "tagged::serialize_pipeline",
            deserialize_with = "tagged::deserialize_pipeline"
        )]
        #[schemars(with = "tagged::PipelineNode")]
        p: BoundPipeline,
    },
    /// Field of the current record, or of another expression when `of` is
    /// present.
    Field {
        /// Field name.
        name: String,
        /// Expression whose field is read.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        of: Option<Box<BoundExpr>>,
    },
    /// Function call.
    Call {
        /// Function name.
        #[serde(rename = "fn")]
        function: String,
        /// Arguments in order.
        args: Vec<BoundExpr>,
    },
    /// Unary operation.
    Un {
        /// Operator.
        op: UnOp,
        /// Operand.
        x: Box<BoundExpr>,
    },
    /// Binary operation.
    Bin {
        /// Operator.
        op: BinOp,
        /// Left operand.
        l: Box<BoundExpr>,
        /// Right operand.
        r: Box<BoundExpr>,
    },
    /// Spread of a list into the enclosing list.
    Spread {
        /// Spread expression.
        x: Box<BoundExpr>,
    },
}

/// Entry of a bound map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundMapEntry {
    /// Key.
    pub k: String,
    /// Value.
    pub v: BoundExpr,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Serialization of bound pipelines and stages that appear outside a tagged
/// union but still carry their member `"t"`.
mod tagged {
    use schemars::JsonSchema;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::{BoundPipeline, BoundStage};

    #[derive(Serialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    enum PipelineRef<'a> {
        Pipeline(&'a BoundPipeline),
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub(super) enum PipelineNode {
        Pipeline(BoundPipeline),
    }

    #[derive(Serialize)]
    #[serde(tag = "t", rename_all = "snake_case")]
    enum StageRef<'a> {
        Stage(&'a BoundStage),
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(tag = "t", rename_all = "snake_case")]
    pub(super) enum StageNode {
        Stage(BoundStage),
    }

    pub(super) fn serialize_pipeline<S: Serializer>(
        pipeline: &BoundPipeline,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        PipelineRef::Pipeline(pipeline).serialize(serializer)
    }

    pub(super) fn deserialize_pipeline<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BoundPipeline, D::Error> {
        let PipelineNode::Pipeline(pipeline) = PipelineNode::deserialize(deserializer)?;
        Ok(pipeline)
    }

    pub(super) fn serialize_stages<S: Serializer>(
        stages: &[BoundStage],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(stages.iter().map(StageRef::Stage))
    }

    pub(super) fn deserialize_stages<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<BoundStage>, D::Error> {
        let nodes = Vec::<StageNode>::deserialize(deserializer)?;
        Ok(nodes
            .into_iter()
            .map(|StageNode::Stage(stage)| stage)
            .collect())
    }
}
