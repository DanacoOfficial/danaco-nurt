// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_flow.rs
// Responsibility:   Parsing of the control constructs: conditions, loops, `steps`, functions.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Parsing of the control constructs: conditions, loops, `steps`, functions
//! and script parameters, in the three syntax variants.

// Test code asserts with unwrap, expect and panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::{AstDocument, Statement};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

fn parse_a(text: &str) -> AstDocument {
    parse(text, Variant::A, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{text}\n{error:?}"))
}

fn kinds(document: &AstDocument) -> Vec<&'static str> {
    document
        .program
        .statements
        .iter()
        .map(|statement| match statement {
            Statement::Let(_) => "let",
            Statement::Pipeline(_) => "pipeline",
            Statement::When(_) => "when",
            Statement::Each(_) => "each",
            Statement::Repeat(_) => "repeat",
            Statement::While(_) => "while",
            Statement::Break(_) => "break",
            Statement::Continue(_) => "continue",
            Statement::Steps(_) => "steps",
            Statement::Define(_) => "define",
            Statement::Params(_) => "params",
            _ => "other",
        })
        .collect()
}

#[test]
fn parses_the_condition_of_the_design() {
    let document = parse_a(
        "when (read \"/etc/app.toml\" key: \"mode\") == \"prod\" {\n    replace /debug = true/ \"debug = false\" in: \"/etc/app.toml\" expect: 1\n} otherwise when exists \"/srv/app/.staging\" {\n    write \"/srv/app/.ready\" \"staging\\n\"\n} otherwise {\n    write \"/srv/app/.dev\" \"1\\n\"\n}\n",
    );
    assert_eq!(kinds(&document), ["when"]);
    let Statement::When(construct) = &document.program.statements[0] else {
        panic!("not a when")
    };
    assert_eq!(construct.branches.len(), 2);
    assert!(construct.otherwise.is_some());
}

#[test]
fn parses_otherwise_on_the_next_line() {
    let document =
        parse_a("when true {\n  write \"/a\" \"x\"\n}\notherwise {\n  write \"/b\" \"y\"\n}\n");
    assert_eq!(kinds(&document), ["when"]);
}

#[test]
fn parses_a_one_line_block() {
    let document = parse_a("when exists \"/a\" { remove \"/a\" }");
    assert_eq!(kinds(&document), ["when"]);
}

#[test]
fn parses_the_loops_of_the_design() {
    let document = parse_a(
        "each (find \"/srv/app\" glob: \"**/*.log\") as f {\n    lines f.path |> filter text ~ /ERROR/ |> write \"/srv/reports/\" + f.name + \".err\"\n}\n\nrepeat until: ((http.request \"https://svc/health\") |> first).status == 200 every: 5s limit: 2m {\n    run [\"systemctl\", \"restart\", \"svc\"] cwd: \"/\" privilege: elevated\n}\n\nwhile (disks |> filter mount == \"/srv\" |> first).free < 10gib limit: 20 {\n    find \"/srv/cache\" glob: \"**/*\" |> sort modified |> first 1000 |> remove\n}\n",
    );
    assert_eq!(kinds(&document), ["each", "repeat", "while"]);
}

#[test]
fn parses_break_and_continue_inside_a_condition() {
    let document = parse_a(
        "each (find \"/a\") as f {\n  when f.size > 1mb {\n    continue\n  }\n  when f.size == 0 { break }\n  remove f.path\n}\n",
    );
    assert_eq!(kinds(&document), ["each"]);
}

#[test]
fn parses_steps_and_nesting() {
    let document = parse_a(
        "steps {\n    find \"/srv/app/old\" glob: \"*.conf\" |> move to: \"/srv/app/conf\"\n    replace /old\\/path/ \"conf/path\" in: \"/srv/app/app.toml\" expect: 3\n    let r = run [\"systemctl\", \"reload\", \"app\"] cwd: \"/\" privilege: elevated\n    assert r.exit_code == 0\n}\n",
    );
    assert_eq!(kinds(&document), ["steps"]);
}

