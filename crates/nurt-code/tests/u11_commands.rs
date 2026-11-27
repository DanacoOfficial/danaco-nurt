// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `code.symbols` and `code.outline` stages on a project in
//! the virtual file system, with the listing double and a scripted
//! semantic provider.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u11_support;

use std::sync::Arc;

use nurt_code::commands::outline::CodeOutline;
use nurt_code::commands::symbols::CodeSymbols;
use nurt_code::semantic::{Location, SemanticErrorKind, SemanticProvider};
use nurt_contracts::error::ErrorCode;
use nurt_contracts::record::{SymbolKind as RecordKind, SymbolSource};
use nurt_contracts::report::Notable;
use nurt_contracts::services::TextPosition;
use nurt_contracts::stage::{ExecCtx, StageImpl};
use nurt_contracts::value::{Regex, Value};

use u11_support::{
    ROOT, Script, ScriptedSemantic, Workspace, args, names, path_value, relative, run, run_symbols,
    server_symbol, text, word,
};

const SHAPES_PY: &str = "class Shape:
    def area(self):
        return 0


def parse_config(path):
    return path


def _private_helper():
    return parse_config(\"x\")
";

const SERVER_GO: &str = "package server

func ParseRequest() {}

func parseHeader() {}
";

const LIB_RS: &str = "pub fn parse() {}

fn parse_inner() {}

pub struct Parser;
";

const TEST_SHAPES_PY: &str = "def test_area():
    parse_config(\"y\")
";

const APP_TS: &str = "export function parseUrl(url: string): string {
  return url;
}
";

const TOOL: &str = "#!/usr/bin/env python3
def parse_cli():
    pass
";

fn project() -> Workspace {
    let workspace = Workspace::new();
    workspace.write_all(&[
        ("src/shapes.py", SHAPES_PY),
        ("src/server.go", SERVER_GO),
        ("src/lib.rs", LIB_RS),
        ("tests/test_shapes.py", TEST_SHAPES_PY),
        ("web/app.ts", APP_TS),
        ("bin/tool", TOOL),
        ("config/settings.json", "{\"parse\": 1}\n"),
        ("node_modules/dep/index.js", "function parseDep() {}\n"),
        (".git/hooks/parse_hook.py", "def parse_hook():\n    pass\n"),
    ]);
    workspace
}

fn symbols(ctx: &ExecCtx, pairs: &[(&str, Value)]) -> Vec<nurt_contracts::record::Symbol> {
    symbols_with(&CodeSymbols::new(None), ctx, pairs)
}

fn symbols_with(
    stage: &CodeSymbols,
    ctx: &ExecCtx,
    pairs: &[(&str, Value)],
) -> Vec<nurt_contracts::record::Symbol> {
    run_symbols(stage, &args("code.symbols", pairs), ctx).unwrap()
}

fn symbols_error(stage: &dyn StageImpl, ctx: &ExecCtx, pairs: &[(&str, Value)]) -> ErrorCode {
    match run(stage, &args("code.symbols", pairs), ctx) {
        Ok(records) => panic!("expected an error, got {} records", records.len()),
        Err(error) => error.code(),
    }
}

fn notes_of(ctx: &ExecCtx, code: Notable) -> Vec<String> {
    ctx.take_notes()
        .into_iter()
        .filter(|entry| entry.code == code)
        .map(|entry| entry.detail.unwrap_or_default())
        .collect()
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn exact_name_finds_one_definition() {
    let workspace = project();
    let found = symbols(
        &workspace.ctx(),
        &[("name", text("parse")), ("match", word("exact"))],
    );
    assert_eq!(names(&found), ["parse"]);
    let symbol = &found[0];
    assert_eq!(relative(symbol), "src/lib.rs");
    assert_eq!(symbol.kind, RecordKind::Function);
    assert_eq!(symbol.language, "rust");
    assert_eq!(symbol.source, SymbolSource::Syntax);
    assert_eq!((symbol.line, symbol.column), (1, 8));
    assert_eq!(symbol.end_line, 1);
    assert_eq!(symbol.exported, Some(true));
    assert!(symbol.etag.starts_with("sha256:"));
    assert_eq!(symbol.signature, None);
    assert_eq!(symbol.references, None);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn prefix_match_ranks_the_exact_name_first_and_skips_dependencies() {
    let workspace = project();
    let found = symbols(
        &workspace.ctx(),
        &[("name", text("parse")), ("match", word("prefix"))],
    );
    let listed = names(&found);
    assert_eq!(listed[0], "parse");
    for expected in [
        "ParseRequest",
        "parseHeader",
        "parse_inner",
        "Parser",
        "parse_config",
        "parseUrl",
        "parse_cli",
    ] {
        assert!(
            listed.contains(&expected.to_owned()),
            "{expected} in {listed:?}"
        );
    }
    assert!(!listed.contains(&"parseDep".to_owned()));
    assert!(!listed.contains(&"parse_hook".to_owned()));
    let mut paths: Vec<String> = found[1..].iter().map(relative).collect();
    let sorted = {
        let mut copy = paths.clone();
        copy.sort();
        copy
    };
    assert_eq!(paths, sorted);
    paths.dedup();
    assert!(paths.contains(&"bin/tool".to_owned()));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn data_files_are_searched_only_when_their_language_is_named() {
    let workspace = project();
    let ctx = workspace.ctx();
    let default = symbols(&ctx, &[("name", text("parse")), ("match", word("exact"))]);
    assert!(default.iter().all(|symbol| symbol.language != "json"));
    let json = symbols(
        &ctx,
        &[
            ("name", text("parse")),
            ("match", word("exact")),
            ("language", word("json")),
        ],
    );
    assert_eq!(names(&json), ["parse"]);
    assert_eq!(relative(&json[0]), "config/settings.json");
    assert_eq!(json[0].kind, RecordKind::Field);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn test_files_are_excluded_by_the_tests_preset() {
    let workspace = project();
    let ctx = workspace.ctx();
    let query = [("name", text("test")), ("match", word("prefix"))];
    assert_eq!(names(&symbols(&ctx, &query)), ["test_area"]);
    let mut excluded = query.to_vec();
    excluded.push(("exclude_preset", Value::List(vec![word("tests")])));
    assert!(symbols(&ctx, &excluded).is_empty());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn glob_restricts_the_searched_files() {
    let workspace = project();
    let found = symbols(
        &workspace.ctx(),
        &[
            ("name", text("parse")),
            ("match", word("prefix")),
            ("glob", Value::Glob("src/**".to_owned())),
        ],
    );
    assert!(!found.is_empty());
    assert!(
        found
            .iter()
            .all(|symbol| relative(symbol).starts_with("src/"))
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn exclusions_reach_the_listing_service() {
    let workspace = project();
    let found = symbols(
        &workspace.ctx(),
        &[
            ("name", text("parse")),
            ("match", word("prefix")),
            ("exclude", Value::List(vec![text("web")])),
        ],
    );
    assert!(!names(&found).contains(&"parseUrl".to_owned()));
    let requests = workspace.listing.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].exclude, ["web"]);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn kind_and_language_filter_the_symbols() {
    let workspace = project();
    let ctx = workspace.ctx();
    let classes = symbols(&ctx, &[("kind", word("class"))]);
    assert_eq!(names(&classes), ["Shape"]);
    let go = symbols(&ctx, &[("language", word("go"))]);
    assert_eq!(names(&go), ["ParseRequest", "parseHeader"]);
    let golang = symbols(&ctx, &[("language", text("golang"))]);
    assert_eq!(names(&golang), names(&go));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn exported_only_keeps_public_symbols() {
    let workspace = project();
    let ctx = workspace.ctx();
    let go = symbols(
        &ctx,
        &[
            ("language", word("go")),
            ("exported_only", Value::Bool(true)),
        ],
    );
    assert_eq!(names(&go), ["ParseRequest"]);
    let rust = symbols(
        &ctx,
        &[
            ("language", word("rust")),
            ("exported_only", Value::Bool(true)),
        ],
    );
    assert_eq!(names(&rust), ["parse", "Parser"]);
    let python = symbols(
        &ctx,
        &[
            ("name", text("helper")),
            ("match", word("substring")),
            ("exported_only", Value::Bool(true)),
        ],
    );
    assert!(python.is_empty());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn name_list_and_regex_select_several_names() {
    let workspace = project();
    let ctx = workspace.ctx();
    let listed = symbols(
        &ctx,
        &[
            (
                "name",
                Value::List(vec![text("parse_config"), text("parseUrl")]),
            ),
            ("match", word("exact")),
        ],
    );
    let mut listed = names(&listed);
    listed.sort();
    assert_eq!(listed, ["parseUrl", "parse_config"]);
    let regex = symbols(
        &ctx,
        &[("name", Value::Regex(Regex::new("^parse[A-Z]", "").unwrap()))],
    );
    let mut regex = names(&regex);
    regex.sort();
    assert_eq!(regex, ["parseHeader", "parseUrl"]);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn fuzzy_match_is_the_default() {
    let workspace = project();
    let found = symbols(&workspace.ctx(), &[("name", text("pconf"))]);
    assert_eq!(names(&found), ["parse_config"]);
    let words = symbols(&workspace.ctx(), &[("name", text("parse_request"))]);
    assert_eq!(names(&words), ["ParseRequest"]);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn references_are_counted_across_files_of_the_language() {
    let workspace = project();
    let found = symbols(
        &workspace.ctx(),
        &[
            ("name", text("parse_config")),
            ("match", word("exact")),
            ("with_references", Value::Bool(true)),
            ("with_signature", Value::Bool(true)),
        ],
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].references, Some(2));
    let signature = found[0].signature.as_deref().unwrap();
    assert!(signature.contains("parse_config(path)"), "{signature}");
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn single_file_is_searched_without_listing() {
    let workspace = project();
    let ctx = workspace.ctx();
    let found = symbols(
        &ctx,
        &[("path", path_value(&workspace.path("src/shapes.py")))],
    );
    assert_eq!(
        names(&found),
        ["Shape", "area", "parse_config", "_private_helper"]
    );
    assert_eq!(found[1].container.as_deref(), Some("Shape"));
    assert_eq!(found[1].kind, RecordKind::Method);
    assert!(workspace.listing.requests().is_empty());
    let relative_path = symbols(&ctx, &[("path", Value::Path("src/lib.rs".to_owned()))]);
    assert_eq!(names(&relative_path), ["parse", "parse_inner", "Parser"]);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn file_of_an_unknown_language_is_rejected() {
    let workspace = project();
    workspace.write("notes/readme.txt", "plain text\n");
    let stage = CodeSymbols::new(None);
    let code = symbols_error(
        &stage,
        &workspace.ctx(),
        &[("path", path_value(&workspace.path("notes/readme.txt")))],
    );
    assert_eq!(code, ErrorCode::UnsupportedLanguage);
}

#[test]
fn invalid_arguments_are_rejected() {
    let workspace = project();
    let ctx = workspace.ctx();
    let stage = CodeSymbols::new(None);
    assert_eq!(
        symbols_error(&stage, &ctx, &[("kind", word("widget"))]),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        symbols_error(&stage, &ctx, &[("language", word("cobol"))]),
        ErrorCode::UnsupportedLanguage
    );
    assert_eq!(
        symbols_error(&stage, &ctx, &[("name", Value::Int(3))]),
        ErrorCode::Type
    );
    assert_eq!(
        symbols_error(&stage, &ctx, &[("exclude_preset", word("docs"))]),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        symbols_error(&stage, &ctx, &[("glob", Value::Glob("a[".to_owned()))]),
        ErrorCode::InvalidValue
    );
    let match_error = run(
        &stage,
        &args(
            "code.symbols",
            &[("name", text("x")), ("match", word("glob"))],
        ),
        &ctx,
    );
    assert!(match_error.is_err());
}

#[test]
fn search_without_path_needs_a_base_directory() {
    let workspace = project();
    let stage = CodeSymbols::new(None);
    let code = symbols_error(
        &stage,
        &workspace.ctx_without_state(),
        &[("name", text("parse"))],
    );
    assert_eq!(code, ErrorCode::MissingParam);
}

fn server_answers() -> Arc<ScriptedSemantic> {
    let lib = std::path::Path::new(ROOT).join("src/lib.rs");
    let outside = std::path::Path::new("/elsewhere/other.rs");
    ScriptedSemantic::new(Script {
        workspace: Ok(vec![
            server_symbol("parse", RecordKind::Function, &lib, 1),
            server_symbol("Parser", RecordKind::Struct, &lib, 5),
            server_symbol("parse", RecordKind::Function, &lib, 1),
            server_symbol("parse_remote", RecordKind::Function, outside, 1),
        ]),
        references: Ok(vec![
            Location {
                path: lib.clone(),
                start: TextPosition { line: 3, column: 1 },
                end: TextPosition { line: 3, column: 6 },
            };
            3
        ]),
        ..Script::default()
    })
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn auto_source_asks_the_server_first() {
    let workspace = project();
    let provider = server_answers();
    let stage = CodeSymbols::new(Some(provider.clone() as Arc<dyn SemanticProvider>));
    let ctx = workspace.ctx();
    let found = symbols_with(
        &stage,
        &ctx,
        &[("name", text("parse")), ("match", word("prefix"))],
    );
    assert_eq!(names(&found), ["parse", "Parser"]);
    assert!(
        found
            .iter()
            .all(|symbol| symbol.source == SymbolSource::Lsp)
    );
    assert_eq!(found[0].language, "rust");
    assert!(found[0].etag.starts_with("sha256:"));
    assert_eq!(provider.queries(), ["parse"]);
    let structs = symbols_with(
        &stage,
        &ctx,
        &[
            ("name", text("parse")),
            ("match", word("prefix")),
            ("kind", word("struct")),
        ],
    );
    assert_eq!(names(&structs), ["Parser"]);
}

#[test]
fn server_references_are_counted_by_the_server() {
    let workspace = project();
    let stage = CodeSymbols::new(Some(server_answers() as Arc<dyn SemanticProvider>));
    let found = symbols_with(
        &stage,
        &workspace.ctx(),
        &[
            ("name", text("parse")),
            ("match", word("exact")),
            ("with_references", Value::Bool(true)),
            ("with_signature", Value::Bool(true)),
        ],
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].references, Some(3));
    assert_eq!(found[0].signature.as_deref(), Some("parse()"));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn unavailable_server_falls_back_to_syntax_with_a_note() {
    let workspace = project();
    let stage = CodeSymbols::new(Some(
        ScriptedSemantic::failing(SemanticErrorKind::Unavailable) as Arc<dyn SemanticProvider>,
    ));
    let ctx = workspace.ctx();
    let found = symbols_with(
        &stage,
        &ctx,
        &[("name", text("parse")), ("match", word("exact"))],
    );
    assert_eq!(names(&found), ["parse"]);
    assert_eq!(found[0].source, SymbolSource::Syntax);
    let notes = notes_of(&ctx, Notable::SemanticFallback);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("no server for the language"), "{notes:?}");
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn missing_provider_falls_back_to_syntax_with_a_note() {
    let workspace = project();
    let ctx = workspace.ctx();
    let found = symbols(&ctx, &[("name", text("parse")), ("match", word("exact"))]);
    assert_eq!(names(&found), ["parse"]);
    assert_eq!(notes_of(&ctx, Notable::SemanticFallback).len(), 1);
}

#[test]
fn lsp_source_requires_a_server() {
    let workspace = project();
    let ctx = workspace.ctx();
    let query = [("name", text("parse")), ("source", word("lsp"))];
    let failing = CodeSymbols::new(Some(
        ScriptedSemantic::failing(SemanticErrorKind::Unavailable) as Arc<dyn SemanticProvider>,
    ));
    assert_eq!(
        symbols_error(&failing, &ctx, &query),
        ErrorCode::ExternalMissing
    );
    let timed_out = CodeSymbols::new(Some(
        ScriptedSemantic::failing(SemanticErrorKind::TimedOut) as Arc<dyn SemanticProvider>
    ));
    assert_eq!(symbols_error(&timed_out, &ctx, &query), ErrorCode::Timeout);
    let none = CodeSymbols::new(None);
    assert_eq!(
        symbols_error(&none, &ctx, &query),
        ErrorCode::ExternalMissing
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn syntax_source_and_unnamed_searches_do_not_ask_the_server() {
    let workspace = project();
    let provider = server_answers();
    let stage = CodeSymbols::new(Some(provider.clone() as Arc<dyn SemanticProvider>));
    let ctx = workspace.ctx();
    let syntax = symbols_with(
        &stage,
        &ctx,
        &[
            ("name", text("parse")),
            ("match", word("exact")),
            ("source", word("syntax")),
        ],
    );
    assert_eq!(syntax[0].source, SymbolSource::Syntax);
    let unnamed = symbols_with(&stage, &ctx, &[("language", word("go"))]);
    assert_eq!(names(&unnamed), ["ParseRequest", "parseHeader"]);
    assert!(provider.queries().is_empty());
}

#[test]
fn exported_only_conflicts_with_the_lsp_source() {
    let workspace = project();
    let stage = CodeSymbols::new(Some(server_answers() as Arc<dyn SemanticProvider>));
    let code = symbols_error(
        &stage,
        &workspace.ctx(),
        &[
            ("name", text("parse")),
            ("source", word("lsp")),
            ("exported_only", Value::Bool(true)),
        ],
    );
    assert_eq!(code, ErrorCode::ParamConflict);
}

fn outline_args(pairs: &[(&str, Value)]) -> nurt_contracts::stage::BoundArgs {
    args("code.outline", pairs)
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn outline_lists_symbols_in_source_order_with_signatures() {
    let workspace = project();
    let stage = CodeOutline::new(None);
    let found = run_symbols(
        &stage,
        &outline_args(&[("path", path_value(&workspace.path("src/shapes.py")))]),
        &workspace.ctx(),
    )
    .unwrap();
    assert_eq!(
        names(&found),
        ["Shape", "area", "parse_config", "_private_helper"]
    );
    assert!(found.iter().all(|symbol| symbol.signature.is_some()));
    assert_eq!(found[1].container.as_deref(), Some("Shape"));
    assert_eq!(found[3].exported, Some(false));
    assert!(
        found
            .iter()
            .all(|symbol| symbol.source == SymbolSource::Syntax)
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn outline_depth_limits_nesting() {
    let workspace = project();
    let stage = CodeOutline::new(None);
    let found = run_symbols(
        &stage,
        &outline_args(&[
            ("path", Value::Path("src/shapes.py".to_owned())),
            ("depth", Value::Int(0)),
        ]),
        &workspace.ctx(),
    )
    .unwrap();
    assert_eq!(names(&found), ["Shape", "parse_config", "_private_helper"]);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn outline_of_a_language_without_grammar_asks_the_server() {
    let workspace = project();
    let path = workspace.write("src/main.zig", "pub fn main() void {}\n");
    let provider = ScriptedSemantic::new(Script {
        document: Ok(vec![
            server_symbol("main", RecordKind::Function, &path, 1),
            nurt_code::semantic::SemanticSymbol {
                depth: 1,
                container: Some("main".to_owned()),
                ..server_symbol("inner", RecordKind::Variable, &path, 1)
            },
        ]),
        ..Script::default()
    });
    let stage = CodeOutline::new(Some(provider.clone() as Arc<dyn SemanticProvider>));
    let ctx = workspace.ctx();
    let all = run_symbols(&stage, &outline_args(&[("path", path_value(&path))]), &ctx).unwrap();
    assert_eq!(names(&all), ["main", "inner"]);
    assert!(all.iter().all(|symbol| symbol.source == SymbolSource::Lsp));
    assert!(all.iter().all(|symbol| symbol.language == "zig"));
    let top = run_symbols(
        &stage,
        &outline_args(&[("path", path_value(&path)), ("depth", Value::Int(0))]),
        &ctx,
    )
    .unwrap();
    assert_eq!(names(&top), ["main"]);
    assert_eq!(provider.documents(), [path.clone(), path]);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn outline_without_grammar_or_server_is_rejected() {
    let workspace = project();
    let path = workspace.write("src/main.zig", "pub fn main() void {}\n");
    let stage = CodeOutline::new(None);
    let error = run(
        &stage,
        &outline_args(&[("path", path_value(&path))]),
        &workspace.ctx(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    let failing = CodeOutline::new(Some(
        ScriptedSemantic::failing(SemanticErrorKind::Unavailable) as Arc<dyn SemanticProvider>,
    ));
    let error = run(
        &failing,
        &outline_args(&[("path", path_value(&path))]),
        &workspace.ctx(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    assert!(
        error.to_string().contains("no server for the language"),
        "{error}"
    );
    let timed_out = CodeOutline::new(Some(
        ScriptedSemantic::failing(SemanticErrorKind::TimedOut) as Arc<dyn SemanticProvider>
    ));
    let error = run(
        &timed_out,
        &outline_args(&[("path", path_value(&path))]),
        &workspace.ctx(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Timeout);
}

#[test]
fn outline_needs_a_path() {
    let workspace = project();
    let error = run(
        &CodeOutline::new(None),
        &outline_args(&[]),
        &workspace.ctx(),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::MissingParam);
}

#[test]
fn stages_describe_themselves_as_sources() {
    let symbols = CodeSymbols::new(None).descriptor();
    let outline = CodeOutline::new(None).descriptor();
    assert_eq!(symbols.command, "code.symbols");
    assert_eq!(outline.command, "code.outline");
    assert!(symbols.source && outline.source);
}
