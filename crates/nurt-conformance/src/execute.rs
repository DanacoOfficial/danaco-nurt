// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Execution of one case on a connection.

use std::path::Path;

use serde_json::{Value, json};

use crate::case::{
    Case, EventExpectation, EventOrder, Expectation, FsExpectation, Golden, Plan, Step,
};
use crate::client::{CallError, Connection};
use crate::compare::{self, Match};
use crate::normalize::{
    self, EVENT_VOLATILE, HTTP_BASE_PLACEHOLDER, SECOND_VOLUME_PLACEHOLDER, VolatilePaths,
};
use crate::pointer::{self, Bindings, variable};
use crate::protocol::{self, code, member, method};
use crate::report::Failure;
use crate::schemas::SchemaSet;
use crate::snapshot::{Difference, Snapshot};

/// Name of the check of the working directory.
const CHECK_FS: &str = "fs";

/// Name of the check of the directory on the second volume.
const CHECK_SECOND_VOLUME: &str = "second_volume";

/// Shared data of the execution of cases.
#[derive(Debug)]
pub struct Context<'a> {
    /// Published schemas.
    pub schemas: &'a SchemaSet,
    /// Volatile members of responses `nurt-response/1`.
    pub response_volatile: &'a VolatilePaths,
}

impl Context<'_> {
    /// Returns the volatile members of `nurt-response/1` derived from the
    /// report schema.
    #[must_use]
    pub fn response_volatile_paths(schemas: &SchemaSet) -> VolatilePaths {
        schemas
            .document("report")
            .map(|schema| VolatilePaths::from_schema(schema, "/report"))
            .unwrap_or_default()
    }
}

/// Directories and addresses that the runner provides to one case.
#[derive(Debug, Clone, Copy)]
pub struct Workspace<'a> {
    /// Working directory, `${base}`.
    pub work: &'a Path,
    /// Directory of the case on the second volume, `${second_volume}`.
    pub second_volume: Option<&'a Path>,
    /// Address of the local HTTP server of the case, `${http_base}`.
    pub http_base: Option<&'a str>,
}

impl<'a> Workspace<'a> {
    /// Returns a workspace with the working directory `work` and no
    /// fixtures.
    #[must_use]
    pub fn new(work: &'a Path) -> Self {
        Self {
            work,
            second_volume: None,
            http_base: None,
        }
    }
}

/// Result of the execution of a case.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// Failed checks; empty when the case passed or was skipped.
    pub failures: Vec<Failure>,
    /// Reason of a skip: the implementation answered a call of a method of
    /// the feature `testkit` with "method not found" before any check
    /// failed.
    pub skipped: Option<String>,
}

