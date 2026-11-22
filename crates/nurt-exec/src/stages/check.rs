// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The stages `assert` and `verify`.

use std::collections::BTreeSet;

use nurt_contracts::bound::{BoundExpr, BoundPipeline};
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageData,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Range, Record, RecordKind, Value};
use serde_json::{Value as Json, json};

use super::common::{
    PATH_FIELD, Parts, SAMPLE_SIZE, descriptor, is_present, known_value, missing_param,
    sample_item, truth, type_error,
};

const ASSERT: &str = "assert";
const CONDITIONS: [&str; 8] = [
    "count",
    "paths",
    "exit_code",
    "all",
    "any",
    "empty",
    "contains",
    "if_match",
];
const EXIT_CODE_FIELD: &str = "exit_code";
const ETAG_FIELD: &str = "etag";
const COMMAND_FIELD: &str = "command";
const AT_LEAST_ONE: &str = "1..";

/// The single condition of an `assert` stage.
#[derive(Debug, Clone, PartialEq)]
enum Condition<'a> {
    Count(Range),
    Paths(Range),
    ExitCode(Range),
    All,
    Any,
    Empty(bool),
    Contains(&'a str),
    IfMatch(&'a str),
}

/// A condition that does not hold.
struct Failure {
    message: String,
    expected: Json,
    actual: Json,
    sample: Vec<String>,
}

fn given(args: &BoundArgs) -> Vec<&'static str> {
    CONDITIONS
        .into_iter()
        .filter(|name| args.is_given(name) && is_present(args, name))
        .collect()
}

fn condition(args: &BoundArgs) -> Result<(&'static str, Condition<'_>)> {
    match given(args).as_slice() {
        [] => Err(Error::new(
            ErrorCode::MissingParam,
            "`assert` needs exactly one condition: count, paths, exit_code, all, any, empty, \
             contains or if_match",
        )
        .with_stage(Stage::Bind)
        .with_expected(json!(CONDITIONS))
        .with_help(ASSERT)),
        [name] => Ok((name, parse_condition(args, name)?)),
        several => Err(conflict(args, several)),
    }
}

fn conflict(args: &BoundArgs, names: &[&str]) -> Error {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    let mut error = Error::new(
        ErrorCode::ParamConflict,
        format!(
            "`assert` takes exactly one condition, found {}; keep one of them",
            quoted.join(", ")
        ),
    )
    .with_stage(Stage::Bind)
    .with_expected(json!(CONDITIONS))
    .with_found(names.join(", "))
    .with_help(ASSERT);
    for name in names {
        if let Some(text) = known_value(args, name).and_then(literal) {
            error = error.with_fix(format!("assert {name}: {text}"));
        }
    }
    error
}

/// Program text of a value given to a condition, when it is a literal.
fn literal(value: &Value) -> Option<String> {
    match value {
        Value::Range(range) => Some(range.to_string()),
        Value::Int(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Text(text) => serde_json::to_string(text).ok(),
        _ => None,
    }
}

fn parse_condition<'a>(args: &'a BoundArgs, name: &str) -> Result<Condition<'a>> {
    match name {
        "count" => range(args, name).map(Condition::Count),
        "paths" => range(args, name).map(Condition::Paths),
        "exit_code" => range(args, name).map(Condition::ExitCode),
        "all" => Ok(Condition::All),
        "any" => Ok(Condition::Any),
        "empty" => match known_value(args, name) {
            Some(Value::Bool(flag)) => Ok(Condition::Empty(*flag)),
            other => Err(type_error(args, name, &["bool"], found_type(other))),
        },
        "contains" => text(args, name).map(Condition::Contains),
        _ => text(args, name).map(Condition::IfMatch),
    }
}

fn found_type(value: Option<&Value>) -> &'static str {
    value.map_or("expression", Value::type_name)
}

fn range(args: &BoundArgs, name: &str) -> Result<Range> {
    match known_value(args, name) {
        Some(Value::Range(range)) => Ok(*range),
        Some(Value::Int(number)) => Ok(Range::single(*number)),
        other => Err(type_error(args, name, &["range"], found_type(other))),
    }
}

fn text<'a>(args: &'a BoundArgs, name: &str) -> Result<&'a str> {
    match known_value(args, name) {
        Some(Value::Text(text)) => Ok(text),
        other => Err(type_error(args, name, &["text"], found_type(other))),
    }
}

fn range_json(range: Range) -> Json {
    match (range.start(), range.end()) {
        (Some(start), Some(end)) if start == end => json!(start),
        _ => json!(range.to_string()),
    }
}

fn len_i64(len: usize) -> i64 {
    i64::try_from(len).unwrap_or(i64::MAX)
}

fn sample<'a>(records: impl IntoIterator<Item = &'a Record>) -> Vec<String> {
    records
        .into_iter()
        .take(SAMPLE_SIZE)
        .map(sample_item)
        .collect()
}

