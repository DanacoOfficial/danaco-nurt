// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Checks of the case files of the `conformance/` directory: every error code
//! has a case, golden cases are copies of the golden files of the
//! specification, every invalid surface tree of the specification has an
//! error case, every published schema is checked against the meta-schema, and
//! the binding cases cover the whole interface. Requests that the cases and the
//! binding cases make follow the schema of the protocol.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nurt_conformance::binding::{self, CALLS, Outcome};
use nurt_conformance::case::{Kind, Plan, Suite};
use nurt_conformance::compare::ast_hash;
use nurt_conformance::protocol;
use nurt_conformance::schemas::{METASCHEMA, SchemaSet};
use nurt_contracts::error::ErrorCode;
use serde_json::Value;

/// Generated schema of a golden directory; not a golden case.
const GENERATED_SCHEMA: &str = "schema.generated.json";

/// Directory of the documents a reader rejects.
const INVALID_DIR: &str = "invalid";

/// Table of the rejected documents and their error codes.
const INVALID_TABLE: &str = "expected.tsv";

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

fn spec_golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/golden")
}

fn suite() -> Suite {
    Suite::load(&conformance(), &SchemaSet::new().unwrap()).unwrap()
}

/// Collects every text of a member `code` that names an error code.
fn codes_in(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(members) => {
            for (name, member) in members {
                match member {
                    Value::String(code) if name == "code" && code.starts_with("E_") => {
                        out.insert(code.clone());
                    }
                    other => codes_in(other, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| codes_in(item, out)),
        _ => {}
    }
}

#[test]
fn every_error_code_has_an_error_case_that_expects_it() {
    let suite = suite();
    let known: BTreeSet<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
    let mut covered = BTreeSet::new();
    for case in suite.cases.iter().filter(|case| case.kind == Kind::Error) {
        let code = case.code.clone().unwrap();
        assert!(
            known.contains(code.as_str()),
            "{}: unknown code {code}",
            case.id
        );
        let mut expected = BTreeSet::new();
        for value in case.expected_values() {
            codes_in(value, &mut expected);
        }
        assert!(
            expected.contains(&code),
            "{}: no expectation names {code}",
            case.id
        );
        covered.insert(code);
    }
    let missing: Vec<&str> = known
        .iter()
        .copied()
        .filter(|code| !covered.contains(*code))
        .collect();
    assert!(
        missing.is_empty(),
        "error codes without a case: {missing:?}"
    );
}

/// Golden files of one format of the specification, without the generated
/// schema and the directory of invalid documents.
fn spec_files(format: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(spec_golden().join(format))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file() && !path.ends_with(GENERATED_SCHEMA))
        .collect();
    files.sort();
    files
}

/// Name of the golden case and file of the case directory that hold a copy
/// of a golden file of the specification.
fn copy_of(format: &str, file_name: &str) -> (String, &'static str) {
    const COPIES: [(&str, &str, &str); 9] = [
        ("ast", ".b.nurt", "b.nurt"),
        ("ast", ".c.nurt", "c.nurt"),
        ("ast", ".c.json", "c.json"),
        ("ast", ".c.hash", "c.hash"),
        ("ast", ".nurt", "a.nurt"),
        ("ast", ".json", "ast.json"),
        ("ast", ".hash", "ast.hash"),
        ("bound", ".json", "bound.json"),
        ("bound", ".hash", "bound.hash"),
    ];
    COPIES
        .iter()
        .find_map(|(copy_format, suffix, copy)| {
            let name = file_name.strip_suffix(suffix)?;
            (*copy_format == format).then(|| (name.to_owned(), *copy))
        })
        .unwrap_or_else(|| panic!("unexpected golden file {format}/{file_name}"))
}

#[test]
fn golden_cases_are_copies_of_the_specification_files() {
    for format in ["ast", "bound"] {
        for source in spec_files(format) {
            let file_name = source.file_name().unwrap().to_string_lossy().into_owned();
            let (name, file) = copy_of(format, &file_name);
            let copy = conformance().join("golden").join(name).join(file);
            let expected = fs::read(&source).unwrap();
            let actual = fs::read(&copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display()));
            assert_eq!(
                actual,
                expected,
                "{} differs from {}",
                copy.display(),
                source.display()
            );
        }
    }
}

#[test]
fn golden_hashes_match_their_trees() {
    let suite = suite();
    let mut checked = 0;
    for case in &suite.cases {
        let Plan::Golden(golden) = &case.plan else {
            continue;
        };
        if let Some(hash) = &golden.ast_hash {
            assert_eq!(&ast_hash(&golden.ast).unwrap(), hash, "{}", case.id);
        }
        if let (Some(bound), Some(hash)) = (&golden.bound, &golden.bound_hash) {
            assert_eq!(&ast_hash(bound).unwrap(), hash, "{}", case.id);
        }
        if let (Some(tree), Some(hash)) = (&golden.c_ast, &golden.c_ast_hash) {
            assert_eq!(&ast_hash(tree).unwrap(), hash, "{} variant c", case.id);
        }
        checked += 1;
    }
    let programs = spec_files("ast")
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json") && !name.ends_with(".c.json"))
        .count();
    assert!(
        checked >= programs,
        "{checked} golden cases, {programs} trees"
    );
}

