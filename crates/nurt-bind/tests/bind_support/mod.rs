// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-bind/tests/bind_support/mod.rs
// Responsibility:   Shared helpers of the binder tests: the reference catalog and builders of surface trees.
// Dependencies:     nurt_contracts, nurt_correct, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Shared helpers of the binder tests: the reference catalog and builders of
//! surface trees.

// Each test crate uses a different subset of the helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use nurt_bind::{BindContext, BindError, Catalog, bind};
use nurt_contracts::ast::{
    Arg, AstDocument, Expr, Head, Let, Pipeline, Program, Ref, Stage, Statement, UnOp,
};
use nurt_contracts::bound::{BoundArg, BoundDocument, BoundHead, BoundStage, BoundStatement};
use nurt_contracts::spec::Os;

/// Root of the workspace.
pub(crate) fn workspace() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("..")
    .join("..")
}

/// The reference catalog of the workspace, read once.
pub(crate) fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::load_dir(&workspace().join("catalog"))
            .unwrap_or_else(|error| panic!("reference catalog: {error}"))
    })
}

/// Revision of the embedded correction tables.
pub(crate) fn corr_rev() -> String {
    nurt_correct::tables::corr_rev()
        .unwrap_or_else(|error| panic!("embedded correction tables: {error:?}"))
}

/// Context of a Linux host without `base`.
pub(crate) fn linux() -> BindContext {
    BindContext::new(Os::Linux)
}

/// Binds a document with the reference catalog.
pub(crate) fn bind_with(
    document: &AstDocument,
    context: &BindContext,
) -> Result<BoundDocument, BindError> {
    bind(document, catalog(), context)
}

/// Binds a document on a Linux host and expects success.
pub(crate) fn bound(document: &AstDocument) -> BoundDocument {
    bind_with(document, &linux()).unwrap_or_else(|error| panic!("binding failed: {error}"))
}

/// Binds a document on a Linux host and expects an error.
pub(crate) fn failed(document: &AstDocument) -> BindError {
    match bind_with(document, &linux()) {
        Ok(_) => panic!("binding succeeded, an error is expected"),
        Err(error) => error,
    }
}

/// `ast_hash` of a bound tree.
pub(crate) fn hash(document: &BoundDocument) -> String {
    nurt_syntax::hash::ast_hash(document).unwrap_or_else(|error| panic!("hash: {error:?}"))
}

pub(crate) fn text(v: &str) -> Expr {
    Expr::Text {
        v: v.to_owned(),
        span: None,
    }
}

pub(crate) fn ident(v: &str) -> Expr {
    Expr::Ident {
        v: v.to_owned(),
        span: None,
    }
}

pub(crate) fn int(v: i64) -> Expr {
    Expr::Int { v, span: None }
}

pub(crate) fn dur(v: u64) -> Expr {
    Expr::Dur { v, span: None }
}

pub(crate) fn negative(x: Expr) -> Expr {
    Expr::Un {
        op: UnOp::Neg,
        x: Box::new(x),
        span: None,
    }
}

pub(crate) fn regex(v: &str) -> Expr {
    Expr::Regex {
        v: v.to_owned(),
        flags: String::new(),
        span: None,
    }
}

pub(crate) fn list(items: Vec<Expr>) -> Expr {
    Expr::List { items, span: None }
}

pub(crate) fn bin(op: nurt_contracts::ast::BinOp, l: Expr, r: Expr) -> Expr {
    Expr::Bin {
        op,
        l: Box::new(l),
        r: Box::new(r),
        span: None,
    }
}

pub(crate) fn sub(pipeline: Pipeline) -> Expr {
    Expr::Sub {
        p: pipeline,
        span: None,
    }
}

pub(crate) fn pos(value: Expr) -> Arg {
    Arg {
        name: None,
        value,
        span: None,
    }
}

pub(crate) fn named(name: &str, value: Expr) -> Arg {
    Arg {
        name: Some(name.to_owned()),
        value,
        span: None,
    }
}

pub(crate) fn stage(name: &str, args: Vec<Arg>) -> Stage {
    Stage {
        name: name.to_owned(),
        args,
        span: None,
    }
}

pub(crate) fn pipeline(head: Stage, stages: Vec<Stage>) -> Pipeline {
    Pipeline {
        head: Head::Stage(head),
        stages,
        span: None,
    }
}

pub(crate) fn ref_pipeline(name: &str, stages: Vec<Stage>) -> Pipeline {
    Pipeline {
        head: Head::Ref(Ref {
            name: name.to_owned(),
            fields: Vec::new(),
            span: None,
        }),
        stages,
        span: None,
    }
}

pub(crate) fn let_statement(name: &str, value: Pipeline) -> Statement {
    Statement::Let(Let {
        name: name.to_owned(),
        value,
        span: None,
    })
}

pub(crate) fn program(statements: Vec<Statement>) -> AstDocument {
    AstDocument::new(Program { statements })
}

/// A program of one pipeline.
pub(crate) fn single(head: Stage, stages: Vec<Stage>) -> AstDocument {
    program(vec![Statement::Pipeline(pipeline(head, stages))])
}

/// Stages of a bound pipeline statement, the head first.
pub(crate) fn stages_of(document: &BoundDocument, statement: usize) -> Vec<&BoundStage> {
    let pipeline = match &document.program.statements[statement] {
        BoundStatement::Pipeline(pipeline) => pipeline,
        BoundStatement::Let(binding) => &binding.value,
        _ => panic!("unknown statement kind"),
    };
    let mut stages = Vec::new();
    if let BoundHead::Stage(head) = &pipeline.head {
        stages.push(head);
    }
    stages.extend(pipeline.stages.iter());
    stages
}

/// Argument of a bound stage by name.
pub(crate) fn arg<'d>(stage: &'d BoundStage, name: &str) -> &'d BoundArg {
    stage
        .args
        .iter()
        .find(|arg| arg.name == name)
        .unwrap_or_else(|| panic!("`{}` has no argument `{name}`", stage.name))
}

/// Expected names of an error.
pub(crate) fn expected_names(error: &BindError) -> Vec<String> {
    error
        .error()
        .expected
        .as_ref()
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
