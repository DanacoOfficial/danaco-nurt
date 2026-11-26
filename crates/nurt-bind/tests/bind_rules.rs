// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding rules: signature order and defaults, casts, expansion of source
//! forms, phases and the rule of one effect, and the errors of stage `1`.

// Test failures report through panics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bind_support;

use bind_support::{
    arg, bin, bind_with, bound, catalog, dur, expected_names, failed, hash, ident, int,
    let_statement, linux, list, named, negative, pipeline, pos, program, ref_pipeline, regex,
    single, stage, stages_of, sub, text,
};
use nurt_contracts::ast::{BinOp, Expr, Span, Statement};
use nurt_contracts::bound::{BoundExpr, Phase};
use nurt_contracts::error::{ErrorCode, Stage as ExecStage};
use nurt_contracts::kinds::EffectKind;
use nurt_contracts::spec::Os;

fn path(v: &str) -> BoundExpr {
    BoundExpr::Path { v: v.to_owned() }
}

fn enumeration(v: &str) -> BoundExpr {
    BoundExpr::Enum { v: v.to_owned() }
}

#[test]
fn writes_every_parameter_in_signature_order() {
    let tree = bound(&single(stage("find", vec![pos(text("/srv/app"))]), vec![]));
    let find = stages_of(&tree, 0)[0];
    let names: Vec<&str> = find.args.iter().map(|arg| arg.name.as_str()).collect();
    let signature: Vec<&str> = catalog()
        .command("find")
        .unwrap()
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(names, signature);
    assert_eq!(arg(find, "path").value, path("/srv/app"));
    assert!(!arg(find, "path").default);
    assert_eq!(
        arg(find, "glob").value,
        BoundExpr::Glob { v: "**".to_owned() }
    );
    assert!(arg(find, "glob").default);
    assert_eq!(arg(find, "depth").value, BoundExpr::Null);
    assert!(arg(find, "depth").default);
    assert_eq!(find.phase, Phase::Source);
    assert_eq!(find.effect, EffectKind::Read);
}

#[test]
fn named_and_positional_arguments_bind_alike() {
    let positional = single(
        stage("find", vec![pos(text("/srv/app")), pos(text("**/*.go"))]),
        vec![],
    );
    let by_name = single(
        stage(
            "find",
            vec![
                named("glob", text("**/*.go")),
                named("path", text("/srv/app")),
            ],
        ),
        vec![],
    );
    assert_eq!(hash(&bound(&positional)), hash(&bound(&by_name)));
}

#[test]
fn expands_a_source_form_to_an_info_head() {
    let short = bound(&single(stage("remove", vec![pos(text("/srv/a"))]), vec![]));
    let long = bound(&single(
        stage("info", vec![pos(text("/srv/a"))]),
        vec![stage("remove", vec![])],
    ));
    assert_eq!(hash(&short), hash(&long));
    let stages = stages_of(&short, 0);
    assert_eq!(stages[0].name, "info");
    assert_eq!(
        arg(stages[0], "path").value,
        BoundExpr::List {
            items: vec![path("/srv/a")]
        }
    );
    assert_eq!(stages[1].name, "remove");
    assert_eq!(stages[1].phase, Phase::Mutate);
    assert_eq!(arg(stages[1], "path").value, BoundExpr::Null);
    assert!(arg(stages[1], "path").default);
}

#[test]
fn expands_copy_with_several_sources() {
    let tree = bound(&single(
        stage(
            "copy",
            vec![
                pos(text("/srv/a")),
                pos(text("/srv/b")),
                pos(text("/srv/dst")),
            ],
        ),
        vec![],
    ));
    let stages = stages_of(&tree, 0);
    assert_eq!(
        arg(stages[0], "path").value,
        BoundExpr::List {
            items: vec![path("/srv/a"), path("/srv/b")]
        }
    );
    assert_eq!(arg(stages[1], "to").value, path("/srv/dst"));
    assert!(!arg(stages[1], "to").default);
    assert_eq!(arg(stages[1], "from").value, BoundExpr::Null);
}

#[test]
fn expands_copy_with_a_named_destination() {
    let tree = bound(&single(
        stage(
            "copy",
            vec![pos(text("/srv/app")), named("into", text("/srv/backup"))],
        ),
        vec![],
    ));
    let copy = stages_of(&tree, 0)[1];
    assert_eq!(arg(copy, "into").value, path("/srv/backup"));
    assert_eq!(arg(copy, "to").value, BoundExpr::Null);
    assert!(arg(copy, "to").default);
}

