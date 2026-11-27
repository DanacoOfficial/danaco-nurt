// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/fixtures/stages/harness.rs
// Responsibility:   Harness of the record stage tests: an execution context with the.
// Dependencies:     nurt_contracts, nurt_testkit, nurt_types, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Harness of the record stage tests: an execution context with the
//! expression evaluator of `nurt-types`, builders of arguments and records,
//! a scripted pipeline runner and the runner of JSON cases shared by the
//! conformance and documentation tests.
//!
//! A JSON case is an object with the members `command`, `args` (parameter
//! name to the JSON form of `ArgValue`), `defaulted` (names of parameters
//! filled from the catalog default), `step` (execution step, `"2"` when
//! omitted), `max_records`, `check` (outcome of the pipeline run by `verify`:
//! `{"records": […]}` or `{"error": {"code": …, "message": …}}`) and
//! `input` (records in the JSON form of `Record`). The result is
//! `{"records": […]}`, `{"value": …}` or `{"error": {…}}` with the code,
//! stage, `expected`, `actual` and `sample` of the error.

// Each test binary uses a different part of the harness.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nurt_contracts::ast::BinOp;
use nurt_contracts::bound::{BoundExpr, BoundPipeline};
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, ExecCtx, ExprEvaluator, Limits, PipelineRunner, RecordStream, StageData,
    StageFrame, StageImpl, StageInput, StageOutput,
};
use nurt_contracts::value::{Instant, Path, Record, RecordKind, Size, Value};
use nurt_testkit::ctx::TestContext;
use nurt_types::{Evaluator, FixedEnv};
use serde_json::{Value as Json, json};

/// Evaluator of the context: `nurt-types` on a fixed clock.
pub(crate) struct TypesEvaluator {
    evaluator: Evaluator,
    env: FixedEnv,
}

impl ExprEvaluator for TypesEvaluator {
    fn eval(&self, expr: &BoundExpr, record: Option<&Record>, _ctx: &ExecCtx) -> Result<Value> {
        self.evaluator.eval(expr, record, &self.env)
    }
}

/// Pipeline runner that returns a fixed outcome and counts its calls.
pub(crate) struct ScriptedRunner {
    outcome: Result<Vec<Record>>,
    calls: AtomicUsize,
}

impl ScriptedRunner {
    pub(crate) fn new(outcome: Result<Vec<Record>>) -> Arc<Self> {
        Arc::new(Self {
            outcome,
            calls: AtomicUsize::new(0),
        })
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PipelineRunner for ScriptedRunner {
    fn run<'a>(
        &'a self,
        _pipeline: &'a BoundPipeline,
        input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        input.into_records()?.for_each(drop);
        match &self.outcome {
            Ok(records) => Ok(StageOutput::records(RecordStream::new(
                records.clone().into_iter().map(Ok),
            ))),
            Err(error) => Ok(StageOutput::records(RecordStream::failed(error.clone()))),
        }
    }
}

/// Options of a test context.
#[derive(Default)]
pub(crate) struct Setup {
    pub(crate) step: Option<Stage>,
    pub(crate) max_records: Option<u64>,
    pub(crate) runner: Option<Arc<ScriptedRunner>>,
}

/// Execution context of the given step with the evaluator.
pub(crate) fn ctx(step: Stage) -> ExecCtx {
    context(Setup {
        step: Some(step),
        ..Setup::default()
    })
}

/// Execution context from options.
pub(crate) fn context(setup: Setup) -> ExecCtx {
    let evaluator = TypesEvaluator {
        evaluator: Evaluator::new(),
        env: FixedEnv::new(Instant::UNIX_EPOCH, 0),
    };
    let mut builder = TestContext::new()
        .builder()
        .evaluator(Arc::new(evaluator))
        .frame(StageFrame {
            step: setup.step.unwrap_or(Stage::Read),
            ..StageFrame::default()
        });
    if let Some(max_records) = setup.max_records {
        builder = builder.limits(Limits {
            max_records,
            ..Limits::default()
        });
    }
    if let Some(runner) = setup.runner {
        builder = builder.pipelines(runner);
    }
    builder.build()
}

/// Output of a stage, read to the end.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Records(Vec<Record>),
    Value(Value),
}

impl Outcome {
    pub(crate) fn records(self) -> Vec<Record> {
        match self {
            Self::Records(records) => records,
            Self::Value(value) => panic!("expected records, found the value {value:?}"),
        }
    }
}

/// Reads an output to the end.
pub(crate) fn read(output: StageOutput<'_>) -> Result<Outcome> {
    match output.data {
        StageData::Records(records) => records.collect::<Result<Vec<_>>>().map(Outcome::Records),
        StageData::Value(value) => Ok(Outcome::Value(value)),
        _ => Ok(Outcome::Records(Vec::new())),
    }
}

/// Runs a stage on records held in memory.
pub(crate) fn run(
    stage: &dyn StageImpl,
    args: &BoundArgs,
    input: Vec<Record>,
    ctx: &ExecCtx,
) -> Result<Outcome> {
    let read_stage = stage
        .as_read()
        .expect("record stages have a read interface");
    read(read_stage.execute(
        args,
        StageInput::records(RecordStream::from_records(input)),
        ctx,
    )?)
}

/// Runs a stage in the read step and returns its records.
pub(crate) fn records_of(
    stage: &dyn StageImpl,
    args: &BoundArgs,
    input: Vec<Record>,
) -> Vec<Record> {
    run(stage, args, input, &ctx(Stage::Read))
        .expect("the stage succeeds")
        .records()
}

/// Runs a stage in the read step and returns its error.
pub(crate) fn error_of(stage: &dyn StageImpl, args: &BoundArgs, input: Vec<Record>) -> Error {
    run(stage, args, input, &ctx(Stage::Read)).expect_err("the stage fails")
}

