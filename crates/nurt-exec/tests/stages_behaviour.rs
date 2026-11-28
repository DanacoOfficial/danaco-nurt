// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/stages_behaviour.rs
// Responsibility:   Behaviour of the record stages on streams built in the tests.
// Dependencies:     harness, nurt_catalog, nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Behaviour of the record stages on streams built in the tests.

// Tests fail through unwrap, expect and panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "fixtures/stages/harness.rs"]
mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};

use harness::{
    Outcome, ScriptedRunner, Setup, args, bin, code, context, counted, ctx, entry, error_of, exec,
    expr, field, fields, int, line, read, records_of, run, value,
};
use nurt_contracts::ast::BinOp;
use nurt_contracts::bound::{
    BoundExpr, BoundHead, BoundMapEntry, BoundPipeline, BoundStage, Phase,
};
use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, OrderKey, RecordStream, StageImpl, StageInput, StreamOrder,
};
use nurt_contracts::value::{Bytes, Duration, Range, Record, RecordKind, Regex, Size, Value};
use nurt_exec::stages::{
    Assert, Count, Distinct, Filter, First, Group, Last, Parse, Select, Skip, Sort, Split,
    Summarize, Verify,
};

fn paths(records: &[Record]) -> Vec<&str> {
    records
        .iter()
        .map(|record| {
            record
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .collect()
}

fn files() -> Vec<Record> {
    vec![
        entry("/home/user/project/a.rs", 300),
        entry("/home/user/project/b.md", 100),
        entry("/home/user/project/c.rs", 200),
        entry("/home/user/project/d.md", 100),
    ]
}

fn larger_than(bytes: u64) -> BoundExpr {
    bin(BinOp::Gt, field("size"), BoundExpr::Size { v: bytes })
}

fn range(start: i64, end: i64) -> ArgValue {
    value(Range::new(Some(start), Some(end)).unwrap())
}

#[test]
fn filter_keeps_records_that_meet_the_condition() {
    let args = args("filter", &[("condition", expr(larger_than(150)))]);
    let kept = records_of(&Filter, &args, files());
    assert_eq!(
        paths(&kept),
        ["/home/user/project/a.rs", "/home/user/project/c.rs"]
    );
}

#[test]
fn filter_treats_null_as_false_and_rejects_other_types() {
    let null = args("filter", &[("condition", value(Value::Null))]);
    assert_eq!(
        code(&error_of(&Filter, &null, files())),
        ErrorCode::MissingParam
    );
    let empty = args("filter", &[("condition", expr(BoundExpr::Null))]);
    assert!(records_of(&Filter, &empty, files()).is_empty());
    let text = args("filter", &[("condition", expr(field("name")))]);
    let error = error_of(&Filter, &text, files());
    assert_eq!(error.code(), ErrorCode::Type);
    assert_eq!(error.expected, Some(serde_json::json!(["bool"])));
}

#[test]
fn filter_reports_unknown_fields_with_the_record_fields() {
    let args = args(
        "filter",
        &[("condition", expr(bin(BinOp::Gt, field("sise"), int(1))))],
    );
    let error = error_of(&Filter, &args, files());
    assert_eq!(error.code(), ErrorCode::UnknownField);
    let expected = error.expected.clone().unwrap_or_default().to_string();
    assert!(expected.contains("size"), "{expected}");
}

#[test]
fn filter_on_an_empty_stream_yields_nothing() {
    let args = args("filter", &[("condition", expr(larger_than(1)))]);
    assert!(records_of(&Filter, &args, Vec::new()).is_empty());
}

#[test]
fn filter_and_first_stop_pulling_the_source() {
    let pulled = AtomicUsize::new(0);
    let records: Vec<Record> = (0..1000)
        .map(|index| entry(&format!("/home/user/project/f{index}.rs"), index))
        .collect();
    let ctx = ctx(Stage::Read);
    let filter = args("filter", &[("condition", expr(larger_than(9)))]);
    let first = args("first", &[("count", value(2))]);
    let filtered = Filter
        .as_read()
        .unwrap()
        .execute(
            &filter,
            StageInput::records(counted(records, &pulled)),
            &ctx,
        )
        .unwrap();
    let output = First
        .as_read()
        .unwrap()
        .execute(&first, filtered.into(), &ctx)
        .unwrap();
    assert_eq!(read(output).unwrap().records().len(), 2);
    assert_eq!(pulled.load(Ordering::SeqCst), 12);
}

#[test]
fn filter_works_on_exec_records_after_the_effect() {
    let args = args(
        "filter",
        &[(
            "condition",
            expr(bin(BinOp::Ne, field("exit_code"), int(0))),
        )],
    );
    let input = vec![exec(&["go", "test"], 0), exec(&["go", "vet"], 1)];
    let kept = run(&Filter, &args, input, &ctx(Stage::After))
        .unwrap()
        .records();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].get("exit_code"), Some(&Value::Int(1)));
}