#[test]
fn missing_write_mode_offers_ready_programs() {
    let error = failed(&single(
        stage("write", vec![pos(text("/srv/a.txt")), pos(text("x"))]),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::MissingParam);
    assert_eq!(error.error().stage(), Some(ExecStage::Bind));
    let fixes = &error.error().fix;
    assert_eq!(fixes.len(), 4);
    assert!(fixes.iter().any(|fix| fix.contains("mode: append")));
}

#[test]
fn missing_create_type_is_reported() {
    let error = failed(&single(stage("create", vec![pos(text("/srv/a"))]), vec![]));
    assert_eq!(error.code(), ErrorCode::MissingParam);
    assert_eq!(expected_names(&error), ["type"]);
}

#[test]
fn assert_needs_exactly_one_condition() {
    let none = failed(&single(
        stage("find", vec![pos(text("/srv"))]),
        vec![stage("assert", vec![])],
    ));
    assert_eq!(none.code(), ErrorCode::MissingParam);
    assert!(expected_names(&none).contains(&"count".to_owned()));
    let two = failed(&single(
        stage("find", vec![pos(text("/srv"))]),
        vec![stage(
            "assert",
            vec![named("count", int(1)), named("paths", int(2))],
        )],
    ));
    assert_eq!(two.code(), ErrorCode::ParamConflict);
}

#[test]
fn unknown_command_lists_nearest_names() {
    let error = failed(&single(stage("remvoe", vec![pos(text("/srv/a"))]), vec![]));
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
    let names = expected_names(&error);
    assert_eq!(names.first().map(String::as_str), Some("remove"));
    assert!(names.len() <= 8);
}

#[test]
fn unknown_parameter_lists_nearest_names() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv")), named("glb", text("*"))]),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::UnknownParam);
    assert!(expected_names(&error).contains(&"glob".to_owned()));
}

#[test]
fn parameter_of_the_source_form_is_rejected_in_a_stage() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv"))]),
        vec![stage("remove", vec![named("path", text("/srv/a"))])],
    ));
    assert_eq!(error.code(), ErrorCode::UnknownParam);
}

#[test]
fn unknown_enumeration_value_lists_values() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv")), named("type", ident("fil"))]),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::UnknownValue);
    assert_eq!(
        expected_names(&error).first().map(String::as_str),
        Some("file")
    );
}

#[test]
fn relative_path_needs_base() {
    let document = single(stage("find", vec![pos(text("src"))]), vec![]);
    assert_eq!(failed(&document).code(), ErrorCode::PathRelative);
    let tree = bind_with(&document, &linux().with_base("/srv/app")).unwrap();
    assert_eq!(
        arg(stages_of(&tree, 0)[0], "path").value,
        path("/srv/app/src")
    );
}

#[test]
fn drive_letter_is_foreign_on_linux() {
    let document = single(stage("find", vec![pos(text("C:/proj"))]), vec![]);
    assert_eq!(failed(&document).code(), ErrorCode::PathForeign);
    let windows = nurt_bind::BindContext::new(Os::Windows);
    let tree = bind_with(&document, &windows).unwrap();
    assert_eq!(arg(stages_of(&tree, 0)[0], "path").value, path("C:/proj"));
}

#[test]
fn home_directory_expands() {
    let document = single(stage("find", vec![pos(text("~/notes"))]), vec![]);
    assert_eq!(failed(&document).code(), ErrorCode::PathRelative);
    let tree = bind_with(&document, &linux().with_home("/home/user")).unwrap();
    assert_eq!(
        arg(stages_of(&tree, 0)[0], "path").value,
        path("/home/user/notes")
    );
}

#[test]
fn run_working_directory_defaults_to_base() {
    let document = single(stage("run", vec![pos(list(vec![text("make")]))]), vec![]);
    let error = failed(&document);
    assert_eq!(error.code(), ErrorCode::MissingParam);
    assert_eq!(expected_names(&error), ["cwd"]);
    let tree = bind_with(&document, &linux().with_base("/srv/app")).unwrap();
    let run = stages_of(&tree, 0)[0];
    assert_eq!(arg(run, "cwd").value, path("/srv/app"));
    assert!(arg(run, "cwd").default);
    assert_eq!(
        arg(run, "watch").value,
        BoundExpr::List {
            items: vec![path("/srv/app")]
        }
    );
    assert_eq!(run.phase, Phase::Run);
    assert_eq!(run.effect, EffectKind::Process);
}