/// Stream that counts how many records were pulled from it.
pub(crate) fn counted<'a>(records: Vec<Record>, pulled: &'a AtomicUsize) -> RecordStream<'a> {
    RecordStream::new(records.into_iter().map(move |record| {
        pulled.fetch_add(1, Ordering::SeqCst);
        Ok(record)
    }))
}

/// Arguments of a command.
pub(crate) fn args(command: &str, values: &[(&str, ArgValue)]) -> BoundArgs {
    values
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, value.clone(), false)
        })
}

/// Argument known before the stage runs.
pub(crate) fn value(value: impl Into<Value>) -> ArgValue {
    ArgValue::Value(value.into())
}

/// Per-record expression argument.
pub(crate) fn expr(expr: BoundExpr) -> ArgValue {
    ArgValue::Expr(expr)
}

/// Expression argument listing fields.
pub(crate) fn fields(names: &[&str]) -> ArgValue {
    ArgValue::Expr(BoundExpr::List {
        items: names.iter().map(|name| field(name)).collect(),
    })
}

/// Read of a field of the current record.
pub(crate) fn field(name: &str) -> BoundExpr {
    BoundExpr::Field {
        name: name.to_owned(),
        of: None,
    }
}

/// Binary operation.
pub(crate) fn bin(op: BinOp, left: BoundExpr, right: BoundExpr) -> BoundExpr {
    BoundExpr::Bin {
        op,
        l: Box::new(left),
        r: Box::new(right),
    }
}

/// Integer literal.
pub(crate) fn int(value: i64) -> BoundExpr {
    BoundExpr::Int { v: value }
}

/// File entry with a path and a size in bytes.
pub(crate) fn entry(path: &str, size: u64) -> Record {
    let name = path.rsplit('/').next().unwrap_or(path);
    let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
    Record::new(RecordKind::Entry)
        .with("path", Value::Path(path.to_owned()))
        .with("name", Value::from(name))
        .with("extension", Value::from(extension))
        .with("size", Value::Size(Size::from_bytes(size)))
}

/// Line of a file.
pub(crate) fn line(path: &str, no: i64, text: &str) -> Record {
    Record::new(RecordKind::Line)
        .with("path", Value::Path(path.to_owned()))
        .with("no", Value::Int(no))
        .with("text", Value::from(text))
}

/// Result of a program run.
pub(crate) fn exec(command: &[&str], exit_code: i64) -> Record {
    Record::new(RecordKind::Exec)
        .with(
            "command",
            Value::List(command.iter().map(|word| Value::from(*word)).collect()),
        )
        .with("exit_code", Value::Int(exit_code))
}

/// Effect record of a change of one file.
pub(crate) fn effect(path: &str) -> Record {
    Record::new(RecordKind::Effect).with("path", Value::Path(Path::from(path).into_string()))
}

/// The JSON form of an error compared by the cases.
pub(crate) fn error_json(error: &Error) -> Json {
    let mut fields = serde_json::Map::new();
    fields.insert("code".to_owned(), json!(error.code));
    if let Some(stage) = error.stage() {
        fields.insert("stage".to_owned(), json!(stage));
    }
    for (name, member) in [("expected", &error.expected), ("actual", &error.actual)] {
        if let Some(member) = member {
            fields.insert(name.to_owned(), member.clone());
        }
    }
    if !error.sample.is_empty() {
        fields.insert("sample".to_owned(), json!(error.sample));
    }
    json!({ "error": Json::Object(fields) })
}

fn case_args(command: &str, case: &Json) -> BoundArgs {
    let defaulted: Vec<&str> = case["defaulted"]
        .as_array()
        .map(|names| names.iter().filter_map(Json::as_str).collect())
        .unwrap_or_default();
    let mut args = BoundArgs::new(command);
    if let Some(values) = case["args"].as_object() {
        for (name, value) in values {
            let value: ArgValue =
                serde_json::from_value(value.clone()).expect("argument in the form of ArgValue");
            args.insert(name.clone(), value, defaulted.contains(&name.as_str()));
        }
    }
    args
}

fn case_runner(case: &Json) -> Option<Arc<ScriptedRunner>> {
    let check = case.get("check")?;
    let outcome = match check.get("error") {
        Some(error) => Err(Error::new(
            serde_json::from_value(error["code"].clone()).expect("error code"),
            error["message"].as_str().unwrap_or_default(),
        )),
        None => Ok(serde_json::from_value(check["records"].clone()).expect("records")),
    };
    Some(ScriptedRunner::new(outcome))
}

/// Runs a JSON case and returns its JSON result.
pub(crate) fn run_case(case: &Json) -> Json {
    let command = case["command"].as_str().expect("case names its command");
    let stage = nurt_exec::stages::implementations()
        .into_iter()
        .find(|stage| stage.descriptor().command == command)
        .expect("command of the record stages");
    let step = case["step"]
        .as_str()
        .map_or(Stage::Read, |step| step.parse().expect("execution step"));
    let ctx = context(Setup {
        step: Some(step),
        max_records: case["max_records"].as_u64(),
        runner: case_runner(case),
    });
    let input: Vec<Record> =
        serde_json::from_value(case["input"].clone()).expect("input records in the form of Record");
    let args = case_args(command, case);
    match run(stage.as_ref(), &args, input, &ctx) {
        Ok(Outcome::Records(records)) => json!({ "records": records }),
        Ok(Outcome::Value(value)) => json!({ "value": value }),
        Err(error) => error_json(&error),
    }
}

/// Code of an error, for assertions.
pub(crate) fn code(error: &Error) -> ErrorCode {
    error.code()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
