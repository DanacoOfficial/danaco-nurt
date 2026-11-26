// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Productions of the variant A grammar: statements, stages, arguments,
//! expressions, sub-pipelines, spans, nesting and metadata.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

mod parse_a_support;

use nurt_contracts::ast::strip_for_hash;
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use parse_a_support::{check_invariants, first_arg, json_a, parse_a, statement};
use serde_json::{Value, json};

fn int(v: i64) -> Value {
    json!({"t": "int", "v": v})
}

fn ident(v: &str) -> Value {
    json!({"t": "ident", "v": v})
}

fn bin(op: &str, l: Value, r: Value) -> Value {
    json!({"t": "bin", "op": op, "l": l, "r": r})
}

fn un(op: &str, x: Value) -> Value {
    json!({"t": "un", "op": op, "x": x})
}

/// Values of all positional and named arguments of the head stage.
fn arg_values(text: &str) -> Vec<Value> {
    statement(text)["head"]["args"]
        .as_array()
        .expect("arguments")
        .iter()
        .map(|arg| arg["value"].clone())
        .collect()
}

#[test]
fn parses_let_with_a_pipeline_and_a_reference_head() {
    let text = "let recent = find \"/a\" |> filter size > 1mb\n@recent.path |> count";
    let json = strip_for_hash(&json_a(text));
    let statements = &json["program"]["statements"];
    assert_eq!(statements[0]["t"], "let");
    assert_eq!(statements[0]["name"], "recent");
    assert_eq!(statements[0]["value"]["t"], "pipeline");
    assert_eq!(statements[0]["value"]["stages"][0]["name"], "filter");
    assert_eq!(
        statements[1]["head"],
        json!({"t": "ref", "name": "recent", "fields": ["path"]})
    );
    assert_eq!(statements[1]["stages"][0]["name"], "count");
}

#[test]
fn accepts_several_lets_and_blank_lines_between_statements() {
    let text = "\n\nlet a = count\n\n# note\nlet b = count\n@a\n\n";
    let json = json_a(text);
    assert_eq!(
        json["program"]["statements"].as_array().map(Vec::len),
        Some(3)
    );
}

#[test]
fn joins_command_name_members_with_dots() {
    assert_eq!(
        statement("git.status \"/srv/app\"")["head"]["name"],
        "git.status"
    );
    assert_eq!(
        statement("archive.extract \"/a\"")["head"]["name"],
        "archive.extract"
    );
}

#[test]
fn continues_a_pipeline_on_the_next_line_after_or_before_the_operator() {
    let text = "find \"/a\"\n|> first 1\n  |> count";
    assert_eq!(statement(text)["stages"].as_array().map(Vec::len), Some(2));
}

#[test]
fn reads_named_arguments_including_keyword_names() {
    let head = statement("replace \"a\" \"b\" in: [\"/srv/a.txt\"] not: true")["head"].clone();
    assert_eq!(head["args"][2]["name"], "in");
    assert_eq!(head["args"][2]["value"]["t"], "list");
    assert_eq!(head["args"][3]["name"], "not");
    assert_eq!(head["args"][3]["value"], json!({"t": "bool", "v": true}));
}

#[test]
fn keeps_positional_arguments_after_named_ones_in_written_order() {
    let head = statement("find glob: \"*.go\" \"/srv/app\"")["head"].clone();
    assert_eq!(head["args"][0]["name"], "glob");
    assert!(head["args"][1].get("name").is_none());
}

#[test]
fn not_binds_looser_than_comparisons() {
    assert_eq!(
        first_arg("filter not size > 1mb"),
        un(
            "not",
            bin(">", ident("size"), json!({"t": "size", "v": 1_000_000}))
        )
    );
    assert_eq!(
        first_arg("filter not a and b"),
        bin("and", un("not", ident("a")), ident("b"))
    );
}

#[test]
fn and_binds_tighter_than_or() {
    assert_eq!(
        first_arg("filter a or b and c"),
        bin("or", ident("a"), bin("and", ident("b"), ident("c")))
    );
}

#[test]
fn multiplication_binds_tighter_than_addition_and_both_associate_left() {
    assert_eq!(
        first_arg("filter a + b * c"),
        bin("+", ident("a"), bin("*", ident("b"), ident("c")))
    );
    assert_eq!(
        first_arg("filter a - b - c"),
        bin("-", bin("-", ident("a"), ident("b")), ident("c"))
    );
    assert_eq!(
        first_arg("filter a / b / c"),
        bin("/", bin("/", ident("a"), ident("b")), ident("c"))
    );
}

