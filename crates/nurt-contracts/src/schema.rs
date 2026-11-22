// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Published JSON schemas of the Danaco Nurt formats.
//!
//! The schemas are JSON Schema 2020-12 documents kept in `spec/schemas/` and
//! embedded in the library, so that `nurt schema <name>` and `nurt_schema`
//! return exactly the files that the conformance suite validates against.

/// Schema of the surface syntax tree `nurt-ast/1`.
pub const AST: &str = include_str!("../../../spec/schemas/nurt-ast-1.schema.json");

/// Schema of the bound syntax tree `nurt-bound/1`.
pub const BOUND: &str = include_str!("../../../spec/schemas/nurt-bound-1.schema.json");

/// Schema of the effect report `nurt-report/1`.
pub const REPORT: &str = include_str!("../../../spec/schemas/nurt-report-1.schema.json");

/// Schema of the request envelope `nurt-request/1`.
pub const REQUEST: &str = include_str!("../../../spec/schemas/nurt-request-1.schema.json");

/// Schema of the configuration file `nurt-config/1`.
pub const CONFIG: &str = include_str!("../../../spec/schemas/nurt-config-1.schema.json");

/// Schema of the command catalog files `nurt-catalog/1`.
pub const CATALOG: &str = include_str!("../../../spec/schemas/nurt-catalog-1.schema.json");

/// Schema of values and records `nurt-value/1`.
pub const VALUE: &str = include_str!("../../../spec/schemas/nurt-value-1.schema.json");

/// Name and content of every published schema, in a stable order.
pub const ALL: &[(&str, &str)] = &[
    ("ast", AST),
    ("bound", BOUND),
    ("report", REPORT),
    ("request", REQUEST),
    ("config", CONFIG),
    ("catalog", CATALOG),
    ("value", VALUE),
];

/// Returns the schema with the given name.
///
/// Known names: `ast`, `bound`, `report`, `request`, `config`, `catalog` and `value`.
///
/// # Examples
///
/// ```
/// let schema = nurt_contracts::schema::by_name("ast");
/// assert!(schema.is_some());
/// assert!(nurt_contracts::schema::by_name("unknown").is_none());
/// ```
#[must_use]
pub fn by_name(name: &str) -> Option<&'static str> {
    ALL.iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, content)| *content)
}
