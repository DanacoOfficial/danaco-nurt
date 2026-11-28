// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-help/tests/stages.rs
// Responsibility:   Tests of the commands help, describe and evaluate on a test execution context.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the commands help, describe and evaluate on a test execution context.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nurt_contracts::bound::BoundExpr;
use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
use nurt_contracts::envelope::{OutputFormat, RequestOptions};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{Evaluation, FieldInfo};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, CancelToken, ExecCtx, ExprEvaluator, InputKinds,
    RecordStream, StageData, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_help::commands::{DescribeStage, EvaluateStage, HelpStage};

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> WallTime {
        WallTime::UNIX_EPOCH
    }

    fn monotonic(&self) -> Duration {
        Duration::ZERO
    }

    fn stamp(&self) -> ClockStamp {
        ClockStamp {
            wall: WallTime::UNIX_EPOCH,
            boot: BootId("test".to_owned()),
            since_boot_with_suspend: Duration::ZERO,
            since_boot_active: Duration::ZERO,
        }
    }

    fn sleep(&self, _duration: Duration, _cancel: &CancelToken) -> Result<(), CancelReason> {
        Ok(())
    }
}

struct TestHost(HostInfo);

impl Host for TestHost {
    fn info(&self) -> &HostInfo {
        &self.0
    }

    fn env_var(&self, _name: &str) -> Option<OsString> {
        None
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        Vec::new()
    }
}

/// Evaluator of constants: an integer, a text or a boolean evaluates to
/// itself and every other expression fails with `E_TYPE`.
struct ConstantEvaluator;

impl ExprEvaluator for ConstantEvaluator {
    fn eval(
        &self,
        expr: &BoundExpr,
        _record: Option<&Record>,
        _ctx: &ExecCtx,
    ) -> Result<Value, Error> {
        match expr {
            BoundExpr::Int { v } => Ok(Value::Int(*v)),
            BoundExpr::Text { v } => Ok(Value::Text(v.clone())),
            BoundExpr::Bool { v } => Ok(Value::Bool(*v)),
            _ => Err(Error::new(
                ErrorCode::Type,
                "the test evaluator knows constants only",
            )),
        }
    }
}

fn host() -> Arc<dyn Host> {
    Arc::new(TestHost(HostInfo {
        os: Os::Linux,
        os_version: "6.8.0".to_owned(),
        architecture: "x86_64".to_owned(),
        hostname: "host".to_owned(),
        user: "user".to_owned(),
        home: PathBuf::from("/home/user"),
        temp: PathBuf::from("/tmp"),
        path_style: PathStyle::Posix,
        timezone: "UTC".to_owned(),
        locale: None,
        cpus: 1,
    }))
}

fn ctx(evaluator: bool, options: RequestOptions) -> ExecCtx {
    let mut builder = ExecCtx::builder(Arc::new(TestClock), host()).options(Arc::new(options));
    if evaluator {
        builder = builder.evaluator(Arc::new(ConstantEvaluator));
    }
    builder.build()
}

fn records(output: StageOutput<'_>) -> Vec<Record> {
    match output.data {
        StageData::Records(stream) => stream.collect::<Result<Vec<_>, _>>().unwrap(),
        other => panic!("expected records, found {other:?}"),
    }
}

fn run_read<'a>(
    stage: &'a dyn StageImpl,
    args: &'a BoundArgs,
    input: StageInput<'a>,
    ctx: &'a ExecCtx,
) -> Result<StageOutput<'a>, Error> {
    stage.as_read().unwrap().execute(args, input, ctx)
}

fn entry(path: &str, size: i64) -> Record {
    Record::new(RecordKind::Entry)
        .with("path", Value::Text(path.to_owned()))
        .with("size", Value::Int(size))
}

fn infos(records: Vec<Record>) -> Vec<FieldInfo> {
    records
        .into_iter()
        .map(|record| FieldInfo::try_from(record).unwrap())
        .collect()
}

#[test]
fn describe_lists_fields_in_order_of_first_occurrence_with_type_and_example() {
    let mut second = entry("/home/user/b.txt", 3);
    second.set("hash", Value::Text("abc".to_owned()));
    let input = StageInput::records(RecordStream::from_records(vec![
        entry("/home/user/a.txt", 12),
        second,
    ]));
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    let fields = infos(records(
        run_read(&DescribeStage::new(), &args, input, &ctx).unwrap(),
    ));
    let names: Vec<&str> = fields.iter().map(|field| field.field.as_str()).collect();
    assert_eq!(names, ["path", "size", "hash"]);
    assert_eq!(fields[0].kind, "text");
    assert_eq!(
        fields[0].example,
        Some(Value::Text("/home/user/a.txt".to_owned()))
    );
    assert_eq!(fields[1].kind, "int");
    assert_eq!(fields[1].required, Some(true));
    assert_eq!(fields[1].records, Some(2));
    assert_eq!(fields[2].required, Some(false));
    assert_eq!(fields[2].records, Some(1));
}

