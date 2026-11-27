// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Control constructs of the surface tree: conditions, loops, `steps`,
//! functions and script parameters.
//!
//! Every construct is a statement node; a block is a list of statements.
//! No construct holds program text: conditions and values are expressions of
//! the tree, so a construct is checked before any effect and executed as one
//! journal entry.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Arg, Expr, Span, Statement};

/// Names of the node kinds this module adds to the surface tree.
pub const FLOW_KINDS: [&str; 9] = [
    "when", "each", "repeat", "while", "break", "continue", "steps", "define", "params",
];

/// A branch of a `when`: a condition and the block that runs when it is the
/// first condition that holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Branch {
    /// Condition of the branch; a `bool` expression.
    pub cond: Expr,
    /// Statements of the branch, at least one.
    pub body: Vec<Statement>,
    /// Source range, from the keyword to the closing brace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Conditional: `when c { … } otherwise when d { … } otherwise { … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct When {
    /// The `when` branch followed by every `otherwise when` branch.
    pub branches: Vec<Branch>,
    /// The `otherwise` block; absent when the construct has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<Vec<Statement>>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Loop over the records of an expression: `each (find …) as f { … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct Each {
    /// Expression whose records the loop visits, in input order.
    pub source: Expr,
    /// Name bound to the record of the iteration; referenced as `@name`.
    pub name: String,
    /// Named options: `parallel`, `atomic`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Arg>,
    /// Statements of the body.
    pub body: Vec<Statement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Loop that tests its condition after each run of the body:
/// `repeat until: c every: 5s limit: 2m { … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct Repeat {
    /// Named options: `until` (required), `every`, `limit`, `atomic`.
    pub options: Vec<Arg>,
    /// Statements of the body.
    pub body: Vec<Statement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Loop that tests its condition before each run of the body:
/// `while c limit: 20 { … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct While {
    /// Condition; a `bool` expression.
    pub cond: Expr,
    /// Named options: `limit`, `atomic`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Arg>,
    /// Statements of the body.
    pub body: Vec<Statement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// `break` or `continue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct Jump {
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Several changes in one call: `steps { … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct Steps {
    /// Steps in order, at least one.
    pub body: Vec<Statement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Declaration of a typed parameter of a function or of a script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// Parameter name, `[a-z][a-z0-9_]*`.
    pub name: String,
    /// Type name, for example `path`, `int`, `bool`, `text`.
    #[serde(rename = "type")]
    pub ty: String,
    /// Default value; absent for a required parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Expr>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Definition of a function: `define name(a: path, b: int = 7) { … }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct Define {
    /// Function name, `[a-z][a-z0-9_]*`.
    pub name: String,
    /// Declared parameters in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<Param>,
    /// Statements of the body, at least one.
    pub body: Vec<Statement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Parameters of a `.nurt` file: `params { dir: path  days: int = 30 }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct Params {
    /// Declared parameters in order.
    pub params: Vec<Param>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}
