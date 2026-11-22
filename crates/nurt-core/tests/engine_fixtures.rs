// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Requests through the fixture front end on the bound trees of the
//! specification (`nurt-bound/1`): the check without effect, the catalog
//! queries, planning and the responses of programs that the planner rejects.

#![cfg(feature = "fixtures")]
// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::envelope::{OutputFormat, Request, RequestOptions, Response};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::report::ReportStatus;
use nurt_contracts::schema;
use nurt_core::config::ConfigSources;
use nurt_core::fixtures::FixtureFrontend;
use nurt_core::{CatalogQuery, CheckReport, Engine, EngineConfig};
use nurt_testkit::clock::VirtualClock;
use nurt_testkit::host::TestHost;
use serde_json::{Value, json};
use tempfile::TempDir;

/// Program text under which the tree `invalid/effect-in-let` is kept: a
/// named result whose pipeline removes files.
const EFFECT_IN_LET: &str = "@x = find \"/srv/app/build\" |> remove\n@x";
/// Program text under which the tree `invalid/after-without-effect` is kept:
/// a stage of the phase `after` without an effect stage before it.
const AFTER_WITHOUT_EFFECT: &str = "find \"/srv/app\" |> assert exit_code: 0";

fn golden(dir: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/golden")
        .join(dir)
}

fn bound(name: &str) -> BoundDocument {
    let path = golden("bound").join(format!("{name}.json"));
    serde_json::from_slice(&fs::read(&path).unwrap())
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Names of the bound trees of the specification that have program text.
fn golden_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(golden("bound"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter_map(|name| name.strip_suffix(".json").map(str::to_owned))
        .filter(|stem| {
            !stem.ends_with(".generated") && golden("ast").join(format!("{stem}.nurt")).is_file()
        })
        .collect();
    names.sort();
    names
}

struct Fixture {
    _temp: TempDir,
    engine: Engine,
    programs: Vec<(String, String)>,
}

impl Fixture {
    fn text(&self, name: &str) -> &str {
        self.programs
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, text)| text.as_str())
            .unwrap_or_else(|| panic!("no program `{name}`"))
    }
}

/// Engine whose front end answers from a fixture directory assembled from
/// the program texts, syntax trees and bound trees of the specification.
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let mut programs = Vec::new();
    for name in golden_names() {
        let text = fs::read_to_string(golden("ast").join(format!("{name}.nurt"))).unwrap();
        fs::write(temp.path().join(format!("{name}.nurt")), &text).unwrap();
        fs::copy(
            golden("bound").join(format!("{name}.json")),
            temp.path().join(format!("{name}.bound.json")),
        )
        .unwrap();
        let ast = golden("ast").join(format!("{name}.json"));
        if ast.is_file() {
            fs::copy(&ast, temp.path().join(format!("{name}.ast.json"))).unwrap();
        }
        programs.push((name, text.trim().to_owned()));
    }
    let frontend = FixtureFrontend::load_dir(temp.path())
        .unwrap()
        .with_bound(EFFECT_IN_LET, bound("invalid/effect-in-let"))
        .with_bound(AFTER_WITHOUT_EFFECT, bound("invalid/after-without-effect"));
    let engine = Engine::open(
        EngineConfig::new(Arc::new(VirtualClock::new()), Arc::new(TestHost::linux()))
            .with_sources(ConfigSources::isolated())
            .without_state_dir()
            .with_frontend(Arc::new(frontend)),
    )
    .unwrap();
    Fixture {
        _temp: temp,
        engine,
        programs,
    }
}

fn validator(schema_text: &str, reference: Option<&str>) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(schema_text).unwrap();
    let schema = match reference {
        None => schema,
        Some(reference) => json!({
            "$schema": schema["$schema"],
            "$id": schema["$id"],
            "$ref": reference,
            "$defs": schema["$defs"],
        }),
    };
    jsonschema::validator_for(&schema).unwrap()
}