#[test]
fn select_keeps_fields_in_order_and_kind() {
    let args = args("select", &[("fields", fields(&["size", "path"]))]);
    let selected = records_of(&Select, &args, files());
    assert_eq!(selected[0].kind, RecordKind::Entry);
    assert_eq!(
        selected[0].field_names().collect::<Vec<_>>(),
        ["size", "path"]
    );
}

#[test]
fn select_computes_named_expressions_into_map_records() {
    let doubled = BoundExpr::Map {
        entries: vec![BoundMapEntry {
            k: "double".to_owned(),
            v: bin(BinOp::Mul, field("size"), int(2)),
        }],
    };
    let args = args(
        "select",
        &[(
            "fields",
            expr(BoundExpr::List {
                items: vec![field("name"), doubled],
            }),
        )],
    );
    let selected = records_of(&Select, &args, files());
    assert_eq!(selected[0].kind, RecordKind::Map);
    assert_eq!(
        selected[0].get("double"),
        Some(&Value::Size(Size::from_bytes(600)))
    );
    assert_eq!(
        selected[0].field_names().collect::<Vec<_>>(),
        ["name", "double"]
    );
}

#[test]
fn select_accepts_field_names_as_values() {
    let args = args(
        "select",
        &[("fields", value(Value::List(vec![Value::from("name")])))],
    );
    let selected = records_of(&Select, &args, files());
    assert_eq!(selected[3].get("name"), Some(&Value::from("d.md")));
    let constant = args_map("label", Value::from("checked"));
    assert_eq!(
        records_of(&Select, &constant, files())[0].get("label"),
        Some(&Value::from("checked"))
    );
}

fn args_map(name: &str, constant: Value) -> nurt_contracts::stage::BoundArgs {
    let mut entries = nurt_contracts::value::Fields::new();
    entries.insert(name.to_owned(), constant);
    args("select", &[("fields", value(Value::Map(entries)))])
}

#[test]
fn select_rejects_unknown_fields_and_missing_fields() {
    let unknown = args("select", &[("fields", fields(&["owner"]))]);
    let error = error_of(&Select, &unknown, files());
    assert_eq!(error.code(), ErrorCode::UnknownField);
    assert_eq!(error.found.as_deref(), Some("owner"));
    let none = args("select", &[]);
    assert_eq!(
        code(&error_of(&Select, &none, files())),
        ErrorCode::MissingParam
    );
    let wrong = args("select", &[("fields", expr(int(3)))]);
    assert_eq!(code(&error_of(&Select, &wrong, files())), ErrorCode::Type);
}

#[test]
fn sort_orders_by_several_keys_and_is_stable() {
    let args = args("sort", &[("by", fields(&["extension", "size"]))]);
    let sorted = records_of(&Sort, &args, files());
    assert_eq!(
        paths(&sorted),
        [
            "/home/user/project/b.md",
            "/home/user/project/d.md",
            "/home/user/project/c.rs",
            "/home/user/project/a.rs",
        ]
    );
}

#[test]
fn sort_descending_keeps_ties_in_input_order() {
    let args = args(
        "sort",
        &[
            ("by", fields(&["size"])),
            ("order", value(Value::Enum("desc".to_owned()))),
        ],
    );
    let sorted = records_of(&Sort, &args, files());
    assert_eq!(
        paths(&sorted),
        [
            "/home/user/project/a.rs",
            "/home/user/project/c.rs",
            "/home/user/project/b.md",
            "/home/user/project/d.md",
        ]
    );
}

#[test]
fn sort_uses_the_total_order_for_mixed_and_null_values() {
    let records = vec![
        Record::new(RecordKind::Map).with("v", Value::Number(2.5)),
        Record::new(RecordKind::Map).with("v", Value::Null),
        Record::new(RecordKind::Map).with("v", Value::Int(2)),
        Record::new(RecordKind::Map).with("v", Value::from("text")),
    ];
    let args = args("sort", &[("by", fields(&["v"]))]);
    let sorted: Vec<Value> = records_of(&Sort, &args, records)
        .into_iter()
        .map(|record| record.get("v").cloned().unwrap())
        .collect();
    assert_eq!(
        sorted,
        [
            Value::Null,
            Value::Int(2),
            Value::Number(2.5),
            Value::from("text")
        ]
    );
}

#[test]
fn sort_is_deterministic_for_any_input_order() {
    let args = args("sort", &[("by", fields(&["size", "path"]))]);
    let mut reversed = files();
    reversed.reverse();
    assert_eq!(
        records_of(&Sort, &args, files()),
        records_of(&Sort, &args, reversed)
    );
}

#[test]
fn sort_declares_its_order_and_passes_sorted_streams_through() {
    let args = args("sort", &[("by", fields(&["size"]))]);
    let ctx = ctx(Stage::Read);
    let output = Sort
        .as_read()
        .unwrap()
        .execute(
            &args,
            StageInput::records(nurt_contracts::stage::RecordStream::from_records(files())),
            &ctx,
        )
        .unwrap();
    let order = StreamOrder {
        keys: vec![OrderKey {
            field: "size".to_owned(),
            descending: false,
        }],
    };
    assert_eq!(output.order, Some(order.clone()));
    let mut input = StageInput::records(nurt_contracts::stage::RecordStream::from_records(files()));
    input.order = Some(order);
    let unchanged = Sort.as_read().unwrap().execute(&args, input, &ctx).unwrap();
    assert_eq!(read(unchanged).unwrap().records(), files());
}

