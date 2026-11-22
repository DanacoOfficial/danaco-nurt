// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Execution of one case on a connection.

use std::path::Path;

use serde_json::{Value, json};

use crate::case::{Case, Expectation, FsExpectation, Golden, Plan, Step};
use crate::client::{CallError, Connection};
use crate::compare::{self, Match};
use crate::normalize::{self, VolatilePaths};
use crate::pointer::{self, Bindings};
use crate::protocol::{self, member, method};
use crate::report::Failure;
use crate::schemas::SchemaSet;
use crate::snapshot::{Difference, Snapshot};

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

/// Runs a case whose working directory is `base` and returns the failed
/// checks; an empty list means the case passed.
pub fn execute(
    case: &Case,
    connection: &mut dyn Connection,
    base: &Path,
    context: &Context<'_>,
) -> Vec<Failure> {
    let mut run = CaseRun {
        case,
        connection,
        base,
        base_text: base.to_string_lossy().replace('\\', "/"),
        context,
        results: Vec::new(),
        failures: Vec::new(),
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
    run.failures
}

/// Marker of a failure after which the case cannot continue.
struct Stop;

struct CaseRun<'a, 'c> {
    case: &'a Case,
    connection: &'c mut dyn Connection,
    base: &'a Path,
    base_text: String,
    context: &'a Context<'a>,
    results: Vec<Value>,
    failures: Vec<Failure>,
}

impl Bindings for CaseRun<'_, '_> {
    fn base(&self) -> Option<&str> {
        Some(&self.base_text)
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

    fn capture(&mut self, step: usize) -> Result<Snapshot, Stop> {
        Snapshot::capture(self.base, cfg!(unix), true).map_err(|error| {
            self.fail(Some(step), "fs", error.to_string());
            Stop
        })
    }

    fn run_step(&mut self, number: usize, step: &Step) -> Result<(), Stop> {
        let before = match step.fs_after {
            Some(FsExpectation::Unchanged) => Some(self.capture(number)?),
            _ => None,
        };
        let params = pointer::substitute(&step.params, self).map_err(|error| {
            self.fail(Some(number), "params", error.0);
            Stop
        })?;
        let mut call_params = params;
        if protocol::takes_request(&step.method) {
            protocol::set_base(&mut call_params, &self.base_text);
        }
        let outcome = self.connection.call(&step.method, &call_params);
        let result = self.outcome(number, step, outcome)?;
        if let Some(result) = &result {
            self.check_result(number, step, result);
        }
        self.results.push(result.unwrap_or(Value::Null));
        let expected = match (&step.fs_after, before) {
            (Some(FsExpectation::Unchanged), Some(before)) => Some(before),
            (Some(FsExpectation::Snapshot(snapshot)), _) => Some(snapshot.clone()),
            _ => None,
        };
        if let Some(expected) = expected {
            self.check_fs(number, &expected);
        }
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
        normalize::rewrite_base(&mut actual, &self.base_text);
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

    fn check_fs(&mut self, number: usize, expected: &Snapshot) {
        match expected.diff(self.base) {
            Ok(differences) => {
                for difference in differences.iter().take(compare::MAX_DIFFERENCES) {
                    self.failures.push(fs_failure(number, difference));
                }
            }
            Err(error) => self.fail(Some(number), "fs", error.to_string()),
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
                protocol::print_params(&golden.ast, variant),
            )?;
            let Some(text) = result.get(member::TEXT).and_then(Value::as_str) else {
                self.fail(None, &format!("print {variant}"), "the result has no text");
                continue;
            };
            let text = text.to_owned();
            self.golden_parse(golden, variant, &text, "print and parse")?;
            if variant == "c"
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
        // Variant C writes every argument by name, so only its bound tree is compared.
        if variant != "c" {
            self.compare_tree(&check, &golden.ast, tree);
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
        if let Some(reported) = result.get(member::AST_HASH).and_then(Value::as_str) {
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

fn fs_failure(step: usize, difference: &Difference) -> Failure {
    Failure {
        step: Some(step),
        check: Some("fs".to_owned()),
        path: Some(difference.path.clone()),
        ..Failure::new(difference.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::case::{Class, Kind};
    use std::collections::VecDeque;

    /// Connection that returns prepared answers and records the calls.
    struct Scripted {
        answers: VecDeque<Result<Value, CallError>>,
        calls: Vec<(String, Value)>,
    }

    impl Connection for Scripted {
        fn call(&mut self, method: &str, params: &Value) -> Result<Value, CallError> {
            self.calls.push((method.to_owned(), params.clone()));
            self.answers
                .pop_front()
                .unwrap_or_else(|| Err(CallError::Transport("no answer".to_owned())))
        }
    }

    fn scripted(answers: Vec<Result<Value, CallError>>) -> Scripted {
        Scripted {
            answers: answers.into(),
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
            plan,
            source: "case.toml".into(),
        }
    }

    fn step(method: &str, params: Value) -> Step {
        Step {
            method: method.to_owned(),
            params,
            expectations: Vec::new(),
            expect_error: None,
            validate: None,
            fs_after: None,
        }
    }

    fn run(case: &Case, connection: &mut Scripted, base: &Path) -> Vec<Failure> {
        let schemas = SchemaSet::new().unwrap();
        let volatile = Context::response_volatile_paths(&schemas);
        let context = Context {
            schemas: &schemas,
            response_volatile: &volatile,
        };
        execute(case, connection, base, &context)
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
        let checks: Vec<(Option<usize>, Option<&str>)> = failures
            .iter()
            .map(|f| (f.step, f.check.as_deref()))
            .collect();
        assert_eq!(
            checks,
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
        // The first answer is consumed before the effect appears, so the
        // effect is created by the test between the calls.
        let failures = {
            let schemas = SchemaSet::new().unwrap();
            let volatile = VolatilePaths::default();
            let context = Context {
                schemas: &schemas,
                response_volatile: &volatile,
            };
            let mut effect = Effect {
                inner: &mut connection,
                path: &path,
            };
            execute(&case, &mut effect, base.path(), &context)
        };
        let paths: Vec<Option<&str>> = failures.iter().map(|f| f.path.as_deref()).collect();
        assert_eq!(paths, vec![Some("effect.txt"), Some("effect.txt")]);
        assert!(failures[0].message.starts_with("extra effect.txt"));
    }

    /// Connection that creates a file before every answer.
    struct Effect<'a> {
        inner: &'a mut Scripted,
        path: &'a Path,
    }

    impl Connection for Effect<'_> {
        fn call(&mut self, method: &str, params: &Value) -> Result<Value, CallError> {
            std::fs::write(self.path, "x").unwrap();
            self.inner.call(method, params)
        }
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
    fn golden_results_without_trees_are_failures() {
        let base = tempfile::tempdir().unwrap();
        let golden = Golden {
            texts: Vec::new(),
            ast: golden_tree("count"),
            ast_hash: None,
            bound: Some(json!({})),
            bound_hash: None,
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
