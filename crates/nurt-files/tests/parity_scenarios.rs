// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the parity scenario files.

// Test code unwraps results so that a failure aborts the test with the original error.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support_parity;

use std::path::PathBuf;

use support_parity::{
    AfterEntry, Class, Divergence, Expect, FieldCheck, Fixture, FixtureFile, FsEntry, HostOnly,
    Model, Pending, PendingScenario, Row, Scenario, ScenarioFile, Source, generator_error,
};

const SOURCE_CASES: u32 = 293;
const SOURCE_FILES: usize = 17;

fn data_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("tests/parity")
}

fn model() -> Model {
    Model::load(&data_dir()).unwrap()
}

#[test]
fn scenario_files_have_no_violations() {
    let problems = model().validate();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_source_case_is_a_scenario_or_a_host_only_entry() {
    let model = model();
    assert_eq!(model.sources.len(), SOURCE_FILES);
    assert_eq!(
        model.sources.iter().map(|s| s.cases).sum::<u32>(),
        SOURCE_CASES
    );
}

#[test]
fn scenario_files_are_named_after_their_source() {
    let model = model();
    for source in &model.sources {
        if let Some(name) = &source.scenarios {
            let stem = source
                .path
                .rsplit('/')
                .next()
                .unwrap()
                .trim_end_matches(".test.ts");
            assert!(
                name.trim_end_matches(".toml").ends_with(stem),
                "{name} does not match {}",
                source.path
            );
        }
    }
}

#[test]
fn provides_at_least_two_hundred_scenarios() {
    let model = model();
    let scenarios = model.scenarios().count();
    assert!(scenarios >= 200, "only {scenarios} scenarios");
}

#[test]
fn generators_accept_documented_directives_only() {
    for valid in [
        "big_source:80",
        "png:640x480",
        "pdf:text",
        "repeat:x*2500",
        "webp:VP8X,320x240",
        "webp_image:320x240",
        "bytes:00ff",
        "notebook:flat",
    ] {
        assert_eq!(generator_error(valid), None, "{valid}");
    }
    for invalid in [
        "big_source",
        "png:640",
        "repeat:x",
        "webp:VP9,1x1",
        "webp_image:320",
        "bytes:0",
        "notebook:none",
        "other:1",
    ] {
        assert!(generator_error(invalid).is_some(), "{invalid}");
    }
}

fn base_model() -> Model {
    let scenario = Scenario {
        name: "reads a file".into(),
        case: None,
        class: Class::Direct,
        parity_rows: vec!["RD-01".into()],
        open_gap: None,
        fs_before: vec![FsEntry {
            path: Some("a.txt".into()),
            content: Some("a\n".into()),
            ..FsEntry::default()
        }],
        native_call: "Read".into(),
        nurt_program: "read \"{root}/a.txt\"".into(),
        expect: Expect {
            fs_unchanged: true,
            fs_after: Vec::new(),
            fields: vec![FieldCheck {
                field: "content.lines".into(),
                value: Some(toml::Value::Integer(1)),
                contains: None,
                excludes: None,
            }],
        },
    };
    let mut files = std::collections::BTreeMap::new();
    files.insert(
        "read.toml".to_string(),
        ScenarioFile {
            source: "tests/read.test.ts".into(),
            cases: 2,
            scenario: vec![scenario],
        },
    );
    Model {
        rows: vec![
            Row {
                id: "RD-01".into(),
                state: "equal".into(),
                gap: None,
            },
            Row {
                id: "RD-02".into(),
                state: "gap".into(),
                gap: Some("LP-04".into()),
            },
            Row {
                id: "RD-03".into(),
                state: "dropped".into(),
                gap: None,
            },
        ],
        fixtures: vec![Fixture {
            name: "tree".into(),
            file: vec![FixtureFile {
                path: "a.txt".into(),
                content: Some("a\n".into()),
                generate: None,
            }],
        }],
        files,
        host_only: vec![HostOnly {
            name: "limit".into(),
            source: "tests/read.test.ts".into(),
            owner: "host".into(),
            cases: 1,
            reason: "Character budget.".into(),
        }],
        sources: vec![Source {
            path: "tests/read.test.ts".into(),
            cases: 2,
            scenarios: Some("read.toml".into()),
        }],
        pending: Vec::new(),
        pending_scenarios: Vec::new(),
        divergences: Vec::new(),
    }
}

#[test]
fn rejects_listed_scenarios_that_do_not_exist_or_are_listed_twice() {
    let mut model = base_model();
    model.pending_scenarios.push(PendingScenario {
        name: "reads a file".into(),
        reason: "command not merged: read".into(),
    });
    assert_eq!(model.validate(), Vec::<String>::new());
    model.divergences.push(Divergence {
        name: "reads a file".into(),
        command: "read".into(),
        detail: "content.lines: expected 1, found 2".into(),
    });
    assert_reports(&model, "both pending and diverging");
    model.divergences[0].name = "missing".into();
    assert_reports(&model, "divergence missing names no scenario");
    model.pending_scenarios.push(PendingScenario {
        name: "reads a file".into(),
        reason: " ".into(),
    });
    assert_reports(&model, "pending scenario reads a file needs a reason");
    assert_reports(&model, "pending scenario reads a file is listed twice");
}

fn scenario_mut(model: &mut Model) -> &mut Scenario {
    &mut model.files.get_mut("read.toml").unwrap().scenario[0]
}

fn assert_reports(model: &Model, fragment: &str) {
    let problems = model.validate();
    assert!(
        problems.iter().any(|p| p.contains(fragment)),
        "expected {fragment:?} in {problems:?}"
    );
}

#[test]
fn accepts_a_consistent_model() {
    assert_eq!(base_model().validate(), Vec::<String>::new());
}

#[test]
fn rejects_an_unknown_parity_row() {
    let mut model = base_model();
    scenario_mut(&mut model).parity_rows = vec!["RD-99".into()];
    assert_reports(&model, "unknown row RD-99");
}

#[test]
fn rejects_a_scenario_for_a_gap_row() {
    let mut model = base_model();
    scenario_mut(&mut model).parity_rows.push("RD-02".into());
    assert_reports(&model, "row RD-02 is gap");
}

#[test]
fn rejects_a_row_without_scenario_or_pending_entry() {
    let mut model = base_model();
    model.rows.push(Row {
        id: "RD-04".into(),
        state: "partial".into(),
        gap: None,
    });
    assert_reports(&model, "row RD-04 has neither");
    model.pending.push(Pending {
        id: "RD-04".into(),
        reason: "Awaits the command.".into(),
    });
    assert_eq!(model.validate(), Vec::<String>::new());
}

#[test]
fn rejects_a_gap_row_without_open_item() {
    let mut model = base_model();
    model.rows[1].gap = None;
    assert_reports(&model, "gap row RD-02 lacks an open item");
}

#[test]
fn rejects_a_pending_row_that_has_a_scenario() {
    let mut model = base_model();
    model.pending.push(Pending {
        id: "RD-01".into(),
        reason: "Awaits.".into(),
    });
    assert_reports(&model, "pending row RD-01 has a scenario");
}

#[test]
fn rejects_duplicate_scenario_names() {
    let mut model = base_model();
    let copy = scenario_mut(&mut model).clone();
    model
        .files
        .get_mut("read.toml")
        .unwrap()
        .scenario
        .push(copy);
    assert_reports(&model, "name is not unique");
}

#[test]
fn rejects_unknown_fixture_and_absolute_paths() {
    let mut model = base_model();
    scenario_mut(&mut model).fs_before = vec![
        FsEntry {
            fixture: Some("missing".into()),
            ..FsEntry::default()
        },
        FsEntry {
            path: Some("/etc/passwd".into()),
            content: Some(String::new()),
            ..FsEntry::default()
        },
    ];
    assert_reports(&model, "unknown fixture missing");
    assert_reports(&model, "must be relative and normalized");
}

#[test]
fn rejects_programs_with_absolute_paths() {
    let mut model = base_model();
    scenario_mut(&mut model).nurt_program = "read \"/tmp/a.txt\"".into();
    assert_reports(&model, "absolute path");
}

#[test]
fn rejects_scenarios_that_expect_nothing() {
    let mut model = base_model();
    scenario_mut(&mut model).expect = Expect::default();
    assert_reports(&model, "nothing is expected");
}

#[test]
fn rejects_ambiguous_assertions() {
    let mut model = base_model();
    let expect = &mut scenario_mut(&mut model).expect;
    expect.fs_unchanged = false;
    expect.fs_after = vec![AfterEntry {
        path: "a.txt".into(),
        content: Some("a".into()),
        contains: Some("a".into()),
        ..AfterEntry::default()
    }];
    assert_reports(&model, "needs exactly one assertion");
}

#[test]
fn rejects_field_checks_without_a_comparison() {
    let mut model = base_model();
    scenario_mut(&mut model).expect.fields[0].value = None;
    assert_reports(&model, "needs one comparison");
}

#[test]
fn rejects_a_case_count_that_does_not_add_up() {
    let mut model = base_model();
    model.host_only.clear();
    assert_reports(&model, "1 scenario cases and 0 host-only cases, expected 2");
}

#[test]
fn rejects_a_case_listed_as_scenario_and_host_only() {
    let mut model = base_model();
    model.host_only[0].name = "reads a file".into();
    assert_reports(&model, "both a scenario and host-only");
}

#[test]
fn rejects_an_unknown_host_owner() {
    let mut model = base_model();
    model.host_only[0].owner = "nobody".into();
    assert_reports(&model, "unknown owner");
}

#[test]
fn rejects_an_unknown_open_item() {
    let mut model = base_model();
    scenario_mut(&mut model).open_gap = Some("LP-77".into());
    assert_reports(&model, "open item LP-77 is not tracked");
    scenario_mut(&mut model).open_gap = Some("LP-04".into());
    assert_eq!(model.validate(), Vec::<String>::new());
}
