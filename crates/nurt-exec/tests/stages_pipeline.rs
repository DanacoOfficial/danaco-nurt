// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/stages_pipeline.rs
// Responsibility:   Pipelines of record stages run by the executor over a counting test.
// Dependencies:     common, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Pipelines of record stages run by the executor over a counting test
//! source: results and the number of records read from the source.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Source, World, error_of, pipeline, single, stage};
use nurt_contracts::ast::BinOp;
use nurt_contracts::bound::{BoundArg, BoundExpr, BoundStage, Phase};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Record, RecordKind, Size, Value};
use nurt_exec::{Executor, Outcome};

const LIST: &str = "mock.list";

fn file(index: u64) -> Record {
    let extension = if index.is_multiple_of(2) { "rs" } else { "md" };
    let name = format!("f{index:02}.{extension}");
    Record::new(RecordKind::Entry)
        .with("path", Value::Path(format!("/home/user/project/{name}")))
        .with("name", Value::from(name))
        .with("extension", Value::from(extension))
        .with("size", Value::Size(Size::from_bytes(index * 100)))
}

fn files(total: u64) -> Vec<Record> {
    (0..total).map(file).collect()
}

fn field(name: &str) -> BoundExpr {
    BoundExpr::Field {
        name: name.to_owned(),
        of: None,
    }
}

fn names(items: &[&str]) -> BoundExpr {
    BoundExpr::List {
        items: items.iter().map(|name| field(name)).collect(),
    }
}

fn arg(name: &str, value: BoundExpr) -> BoundArg {
    BoundArg::given(name, value)
}

fn transform(name: &str, args: Vec<BoundArg>) -> BoundStage {
    stage(name, Phase::Transform, args)
}

fn int(value: i64) -> BoundExpr {
    BoundExpr::Int { v: value }
}

fn larger_than(bytes: u64) -> BoundStage {
    transform(
        "filter",
        vec![arg(
            "condition",
            BoundExpr::Bin {
                op: BinOp::Gt,
                l: Box::new(field("size")),
                r: Box::new(BoundExpr::Size { v: bytes }),
            },
        )],
    )
}

fn sorted_by(key: &str, order: &str) -> BoundStage {
    transform(
        "sort",
        vec![
            arg("by", names(&[key])),
            arg(
                "order",
                BoundExpr::Enum {
                    v: order.to_owned(),
                },
            ),
        ],
    )
}

fn limit(name: &str, count: i64) -> BoundStage {
    transform(name, vec![arg("count", int(count))])
}

/// Runs the stages over `total` files and returns the outcome with the
/// number of records the source delivered.
fn run(total: u64, stages: Vec<BoundStage>) -> (Outcome, usize) {
    let world = World::new();
    let source = Source::new(LIST, files(total));
    let pulled: Arc<AtomicUsize> = Arc::clone(&source.pulled);
    let env = world.env(&[LIST], vec![Arc::new(source)]);
    let mut all = vec![stage(LIST, Phase::Source, Vec::new())];
    all.extend(stages);
    let outcome = Executor::new().run(&single(pipeline(None, all)), &env);
    (outcome, pulled.load(Ordering::SeqCst))
}

fn names_of(outcome: &Outcome) -> Vec<String> {
    outcome
        .last()
        .unwrap()
        .result
        .records
        .iter()
        .map(|record| {
            record
                .get("name")
                .and_then(Value::as_str)
                .unwrap()
                .to_owned()
        })
        .collect()
}

fn count_of(outcome: &Outcome) -> Option<Value> {
    outcome.last().unwrap().result.value.clone()
}

