// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syntax::schema
// Responsibility:   JSON Schemas of the syntax tree documents.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! JSON Schemas of the syntax tree documents.
//!
//! The schemas are the normative definition of the JSON form of the surface
//! tree `nurt-ast/1` and the bound tree `nurt-bound/1` (JSON Schema draft
//! 2020-12). `nurt schema ast` and `nurt schema bound` print them.

use nurt_contracts::ast::AST_FORMAT;
use nurt_contracts::bound::BOUND_FORMAT;

/// JSON Schema of the surface tree `nurt-ast/1`.
pub const AST_SCHEMA: &str = include_str!("../../../spec/schemas/nurt-ast-1.schema.json");

/// JSON Schema of the bound tree `nurt-bound/1`.
pub const BOUND_SCHEMA: &str = include_str!("../../../spec/schemas/nurt-bound-1.schema.json");

/// Short name of the surface tree schema.
pub const AST_SCHEMA_NAME: &str = "ast";

/// Short name of the bound tree schema.
pub const BOUND_SCHEMA_NAME: &str = "bound";

/// Returns the schema with the given short name (`ast`, `bound`) or format
/// name (`nurt-ast/1`, `nurt-bound/1`).
///
/// # Examples
///
/// ```
/// use nurt_syntax::schema::{AST_SCHEMA, by_name};
///
/// assert_eq!(by_name("ast"), Some(AST_SCHEMA));
/// assert_eq!(by_name("nurt-ast/1"), Some(AST_SCHEMA));
/// assert_eq!(by_name("report"), None);
/// ```
#[must_use]
pub fn by_name(name: &str) -> Option<&'static str> {
    match name {
        AST_SCHEMA_NAME | AST_FORMAT => Some(AST_SCHEMA),
        BOUND_SCHEMA_NAME | BOUND_FORMAT => Some(BOUND_SCHEMA),
        _ => None,
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
