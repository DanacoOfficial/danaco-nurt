// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-code/tests/u12_structural.rs
// Responsibility:   Code service: structural replacement with templates, classification of.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Code service: structural replacement with templates, classification of
//! source spans, enclosing declarations, syntax checks and references.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use nurt_code::pattern::node_record;
use nurt_code::service::CodeAnalysis;
use nurt_code::structural::replacement_edits;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::services::{
    ClassifiedSpan, CodeService, LanguageId, ReferenceRole, SpanClass, SymbolKind, SyntaxMatch,
};

fn language(id: &str) -> LanguageId {
    LanguageId::new(id).unwrap()
}

fn search(service: &CodeAnalysis, id: &str, pattern: &str, source: &str) -> Vec<SyntaxMatch> {
    let compiled = service.compile_pattern(pattern, &language(id)).unwrap();
    service
        .syntax_search(source.as_bytes(), compiled.as_ref())
        .unwrap()
}

/// Applies the replacements of `pattern` by `template` to `source`.
fn replace_all(id: &str, pattern: &str, template: &str, source: &str) -> String {
    let service = CodeAnalysis::builtin();
    let found = search(&service, id, pattern, source);
    let edits = replacement_edits(source.as_bytes(), &found, template);
    let mut out = source.to_owned();
    for edit in edits.iter().rev() {
        let start = usize::try_from(edit.bytes.start).unwrap();
        let end = usize::try_from(edit.bytes.end).unwrap();
        out.replace_range(start..end, &edit.text);
    }
    out
}

fn text_of<'s>(source: &'s str, span: &ClassifiedSpan) -> &'s str {
    let start = usize::try_from(span.bytes.start).unwrap();
    let end = usize::try_from(span.bytes.end).unwrap();
    &source[start..end]
}

fn classify(id: &str, source: &str) -> Vec<(SpanClass, String)> {
    let service = CodeAnalysis::builtin();
    service
        .classify_spans(source.as_bytes(), &language(id))
        .unwrap()
        .iter()
        .map(|span| (span.class, text_of(source, span).to_owned()))
        .collect()
}

#[test]
fn template_replaces_single_line_matches() {
    let out = replace_all(
        "javascript",
        "var $A = $B;",
        "let $A = $B;",
        "var a = 1;\nvar b = f(2);\n",
    );
    assert_eq!(out, "let a = 1;\nlet b = f(2);\n");
}

#[test]
fn multi_line_template_lines_follow_the_indentation_of_the_match() {
    let source = "fn main() {\n    let v = compute(a, b);\n}\n";
    let out = replace_all(
        "rust",
        "compute($A, $B)",
        "compute(\n    $A,\n    $B,\n)",
        source,
    );
    assert_eq!(
        out,
        "fn main() {\n    let v = compute(\n        a,\n        b,\n    );\n}\n"
    );
}

#[test]
fn replacement_in_a_crlf_document_uses_crlf_line_endings() {
    let source = "fn main() {\r\n    let v = compute(a, b);\r\n}\r\n";
    let out = replace_all(
        "rust",
        "compute($A, $B)",
        "compute(\n    $A,\n    $B,\n)",
        source,
    );
    assert_eq!(
        out,
        "fn main() {\r\n    let v = compute(\r\n        a,\r\n        b,\r\n    );\r\n}\r\n"
    );
}

#[test]
fn multi_line_capture_on_the_match_line_keeps_its_lines() {
    let source = "function run() {\n  wrap(function () {\n    go();\n  });\n}\n";
    let out = replace_all("javascript", "wrap($F)", "guard($F)", source);
    assert_eq!(
        out,
        "function run() {\n  guard(function () {\n    go();\n  });\n}\n"
    );
}

#[test]
fn multi_line_capture_on_a_later_line_is_reindented_under_the_template() {
    let source = "class A:\n    def f(self):\n        return compute(\n            first,\n            second)\n";
    let out = replace_all(
        "python",
        "compute($$$ARGS)",
        "wrapped(\n    $$$ARGS\n)",
        source,
    );
    assert_eq!(
        out,
        "class A:\n    def f(self):\n        return wrapped(\n            first,\n            second\n        )\n"
    );
}