fn check_count(range: Range, records: &[Record]) -> Option<Failure> {
    let found = len_i64(records.len());
    (!range.contains(found)).then(|| Failure {
        message: format!("expected {range} records, found {found}"),
        expected: range_json(range),
        actual: json!(found),
        sample: sample(records),
    })
}

fn check_paths(range: Range, records: &[Record]) -> Option<Failure> {
    let paths: BTreeSet<&str> = records
        .iter()
        .filter_map(|record| record.get(PATH_FIELD).and_then(Value::as_str))
        .collect();
    let found = len_i64(paths.len());
    (!range.contains(found)).then(|| Failure {
        message: format!("expected {range} distinct paths, found {found}"),
        expected: range_json(range),
        actual: json!(found),
        sample: paths
            .into_iter()
            .take(SAMPLE_SIZE)
            .map(str::to_owned)
            .collect(),
    })
}

fn exit_code_of(record: &Record) -> Option<i64> {
    record.get(EXIT_CODE_FIELD).and_then(Value::as_int)
}

fn command_text(record: &Record) -> String {
    match record.get(COMMAND_FIELD) {
        Some(Value::List(words)) => words
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => sample_item(record),
    }
}

fn check_exit_code(range: Range, records: &[Record]) -> Option<Failure> {
    if records.is_empty() {
        return Some(Failure {
            message: format!("expected exit code {range}, but no program ran"),
            expected: range_json(range),
            actual: Json::Null,
            sample: Vec::new(),
        });
    }
    let failing: Vec<&Record> = records
        .iter()
        .filter(|record| !exit_code_of(record).is_some_and(|code| range.contains(code)))
        .collect();
    if failing.is_empty() {
        return None;
    }
    let codes: Vec<Json> = failing
        .iter()
        .map(|record| exit_code_of(record).map_or(Json::Null, |code| json!(code)))
        .collect();
    let shown: Vec<String> = codes.iter().map(Json::to_string).collect();
    let actual = match codes.as_slice() {
        [single] => single.clone(),
        _ => Json::Array(codes),
    };
    Some(Failure {
        message: format!("expected exit code {range}, found {}", shown.join(", ")),
        expected: range_json(range),
        actual,
        sample: failing
            .into_iter()
            .take(SAMPLE_SIZE)
            .map(command_text)
            .collect(),
    })
}

fn meets(args: &BoundArgs, name: &str, record: &Record, ctx: &ExecCtx) -> Result<bool> {
    let value = args.eval(name, Some(record), ctx)?.unwrap_or_default();
    truth(args, name, &value)
}

fn check_all(args: &BoundArgs, records: &[Record], ctx: &ExecCtx) -> Result<Option<Failure>> {
    let mut failing = Vec::new();
    for record in records {
        if !meets(args, "all", record, ctx)? {
            failing.push(record);
        }
    }
    if failing.is_empty() {
        return Ok(None);
    }
    let total = records.len();
    Ok(Some(Failure {
        message: format!(
            "{} of {total} records do not meet the condition",
            failing.len()
        ),
        expected: json!(total),
        actual: json!(total - failing.len()),
        sample: sample(failing),
    }))
}

fn check_any(args: &BoundArgs, records: &[Record], ctx: &ExecCtx) -> Result<Option<Failure>> {
    for record in records {
        if meets(args, "any", record, ctx)? {
            return Ok(None);
        }
    }
    Ok(Some(Failure {
        message: format!("none of the {} records meets the condition", records.len()),
        expected: json!(AT_LEAST_ONE),
        actual: json!(0),
        sample: sample(records),
    }))
}

fn check_empty(expected: bool, records: &[Record]) -> Option<Failure> {
    let found = records.len();
    match (expected, found) {
        (true, 0) | (false, 1..) => None,
        (true, _) => Some(Failure {
            message: format!("expected no records, found {found}"),
            expected: json!(0),
            actual: json!(found),
            sample: sample(records),
        }),
        (false, _) => Some(Failure {
            message: "expected at least one record, found none".to_owned(),
            expected: json!(AT_LEAST_ONE),
            actual: json!(0),
            sample: Vec::new(),
        }),
    }
}

fn value_contains(value: &Value, text: &str) -> bool {
    match value {
        Value::List(items) => items.iter().any(|item| value_contains(item, text)),
        other => other.as_str().is_some_and(|own| own.contains(text)),
    }
}

fn check_contains(text: &str, records: &[Record]) -> Option<Failure> {
    let found = records.iter().any(|record| {
        record
            .fields
            .values()
            .any(|value| value_contains(value, text))
    });
    (!found).then(|| Failure {
        message: format!("none of the {} records contains `{text}`", records.len()),
        expected: json!(text),
        actual: json!(0),
        sample: sample(records),
    })
}

fn etag_of(record: &Record) -> Option<&str> {
    record.get(ETAG_FIELD).and_then(Value::as_str)
}

