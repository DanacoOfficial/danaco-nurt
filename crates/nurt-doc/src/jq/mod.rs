// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Evaluation of `jq` filters on document values.
//!
//! A [`Query`] is compiled once and run on any number of values. Every result
//! of a run carries the path of the value in the input when the filter is a
//! path expression (`.a.b`, `.[]`, `.. | select(…)`), so callers can report
//! a key path for each result; results computed by other filters (`.a + 1`,
//! `{x: .a}`) carry no path.
//!
//! The filter language is the one of `jq` with its standard library. Filters
//! reach the world only through the execution context: `$ENV` and `env` read
//! the environment of the host, `now` reads the context clock, `debug` and
//! `stderr` emit log events, and `input`, `inputs` and modules are not
//! available.

mod convert;
mod guard;
mod runtime;

use std::fmt;

use jaq_core::load::{Arena, File, Loader, lex, parse};
use jaq_core::{Ctx, Exn, Vars};
use jaq_json::Val;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::Value;

pub use convert::value_type;
use convert::{from_val, key_text, to_val};
use runtime::{Doc, ENV_VAR, Runtime, is_stop_code};

/// Longest fragment of the filter quoted in an error.
const MAX_FOUND_CHARS: usize = 40;

/// A compiled `jq` filter.
///
/// # Examples
///
/// ```
/// use nurt_doc::jq::Query;
///
/// let query = Query::compile(".scripts | keys")?;
/// assert_eq!(query.expression(), ".scripts | keys");
/// assert!(Query::compile(".a | (").is_err());
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub struct Query {
    expression: String,
    filter: jaq_core::Filter<Doc>,
    /// Whether the filter defines filters of its own and carries a guard
    /// against unbounded recursion in every definition; such a filter runs
    /// on a thread with a stack of known size.
    guarded: bool,
}

/// One result of a filter.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryOutput {
    /// The value.
    pub value: Value,
    /// Path of the value in the input, when the filter is a path expression.
    pub path: Option<Vec<PathStep>>,
}

/// One step of the path of a result.
#[derive(Debug, Clone, PartialEq)]
pub enum PathStep {
    /// Member of an object.
    Key(String),
    /// Element of an array; negative values count from the end.
    Index(i64),
    /// A step without a key path counterpart, such as the slice of `.[1:3]`.
    Other(Value),
}

/// Results of one run of a filter.
///
/// A filter may produce results before it fails, as `.[] | .a` does on
/// `[{"a": 1}, "x"]`; the results come first and the error ends them.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryRun {
    /// Results in the order the filter produced them.
    pub outputs: Vec<QueryOutput>,
    /// Error that ended the run.
    pub error: Option<Error>,
}

impl QueryRun {
    /// The results, or the error when the run failed.
    ///
    /// # Errors
    ///
    /// Returns the error that ended the run.
    pub fn into_result(self) -> Result<Vec<QueryOutput>> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.outputs),
        }
    }
}

enum Outcome {
    Halted,
    Failed(jaq_core::Error<Val>),
    Fatal(Error),
}

type RawOutput = (Val, Option<Vec<Val>>);

/// Result of running a filter as a path expression.
enum PathRun {
    /// Every result, with its path.
    Done(Vec<RawOutput>),
    /// The results before the error that ended the run.
    Failed(Vec<RawOutput>, Error),
    /// The filter is not a path expression.
    NotPath,
}

/// Start of the message of the error of a non-path filter run as a path
/// expression.
const PATH_REFUSAL: &str = "invalid path expression";

fn is_path_refusal(error: &jaq_core::Error<Val>) -> bool {
    error.to_string().starts_with(PATH_REFUSAL)
}

/// Compiles `source`, which is `expression` or `expression` with guards;
/// errors quote `expression`.
fn compile_source(expression: &str, source: &str) -> Result<jaq_core::Filter<Doc>> {
    let prelude = runtime::prelude().ok_or_else(|| {
        Error::new(
            ErrorCode::StageFailed,
            "the built-in query definitions do not parse",
        )
    })?;
    let arena = Arena::default();
    let file = File {
        code: source,
        path: (),
    };
    let modules = Loader::new(prelude)
        .load(&arena, file)
        .map_err(|errors| load_error(expression, errors))?;
    jaq_core::Compiler::default()
        .with_funs(runtime::funs())
        .with_global_vars([ENV_VAR])
        .compile(modules)
        .map_err(|errors| compile_error(expression, errors))
}