#[test]
fn sort_rejects_unknown_order_and_fields_and_respects_the_limit() {
    let order = args(
        "sort",
        &[("by", fields(&["size"])), ("order", value("down"))],
    );
    assert_eq!(
        code(&error_of(&Sort, &order, files())),
        ErrorCode::UnknownValue
    );
    let unknown = args("sort", &[("by", fields(&["owner"]))]);
    assert_eq!(
        code(&error_of(&Sort, &unknown, files())),
        ErrorCode::UnknownField
    );
    let by = args("sort", &[("by", fields(&["size"]))]);
    let limited = context(Setup {
        max_records: Some(3),
        ..Setup::default()
    });
    assert_eq!(
        code(&run(&Sort, &by, files(), &limited).unwrap_err()),
        ErrorCode::Limit
    );
    assert!(records_of(&Sort, &by, Vec::new()).is_empty());
}

#[test]
fn first_zero_reads_nothing_and_first_reports_its_limit() {
    let pulled = AtomicUsize::new(0);
    let args0 = args("first", &[("count", value(0))]);
    let ctx = ctx(Stage::Read);
    let output = First
        .as_read()
        .unwrap()
        .execute(&args0, StageInput::records(counted(files(), &pulled)), &ctx)
        .unwrap();
    assert!(read(output).unwrap().records().is_empty());
    assert_eq!(pulled.load(Ordering::SeqCst), 0);
    let host = nurt_testkit::host::TestHost::native();
    let config = nurt_contracts::config::Config::default();
    let registry = nurt_catalog::Registry::load_embedded().unwrap();
    let spec = registry.command("first").unwrap();
    let check = nurt_contracts::stage::CheckCtx::new(spec, &host, &config);
    let info = First
        .check(&args("first", &[("count", value(20))]), &check)
        .unwrap();
    assert_eq!(
        info.details.get(nurt_exec::stages::LIMIT_DETAIL),
        Some(&Value::Int(20))
    );
}

#[test]
fn skip_and_last_cut_the_stream() {
    let skip = args("skip", &[("count", value(3))]);
    assert_eq!(
        paths(&records_of(&Skip, &skip, files())),
        ["/home/user/project/d.md"]
    );
    let last = args("last", &[("count", value(2))]);
    assert_eq!(
        paths(&records_of(&Last, &last, files())),
        ["/home/user/project/c.rs", "/home/user/project/d.md"]
    );
    let more = args("last", &[("count", value(10))]);
    assert_eq!(records_of(&Last, &more, files()).len(), 4);
    let negative = args("skip", &[("count", value(-1))]);
    assert_eq!(
        code(&error_of(&Skip, &negative, files())),
        ErrorCode::InvalidValue
    );
    let text = args("first", &[("count", value("3"))]);
    assert_eq!(code(&error_of(&First, &text, files())), ErrorCode::Type);
}

#[test]
fn skip_then_first_pulls_only_what_it_needs() {
    let pulled = AtomicUsize::new(0);
    let records: Vec<Record> = (0..500)
        .map(|index| entry(&format!("/f{index}"), index))
        .collect();
    let ctx = ctx(Stage::Read);
    let skip = args("skip", &[("count", value(5))]);
    let first = args("first", &[("count", value(3))]);
    let skipped = Skip
        .as_read()
        .unwrap()
        .execute(&skip, StageInput::records(counted(records, &pulled)), &ctx)
        .unwrap();
    let output = First
        .as_read()
        .unwrap()
        .execute(&first, skipped.into(), &ctx)
        .unwrap();
    assert_eq!(
        paths(&read(output).unwrap().records()),
        ["/f5", "/f6", "/f7"]
    );
    assert_eq!(pulled.load(Ordering::SeqCst), 8);
}

#[test]
fn last_respects_the_record_limit() {
    let last = args("last", &[("count", value(10))]);
    let limited = context(Setup {
        max_records: Some(2),
        ..Setup::default()
    });
    assert_eq!(
        code(&run(&Last, &last, files(), &limited).unwrap_err()),
        ErrorCode::Limit
    );
    let small = args("last", &[("count", value(1))]);
    assert_eq!(
        run(&Last, &small, files(), &limited)
            .unwrap()
            .records()
            .len(),
        1
    );
}

#[test]
fn count_counts_records_before_and_after_the_effect() {
    let count = args("count", &[]);
    assert_eq!(
        run(&Count, &count, files(), &ctx(Stage::Read)).unwrap(),
        Outcome::Value(Value::Int(4))
    );
    assert_eq!(
        run(&Count, &count, Vec::new(), &ctx(Stage::Read)).unwrap(),
        Outcome::Value(Value::Int(0))
    );
    let after = run(&Count, &count, vec![exec(&["make"], 0)], &ctx(Stage::After)).unwrap();
    assert_eq!(after, Outcome::Value(Value::Int(1)));
    let ctx = ctx(Stage::Read);
    let single = Count
        .as_read()
        .unwrap()
        .execute(&count, StageInput::value(Value::Int(3)), &ctx);
    assert_eq!(code(&single.unwrap_err()), ErrorCode::Type);
}

