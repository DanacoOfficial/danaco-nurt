// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Grammar of the call notation (variant B): stages, argument lists, the
//! meaning of `name(…)` by position and sub-pipelines as arguments.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::strip_for_hash;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use serde_json::{Value, json};

/// Parses variant B text and returns its statements without spans.
fn statements(text: &str) -> Value {
    let document = parse(text, Variant::B, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{text:?}: {error:?}"));
    let tree = strip_for_hash(&serde_json::to_value(&document).unwrap());
    tree["program"]["statements"].clone()
}

/// Returns the head of the first statement.
fn head(text: &str) -> Value {
    statements(text)[0]["head"].clone()
}

/// Returns the value of the only argument of the first stage.
fn only_value(text: &str) -> Value {
    let head = head(text);
    let args = head["args"].as_array().unwrap();
    assert_eq!(args.len(), 1, "{text:?}: {args:?}");
    args[0]["value"].clone()
}

fn error_code(text: &str) -> ErrorCode {
    let error = parse(text, Variant::B, &ParseOptions::default())
        .err()
        .unwrap_or_else(|| panic!("{text:?} parses"));
    Error::from(error).code()
}

fn assert_same_tree(text: &str, equivalent: &str) {
    assert_eq!(statements(text), statements(equivalent), "{text:?}");
}

fn stage(name: &str, args: Value) -> Value {
    json!({"t": "stage", "name": name, "args": args})
}

fn single_stage_sub(stage: Value) -> Value {
    json!({"t": "sub", "p": {"t": "pipeline", "head": stage, "stages": []}})
}

#[test]
fn stage_without_arguments_has_empty_parentheses() {
    let tree = statements("find(\"/a\") |> remove()");
    assert_eq!(tree[0]["stages"][0], stage("remove", json!([])));
}

#[test]
fn stage_without_parentheses_is_rejected() {
    assert_eq!(error_code("find(\"/a\") |> remove"), ErrorCode::Syntax);
    assert_eq!(error_code("remove"), ErrorCode::Syntax);
}

#[test]
fn reference_head_is_followed_by_stage() {
    let tree = statements("@x |> count()");
    assert_eq!(tree[0]["head"], json!({"t": "ref", "name": "x"}));
    assert_eq!(tree[0]["stages"][0], stage("count", json!([])));
}

#[test]
fn reference_after_pipe_is_rejected() {
    assert_eq!(error_code("find(\"/a\") |> @x"), ErrorCode::Syntax);
}

#[test]
fn command_name_may_contain_groups() {
    let tree = statements("code.symbols(\"a\") |> code.rename(\"b\")");
    assert_eq!(tree[0]["head"]["name"], "code.symbols");
    assert_eq!(tree[0]["stages"][0]["name"], "code.rename");
}

#[test]
fn keyword_is_rejected_as_command_name() {
    assert_eq!(error_code("true()"), ErrorCode::Syntax);
    assert_eq!(error_code("find(\"/a\") |> not()"), ErrorCode::Syntax);
    assert_eq!(error_code("git.in()"), ErrorCode::Syntax);
}

#[test]
fn space_between_command_and_parenthesis_is_accepted() {
    assert_same_tree("find (\"/a\") |> count ()", "find(\"/a\") |> count()");
}

#[test]
fn named_argument_may_be_a_keyword() {
    let tree = statements("replace(\"a\", \"b\", in: [\"/srv/a.txt\"], not: 1, and: true)");
    let names: Vec<Value> = tree[0]["head"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.get("name").cloned().unwrap_or(Value::Null))
        .collect();
    assert_eq!(
        names,
        vec![
            Value::Null,
            Value::Null,
            json!("in"),
            json!("not"),
            json!("and")
        ]
    );
}

#[test]
fn trailing_comma_is_accepted() {
    assert_same_tree(
        "find(\"/a\", glob: \"*.go\",)",
        "find(\"/a\", glob: \"*.go\")",
    );
    assert_same_tree("filter(size > 1,)", "filter(size > 1)");
}

#[test]
fn argument_list_may_span_several_lines() {
    assert_same_tree(
        "find(\n  \"/a\",\n  glob: \"*.go\",\n)\n|> count()",
        "find(\"/a\", glob: \"*.go\") |> count()",
    );
    assert_same_tree("find(\n\"/a\"\n)", "find(\"/a\")");
    assert_same_tree("count(\n)", "count()");
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    assert_same_tree(
        "\n# list\nfind(\"/a\", # root\n  glob: \"*\")\n\n|> count() # total\n",
        "find(\"/a\", glob: \"*\") |> count()",
    );
}

#[test]
fn argument_boundary_is_comma_or_parenthesis() {
    let minus = only_value("first(20 -1)");
    assert_eq!(minus["t"], "bin");
    assert_eq!(minus["op"], "-");
    assert_same_tree("first(20 -1)", "first(20 - 1)");
    assert_same_tree("first(20-1)", "first(20 - 1)");
    let plus = only_value("first(20 +1)");
    assert_eq!(plus["op"], "+");
}

#[test]
fn slash_after_operand_is_division() {
    let division = only_value("filter(size /2)");
    assert_eq!(division["t"], "bin");
    assert_eq!(division["op"], "/");
    assert_same_tree("filter(size /2 > 1)", "filter(size / 2 > 1)");
}

#[test]
fn slash_in_value_position_starts_regular_expression() {
    let tree = statements("search(/oldName\\(/i, \"/srv\")");
    assert_eq!(
        tree[0]["head"]["args"][0]["value"],
        json!({"t": "regex", "v": "oldName\\(", "flags": "i"})
    );
    let after_name = statements("read(\"/a\", around: /func main/)");
    assert_eq!(after_name[0]["head"]["args"][1]["value"]["t"], "regex");
}

#[test]
fn negative_number_is_negation() {
    assert_eq!(
        only_value("filter(exit_code != -1)")["r"],
        json!({"t": "un", "op": "neg", "x": {"t": "int", "v": 1}})
    );
    assert_eq!(
        only_value("skip(-1)"),
        json!({"t": "un", "op": "neg", "x": {"t": "int", "v": 1}})
    );
}

#[test]
fn not_binds_looser_than_comparison() {
    assert_eq!(
        only_value("filter(not size > 1mb)"),
        json!({"t": "un", "op": "not", "x": {"t": "bin", "op": ">",
            "l": {"t": "ident", "v": "size"}, "r": {"t": "size", "v": 1_000_000}}})
    );
}

#[test]
fn comparisons_are_not_associative() {
    assert_eq!(error_code("filter(a < b < c)"), ErrorCode::Syntax);
}

#[test]
fn call_in_value_position_is_function_call() {
    assert_eq!(
        only_value("filter(ago(1h))"),
        json!({"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 3_600_000}]})
    );
    assert_eq!(
        only_value("filter(now())"),
        json!({"t": "call", "fn": "now", "args": []})
    );
    assert_eq!(only_value("verify(f(x))")["t"], "call");
    assert_eq!(only_value("filter(lower(name) == \"a\")")["l"]["t"], "call");
}

#[test]
fn bare_identifier_argument_is_identifier() {
    assert_eq!(
        only_value("verify(count)"),
        json!({"t": "ident", "v": "count"})
    );
    assert_eq!(only_value("sort(size)"), json!({"t": "ident", "v": "size"}));
}

#[test]
fn argument_with_pipe_is_sub_pipeline() {
    let value = only_value("verify(run([\"go\"], cwd: \"/a\") |> assert(exit_code: 0))");
    assert_eq!(
        value,
        json!({"t": "sub", "p": {"t": "pipeline",
            "head": stage("run", json!([
                {"value": {"t": "list", "items": [{"t": "text", "v": "go"}]}},
                {"name": "cwd", "value": {"t": "text", "v": "/a"}}])),
            "stages": [stage("assert", json!([{"name": "exit_code", "value": {"t": "int", "v": 0}}]))]}})
    );
}

#[test]
fn sub_pipeline_argument_may_start_with_reference() {
    assert_eq!(
        only_value("verify(@x |> count())"),
        json!({"t": "sub", "p": {"t": "pipeline", "head": {"t": "ref", "name": "x"},
            "stages": [stage("count", json!([]))]}})
    );
}

#[test]
fn sub_pipeline_argument_may_be_named() {
    let tree = statements("schedule.create(\"b\", action: run([\"x\"]) |> count())");
    let action = &tree[0]["head"]["args"][1];
    assert_eq!(action["name"], "action");
    assert_eq!(action["value"]["t"], "sub");
    assert_eq!(action["value"]["p"]["stages"][0]["name"], "count");
}

#[test]
fn call_with_named_argument_is_single_stage_sub_pipeline() {
    assert_eq!(
        only_value("verify(run([\"go\"], cwd: \"/a\"))"),
        single_stage_sub(stage(
            "run",
            json!([
            {"value": {"t": "list", "items": [{"t": "text", "v": "go"}]}},
            {"name": "cwd", "value": {"t": "text", "v": "/a"}}])
        ))
    );
}

#[test]
fn call_with_group_name_is_single_stage_sub_pipeline() {
    assert_eq!(
        only_value("verify(git.status())"),
        single_stage_sub(stage("git.status", json!([])))
    );
    assert_eq!(
        only_value("verify(git.status(\"/a\"))"),
        single_stage_sub(stage(
            "git.status",
            json!([{"value": {"t": "text", "v": "/a"}}])
        ))
    );
}

#[test]
fn parenthesized_call_is_single_stage_sub_pipeline() {
    assert_eq!(
        only_value("verify((count()))"),
        single_stage_sub(stage("count", json!([])))
    );
    assert_eq!(
        only_value("verify((f(x)))"),
        single_stage_sub(stage("f", json!([{"value": {"t": "ident", "v": "x"}}])))
    );
    assert_same_tree(
        "verify((run([\"go\"], cwd: \"/a\")))",
        "verify(run([\"go\"], cwd: \"/a\"))",
    );
    assert_same_tree("verify((git.status()))", "verify(git.status())");
}

#[test]
fn parenthesized_pipeline_is_sub_pipeline() {
    assert_same_tree(
        "verify((run([\"go\"]) |> assert(exit_code: 0)))",
        "verify(run([\"go\"]) |> assert(exit_code: 0))",
    );
}

#[test]
fn parenthesized_expression_stays_expression() {
    assert_eq!(
        only_value("verify((count))"),
        json!({"t": "ident", "v": "count"})
    );
    assert_eq!(only_value("filter((x -1))")["op"], "-");
    assert_eq!(only_value("filter((f(x) + 1))")["t"], "bin");
    assert_eq!(only_value("filter((size > 1))")["t"], "bin");
    assert_eq!(
        only_value("filter(not hidden and (size >= 1 or a))")["r"]["op"],
        "or"
    );
}

#[test]
fn group_name_without_parentheses_is_field() {
    assert_eq!(
        only_value("verify(git.status)"),
        json!({"t": "field", "name": "status", "of": {"t": "ident", "v": "git"}})
    );
}

#[test]
fn call_inside_expression_keeps_named_arguments_invalid() {
    assert_eq!(error_code("filter([f(a: 1)])"), ErrorCode::Syntax);
    assert_eq!(error_code("filter(f(a: 1) + 1)"), ErrorCode::Syntax);
}

#[test]
fn nested_sub_pipelines_use_call_notation() {
    let value = only_value("verify(run([\"a\"]) |> verify(git.status() |> count()))");
    let inner = &value["p"]["stages"][0]["args"][0]["value"];
    assert_eq!(inner["t"], "sub");
    assert_eq!(inner["p"]["head"]["name"], "git.status");
    assert_eq!(inner["p"]["stages"][0]["name"], "count");
}

#[test]
fn let_statement_binds_pipeline() {
    let tree = statements(
        "let recent = find(\"/srv/app/src\", glob: \"**/*.ts\") |> filter(modified > ago(1h))\n\
         @recent |> count()",
    );
    assert_eq!(tree[0]["t"], "let");
    assert_eq!(tree[0]["name"], "recent");
    assert_eq!(tree[0]["value"]["head"]["name"], "find");
    assert_eq!(tree[0]["value"]["stages"][0]["name"], "filter");
    assert_eq!(tree[1]["head"], json!({"t": "ref", "name": "recent"}));
}

#[test]
fn statements_are_separated_by_line_breaks() {
    let tree = statements("history()\n\n\njournal.space()\n");
    assert_eq!(tree.as_array().unwrap().len(), 2);
    assert_eq!(error_code("history() journal.space()"), ErrorCode::Syntax);
}

#[test]
fn empty_program_is_rejected() {
    assert_eq!(error_code(""), ErrorCode::Syntax);
    assert_eq!(error_code("\n# only a comment\n"), ErrorCode::Syntax);
}

#[test]
fn literals_lists_maps_and_ranges_come_from_shared_grammar() {
    let tree = statements(
        "edit(\"/a\", changes: [{replace: \"x\", with: \"y\"}, {delete_lines: 10..12}], \
         lines: ..5, all: .., size: 1.5mb, wait: 1h30m, raw: r#\"a\"b\"#, nothing: null)",
    );
    let args = &tree[0]["head"]["args"];
    assert_eq!(
        args[1]["value"]["items"][1]["entries"][0]["v"],
        json!({"t": "range", "lo": 10, "hi": 12})
    );
    assert_eq!(args[2]["value"], json!({"t": "range", "hi": 5}));
    assert_eq!(args[3]["value"], json!({"t": "range"}));
    assert_eq!(args[4]["value"], json!({"t": "size", "v": 1_500_000}));
    assert_eq!(args[5]["value"], json!({"t": "dur", "v": 5_400_000}));
    assert_eq!(args[6]["value"], json!({"t": "text", "v": "a\"b"}));
    assert_eq!(args[7]["value"], json!({"t": "null"}));
}

#[test]
fn spread_is_accepted_only_in_list() {
    assert_eq!(
        only_value("info([...@logs.path])")["items"][0],
        json!({"t": "spread", "x": {"t": "ref", "name": "logs", "fields": ["path"]}})
    );
    assert_eq!(error_code("info(...@logs.path)"), ErrorCode::Syntax);
}

#[test]
fn duplicate_map_key_is_rejected() {
    assert_eq!(
        error_code("edit(\"/a\", changes: [{a: 1, a: 2}])"),
        ErrorCode::Syntax
    );
}

#[test]
fn word_notation_is_not_accepted() {
    assert_eq!(error_code("find \"/a\" glob: \"*.go\""), ErrorCode::Syntax);
    assert_eq!(error_code("find(\"/a\") |> sort size"), ErrorCode::Syntax);
}

#[test]
fn deep_nesting_is_rejected_without_stack_overflow() {
    let depth = 1_000;
    let text = format!("{}count(){}", "a.b(".repeat(depth), ")".repeat(depth));
    assert_eq!(error_code(&text), ErrorCode::Syntax);
    let list = format!("f({}1{})", "[".repeat(depth), "]".repeat(depth));
    assert_eq!(error_code(&list), ErrorCode::Syntax);
}

#[test]
fn moderate_nesting_is_accepted() {
    let depth = 20;
    let text = format!("{}count(){}", "a.b(".repeat(depth), ")".repeat(depth));
    let mut value = head(&text);
    for _ in 1..depth {
        assert_eq!(value["name"], "a.b");
        value = value["args"][0]["value"]["p"]["head"].clone();
    }
    assert_eq!(value["name"], "a.b");
}

#[test]
fn raw_text_is_read_in_every_argument_position() {
    let tree = statements("write(r\"/a\", r#\"x\"y\"#, mode: r\"z\")");
    let args = &tree[0]["head"]["args"];
    assert_eq!(args[0]["value"], json!({"t": "text", "v": "/a"}));
    assert_eq!(args[1]["value"], json!({"t": "text", "v": "x\"y"}));
    assert_eq!(args[2]["value"], json!({"t": "text", "v": "z"}));
    assert_eq!(
        only_value("verify(run([r\"go\"], cwd: r\"/a\") |> count())")["p"]["head"]["args"][1]["value"],
        json!({"t": "text", "v": "/a"})
    );
}