#[test]
fn parses_functions_and_calls() {
    let document = parse_a(
        "define rotate_logs(dir: path, keep: int = 7) {\n    find @dir glob: \"*.log.*\" |> sort modified desc |> skip @keep |> remove\n}\nrotate_logs(\"/var/log/app1\")\nrotate_logs(\"/var/log/app2\", keep: 14)\n",
    );
    assert_eq!(kinds(&document), ["define", "pipeline", "pipeline"]);
}

#[test]
fn parses_script_parameters() {
    let document = parse_a(
        "params {\n    dir: path\n    days: int = 30\n    dry: bool = false\n    names: list<text> = []\n}\nfind @dir glob: \"**/*.log\" |> filter modified < ago(@days * 1d) |> remove preview: @dry\n",
    );
    assert_eq!(kinds(&document), ["params", "pipeline"]);
    let Statement::Params(params) = &document.program.statements[0] else {
        panic!("not params")
    };
    assert_eq!(params.params.len(), 4);
    assert_eq!(params.params[3].ty, "list<text>");
}

#[test]
fn rejects_an_empty_block() {
    let error = parse("steps {\n}\n", Variant::A, &ParseOptions::default()).unwrap_err();
    assert_eq!(error.code, nurt_contracts::error::ErrorCode::Syntax);
}

#[test]
fn rejects_a_missing_brace() {
    assert!(
        parse(
            "when true\n  remove \"/a\"\n",
            Variant::A,
            &ParseOptions::default()
        )
        .is_err()
    );
}

#[test]
fn constructs_parse_in_every_variant() {
    for variant in [Variant::A, Variant::B, Variant::C] {
        let text = match variant {
            Variant::A => "steps {\n  remove \"/a\"\n}\n",
            Variant::B => "steps {\n  remove(\"/a\")\n}\n",
            _ => "steps {\n  remove path=\"/a\"\n}\n",
        };
        parse(text, variant, &ParseOptions::default())
            .unwrap_or_else(|error| panic!("{variant:?}: {error:?}"));
    }
}

const PROGRAMS: [&str; 6] = [
    "when (read \"/etc/app.toml\" key: \"mode\") == \"prod\" {\n    replace /debug = true/ \"debug = false\" in: \"/etc/app.toml\" expect: 1\n} otherwise when exists \"/srv/app/.staging\" {\n    write \"/srv/app/.ready\" \"staging\\n\"\n} otherwise {\n    write \"/srv/app/.dev\" \"1\\n\"\n}\n",
    "each (find \"/srv/app\" glob: \"**/*.log\") as f parallel: 4 atomic: false {\n    when f.size > 1mb {\n        continue\n    }\n    remove f.path\n}\n",
    "repeat until: ((http.request \"https://svc/health\") |> first).status == 200 every: 5s limit: 2m {\n    run [\"systemctl\", \"restart\", \"svc\"] cwd: \"/\"\n}\n",
    "while (disks |> filter mount == \"/srv\" |> first).free < 10gib limit: 20 {\n    find \"/srv/cache\" glob: \"**/*\" |> first 1000 |> remove\n}\n",
    "steps {\n    find \"/srv/app/old\" glob: \"*.conf\" |> move to: \"/srv/app/conf\"\n    let r = run [\"systemctl\", \"reload\", \"app\"] cwd: \"/\"\n    assert r.exit_code == 0\n}\n",
    "define rotate_logs(dir: path, keep: int = 7) {\n    find @dir glob: \"*.log.*\" |> skip @keep |> remove\n}\nrotate_logs(\"/var/log/app1\")\nrotate_logs(\"/var/log/app2\", keep: 14)\n",
];

#[test]
fn printing_and_parsing_again_keeps_the_tree() {
    use nurt_syntax::hash::ast_hash;
    use nurt_syntax::print::try_print;
    for text in PROGRAMS {
        let document = parse_a(text);
        for variant in [Variant::A, Variant::B] {
            let printed = try_print(&document, variant)
                .unwrap_or_else(|error| panic!("{variant:?}: {error}\n{text}"));
            let again = parse(&printed, variant, &ParseOptions::default())
                .unwrap_or_else(|error| panic!("{variant:?}\n{printed}\n{error:?}"));
            assert_eq!(
                ast_hash(&document).unwrap(),
                ast_hash(&again).unwrap(),
                "{variant:?}\n{printed}"
            );
        }
    }
}