fn assert_with(name: &str, argument: ArgValue) -> nurt_contracts::stage::BoundArgs {
    args("assert", &[(name, argument)])
}

fn assert_fails(args: &nurt_contracts::stage::BoundArgs, input: Vec<Record>) -> Error {
    let error = error_of(&Assert, args, input);
    assert_eq!(error.code(), ErrorCode::Assert, "{error}");
    assert_eq!(error.stage(), Some(Stage::Check));
    error
}

#[test]
fn assert_count_and_paths_compare_with_ranges() {
    assert_eq!(
        records_of(&Assert, &assert_with("count", range(4, 4)), files()),
        files()
    );
    let error = assert_fails(&assert_with("count", range(1, 2)), files());
    assert_eq!(error.expected, Some(serde_json::json!("1..2")));
    assert_eq!(error.actual, Some(serde_json::json!(4)));
    assert_eq!(error.sample.len(), 4);
    let lines = vec![line("/a", 1, "x"), line("/a", 2, "y"), line("/b", 1, "z")];
    assert_eq!(
        records_of(&Assert, &assert_with("paths", value(2)), lines.clone()).len(),
        3
    );
    let error = assert_fails(&assert_with("paths", value(3)), lines);
    assert_eq!(error.expected, Some(serde_json::json!(3)));
    assert_eq!(error.actual, Some(serde_json::json!(2)));
    assert_eq!(error.sample, ["/a", "/b"]);
}

#[test]
fn assert_paths_tells_apart_two_paths_that_are_not_valid_unicode_by_their_bytes() {
    let named = |raw: &[u8]| {
        line("/d/n\u{fffd}", 1, "x").with("raw_path", Value::Bytes(Bytes::new(raw.to_vec())))
    };
    let both = vec![named(b"/d/n\xff"), named(b"/d/n\xfe"), named(b"/d/n\xff")];
    assert_eq!(
        records_of(&Assert, &assert_with("paths", value(2)), both.clone()).len(),
        3
    );
    let error = assert_fails(&assert_with("paths", value(1)), both);
    assert_eq!(error.actual, Some(serde_json::json!(2)));
}

#[test]
fn assert_sample_is_cut_to_five_items() {
    let many: Vec<Record> = (0..9)
        .map(|index| entry(&format!("/home/user/f{index}"), 1))
        .collect();
    let error = assert_fails(&assert_with("empty", value(true)), many);
    assert_eq!(error.sample.len(), 5);
    assert_eq!(error.actual, Some(serde_json::json!(9)));
}

#[test]
fn assert_exit_code_checks_every_program_after_the_effect() {
    let args = assert_with("exit_code", range(0, 0));
    let passed = run(
        &Assert,
        &args,
        vec![exec(&["go", "build"], 0)],
        &ctx(Stage::After),
    )
    .unwrap();
    assert_eq!(passed.records().len(), 1);
    let failed = run(
        &Assert,
        &args,
        vec![exec(&["go", "test", "./..."], 2)],
        &ctx(Stage::After),
    )
    .unwrap_err();
    assert_eq!(failed.code(), ErrorCode::Assert);
    assert_eq!(failed.stage(), Some(Stage::After));
    assert_eq!(failed.actual, Some(serde_json::json!(2)));
    assert_eq!(failed.sample, ["go test ./..."]);
    let none = run(&Assert, &args, Vec::new(), &ctx(Stage::After)).unwrap_err();
    assert_eq!(none.actual, Some(serde_json::Value::Null));
}

#[test]
fn assert_all_and_any_evaluate_conditions() {
    let all = assert_with("all", expr(larger_than(150)));
    let error = assert_fails(&all, files());
    assert_eq!(error.expected, Some(serde_json::json!(4)));
    assert_eq!(error.actual, Some(serde_json::json!(2)));
    assert_eq!(
        error.sample,
        ["/home/user/project/b.md", "/home/user/project/d.md"]
    );
    assert_eq!(
        records_of(
            &Assert,
            &assert_with("any", expr(larger_than(250))),
            files()
        )
        .len(),
        4
    );
    let none = assert_fails(&assert_with("any", expr(larger_than(1000))), files());
    assert_eq!(none.actual, Some(serde_json::json!(0)));
    let typed = error_of(&Assert, &assert_with("all", expr(field("name"))), files());
    assert_eq!(typed.code(), ErrorCode::Type);
}

