// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Check without effect: network endpoints, launched programs, paths,
//! waiting, effect class and the static checks of implementations.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::sync::Arc;

use common::{field, find, given, int, path, pipeline, run, single, source, text, transform};
use nurt_catalog::Registry;
use nurt_contracts::bound::{BoundDocument, BoundStage, Phase};
use nurt_contracts::config::Config;
use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::kinds::{EffectKind, Implementation, Reversibility};
use nurt_contracts::report::{Correction, CorrectionClass};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExternalProgram, InputKinds, NetworkTarget, StageDescriptor, StageImpl,
    StaticInfo,
};
use nurt_contracts::value::Value;
use nurt_plan::{CheckSummary, PlanOptions, TargetSource};

fn summarize_with(
    document: &BoundDocument,
    registry: &Registry,
    options: &PlanOptions,
) -> Result<CheckSummary> {
    let plan = nurt_plan::plan_with(document, registry, options)?;
    nurt_plan::check(&plan, registry, &common::host(), &Config::default())
}

fn summarize(document: &BoundDocument) -> CheckSummary {
    summarize_with(document, &common::registry(), &PlanOptions::new()).unwrap()
}

fn request(url: nurt_contracts::bound::BoundExpr) -> BoundStage {
    source("http.request", vec![given("url", url)])
}

fn push(remote: &str) -> BoundStage {
    common::stage(
        "git.push",
        EffectKind::External,
        Phase::Mutate,
        vec![
            given("repo", path("/srv/app")),
            given("remote", text(remote)),
        ],
    )
}

#[test]
fn http_request_reports_literal_endpoint_without_credentials() {
    let summary = summarize(&single(pipeline(
        request(text("https://user:secret@api.example.com:8443/v1?q=1")),
        Vec::new(),
    )));
    assert_eq!(summary.network.len(), 1);
    let found = &summary.network[0];
    assert_eq!(found.command, "http.request");
    assert_eq!(found.param.as_deref(), Some("url"));
    assert_eq!(found.source, TargetSource::Literal);
    assert_eq!(
        found.target,
        Some(NetworkTarget {
            scheme: "https".to_owned(),
            host: "api.example.com".to_owned(),
            port: Some(8443),
            url: Some("https://api.example.com:8443/v1?q=1".to_owned()),
        })
    );
    assert_eq!(summary.effect, EffectKind::Read);
    assert_eq!(summary.groups, ["network"]);
    let json = serde_json::to_string(&summary).unwrap();
    assert!(!json.contains("secret"));
}

#[test]
fn address_computed_per_record_is_dynamic() {
    let summary = summarize(&single(pipeline(request(field("link")), Vec::new())));
    assert_eq!(summary.network[0].source, TargetSource::Record);
    assert!(summary.network[0].target.is_none());
}

#[test]
fn batch_file_runs_through_launcher() {
    let mut build = run(&["C:/tools/build.bat", "release"]);
    build.waits = true;
    let summary = summarize(&single(pipeline(build, Vec::new())));
    assert_eq!(summary.programs.len(), 1);
    let program = &summary.programs[0];
    assert_eq!(program.command, "run");
    assert_eq!(program.program.as_deref(), Some("C:/tools/build.bat"));
    assert_eq!(program.launcher.as_deref(), Some("cmd.exe"));
    assert_eq!(program.source, TargetSource::Literal);
    assert!(summary.waits);
    assert_eq!(summary.effect, EffectKind::Process);
}

#[test]
fn program_computed_per_record_is_dynamic() {
    let line = nurt_contracts::bound::BoundExpr::List {
        items: vec![field("tool"), text("--version")],
    };
    let tool = common::stage(
        "run",
        EffectKind::Process,
        Phase::Run,
        vec![given("command", line)],
    );
    let summary = summarize(&single(pipeline(tool, Vec::new())));
    assert_eq!(summary.programs[0].source, TargetSource::Record);
    assert!(summary.programs[0].program.is_none());
}