#[test]
fn describe_marks_a_field_with_values_of_different_types_as_mixed() {
    let a = Record::new(RecordKind::Item).with("value", Value::Int(1));
    let b = Record::new(RecordKind::Item).with("value", Value::Text("x".to_owned()));
    let input = StageInput::records(RecordStream::from_records(vec![a, b]));
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    let fields = infos(records(
        run_read(&DescribeStage::new(), &args, input, &ctx).unwrap(),
    ));
    assert_eq!(fields[0].kind, "mixed");
    assert_eq!(fields[0].example, Some(Value::Int(1)));
}

#[test]
fn describe_takes_the_type_and_example_of_the_first_value_that_is_not_null() {
    let a = Record::new(RecordKind::Item).with("note", Value::Null);
    let b = Record::new(RecordKind::Item).with("note", Value::Bool(true));
    let input = StageInput::records(RecordStream::from_records(vec![a, b]));
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    let fields = infos(records(
        run_read(&DescribeStage::new(), &args, input, &ctx).unwrap(),
    ));
    assert_eq!(fields[0].kind, "bool");
    assert_eq!(fields[0].example, Some(Value::Bool(true)));
}

#[test]
fn describe_of_empty_input_yields_no_records() {
    let input = StageInput::records(RecordStream::empty());
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    let stage = DescribeStage::new();
    let output = run_read(&stage, &args, input, &ctx).unwrap();
    assert!(records(output).is_empty());
}

#[test]
fn describe_is_deterministic() {
    let make = || {
        StageInput::records(RecordStream::from_records(vec![
            entry("/home/user/a.txt", 1),
            entry("/home/user/b.txt", 2),
        ]))
    };
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    let first = records(run_read(&DescribeStage::new(), &args, make(), &ctx).unwrap());
    let second = records(run_read(&DescribeStage::new(), &args, make(), &ctx).unwrap());
    assert_eq!(first, second);
}

#[test]
fn describe_rejects_input_that_is_not_records() {
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    for input in [StageInput::none(), StageInput::value(Value::Int(3))] {
        let error = run_read(&DescribeStage::new(), &args, input, &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }
}

#[test]
fn describe_propagates_an_error_of_the_input_stream() {
    let failing = RecordStream::new(
        vec![
            Ok(entry("/home/user/a.txt", 1)),
            Err(Error::new(ErrorCode::Io, "read failed")),
        ]
        .into_iter(),
    );
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("describe");
    let error = run_read(
        &DescribeStage::new(),
        &args,
        StageInput::records(failing),
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Io);
}

#[test]
fn evaluate_returns_the_value_and_its_type_from_the_evaluator() {
    let ctx = ctx(true, RequestOptions::default());
    let args = BoundArgs::new("evaluate").with(
        "expression",
        ArgValue::Expr(BoundExpr::Int { v: 42 }),
        false,
    );
    let stage = EvaluateStage::new();
    let output = run_read(&stage, &args, StageInput::none(), &ctx).unwrap();
    let rows = records(output);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, RecordKind::Value);
    let result = Evaluation::try_from(rows[0].clone()).unwrap();
    assert_eq!(result.value, Value::Int(42));
    assert_eq!(result.kind, "int");
}

#[test]
fn evaluate_reports_the_type_of_a_text_and_of_a_boolean() {
    let ctx = ctx(true, RequestOptions::default());
    for (expr, kind) in [
        (BoundExpr::Text { v: "x".to_owned() }, "text"),
        (BoundExpr::Bool { v: true }, "bool"),
    ] {
        let args = BoundArgs::new("evaluate").with("expression", ArgValue::Expr(expr), false);
        let rows =
            records(run_read(&EvaluateStage::new(), &args, StageInput::none(), &ctx).unwrap());
        assert_eq!(Evaluation::try_from(rows[0].clone()).unwrap().kind, kind);
    }
}

#[test]
fn evaluate_of_a_constant_argument_needs_no_evaluator() {
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("evaluate").with("expression", ArgValue::Value(Value::Int(7)), false);
    let rows = records(run_read(&EvaluateStage::new(), &args, StageInput::none(), &ctx).unwrap());
    assert_eq!(
        Evaluation::try_from(rows[0].clone()).unwrap().value,
        Value::Int(7)
    );
}