#[test]
fn assert_empty_contains_and_if_match() {
    assert!(records_of(&Assert, &assert_with("empty", value(true)), Vec::new()).is_empty());
    assert_fails(&assert_with("empty", value(false)), Vec::new());
    let lines = vec![line("/a", 1, "fn main() {}"), line("/a", 2, "}")];
    assert_eq!(
        records_of(
            &Assert,
            &assert_with("contains", value("main")),
            lines.clone()
        )
        .len(),
        2
    );
    let error = assert_fails(&assert_with("contains", value("unsafe block")), lines);
    assert_eq!(error.expected, Some(serde_json::json!("unsafe block")));
    let tagged = vec![entry("/a", 1).with("etag", Value::from("e1"))];
    assert_eq!(
        records_of(
            &Assert,
            &assert_with("if_match", value("e1")),
            tagged.clone()
        )
        .len(),
        1
    );
    let stale = assert_fails(&assert_with("if_match", value("e0")), tagged);
    assert_eq!(stale.actual, Some(serde_json::json!("e1")));
    assert_fails(&assert_with("if_match", value("e0")), Vec::new());
}

#[test]
fn assert_needs_exactly_one_condition() {
    let none = BoundArgs::new("assert").with("count", value(Value::Null), true);
    let error = error_of(&Assert, &none, files());
    assert_eq!(error.code(), ErrorCode::MissingParam);
    assert_eq!(error.stage(), Some(Stage::Bind));
    let two = args("assert", &[("count", value(3)), ("empty", value(false))]);
    let error = error_of(&Assert, &two, files());
    assert_eq!(error.code(), ErrorCode::ParamConflict);
    assert_eq!(error.fix, ["assert count: 3", "assert empty: false"]);
    let defaulted = BoundArgs::new("assert")
        .with("count", value(4), false)
        .with("empty", value(false), true);
    assert_eq!(records_of(&Assert, &defaulted, files()).len(), 4);
    let wrong = assert_with("count", value("four"));
    assert_eq!(code(&error_of(&Assert, &wrong, files())), ErrorCode::Type);
}

#[test]
fn assert_passes_the_header_and_order_on() {
    let ctx = ctx(Stage::Check);
    let header = Record::new(RecordKind::TestRun).with("passed", Value::Int(3));
    let mut input = StageInput::records(RecordStream::from_records(files()));
    input.header = Some(header.clone());
    let condition = assert_with("count", value(4));
    let output = Assert
        .as_read()
        .unwrap()
        .execute(&condition, input, &ctx)
        .unwrap();
    assert_eq!(output.header, Some(header));
    assert_eq!(read(output).unwrap().records(), files());
}

fn group_by(name: &str, keep_items: bool) -> BoundArgs {
    args(
        "group",
        &[("by", expr(field(name))), ("keep_items", value(keep_items))],
    )
}

#[test]
fn group_counts_records_by_key_in_key_order() {
    let groups = records_of(&Group, &group_by("extension", false), files());
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].kind, RecordKind::Group);
    assert_eq!(groups[0].get("key"), Some(&Value::from("md")));
    assert_eq!(groups[0].get("count"), Some(&Value::Int(2)));
    assert_eq!(groups[0].get("items"), None);
    let mut reversed = files();
    reversed.reverse();
    assert_eq!(
        records_of(&Group, &group_by("extension", false), reversed),
        groups
    );
}

#[test]
fn group_keeps_items_and_groups_by_expressions() {
    let groups = records_of(&Group, &group_by("extension", true), files());
    let items = groups[1].get("items").and_then(Value::as_list).unwrap();
    assert_eq!(items.len(), 2);
    let large = args("group", &[("by", expr(larger_than(150)))]);
    let groups = records_of(&Group, &large, files());
    assert_eq!(groups[0].get("key"), Some(&Value::Bool(false)));
    assert_eq!(groups[1].get("count"), Some(&Value::Int(2)));
    assert!(records_of(&Group, &group_by("extension", false), Vec::new()).is_empty());
    assert_eq!(
        code(&error_of(&Group, &group_by("owner", false), files())),
        ErrorCode::UnknownField
    );
    let limited = context(Setup {
        max_records: Some(3),
        ..Setup::default()
    });
    let kept = run(&Group, &group_by("extension", true), files(), &limited).unwrap_err();
    assert_eq!(kept.code(), ErrorCode::Limit);
}

#[test]
fn summarize_computes_statistics_per_field() {
    let args = args("summarize", &[("fields", fields(&["size", "name"]))]);
    let summary = records_of(&Summarize, &args, files());
    assert_eq!(summary.len(), 2);
    assert_eq!(summary[0].kind, RecordKind::Summary);
    assert_eq!(summary[0].get("field"), Some(&Value::from("size")));
    assert_eq!(summary[0].get("count"), Some(&Value::Int(4)));
    assert_eq!(
        summary[0].get("sum"),
        Some(&Value::Size(Size::from_bytes(700)))
    );
    assert_eq!(
        summary[0].get("min"),
        Some(&Value::Size(Size::from_bytes(100)))
    );
    assert_eq!(
        summary[0].get("max"),
        Some(&Value::Size(Size::from_bytes(300)))
    );
    assert_eq!(
        summary[0].get("average"),
        Some(&Value::Size(Size::from_bytes(175)))
    );
    assert_eq!(summary[1].get("sum"), Some(&Value::Null));
    assert_eq!(summary[1].get("min"), Some(&Value::from("a.rs")));
    assert_eq!(summary[1].get("max"), Some(&Value::from("d.md")));
}