#[test]
fn command_given_as_one_text_names_the_program() {
    let single_text = common::stage(
        "run",
        EffectKind::Process,
        Phase::Run,
        vec![given("command", text("make"))],
    );
    let summary = summarize(&single(pipeline(single_text, Vec::new())));
    assert_eq!(summary.programs.len(), 1);
    assert_eq!(summary.programs[0].program.as_deref(), Some("make"));
    assert_eq!(summary.programs[0].source, TargetSource::Literal);
}

#[test]
fn empty_command_list_names_no_program() {
    let empty = nurt_contracts::bound::BoundExpr::List { items: Vec::new() };
    let none = common::stage(
        "run",
        EffectKind::Process,
        Phase::Run,
        vec![given("command", empty)],
    );
    let summary = summarize(&single(pipeline(none, Vec::new())));
    assert!(summary.programs.is_empty());
}

#[test]
fn git_push_to_named_remote_is_resolved_later() {
    let summary = summarize(&single(pipeline(push("origin"), Vec::new())));
    assert_eq!(summary.network.len(), 1);
    assert_eq!(summary.network[0].source, TargetSource::Remote);
    assert_eq!(summary.network[0].remote.as_deref(), Some("origin"));
    assert_eq!(summary.effect, EffectKind::External);
    assert_eq!(summary.reversibility, Reversibility::Irreversible);
    assert_eq!(summary.groups, ["git"]);
    assert_eq!(summary.paths, ["/srv/app"]);
}

#[test]
fn git_push_to_address_reports_ssh_endpoint() {
    let summary = summarize(&single(pipeline(
        push("git@github.com:org/repo.git"),
        Vec::new(),
    )));
    let target = summary.network[0].target.as_ref().unwrap();
    assert_eq!(
        (target.scheme.as_str(), target.host.as_str()),
        ("ssh", "github.com")
    );
    assert_eq!(target.port, Some(22));
}

#[test]
fn read_only_pipeline_touches_nothing() {
    let filter = transform(
        "filter",
        vec![given("condition", common::greater("size", int(10)))],
    );
    let first = transform("first", vec![given("count", int(10))]);
    let summary = summarize(&single(pipeline(find("/srv/app"), vec![filter, first])));
    assert_eq!(summary.format, "nurt-check/1");
    assert_eq!(summary.effect, EffectKind::Read);
    assert_eq!(summary.reversibility, Reversibility::Read);
    assert_eq!(summary.paths, ["/srv/app"]);
    assert!(summary.network.is_empty());
    assert!(summary.programs.is_empty());
    assert!(!summary.waits);
    assert_eq!(summary.operations, 1);
    assert_eq!(summary.groups, ["core"]);
}

#[test]
fn corrections_are_passed_through() {
    let correction = Correction {
        id: "C1".to_owned(),
        rule: "L01".to_owned(),
        class: CorrectionClass::Fuzzy,
        at: None,
        from: "fnd".to_owned(),
        to: "find".to_owned(),
        kind: Some("command".to_owned()),
        alternatives: Vec::new(),
    };
    let options = PlanOptions::new().with_corrections(vec![correction.clone()]);
    let program = single(pipeline(find("/a"), Vec::new()));
    let summary = summarize_with(&program, &common::registry(), &options).unwrap();
    assert_eq!(summary.corrections, [correction]);
}

#[test]
fn script_counts_operations() {
    let program = common::document(vec![
        common::operation(pipeline(find("/a"), Vec::new())),
        common::operation(pipeline(run(&["make"]), Vec::new())),
    ]);
    let summary = summarize(&program);
    assert_eq!(summary.operations, 2);
    assert_eq!(summary.programs[0].program.as_deref(), Some("make"));
}

/// Implementation of `http.request` whose static check reports fixed facts
/// or rejects the arguments.
struct StaticRequest {
    reject: bool,
}