impl Query {
    /// Compiles a filter in `jq` syntax.
    ///
    /// # Errors
    ///
    /// Returns `E_QUERY` when the filter does not parse, calls an undefined
    /// filter or variable, or imports a module. `found` holds the fragment
    /// and the details `line`, `column` and `offset` its position in the
    /// filter.
    pub fn compile(expression: &str) -> Result<Self> {
        let filter = compile_source(expression, expression)?;
        // The filter as written is compiled first, so that errors name the
        // text of the user; a filter with definitions is compiled again with
        // the guards in them, which cannot fail where this compilation passed.
        if let Some(source) = guard::guarded(expression)
            && let Ok(guarded) = compile_source(expression, &source)
        {
            return Ok(Self {
                expression: expression.to_owned(),
                filter: guarded,
                guarded: true,
            });
        }
        Ok(Self {
            expression: expression.to_owned(),
            filter,
            guarded: false,
        })
    }

    /// The filter as written.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Runs the filter on `input`.
    ///
    /// The run stops with `E_CANCELLED` or `E_TIMEOUT` when the execution is
    /// cancelled or its deadline passes, and with `E_LIMIT` when the filter
    /// produces more values than
    /// [`Limits::max_records`](nurt_contracts::stage::Limits::max_records).
    /// `halt` ends the results; `halt_error` and other exit codes than 0 end
    /// them with `E_QUERY`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::Arc;
    /// # use nurt_contracts::stage::ExecCtx;
    /// # fn run(ctx: &ExecCtx) -> nurt_contracts::error::Result<()> {
    /// use nurt_doc::jq::{PathStep, Query};
    /// use nurt_contracts::value::Value;
    ///
    /// let input = Value::List(vec![Value::Int(1), Value::Int(2)]);
    /// let outputs = Query::compile(".[1]")?.run(&input, ctx).into_result()?;
    /// assert_eq!(outputs[0].value, Value::Int(2));
    /// assert_eq!(outputs[0].path, Some(vec![PathStep::Index(1)]));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A filter that defines filters of its own and recurses without end
    /// stops with `E_LIMIT` when it has used the stack of its run, instead of
    /// ending the process.
    #[must_use]
    pub fn run(&self, input: &Value, ctx: &ExecCtx) -> QueryRun {
        if self.guarded {
            return guard::on_large_stack(|floor| self.run_above(input, ctx, floor));
        }
        self.run_above(input, ctx, 0)
    }

    /// Runs the filter; a guard stops it when the stack in use reaches the
    /// address `floor`, unless `floor` is 0.
    fn run_above(&self, input: &Value, ctx: &ExecCtx, floor: usize) -> QueryRun {
        let max_bytes = usize::try_from(ctx.limits().max_file_bytes_text).unwrap_or(usize::MAX);
        let (mut run, exceeded) = jaq_json::limit::with_repeat_limit(max_bytes, || {
            self.run_with_repeat_limit(input, ctx, floor)
        });
        if exceeded
            && run
                .error
                .as_ref()
                .is_none_or(|error| error.code == ErrorCode::Query)
        {
            run.error = Some(Error::new(
                ErrorCode::Limit,
                format!(
                    "the filter made a string longer than {max_bytes} bytes by multiplying a string; narrow the filter"
                ),
            ));
        }
        run
    }

    /// Runs the filter once the thread has the limit on repeated strings; a
    /// repetition above it is an error that `try` can catch, so the caller
    /// reads the outcome from the limit itself.
    fn run_with_repeat_limit(&self, input: &Value, ctx: &ExecCtx, floor: usize) -> QueryRun {
        let val = to_val(input);
        let (raw, error) = match self.run_paths(&val, ctx, floor) {
            PathRun::Done(raw) => (raw, None),
            PathRun::Failed(raw, error) => (raw, Some(error)),
            PathRun::NotPath => self.run_values(val, ctx, floor),
        };
        let outputs = raw
            .into_iter()
            .map(|(value, path)| output(input, &value, path))
            .collect();
        QueryRun { outputs, error }
    }

    /// Runs the filter as a path expression.
    ///
    /// A failure that is not the refusal of a non-path filter ends the run
    /// here, so that the filter is evaluated once; only a non-path filter is
    /// evaluated again for its values.
    fn run_paths(&self, input: &Val, ctx: &ExecCtx, floor: usize) -> PathRun {
        let runtime = Runtime::new(&self.filter.lut, ctx, true, floor);
        let vars = Vars::new([runtime.environment()]);
        let start = (
            Ctx::<Doc>::new(&runtime, vars),
            (input.clone(), Default::default()),
        );
        let mut outputs = Vec::new();
        for item in self.filter.id.paths(start) {
            match item {
                Ok((value, path)) => {
                    let mut steps: Vec<Val> = path.iter().cloned().collect();
                    steps.reverse();
                    if let Err(error) = push_bounded(&mut outputs, (value, Some(steps)), ctx) {
                        return PathRun::Failed(Vec::new(), error);
                    }
                }
                Err(exn) => match outcome(exn, &runtime) {
                    Outcome::Halted => break,
                    Outcome::Fatal(error) => return PathRun::Failed(Vec::new(), error),
                    Outcome::Failed(error) if is_path_refusal(&error) => return PathRun::NotPath,
                    Outcome::Failed(error) => {
                        runtime.release_logs();
                        return PathRun::Failed(outputs, runtime_error(error));
                    }
                },
            }
        }
        runtime.release_logs();
        PathRun::Done(outputs)
    }

