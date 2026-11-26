// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Order of phases checked by the planner independently of binding: the
//! corpus of pipelines rejected with `E_ONE_EFFECT`, malformed phases and the
//! stages allowed after the effect.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests fail by panicking"
)]

mod common;

use common::{
    after, binding, checking, document, find, from_ref, given, mutate, operation, path, pipeline,
    run, single, source, transform, verify,
};
use nurt_contracts::ast::Span;
use nurt_contracts::bound::{BoundDocument, Phase};
use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::stage::ExecStep;

fn rejected(document: &BoundDocument) -> Error {
    nurt_plan::plan(document, &common::registry()).expect_err("the program is rejected")
}

fn move_into(dir: &str) -> nurt_contracts::bound::BoundStage {
    mutate("move", vec![given("into", path(dir))])
}

#[test]
fn second_effect_stage_is_split_into_two_calls() {
    let program = single(pipeline(
        find("/a"),
        vec![mutate("remove", Vec::new()), move_into("/b")],
    ));
    let error = rejected(&program);
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(error.stage(), Some(Stage::Bind));
    assert_eq!(
        error.next_steps,
        [
            "find path: \"/a\" |> remove",
            "find path: \"/a\" |> move into: \"/b\""
        ]
    );
}

#[test]
fn run_followed_by_change_is_rejected() {
    let write = mutate("write", vec![given("path", path("/a/out.txt"))]);
    let error = rejected(&single(pipeline(run(&["make"]), vec![write])));
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(error.next_steps.len(), 2);
    assert_eq!(error.next_steps[1], "write path: \"/a/out.txt\"");
}

#[test]
fn stage_not_allowed_after_effect_is_rejected() {
    for phase in [Phase::After, Phase::Transform] {
        let distinct = common::stage("distinct", EffectKind::Read, phase, Vec::new());
        let program = single(pipeline(
            find("/a"),
            vec![mutate("remove", Vec::new()), distinct],
        ));
        let error = rejected(&program);
        assert_eq!(error.code(), ErrorCode::OneEffect);
        assert_eq!(error.next_steps, ["find path: \"/a\" |> remove"]);
        assert!(error.message.contains("distinct"));
    }
}

#[test]
fn allowed_stage_after_effect_needs_phase_after() {
    let program = single(pipeline(
        find("/a"),
        vec![
            mutate("remove", Vec::new()),
            transform("filter", Vec::new()),
        ],
    ));
    assert_eq!(rejected(&program).code(), ErrorCode::AstSchema);
}

#[test]
fn effect_inside_named_result_lists_separate_calls() {
    let program = document(vec![
        binding(
            "x",
            pipeline(find("/a"), vec![mutate("remove", Vec::new())]),
        ),
        operation(from_ref("x", vec![transform("count", Vec::new())])),
    ]);
    let error = rejected(&program);
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(
        error.next_steps,
        ["find path: \"/a\" |> remove", "@x |> count"]
    );
}

#[test]
fn change_inside_verify_is_rejected() {
    let check = pipeline(find("/a"), vec![mutate("remove", Vec::new())]);
    let program = single(pipeline(find("/a"), vec![move_into("/b"), verify(check)]));
    let error = rejected(&program);
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert!(error.message.contains("verify"));
}

#[test]
fn stage_after_verify_is_rejected() {
    let check = pipeline(run(&["go", "build"]), Vec::new());
    let program = single(pipeline(
        find("/a"),
        vec![move_into("/b"), verify(check), after("count", Vec::new())],
    ));
    let error = rejected(&program);
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(error.next_steps.len(), 1);
    assert!(error.next_steps[0].contains("|> verify check: (run command: [\"go\", \"build\"])"));
}

#[test]
fn second_run_inside_verify_is_rejected() {
    let check = pipeline(run(&["make"]), vec![run(&["make", "test"])]);
    let program = single(pipeline(find("/a"), vec![move_into("/b"), verify(check)]));
    let error = rejected(&program);
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert!(error.message.contains("second effect stage"));
}

#[test]
fn local_command_inside_verify_is_a_change() {
    let local = common::stage("run", EffectKind::Local, Phase::Run, Vec::new());
    let check = pipeline(local, Vec::new());
    let program = single(pipeline(find("/a"), vec![move_into("/b"), verify(check)]));
    let error = rejected(&program);
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert!(error.message.contains("inside `verify`"));
}

#[test]
fn verify_inside_a_named_result_is_malformed() {
    let check = pipeline(run(&["true"]), Vec::new());
    let program = document(vec![
        binding("x", pipeline(find("/a"), vec![verify(check)])),
        operation(from_ref("x", vec![transform("count", Vec::new())])),
    ]);
    assert_eq!(rejected(&program).code(), ErrorCode::AstSchema);
}

#[test]
fn after_stage_inside_a_named_result_is_malformed() {
    let program = document(vec![
        binding("x", pipeline(find("/a"), vec![after("count", Vec::new())])),
        operation(from_ref("x", vec![transform("count", Vec::new())])),
    ]);
    assert_eq!(rejected(&program).code(), ErrorCode::AstSchema);
}