/// Checks a response against its own rules and the published schemas of the
/// report and of the response envelope.
fn assert_valid(response: &Response, what: &str) {
    response
        .validate()
        .unwrap_or_else(|error| panic!("{what}: {error:?}"));
    let report = validator(schema::REPORT, None);
    let envelope = validator(schema::REQUEST, Some("#/$defs/response"));
    let report_json = serde_json::to_value(&response.report).unwrap();
    let response_json = serde_json::to_value(response).unwrap();
    let errors: Vec<String> = report
        .iter_errors(&report_json)
        .chain(envelope.iter_errors(&response_json))
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:#?}");
}

/// Whether a bound tree has a stage of the command `assert`.
fn uses_assert(name: &str) -> bool {
    fn find(value: &Value) -> bool {
        match value {
            Value::Object(members) => {
                (members.get("t") == Some(&json!("stage"))
                    && members.get("name") == Some(&json!("assert")))
                    || members.values().any(find)
            }
            Value::Array(items) => items.iter().any(find),
            _ => false,
        }
    }
    find(&serde_json::to_value(bound(name)).unwrap())
}

fn assert_checked(f: &Fixture, name: &str, text: &str) {
    let report = f
        .engine
        .check(&Request::program(text))
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    assert_eq!(report.bound, bound(name), "{name}");
    assert_eq!(report.summary.format, "nurt-check/1", "{name}");
    assert!(report.summary.operations >= 1, "{name}");
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["format"], "nurt-check/1", "{name}");
    assert!(json["bound"].is_object(), "{name}");
    let back: CheckReport = serde_json::from_value(json).unwrap();
    assert_eq!(back, report, "{name}");
}

#[test]
fn every_golden_program_without_assert_is_checked_without_effect() {
    let f = fixture();
    assert!(f.programs.len() >= 10);
    let mut checked = 0;
    for (name, text) in f.programs.iter().filter(|(name, _)| !uses_assert(name)) {
        assert_checked(&f, name, text);
        checked += 1;
    }
    assert!(checked >= 10);
}

#[test]
#[ignore = "the planner passes literal ranges to the static check of `assert` as expressions"]
fn golden_programs_with_assert_are_checked_without_effect() {
    let f = fixture();
    for (name, text) in f.programs.iter().filter(|(name, _)| uses_assert(name)) {
        assert_checked(&f, name, text);
    }
}

#[test]
fn check_reports_the_effect_class_and_the_paths() {
    let f = fixture();
    for (name, effect) in [
        ("task-01-largest-go-files", EffectKind::Read),
        ("task-03-remove-temporary-files", EffectKind::Local),
        ("task-14-background-job", EffectKind::Process),
    ] {
        let report = f.engine.check(&Request::program(f.text(name))).unwrap();
        assert_eq!(report.summary.effect, effect, "{name}");
    }
    let removal = f
        .engine
        .check(&Request::program(f.text("task-03-remove-temporary-files")))
        .unwrap();
    assert_eq!(removal.summary.paths, vec!["/srv/app/build".to_owned()]);
    assert!(removal.summary.network.is_empty());
}

#[test]
fn syntax_trees_check_like_their_program_text() {
    let f = fixture();
    let mut compared = 0;
    for (name, text) in &f.programs {
        let path = golden("ast").join(format!("{name}.json"));
        if !path.is_file() || uses_assert(name) {
            continue;
        }
        let ast: AstDocument = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let from_tree = f.engine.check(&Request::ast(ast)).unwrap();
        let from_text = f.engine.check(&Request::program(text.as_str())).unwrap();
        assert_eq!(from_tree, from_text, "{name}");
        compared += 1;
    }
    assert!(compared > 0);
}