    fn run_values(
        &self,
        input: Val,
        ctx: &ExecCtx,
        floor: usize,
    ) -> (Vec<RawOutput>, Option<Error>) {
        let runtime = Runtime::new(&self.filter.lut, ctx, false, floor);
        let vars = Vars::new([runtime.environment()]);
        let mut outputs = Vec::new();
        for item in self.filter.id.run((Ctx::<Doc>::new(&runtime, vars), input)) {
            let result = match item {
                Ok(value) => push_bounded(&mut outputs, (value, None), ctx),
                Err(exn) => match outcome(exn, &runtime) {
                    Outcome::Halted => break,
                    Outcome::Fatal(error) => Err(error),
                    Outcome::Failed(error) => Err(runtime_error(error)),
                },
            };
            if let Err(error) = result {
                return (outputs, Some(error));
            }
        }
        (outputs, None)
    }
}

impl fmt::Debug for Query {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Query")
            .field("expression", &self.expression)
            .finish_non_exhaustive()
    }
}

/// Language value of a `jq` value read from JSON text.
pub(crate) fn value_of(val: &Val) -> Value {
    from_val(val)
}

fn push_bounded(outputs: &mut Vec<RawOutput>, item: RawOutput, ctx: &ExecCtx) -> Result<()> {
    ctx.checkpoint()?;
    let max = ctx.limits().max_records;
    if u64::try_from(outputs.len()).unwrap_or(u64::MAX) >= max {
        return Err(Error::new(
            ErrorCode::Limit,
            format!("the filter produced more than {max} values; narrow the filter"),
        ));
    }
    outputs.push(item);
    Ok(())
}

fn outcome(exn: Exn<'_, Val>, runtime: &Runtime<'_>) -> Outcome {
    let exn = match exn.get_err() {
        Ok(error) => return Outcome::Failed(error),
        Err(exn) => exn,
    };
    match exn.get_halt() {
        Ok(code) if is_stop_code(code) => Outcome::Fatal(
            runtime
                .take_stop()
                .unwrap_or_else(|| Error::new(ErrorCode::Cancelled, "the execution was cancelled")),
        ),
        Ok(0) => Outcome::Halted,
        Ok(code) => Outcome::Fatal(halt_error(code, runtime.last_stderr())),
        Err(_) => Outcome::Fatal(Error::new(
            ErrorCode::Query,
            "the filter used `break` outside of its `label`",
        )),
    }
}

fn halt_error(code: i32, message: Option<String>) -> Error {
    let text = match message {
        Some(message) => format!("the filter stopped with exit code {code}: {message}"),
        None => format!("the filter stopped with exit code {code}"),
    };
    Error::new(ErrorCode::Query, text)
        .with_problem("halt")
        .with_detail("exit_code", serde_json::Value::from(code))
}

fn runtime_error(error: jaq_core::Error<Val>) -> Error {
    let message = error.to_string();
    let value = from_val(&error.into_val()).to_plain_json();
    Error::new(ErrorCode::Query, format!("the filter failed: {message}"))
        .with_problem("runtime")
        .with_detail("error", value)
}

fn output(input: &Value, value: &Val, path: Option<Vec<Val>>) -> QueryOutput {
    let Some(path) = path else {
        return QueryOutput {
            value: from_val(value),
            path: None,
        };
    };
    let steps: Vec<PathStep> = path.iter().map(path_step).collect();
    let value = match lookup(input, &steps) {
        Some(original) if to_val(original) == *value => original.clone(),
        _ => from_val(value),
    };
    QueryOutput {
        value,
        path: Some(steps),
    }
}

fn path_step(step: &Val) -> PathStep {
    match step {
        Val::TStr(_) => PathStep::Key(key_text(step)),
        Val::Num(number) => number
            .as_isize()
            .and_then(|index| i64::try_from(index).ok())
            .map_or_else(|| PathStep::Other(from_val(step)), PathStep::Index),
        other => PathStep::Other(from_val(other)),
    }
}

