// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-plan/tests/constructs.rs
// Responsibility:   Planning of the control constructs.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Planning of the control constructs: the plan keeps the structure, every
//! pipeline inside is checked like a pipeline of the top level, and the
//! effect of a construct is the strongest effect inside.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use common::{document, find, mutate, operation, pipeline};
use nurt_contracts::bound::{
    BoundBranch, BoundCall, BoundEach, BoundExpr, BoundHead, BoundJump, BoundPipeline,
    BoundStatement, BoundSteps, BoundWhen,
};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::{EffectKind, Reversibility};
use nurt_plan::{ConstructShape, PlanStatement};

fn when(
    branches: Vec<(BoundExpr, Vec<BoundStatement>)>,
    otherwise: Option<Vec<BoundStatement>>,
) -> BoundStatement {
    BoundStatement::When(BoundWhen {
        branches: branches
            .into_iter()
            .map(|(cond, body)| BoundBranch {
                cond,
                body,
                span: None,
            })
            .collect(),
        otherwise,
        span: None,
    })
}

fn steps(body: Vec<BoundStatement>) -> BoundStatement {
    BoundStatement::Steps(BoundSteps { body, span: None })
}

fn remove() -> BoundStatement {
    operation(pipeline(find("/a"), vec![mutate("remove", Vec::new())]))
}

fn read() -> BoundStatement {
    operation(pipeline(find("/a"), Vec::new()))
}

#[test]
fn the_plan_keeps_the_branches_and_the_effect_of_the_strongest_branch() {
    let program = document(vec![when(
        vec![(BoundExpr::Bool { v: true }, vec![read()])],
        Some(vec![remove()]),
    )]);
    let plan = nurt_plan::plan(&program, &common::registry()).unwrap();
    let construct = plan.constructs().next().unwrap();
    assert_eq!(construct.name(), "when");
    let ConstructShape::When {
        branches,
        otherwise,
    } = &construct.shape
    else {
        panic!("not a when")
    };
    assert_eq!(branches.len(), 1);
    assert!(otherwise.is_some());
    assert_eq!(construct.effect, EffectKind::Local);
    assert_eq!(plan.effect, EffectKind::Local);
    assert_ne!(plan.reversibility, Reversibility::Read);
}

#[test]
fn a_construct_of_readings_is_a_reading() {
    let program = document(vec![steps(vec![read(), read()])]);
    let plan = nurt_plan::plan(&program, &common::registry()).unwrap();
    assert_eq!(plan.effect, EffectKind::Read);
    assert_eq!(plan.constructs().count(), 1);
    assert_eq!(plan.operations().count(), 0);
}

#[test]
fn several_changing_pipelines_are_allowed_inside_steps_only() {
    let program = document(vec![steps(vec![remove(), remove()])]);
    assert!(nurt_plan::plan(&program, &common::registry()).is_ok());
    let outside = document(vec![remove(), remove()]);
    let plan = nurt_plan::plan(&outside, &common::registry()).unwrap();
    assert!(plan.is_script());
}

#[test]
fn a_pipeline_inside_a_construct_is_checked_like_one_outside() {
    let two_effects = operation(pipeline(
        find("/a"),
        vec![mutate("remove", Vec::new()), mutate("remove", Vec::new())],
    ));
    let error = nurt_plan::plan(
        &document(vec![steps(vec![two_effects])]),
        &common::registry(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::OneEffect);
}

#[test]
fn only_the_operations_at_the_top_are_numbered() {
    let each = BoundStatement::Each(BoundEach {
        source: BoundExpr::List { items: Vec::new() },
        name: "f".to_owned(),
        parallel: None,
        atomic: true,
        body: vec![remove(), BoundStatement::Break(BoundJump { span: None })],
        span: None,
    });
    let program = document(vec![read(), each, read()]);
    let plan = nurt_plan::plan(&program, &common::registry()).unwrap();
    let numbers: Vec<u32> = plan.operations().map(|operation| operation.index).collect();
    assert_eq!(numbers, [0, 2]);
    assert_eq!(plan.constructs().next().unwrap().index, 1);
}

#[test]
fn a_call_inherits_the_effect_of_its_function() {
    let define = BoundStatement::Define(nurt_contracts::bound::BoundDefine {
        name: "clean".to_owned(),
        params: Vec::new(),
        body: vec![remove()],
        span: None,
    });
    let call = operation(BoundPipeline {
        head: BoundHead::Call(BoundCall {
            name: "clean".to_owned(),
            args: Vec::new(),
            span: None,
        }),
        stages: Vec::new(),
        span: None,
    });
    let plan = nurt_plan::plan(&document(vec![define, call]), &common::registry()).unwrap();
    assert_eq!(plan.effect, EffectKind::Local);
    let PlanStatement::Operation(operation) = &plan.statements[1] else {
        panic!("not an operation")
    };
    assert_eq!(operation.pipeline.effect, EffectKind::Local);
    assert!(operation.pipeline.call.is_some());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