#[test]
fn nested_matches_are_replaced_once_by_the_outer_match() {
    let out = replace_all("python", "f($A)", "g($A)", "f(f(1))\n");
    assert_eq!(out, "g(f(1))\n");
}

#[test]
fn service_renders_templates_with_inferred_indentation() {
    let service = CodeAnalysis::builtin();
    let found = search(
        &service,
        "go",
        "fmt.Println($$$A)",
        "package main\nfunc f() {\n\tfmt.Println(1, 2)\n}\n",
    );
    let text = service
        .render_template(&found[0], "log.Print($$$A)")
        .unwrap();
    assert_eq!(text, "log.Print(1, 2)");
}

#[test]
fn node_records_carry_position_text_and_captures() {
    let service = CodeAnalysis::builtin();
    let found = search(&service, "python", "print($A)", "x = 1\nprint(x)\n");
    let record = node_record("/p/a.py", &language("python"), "e1", &found[0]);
    assert_eq!((record.line, record.column, record.end_line), (2, 1, 2));
    assert_eq!((record.start, record.end), (6, 14));
    assert_eq!(record.kind, "call");
    assert_eq!(record.text, "print(x)");
    assert_eq!(record.captures.unwrap()["A"], "x");
}

#[test]
fn search_in_a_file_with_a_byte_order_mark_keeps_offsets_of_the_file() {
    let service = CodeAnalysis::builtin();
    let source = "\u{feff}using System;\nclass A { void F() { Console.WriteLine(1); } }\n";
    let found = search(&service, "csharp", "Console.WriteLine($A)", source);
    assert_eq!(found.len(), 1);
    let start = usize::try_from(found[0].span.bytes.start).unwrap();
    assert_eq!(start, source.find("Console").unwrap());
    assert_eq!(found[0].span.start.line, 2);
    assert_eq!(found[0].captures[0].text, "1");
}

#[test]
fn checking_syntax_of_files_with_a_byte_order_mark_finds_no_error() {
    let service = CodeAnalysis::builtin();
    for (id, source) in [
        ("csharp", "\u{feff}using System;\nclass A { }\n"),
        ("powershell", "\u{feff}function Get-A { 1 }\n"),
    ] {
        let issues = service
            .check_syntax(source.as_bytes(), &language(id))
            .unwrap();
        assert!(issues.is_empty(), "{id}: {issues:?}");
    }
}

#[test]
fn invalid_patterns_fail_with_pattern_syntax_and_position() {
    let service = CodeAnalysis::builtin();
    for (id, pattern) in [
        ("python", "print($A"),
        ("rust", ""),
        ("javascript", "$$$A"),
        ("go", "a := 1; b := 2"),
    ] {
        let error = service.compile_pattern(pattern, &language(id)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::PatternSyntax, "{id} `{pattern}`");
        assert!(error.at.is_some(), "{id} `{pattern}`");
        assert_eq!(error.details["language"], id);
    }
}

#[test]
fn unknown_language_fails_with_unsupported_language() {
    let service = CodeAnalysis::builtin();
    let cobol = language("cobol");
    let error = service.compile_pattern("$A", &cobol).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
    let error = service.classify_spans(b"x", &cobol).unwrap_err();
    assert_eq!(error.code(), ErrorCode::UnsupportedLanguage);
}

#[test]
fn search_rejects_source_that_is_not_utf8() {
    let service = CodeAnalysis::builtin();
    let compiled = service.compile_pattern("$A", &language("python")).unwrap();
    let error = service
        .syntax_search(b"x = '\xff'\n", compiled.as_ref())
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Encoding);
}

#[test]
fn nested_block_comments_are_one_comment_span() {
    let spans = classify(
        "rust",
        "let a = 1; /* outer /* inner */ still */ let b = 2;",
    );
    assert_eq!(
        spans,
        [(
            SpanClass::Comment,
            "/* outer /* inner */ still */".to_owned()
        )]
    );
}