#[test]
fn unary_minus_binds_looser_than_field_access_and_never_folds_into_a_literal() {
    assert_eq!(
        first_arg("filter -a.b"),
        un("neg", json!({"t": "field", "name": "b", "of": ident("a")}))
    );
    assert_eq!(first_arg("first -1"), un("neg", int(1)));
    assert_eq!(first_arg("first -(-1)"), un("neg", un("neg", int(1))));
}

#[test]
fn comparisons_take_additive_operands_and_in_takes_lists_and_ranges() {
    assert_eq!(
        first_arg("filter a + 1 == b"),
        bin("==", bin("+", ident("a"), int(1)), ident("b"))
    );
    assert_eq!(
        first_arg("filter x in [1, 2]"),
        bin(
            "in",
            ident("x"),
            json!({"t": "list", "items": [int(1), int(2)]})
        )
    );
    assert_eq!(
        first_arg("filter x in 1..5"),
        bin("in", ident("x"), json!({"t": "range", "lo": 1, "hi": 5}))
    );
    assert_eq!(
        first_arg("filter name ~ /a/i and name !~ /b/"),
        bin(
            "and",
            bin(
                "~",
                ident("name"),
                json!({"t": "regex", "v": "a", "flags": "i"})
            ),
            bin(
                "!~",
                ident("name"),
                json!({"t": "regex", "v": "b", "flags": ""})
            )
        )
    );
    assert_eq!(
        first_arg("filter (a < b) == (not c)"),
        bin(
            "==",
            bin("<", ident("a"), ident("b")),
            un("not", ident("c"))
        )
    );
}

#[test]
fn minus_with_whitespace_only_before_it_starts_a_new_argument() {
    assert_eq!(arg_values("first 20 -1"), [int(20), un("neg", int(1))]);
    assert_eq!(
        arg_values("filter size - 100"),
        [bin("-", ident("size"), int(100))]
    );
    assert_eq!(
        arg_values("filter size-100"),
        [bin("-", ident("size"), int(100))]
    );
    assert_eq!(arg_values("filter size -100").len(), 2);
}

#[test]
fn slash_after_whitespace_and_before_a_character_starts_a_regular_expression() {
    assert_eq!(
        arg_values("search x /re/"),
        [ident("x"), json!({"t": "regex", "v": "re", "flags": ""})]
    );
    assert_eq!(
        arg_values("filter a / b"),
        [bin("/", ident("a"), ident("b"))]
    );
    assert_eq!(arg_values("filter a/b"), [bin("/", ident("a"), ident("b"))]);
}

#[test]
fn regular_expression_argument_may_start_with_a_hash() {
    assert_eq!(
        arg_values("search \"/src\" /#include/i"),
        [
            json!({"t": "text", "v": "/src"}),
            json!({"t": "regex", "v": "#include", "flags": "i"})
        ]
    );
}

#[test]
fn crlf_line_endings_end_statements_and_keep_scalar_value_columns() {
    let json = json_a("find \"zażółć\"\r\n|> first 20\r\nlet x = count\r\n");
    let statements = &json["program"]["statements"];
    assert_eq!(statements.as_array().map(Vec::len), Some(2));
    assert_eq!(statements[0]["head"]["span"], json!([1, 1, 1, 14]));
    assert_eq!(statements[0]["stages"][0]["span"], json!([2, 4, 2, 12]));
    assert_eq!(statements[1]["span"], json!([3, 1, 3, 14]));
}

#[test]
fn crlf_inside_a_text_is_stored_as_a_line_feed() {
    assert_eq!(
        first_arg("info \"a\r\nb\" "),
        json!({"t": "text", "v": "a\nb"})
    );
    assert_eq!(
        first_arg("info r\"a\r\nb\""),
        json!({"t": "text", "v": "a\nb"})
    );
}