/// Runs a case in the given workspace and returns the failed checks or the
/// reason of a skip.
pub fn execute(
    case: &Case,
    connection: &mut dyn Connection,
    workspace: &Workspace<'_>,
    context: &Context<'_>,
) -> Outcome {
    let mut run = CaseRun {
        case,
        connection,
        workspace: *workspace,
        base_text: slashes(workspace.work),
        second_text: workspace.second_volume.map(slashes),
        context,
        results: Vec::new(),
        failures: Vec::new(),
        skipped: None,
    };
    match &case.plan {
        Plan::Steps(steps) => {
            for (index, step) in steps.iter().enumerate() {
                if run.run_step(index + 1, step).is_err() {
                    break;
                }
            }
        }
        Plan::Golden(golden) => {
            // A broken connection ends the case; the failure is recorded.
            let _ = run.golden(golden);
        }
    }
    Outcome {
        failures: run.failures,
        skipped: run.skipped,
    }
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Marker of a failure after which the case cannot continue.
struct Stop;

struct CaseRun<'a, 'c> {
    case: &'a Case,
    connection: &'c mut dyn Connection,
    workspace: Workspace<'a>,
    base_text: String,
    second_text: Option<String>,
    context: &'a Context<'a>,
    results: Vec<Value>,
    failures: Vec<Failure>,
    skipped: Option<String>,
}

impl Bindings for CaseRun<'_, '_> {
    fn variable(&self, name: &str) -> Option<String> {
        match name {
            variable::BASE => Some(self.base_text.clone()),
            variable::HTTP_BASE => self.workspace.http_base.map(str::to_owned),
            variable::SECOND_VOLUME => self.second_text.clone(),
            _ => None,
        }
    }

    fn step(&self, number: usize, pointer: &str) -> Option<Value> {
        self.results
            .get(number.checked_sub(1)?)?
            .pointer(pointer)
            .cloned()
    }
}

impl CaseRun<'_, '_> {
    fn fail(&mut self, step: Option<usize>, check: &str, message: impl Into<String>) {
        self.failures.push(Failure {
            step,
            check: Some(check.to_owned()),
            ..Failure::new(message)
        });
    }

    fn call(
        &mut self,
        step: Option<usize>,
        method: &str,
        mut params: Value,
    ) -> Result<Value, Stop> {
        if protocol::takes_request(method) {
            protocol::set_base(&mut params, &self.base_text);
        }
        match self.connection.call(method, &params) {
            Ok(result) => Ok(result),
            Err(error) => {
                self.fail(step, method, error.to_string());
                Err(Stop)
            }
        }
    }

    /// Rewrites the directories and the address of the workspace in an
    /// actual value to their placeholders.
    fn normalize_actual(&self, value: &mut Value) {
        normalize::rewrite_base(value, &self.workspace.work.to_string_lossy());
        if let Some(second) = self.workspace.second_volume {
            normalize::rewrite_path(value, &second.to_string_lossy(), SECOND_VOLUME_PLACEHOLDER);
        }
        if let Some(http_base) = self.workspace.http_base {
            normalize::rewrite_occurrences(value, http_base, HTTP_BASE_PLACEHOLDER);
        }
    }

    /// Captures the directory before a call whose expectation is
    /// `"unchanged"`.
    fn before(
        &mut self,
        number: usize,
        check: &str,
        expectation: Option<&FsExpectation>,
        dir: Option<&Path>,
    ) -> Result<Option<Snapshot>, Stop> {
        let (Some(FsExpectation::Unchanged), Some(dir)) = (expectation, dir) else {
            return Ok(None);
        };
        Snapshot::capture(dir, cfg!(unix), true)
            .map(Some)
            .map_err(|error| {
                self.fail(Some(number), check, error.to_string());
                Stop
            })
    }

    fn run_step(&mut self, number: usize, step: &Step) -> Result<(), Stop> {
        let work = Some(self.workspace.work);
        let second = self.workspace.second_volume;
        let before = self.before(number, CHECK_FS, step.fs_after.as_ref(), work)?;
        let second_before = self.before(
            number,
            CHECK_SECOND_VOLUME,
            step.second_volume_after.as_ref(),
            second,
        )?;
        let params = pointer::substitute(&step.params, self).map_err(|error| {
            self.fail(Some(number), "params", error.0);
            Stop
        })?;
        let mut call_params = params;
        if protocol::takes_request(&step.method) {
            protocol::set_base(&mut call_params, &self.base_text);
        }
        // Events received before this call belong to earlier calls.
        self.connection.take_events();
        let outcome = self.connection.call(&step.method, &call_params);
        let events = self.connection.take_events();
        let result = self.outcome(number, step, outcome)?;
        if let Some(result) = &result {
            self.check_result(number, step, result);
        }
        if let Some(expected) = &step.events {
            self.check_events(number, expected, events);
        }
        self.results.push(result.unwrap_or(Value::Null));
        self.check_dir(number, CHECK_FS, step.fs_after.as_ref(), before, work);
        self.check_dir(
            number,
            CHECK_SECOND_VOLUME,
            step.second_volume_after.as_ref(),
            second_before,
            second,
        );
        Ok(())
    }

    fn outcome(
        &mut self,
        number: usize,
        step: &Step,
        outcome: Result<Value, CallError>,
    ) -> Result<Option<Value>, Stop> {
        match (outcome, step.expect_error) {
            (Ok(result), None) => Ok(Some(result)),
            (Ok(result), Some(code)) => {
                self.failures.push(Failure {
                    step: Some(number),
                    check: Some("error".to_owned()),
                    expected: Some(json!(code)),
                    actual: Some(result),
                    ..Failure::new("the call succeeded where an error was expected")
                });
                Ok(None)
            }
            (Err(CallError::Rpc { code, .. }), Some(expected)) if code == expected => Ok(None),
            (
                Err(CallError::Rpc {
                    code: code::METHOD_NOT_FOUND,
                    ..
                }),
                _,
            ) if step.method.starts_with(method::TESTKIT_PREFIX) => {
                let reason = format!("the implementation does not offer {}", step.method);
                if self.failures.is_empty() {
                    self.skipped = Some(reason);
                } else {
                    self.fail(Some(number), &step.method, reason);
                }
                Err(Stop)
            }
            (Err(error @ CallError::Rpc { .. }), _) => {
                self.fail(Some(number), &step.method, error.to_string());
                Ok(None)
            }
            (Err(error), _) => {
                self.fail(Some(number), &step.method, error.to_string());
                Err(Stop)
            }
        }
    }

    fn check_result(&mut self, number: usize, step: &Step, result: &Value) {
        for expectation in &step.expectations {
            self.check_expectation(number, &step.method, expectation, result);
        }
        for pattern in &step.absent {
            for found in pointer::find_matches(result, &pointer::segments(pattern)) {
                self.failures.push(Failure {
                    step: Some(number),
                    check: Some("absent".to_owned()),
                    actual: result.pointer(&found).cloned(),
                    path: Some(found),
                    ..Failure::new("member must be absent")
                });
            }
        }
        if let Some((at, schema)) = &step.validate {
            match result.pointer(at) {
                Some(value) => self.check_schema(Some(number), schema, at, value),
                None => self.fail(Some(number), "validate", format!("no value at {at:?}")),
            }
        }
    }

    fn volatile(&self, method_name: &str) -> VolatilePaths {
        let mut paths = if method_name == method::RUN || method_name == method::PAGE {
            self.context.response_volatile.clone()
        } else {
            VolatilePaths::default()
        };
        paths.extend(self.case.volatile.iter().map(String::as_str));
        paths
    }

    fn check_expectation(
        &mut self,
        number: usize,
        method_name: &str,
        expectation: &Expectation,
        result: &Value,
    ) {
        let volatile = self.volatile(method_name);
        let mut actual = result.clone();
        self.normalize_actual(&mut actual);
        volatile.apply(&mut actual);
        let mut expected = json!({});
        pointer::insert(&mut expected, &expectation.at, expectation.value.clone());
        volatile.apply(&mut expected);
        let (Some(expected), Some(actual)) = (
            expected.pointer(&expectation.at),
            actual.pointer(&expectation.at),
        ) else {
            self.fail(
                Some(number),
                "expect",
                format!("no value at {:?}", expectation.at),
            );
            return;
        };
        for difference in compare::compare(expected, actual, expectation.matching) {
            self.failures.push(Failure {
                step: Some(number),
                check: Some("expect".to_owned()),
                path: Some(format!("{}{}", expectation.at, difference.path)),
                expected: difference.expected,
                actual: difference.actual,
                ..Failure::new(difference.message)
            });
        }
    }

    /// Compares the events of a call with the expected ones. Each expected
    /// event is compared with a received event as a subset, after the
    /// volatile members `seq` and `time` are normalized.
    fn check_events(&mut self, number: usize, expected: &EventExpectation, received: Vec<Value>) {
        let mut volatile = VolatilePaths::default();
        volatile.extend(EVENT_VOLATILE);
        let actual: Vec<Value> = received
            .into_iter()
            .map(|mut event| {
                self.normalize_actual(&mut event);
                volatile.apply(&mut event);
                event
            })
            .collect();
        let wanted: Vec<Value> = expected
            .events
            .iter()
            .map(|event| {
                let mut event = event.clone();
                volatile.apply(&mut event);
                event
            })
            .collect();
        let failure = |path: String, message: String, expected: Value, actual: Value| Failure {
            step: Some(number),
            check: Some("events".to_owned()),
            path: Some(path),
            expected: Some(expected),
            actual: Some(actual),
            ..Failure::new(message)
        };
        match expected.order {
            EventOrder::Exact if actual.len() != wanted.len() => {
                self.failures.push(failure(
                    String::new(),
                    format!("{} events instead of {}", actual.len(), wanted.len()),
                    json!(wanted),
                    json!(actual),
                ));
            }
            EventOrder::Exact => {
                for (index, (event, found)) in wanted.iter().zip(&actual).enumerate() {
                    for difference in compare::compare(event, found, Match::Subset) {
                        self.failures.push(failure(
                            format!("/{index}{}", difference.path),
                            difference.message,
                            difference.expected.unwrap_or(Value::Null),
                            difference.actual.unwrap_or(Value::Null),
                        ));
                    }
                }
            }
            EventOrder::Ordered => {
                let mut next = 0;
                for (index, event) in wanted.iter().enumerate() {
                    let position = actual[next..]
                        .iter()
                        .position(|found| compare::compare(event, found, Match::Subset).is_empty());
                    match position {
                        Some(offset) => next += offset + 1,
                        None => {
                            self.failures.push(failure(
                                format!("/{index}"),
                                "event not received after the events expected before it".to_owned(),
                                event.clone(),
                                json!(actual),
                            ));
                            return;
                        }
                    }
                }
            }
        }
    }

    fn check_schema(&mut self, step: Option<usize>, schema: &str, at: &str, value: &Value) {
        if let Err(errors) = self.context.schemas.validate(schema, value) {
            for error in errors.into_iter().take(compare::MAX_DIFFERENCES) {
                self.failures.push(Failure {
                    step,
                    check: Some(format!("schema {schema}")),
                    path: Some(at.to_owned()),
                    ..Failure::new(error)
                });
            }
        }
    }

    fn check_dir(
        &mut self,
        number: usize,
        check: &str,
        expectation: Option<&FsExpectation>,
        before: Option<Snapshot>,
        dir: Option<&Path>,
    ) {
        let expected = match (expectation, before) {
            (Some(FsExpectation::Unchanged), Some(before)) => before,
            (Some(FsExpectation::Snapshot(snapshot)), _) => snapshot.clone(),
            (Some(FsExpectation::Unchanged), None) | (None, _) => {
                if expectation.is_some() {
                    self.fail(Some(number), check, "the directory is not available");
                }
                return;
            }
        };
        let Some(dir) = dir else {
            self.fail(Some(number), check, "the directory is not available");
            return;
        };
        match expected.diff(dir) {
            Ok(differences) => {
                for difference in differences.iter().take(compare::MAX_DIFFERENCES) {
                    self.failures.push(fs_failure(number, check, difference));
                }
            }
            Err(error) => self.fail(Some(number), check, error.to_string()),
        }
    }

    fn golden(&mut self, golden: &Golden) -> Result<(), Stop> {
        for (variant, text) in &golden.texts {
            self.golden_parse(golden, variant, text, "parse")?;
            if let Some(bound) = &golden.bound {
                self.golden_check(golden, bound, variant, text)?;
            }
        }
        let printed: Vec<String> = if golden.texts.is_empty() {
            protocol::VARIANTS.iter().map(|v| (*v).to_owned()).collect()
        } else {
            golden
                .texts
                .iter()
                .map(|(variant, _)| variant.clone())
                .collect()
        };
        for variant in &printed {
            let result = self.call(
                None,
                method::PRINT,
                protocol::print_params(golden.printed_tree(variant), variant),
            )?;
            let Some(text) = result.get(member::TEXT).and_then(Value::as_str) else {
                self.fail(None, &format!("print {variant}"), "the result has no text");
                continue;
            };
            let text = text.to_owned();
            self.golden_parse(golden, variant, &text, "print and parse")?;
            if variant == protocol::VARIANT_C
                && let Some(bound) = &golden.bound
            {
                self.golden_check(golden, bound, variant, &text)?;
            }
        }
        Ok(())
    }

    fn golden_parse(
        &mut self,
        golden: &Golden,
        variant: &str,
        text: &str,
        check: &str,
    ) -> Result<(), Stop> {
        let check = format!("{check} {variant}");
        let result = self.call(None, method::PARSE, protocol::text_params(text, variant))?;
        let Some(tree) = result.get(member::AST) else {
            self.fail(None, &check, format!("the result has no tree: {result}"));
            return Ok(());
        };
        self.check_schema(None, "ast", "/ast", tree);
        if let Some(expected) = golden.surface(variant) {
            self.compare_tree(&check, expected, tree);
        }
        Ok(())
    }

    fn golden_check(
        &mut self,
        golden: &Golden,
        bound: &Value,
        variant: &str,
        text: &str,
    ) -> Result<(), Stop> {
        let check = format!("check {variant}");
        let request = protocol::program_request(text, variant, None);
        let result = self.call(None, method::CHECK, request)?;
        let Some(tree) = result.get(member::BOUND) else {
            self.fail(
                None,
                &check,
                format!("the result has no bound tree: {result}"),
            );
            return Ok(());
        };
        self.check_schema(None, "bound", "/bound", tree);
        self.compare_tree(&check, bound, tree);
        let Some(reported) = result.get(member::AST_HASH).and_then(Value::as_str) else {
            // The protocol allows the member to be absent only for a tree that is
            // not known; here the program was bound from a text, so it is known.
            self.failures.push(Failure {
                check: Some(format!("{check} ast_hash")),
                path: Some("/ast_hash".to_owned()),
                ..Failure::new("the result has no ast_hash although the tree of the program is known")
            });
            return Ok(());
        };
        {
            let computed = compare::ast_hash(tree).unwrap_or_default();
            let recorded = golden
                .bound_hash
                .clone()
                .unwrap_or_else(|| computed.clone());
            if reported != computed || reported != recorded {
                self.failures.push(Failure {
                    check: Some(format!("{check} ast_hash")),
                    path: Some("/ast_hash".to_owned()),
                    expected: Some(json!(recorded)),
                    actual: Some(json!(reported)),
                    ..Failure::new("ast_hash differs from the hash of the bound tree")
                });
            }
        }
        Ok(())
    }

    fn compare_tree(&mut self, check: &str, expected: &Value, actual: &Value) {
        for difference in compare::compare(expected, actual, Match::Tree) {
            self.failures.push(Failure {
                check: Some(check.to_owned()),
                path: Some(difference.path),
                expected: difference.expected,
                actual: difference.actual,
                ..Failure::new(difference.message)
            });
        }
    }
}

fn fs_failure(step: usize, check: &str, difference: &Difference) -> Failure {
    Failure {
        step: Some(step),
        check: Some(check.to_owned()),
        path: Some(difference.path.clone()),
        ..Failure::new(difference.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::{Class, Kind};
    use std::collections::VecDeque;

    /// Connection that returns prepared answers and events and records the
    /// calls.
    struct Scripted {
        answers: VecDeque<Result<Value, CallError>>,
        events: VecDeque<Vec<Value>>,
        pending: Vec<Value>,
        calls: Vec<(String, Value)>,
    }

    impl Connection for Scripted {
        fn call(&mut self, method: &str, params: &Value) -> Result<Value, CallError> {
            self.calls.push((method.to_owned(), params.clone()));
            self.pending = self.events.pop_front().unwrap_or_default();
            self.answers
                .pop_front()
                .unwrap_or_else(|| Err(CallError::Transport("no answer".to_owned())))
        }

        fn take_events(&mut self) -> Vec<Value> {
            std::mem::take(&mut self.pending)
        }
    }

    fn scripted(answers: Vec<Result<Value, CallError>>) -> Scripted {
        Scripted {
            answers: answers.into(),
            events: VecDeque::new(),
            pending: Vec::new(),
            calls: Vec::new(),
        }
    }

    fn case(plan: Plan) -> Case {
        Case {
            id: "t".to_owned(),
            kind: Kind::Exec,
            class: Class::Mandatory,
            language: "1.0".to_owned(),
            requires: Vec::new(),
            summary: "s".to_owned(),
            variant: "a".to_owned(),
            code: None,
            volatile: vec!["/report/extra".to_owned()],
            config: None,
            fs_before: None,
            second_volume_before: None,
            http_files: None,
            plan,
            source: "case.toml".into(),
        }
    }

    fn step(method: &str, params: Value) -> Step {
        Step {
            method: method.to_owned(),
            params,
            ..Step::default()
        }
    }

    fn run_in(case: &Case, connection: &mut dyn Connection, workspace: &Workspace<'_>) -> Outcome {
        let schemas = SchemaSet::new().unwrap();
        let volatile = Context::response_volatile_paths(&schemas);
        let context = Context {
            schemas: &schemas,
            response_volatile: &volatile,
        };
        execute(case, connection, workspace, &context)
    }

    fn run(case: &Case, connection: &mut Scripted, base: &Path) -> Vec<Failure> {
        run_in(case, connection, &Workspace::new(base)).failures
    }

    fn checks(failures: &[Failure]) -> Vec<(Option<usize>, Option<&str>)> {
        failures
            .iter()
            .map(|f| (f.step, f.check.as_deref()))
            .collect()
    }

    #[test]
    fn steps_substitute_placeholders_normalize_and_compare() {
        let base = tempfile::tempdir().unwrap();
        let base_text = base.path().to_string_lossy().replace('\\', "/");
        let mut first = step("run", json!({"nurt": "1.0", "program": "count"}));
        first.expectations.push(Expectation {
            at: "/report".to_owned(),
            value: json!({"status": "ok", "op": 1, "result_file": "${base}/out.txt", "extra": 1}),
            matching: Match::Subset,
        });
        let mut second = step("page", json!({"cursor": "${step1/report/next_cursor}"}));
        second.validate = Some(("/report".to_owned(), "report".to_owned()));
        let case = case(Plan::Steps(vec![first, second]));
        let mut connection = scripted(vec![
            Ok(
                json!({"nurt": "1.0", "report": {"nurt": "1.0", "status": "ok", "kind": "count", "op": 99,
                "result_file": format!("{base_text}/out.txt"), "next_cursor": "c9", "extra": 2}}),
            ),
            Ok(json!({"report": {"nurt": "1.0", "status": "ok", "kind": "count"}})),
        ]);
        let failures = run(&case, &mut connection, base.path());
        assert_eq!(failures, Vec::new());
        assert_eq!(connection.calls[0].1["options"]["base"], json!(base_text));
        assert_eq!(connection.calls[1].1, json!({"cursor": "c9"}));
    }

    #[test]
    fn differences_errors_and_schema_violations_are_reported() {
        let base = tempfile::tempdir().unwrap();
        let mut differs = step("run", json!({"nurt": "1.0", "program": "count"}));
        differs.expectations.push(Expectation {
            at: "/report".to_owned(),
            value: json!({"status": "ok"}),
            matching: Match::Subset,
        });
        let mut missing = step("parse", json!({}));
        missing.expectations.push(Expectation {
            at: "/ast".to_owned(),
            value: json!({}),
            matching: Match::Tree,
        });
        missing.validate = Some(("/none".to_owned(), "ast".to_owned()));
        let mut invalid = step("schema", json!({"which": "report"}));
        invalid.validate = Some((String::new(), "metaschema".to_owned()));
        let mut error_expected = step("run", json!({}));
        error_expected.expect_error = Some(-32602);
        let mut error_matches = step("run", json!({}));
        error_matches.expect_error = Some(-32602);
        let unexpected_error = step("info", json!({}));
        let bad_placeholder = step("page", json!({"cursor": "${step99/x}"}));
        let case = case(Plan::Steps(vec![
            differs,
            missing,
            invalid,
            error_expected,
            error_matches,
            unexpected_error,
            bad_placeholder,
        ]));
        let rpc = |code| CallError::Rpc {
            code,
            message: "m".to_owned(),
            data: None,
        };
        let mut connection = scripted(vec![
            Ok(json!({"report": {"status": "failed"}})),
            Ok(json!({})),
            Ok(json!({"type": 12})),
            Ok(json!({})),
            Err(rpc(-32602)),
            Err(rpc(-32601)),
        ]);
        let failures = run(&case, &mut connection, base.path());
        assert_eq!(
            checks(&failures),
            vec![
                (Some(1), Some("expect")),
                (Some(2), Some("expect")),
                (Some(2), Some("validate")),
                (Some(3), Some("schema metaschema")),
                (Some(4), Some("error")),
                (Some(6), Some("info")),
                (Some(7), Some("params")),
            ]
        );
        assert_eq!(failures[0].path.as_deref(), Some("/report/status"));
    }

    #[test]
    fn absent_members_are_reported_where_they_exist() {
        let base = tempfile::tempdir().unwrap();
        let mut absent = step("run", json!({"nurt": "1.0", "program": "count"}));
        absent.absent = vec![
            "/report/error".to_owned(),
            "/report/changes/*/rows_changed".to_owned(),
            "/report/irreversible".to_owned(),
        ];
        let case = case(Plan::Steps(vec![absent.clone(), absent]));
        let mut connection = scripted(vec![
            Ok(json!({"report": {"status": "ok", "changes": [{"path": "a"}, {"path": "b"}]}})),
            Ok(
                json!({"report": {"error": {"code": "E_IO"}, "changes": [{"path": "a"}, {"rows_changed": 2}]}}),
            ),
        ]);
        let failures = run(&case, &mut connection, base.path());
        let paths: Vec<(Option<usize>, Option<&str>)> = failures
            .iter()
            .map(|f| (f.step, f.path.as_deref()))
            .collect();
        assert_eq!(
            paths,
            vec![
                (Some(2), Some("/report/error")),
                (Some(2), Some("/report/changes/1/rows_changed")),
            ]
        );
        assert_eq!(failures[1].actual, Some(json!(2)));
        assert_eq!(failures[0].check.as_deref(), Some("absent"));
    }

    fn events_step(order: EventOrder, events: Vec<Value>) -> Step {
        Step {
            events: Some(EventExpectation { events, order }),
            ..step("run", json!({"nurt": "1.0", "program": "count"}))
        }
    }

    #[test]
    fn events_are_checked_in_order_and_by_their_members() {
        let base = tempfile::tempdir().unwrap();
        let base_text = base.path().to_string_lossy().replace('\\', "/");
        let started = json!({"t": "exec.started", "seq": 1});
        let finished = json!({"t": "exec.finished", "status": "ok"});
        let expected = vec![started.clone(), finished.clone()];
        let case = case(Plan::Steps(vec![
            events_step(EventOrder::Ordered, expected.clone()),
            events_step(EventOrder::Ordered, expected.clone()),
            events_step(EventOrder::Exact, expected.clone()),
            events_step(EventOrder::Exact, expected.clone()),
            events_step(
                EventOrder::Exact,
                vec![json!({"t": "x", "path": "${base}/a"})],
            ),
        ]));
        let progress = json!({"t": "exec.progress", "seq": 2});
        let mut connection = scripted((0..5).map(|_| Ok(json!({}))).collect());
        connection.events = VecDeque::from(vec![
            vec![
                json!({"t": "exec.started", "seq": 7, "time": "t"}),
                progress.clone(),
                json!({"t": "exec.finished", "seq": 9, "status": "ok"}),
            ],
            vec![finished.clone(), started.clone()],
            vec![started.clone(), progress, finished.clone()],
            vec![started, json!({"t": "exec.finished", "status": "failed"})],
            vec![json!({"t": "x", "path": format!("{base_text}/a")})],
        ]);
        let failures = run(&case, &mut connection, base.path());
        let found: Vec<(Option<usize>, Option<&str>, Option<&str>)> = failures
            .iter()
            .map(|f| (f.step, f.check.as_deref(), f.path.as_deref()))
            .collect();
        assert_eq!(
            found,
            vec![
                (Some(2), Some("events"), Some("/1")),
                (Some(3), Some("events"), Some("")),
                (Some(4), Some("events"), Some("/1/status")),
            ]
        );
        assert_eq!(failures[1].message, "3 events instead of 2");
    }

    #[test]
    fn missing_testkit_methods_skip_the_case() {
        let base = tempfile::tempdir().unwrap();
        let not_found = || {
            Err(CallError::Rpc {
                code: code::METHOD_NOT_FOUND,
                message: "m".to_owned(),
                data: None,
            })
        };
        let clock = step("testkit.clock.advance", json!({"by": "2h"}));
        let count = step("run", json!({"nurt": "1.0", "program": "count"}));
        let skipped = case(Plan::Steps(vec![clock.clone(), count.clone()]));
        let mut connection = scripted(vec![not_found(), Ok(json!({}))]);
        let outcome = run_in(&skipped, &mut connection, &Workspace::new(base.path()));
        assert_eq!(outcome.failures, Vec::new());
        assert_eq!(
            outcome.skipped.as_deref(),
            Some("the implementation does not offer testkit.clock.advance")
        );
        assert_eq!(connection.calls.len(), 1);

        let mut failing = count.clone();
        failing.expect_error = Some(-32602);
        let failed = case(Plan::Steps(vec![failing, clock.clone()]));
        let mut connection = scripted(vec![Ok(json!({})), not_found()]);
        let outcome = run_in(&failed, &mut connection, &Workspace::new(base.path()));
        assert_eq!(outcome.skipped, None);
        assert_eq!(
            checks(&outcome.failures),
            vec![
                (Some(1), Some("error")),
                (Some(2), Some("testkit.clock.advance"))
            ]
        );

        let mut expected = clock;
        expected.expect_error = Some(code::METHOD_NOT_FOUND);
        let declared = case(Plan::Steps(vec![expected, count]));
        let mut connection = scripted(vec![not_found(), Ok(json!({}))]);
        let outcome = run_in(&declared, &mut connection, &Workspace::new(base.path()));
        assert_eq!(outcome, Outcome::default());
        assert_eq!(connection.calls.len(), 2);
    }

    #[test]
    fn the_second_volume_and_the_http_base_are_substituted_normalized_and_checked() {
        let work = tempfile::tempdir().unwrap();
        let volume = tempfile::tempdir().unwrap();
        std::fs::write(volume.path().join("kept.txt"), "k").unwrap();
        let volume_text = volume.path().to_string_lossy().replace('\\', "/");
        let http = "http://127.0.0.1:4321";
        let mut moved = step(
            "run",
            json!({"nurt": "1.0", "program": "move \"a\" \"${second_volume}/a\" from: \"${http_base}/x\""}),
        );
        moved.expectations.push(Expectation {
            at: "/report".to_owned(),
            value: json!({"target": "${second_volume}/a", "url": "GET ${http_base}/x"}),
            matching: Match::Subset,
        });
        moved.second_volume_after = Some(FsExpectation::Snapshot(
            Snapshot::from_json(&json!({"entries": [
                {"path": "kept.txt", "type": "file", "content_utf8": "k"},
                {"path": "a", "type": "file", "content_utf8": "0"}
            ]}))
            .unwrap(),
        ));
        let mut unchanged = step("run", json!({"nurt": "1.0", "program": "count"}));
        unchanged.second_volume_after = Some(FsExpectation::Unchanged);
        let case = case(Plan::Steps(vec![moved, unchanged]));
        let mut connection = scripted(vec![
            Ok(
                json!({"report": {"target": format!("{volume_text}/a"), "url": format!("GET {http}/x")}}),
            ),
            Ok(json!({})),
        ]);
        let target = volume.path().join("a");
        let mut writer = Writer {
            inner: &mut connection,
            path: &target,
        };
        let workspace = Workspace {
            work: work.path(),
            second_volume: Some(volume.path()),
            http_base: Some(http),
        };
        let outcome = run_in(&case, &mut writer, &workspace);
        let paths: Vec<Option<&str>> = outcome.failures.iter().map(|f| f.path.as_deref()).collect();
        assert_eq!(paths, vec![Some("a")], "{:?}", outcome.failures);
        assert_eq!(outcome.failures[0].step, Some(2));
        assert_eq!(outcome.failures[0].check.as_deref(), Some("second_volume"));
        assert_eq!(
            connection.calls[0].1["program"],
            format!("move \"a\" \"{volume_text}/a\" from: \"{http}/x\"")
        );

        let mut unavailable = step("run", json!({"nurt": "1.0", "program": "${second_volume}"}));
        unavailable.second_volume_after = Some(FsExpectation::Unchanged);
        let mut snapshot_without_volume = step("run", json!({"nurt": "1.0", "program": "count"}));
        snapshot_without_volume.second_volume_after = Some(FsExpectation::Snapshot(
            Snapshot::from_json(&json!({"entries": []})).unwrap(),
        ));
        let mut unchanged_without_volume = step("run", json!({"nurt": "1.0", "program": "count"}));
        unchanged_without_volume.second_volume_after = Some(FsExpectation::Unchanged);
        let case = super::tests::case(Plan::Steps(vec![
            snapshot_without_volume,
            unchanged_without_volume,
            unavailable,
        ]));
        let mut connection = scripted(vec![Ok(json!({})), Ok(json!({}))]);
        let failures = run(&case, &mut connection, work.path());
        assert_eq!(
            checks(&failures),
            vec![
                (Some(1), Some("second_volume")),
                (Some(2), Some("second_volume")),
                (Some(3), Some("params")),
            ]
        );
    }

    /// Connection that writes a file before every answer; the content is the
    /// number of earlier calls.
    struct Writer<'a> {
        inner: &'a mut Scripted,
        path: &'a Path,
    }

    impl Connection for Writer<'_> {
        fn call(&mut self, method: &str, params: &Value) -> Result<Value, CallError> {
            std::fs::write(self.path, self.inner.calls.len().to_string()).unwrap();
            self.inner.call(method, params)
        }
    }

    #[test]
    fn broken_connections_stop_the_case() {
        let base = tempfile::tempdir().unwrap();
        let case = case(Plan::Steps(vec![
            step("info", json!({})),
            step("info", json!({})),
        ]));
        let mut connection = scripted(vec![Err(CallError::Timeout)]);
        let failures = run(&case, &mut connection, base.path());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].message, "time limit reached");
        assert_eq!(connection.calls.len(), 1);
    }

    #[test]
    fn file_system_expectations_are_checked() {
        let base = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("a.txt"), "a").unwrap();
        let mut unchanged = step("check", json!({"nurt": "1.0", "program": "count"}));
        unchanged.fs_after = Some(FsExpectation::Unchanged);
        let mut expected = step("run", json!({"nurt": "1.0", "program": "count"}));
        expected.fs_after = Some(FsExpectation::Snapshot(
            Snapshot::from_json(
                &json!({"entries": [{"path": "a.txt", "type": "file", "content_utf8": "a"}]}),
            )
            .unwrap(),
        ));
        let case = case(Plan::Steps(vec![unchanged, expected]));
        let path = base.path().join("effect.txt");
        let mut connection = scripted(vec![Ok(json!({})), Ok(json!({}))]);
        // The effect is created by the connection before each answer, so it
        // appears after the state before the first call was captured.
        let failures = {
            let mut effect = Writer {
                inner: &mut connection,
                path: &path,
            };
            run_in(&case, &mut effect, &Workspace::new(base.path())).failures
        };
        let paths: Vec<Option<&str>> = failures.iter().map(|f| f.path.as_deref()).collect();
        assert_eq!(paths, vec![Some("effect.txt"), Some("effect.txt")]);
        assert!(failures[0].message.starts_with("extra effect.txt"));
    }

    fn golden_tree(name: &str) -> Value {
        json!({"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
            "head": {"t": "stage", "name": name, "args": [], "span": [1, 1, 1, 6]}, "stages": []}]}})
    }

    #[test]
    fn golden_cases_compare_parse_check_and_print() {
        let base = tempfile::tempdir().unwrap();
        let ast = golden_tree("count");
        let bound = json!({"nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
            "program": {"statements": []}});
        let bound_hash = compare::ast_hash(&bound).unwrap();
        let golden = Golden {
            texts: vec![
                ("a".to_owned(), "count".to_owned()),
                ("c".to_owned(), "count".to_owned()),
            ],
            ast: ast.clone(),
            ast_hash: None,
            bound: Some(bound.clone()),
            bound_hash: Some(bound_hash.clone()),
            c_ast: None,
            c_ast_hash: None,
        };
        let case = case(Plan::Golden(Box::new(golden)));
        let parsed = Ok(json!({"ast": ast}));
        let checked = Ok(json!({"bound": bound, "ast_hash": bound_hash}));
        let mut connection = scripted(vec![
            parsed.clone(),
            checked.clone(),
            Ok(json!({"ast": golden_tree("other")})),
            checked.clone(),
            Ok(json!({"text": "count"})),
            parsed.clone(),
            Ok(json!({"text": "count"})),
            parsed,
            Ok(json!({"bound": {"nurt": "1.0"}, "ast_hash": "sha256:00"})),
        ]);
        let failures = run(&case, &mut connection, base.path());
        let methods: Vec<&str> = connection.calls.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            methods,
            [
                "parse", "check", "parse", "check", "print", "parse", "print", "parse", "check"
            ]
        );
        let checks: Vec<&str> = failures.iter().filter_map(|f| f.check.as_deref()).collect();
        assert!(checks.contains(&"schema bound"), "{checks:?}");
        assert!(checks.contains(&"check c"), "{checks:?}");
        assert!(checks.contains(&"check c ast_hash"), "{checks:?}");
        assert!(
            !checks.contains(&"parse c"),
            "variant c surface trees are not compared"
        );
    }

    #[test]
    fn a_check_without_ast_hash_fails_for_a_program_text() {
        let base = tempfile::tempdir().unwrap();
        let ast = golden_tree("count");
        let bound = json!({"nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
            "program": {"statements": []}});
        let golden = Golden {
            texts: vec![
                ("a".to_owned(), "count".to_owned()),
                ("c".to_owned(), "count".to_owned()),
            ],
            ast: ast.clone(),
            ast_hash: None,
            bound: Some(bound.clone()),
            bound_hash: Some(compare::ast_hash(&bound).unwrap()),
            c_ast: None,
            c_ast_hash: None,
        };
        let case = case(Plan::Golden(Box::new(golden)));
        let parsed = Ok(json!({"ast": ast}));
        let unhashed = Ok(json!({"bound": bound}));
        let mut connection = scripted(vec![
            parsed.clone(),
            unhashed.clone(),
            parsed.clone(),
            unhashed.clone(),
            Ok(json!({"text": "count"})),
            parsed.clone(),
            Ok(json!({"text": "count"})),
            parsed,
            unhashed,
        ]);
        let failures = run(&case, &mut connection, base.path());
        let checks: Vec<&str> = failures.iter().filter_map(|f| f.check.as_deref()).collect();
        assert!(checks.contains(&"check a ast_hash"), "{checks:?}");
        assert!(checks.contains(&"check c ast_hash"), "{checks:?}");
    }

    #[test]
    fn variant_c_surface_trees_are_compared_with_their_own_tree() {
        let base = tempfile::tempdir().unwrap();
        let c_ast = golden_tree("named");
        let golden = Golden {
            texts: vec![("c".to_owned(), "count".to_owned())],
            ast: golden_tree("count"),
            ast_hash: None,
            bound: None,
            bound_hash: None,
            c_ast: Some(c_ast.clone()),
            c_ast_hash: None,
        };
        let case = case(Plan::Golden(Box::new(golden)));
        let mut connection = scripted(vec![
            Ok(json!({"ast": golden_tree("count")})),
            Ok(json!({"text": "count"})),
            Ok(json!({"ast": c_ast.clone()})),
        ]);
        let failures = run(&case, &mut connection, base.path());
        let checks: Vec<&str> = failures
            .iter()
            .filter_map(|f| f.check.as_deref())
            .filter(|check| !check.starts_with("schema"))
            .collect();
        assert_eq!(checks, ["parse c"]);
        assert_eq!(connection.calls[1].0, "print");
        assert_eq!(connection.calls[1].1["ast"], c_ast);
    }

    #[test]
    fn golden_results_without_trees_are_failures() {
        let base = tempfile::tempdir().unwrap();
        let golden = Golden {
            texts: Vec::new(),
            ast: golden_tree("count"),
            ast_hash: None,
            bound: Some(json!({})),
            bound_hash: None,
            c_ast: None,
            c_ast_hash: None,
        };
        let case = case(Plan::Golden(Box::new(golden.clone())));
        let mut connection = scripted(vec![
            Ok(json!({})),
            Ok(json!({"text": "count"})),
            Ok(json!({})),
            Ok(json!({"text": "count"})),
            Ok(json!({"ast": golden_tree("count")})),
            Ok(json!({})),
        ]);
        let failures = run(&case, &mut connection, base.path());
        let checks: Vec<&str> = failures
            .iter()
            .filter_map(|f| f.check.as_deref())
            .filter(|check| !check.starts_with("schema"))
            .collect();
        assert_eq!(checks, ["print a", "print and parse b", "check c"]);

        let mut with_text = golden;
        with_text.texts = vec![("a".to_owned(), "count".to_owned())];
        let case = super::tests::case(Plan::Golden(Box::new(with_text)));
        let mut connection = scripted(vec![
            Ok(json!({"ast": golden_tree("count")})),
            Ok(json!({})),
        ]);
        let failures = run(&case, &mut connection, base.path());
        let checks: Vec<&str> = failures.iter().filter_map(|f| f.check.as_deref()).collect();
        assert!(checks.contains(&"check a"), "{checks:?}");
        assert_eq!(checks.last(), Some(&"print"));
    }
}