#[test]
fn multi_line_strings_are_one_string_span() {
    let source = "doc = \"\"\"first\nsecond # not a comment\n\"\"\"\n# real\n";
    let spans = classify("python", source);
    assert_eq!(
        spans,
        [
            (
                SpanClass::String,
                "\"\"\"first\nsecond # not a comment\n\"\"\"".to_owned()
            ),
            (SpanClass::Comment, "# real".to_owned()),
        ]
    );
}

#[test]
fn template_literal_substitutions_are_code_with_their_own_comments() {
    let source = "const s = `a ${ /* c */ x } b`;";
    let spans = classify("javascript", source);
    assert_eq!(
        spans,
        [
            (SpanClass::String, "`a ".to_owned()),
            (SpanClass::Comment, "/* c */".to_owned()),
            (SpanClass::String, " b`".to_owned()),
        ]
    );
}

#[test]
fn interpolations_in_python_and_ruby_strings_are_code() {
    for (id, source) in [
        ("python", "s = f\"a {x} b\"\n"),
        ("ruby", "s = \"a #{x} b\"\n"),
    ] {
        let service = CodeAnalysis::builtin();
        let spans = service
            .classify_spans(source.as_bytes(), &language(id))
            .unwrap();
        let x = u64::try_from(source.find('x').unwrap()).unwrap();
        let a = u64::try_from(source.find('a').unwrap()).unwrap();
        assert_eq!(
            nurt_code::spans::class_at(&spans, x),
            SpanClass::Code,
            "{id}"
        );
        assert_eq!(
            nurt_code::spans::class_at(&spans, a),
            SpanClass::String,
            "{id}"
        );
    }
}

#[test]
fn spans_are_classified_in_every_language_of_the_registry() {
    let samples = [
        ("go", "package main\n// c\nvar s = \"x\"\n"),
        ("c", "/* c */ char *s = \"x\";\n"),
        ("cpp", "// c\nauto s = R\"(x)\";\n"),
        ("java", "/* c */ class A { String s = \"x\"; }\n"),
        ("csharp", "// c\nclass A { string s = @\"x\"; }\n"),
        ("php", "<?php # c\n$s = 'x';\n"),
        ("bash", "# c\necho \"x\"\n"),
        ("kotlin", "// c\nval s = \"x\"\n"),
        ("swift", "// c\nlet s = \"x\"\n"),
        ("typescript", "// c\nconst s: string = 'x';\n"),
        ("css", "/* c */ a { content: \"x\"; }\n"),
        ("yaml", "# c\nkey: \"x\"\n"),
        ("toml", "# c\nkey = \"x\"\n"),
        ("html", "<!-- c --><a title=\"x\"></a>\n"),
        ("powershell", "# c\n$s = \"x\"\n"),
    ];
    for (id, source) in samples {
        let spans = classify(id, source);
        let classes: Vec<SpanClass> = spans.iter().map(|(class, _)| *class).collect();
        assert!(classes.contains(&SpanClass::Comment), "{id}: {spans:?}");
        assert!(classes.contains(&SpanClass::String), "{id}: {spans:?}");
    }
}

#[test]
fn tokens_skip_whitespace_and_keep_literals_whole() {
    let service = CodeAnalysis::builtin();
    let tokens = service
        .tokens(b"x = \"a b\"  # c\n", &language("python"))
        .unwrap();
    let kinds: Vec<(&str, SpanClass)> = tokens.iter().map(|t| (t.kind.as_str(), t.class)).collect();
    assert_eq!(
        kinds,
        [
            ("identifier", SpanClass::Code),
            ("=", SpanClass::Code),
            ("string", SpanClass::String),
            ("comment", SpanClass::Comment),
        ]
    );
}