#[test]
fn invalid_surface_trees_of_the_specification_have_error_cases() {
    let suite = suite();
    let directory = spec_golden().join("ast").join(INVALID_DIR);
    let table = fs::read_to_string(directory.join(INVALID_TABLE)).unwrap();
    let rows: Vec<Vec<&str>> = table
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').collect())
        .collect();
    assert!(!rows.is_empty());
    for row in rows {
        let (file, code) = (row[0], row[1]);
        let document: Value =
            serde_json::from_str(&fs::read_to_string(directory.join(file)).unwrap()).unwrap();
        let covered = suite.cases.iter().any(|case| {
            let Plan::Steps(steps) = &case.plan else {
                return false;
            };
            case.kind == Kind::Error
                && case.code.as_deref() == Some(code)
                && steps
                    .iter()
                    .any(|step| step.params.get("ast") == Some(&document))
        });
        assert!(covered, "{file}: no error case sends this tree with {code}");
    }
}

#[test]
fn golden_tasks_are_written_in_all_three_variants() {
    let suite = suite();
    let tasks: Vec<_> = suite
        .cases
        .iter()
        .filter(|case| case.id.starts_with("golden/task-"))
        .collect();
    assert!(tasks.len() >= 14, "{} task cases", tasks.len());
    for case in tasks {
        let Plan::Golden(golden) = &case.plan else {
            panic!("{} is not a golden case", case.id);
        };
        let variants: Vec<&str> = golden.texts.iter().map(|(v, _)| v.as_str()).collect();
        assert_eq!(variants, ["a", "b", "c"], "{}", case.id);
        assert!(golden.c_ast.is_some(), "{}: no variant c tree", case.id);
    }
}

#[test]
fn binding_cases_cover_every_call_and_outcome() {
    let files =
        binding::load_dir(&conformance().join("bindings"), &SchemaSet::new().unwrap()).unwrap();
    let steps: Vec<_> = files
        .iter()
        .flat_map(|(_, file)| &file.cases)
        .flat_map(|case| &case.steps)
        .collect();
    let calls: BTreeSet<&str> = steps.iter().map(|step| step.call.as_str()).collect();
    let missing: Vec<&str> = CALLS
        .iter()
        .copied()
        .filter(|call| !calls.contains(call))
        .collect();
    assert!(
        missing.is_empty(),
        "calls without a binding case: {missing:?}"
    );
    for outcome in [Outcome::Ok, Outcome::LanguageError, Outcome::CallError] {
        assert!(
            steps.iter().any(|step| step.expect.outcome == outcome),
            "{outcome:?}"
        );
    }
    let chained = steps.iter().any(|step| {
        step.call == "page" && step.args.values().any(|v| v.to_string().contains("${step"))
    });
    assert!(chained, "no page call uses the cursor of an earlier step");
    assert!(
        steps
            .iter()
            .any(|step| step.args.contains_key("raise_on_error"))
    );
}

#[test]
fn every_published_schema_has_a_case_that_checks_it_against_the_metaschema() {
    let suite = suite();
    for (name, _) in nurt_contracts::schema::ALL {
        let covered = suite.cases.iter().any(|case| {
            let Plan::Steps(steps) = &case.plan else {
                return false;
            };
            case.kind == Kind::Golden
                && steps.iter().any(|step| {
                    step.method == "schema"
                        && step.params.get("name").and_then(Value::as_str) == Some(*name)
                        && step.validate == Some((String::new(), METASCHEMA.to_owned()))
                })
        });
        assert!(covered, "schema {name} has no metaschema case");
    }
}

#[test]
fn suite_has_cases_of_every_kind_of_this_directory() {
    let suite = suite();
    for kind in [Kind::Golden, Kind::Bind, Kind::Error] {
        let count = suite.cases.iter().filter(|case| case.kind == kind).count();
        assert!(count > 0, "no case of kind {}", kind.name());
    }
    let ids: BTreeSet<&str> = suite.cases.iter().map(|case| case.id.as_str()).collect();
    assert_eq!(ids.len(), suite.cases.len());
}

#[test]
fn binding_calls_without_call_errors_map_to_requests_of_the_protocol_schema() {
    let schemas = SchemaSet::new().unwrap();
    let files = binding::load_dir(&conformance().join("bindings"), &schemas).unwrap();
    for (_, file) in &files {
        for case in &file.cases {
            for step in &case.steps {
                let Some((method, params)) = step.rpc() else {
                    continue;
                };
                if step.expect.outcome == Outcome::CallError {
                    continue;
                }
                let message = protocol::request(1, method, &params);
                assert_eq!(
                    schemas.validate_message(&message),
                    Ok(()),
                    "{}: {}",
                    case.id,
                    step.call
                );
            }
        }
    }
}

#[test]
fn steps_of_the_golden_bind_and_error_cases_are_requests_of_the_protocol_schema() {
    let schemas = SchemaSet::new().unwrap();
    for case in &suite().cases {
        let Plan::Steps(steps) = &case.plan else {
            continue;
        };
        if !matches!(case.kind, Kind::Golden | Kind::Bind | Kind::Error) {
            continue;
        }
        let protocol_steps = steps.iter().filter(|step| {
            step.expect_error.is_none()
                && !step.method.starts_with(protocol::method::TESTKIT_PREFIX)
        });
        for step in protocol_steps {
            let mut params = step.params.clone();
            if protocol::takes_request(&step.method) {
                protocol::set_base(&mut params, "/work");
            }
            let message = protocol::request(1, &step.method, &params);
            assert_eq!(
                schemas.validate_message(&message),
                Ok(()),
                "{}: {}",
                case.id,
                step.method
            );
        }
    }
}