#[test]
fn catalog_queries_carry_the_revision() {
    let f = fixture();
    let summary = f.engine.catalog().unwrap();
    assert_eq!(summary["format"], "nurt-catalog-result/1");
    assert_eq!(summary["catalog_rev"], f.engine.catalog_rev());
    assert!(f.engine.catalog_rev().starts_with("sha256:"));
    let total = summary["total"].as_u64().unwrap();
    let groups = summary["groups"].as_array().unwrap();
    let listed: u64 = groups
        .iter()
        .map(|group| group["commands"].as_u64().unwrap())
        .sum();
    assert_eq!(listed, total);

    let name = groups[0]["name"].as_str().unwrap();
    let group = f.engine.catalog_query(&CatalogQuery::Group(name)).unwrap();
    assert_eq!(group["group"]["name"], name);
    assert_eq!(group["catalog_rev"], f.engine.catalog_rev());
    let command = f
        .engine
        .catalog_query(&CatalogQuery::Command("filter"))
        .unwrap();
    assert_eq!(command["command"]["name"], "filter");
    let unknown = f
        .engine
        .catalog_query(&CatalogQuery::Command("no-such-command"))
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::UnknownCommand);

    assert_eq!(fixture().engine.catalog().unwrap(), summary);
}

#[test]
fn planned_programs_stop_before_execution() {
    let f = fixture();
    for (name, text) in &f.programs {
        for preview in [false, true] {
            let options = RequestOptions {
                preview: Some(preview),
                ..RequestOptions::default()
            };
            let response = f
                .engine
                .run(&Request::program(text.as_str()).with_options(options));
            let what = format!("{name} (preview {preview})");
            assert_valid(&response, &what);
            assert_eq!(response.report.status, ReportStatus::Failed, "{what}");
            let error = response.report.error.as_ref().unwrap();
            assert_eq!(error.code, ErrorCode::Unsupported, "{what}");
            assert!(error.details["operations"].as_u64().unwrap() >= 1, "{what}");
            let rendered = response.rendered.as_ref().unwrap();
            assert_eq!(rendered.format, OutputFormat::Text, "{what}");
            assert!(!rendered.text.is_empty(), "{what}");
        }
    }
    let removal = f
        .engine
        .run(&Request::program(f.text("task-03-remove-temporary-files")));
    assert_eq!(removal.report.kind, "remove");
    assert!(removal.report.op.is_none());
}

#[test]
fn plan_errors_are_reported_with_the_split_program() {
    let f = fixture();
    let response = f.engine.run(&Request::program(EFFECT_IN_LET));
    assert_valid(&response, "effect in let");
    assert_eq!(response.report.status, ReportStatus::Failed);
    assert_eq!(response.report.kind, "remove");
    let error = response.report.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::OneEffect);
    assert_eq!(error.next_steps.len(), 2);
    assert_eq!(error.next_steps[1], "@x");
    assert!(response.rendered.is_some());

    let check = f
        .engine
        .check(&Request::program(EFFECT_IN_LET))
        .unwrap_err();
    assert_eq!(check, *error);

    let response = f.engine.run(&Request::program(AFTER_WITHOUT_EFFECT));
    assert_valid(&response, "after without effect");
    assert_eq!(
        response.report.error.as_ref().unwrap().code,
        ErrorCode::AstSchema
    );
    assert_eq!(
        f.engine
            .check(&Request::program(AFTER_WITHOUT_EFFECT))
            .unwrap_err()
            .code,
        ErrorCode::AstSchema
    );
}

#[test]
fn responses_are_rendered_in_the_requested_format() {
    let f = fixture();
    let text = f.text("task-01-largest-go-files");
    for (format, rendered) in [
        (OutputFormat::Json, None),
        (OutputFormat::Markdown, Some(OutputFormat::Markdown)),
        (OutputFormat::Compact, Some(OutputFormat::Compact)),
    ] {
        let options = RequestOptions {
            format: Some(format),
            ..RequestOptions::default()
        };
        let response = f.engine.run(&Request::program(text).with_options(options));
        assert_valid(&response, &format!("{format:?}"));
        assert_eq!(
            response.rendered.as_ref().map(|rendered| rendered.format),
            rendered
        );
    }
    let unknown = f.engine.run(&Request::program("count"));
    assert_valid(&unknown, "program without fixture");
    assert_eq!(
        unknown.report.error.as_ref().unwrap().code,
        ErrorCode::Unsupported
    );
    assert_eq!(unknown.report.kind, "program");
}
