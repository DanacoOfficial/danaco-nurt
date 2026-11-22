// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of local analysis (definitions, references, roles, calls, test
//! files) on samples of Python, TypeScript, Go, Rust, Java and C#, and of
//! the syntax check.

// Tests report failures by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u11_support;

use std::path::Path;

use nurt_code::local::{
    LocalAnalysis, Occurrence, OccurrenceKind, analyze, is_test_path, offset_of, references,
};
use nurt_code::registry::Registry;
use nurt_code::syntax_check::{DIAGNOSTIC_SOURCE, MAX_ISSUES, check_syntax, diagnostic_record};
use nurt_contracts::record::DiagnosticSeverity;
use nurt_contracts::services::{IssueSeverity, ReferenceRole, SpanClass, TextPosition};

use u11_support::language;

fn analysis<'a>(id: &str, source: &'a str) -> LocalAnalysis<'a> {
    analyze(Registry::builtin(), source.as_bytes(), &language(id)).unwrap()
}

fn at<'a>(analysis: &'a LocalAnalysis<'_>, source: &str, line: u32, column: u32) -> &'a Occurrence {
    let offset = offset_of(source.as_bytes(), TextPosition { line, column }).unwrap();
    analysis
        .occurrence_at(u64::try_from(offset).unwrap())
        .unwrap_or_else(|| panic!("no identifier at {line}:{column}"))
}

fn definition_line(analysis: &LocalAnalysis<'_>, occurrence: &Occurrence) -> u32 {
    analysis
        .definition_of(occurrence)
        .unwrap_or_else(|| panic!("`{}` is not bound", occurrence.name))
        .span
        .start
        .line
}

fn roles(id: &str, source: &str, name: &str) -> Vec<ReferenceRole> {
    references(Registry::builtin(), source.as_bytes(), &language(id), name)
        .unwrap()
        .iter()
        .filter(|found| found.class == SpanClass::Code)
        .map(|found| found.role)
        .collect()
}

const PYTHON: &str = "import os

value = 1


def shadow(value):
    return value + 1


def area(r):
    return 3 * r * r


class Counter:
    def bump(self, step):
        return self.helper(step)

    def helper(self, step):
        return area(step) + len(os.sep)
";

#[test]
fn python_parameters_shadow_module_variables() {
    let analysis = analysis("python", PYTHON);
    let use_of_value = at(&analysis, PYTHON, 7, 12);
    assert_eq!(use_of_value.name, "value");
    assert_eq!(use_of_value.kind, OccurrenceKind::Use);
    assert_eq!(definition_line(&analysis, use_of_value), 6);
    let parameter = at(&analysis, PYTHON, 10, 10);
    assert_eq!(parameter.kind, OccurrenceKind::Local);
    assert_eq!(analysis.bound_to(parameter).len(), 3);
}

#[test]
fn python_imports_and_calls_are_recognized() {
    let analysis = analysis("python", PYTHON);
    let import = at(&analysis, PYTHON, 1, 8);
    assert_eq!(import.kind, OccurrenceKind::Import);
    assert_eq!(import.role(), ReferenceRole::Import);
    assert_eq!(
        roles("python", PYTHON, "area"),
        [ReferenceRole::Definition, ReferenceRole::Usage]
    );
    let helper = analysis
        .symbols()
        .iter()
        .position(|symbol| symbol.name == "helper")
        .unwrap();
    let callees: Vec<&str> = analysis
        .outgoing(helper)
        .iter()
        .map(|call| call.callee.as_str())
        .collect();
    assert!(callees.contains(&"area"), "{callees:?}");
    let callers: Vec<Option<usize>> = analysis
        .incoming("helper")
        .iter()
        .map(|call| call.caller)
        .collect();
    let bump = analysis
        .symbols()
        .iter()
        .position(|symbol| symbol.name == "bump");
    assert_eq!(callers, [bump]);
    let container = analysis.container_at(u64::try_from(PYTHON.find("len(").unwrap()).unwrap());
    assert_eq!(container.as_deref(), Some("Counter.helper"));
}

#[test]
fn python_comments_and_strings_are_classified() {
    let source = "# area of a circle\ndef area(r):\n    return \"area\"\n";
    let found = references(
        Registry::builtin(),
        source.as_bytes(),
        &language("python"),
        "area",
    )
    .unwrap();
    let classes: Vec<SpanClass> = found.iter().map(|item| item.class).collect();
    assert_eq!(
        classes,
        [SpanClass::Comment, SpanClass::Code, SpanClass::String]
    );
    assert_eq!(found[1].role, ReferenceRole::Definition);
    assert_eq!(found[1].line_text, "def area(r):");
}