impl StageImpl for StaticRequest {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "http.request".to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if self.reject {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "header names must be tokens",
            ));
        }
        let mut info = StaticInfo {
            waits: true,
            network: vec![NetworkTarget {
                scheme: "https".to_owned(),
                host: "proxy.example.com".to_owned(),
                port: Some(3128),
                url: None,
            }],
            programs: vec![ExternalProgram {
                program: "helper.cmd".to_owned(),
                implementation: Implementation::SystemProgram,
            }],
            paths: vec!["/tmp/cache".to_owned()],
            ..StaticInfo::default()
        };
        let url = args.value("url").cloned().unwrap_or(Value::Null);
        info.details.insert("url".to_owned(), url);
        Ok(info)
    }
}

fn registry_with(reject: bool) -> Registry {
    let mut registry = common::registry();
    registry
        .register_impl(Arc::new(StaticRequest { reject }))
        .unwrap();
    registry
}

#[test]
fn static_check_of_implementation_is_merged() {
    let program = single(pipeline(request(text("https://example.com/")), Vec::new()));
    let summary = summarize_with(&program, &registry_with(false), &PlanOptions::new()).unwrap();
    assert!(summary.waits);
    assert_eq!(summary.network.len(), 2);
    assert_eq!(summary.network[1].source, TargetSource::Implementation);
    assert_eq!(summary.programs[0].program.as_deref(), Some("helper.cmd"));
    assert_eq!(summary.programs[0].launcher.as_deref(), Some("cmd.exe"));
    assert_eq!(summary.paths, ["/tmp/cache"]);
    assert_eq!(summary.details.len(), 1);
    assert_eq!(summary.details[0].command, "http.request");
    assert_eq!(
        summary.details[0].details.get("url"),
        Some(&Value::Text("https://example.com/".to_owned()))
    );
}

#[test]
fn rejected_static_check_fails_the_check() {
    let program = single(pipeline(request(text("https://example.com/")), Vec::new()));
    let error = summarize_with(&program, &registry_with(true), &PlanOptions::new()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert_eq!(error.stage(), Some(Stage::Bind));
}

#[test]
fn summary_schema_names_every_member() {
    let schema = serde_json::to_value(schemars::schema_for!(CheckSummary)).unwrap();
    let properties = schema["properties"].as_object().unwrap();
    for member in [
        "format",
        "effect",
        "reversibility",
        "operations",
        "waits",
        "paths",
        "network",
        "programs",
        "groups",
        "corrections",
        "details",
    ] {
        assert!(properties.contains_key(member), "{member}");
    }
}

/// Stand-in for `assert` that reports the parameters whose value reached
/// the static check as a range.
struct RangeReader;

const RANGE_PARAMS: [&str; 3] = ["count", "paths", "exit_code"];

impl StageImpl for RangeReader {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: "assert".to_owned(),
            source: false,
            input: InputKinds::Any,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let mut info = StaticInfo::default();
        for name in RANGE_PARAMS {
            if let Some(value @ Value::Range(_)) = args.value(name) {
                info.details.insert(name.to_owned(), value.clone());
            }
        }
        Ok(info)
    }
}

#[test]
fn golden_programs_with_assert_pass_range_literals_to_the_static_check() {
    let mut registry = common::registry();
    registry.register_impl(Arc::new(RangeReader)).unwrap();
    for name in [
        "task-02-replace-identifier-expected-count",
        "task-07-tests-exit-code",
        "task-12-replace-verify-rollback",
    ] {
        let summary = summarize_with(&common::golden(name), &registry, &PlanOptions::new())
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let reported: Vec<_> = summary
            .details
            .iter()
            .filter(|found| found.command == "assert")
            .collect();
        assert!(!reported.is_empty(), "{name}");
        assert!(
            reported.iter().all(|found| !found.details.is_empty()),
            "{name}"
        );
    }
}