#[test]
fn the_formatter_is_idempotent_on_constructs() {
    for text in PROGRAMS {
        let once = nurt_syntax::fmt::format(text, Variant::A).unwrap();
        let twice = nurt_syntax::fmt::format(&once, Variant::A).unwrap();
        assert_eq!(once, twice);
    }
}

#[test]
fn json_and_cbor_round_trip_constructs() {
    use nurt_syntax::{cbor, json, validate::validate_surface};
    for text in PROGRAMS {
        let document = parse_a(text);
        validate_surface(&document).unwrap();
        let encoded = json::encode(&document).unwrap();
        assert_eq!(json::decode_surface(&encoded).unwrap(), document);
        let bytes = cbor::encode(&document).unwrap();
        assert_eq!(cbor::decode_surface(&bytes).unwrap(), document);
    }
}

#[test]
fn the_decoder_rejects_an_unknown_member_of_a_construct() {
    let text = r#"{"nurt":"1.1","program":{"statements":[{"t":"steps","body":[{"t":"pipeline","head":{"t":"stage","name":"count","args":[]},"stages":[]}],"extra":1}]}}"#;
    assert!(nurt_syntax::json::decode_surface(text).is_err());
}

#[test]
fn the_validator_rejects_an_empty_block_and_a_repeated_parameter() {
    use nurt_contracts::ast::{AstDocument, Param, Params, Program, Statement, Steps};
    use nurt_syntax::validate::validate_surface;
    let steps = AstDocument::new(Program {
        statements: vec![Statement::Steps(Steps {
            body: Vec::new(),
            span: None,
        })],
    });
    assert!(validate_surface(&steps).is_err());
    let param = |name: &str| Param {
        name: name.to_owned(),
        ty: "int".to_owned(),
        default: None,
        span: None,
    };
    let twice = AstDocument::new(Program {
        statements: vec![Statement::Params(Params {
            params: vec![param("a"), param("a")],
            span: None,
        })],
    });
    assert!(validate_surface(&twice).is_err());
}

#[test]
fn a_block_needs_its_braces_and_a_statement() {
    for text in [
        "when true {\n",
        "when true { }",
        "each (find \"/a\") f { remove f.path }",
        "each (find \"/a\") as { remove \"/a\" }",
        "define f { find \"/a\" }",
        "define f(a path) { find \"/a\" }",
        "params {\n  dir path\n}\nfind \"/a\"",
        "steps remove \"/a\"",
    ] {
        assert!(
            parse(text, Variant::A, &ParseOptions::default()).is_err(),
            "{text}"
        );
    }
}

#[test]
fn a_construct_word_stays_a_name_outside_the_start_of_a_statement() {
    let document = parse_a("find \"/a\" |> first 3\nwrite \"/b\" \"x\" mode: overwrite\n");
    assert_eq!(kinds(&document), ["pipeline", "pipeline"]);
    let document = parse_a("history steps: 3\n");
    assert_eq!(kinds(&document), ["pipeline"]);
}

#[test]
fn nested_constructs_parse_to_the_same_tree_in_every_variant_that_prints() {
    let text = "steps {\n    when exists \"/a\" {\n        each (find \"/a\") as f {\n            when f.size > 1 {\n                continue\n            }\n            remove f.path\n        }\n    } otherwise {\n        write \"/b\" \"x\" mode: overwrite\n    }\n}\n";
    let document = parse_a(text);
    let printed = nurt_syntax::print::try_print(&document, Variant::B).unwrap();
    let again = parse(&printed, Variant::B, &ParseOptions::default()).unwrap();
    assert_eq!(
        nurt_syntax::hash::ast_hash(&document).unwrap(),
        nurt_syntax::hash::ast_hash(&again).unwrap()
    );
}

#[test]
fn a_call_of_a_function_reads_in_every_variant() {
    for (variant, text) in [
        (Variant::A, "define f(a: int) {\n  find \"/a\"\n}\nf(1)\n"),
        (Variant::B, "define f(a: int) {\n  find(\"/a\")\n}\nf(1)\n"),
    ] {
        let document = parse(text, variant, &ParseOptions::default()).unwrap();
        assert_eq!(kinds(&document), ["define", "pipeline"], "{variant:?}");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