fn check_if_match(etag: &str, records: &[Record]) -> Option<Failure> {
    if records.is_empty() {
        return Some(Failure {
            message: format!("expected etag `{etag}`, but there is no input file"),
            expected: json!(etag),
            actual: Json::Null,
            sample: Vec::new(),
        });
    }
    let differing: Vec<&Record> = records
        .iter()
        .filter(|record| etag_of(record) != Some(etag))
        .collect();
    let first = differing.first()?;
    let found = etag_of(first);
    Some(Failure {
        message: format!(
            "expected etag `{etag}`, found `{}`; read the file again",
            found.unwrap_or("none")
        ),
        expected: json!(etag),
        actual: found.map_or(Json::Null, |found| json!(found)),
        sample: sample(differing),
    })
}

fn evaluate(
    condition: &Condition<'_>,
    args: &BoundArgs,
    records: &[Record],
    ctx: &ExecCtx,
) -> Result<Option<Failure>> {
    Ok(match condition {
        Condition::Count(range) => check_count(*range, records),
        Condition::Paths(range) => check_paths(*range, records),
        Condition::ExitCode(range) => check_exit_code(*range, records),
        Condition::All => check_all(args, records, ctx)?,
        Condition::Any => check_any(args, records, ctx)?,
        Condition::Empty(expected) => check_empty(*expected, records),
        Condition::Contains(text) => check_contains(text, records),
        Condition::IfMatch(etag) => check_if_match(etag, records),
    })
}

fn assert_error(failure: Failure, name: &str, ctx: &ExecCtx) -> Error {
    let stage = if ctx.step() == Stage::After {
        Stage::After
    } else {
        Stage::Check
    };
    let mut error = Error::new(ErrorCode::Assert, failure.message)
        .with_stage(stage)
        .with_expected(failure.expected)
        .with_actual(failure.actual)
        .with_detail("condition", json!(name))
        .with_help(ASSERT);
    error.sample = failure.sample;
    error
}

/// `assert`: stops the program with `E_ASSERT` unless the records meet
/// exactly one condition.
///
/// The conditions are `count: range` (number of records), `paths: range`
/// (number of distinct `path` values), `exit_code: range` (every record of a
/// program run has an exit code in the range), `all: expression`,
/// `any: expression`, `empty: bool`, `contains: text` (a text field of a
/// record contains the text) and `if_match: text` (every input record has
/// this `etag`). No condition fails with `E_MISSING_PARAM`, several with
/// `E_PARAM_CONFLICT`. A failed condition is `E_ASSERT` with `expected`,
/// `actual` and a sample of up to five records, in the stage `3` before an
/// effect and `5a` after it. The stage reads the whole input and passes it
/// on unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct Assert;

impl StageImpl for Assert {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(ASSERT, InputKinds::Any)
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        condition(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Assert {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let (name, condition) = condition(args)?;
        let (records, description) = Parts::of(input, ctx)?.collect(ctx)?;
        if let Some(failure) = evaluate(&condition, args, &records, ctx)? {
            return Err(assert_error(failure, name, ctx));
        }
        Ok(description.output(records))
    }
}

const VERIFY: &str = "verify";
const CHECK: &str = "check";

fn check_pipeline(args: &BoundArgs) -> Result<&BoundPipeline> {
    match args.get(CHECK) {
        Some(ArgValue::Pipeline(pipeline)) => Ok(pipeline),
        Some(ArgValue::Expr(BoundExpr::Sub { p })) => Ok(p),
        Some(ArgValue::Value(Value::Null) | ArgValue::Absent) | None => {
            Err(missing_param(args, CHECK))
        }
        Some(ArgValue::Value(value)) => {
            Err(type_error(args, CHECK, &["pipeline"], value.type_name()))
        }
        Some(_) => Err(type_error(args, CHECK, &["pipeline"], "expression")),
    }
}

fn drain(output: StageOutput<'_>) -> Result<()> {
    if let StageData::Records(records) = output.data {
        for item in records {
            item?;
        }
    }
    Ok(())
}

/// `verify check`: checks the result of an effect with a pipeline.
///
/// The stage runs the check pipeline through the pipeline runner of the
/// context, with the `effect` records as its input, and reads the whole
/// output of the check so that every error surfaces. On success the
/// `effect` records pass on unchanged; on failure the error of the check is
/// returned, and the executor rolls the effect back and reports
/// `E_ROLLED_BACK` or `E_ROLLBACK_FAILED`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Verify;

impl StageImpl for Verify {
    fn descriptor(&self) -> StageDescriptor {
        descriptor(VERIFY, InputKinds::Only(vec![RecordKind::Effect]))
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        check_pipeline(args)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Verify {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let pipeline = check_pipeline(args)?;
        let runner = ctx.pipelines()?;
        let (records, description) = Parts::of(input, ctx)?.collect(ctx)?;
        let check_input = StageInput {
            data: StageData::Records(RecordStream::from_records(records.clone())),
            header: description.header.clone(),
            order: description.order.clone(),
            render: None,
        };
        drain(runner.run(pipeline, check_input, ctx)?)?;
        Ok(description.output(records))
    }
}
