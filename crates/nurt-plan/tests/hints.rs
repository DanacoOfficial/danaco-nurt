// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Field demand, entity tags and limits computed for the stages of a plan.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::collections::BTreeSet;

use common::{
    after, binding, checking, document, field, find, from_ref, given, greater, int, mutate,
    operation, pipeline, run, single, source, text, transform,
};
use nurt_contracts::bound::{BoundDocument, BoundExpr};
use nurt_contracts::stage::{Demand, FieldDemand};
use nurt_plan::{Plan, PlanOptions};

fn plan(document: &BoundDocument, options: &PlanOptions) -> Plan {
    nurt_plan::plan_with(document, &common::registry(), options).unwrap()
}

/// Demands of the stages of the first operation.
fn demands(plan: &Plan) -> Vec<Demand> {
    plan.operations()
        .next()
        .unwrap()
        .pipeline
        .stages
        .iter()
        .map(|stage| stage.demand.clone())
        .collect()
}

fn only(fields: &[&str]) -> FieldDemand {
    FieldDemand::Only(
        fields
            .iter()
            .map(|field| (*field).to_owned())
            .collect::<BTreeSet<_>>(),
    )
}

fn filter_large() -> nurt_contracts::bound::BoundStage {
    transform(
        "filter",
        vec![given(
            "condition",
            greater("size", BoundExpr::Size { v: 1_000 }),
        )],
    )
}

fn first(count: i64) -> nurt_contracts::bound::BoundStage {
    transform("first", vec![given("count", int(count))])
}

#[test]
fn find_filter_first_limits_only_after_filter() {
    let program = single(pipeline(find("/srv/app"), vec![filter_large(), first(10)]));
    let demands = demands(&plan(&program, &PlanOptions::new().with_limit(50)));
    assert_eq!(demands[0].limit, None);
    assert_eq!(demands[1].limit, Some(10));
    assert_eq!(demands[2].limit, Some(50));
    assert!(demands.iter().all(|demand| !demand.etag));
    assert!(
        demands
            .iter()
            .all(|demand| demand.fields == FieldDemand::All)
    );
}

#[test]
fn requested_fields_flow_back_through_the_pipeline() {
    let program = common::golden("task-01-largest-go-files");
    let demands = demands(&plan(&program, &PlanOptions::new().with_fields(["path"])));
    assert_eq!(demands[3].fields, only(&["path"]));
    assert_eq!(demands[2].fields, only(&["path"]));
    assert_eq!(demands[1].fields, only(&["path", "size"]));
    let FieldDemand::Only(source) = &demands[0].fields else {
        panic!("the source demand lists fields");
    };
    assert!(source.contains("path") && source.contains("size"));
    assert_eq!(demands[2].limit, Some(20));
}

#[test]
fn search_before_replace_needs_entity_tags() {
    let program = common::golden("task-12-replace-verify-rollback");
    let plan = plan(&program, &PlanOptions::new().with_limit(50));
    let demands = demands(&plan);
    assert!(demands[0].etag);
    assert_eq!(demands[0].fields, FieldDemand::All);
    assert_eq!(demands[0].limit, None);
    assert!(!demands[1].etag);
    assert_eq!(demands[1].limit, None);
    let check = plan.operations().next().unwrap().pipeline.verify().unwrap();
    assert!(check.stages.iter().all(|stage| !stage.demand.etag));
}

#[test]
fn entity_tags_pass_through_checks_before_the_change() {
    let program = common::golden("task-02-replace-identifier-expected-count");
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert!(demands[0].etag);
    assert!(demands[1].etag);
    assert!(!demands[2].etag);
}

#[test]
fn assert_count_bounds_the_source() {
    let assert = checking("assert", vec![given("count", common::range(3, 3))]);
    let program = single(pipeline(
        find("/a"),
        vec![assert, mutate("remove", Vec::new())],
    ));
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, Some(4));
    assert_eq!(demands[1].limit, None);
}

#[test]
fn skip_adds_to_the_limit_after_it() {
    let skip = transform("skip", vec![given("count", int(5))]);
    let program = single(pipeline(find("/a"), vec![skip, first(10)]));
    let demands = demands(&plan(&program, &PlanOptions::new().with_limit(3)));
    assert_eq!(demands[0].limit, Some(8));
    assert_eq!(demands[1].limit, Some(3));
}

#[test]
fn count_needs_no_fields() {
    let program = single(pipeline(find("/a"), vec![transform("count", Vec::new())]));
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert_eq!(demands[0].fields, only(&[]));
    assert_eq!(demands[1].fields, FieldDemand::All);
}

#[test]
fn select_projects_its_fields_and_keeps_the_limit() {
    let fields = BoundExpr::List {
        items: vec![field("path"), field("size")],
    };
    let select = transform("select", vec![given("fields", fields)]);
    let program = single(pipeline(find("/a"), vec![select, first(5)]));
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert_eq!(demands[0].fields, only(&["path", "size"]));
    assert_eq!(demands[0].limit, Some(5));
}

#[test]
fn assert_if_match_needs_entity_tags() {
    let assert = checking("assert", vec![given("if_match", text("e1"))]);
    let program = single(pipeline(find("/a"), vec![assert]));
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert!(demands[0].etag);
    assert!(!demands[1].etag);
}

#[test]
fn named_result_used_by_a_change_needs_entity_tags() {
    let changed = document(vec![
        binding("old", pipeline(find("/a"), vec![filter_large()])),
        operation(from_ref("old", vec![mutate("remove", Vec::new())])),
    ]);
    let plan_changed = plan(&changed, &PlanOptions::new());
    let binding_plan = plan_changed.lets().next().unwrap();
    assert!(
        binding_plan
            .pipeline
            .stages
            .iter()
            .all(|stage| stage.demand.etag)
    );
    let counted = document(vec![
        binding("old", pipeline(find("/a"), Vec::new())),
        operation(from_ref("old", vec![transform("count", Vec::new())])),
    ]);
    let plan_counted = plan(&counted, &PlanOptions::new().with_limit(10));
    let binding_plan = plan_counted.lets().next().unwrap();
    assert!(!binding_plan.pipeline.stages[0].demand.etag);
    assert_eq!(binding_plan.pipeline.stages[0].demand.limit, None);
}

#[test]
fn page_limit_does_not_apply_to_changes() {
    let program = single(pipeline(find("/a"), vec![mutate("remove", Vec::new())]));
    let demands = demands(&plan(&program, &PlanOptions::new().with_limit(50)));
    assert_eq!(demands[0].limit, None);
    assert_eq!(demands[1].limit, None);
}

#[test]
fn effect_stage_never_gets_a_limit() {
    let program = single(pipeline(
        run(&["go", "test"]),
        vec![after("first", vec![given("count", int(5))])],
    ));
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, None);
    assert!(!demands[0].etag);
}

#[test]
fn requested_fields_reach_the_last_stage() {
    let program = single(pipeline(
        source("history", Vec::new()),
        vec![transform("distinct", Vec::new())],
    ));
    let demands = demands(&plan(&program, &PlanOptions::new().with_fields(["id"])));
    assert_eq!(demands[1].fields, only(&["id"]));
}