#[test]
fn enclosing_reports_the_innermost_declaration() {
    let service = CodeAnalysis::builtin();
    let source = "class Stack:\n    def push(self, item):\n        self.items.append(item)\n";
    let offset = u64::try_from(source.find("append").unwrap()).unwrap();
    let symbol = service
        .enclosing(source.as_bytes(), &language("python"), offset)
        .unwrap()
        .unwrap();
    assert_eq!(symbol.name, "push");
    assert_eq!(symbol.kind, SymbolKind::Method);
    assert_eq!(symbol.container.as_deref(), Some("Stack"));
    assert_eq!(symbol.span.start.line, 2);
    let outside = service
        .enclosing(b"x = 1\n", &language("python"), 0)
        .unwrap();
    assert!(outside.is_none());
}

#[test]
fn outline_and_find_symbol_use_qualified_names() {
    let service = CodeAnalysis::builtin();
    let python = language("python");
    let source =
        b"class Stack:\n    def push(self, item):\n        pass\n\ndef push(x):\n    pass\n";
    let top = service.outline(source, &python, Some(0)).unwrap();
    let names: Vec<&str> = top.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Stack", "push"]);
    let all = service.outline(source, &python, None).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(
        service.find_symbol(source, &python, "push").unwrap().len(),
        2
    );
    let qualified = service.find_symbol(source, &python, "Stack.push").unwrap();
    assert_eq!(qualified.len(), 1);
    assert_eq!(qualified[0].kind, SymbolKind::Method);
    assert!(
        service
            .find_symbol(source, &python, "Queue.push")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn syntax_errors_are_reported_with_kind_and_position() {
    let service = CodeAnalysis::builtin();
    let rust = language("rust");
    assert!(
        service
            .check_syntax(b"fn f() {}\n", &rust)
            .unwrap()
            .is_empty()
    );
    let issues = service
        .check_syntax(b"fn f() {\n    let x = ;\n}\n", &rust)
        .unwrap();
    assert!(!issues.is_empty());
    assert!(issues.iter().all(|issue| issue.kind.is_some()));
    assert_eq!(issues[0].span.start.line, 2);
    let missing = service
        .check_syntax(b"int main() { return 0 }\n", &language("c"))
        .unwrap();
    assert!(
        missing
            .iter()
            .any(|issue| issue.kind.as_deref() == Some("missing"))
    );
}

#[test]
fn references_report_roles_classes_and_containers() {
    let service = CodeAnalysis::builtin();
    let source = "use crate::run;\nfn run() {}\nfn main() {\n    // run later\n    run();\n    let s = \"run\";\n    let runner = 1;\n}\n";
    let found = service
        .references(source.as_bytes(), &language("rust"), "run")
        .unwrap();
    let summary: Vec<(u32, ReferenceRole, SpanClass)> = found
        .iter()
        .map(|r| (r.span.start.line, r.role, r.class))
        .collect();
    assert_eq!(
        summary,
        [
            (1, ReferenceRole::Import, SpanClass::Code),
            (2, ReferenceRole::Definition, SpanClass::Code),
            (4, ReferenceRole::Usage, SpanClass::Comment),
            (5, ReferenceRole::Usage, SpanClass::Code),
            (6, ReferenceRole::Usage, SpanClass::String),
        ]
    );
    assert_eq!(found[1].container, None);
    assert_eq!(found[3].container.as_deref(), Some("main"));
    assert_eq!(found[3].line_text, "    run();");
    assert!(
        service
            .references(source.as_bytes(), &language("rust"), "")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn function_metrics_count_parameters_and_decisions() {
    let service = CodeAnalysis::builtin();
    let source = b"fn f(a: i32, b: i32) -> i32 {\n    if a > b {\n        a\n    } else {\n        b\n    }\n}\n";
    let metrics = service.function_metrics(source, &language("rust")).unwrap();
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].name, "f");
    assert_eq!(metrics[0].parameters, 2);
    assert_eq!(metrics[0].cyclomatic, 2);
    assert_eq!(metrics[0].lines, 7);
}

#[test]
fn languages_are_listed_and_detected() {
    let service = CodeAnalysis::default();
    assert!(service.languages().len() >= 20);
    assert_eq!(
        service.language_of(Some(Path::new("src/main.rs")), b""),
        Some(language("rust"))
    );
    assert_eq!(
        service.language_of(None, b"#!/usr/bin/env python3\nprint(1)\n"),
        Some(language("python"))
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