#[test]
fn summarize_groups_skips_nulls_and_handles_empty_input() {
    let args = args(
        "summarize",
        &[
            ("fields", fields(&["size"])),
            ("by", expr(field("extension"))),
        ],
    );
    let summary = records_of(&Summarize, &args, files());
    assert_eq!(summary.len(), 2);
    assert_eq!(summary[0].get("extension"), Some(&Value::from("md")));
    assert_eq!(
        summary[1].get("sum"),
        Some(&Value::Size(Size::from_bytes(500)))
    );
    let numbers = vec![
        Record::new(RecordKind::Map).with("n", Value::Int(1)),
        Record::new(RecordKind::Map).with("n", Value::Null),
        Record::new(RecordKind::Map).with("n", Value::Number(2.0)),
        Record::new(RecordKind::Map).with("n", Value::Duration(Duration::from_millis(5))),
    ];
    let mixed = records_of(&Summarize, &args_fields(&["n"]), numbers[..3].to_vec());
    assert_eq!(mixed[0].get("count"), Some(&Value::Int(2)));
    assert_eq!(mixed[0].get("sum"), Some(&Value::Number(3.0)));
    assert_eq!(mixed[0].get("average"), Some(&Value::Number(1.5)));
    let not_numeric = records_of(&Summarize, &args_fields(&["n"]), numbers);
    assert_eq!(not_numeric[0].get("sum"), Some(&Value::Null));
    let empty = records_of(&Summarize, &args_fields(&["n"]), Vec::new());
    assert_eq!(empty[0].get("count"), Some(&Value::Int(0)));
    assert_eq!(empty[0].get("average"), Some(&Value::Null));
    let counts = records_of(
        &Summarize,
        &harness::args("summarize", &[("by", expr(field("extension")))]),
        files(),
    );
    assert_eq!(counts[1].get("count"), Some(&Value::Int(2)));
}

fn args_fields(names: &[&str]) -> BoundArgs {
    args("summarize", &[("fields", fields(names))])
}

#[test]
fn distinct_keeps_first_occurrences_with_counts() {
    let args = args("distinct", &[("by", fields(&["size"]))]);
    let unique = records_of(&Distinct, &args, files());
    assert_eq!(
        paths(&unique),
        [
            "/home/user/project/a.rs",
            "/home/user/project/b.md",
            "/home/user/project/c.rs"
        ]
    );
    assert_eq!(unique[1].get("count"), Some(&Value::Int(2)));
    let all = args_distinct_all();
    let duplicated = vec![line("/a", 1, "x"), line("/a", 1, "x"), line("/a", 2, "x")];
    let unique = records_of(&Distinct, &all, duplicated);
    assert_eq!(unique.len(), 2);
    assert_eq!(unique[0].get("count"), Some(&Value::Int(2)));
    assert!(records_of(&Distinct, &all, Vec::new()).is_empty());
}

fn args_distinct_all() -> BoundArgs {
    BoundArgs::new("distinct").with("by", value(Value::Null), true)
}

#[test]
fn distinct_is_deterministic_and_bounded() {
    let args = args("distinct", &[("by", fields(&["extension"]))]);
    let first = records_of(&Distinct, &args, files());
    let second = records_of(&Distinct, &args, files());
    assert_eq!(first, second);
    let limited = context(Setup {
        max_records: Some(1),
        ..Setup::default()
    });
    assert_eq!(
        code(&run(&Distinct, &args, files(), &limited).unwrap_err()),
        ErrorCode::Limit
    );
    let unknown = args_by("owner");
    assert_eq!(
        code(&error_of(&Distinct, &unknown, files())),
        ErrorCode::UnknownField
    );
}

fn args_by(name: &str) -> BoundArgs {
    args("distinct", &[("by", fields(&[name]))])
}

fn pattern(text: &str) -> ArgValue {
    value(Regex::new(text, "").unwrap())
}

fn cells(record: &Record) -> &nurt_contracts::value::Fields {
    record.get("cells").and_then(Value::as_map).unwrap()
}

#[test]
fn parse_turns_named_groups_into_row_cells() {
    let args = args(
        "parse",
        &[("pattern", pattern(r"(?<level>[A-Z]+): (?<message>.*)"))],
    );
    let input = vec![
        line("/var/log/app.log", 1, "INFO: started"),
        line("/var/log/app.log", 2, "plain text"),
        line("/var/log/app.log", 3, "ERROR: disk full"),
    ];
    let rows = records_of(&Parse, &args, input);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].kind, RecordKind::Row);
    assert_eq!(rows[1].get("no"), Some(&Value::Int(3)));
    assert_eq!(
        rows[1].get("path"),
        Some(&Value::Path("/var/log/app.log".to_owned()))
    );
    assert_eq!(cells(&rows[1]).get("level"), Some(&Value::from("ERROR")));
}

