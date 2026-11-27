// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-contracts::bound::flow
// Responsibility:   Bound control constructs: conditions, loops, `steps`, functions and.
// Dependencies:     schemars, serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Bound control constructs: conditions, loops, `steps`, functions and
//! script parameters.
//!
//! The binder checks the constructs against the catalog and the declared
//! names before any effect: conditions are `bool`, loop options have their
//! types and names refer to parameters, `let` results or loop variables.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BoundArg, BoundExpr, BoundStatement, Span};

/// Names of the node kinds this module adds to the bound tree.
pub const FLOW_BOUND_KINDS: [&str; 10] = [
    "when", "each", "repeat", "while", "break", "continue", "steps", "define", "params", "call",
];

/// Largest number of iterations a loop without an explicit limit runs, the
/// default of the configuration key `loop.max_iterations`.
pub const DEFAULT_MAX_ITERATIONS: u64 = 10_000;

/// Longest time in milliseconds a loop without an explicit limit runs, the
/// default of the configuration key `loop.max_time` (10 minutes).
pub const DEFAULT_MAX_TIME_MS: u64 = 600_000;

/// Deepest nesting of function calls, the default of the configuration key
/// `function.max_depth`.
pub const DEFAULT_MAX_DEPTH: u32 = 64;

/// Limit of a loop: a number of iterations or a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundLimit {
    /// At most `n` iterations.
    Iterations {
        /// Number of iterations.
        n: u64,
    },
    /// At most `ms` milliseconds from the start of the loop.
    Time {
        /// Milliseconds.
        ms: u64,
    },
}

/// A branch of a bound `when`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundBranch {
    /// Condition, of type `bool`.
    pub cond: BoundExpr,
    /// Statements of the branch.
    pub body: Vec<BoundStatement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound conditional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundWhen {
    /// Branches in order; the first whose condition holds runs.
    pub branches: Vec<BoundBranch>,
    /// Statements that run when no condition holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otherwise: Option<Vec<BoundStatement>>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound loop over records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundEach {
    /// Records to visit.
    pub source: BoundExpr,
    /// Name bound to the record of the iteration.
    pub name: String,
    /// Number of iterations that may run concurrently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<u32>,
    /// Whether a failure rolls back the iterations already completed.
    #[serde(default = "default_atomic", skip_serializing_if = "is_true")]
    pub atomic: bool,
    /// Statements of the body.
    pub body: Vec<BoundStatement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound loop that tests its condition after the body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundRepeat {
    /// Condition that ends the loop when it holds.
    pub until: BoundExpr,
    /// Wait between iterations, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_ms: Option<u64>,
    /// Limit; the configuration limits apply when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<BoundLimit>,
    /// Whether a failure rolls back the iterations already completed.
    #[serde(default = "default_atomic", skip_serializing_if = "is_true")]
    pub atomic: bool,
    /// Statements of the body.
    pub body: Vec<BoundStatement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound loop that tests its condition before the body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundWhile {
    /// Condition that keeps the loop running while it holds.
    pub cond: BoundExpr,
    /// Limit; the configuration limits apply when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<BoundLimit>,
    /// Whether a failure rolls back the iterations already completed.
    #[serde(default = "default_atomic", skip_serializing_if = "is_true")]
    pub atomic: bool,
    /// Statements of the body.
    pub body: Vec<BoundStatement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// `break` or `continue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundJump {
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound `steps` block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundSteps {
    /// Steps in order.
    pub body: Vec<BoundStatement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Declared parameter of a function or of a script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundParam {
    /// Name of the parameter.
    pub name: String,
    /// Canonical type, as written in the catalog (`path`, `list<text>`).
    #[serde(rename = "type")]
    pub ty: String,
    /// Default value; absent for a required parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<BoundExpr>,
}

/// Bound definition of a function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundDefine {
    /// Function name.
    pub name: String,
    /// Declared parameters in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<BoundParam>,
    /// Statements of the body; the value of the last one is the result.
    pub body: Vec<BoundStatement>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Bound parameters of a script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundParams {
    /// Declared parameters in order.
    pub params: Vec<BoundParam>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

/// Call of a defined function, the head of a pipeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
// The enclosing tagged union adds the member `t`, so the struct schema stays open.
#[schemars(extend("additionalProperties" = true))]
pub struct BoundCall {
    /// Name of the function.
    pub name: String,
    /// Arguments by parameter name, defaults included.
    pub args: Vec<BoundArg>,
    /// Source range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
}

fn default_atomic() -> bool {
    true
}

// Serde passes the field by reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(value: &bool) -> bool {
    *value
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
