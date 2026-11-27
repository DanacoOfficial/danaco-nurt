// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Binding of the control constructs and of the boolean shorthand: every
//! check happens before any effect and names the nearest correct form.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bind_support;

use bind_support::{bound, failed};
use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::{BoundExpr, BoundHead, BoundStatement};
use nurt_contracts::error::ErrorCode;
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use nurt_syntax::validate::validate_bound;

fn tree(text: &str) -> AstDocument {
    parse(text, Variant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{text}\n{error:?}"))
}

fn code(text: &str) -> ErrorCode {
    failed(&tree(text)).code()
}

#[test]
fn constructs_bind_and_the_bound_tree_is_valid() {
    let programs = [
        "when exists \"/srv/a\" {\n  remove \"/srv/a\"\n} otherwise when (find \"/srv\" |> count) > 1 {\n  remove \"/srv/b\"\n} otherwise {\n  find \"/srv\"\n}\n",
        "each (find \"/srv\" glob: \"*.log\") as f parallel: 2 atomic: false {\n  remove f.path\n}\n",
        "repeat until: exists \"/srv/ready\" every: 5s limit: 2m {\n  find \"/srv\"\n}\n",
        "while (find \"/srv\" |> count) > 0 limit: 20 {\n  find \"/srv\" |> first 10 |> remove\n}\n",
        "steps {\n  remove \"/srv/a\"\n  remove \"/srv/b\"\n}\n",
        "define rotate(dir: path, keep: int = 7) {\n  find @dir |> skip @keep |> remove\n}\nrotate(\"/srv/a\")\nrotate(\"/srv/b\", keep: 14)\n",
        "params {\n  dir: path\n  days: int = 30\n}\nfind @dir\n",
    ];
    for text in programs {
        let document = bound(&tree(text));
        validate_bound(&document).unwrap_or_else(|error| panic!("{text}\n{error:?}"));
    }
}

#[test]
fn a_condition_that_is_not_bool_is_a_type_error() {
    assert_eq!(code("when 5 {\n  find \"/srv\"\n}\n"), ErrorCode::Type);
    assert_eq!(
        code("when \"yes\" {\n  find \"/srv\"\n}\n"),
        ErrorCode::Type
    );
    assert_eq!(
        code("while [1] limit: 2 {\n  find \"/srv\"\n}\n"),
        ErrorCode::Type
    );
}

#[test]
fn the_options_of_a_loop_have_their_names_and_types() {
    assert_eq!(
        code("each (find \"/srv\") as f paralel: 2 {\n  find \"/srv\"\n}\n"),
        ErrorCode::UnknownParam
    );
    assert_eq!(
        code("each (find \"/srv\") as f parallel: \"two\" {\n  find \"/srv\"\n}\n"),
        ErrorCode::Type
    );
    assert_eq!(
        code("while exists \"/a\" limit: \"many\" {\n  find \"/srv\"\n}\n"),
        ErrorCode::Type
    );
    assert_eq!(
        code("repeat every: 5s {\n  find \"/srv\"\n}\n"),
        ErrorCode::MissingParam
    );
    assert_eq!(
        code("each (find \"/srv\") as f atomic: false atomic: true {\n  find \"/srv\"\n}\n"),
        ErrorCode::ParamConflict
    );
}

#[test]
fn the_unknown_option_names_the_nearest() {
    let error = failed(&tree(
        "each (find \"/srv\") as f paralel: 2 {\n  find \"/srv\"\n}\n",
    ));
    let expected = serde_json::to_string(&error.error().expected).unwrap();
    assert!(expected.contains("parallel"), "{expected}");
}

#[test]
fn break_and_continue_stand_inside_a_loop() {
    assert_eq!(
        code("steps {\n  find \"/srv\"\n  break\n}\n"),
        ErrorCode::Arg
    );
    assert_eq!(code("continue\n"), ErrorCode::Arg);
    let document = bound(&tree(
        "each (find \"/srv\") as f {\n  when f.size > 1 {\n    continue\n  }\n  break\n}\n",
    ));
    assert!(matches!(
        document.program.statements[0],
        BoundStatement::Each(_)
    ));
}

#[test]
fn the_loop_variable_is_read_without_a_sigil_and_is_a_reference() {
    let document = bound(&tree("each (find \"/srv\") as f {\n  remove f.path\n}\n"));
    let BoundStatement::Each(each) = &document.program.statements[0] else {
        panic!("not an each")
    };
    let BoundStatement::Pipeline(pipeline) = &each.body[0] else {
        panic!("not a pipeline")
    };
    let BoundHead::Stage(stage) = &pipeline.head else {
        panic!("not a stage")
    };
    let path = stage.args.iter().find(|arg| arg.name == "path").unwrap();
    assert!(
        matches!(&path.value, BoundExpr::Ref(reference) if reference.name == "f" && reference.fields == ["path"]),
        "{:?}",
        path.value
    );
}

#[test]
fn names_defined_in_a_block_are_not_visible_after_it() {
    assert_eq!(
        code("steps {\n  let inner = find \"/srv\"\n}\n@inner |> count\n"),
        ErrorCode::UnknownName
    );
    assert_eq!(
        code("each (find \"/srv\") as f {\n  find \"/srv\"\n}\nremove f.path\n"),
        ErrorCode::Type
    );
}

#[test]
fn a_function_sees_only_its_parameters() {
    assert_eq!(
        code("let outer = find \"/srv\"\ndefine f() {\n  @outer |> count\n}\nf()\n"),
        ErrorCode::UnknownName
    );
}

#[test]
fn a_call_is_checked_against_the_declaration() {
    let define = "define put(dir: path, times: int = 1, loud: bool = false) {\n  find @dir\n}\n";
    let check = |call: &str| code(&format!("{define}{call}\n"));
    assert_eq!(check("put()"), ErrorCode::MissingParam);
    assert_eq!(check("put(\"/a\", tims: 2)"), ErrorCode::UnknownParam);
    assert_eq!(check("put(\"/a\", times: \"x\")"), ErrorCode::Type);
    assert_eq!(check("put(\"/a\", 1, false, 4)"), ErrorCode::Arity);
    assert_eq!(check("put(\"/a\", 1, times: 2)"), ErrorCode::ParamConflict);
    assert_eq!(check("put(\"relative\")"), ErrorCode::PathRelative);
    let ok = bound(&tree(&format!("{define}put(\"/a\", loud)\n")));
    let BoundStatement::Pipeline(call) = &ok.program.statements[1] else {
        panic!("not a call")
    };
    let BoundHead::Call(call) = &call.head else {
        panic!("not a call head")
    };
    let loud = call.args.iter().find(|arg| arg.name == "loud").unwrap();
    assert_eq!(loud.value, BoundExpr::Bool { v: true });
    assert!(!loud.default);
    let times = call.args.iter().find(|arg| arg.name == "times").unwrap();
    assert!(times.default);
}

#[test]
fn a_function_is_defined_once_and_is_no_command() {
    assert_eq!(
        code("define f() {\n  find \"/a\"\n}\ndefine f() {\n  find \"/b\"\n}\n"),
        ErrorCode::Arg
    );
    assert_eq!(code("define find() {\n  find \"/a\"\n}\n"), ErrorCode::Arg);
}

#[test]
fn params_is_the_first_statement_with_known_types() {
    assert_eq!(
        code("find \"/srv\"\nparams {\n  dir: path\n}\n"),
        ErrorCode::Arg
    );
    assert_eq!(
        code("params {\n  dir: folder\n}\nfind \"/srv\"\n"),
        ErrorCode::Type
    );
    assert_eq!(
        code("params {\n  dir: path\n  dir: path\n}\nfind \"/srv\"\n"),
        ErrorCode::Arg
    );
    assert_eq!(
        code("params {\n  days: int = \"x\"\n}\nfind \"/srv\"\n"),
        ErrorCode::Type
    );
}

#[test]
fn the_parameters_of_a_script_are_names_of_the_program() {
    let document = bound(&tree("params {\n  dir: path\n}\nfind @dir\n"));
    assert!(matches!(
        document.program.statements[0],
        BoundStatement::Params(_)
    ));
}

#[test]
fn a_bool_parameter_without_a_value_of_a_command_is_true() {
    let short = bound(&tree("find \"/srv\" hidden\n"));
    let long = bound(&tree("find \"/srv\" hidden: true\n"));
    assert_eq!(bind_support::hash(&short), bind_support::hash(&long));
    let off = bound(&tree("find \"/srv\" hidden: false\n"));
    assert_ne!(bind_support::hash(&short), bind_support::hash(&off));
}

#[test]
fn an_unknown_function_names_the_nearest_function() {
    let error = failed(&tree(
        "define rotate_logs(dir: path) {\n  find @dir\n}\nrotate_log(\"/srv\")\n",
    ));
    assert_eq!(error.code(), ErrorCode::UnknownName);
    let expected = serde_json::to_string(&error.error().expected).unwrap();
    assert!(expected.contains("rotate_logs"), "{expected}");
}