const TYPESCRIPT: &str = "import { readFile } from \"fs\";

const value = 1;

function shadow(value: number): number {
  return value + 1;
}

export function main(): void {
  const r = shadow(2);
  readFile(\"x\", () => console.log(r, value));
}
";

#[test]
fn typescript_bindings_follow_scopes() {
    let analysis = analysis("typescript", TYPESCRIPT);
    let import = at(&analysis, TYPESCRIPT, 1, 10);
    assert_eq!(import.kind, OccurrenceKind::Import);
    let use_of_import = at(&analysis, TYPESCRIPT, 11, 3);
    assert_eq!(definition_line(&analysis, use_of_import), 1);
    let shadowed = at(&analysis, TYPESCRIPT, 6, 10);
    assert_eq!(definition_line(&analysis, shadowed), 5);
    let module_value = at(&analysis, TYPESCRIPT, 11, 38);
    assert_eq!(module_value.name, "value");
    assert_eq!(definition_line(&analysis, module_value), 3);
    let local = at(&analysis, TYPESCRIPT, 11, 35);
    assert_eq!(definition_line(&analysis, local), 10);
    assert_eq!(
        roles("typescript", TYPESCRIPT, "readFile"),
        [ReferenceRole::Import, ReferenceRole::Usage]
    );
}

const GO: &str = "package main

import \"fmt\"

func area(r int) int {
\treturn 3 * r * r
}

func main() {
\tr := area(2)
\tfmt.Println(r)
}
";

#[test]
fn go_short_declarations_define_locals() {
    let analysis = analysis("go", GO);
    let declared = at(&analysis, GO, 10, 2);
    assert_eq!(declared.kind, OccurrenceKind::Local);
    let used = at(&analysis, GO, 11, 14);
    assert_eq!(definition_line(&analysis, used), 10);
    let parameter_use = at(&analysis, GO, 6, 13);
    assert_eq!(definition_line(&analysis, parameter_use), 5);
    let found = references(Registry::builtin(), GO.as_bytes(), &language("go"), "area").unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].role, ReferenceRole::Definition);
    assert_eq!(found[1].container.as_deref(), Some("main"));
    let main = analysis
        .symbols()
        .iter()
        .position(|symbol| symbol.name == "main")
        .unwrap();
    let callees: Vec<&str> = analysis
        .outgoing(main)
        .iter()
        .map(|call| call.callee.as_str())
        .collect();
    assert!(callees.contains(&"area"), "{callees:?}");
}

const RUST: &str = "use std::fmt::Display;

fn shadow(value: u32) -> u32 {
    let value = value + 1;
    value
}

fn show<T: Display>(item: T) -> String {
    format!(\"{}\", item)
}
";

#[test]
fn rust_let_bindings_shadow_parameters() {
    let analysis = analysis("rust", RUST);
    let parameter = at(&analysis, RUST, 3, 11);
    assert_eq!(parameter.kind, OccurrenceKind::Local);
    let rebound = at(&analysis, RUST, 4, 9);
    assert_eq!(rebound.kind, OccurrenceKind::Local);
    let last = at(&analysis, RUST, 5, 5);
    assert_eq!(definition_line(&analysis, last), 4);
    let import = at(&analysis, RUST, 1, 15);
    assert_eq!(import.name, "Display");
    assert_eq!(import.role(), ReferenceRole::Import);
    let item = at(&analysis, RUST, 9, 19);
    assert_eq!(definition_line(&analysis, item), 8);
}

const JAVA: &str = "public class Geometry {
    private int value = 1;

    static int area(int r) {
        return 3 * r * r;
    }

    int shadow(int value) {
        return value + area(value);
    }
}
";

#[test]
fn java_parameters_and_methods_resolve() {
    let analysis = analysis("java", JAVA);
    let use_of_r = at(&analysis, JAVA, 5, 20);
    assert_eq!(definition_line(&analysis, use_of_r), 4);
    let use_of_value = at(&analysis, JAVA, 9, 16);
    assert_eq!(definition_line(&analysis, use_of_value), 8);
    let found = references(
        Registry::builtin(),
        JAVA.as_bytes(),
        &language("java"),
        "area",
    )
    .unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].role, ReferenceRole::Definition);
    assert_eq!(found[1].role, ReferenceRole::Usage);
    assert_eq!(found[1].container.as_deref(), Some("Geometry.shadow"));
}

const CSHARP: &str = "using System;

public class Counter
{
    private int value;

    public int Bump(int step)
    {
        var total = value + step;
        return Twice(total);
    }

    private int Twice(int step) => step * 2;
}
";

