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
use std::sync::Arc;

use common::{
    after, binding, checking, document, field, find, from_ref, given, greater, int, mutate,
    operation, pipeline, run, single, source, text, transform,
};
use nurt_catalog::Registry;
use nurt_contracts::bound::{BoundDocument, BoundExpr, BoundMapEntry, BoundRef};
use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, Demand, FieldDemand, InputKinds, StageDescriptor, StageImpl, StaticInfo,
};
use nurt_contracts::value::Value;
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
fn named_result_used_inside_a_map_of_a_change_needs_entity_tags() {
    let argument = BoundExpr::Map {
        entries: vec![BoundMapEntry {
            k: "keep".to_owned(),
            v: BoundExpr::Ref(BoundRef {
                name: "old".to_owned(),
                fields: Vec::new(),
            }),
        }],
    };
    let changed = document(vec![
        binding("old", pipeline(find("/a"), Vec::new())),
        operation(pipeline(
            find("/b"),
            vec![mutate("remove", vec![given("except", argument)])],
        )),
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
}

#[test]
fn sort_needs_every_record_so_the_limit_does_not_reach_the_source() {
    let by = BoundExpr::List {
        items: vec![field("size")],
    };
    let sort = transform("sort", vec![given("by", by)]);
    let program = single(pipeline(find("/a"), vec![sort, first(3)]));
    let demands = demands(&plan(&program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, None);
    assert_eq!(demands[1].limit, Some(3));
    assert_eq!(demands[2].limit, None);
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

#[test]
fn distinct_without_by_needs_every_field_of_its_input() {
    let program = single(pipeline(
        source("history", Vec::new()),
        vec![transform("distinct", Vec::new())],
    ));
    let demands = demands(&plan(&program, &PlanOptions::new().with_fields(["id"])));
    assert_eq!(demands[0].fields, FieldDemand::All);
}

/// What the static check of a stand-in stage reports as the detail `limit`.
#[derive(Clone)]
enum Report {
    /// The value of the argument `count`, as the record stages report it.
    Count,
    /// A fixed value.
    Fixed(Value),
    /// No detail.
    Nothing,
    /// An error.
    Reject,
}

/// Implementation of a record stage that takes any input and has no
/// behaviour except its static check.
struct Limiting {
    command: &'static str,
    report: Report,
}

impl StageImpl for Limiting {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: self.command.to_owned(),
            source: false,
            input: InputKinds::Any,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let mut info = StaticInfo::default();
        let limit = match &self.report {
            Report::Count => args.value("count").cloned(),
            Report::Fixed(value) => Some(value.clone()),
            Report::Nothing => None,
            Report::Reject => return Err(Error::new(ErrorCode::InvalidValue, "count too large")),
        };
        if let Some(limit) = limit {
            info.details.insert("limit".to_owned(), limit);
        }
        Ok(info)
    }
}

fn registry_reporting(stages: &[(&'static str, Report)]) -> Registry {
    let mut registry = common::registry();
    for (command, report) in stages {
        registry
            .register_impl(Arc::new(Limiting {
                command,
                report: report.clone(),
            }))
            .unwrap();
    }
    registry
}

fn plan_in(registry: &Registry, document: &BoundDocument, options: &PlanOptions) -> Plan {
    nurt_plan::plan_with(document, registry, options).unwrap()
}

#[test]
fn reported_limit_reaches_the_source() {
    let registry = registry_reporting(&[("first", Report::Count)]);
    let program = single(pipeline(find("/srv/app"), vec![filter_large(), first(10)]));
    let demands = demands(&plan_in(&registry, &program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, None);
    assert_eq!(demands[1].limit, Some(10));
    assert_eq!(demands[2].limit, None);
}

#[test]
fn reported_limit_replaces_the_argument_of_the_command() {
    let registry = registry_reporting(&[("first", Report::Fixed(Value::Int(7)))]);
    let program = single(pipeline(find("/srv/app"), vec![first(10)]));
    let demands = demands(&plan_in(&registry, &program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, Some(7));
}

#[test]
fn reported_limit_is_capped_by_the_limit_after_the_stage() {
    let registry = registry_reporting(&[("first", Report::Count)]);
    let program = single(pipeline(find("/a"), vec![first(10), first(3)]));
    let capped = demands(&plan_in(&registry, &program, &PlanOptions::new()));
    assert_eq!(capped[0].limit, Some(3));
    assert_eq!(capped[1].limit, Some(3));
    let single_stage = single(pipeline(find("/a"), vec![first(10)]));
    let paged = demands(&plan_in(
        &registry,
        &single_stage,
        &PlanOptions::new().with_limit(4),
    ));
    assert_eq!(paged[0].limit, Some(4));
}

#[test]
fn reported_limit_of_zero_reads_nothing() {
    let registry = registry_reporting(&[("first", Report::Count)]);
    let program = single(pipeline(find("/a"), vec![first(0)]));
    let demands = demands(&plan_in(&registry, &program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, Some(0));
}

#[test]
fn any_stage_may_report_a_limit() {
    let registry = registry_reporting(&[("distinct", Report::Fixed(Value::Int(5)))]);
    let program = single(pipeline(
        find("/a"),
        vec![transform("distinct", Vec::new())],
    ));
    let demands = demands(&plan_in(&registry, &program, &PlanOptions::new()));
    assert_eq!(demands[0].limit, Some(5));
}

#[test]
fn stage_without_a_usable_report_keeps_the_rule_of_the_command() {
    let program = single(pipeline(find("/a"), vec![first(10)]));
    let unusable = [
        Report::Nothing,
        Report::Reject,
        Report::Fixed(Value::Int(-1)),
        Report::Fixed(Value::Text("7".to_owned())),
    ];
    for report in unusable {
        let registry = registry_reporting(&[("first", report)]);
        let demands = demands(&plan_in(&registry, &program, &PlanOptions::new()));
        assert_eq!(demands[0].limit, Some(10));
    }
}

#[test]
fn registered_stand_in_does_not_change_golden_plans() {
    let registry = registry_reporting(&[("first", Report::Count)]);
    let plain = common::registry();
    for entry in std::fs::read_dir(common::golden_dir()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        if stem.ends_with(".generated") {
            continue;
        }
        let program = common::golden(stem);
        let options = PlanOptions::new().with_limit(50);
        assert_eq!(
            plan_in(&registry, &program, &options),
            plan_in(&plain, &program, &options),
            "{stem}"
        );
    }
}

#[test]
fn golden_tree_with_first_limits_the_source_by_the_reported_limit() {
    let registry = registry_reporting(&[("first", Report::Fixed(Value::Int(6)))]);
    let program = common::golden("task-01-largest-go-files");
    let reported = plan_in(&registry, &program, &PlanOptions::new());
    let plain = plan_in(&common::registry(), &program, &PlanOptions::new());
    let first_index = reported
        .operations()
        .next()
        .unwrap()
        .pipeline
        .stages
        .iter()
        .position(|stage| stage.command() == "first")
        .unwrap();
    assert_eq!(demands(&reported)[first_index].limit, None);
    let upstream = |plan: &Plan| demands(plan)[first_index - 1].limit;
    assert_eq!(upstream(&reported), Some(6));
    assert_eq!(upstream(&plain), Some(20));
}

#[test]
fn rejected_check_fails_the_check_of_the_plan_but_not_the_plan() {
    let registry = registry_reporting(&[("first", Report::Reject)]);
    let program = single(pipeline(find("/a"), vec![first(10)]));
    let plan = plan_in(&registry, &program, &PlanOptions::new());
    let error =
        nurt_plan::check(&plan, &registry, &common::host(), &Config::default()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn check_summary_lists_the_reported_limit() {
    let registry = registry_reporting(&[("first", Report::Count)]);
    let program = single(pipeline(find("/a"), vec![first(10)]));
    let plan = plan_in(&registry, &program, &PlanOptions::new());
    let summary = nurt_plan::check(&plan, &registry, &common::host(), &Config::default()).unwrap();
    assert_eq!(summary.details.len(), 1);
    assert_eq!(summary.details[0].command, "first");
    assert_eq!(
        summary.details[0].details.get("limit"),
        Some(&Value::Int(10))
    );
}