fn lookup<'v>(value: &'v Value, steps: &[PathStep]) -> Option<&'v Value> {
    steps
        .iter()
        .try_fold(value, |current, step| match (current, step) {
            (Value::Map(fields), PathStep::Key(name)) => fields.get(name),
            (Value::Record(record), PathStep::Key(name)) => record.get(name),
            (Value::List(items), PathStep::Index(index)) => {
                let len = i64::try_from(items.len()).ok()?;
                let index = if *index < 0 { len + index } else { *index };
                items.get(usize::try_from(index).ok()?)
            }
            _ => None,
        })
}

/// Position of a fragment of the filter: byte offset, line and column, both
/// from 1, the column in characters.
fn position(code: &str, part: &str) -> Option<(usize, usize, usize)> {
    let start = code.as_ptr() as usize;
    let at = part.as_ptr() as usize;
    if at < start || at + part.len() > start + code.len() {
        return None;
    }
    let offset = at - start;
    let before = code.get(..offset)?;
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let column = before.get(line_start..)?.chars().count() + 1;
    Some((offset, line, column))
}

fn located(code: &str, part: &str, problem: &str, what: &str) -> Error {
    let found: String = part.chars().take(MAX_FOUND_CHARS).collect();
    let mut error = Error::new(ErrorCode::Query, what.to_owned()).with_problem(problem);
    if !found.is_empty() {
        error = error.with_found(found);
    }
    if let Some((offset, line, column)) = position(code, part) {
        error.message = format!("{what} at line {line}, column {column} of the filter");
        error = error
            .with_detail("offset", serde_json::Value::from(offset))
            .with_detail("line", serde_json::Value::from(line))
            .with_detail("column", serde_json::Value::from(column));
    }
    error
}

fn lex_expected(expect: &lex::Expect<&str>) -> String {
    match expect {
        lex::Expect::Delim(open) => format!("a delimiter closing `{open}`"),
        other => other.as_str().to_owned(),
    }
}

fn load_error(code: &str, errors: jaq_core::load::Errors<&str, ()>) -> Error {
    let Some((_, error)) = errors.into_iter().next() else {
        return Error::new(ErrorCode::Query, "the filter does not parse");
    };
    match error {
        jaq_core::load::Error::Lex(errors) => match errors.first() {
            Some((expect, found)) => located(
                code,
                found,
                "syntax",
                &format!(
                    "the filter does not parse: expected {}",
                    lex_expected(expect)
                ),
            ),
            None => Error::new(ErrorCode::Query, "the filter does not parse"),
        },
        jaq_core::load::Error::Parse(errors) => match errors.first() {
            Some((expect, found)) => parse_error(code, expect, found),
            None => Error::new(ErrorCode::Query, "the filter does not parse"),
        },
        jaq_core::load::Error::Io(errors) => {
            let module = errors.first().map_or("", |(path, _)| *path);
            located(
                code,
                module,
                "module",
                "the filter includes or imports a module; modules are not available in queries",
            )
        }
    }
}

fn parse_error(code: &str, expect: &parse::Expect<&str>, found: &str) -> Error {
    let what = if found.is_empty() {
        format!(
            "the filter does not parse: expected {} before the end of the filter",
            expect.as_str()
        )
    } else {
        format!("the filter does not parse: expected {}", expect.as_str())
    };
    located(code, found, "syntax", &what)
}