#[test]
fn effect_with_reading_phase_is_malformed() {
    let remove = common::stage("remove", EffectKind::Local, Phase::Transform, Vec::new());
    let error = rejected(&single(pipeline(find("/a"), vec![remove])));
    assert_eq!(error.code(), ErrorCode::AstSchema);
    assert_eq!(error.stage(), Some(Stage::Parse));
}

#[test]
fn misplaced_phases_are_malformed() {
    let cases = [
        pipeline(find("/a"), vec![find("/b")]),
        pipeline(transform("filter", Vec::new()), Vec::new()),
        pipeline(
            find("/a"),
            vec![verify(pipeline(run(&["true"]), Vec::new()))],
        ),
        pipeline(
            find("/a"),
            vec![common::stage(
                "filter",
                EffectKind::Read,
                Phase::Verify,
                Vec::new(),
            )],
        ),
    ];
    for case in cases {
        assert_eq!(rejected(&single(case)).code(), ErrorCode::AstSchema);
    }
    let nested = verify(pipeline(run(&["true"]), Vec::new()));
    let check = pipeline(run(&["make"]), vec![nested]);
    let program = single(pipeline(find("/a"), vec![move_into("/b"), verify(check)]));
    assert_eq!(rejected(&program).code(), ErrorCode::AstSchema);
}

#[test]
fn verify_without_pipeline_is_missing_parameter() {
    let bare = common::stage("verify", EffectKind::Read, Phase::Verify, Vec::new());
    let program = single(pipeline(find("/a"), vec![move_into("/b"), bare]));
    assert_eq!(rejected(&program).code(), ErrorCode::MissingParam);
}

#[test]
fn unknown_command_is_rejected() {
    let program = single(pipeline(
        find("/a"),
        vec![transform("frobnicate", Vec::new())],
    ));
    assert_eq!(rejected(&program).code(), ErrorCode::UnknownCommand);
}

#[test]
fn error_points_at_the_offending_stage() {
    let mut second = move_into("/b");
    second.span = Some(Span {
        line: 2,
        column: 4,
        end_line: 2,
        end_column: 18,
    });
    let program = single(pipeline(
        find("/a"),
        vec![mutate("remove", Vec::new()), second],
    ));
    let at = rejected(&program).at.unwrap();
    assert_eq!(
        (at.line, at.column, at.length),
        (Some(2), Some(4), Some(14))
    );
    assert_eq!(at.stage, Some(Stage::Bind));
}

#[test]
fn every_stage_allowed_after_effect_plans_in_step_after() {
    let names = [
        "assert", "filter", "select", "first", "last", "skip", "count", "sort", "group",
    ];
    for name in names {
        let program = single(pipeline(
            run(&["go", "test"]),
            vec![after(name, Vec::new())],
        ));
        let plan = nurt_plan::plan(&program, &common::registry())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let stages = &plan.operations().next().unwrap().pipeline.stages;
        assert_eq!(stages[1].step, ExecStep::After, "{name}");
        assert_eq!(stages[1].steps(), [ExecStep::After]);
    }
}

#[test]
fn checks_before_effect_plan_in_step_check() {
    let assert = checking("assert", vec![given("count", common::range(2, 2))]);
    let program = single(pipeline(
        find("/a"),
        vec![assert, mutate("remove", Vec::new())],
    ));
    let plan = nurt_plan::plan(&program, &common::registry()).unwrap();
    let pipeline = &plan.operations().next().unwrap().pipeline;
    assert_eq!(pipeline.stages_in(ExecStep::Check).count(), 1);
    let effect = pipeline.effect_stage().unwrap();
    assert_eq!(effect.steps(), [ExecStep::Plan, ExecStep::Apply]);
    assert_eq!(effect.frame().step, ExecStep::Plan);
}

#[test]
fn script_plans_each_pipeline_as_its_own_operation() {
    let program = document(vec![
        binding("logs", pipeline(find("/var/log"), Vec::new())),
        operation(from_ref("logs", vec![mutate("remove", Vec::new())])),
        operation(pipeline(run(&["sync"]), Vec::new())),
        operation(pipeline(source("history", Vec::new()), Vec::new())),
    ]);
    let plan = nurt_plan::plan(&program, &common::registry()).unwrap();
    assert!(plan.is_script());
    let indexes: Vec<u32> = plan.operations().map(|operation| operation.index).collect();
    assert_eq!(indexes, [0, 1, 2]);
    let effects: Vec<EffectKind> = plan
        .operations()
        .map(|operation| operation.pipeline.effect)
        .collect();
    assert_eq!(
        effects,
        [EffectKind::Local, EffectKind::Process, EffectKind::Read]
    );
    assert_eq!(plan.effect, EffectKind::Process);
    assert_eq!(plan.lets().count(), 1);
}

#[test]
fn waiting_stage_marks_the_plan() {
    let mut sleeping = run(&["sleep", "60"]);
    sleeping.waits = true;
    let plan =
        nurt_plan::plan(&single(pipeline(sleeping, Vec::new())), &common::registry()).unwrap();
    assert!(plan.waits());
    let quiet = nurt_plan::plan(
        &single(pipeline(find("/a"), Vec::new())),
        &common::registry(),
    )
    .unwrap();
    assert!(!quiet.waits());
}