#[test]
fn long_wait_marks_the_stage() {
    let run = |extra: Vec<nurt_contracts::ast::Arg>| {
        let mut args = vec![
            pos(list(vec![text("make")])),
            named("cwd", text("/srv/app")),
        ];
        args.extend(extra);
        bound(&single(stage("run", args), vec![]))
    };
    assert!(!stages_of(&run(vec![]), 0)[0].waits);
    assert!(!stages_of(&run(vec![named("wait", dur(3_000))]), 0)[0].waits);
    assert!(stages_of(&run(vec![named("wait", dur(10_000))]), 0)[0].waits);
    assert!(stages_of(&run(vec![named("wait", ident("forever"))]), 0)[0].waits);
    let jobs = bound(&single(
        stage("jobs", vec![pos(text("j1")), named("wait", dur(1_000))]),
        vec![],
    ));
    assert!(stages_of(&jobs, 0)[0].waits);
}

#[test]
fn exit_code_after_run_is_a_range_in_phase_after() {
    let tree = bound(&single(
        stage(
            "run",
            vec![
                pos(list(vec![text("go"), text("test")])),
                named("cwd", text("/srv/app")),
            ],
        ),
        vec![stage("assert", vec![named("exit_code", int(0))])],
    ));
    let check = stages_of(&tree, 0)[1];
    assert_eq!(check.phase, Phase::After);
    assert_eq!(
        arg(check, "exit_code").value,
        BoundExpr::Range {
            lo: Some(0),
            hi: Some(0)
        }
    );
}

#[test]
fn test_results_after_run_are_in_phase_after() {
    let tree = bound(&single(
        stage("test.run", vec![pos(text("/srv/app"))]),
        vec![stage(
            "filter",
            vec![pos(bin(BinOp::Eq, ident("status"), text("failed")))],
        )],
    ));
    let stages = stages_of(&tree, 0);
    assert_eq!(stages[0].phase, Phase::Run);
    assert_eq!(stages[1].phase, Phase::After);
}

#[test]
fn folds_constant_negation() {
    let tree = bound(&single(
        stage("jobs", vec![]),
        vec![stage(
            "filter",
            vec![pos(bin(BinOp::Ne, ident("exit_code"), negative(int(1))))],
        )],
    ));
    let condition = &arg(stages_of(&tree, 0)[1], "condition").value;
    let BoundExpr::Bin { l, r, .. } = condition else {
        panic!("a binary expression is expected");
    };
    assert_eq!(
        **l,
        BoundExpr::Field {
            name: "exit_code".to_owned(),
            of: None
        }
    );
    assert_eq!(**r, BoundExpr::Int { v: -1 });
}

#[test]
fn second_effect_is_rejected_with_separate_calls() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv/a"))]),
        vec![
            stage("remove", vec![]),
            stage("copy", vec![named("into", text("/srv/b"))]),
        ],
    ));
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(error.error().next_steps.len(), 2);
}

#[test]
fn stage_after_the_effect_must_inspect_its_records() {
    let error = failed(&single(
        stage(
            "run",
            vec![pos(list(vec![text("make")])), named("cwd", text("/srv"))],
        ),
        vec![stage("lines", vec![])],
    ));
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert!(expected_names(&error).contains(&"assert".to_owned()));
}

#[test]
fn let_pipeline_has_no_effect() {
    let error = failed(&program(vec![let_statement(
        "gone",
        pipeline(
            stage("find", vec![pos(text("/srv/a"))]),
            vec![stage("remove", vec![])],
        ),
    )]));
    assert_eq!(error.code(), ErrorCode::OneEffect);
    assert_eq!(error.error().next_steps.len(), 1);
}