#[test]
fn csharp_locals_and_calls_resolve() {
    let analysis = analysis("csharp", CSHARP);
    let total = at(&analysis, CSHARP, 10, 22);
    assert_eq!(definition_line(&analysis, total), 9);
    let step = at(&analysis, CSHARP, 9, 29);
    assert_eq!(definition_line(&analysis, step), 7);
    let twice_step = at(&analysis, CSHARP, 13, 36);
    assert_eq!(definition_line(&analysis, twice_step), 13);
    assert_eq!(
        roles("csharp", CSHARP, "Twice"),
        [ReferenceRole::Usage, ReferenceRole::Definition]
    );
    let bump = analysis
        .symbols()
        .iter()
        .position(|symbol| symbol.name == "Bump")
        .unwrap();
    let callees: Vec<&str> = analysis
        .outgoing(bump)
        .iter()
        .map(|call| call.callee.as_str())
        .collect();
    assert_eq!(callees, ["Twice"]);
}

#[test]
fn positions_and_offsets_round_trip() {
    let analysis = analysis("python", PYTHON);
    for (line, column) in [(1, 1), (6, 5), (16, 16)] {
        let offset = offset_of(PYTHON.as_bytes(), TextPosition { line, column }).unwrap();
        let position = analysis.position(u64::try_from(offset).unwrap());
        assert_eq!((position.line, position.column), (line, column));
    }
    assert_eq!(analysis.line_text(0), "import os");
}

#[test]
fn test_files_follow_language_conventions() {
    let cases = [
        ("/p/test_api.py", "python", true),
        ("/p/api.py", "python", false),
        ("/p/user.spec.ts", "typescript", true),
        ("/p/user.ts", "typescript", false),
        ("/p/server_test.go", "go", true),
        ("/p/server.go", "go", false),
        ("/p/crate/tests/parse.rs", "rust", true),
        ("/p/crate/src/parse.rs", "rust", false),
        ("/p/src/UserServiceTest.java", "java", true),
        ("/p/src/UserService.java", "java", false),
        ("/p/Shop.Tests/Cart.cs", "csharp", true),
        ("/p/Shop/CartTests.cs", "csharp", true),
        ("/p/Shop/Cart.cs", "csharp", false),
    ];
    for (path, id, expected) in cases {
        assert_eq!(
            is_test_path(Path::new(path), Some(&language(id))),
            expected,
            "{path}"
        );
    }
}

#[test]
fn syntax_errors_become_diagnostics() {
    let broken = [
        ("python", "def broken(:\n    pass\n"),
        ("typescript", "function broken( {\n"),
        ("go", "package main\n\nfunc main( {\n}\n"),
        ("rust", "fn main() {\n    let x = ;\n}\n"),
        ("java", "class A { void f( { } }\n"),
        ("csharp", "class A { void F( { } }\n"),
        ("json", "{\"a\": }\n"),
    ];
    for (id, source) in broken {
        let issues = check_syntax(Registry::builtin(), source.as_bytes(), &language(id)).unwrap();
        assert!(!issues.is_empty(), "{id}: no issue in {source:?}");
        assert!(
            issues
                .iter()
                .all(|issue| issue.severity == IssueSeverity::Error)
        );
        let path = Path::new("/p/broken.txt");
        let record = diagnostic_record(&issues[0], path, &language(id));
        assert_eq!(record.source, DIAGNOSTIC_SOURCE);
        assert_eq!(record.severity, DiagnosticSeverity::Error);
        assert_eq!(record.path.as_str(), "/p/broken.txt");
        assert_eq!(record.language.as_deref(), Some(id));
        assert!(record.line >= 1 && record.column >= 1);
    }
}

#[test]
fn missing_tokens_are_reported() {
    let issues = check_syntax(
        Registry::builtin(),
        b"fn main() {\n    let x = 1\n}\n",
        &language("rust"),
    )
    .unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].message.contains(';'), "{}", issues[0].message);
    assert_eq!(issues[0].span.start.line, 2);
}

#[test]
fn valid_sources_have_no_syntax_issues() {
    for (id, source) in [
        ("python", PYTHON),
        ("typescript", TYPESCRIPT),
        ("go", GO),
        ("rust", RUST),
        ("java", JAVA),
        ("csharp", CSHARP),
    ] {
        let issues = check_syntax(Registry::builtin(), source.as_bytes(), &language(id)).unwrap();
        assert!(issues.is_empty(), "{id}: {issues:?}");
    }
}

#[test]
fn syntax_issues_are_capped() {
    let source = "fn f( {\n".repeat(MAX_ISSUES * 2);
    let issues = check_syntax(Registry::builtin(), source.as_bytes(), &language("rust")).unwrap();
    assert!(!issues.is_empty());
    assert!(issues.len() <= MAX_ISSUES);
}