#[test]
fn comments_with_crlf_keep_their_text_and_span_without_the_carriage_return() {
    let options = ParseOptions {
        keep_comments: true,
        ..ParseOptions::default()
    };
    let document = parse("# zażółć\r\ncount # end\r\n", Variant::A, &options).expect("parses");
    let meta = Value::Object(document.meta.expect("meta"));
    assert_eq!(meta["comments"][0]["text"], " zażółć");
    assert_eq!(meta["comments"][0]["span"], json!([1, 1, 1, 9]));
    assert_eq!(meta["comments"][1]["text"], " end");
    assert_eq!(meta["comments"][1]["span"], json!([2, 7, 2, 12]));
}

#[test]
fn call_requires_the_parenthesis_directly_after_the_name() {
    assert_eq!(
        arg_values("filter ago (24h)"),
        [ident("ago"), json!({"t": "dur", "v": 86_400_000})]
    );
    assert_eq!(
        arg_values("filter ago(24h)"),
        [json!({"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 86_400_000}]})]
    );
    assert_eq!(
        first_arg("filter max(a, b,\n c)"),
        json!({"t": "call", "fn": "max", "args": [ident("a"), ident("b"), ident("c")]})
    );
    assert_eq!(
        first_arg("filter now()"),
        json!({"t": "call", "fn": "now", "args": []})
    );
}

#[test]
fn field_access_applies_to_every_operand() {
    assert_eq!(
        first_arg("filter \"abc\".len"),
        json!({"t": "field", "name": "len", "of": {"t": "text", "v": "abc"}})
    );
    assert_eq!(
        first_arg("filter ago(1d).x"),
        json!({"t": "field", "name": "x", "of": {"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 86_400_000}]}})
    );
    assert_eq!(
        first_arg("filter (@x).path"),
        json!({"t": "field", "name": "path", "of": {"t": "ref", "name": "x"}})
    );
    assert_eq!(
        first_arg("filter a.b.c"),
        json!({"t": "field", "name": "c", "of": {"t": "field", "name": "b", "of": ident("a")}})
    );
}

#[test]
fn reference_fields_stay_in_the_reference() {
    assert_eq!(
        first_arg("info @logs.path.name"),
        json!({"t": "ref", "name": "logs", "fields": ["path", "name"]})
    );
    assert_eq!(first_arg("info @logs"), json!({"t": "ref", "name": "logs"}));
}

#[test]
fn parenthesized_command_without_arguments_is_an_expression() {
    assert_eq!(first_arg("verify (count)"), ident("count"));
    assert_eq!(
        first_arg("verify (git.status)"),
        json!({"t": "field", "name": "status", "of": ident("git")})
    );
    assert_eq!(first_arg("verify (not x)"), un("not", ident("x")));
    assert_eq!(
        first_arg("verify (f(x))"),
        json!({"t": "call", "fn": "f", "args": [ident("x")]})
    );
    assert_eq!(first_arg("verify (size > 1mb)")["t"], "bin");
}

#[test]
fn parenthesized_command_with_arguments_is_a_sub_pipeline() {
    let sub = first_arg("verify (f (x))");
    assert_eq!(sub["t"], "sub");
    assert_eq!(sub["p"]["head"]["name"], "f");
    assert_eq!(sub["p"]["head"]["args"][0]["value"], ident("x"));
    let sub = first_arg("verify (x -1)");
    assert_eq!(sub["p"]["head"]["args"][0]["value"], un("neg", int(1)));
    let sub = first_arg("verify (a.b c)");
    assert_eq!(sub["p"]["head"]["name"], "a.b");
    let sub = first_arg("verify (filter not hidden)");
    assert_eq!(
        sub["p"]["head"]["args"][0]["value"],
        un("not", ident("hidden"))
    );
    let sub = first_arg("verify (first ..5)");
    assert_eq!(
        sub["p"]["head"]["args"][0]["value"],
        json!({"t": "range", "hi": 5})
    );
}

#[test]
fn parenthesized_pipeline_operator_makes_a_sub_pipeline() {
    let sub = first_arg("verify (count |> first 1)");
    assert_eq!(
        sub["p"]["head"],
        json!({"t": "stage", "name": "count", "args": []})
    );
    assert_eq!(sub["p"]["stages"][0]["name"], "first");
    let sub = first_arg("verify (git.status |> count)");
    assert_eq!(sub["p"]["head"]["name"], "git.status");
    let sub = first_arg("verify (@x |> count)");
    assert_eq!(sub["p"]["head"], json!({"t": "ref", "name": "x"}));
}

#[test]
fn sub_pipeline_may_span_several_lines() {
    let text = "find \"/a\"\n|> verify (\n  run [\"go\", \"build\"]\n    cwd: \"/a\"\n  |> assert exit_code: 0\n)";
    let json = json_a(text);
    let sub = &json["program"]["statements"][0]["stages"][0]["args"][0]["value"];
    assert_eq!(sub["t"], "sub");
    assert_eq!(sub["p"]["head"]["args"][1]["name"], "cwd");
    assert_eq!(sub["span"], json!([2, 11, 6, 2]));
}

#[test]
fn lists_accept_spreads_trailing_commas_and_line_breaks() {
    assert_eq!(
        first_arg("info [\n  ...@logs.path,\n  \"/a\",\n]"),
        json!({"t": "list", "items": [
            {"t": "spread", "x": {"t": "ref", "name": "logs", "fields": ["path"]}},
            {"t": "text", "v": "/a"}
        ]})
    );
    assert_eq!(first_arg("info []"), json!({"t": "list", "items": []}));
}

#[test]
fn maps_accept_identifier_keyword_and_text_keys() {
    assert_eq!(
        first_arg("set {in: 1, \"a.b\": 2, r\"c\": 3, true: null,}"),
        json!({"t": "map", "entries": [
            {"k": "in", "v": int(1)},
            {"k": "a.b", "v": int(2)},
            {"k": "c", "v": int(3)},
            {"k": "true", "v": {"t": "null"}}
        ]})
    );
    assert_eq!(first_arg("set {}"), json!({"t": "map", "entries": []}));
}

#[test]
fn ranges_accept_open_bounds_and_hexadecimal_integers() {
    assert_eq!(
        first_arg("lines 10..12"),
        json!({"t": "range", "lo": 10, "hi": 12})
    );
    assert_eq!(first_arg("lines 40.."), json!({"t": "range", "lo": 40}));
    assert_eq!(first_arg("lines ..9"), json!({"t": "range", "hi": 9}));
    assert_eq!(first_arg("lines .."), json!({"t": "range"}));
    assert_eq!(
        first_arg("lines 0x10..0x20"),
        json!({"t": "range", "lo": 16, "hi": 32})
    );
    assert_eq!(
        first_arg("lines 12..10"),
        json!({"t": "range", "lo": 12, "hi": 10})
    );
    assert_eq!(
        arg_values("lines 2.. 3"),
        [json!({"t": "range", "lo": 2}), int(3)]
    );
}

#[test]
fn literals_keep_their_written_kind() {
    assert_eq!(
        arg_values("x \"a\" r#\"b\"# 1 2.5 1kib 1h30m true false null"),
        [
            json!({"t": "text", "v": "a"}),
            json!({"t": "text", "v": "b"}),
            int(1),
            json!({"t": "num", "v": 2.5}),
            json!({"t": "size", "v": 1024}),
            json!({"t": "dur", "v": 5_400_000}),
            json!({"t": "bool", "v": true}),
            json!({"t": "bool", "v": false}),
            json!({"t": "null"}),
        ]
    );
}

#[test]
fn spans_follow_the_specification_example() {
    let json = json_a("ports state: listen |> filter port == 8080");
    let expected: Value = serde_json::from_str(
        r#"{"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
          "head": {"t": "stage", "name": "ports", "args": [
            {"name": "state", "value": {"t": "ident", "v": "listen", "span": [1, 14, 1, 20]}, "span": [1, 7, 1, 20]}],
            "span": [1, 1, 1, 20]},
          "stages": [{"t": "stage", "name": "filter", "args": [
            {"value": {"t": "bin", "op": "==",
              "l": {"t": "ident", "v": "port", "span": [1, 31, 1, 35]},
              "r": {"t": "int", "v": 8080, "span": [1, 39, 1, 43]}, "span": [1, 31, 1, 43]},
             "span": [1, 31, 1, 43]}], "span": [1, 24, 1, 43]}],
          "span": [1, 1, 1, 43]}]}}"#,
    )
    .expect("expected tree");
    assert_eq!(json, expected);
}

#[test]
fn spans_cover_operators_brackets_ranges_and_unicode_columns() {
    let json = json_a("x -a [1] ..9 2.. \"zé\" (b) @r.f");
    let args = &json["program"]["statements"][0]["head"]["args"];
    assert_eq!(args[0]["value"]["span"], json!([1, 3, 1, 5]));
    assert_eq!(args[1]["value"]["span"], json!([1, 6, 1, 9]));
    assert_eq!(args[2]["value"]["span"], json!([1, 10, 1, 13]));
    assert_eq!(args[3]["value"]["span"], json!([1, 14, 1, 17]));
    assert_eq!(args[4]["value"]["span"], json!([1, 18, 1, 22]));
    assert_eq!(args[5]["value"]["span"], json!([1, 23, 1, 26]));
    assert_eq!(args[6]["value"]["span"], json!([1, 27, 1, 31]));
    assert_eq!(
        json["program"]["statements"][0]["span"],
        json!([1, 1, 1, 31])
    );
    check_invariants(&json).expect("invariants");
}

#[test]
fn let_span_covers_the_keyword_and_the_pipeline() {
    let json = json_a("let a = count |> first 1");
    assert_eq!(
        json["program"]["statements"][0]["span"],
        json!([1, 1, 1, 25])
    );
    assert_eq!(
        json["program"]["statements"][0]["value"]["span"],
        json!([1, 9, 1, 25])
    );
}

/// `first` with `n` nested lists around an integer: the tree depth is
/// `n + 3` (pipeline, stage, lists, integer).
fn nested_lists(n: usize) -> String {
    format!("first {}1{}", "[".repeat(n), "]".repeat(n))
}

#[test]
fn accepts_tree_depth_128_and_rejects_129() {
    assert!(parse_a(&nested_lists(125)).is_ok());
    let error = parse_a(&nested_lists(126)).expect_err("too deep");
    assert_eq!(error.problem.as_deref(), Some("nesting_too_deep"));
    assert_eq!(
        error.message,
        "nesting too deep; split the expression or the program"
    );
}

#[test]
fn rejects_deep_nesting_without_exhausting_the_stack() {
    for text in [
        format!("x {}1{}", "(".repeat(10_000), ")".repeat(10_000)),
        format!("x {}", "[".repeat(100_000)),
        format!("x {}y", "not ".repeat(10_000)),
        format!("x {}y", "- ".repeat(10_000)),
        format!("x {}", "...".repeat(10_000)),
        format!("x 1{}", " + 1".repeat(1_000)),
    ] {
        assert!(parse_a(&text).is_err(), "{}", &text[..20]);
    }
}

#[test]
fn honours_a_smaller_maximum_depth() {
    let options = ParseOptions {
        max_depth: 4,
        ..ParseOptions::default()
    };
    assert!(parse("first [1]", Variant::A, &options).is_ok());
    assert!(parse("first [[1]]", Variant::A, &options).is_err());
}

#[test]
fn stores_variant_comments_and_source_in_meta_on_request() {
    let text = "# list files\nfind \"/a\" # here\n";
    let plain = parse_a(text).expect("parses");
    assert!(plain.meta.is_none());
    let options = ParseOptions {
        keep_comments: true,
        keep_source: true,
        ..ParseOptions::default()
    };
    let document = parse(text, Variant::A, &options).expect("parses");
    let meta = Value::Object(document.meta.clone().expect("meta"));
    assert_eq!(meta["variant"], "a");
    assert_eq!(meta["source"], text);
    assert_eq!(meta["comments"][0]["text"], " list files");
    assert_eq!(meta["comments"][0]["span"], json!([1, 1, 1, 13]));
    assert_eq!(meta["comments"][1]["text"], " here");
    let only_source = ParseOptions {
        keep_source: true,
        ..ParseOptions::default()
    };
    let meta = parse(text, Variant::A, &only_source)
        .expect("parses")
        .meta
        .expect("meta");
    assert!(meta.get("comments").is_none());
    assert_eq!(meta["variant"], "a");
    assert_eq!(document.program, plain.program);
}

#[test]
fn canonical_forms_read_back() {
    for text in [
        "filter not (not x)",
        "filter -(-x)",
        "filter (a < b) < c",
        "filter a - (b - c)",
        "info [...@x.path]",
        "lines ..",
        "x {k: v, \"a b\": 1}",
        "edit \"/a\" changes: [\n  {delete_lines: 10..12},\n  {line: 3}\n]",
        "verify (run [\"go\"] cwd: \"/a\" |> assert exit_code: 0)",
    ] {
        let json = json_a(text);
        check_invariants(&json).unwrap_or_else(|problem| panic!("{text}: {problem}"));
    }
}