#[test]
fn parse_reads_content_and_match_records() {
    let args = args(
        "parse",
        &[("pattern", pattern(r"(?<key>\w+)=(?<value>\w+)?"))],
    );
    let content = Record::new(RecordKind::Content)
        .with("path", Value::Path("/etc/app.conf".to_owned()))
        .with("text", Value::from("a=1\n\nb=\nc=3\n"));
    let rows = records_of(&Parse, &args, vec![content]);
    let numbers: Vec<_> = rows
        .iter()
        .map(|row| row.get("no").cloned().unwrap())
        .collect();
    assert_eq!(numbers, [Value::Int(1), Value::Int(3), Value::Int(4)]);
    assert_eq!(cells(&rows[1]).get("value"), Some(&Value::Null));
    let found = Record::new(RecordKind::Match)
        .with("path", Value::Path("/etc/app.conf".to_owned()))
        .with("no", Value::Int(7))
        .with("text", Value::from("x"))
        .with("line", Value::from("x=9"));
    let rows = records_of(&Parse, &args, vec![found]);
    assert_eq!(cells(&rows[0]).get("value"), Some(&Value::from("9")));
}

#[test]
fn parse_rejects_patterns_without_groups_and_other_kinds() {
    let plain = args("parse", &[("pattern", pattern("[a-z]+"))]);
    assert_eq!(
        code(&error_of(&Parse, &plain, Vec::new())),
        ErrorCode::InvalidValue
    );
    let broken = args("parse", &[("pattern", pattern("(?<a>x"))]);
    assert_eq!(
        code(&error_of(&Parse, &broken, Vec::new())),
        ErrorCode::Regex
    );
    let named = args("parse", &[("pattern", pattern("(?<a>x)"))]);
    assert_eq!(code(&error_of(&Parse, &named, files())), ErrorCode::Type);
    assert!(records_of(&Parse, &named, Vec::new()).is_empty());
}

#[test]
fn split_names_columns_by_position_or_by_names() {
    let plain = args("split", &[("delimiter", value(","))]);
    let rows = records_of(&Split, &plain, vec![line("/data/a.csv", 2, "x,y,z")]);
    let names: Vec<&str> = cells(&rows[0]).keys().map(String::as_str).collect();
    assert_eq!(names, ["c1", "c2", "c3"]);
    assert_eq!(rows[0].get("no"), Some(&Value::Int(2)));
    let named = args(
        "split",
        &[
            ("delimiter", value(",")),
            (
                "names",
                value(Value::List(vec![
                    Value::from("id"),
                    Value::from("name"),
                    Value::from("role"),
                    Value::from("team"),
                ])),
            ),
        ],
    );
    let rows = records_of(&Split, &named, vec![line("/data/a.csv", 1, "7,ada,admin")]);
    assert_eq!(cells(&rows[0]).get("name"), Some(&Value::from("ada")));
    assert_eq!(cells(&rows[0]).get("team"), Some(&Value::Null));
    let short = args(
        "split",
        &[
            ("delimiter", value(",")),
            ("names", value(Value::List(vec![Value::from("id")]))),
        ],
    );
    let rows = records_of(&Split, &short, vec![line("/data/a.csv", 1, "7,ada")]);
    assert_eq!(cells(&rows[0]).get("c2"), Some(&Value::from("ada")));
}

#[test]
fn split_accepts_patterns_and_rejects_invalid_delimiters() {
    let spaces = args("split", &[("delimiter", pattern(r"\s+"))]);
    let rows = records_of(&Split, &spaces, vec![line("/a", 1, "a   b\tc")]);
    assert_eq!(cells(&rows[0]).len(), 3);
    let empty = args("split", &[("delimiter", value(""))]);
    assert_eq!(
        code(&error_of(&Split, &empty, Vec::new())),
        ErrorCode::InvalidValue
    );
    let number = args("split", &[("delimiter", value(3))]);
    assert_eq!(
        code(&error_of(&Split, &number, Vec::new())),
        ErrorCode::Type
    );
    let comma = args("split", &[("delimiter", value(","))]);
    assert_eq!(code(&error_of(&Split, &comma, files())), ErrorCode::Type);
}

fn check_pipeline() -> ArgValue {
    ArgValue::Pipeline(BoundPipeline {
        head: BoundHead::Stage(BoundStage {
            name: "lines".to_owned(),
            effect: EffectKind::Read,
            phase: Phase::Source,
            waits: false,
            args: Vec::new(),
            span: None,
        }),
        stages: Vec::new(),
        span: None,
    })
}

#[test]
fn verify_runs_the_check_and_passes_the_effect_on() {
    let runner = ScriptedRunner::new(Ok(vec![line("/a", 1, "ok")]));
    let ctx = context(Setup {
        step: Some(Stage::Verify),
        runner: Some(runner.clone()),
        ..Setup::default()
    });
    let args = args("verify", &[("check", check_pipeline())]);
    let effects = vec![harness::effect("/home/user/project/main.go")];
    let output = run(&Verify, &args, effects.clone(), &ctx).unwrap();
    assert_eq!(output.records(), effects);
    assert_eq!(runner.calls(), 1);
}