#[test]
fn counts_the_first_records_of_a_sorted_filtered_stream() {
    let (outcome, read) = run(
        20,
        vec![
            larger_than(1500),
            sorted_by("size", "desc"),
            limit("first", 3),
            transform("count", Vec::new()),
        ],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(count_of(&outcome), Some(Value::Int(3)));
    assert_eq!(read, 20);
}

#[test]
fn returns_the_largest_records_in_descending_order() {
    let (outcome, read) = run(
        20,
        vec![
            larger_than(1500),
            sorted_by("size", "desc"),
            limit("first", 3),
        ],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(names_of(&outcome), ["f19.md", "f18.rs", "f17.md"]);
    assert_eq!(read, 20);
}

#[test]
fn stops_reading_the_source_after_first() {
    let (outcome, read) = run(
        1000,
        vec![
            transform(
                "filter",
                vec![arg(
                    "condition",
                    BoundExpr::Bin {
                        op: BinOp::Eq,
                        l: Box::new(field("extension")),
                        r: Box::new(BoundExpr::Text { v: "rs".to_owned() }),
                    },
                )],
            ),
            limit("first", 3),
            transform("count", Vec::new()),
        ],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(count_of(&outcome), Some(Value::Int(3)));
    assert!(read <= 6, "read {read} records");
}

#[test]
fn skips_leading_records_and_keeps_the_rest_in_order() {
    let (outcome, read) = run(6, vec![limit("skip", 4)]);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(names_of(&outcome), ["f04.rs", "f05.md"]);
    assert_eq!(read, 6);
}

#[test]
fn returns_the_trailing_records_with_last() {
    let (outcome, read) = run(10, vec![limit("last", 2)]);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(names_of(&outcome), ["f08.rs", "f09.md"]);
    assert_eq!(read, 10);
}

#[test]
fn counts_an_empty_result_as_zero() {
    let (outcome, read) = run(5, vec![larger_than(10_000), transform("count", Vec::new())]);
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(count_of(&outcome), Some(Value::Int(0)));
    assert_eq!(read, 5);
}

#[test]
fn selects_fields_in_the_requested_order() {
    let (outcome, _) = run(
        4,
        vec![
            limit("first", 2),
            transform("select", vec![arg("fields", names(&["size", "name"]))]),
        ],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let records = &outcome.last().unwrap().result.records;
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].field_names().collect::<Vec<_>>(),
        ["size", "name"]
    );
    assert_eq!(records[1].get("name"), Some(&Value::from("f01.md")));
}

#[test]
fn groups_records_by_key_and_orders_the_groups() {
    let (outcome, read) = run(
        7,
        vec![
            transform("group", vec![arg("by", field("extension"))]),
            sorted_by("count", "desc"),
        ],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let groups = &outcome.last().unwrap().result.records;
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].get("key"), Some(&Value::from("rs")));
    assert_eq!(groups[0].get("count"), Some(&Value::Int(4)));
    assert_eq!(groups[1].get("key"), Some(&Value::from("md")));
    assert_eq!(groups[1].get("count"), Some(&Value::Int(3)));
    assert_eq!(read, 7);
}

#[test]
fn summarizes_a_numeric_field() {
    let (outcome, read) = run(
        5,
        vec![transform(
            "summarize",
            vec![arg("fields", names(&["size"]))],
        )],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let summary = &outcome.last().unwrap().result.records;
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].get("count"), Some(&Value::Int(5)));
    assert_eq!(
        summary[0].get("sum"),
        Some(&Value::Size(Size::from_bytes(1000)))
    );
    assert_eq!(read, 5);
}

#[test]
fn keeps_the_first_record_of_each_distinct_value() {
    let (outcome, _) = run(
        6,
        vec![transform(
            "distinct",
            vec![arg("by", names(&["extension"]))],
        )],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(names_of(&outcome), ["f00.rs", "f01.md"]);
}

#[test]
fn passes_a_stream_that_meets_the_assertion() {
    let (outcome, _) = run(
        5,
        vec![
            larger_than(150),
            transform(
                "assert",
                vec![arg(
                    "count",
                    BoundExpr::Range {
                        lo: Some(3),
                        hi: Some(3),
                    },
                )],
            ),
            transform("count", Vec::new()),
        ],
    );
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    assert_eq!(count_of(&outcome), Some(Value::Int(3)));
}

#[test]
fn fails_with_the_assertion_error_and_reports_the_actual_count() {
    let (outcome, _) = run(
        5,
        vec![transform(
            "assert",
            vec![arg(
                "count",
                BoundExpr::Range {
                    lo: Some(9),
                    hi: Some(9),
                },
            )],
        )],
    );
    let error = error_of(&outcome);
    assert_eq!(error.code(), ErrorCode::Assert);
    assert_eq!(error.actual, Some(serde_json::json!(5)));
    assert_eq!(error.sample.len(), 5);
}

#[test]
fn rejects_a_condition_that_is_not_boolean() {
    let (outcome, _) = run(
        3,
        vec![transform("filter", vec![arg("condition", field("size"))])],
    );
    assert_eq!(error_of(&outcome).code(), ErrorCode::Type);
}

#[test]
fn rejects_an_unknown_field_in_a_sort_key() {
    let (outcome, _) = run(
        3,
        vec![transform("sort", vec![arg("by", names(&["owner"]))])],
    );
    assert_eq!(error_of(&outcome).code(), ErrorCode::UnknownField);
}

#[test]
fn produces_the_same_result_on_every_run() {
    let stages = || {
        vec![
            transform("group", vec![arg("by", field("extension"))]),
            sorted_by("key", "asc"),
        ]
    };
    let (first, _) = run(12, stages());
    let (second, _) = run(12, stages());
    assert_eq!(
        first.last().unwrap().result.records,
        second.last().unwrap().result.records
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