#[test]
fn verify_is_the_last_stage() {
    let checked = || {
        vec![
            stage("replace", vec![pos(text("b"))]),
            stage(
                "verify",
                vec![pos(sub(pipeline(stage("count", vec![]), vec![])))],
            ),
        ]
    };
    let search = || stage("search", vec![pos(regex("a")), pos(text("/srv"))]);
    let tree = bound(&single(search(), checked()));
    let stages = stages_of(&tree, 0);
    assert_eq!(stages[2].phase, Phase::Verify);
    let mut more = checked();
    more.push(stage("count", vec![]));
    assert_eq!(failed(&single(search(), more)).code(), ErrorCode::OneEffect);
}

#[test]
fn command_name_in_a_pipeline_place_is_a_sub_pipeline() {
    let tree = bound(&single(
        stage("search", vec![pos(regex("a")), pos(text("/srv"))]),
        vec![
            stage("replace", vec![pos(text("b"))]),
            stage("verify", vec![pos(ident("count"))]),
        ],
    ));
    let check = &arg(stages_of(&tree, 0)[2], "check").value;
    assert!(matches!(check, BoundExpr::Sub { .. }));
}

#[test]
fn filter_cannot_start_a_pipeline() {
    let error = failed(&single(
        stage("filter", vec![pos(bin(BinOp::Gt, ident("size"), int(1)))]),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::Type);
}

#[test]
fn bare_word_in_a_text_place_comes_with_the_program_that_quotes_it() {
    let written = Expr::Ident {
        v: "run".to_owned(),
        span: Some(Span {
            line: 1,
            column: 6,
            end_line: 1,
            end_column: 9,
        }),
    };
    let error = failed(&single(stage("help", vec![pos(written)]), vec![]));
    assert_eq!(error.code(), ErrorCode::Type);
    assert_eq!(error.error().fix, ["help \"run\""]);
}

#[test]
fn bare_word_without_a_span_has_no_quoted_program() {
    let error = failed(&single(stage("help", vec![pos(ident("run"))]), vec![]));
    assert_eq!(error.code(), ErrorCode::Type);
    assert!(error.error().fix.is_empty());
}

#[test]
fn too_many_positional_arguments_are_rejected() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv"))]),
        vec![stage("first", vec![pos(int(1)), pos(int(2))])],
    ));
    assert_eq!(error.code(), ErrorCode::Arity);
}