#[test]
fn verify_returns_the_error_of_the_check() {
    let failure = Error::new(ErrorCode::Assert, "expected exit code 0, found 1");
    let runner = ScriptedRunner::new(Err(failure));
    let ctx = context(Setup {
        step: Some(Stage::Verify),
        runner: Some(runner),
        ..Setup::default()
    });
    let args = args("verify", &[("check", check_pipeline())]);
    let error = run(&Verify, &args, vec![harness::effect("/a")], &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Assert);
}

fn durations(millis: &[i64]) -> Vec<Record> {
    millis
        .iter()
        .map(|value| {
            Record::new(RecordKind::Map).with("d", Value::Duration(Duration::from_millis(*value)))
        })
        .collect()
}

#[test]
fn summarize_sums_durations_and_rounds_their_average_to_the_nearest_unit() {
    let summary = records_of(&Summarize, &args_fields(&["d"]), durations(&[1, 2]));
    assert_eq!(
        summary[0].get("sum"),
        Some(&Value::Duration(Duration::from_millis(3)))
    );
    assert_eq!(
        summary[0].get("average"),
        Some(&Value::Duration(Duration::from_millis(2)))
    );
    let summary = records_of(&Summarize, &args_fields(&["d"]), durations(&[1, 1, 2]));
    assert_eq!(
        summary[0].get("average"),
        Some(&Value::Duration(Duration::from_millis(1)))
    );
}

#[test]
fn summarize_sum_of_integers_beyond_the_integer_range_is_a_number() {
    let records = vec![
        Record::new(RecordKind::Map).with("n", Value::Int(i64::MAX)),
        Record::new(RecordKind::Map).with("n", Value::Int(i64::MAX)),
    ];
    let summary = records_of(&Summarize, &args_fields(&["n"]), records);
    assert_eq!(summary[0].get("count"), Some(&Value::Int(2)));
    assert_eq!(
        summary[0].get("sum"),
        Some(&Value::Number(2.0 * i64::MAX as f64))
    );
}

#[test]
fn last_zero_keeps_nothing_and_first_beyond_the_input_keeps_everything() {
    let none = args("last", &[("count", value(0))]);
    assert!(records_of(&Last, &none, files()).is_empty());
    let all = args("first", &[("count", value(10))]);
    assert_eq!(records_of(&First, &all, files()).len(), 4);
}

#[test]
fn select_reads_inside_a_field_and_names_the_column_by_its_last_segment() {
    let mut meta = nurt_contracts::value::Fields::new();
    meta.insert("owner".to_owned(), Value::from("ada"));
    let record = Record::new(RecordKind::Entry).with("meta", Value::Map(meta));
    let nested = BoundExpr::Field {
        name: "owner".to_owned(),
        of: Some(Box::new(field("meta"))),
    };
    let args = args("select", &[("fields", expr(nested))]);
    let selected = records_of(&Select, &args, vec![record]);
    assert_eq!(selected[0].kind, RecordKind::Map);
    assert_eq!(selected[0].get("owner"), Some(&Value::from("ada")));
}

#[test]
fn parse_numbers_the_lines_of_content_with_crlf_and_multibyte_text() {
    let args = args(
        "parse",
        &[("pattern", pattern(r"(?<key>\w+)=(?<value>\d+)"))],
    );
    let content = Record::new(RecordKind::Content)
        .with("path", Value::Path("/etc/ą.conf".to_owned()))
        .with("text", Value::from("ąć=1\r\nżź=2\r\n"));
    let rows = records_of(&Parse, &args, vec![content]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get("no"), Some(&Value::Int(1)));
    assert_eq!(rows[1].get("no"), Some(&Value::Int(2)));
    assert_eq!(cells(&rows[1]).get("key"), Some(&Value::from("żź")));
    assert_eq!(cells(&rows[1]).get("value"), Some(&Value::from("2")));
}

#[test]
fn split_cuts_at_a_multibyte_delimiter_and_keeps_the_empty_tail() {
    let args = args("split", &[("delimiter", value("→"))]);
    let rows = records_of(&Split, &args, vec![line("/a", 1, "ą→ę→")]);
    let names: Vec<&str> = cells(&rows[0]).keys().map(String::as_str).collect();
    assert_eq!(names, ["c1", "c2", "c3"]);
    assert_eq!(cells(&rows[0]).get("c1"), Some(&Value::from("ą")));
    assert_eq!(cells(&rows[0]).get("c3"), Some(&Value::from("")));
}

#[test]
fn verify_needs_a_pipeline_and_a_runner() {
    let missing = args("verify", &[]);
    assert_eq!(
        code(&error_of(&Verify, &missing, Vec::new())),
        ErrorCode::MissingParam
    );
    let wrong = args("verify", &[("check", value(1))]);
    assert_eq!(
        code(&error_of(&Verify, &wrong, Vec::new())),
        ErrorCode::Type
    );
    let no_runner = args("verify", &[("check", check_pipeline())]);
    assert_eq!(
        code(&error_of(&Verify, &no_runner, Vec::new())),
        ErrorCode::StageFailed
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
