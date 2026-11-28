// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bind
// Responsibility:   Binding of Danaco Nurt syntax trees to the command catalog.
// Dependencies:     catalog, context, error, facade, nurt_contracts, params
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Binding of Danaco Nurt syntax trees to the command catalog.
//!
//! Binding turns a `nurt-ast/1` tree into a `nurt-bound/1` tree: positional
//! parameters are named, defaults are made explicit, literals are converted to
//! the type of their position, source forms are expanded, the single changing
//! stage rule is enforced and the effect of every stage is classified.
//!
//! Binding is stage `1` of an execution and has no effect on the world. Its
//! errors are language errors of that stage ([`BindError`]), with the
//! admissible names in `expected`, ready programs in `fix` and, for a
//! program with more than one effect, the separate calls in `next_steps`.
//!
//! [`bind_with_correction`] reads a program text with notation correction
//! and binds it, correcting names, values and literals against the catalog.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod binder;
mod cast;
mod catalog;
mod context;
mod correct;
mod error;
mod facade;
mod fieldcheck;
mod fields;
mod flow;
mod literal;
mod params;
mod path;
mod phase;
mod signature;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;

pub use catalog::{Catalog, CatalogError, INDEX_FILE, REVISION_PREFIX};
pub use context::{BindContext, UNCORRECTED_REV, host_os};
pub use error::BindError;
pub use facade::{Outcome, bind_with_correction, offer_candidates};
pub use params::{ParamDescription, bind_params, describe_params};

/// Binds a surface tree to the catalog.
///
/// Every parameter appears in signature order; a value that was not written
/// comes from the catalog with `default: true`. Literals are cast to the
/// type of their place, source forms are expanded and every stage receives
/// its effect, its phase and the `waits` mark. Names are not corrected.
///
/// # Errors
///
/// Returns a [`BindError`] of stage `1`, for example `E_UNKNOWN_COMMAND`,
/// `E_UNKNOWN_FIELD`, `E_TYPE`, `E_PATH_RELATIVE` or `E_ONE_EFFECT`.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use nurt_bind::{BindContext, Catalog, bind};
/// use nurt_contracts::ast::{AstDocument, Head, Pipeline, Program, Stage, Statement};
/// use nurt_contracts::spec::Os;
///
/// let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
/// let catalog = Catalog::load_dir(&dir)?;
/// let stage = Stage { name: "ports".to_owned(), args: Vec::new(), span: None };
/// let pipeline = Pipeline { head: Head::Stage(stage), stages: Vec::new(), span: None };
/// let document = AstDocument::new(Program { statements: vec![Statement::Pipeline(pipeline)] });
/// let bound = bind(&document, &catalog, &BindContext::new(Os::Linux))?;
/// let head = &serde_json::to_value(&bound)?["program"]["statements"][0]["head"];
/// assert_eq!(head["phase"], "source");
/// assert_eq!(head["args"][0]["default"], true);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn bind(
    document: &AstDocument,
    catalog: &Catalog,
    context: &BindContext,
) -> Result<BoundDocument, BindError> {
    binder::bind_document(document, catalog, context)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