#[test]
fn evaluate_without_an_evaluator_fails_instead_of_computing_itself() {
    let ctx = ctx(false, RequestOptions::default());
    let args = BoundArgs::new("evaluate").with(
        "expression",
        ArgValue::Expr(BoundExpr::Int { v: 1 }),
        false,
    );
    let error = run_read(&EvaluateStage::new(), &args, StageInput::none(), &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
}

#[test]
fn evaluate_without_an_expression_is_a_missing_parameter() {
    let ctx = ctx(true, RequestOptions::default());
    let args = BoundArgs::new("evaluate").with("expression", ArgValue::Absent, false);
    let error = run_read(&EvaluateStage::new(), &args, StageInput::none(), &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::MissingParam);
}

#[test]
fn evaluate_passes_the_error_of_the_evaluator() {
    let ctx = ctx(true, RequestOptions::default());
    let args =
        BoundArgs::new("evaluate").with("expression", ArgValue::Expr(BoundExpr::Null), false);
    let error = run_read(&EvaluateStage::new(), &args, StageInput::none(), &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Type);
}

fn help_args(topic: &str) -> BoundArgs {
    BoundArgs::new("help")
        .with(
            "topic",
            ArgValue::Value(Value::Text(topic.to_owned())),
            false,
        )
        .with(
            "kind",
            ArgValue::Value(Value::Enum("command".to_owned())),
            true,
        )
        .with("examples", ArgValue::Value(Value::Int(3)), true)
}

fn help_text(args: &BoundArgs, options: RequestOptions) -> Result<String, Error> {
    let stage = HelpStage::new(Arc::new(common::catalog()));
    let ctx = ctx(false, options);
    match run_read(&stage, args, StageInput::none(), &ctx)?.data {
        StageData::Value(Value::Text(text)) => Ok(text),
        other => panic!("expected text, found {other:?}"),
    }
}

#[test]
fn help_stage_returns_the_answer_as_text() {
    let text = help_text(&help_args("list"), RequestOptions::default()).unwrap();
    assert!(text.starts_with("list — "));
}

#[test]
fn help_stage_without_arguments_returns_the_overview() {
    let args = BoundArgs::new("help").with(
        "kind",
        ArgValue::Value(Value::Enum("command".to_owned())),
        true,
    );
    let text = help_text(&args, RequestOptions::default()).unwrap();
    assert!(text.starts_with("Danaco Nurt "));
}

#[test]
fn help_stage_uses_the_kind_given_by_the_call_and_ignores_the_default() {
    let args = BoundArgs::new("help")
        .with(
            "topic",
            ArgValue::Value(Value::Text("verify".to_owned())),
            false,
        )
        .with(
            "kind",
            ArgValue::Value(Value::Enum("group".to_owned())),
            false,
        );
    let text = help_text(&args, RequestOptions::default()).unwrap();
    assert!(text.starts_with("GROUP verify"));
}

#[test]
fn help_stage_returns_json_when_the_call_asks_for_it() {
    let options = RequestOptions {
        format: Some(OutputFormat::Json),
        ..RequestOptions::default()
    };
    let text = help_text(&help_args("list"), options).unwrap();
    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["command"], "list");
}

#[test]
fn help_stage_applies_the_example_count() {
    let args = BoundArgs::new("help")
        .with(
            "topic",
            ArgValue::Value(Value::Text("evaluate".to_owned())),
            false,
        )
        .with("examples", ArgValue::Value(Value::Int(1)), false);
    let text = help_text(&args, RequestOptions::default()).unwrap();
    assert!(text.contains("evaluate 6 * 7"));
    assert!(!text.contains("Release"));
}

#[test]
fn help_stage_reports_an_unknown_topic() {
    let error = help_text(&help_args("remvoe"), RequestOptions::default()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnknownName);
}

#[test]
fn stages_declare_their_commands_and_input() {
    let help = HelpStage::new(Arc::new(common::catalog())).descriptor();
    assert_eq!(
        (help.command.as_str(), help.source, help.input),
        ("help", true, InputKinds::None)
    );
    let describe = DescribeStage::new().descriptor();
    assert_eq!(
        (describe.command.as_str(), describe.source, describe.input),
        ("describe", false, InputKinds::Any)
    );
    let evaluate = EvaluateStage::new().descriptor();
    assert_eq!(
        (evaluate.command.as_str(), evaluate.source, evaluate.input),
        ("evaluate", true, InputKinds::None)
    );
}

#[test]
fn stage_descriptors_agree_with_the_catalog() {
    let catalog = common::catalog();
    for descriptor in [
        HelpStage::new(Arc::new(common::catalog())).descriptor(),
        DescribeStage::new().descriptor(),
        EvaluateStage::new().descriptor(),
    ] {
        let spec = catalog.command(&descriptor.command).unwrap();
        assert_eq!(
            descriptor.source,
            spec.input.iter().any(nurt_contracts::spec::IoSpec::is_none)
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