#[test]
fn parameter_given_twice_is_rejected() {
    let error = failed(&single(
        stage(
            "find",
            vec![named("path", text("/srv/a")), named("path", text("/srv/b"))],
        ),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::Arity);
}

#[test]
fn reference_needs_a_named_result() {
    let document = program(vec![Statement::Pipeline(ref_pipeline(
        "recent",
        vec![stage("count", vec![])],
    ))]);
    assert_eq!(failed(&document).code(), ErrorCode::UnknownName);
    assert!(bind_with(&document, &linux().with_let("recent")).is_ok());
    let defined = program(vec![
        let_statement(
            "recent",
            pipeline(stage("find", vec![pos(text("/srv"))]), vec![]),
        ),
        Statement::Pipeline(ref_pipeline("recent", vec![stage("count", vec![])])),
    ]);
    assert_eq!(stages_of(&bound(&defined), 1)[0].phase, Phase::Transform);
}

#[test]
fn command_of_another_system_is_rejected() {
    let error = failed(&single(
        stage("windows.cim", vec![named("class", text("Win32_Process"))]),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::OsOnly);
}

#[test]
fn pattern_mode_follows_the_pattern() {
    let mode = |pattern| {
        let tree = bound(&single(
            stage("search", vec![pos(pattern), pos(text("/srv"))]),
            vec![],
        ));
        let search = stages_of(&tree, 0)[0];
        assert!(arg(search, "mode").default);
        arg(search, "mode").value.clone()
    };
    assert_eq!(mode(text("x")), enumeration("literal"));
    assert_eq!(mode(regex("x")), enumeration("regex"));
}

#[test]
fn removing_lines_binds_the_stage_form() {
    let tree = bound(&single(
        stage("lines", vec![pos(text("/srv/app/config.ini"))]),
        vec![
            stage(
                "filter",
                vec![pos(bin(BinOp::Eq, ident("text"), text("debug")))],
            ),
            stage("remove", vec![]),
        ],
    ));
    let remove = stages_of(&tree, 0)[2];
    assert_eq!(remove.effect, EffectKind::Local);
    assert_eq!(arg(remove, "path").value, BoundExpr::Null);
}

#[test]
fn tree_of_another_major_version_is_rejected() {
    let mut document = single(stage("find", vec![pos(text("/srv"))]), vec![]);
    document.nurt = "2.0".to_owned();
    assert_eq!(failed(&document).code(), ErrorCode::AstVersion);
}

#[test]
fn invalid_glob_is_rejected() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv")), named("glob", text("a[b"))]),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn texts_convert_to_instants_and_sizes() {
    let tree = bound(&single(
        stage(
            "attributes",
            vec![
                pos(text("/srv/a")),
                named("modified", text("2026-03-01T14:00:00+02:00")),
            ],
        ),
        vec![],
    ));
    let attributes = stages_of(&tree, 0)[1];
    assert_eq!(
        arg(attributes, "modified").value,
        BoundExpr::Instant {
            v: "2026-03-01T12:00:00Z".to_owned()
        }
    );
    let run = bound(&single(
        stage(
            "run",
            vec![
                pos(list(vec![text("make")])),
                named("cwd", text("/srv")),
                named("max_memory", text("512mb")),
            ],
        ),
        vec![],
    ));
    assert_eq!(
        arg(stages_of(&run, 0)[0], "max_memory").value,
        BoundExpr::Size { v: 512_000_000 }
    );
}

#[test]
fn wrong_type_lists_the_accepted_types() {
    let error = failed(&single(
        stage("find", vec![pos(text("/srv"))]),
        vec![stage("first", vec![pos(text("ten"))])],
    ));
    assert_eq!(error.code(), ErrorCode::Type);
    assert_eq!(expected_names(&error), ["int"]);
}

#[test]
fn binding_records_the_revisions() {
    let tree = bind_with(
        &single(stage("find", vec![pos(text("/srv"))]), vec![]),
        &linux().with_corr_rev("sha256:00"),
    )
    .unwrap();
    assert_eq!(tree.catalog_rev, catalog().revision());
    assert_eq!(tree.corr_rev, "sha256:00");
    nurt_syntax::validate::validate_bound(&tree).unwrap();
}

#[test]
fn texts_convert_to_durations_in_days_and_weeks() {
    let tree = bound(&single(
        stage(
            "run",
            vec![
                pos(list(vec![text("make")])),
                named("cwd", text("/srv")),
                named("timeout", text("2w")),
                named("idle_timeout", text("24h")),
            ],
        ),
        vec![],
    ));
    let run = stages_of(&tree, 0)[0];
    assert_eq!(
        arg(run, "timeout").value,
        BoundExpr::Dur { v: 1_209_600_000 }
    );
    assert_eq!(
        arg(run, "idle_timeout").value,
        BoundExpr::Dur { v: 86_400_000 }
    );
}

#[test]
fn a_wait_of_a_week_written_as_text_marks_the_stage() {
    let tree = bound(&single(
        stage(
            "run",
            vec![
                pos(list(vec![text("make")])),
                named("cwd", text("/srv")),
                named("wait", text("1w")),
            ],
        ),
        vec![],
    ));
    let run = stages_of(&tree, 0)[0];
    assert_eq!(arg(run, "wait").value, BoundExpr::Dur { v: 604_800_000 });
    assert!(run.waits);
}

#[test]
fn malformed_texts_of_sizes_and_durations_are_type_errors() {
    let run = |name: &str, value: &str| {
        failed(&single(
            stage(
                "run",
                vec![
                    pos(list(vec![text("make")])),
                    named("cwd", text("/srv")),
                    named(name, text(value)),
                ],
            ),
            vec![],
        ))
    };
    for (name, value) in [
        ("max_memory", "1MB"),
        ("max_memory", "1é"),
        ("max_memory", "0.0005kb"),
        ("max_memory", "9007199254740992b"),
        ("timeout", "1W"),
        ("timeout", "1ч"),
        ("timeout", "10"),
    ] {
        let error = run(name, value);
        assert_eq!(error.code(), ErrorCode::Type, "{name}: {value}");
        assert_eq!(error.error().stage(), Some(ExecStage::Bind));
    }
}

#[test]
fn error_positions_count_characters_of_the_line() {
    let document = nurt_syntax::parse::parse(
        "find \"/srv/é日\" |> remvoe",
        nurt_contracts::kinds::SyntaxVariant::A,
        &nurt_syntax::parse::ParseOptions::default(),
    )
    .unwrap();
    let error = failed(&document);
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
    let at = error.error().at.unwrap();
    assert_eq!(
        (at.line, at.column, at.length),
        (Some(1), Some(19), Some(6))
    );
    assert_eq!(error.error().found.as_deref(), Some("remvoe"));
}

#[test]
fn unknown_values_of_multi_byte_text_are_reported_as_written() {
    let error = failed(&single(
        stage(
            "find",
            vec![pos(text("/srv/é")), named("type", ident("fïl"))],
        ),
        vec![],
    ));
    assert_eq!(error.code(), ErrorCode::UnknownValue);
    assert_eq!(error.error().found.as_deref(), Some("fïl"));
}

fn parsed(program: &str) -> nurt_contracts::ast::AstDocument {
    nurt_syntax::parse::parse(
        program,
        nurt_contracts::kinds::SyntaxVariant::A,
        &nurt_syntax::parse::ParseOptions::default(),
    )
    .unwrap()
}

#[test]
fn a_literal_that_is_not_a_value_of_the_field_is_rejected_at_binding() {
    let invalid = [
        ("find \"/tmp\" |> filter size > \"abc\"", "size"),
        ("find \"/tmp\" |> filter size == \"abc\"", "size"),
        ("find \"/tmp\" |> filter size != \"1 mb\"", "size"),
        ("find \"/tmp\" |> filter \"abc\" < size", "size"),
        ("find \"/tmp\" |> filter modified < \"never\"", "instant"),
        ("processes |> filter started >= \"soon\"", "instant"),
        ("jobs |> filter elapsed > \"slow\"", "duration"),
        ("jobs |> filter \"slow\" <= elapsed", "duration"),
        (
            "find \"/tmp\" |> filter size > \"1mb\" and modified < \"never\"",
            "instant",
        ),
        ("find \"/tmp\" |> filter not size > \"abc\"", "size"),
    ];
    for (program, ty) in invalid {
        let error = failed(&parsed(program));
        assert_eq!(error.code(), ErrorCode::InvalidValue, "{program}");
        assert_eq!(error.error().stage(), Some(ExecStage::Bind), "{program}");
        assert_eq!(expected_names(&error), [ty], "{program}");
        assert!(error.error().at.is_some(), "{program}");
    }
}

#[test]
fn the_rejected_literal_is_reported_at_its_place() {
    let error = failed(&parsed("find \"/tmp\" |> filter size > \"abc\""));
    assert_eq!(error.error().found.as_deref(), Some("abc"));
    let at = error.error().at.unwrap();
    assert_eq!(
        (at.line, at.column, at.length),
        (Some(1), Some(30), Some(5))
    );
}

#[test]
fn an_ordering_with_a_literal_of_another_family_is_a_type_error() {
    for program in [
        "find \"/tmp\" |> filter size > 5",
        "find \"/tmp\" |> filter name > 5",
        "find \"/tmp\" |> filter exists > 1",
        "find \"/tmp\" |> filter 5 < size",
        "jobs |> filter exit_code > \"abc\"",
        "jobs |> filter elapsed > 5",
        "processes |> filter cpu > \"abc\"",
        "processes |> filter memory < true",
    ] {
        let error = failed(&parsed(program));
        assert_eq!(error.code(), ErrorCode::Type, "{program}");
        assert_eq!(error.error().stage(), Some(ExecStage::Bind), "{program}");
        assert!(error.error().at.is_some(), "{program}");
    }
}

#[test]
fn a_literal_that_can_be_compared_with_the_field_binds() {
    for program in [
        "find \"/tmp\" |> filter size > \"1mb\"",
        "find \"/tmp\" |> filter size > 5mb",
        "find \"/tmp\" |> filter modified > \"2026-01-01\"",
        "find \"/tmp\" |> filter name == 5",
        "find \"/tmp\" |> filter name > \"a\"",
        "find \"/tmp\" |> filter exists == true",
        "find \"/tmp\" |> filter path == \"/tmp/a\"",
        "jobs |> filter elapsed > 5s",
        "jobs |> filter elapsed > \"5s\"",
        "jobs |> filter exit_code == \"abc\"",
        "jobs |> filter exit_code > 1",
        "processes |> filter cpu > 1",
        "processes |> filter cpu > 1.5",
        "processes |> filter name ~ /a/",
    ] {
        bound(&parsed(program));
    }
}