fn compile_error(code: &str, errors: jaq_core::compile::Errors<&str, ()>) -> Error {
    let first = errors
        .into_iter()
        .next()
        .and_then(|(_, undefined)| undefined.into_iter().next());
    let Some((name, kind)) = first else {
        return Error::new(ErrorCode::Query, "the filter does not compile");
    };
    let what = match kind {
        jaq_core::compile::Undefined::Filter(arity) => {
            format!("the filter calls the undefined filter `{name}/{arity}`")
        }
        other => format!("the filter uses the undefined {} `{name}`", other.as_str()),
    };
    located(code, name, "undefined", &what)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
    use nurt_contracts::stage::{CancelReason, CancelToken, Event, EventSink, Limits};
    use nurt_contracts::value::Fields;

    use super::*;

    const NOW_SECONDS: i128 = 1_700_000_000;
    const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

    struct FixedClock;

    impl Clock for FixedClock {
        fn now(&self) -> WallTime {
            WallTime::from_unix_nanos(NOW_SECONDS * NANOSECONDS_PER_SECOND)
        }

        fn monotonic(&self) -> Duration {
            Duration::ZERO
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: self.now(),
                boot: BootId("boot".to_owned()),
                since_boot_with_suspend: Duration::ZERO,
                since_boot_active: Duration::ZERO,
            }
        }

        fn sleep(
            &self,
            _duration: Duration,
            cancel: &CancelToken,
        ) -> std::result::Result<(), CancelReason> {
            cancel.reason().map_or(Ok(()), Err)
        }
    }

    struct FixedHost(HostInfo);

    impl Host for FixedHost {
        fn info(&self) -> &HostInfo {
            &self.0
        }

        fn env_var(&self, name: &str) -> Option<OsString> {
            (name == "HOME").then(|| OsString::from("/home/user"))
        }

        fn env_vars(&self) -> Vec<(OsString, OsString)> {
            vec![(OsString::from("HOME"), OsString::from("/home/user"))]
        }
    }

    #[derive(Default)]
    struct Events(Mutex<Vec<Event>>);

    impl EventSink for Events {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn host() -> Arc<FixedHost> {
        Arc::new(FixedHost(HostInfo {
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

    fn ctx() -> ExecCtx {
        ExecCtx::builder(Arc::new(FixedClock), host()).build()
    }

    fn json(text: &str) -> Value {
        Value::from_plain_json(serde_json::from_str(text).unwrap())
    }

    fn values(filter: &str, input: &str) -> Vec<Value> {
        let run = Query::compile(filter).unwrap().run(&json(input), &ctx());
        run.into_result()
            .unwrap()
            .into_iter()
            .map(|output| output.value)
            .collect()
    }

    fn key(name: &str) -> PathStep {
        PathStep::Key(name.to_owned())
    }

    #[test]
    fn path_filter_reports_paths() {
        let input = json(r#"{"a": {"b": 1}, "l": [5, 6]}"#);
        let query = Query::compile(".a.b, .l[]").unwrap();
        let outputs = query.run(&input, &ctx()).into_result().unwrap();
        assert_eq!(outputs[0].value, Value::Int(1));
        assert_eq!(outputs[0].path, Some(vec![key("a"), key("b")]));
        assert_eq!(outputs[2].path, Some(vec![key("l"), PathStep::Index(1)]));
    }

    #[test]
    fn computed_values_have_no_path() {
        let outputs = Query::compile(".a + 1, {x: .a}")
            .unwrap()
            .run(&json(r#"{"a": 1}"#), &ctx())
            .into_result()
            .unwrap();
        assert_eq!(
            outputs[0],
            QueryOutput {
                value: Value::Int(2),
                path: None
            }
        );
        assert!(outputs[1].path.is_none());
    }

    #[test]
    fn path_results_keep_the_typed_input_value() {
        let mut fields = Fields::new();
        fields.insert("v".to_owned(), Value::Decimal("1.10".to_owned()));
        let outputs = Query::compile(".v")
            .unwrap()
            .run(&Value::Map(fields), &ctx());
        assert_eq!(outputs.outputs[0].value, Value::Decimal("1.10".to_owned()));
    }

    #[test]
    fn slices_are_reported_as_other_steps() {
        let outputs = Query::compile(".[1:2]")
            .unwrap()
            .run(&json("[1, 2, 3]"), &ctx());
        let path = outputs.outputs[0].path.clone().unwrap();
        assert!(matches!(path[0], PathStep::Other(Value::Map(_))));
    }

    #[test]
    fn syntax_error_reports_position() {
        let error = Query::compile(".a |\n  (]").unwrap_err();
        assert_eq!(error.code, ErrorCode::Query);
        assert_eq!(error.problem.as_deref(), Some("syntax"));
        assert_eq!(error.details["line"], serde_json::json!(2));
        assert!(
            error.message.contains("does not parse"),
            "{}",
            error.message
        );
    }

    #[test]
    fn unterminated_filter_reports_end() {
        let error = Query::compile(".a | (").unwrap_err();
        assert_eq!(error.code, ErrorCode::Query);
    }

    #[test]
    fn undefined_filter_and_variable_are_named() {
        let error = Query::compile(".a | frobnicate(1)").unwrap_err();
        assert_eq!(error.found.as_deref(), Some("frobnicate"));
        assert!(
            error.message.contains("`frobnicate/1`"),
            "{}",
            error.message
        );
        assert_eq!(error.details["column"], serde_json::json!(6));
        let error = Query::compile("$nothing").unwrap_err();
        assert!(error.message.contains("variable"), "{}", error.message);
    }

    #[test]
    fn modules_are_rejected() {
        let error = Query::compile(r#"import "lib" as lib; ."#).unwrap_err();
        assert_eq!(error.problem.as_deref(), Some("module"));
        assert!(Query::compile("input").is_err());
    }

    #[test]
    fn runtime_error_follows_earlier_results() {
        let run = Query::compile(".[] | .a")
            .unwrap()
            .run(&json(r#"[{"a": 1}, "x"]"#), &ctx());
        assert_eq!(run.outputs.len(), 1);
        let error = run.error.unwrap();
        assert_eq!(error.code, ErrorCode::Query);
        assert_eq!(error.problem.as_deref(), Some("runtime"));
        assert!(error.message.contains("cannot index"), "{}", error.message);
    }

    #[test]
    fn error_value_is_kept_in_details() {
        let run = Query::compile(r#"error({"reason": "bad"})"#)
            .unwrap()
            .run(&Value::Null, &ctx());
        let error = run.error.unwrap();
        assert_eq!(error.details["error"], serde_json::json!({"reason": "bad"}));
    }

    #[test]
    fn halt_ends_results_and_halt_error_fails() {
        assert_eq!(values("1, halt, 2", "null"), vec![Value::Int(1)]);
        let run = Query::compile(r#""boom" | halt_error(3)"#)
            .unwrap()
            .run(&Value::Null, &ctx());
        let error = run.error.unwrap();
        assert_eq!(error.problem.as_deref(), Some("halt"));
        assert!(
            error.message.contains("exit code 3: boom"),
            "{}",
            error.message
        );
        let error = Query::compile("halt_error")
            .unwrap()
            .run(&Value::Null, &ctx())
            .error
            .unwrap();
        assert!(error.message.contains("exit code 5"), "{}", error.message);
    }

    #[test]
    fn environment_and_clock_come_from_the_context() {
        let home = Value::Text("/home/user".to_owned());
        assert_eq!(
            values("$ENV.HOME, env.HOME", "null"),
            vec![home.clone(), home]
        );
        assert_eq!(
            values("now | floor", "null"),
            vec![Value::Int(1_700_000_000)]
        );
    }

    #[test]
    fn generators_follow_jq() {
        let expected: Vec<Value> = [2, 2, 2, 2].into_iter().map(Value::Int).collect();
        assert_eq!(values("limit(4; repeat(. * 2))", "1"), expected);
        assert_eq!(
            values("[range(0; 10; 3)]", "null"),
            vec![json("[0, 3, 6, 9]")]
        );
        assert_eq!(values("[range(5; 0; -2)]", "null"), vec![json("[5, 3, 1]")]);
        assert_eq!(values("[range(1; 1; 0)]", "null"), vec![json("[]")]);
        assert_eq!(
            values(
                "[.[] | recurse(.c[]?) | .n]",
                r#"[{"n": 1, "c": [{"n": 2}]}]"#
            ),
            vec![json("[1, 2]")]
        );
        assert_eq!(
            values("[while(. < 4; . + 1)]", "1"),
            vec![json("[1, 2, 3]")]
        );
        assert_eq!(values("until(. > 4; . * 2)", "1"), vec![Value::Int(8)]);
        assert_eq!(
            values("[paths]", r#"{"a": [1]}"#),
            vec![json(r#"[["a"], ["a", 0]]"#)]
        );
    }

    #[test]
    fn endless_generator_stops_on_cancellation() {
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Timeout);
        let ctx = ExecCtx::builder(Arc::new(FixedClock), host())
            .cancel(cancel)
            .build();
        for filter in [
            "last(range(1; 2; 0))",
            "try last(repeat(1)) catch 0",
            "last(1 | recurse(.))",
        ] {
            let run = Query::compile(filter).unwrap().run(&Value::Null, &ctx);
            assert_eq!(
                run.error.map(|error| error.code),
                Some(ErrorCode::Timeout),
                "{filter}"
            );
        }
    }

    #[test]
    fn result_count_is_limited() {
        let limits = Limits {
            max_records: 3,
            ..Limits::default()
        };
        let ctx = ExecCtx::builder(Arc::new(FixedClock), host())
            .limits(limits)
            .build();
        for filter in ["range(10)", ".[]"] {
            let run = Query::compile(filter)
                .unwrap()
                .run(&json("[1, 2, 3, 4]"), &ctx);
            assert_eq!(
                run.error.map(|error| error.code),
                Some(ErrorCode::Limit),
                "{filter}"
            );
        }
    }

    #[test]
    fn debug_messages_are_emitted_once() {
        let events = Arc::new(Events::default());
        let ctx = ExecCtx::builder(Arc::new(FixedClock), host())
            .events(events.clone())
            .build();
        for filter in [".a | debug", ".a + 1 | debug", ".a | stderr | empty"] {
            let run = Query::compile(filter)
                .unwrap()
                .run(&json(r#"{"a": 1}"#), &ctx);
            assert!(run.error.is_none(), "{filter}");
        }
        let events = events.0.lock().unwrap();
        let messages: Vec<String> = events
            .iter()
            .map(|event| match event {
                Event::Log(log) => log.message.clone(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(messages, ["[\"DEBUG:\",1]", "[\"DEBUG:\",2]", "1"]);
    }

    #[test]
    fn range_with_non_numeric_bounds_fails_before_any_value() {
        for filter in [
            r#"range("a"; 3)"#,
            "range(0; null)",
            r#"range(0; 3; "x")"#,
            "range([1]; 3)",
        ] {
            let run = Query::compile(filter).unwrap().run(&Value::Null, &ctx());
            assert!(run.outputs.is_empty(), "{filter}");
            let error = run.error.unwrap_or_else(|| panic!("{filter} succeeded"));
            assert!(
                error.message.contains("Range bounds must be numeric"),
                "{filter}: {}",
                error.message
            );
        }
        assert_eq!(values("[range(2; 4)]", "null"), vec![json("[2, 3]")]);
    }

    #[test]
    fn generator_steps_are_limited_by_the_record_limit() {
        let limits = Limits {
            max_records: 10,
            ..Limits::default()
        };
        let ctx = ExecCtx::builder(Arc::new(FixedClock), host())
            .limits(limits)
            .build();
        for filter in ["[range(100)] | length", "[limit(50; repeat(1))] | length"] {
            let run = Query::compile(filter).unwrap().run(&Value::Null, &ctx);
            assert_eq!(
                run.error.map(|error| error.code),
                Some(ErrorCode::Limit),
                "{filter}"
            );
        }
        let run = Query::compile("[range(5)] | length")
            .unwrap()
            .run(&Value::Null, &ctx);
        assert!(run.error.is_none());
    }

    #[test]
    fn a_string_multiplied_by_a_number_follows_jq() {
        for (filter, expected) in [
            (r#""a" * 0"#, r#""""#),
            (r#"0 * "a""#, r#""""#),
            (r#""a" * 0.5"#, r#""""#),
            (r#""ab" * 2.7"#, r#""abab""#),
            (r#""a" * -1"#, "null"),
            (r#""a" * -0.5"#, "null"),
            (r#""a" * nan"#, "null"),
            (r#""" * 5"#, r#""""#),
        ] {
            assert_eq!(values(filter, "null"), vec![json(expected)], "{filter}");
        }
    }

    #[test]
    fn updates_create_and_extend_containers_as_jq_does() {
        for (filter, expected) in [
            ("[1, 2, 3] | .[5] = 1", "[1, 2, 3, null, null, 1]"),
            ("null | .a.b = 1", r#"{"a": {"b": 1}}"#),
            ("null | .[2] = 1", "[null, null, 1]"),
            ("[1, 2, 3] | .[1.5] = 9", "[1, 9, 3]"),
            ("[1, 2, 3] | del(.[7])", "[1, 2, 3]"),
            ("null | .a |= empty", "null"),
            ("[1, 2, 3, 4, 5] | del(.[0, 2], .[3])", "[2, 5]"),
            (r#"{"a": {"b": 1}} | del(.a.b, .a)"#, "{}"),
        ] {
            assert_eq!(values(filter, "null"), vec![json(expected)], "{filter}");
        }
        for filter in ["[1, 2, 3] | .[-5] = 1", "null | .[-1] = 1"] {
            let run = Query::compile(filter).unwrap().run(&Value::Null, &ctx());
            assert!(run.error.is_some(), "{filter}");
        }
    }

    #[test]
    fn an_update_past_the_end_of_a_list_counts_against_the_size_limit() {
        let ctx = repeat_limited_ctx(1000);
        let run = Query::compile("[1] | .[1000] = 1")
            .unwrap()
            .run(&Value::Null, &ctx);
        assert_eq!(run.error.map(|error| error.code), Some(ErrorCode::Limit));
        let run = Query::compile("[1] | .[10] = 1")
            .unwrap()
            .run(&Value::Null, &ctx);
        assert!(run.error.is_none());
    }

    #[test]
    fn reading_follows_jq_for_fractions_objects_and_null() {
        for (filter, expected) in [
            ("[1, 2, 3] | .[1.7]", "2"),
            ("[1, 2, 3] | .[-1.5]", "3"),
            ("[1, 2, 3] | .[nan]", "null"),
            ("null | .[1:]", "null"),
        ] {
            assert_eq!(values(filter, "null"), vec![json(expected)], "{filter}");
        }
        let run = Query::compile(r#"{"a": 1} | .[0]"#)
            .unwrap()
            .run(&Value::Null, &ctx());
        assert!(run.error.is_some());
    }

    #[test]
    fn base64_decoding_follows_jq() {
        for (filter, expected) in [
            (r#""YQ" | @base64d"#, r#""a""#),
            (r#""YQ==" | @base64d"#, r#""a""#),
            (r#""YWJj" | @base64d"#, r#""abc""#),
            (r#""abc" | @base64d"#, "\"i\u{fffd}\""),
            (r#""" | @base64d"#, r#""""#),
        ] {
            assert_eq!(values(filter, "null"), vec![json(expected)], "{filter}");
        }
        for filter in [r#""a" | @base64d"#, r#""a!b" | @base64d"#] {
            let run = Query::compile(filter).unwrap().run(&Value::Null, &ctx());
            assert!(run.error.is_some(), "{filter}");
        }
    }

    #[test]
    fn fromstream_rebuilds_the_values_of_a_stream() {
        assert_eq!(
            values("[1, [2]] | fromstream(tostream)", "null"),
            vec![json("[1, [2]]")]
        );
        let run = Query::compile("fromstream(([0], 1), ([1], 2), ([1]))")
            .unwrap()
            .run(&Value::Null, &ctx());
        assert!(run.error.is_some());
    }

    fn repeat_limited_ctx(max_bytes: u64) -> ExecCtx {
        let limits = Limits {
            max_file_bytes_text: max_bytes,
            ..Limits::default()
        };
        ExecCtx::builder(Arc::new(FixedClock), host())
            .limits(limits)
            .build()
    }

    #[test]
    fn a_string_repeated_beyond_the_size_limit_is_an_error_even_inside_try() {
        let ctx = repeat_limited_ctx(1000);
        for filter in [
            r#""x" * 2000"#,
            r#"2000 * "x""#,
            r#""abcdefghij" * 101"#,
            r#""x" * 1000000000000"#,
            r#"try ("x" * 2000) catch "caught""#,
            r#"[.[] | . * 2000]"#,
        ] {
            let run = Query::compile(filter).unwrap().run(&json(r#"["x"]"#), &ctx);
            assert_eq!(
                run.error.map(|error| error.code),
                Some(ErrorCode::Limit),
                "{filter}"
            );
        }
    }

    #[test]
    fn a_string_repeated_within_the_size_limit_is_unchanged() {
        let ctx = repeat_limited_ctx(1000);
        let run = Query::compile(r#""ab" * 3, ("x" * 1000 | length), ("x" * 0)"#)
            .unwrap()
            .run(&Value::Null, &ctx);
        assert!(run.error.is_none(), "{:?}", run.error);
        let values: Vec<Value> = run.outputs.into_iter().map(|output| output.value).collect();
        assert_eq!(
            values,
            [
                Value::Text("ababab".into()),
                Value::Int(1000),
                Value::Text(String::new())
            ]
        );
    }

    #[test]
    fn the_repeat_limit_of_one_run_does_not_reach_the_next() {
        let strict = repeat_limited_ctx(10);
        let filter = Query::compile(r#""x" * 100 | length"#).unwrap();
        assert_eq!(
            filter.run(&Value::Null, &strict).error.map(|e| e.code),
            Some(ErrorCode::Limit)
        );
        let run = filter.run(&Value::Null, &ctx());
        assert_eq!(run.into_result().unwrap()[0].value, Value::Int(100));
    }

    #[test]
    fn runtime_error_of_a_path_filter_keeps_the_paths_of_earlier_results() {
        let run = Query::compile(".[] | .a")
            .unwrap()
            .run(&json(r#"[{"a": 1}, "x"]"#), &ctx());
        assert_eq!(
            run.outputs[0].path,
            Some(vec![PathStep::Index(0), key("a")])
        );
        assert!(run.error.is_some());
    }

    #[test]
    fn non_path_filter_is_evaluated_for_its_values() {
        let events = Arc::new(Events::default());
        let ctx = ExecCtx::builder(Arc::new(FixedClock), host())
            .events(events.clone())
            .build();
        let run = Query::compile(".a + 1 | debug")
            .unwrap()
            .run(&json(r#"{"a": 1}"#), &ctx);
        assert_eq!(run.outputs[0].value, Value::Int(2));
        assert!(run.outputs[0].path.is_none());
        assert_eq!(events.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn debug_output_shows_the_expression() {
        let query = Query::compile(".").unwrap();
        assert!(format!("{query:?}").contains("expression"));
    }
}
