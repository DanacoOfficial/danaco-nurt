// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Plans of the golden bound trees: the split into execution steps, the
//! effect class and the check without effect.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use std::collections::BTreeSet;

use nurt_contracts::bound::BoundDocument;
use nurt_contracts::config::Config;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::kinds::{EffectKind, Reversibility};

/// Expected outline of every golden bound tree: one entry per statement.
const PLANS: &[(&str, &[&str])] = &[
    ("example-06-path-with-spaces", &["2:info 4:remove"]),
    ("example-31-listening-port", &["2:ports 2:filter"]),
    ("example-32-host-programs", &["2:host"]),
    ("example-35-undo-with-conflict", &["4:undo"]),
    ("example-36-operation-history", &["2:history"]),
    ("extra-01-test-results-after-run", &["4:test.run 5a:filter"]),
    (
        "task-01-largest-go-files",
        &["2:find 2:filter 2:sort 2:first"],
    ),
    (
        "task-02-replace-identifier-expected-count",
        &["2:search 3:assert 4:replace"],
    ),
    ("task-03-remove-temporary-files", &["2:find 4:remove"]),
    (
        "task-06-named-result-count",
        &["let recent = 2:find 2:filter", "@recent 2:count"],
    ),
    ("task-07-tests-exit-code", &["4:run 5a:assert"]),
    ("task-08-append-to-file", &["4:write"]),
    (
        "task-09-insert-line-after-line",
        &["2:lines 2:filter 4:insert"],
    ),
    ("task-10-move-old-logs", &["2:find 2:filter 4:move"]),
    (
        "task-12-replace-verify-rollback",
        &["2:search 4:replace 6:verify{4:run 5a:assert}"],
    ),
    ("task-14-background-job", &["4:run"]),
];

/// Effect class of the golden plans whose class is fixed by the catalog.
const EFFECTS: &[(&str, EffectKind)] = &[
    ("example-06-path-with-spaces", EffectKind::Local),
    ("example-31-listening-port", EffectKind::Read),
    ("task-01-largest-go-files", EffectKind::Read),
    ("task-03-remove-temporary-files", EffectKind::Local),
    ("task-06-named-result-count", EffectKind::Read),
    ("task-07-tests-exit-code", EffectKind::Process),
    ("task-12-replace-verify-rollback", EffectKind::Process),
    ("task-14-background-job", EffectKind::Process),
];

#[test]
fn every_golden_file_has_an_expected_plan() {
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(common::golden_dir()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if let Some(stem) = name.strip_suffix(".json")
            && !stem.ends_with(".generated")
        {
            files.insert(stem.to_owned());
        }
    }
    let listed: BTreeSet<String> = PLANS.iter().map(|(name, _)| (*name).to_owned()).collect();
    assert_eq!(files, listed);
}

#[test]
fn golden_files_plan_into_expected_steps() {
    let registry = common::registry();
    for (name, expected) in PLANS {
        let plan = nurt_plan::plan(&common::golden(name), &registry)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(common::plan_outline(&plan), *expected, "{name}");
        assert_eq!(plan.catalog_rev, "cat-1");
        assert!(!plan.is_script(), "{name}");
    }
}

#[test]
fn golden_plans_have_expected_effect_class() {
    let registry = common::registry();
    for (name, effect) in EFFECTS {
        let plan = nurt_plan::plan(&common::golden(name), &registry).unwrap();
        assert_eq!(plan.effect, *effect, "{name}");
        if *effect == EffectKind::Read {
            assert_eq!(plan.reversibility, Reversibility::Read, "{name}");
        } else {
            assert_ne!(plan.reversibility, Reversibility::Read, "{name}");
        }
    }
}

#[test]
fn every_golden_plan_checks_without_effect() {
    let registry = common::registry();
    let host = common::host();
    let config = Config::default();
    for (name, _) in PLANS {
        let plan = nurt_plan::plan(&common::golden(name), &registry).unwrap();
        let summary = nurt_plan::check(&plan, &registry, &host, &config)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(summary.format, nurt_plan::CHECK_FORMAT);
        assert_eq!(summary.effect, plan.effect, "{name}");
        assert_eq!(summary.operations, 1, "{name}");
        assert!(!summary.groups.is_empty(), "{name}");
    }
}

#[test]
fn background_job_reports_program_and_directory() {
    let registry = common::registry();
    let plan = nurt_plan::plan(&common::golden("task-14-background-job"), &registry).unwrap();
    let summary = nurt_plan::check(&plan, &registry, &common::host(), &Config::default()).unwrap();
    assert_eq!(summary.programs.len(), 1);
    assert_eq!(summary.programs[0].program.as_deref(), Some("npm"));
    assert!(summary.programs[0].launcher.is_none());
    assert!(summary.paths.contains(&"/srv/app".to_owned()));
}

#[test]
fn verify_pipeline_is_planned_as_sub_plan() {
    let registry = common::registry();
    let plan = nurt_plan::plan(
        &common::golden("task-12-replace-verify-rollback"),
        &registry,
    )
    .unwrap();
    let operation = plan.operations().next().unwrap();
    let check = operation.pipeline.verify().unwrap();
    assert_eq!(check.effect, EffectKind::Process);
    assert_eq!(
        operation.pipeline.effect_stage().unwrap().command(),
        "replace"
    );
    let names: Vec<&str> = plan.stages().iter().map(|stage| stage.command()).collect();
    assert_eq!(names, ["search", "replace", "verify", "run", "assert"]);
}

#[test]
fn named_result_is_planned_before_its_use() {
    let registry = common::registry();
    let plan = nurt_plan::plan(&common::golden("task-06-named-result-count"), &registry).unwrap();
    let binding = plan.lets().next().unwrap();
    assert_eq!(binding.name, "recent");
    assert!(binding.pipeline.is_read_only());
    let operation = plan.operations().next().unwrap();
    assert_eq!(operation.pipeline.head.as_ref().unwrap().name, "recent");
    assert_eq!(operation.pipeline.stages[0].index, 1);
}

#[test]
fn after_stage_without_effect_is_rejected() {
    let error = nurt_plan::plan(
        &common::golden("invalid/after-without-effect"),
        &common::registry(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::AstSchema);
}

#[test]
fn effect_inside_named_result_is_split() {
    let error = nurt_plan::plan(
        &common::golden("invalid/effect-in-let"),
        &common::registry(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(error.next_steps.len(), 2);
    assert!(error.next_steps[0].starts_with("find "));
    assert!(error.next_steps[0].ends_with(" |> remove"));
    assert_eq!(error.next_steps[1], "@x");
}

#[test]
fn stage_without_phase_does_not_read() {
    let path = common::golden_dir().join("invalid/missing-phase.json");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(serde_json::from_str::<BoundDocument>(&text).is_err());
}

#[test]
fn plan_round_trips_through_json() {
    let registry = common::registry();
    let plan = nurt_plan::plan(
        &common::golden("task-12-replace-verify-rollback"),
        &registry,
    )
    .unwrap();
    let json = serde_json::to_value(&plan).unwrap();
    assert_eq!(json["statements"][0]["t"], "operation");
    assert_eq!(json["statements"][0]["pipeline"]["stages"][2]["step"], "6");
    let back: nurt_plan::Plan = serde_json::from_value(json).unwrap();
    assert_eq!(back, plan);
}
