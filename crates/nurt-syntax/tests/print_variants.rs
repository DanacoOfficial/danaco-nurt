// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Printing of the reference tasks in variants B and C and naming of
//! positional arguments.
//!
//! The expected texts are the variant B and C texts of the reference tasks
//! with the canonical spelling of literals (`ago(1d)` for `ago(24h)`) and the
//! canonical layout: a pipeline whose head has arguments is printed one stage
//! per line, as in variant A.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{
    Arg, AstDocument, BinOp, Expr, Head, KeySeg, Let, MapEntry, Pipeline, Program, Span, Stage,
    Statement, UnOp, strip_for_hash,
};
use nurt_syntax::parse::{self, ParseOptions, Variant};
use nurt_syntax::print::{self, MAX_NESTING_DEPTH, PositionalNames, PrintError};
use nurt_syntax::{fmt, json};

const B_TEXTS: [&str; 14] = [
    "find(\"/srv/app\", glob: \"**/*.go\")\n|> filter(size > 1mb and modified > ago(1d))\n|> sort(size, order: desc)\n|> first(20)\n",
    "search(/oldName\\(/, \"/srv/app/src\", glob: \"**/*.go\")\n|> assert(paths: 12)\n|> replace(\"newName(\")\n",
    "find(\"/srv/app/build\", glob: \"**/*.tmp\")\n|> remove()\n",
    "set(\"/srv/app/package.json\", key: \"scripts.test\", value: \"vitest\")\n",
    "code.symbols(\"oldName\", path: [\"/srv/app/src\"], kind: function)\n|> code.rename(\"newName\")\n",
    "let recent = find(\"/srv/app/src\", glob: \"**/*.ts\")\n|> filter(modified > ago(1h))\n@recent |> count()\n",
    "run([\"go\", \"test\", \"./...\"], cwd: \"/srv/app\")\n|> assert(exit_code: 0)\n",
    "write(\"/srv/app/.gitignore\", \"node_modules/\\n\", mode: append)\n",
    "lines(\"/srv/app/main.go\")\n|> filter(text == \"package main\")\n|> insert(r#\"import \"fmt\"\"#, position: after)\n",
    "find(\"/var/log/app\", glob: \"*.log\", depth: 1)\n|> filter(modified < ago(7d))\n|> move(into: \"/var/log/app/old\")\n",
    "archive.extract(\"/srv/dl/data.tar.gz\", into: \"/srv/data\")\n",
    "search(/oldName\\(/, \"/srv/app/src\", glob: \"**/*.go\")\n|> replace(\"newName(\")\n|> verify(run([\"go\", \"build\", \"./...\"], cwd: \"/srv/app\") |> assert(exit_code: 0))\n",
    "edit(\"/srv/app/main.go\", changes: [\n  {replace: \"oldName(\", with: \"newName(\", occurrence: 1},\n  {insert: \"// updated\", line: 3, position: before},\n  {delete_lines: 10..12}\n])\n",
    "run([\"npm\", \"run\", \"build\"], cwd: \"/srv/app\", background: true)\n",
];

const C_TEXTS: [&str; 14] = [
    "find path=\"/srv/app\" glob=\"**/*.go\"\n|> filter condition=size > 1mb and modified > ago(1d)\n|> sort by=size order=desc\n|> first count=20\n",
    "search pattern=/oldName\\(/ path=\"/srv/app/src\" glob=\"**/*.go\"\n|> assert paths=12\n|> replace with=\"newName(\"\n",
    "find path=\"/srv/app/build\" glob=\"**/*.tmp\"\n|> remove\n",
    "set path=\"/srv/app/package.json\" key=\"scripts.test\" value=\"vitest\"\n",
    "code.symbols name=\"oldName\" path=[\"/srv/app/src\"] kind=function\n|> code.rename to=\"newName\"\n",
    "let recent = find path=\"/srv/app/src\" glob=\"**/*.ts\"\n|> filter condition=modified > ago(1h)\n@recent |> count\n",
    "run command=[\"go\", \"test\", \"./...\"] cwd=\"/srv/app\"\n|> assert exit_code=0\n",
    "write path=\"/srv/app/.gitignore\" text=\"node_modules/\\n\" mode=append\n",
    "lines path=\"/srv/app/main.go\"\n|> filter condition=text == \"package main\"\n|> insert text=r#\"import \"fmt\"\"# position=after\n",
    "find path=\"/var/log/app\" glob=\"*.log\" depth=1\n|> filter condition=modified < ago(7d)\n|> move into=\"/var/log/app/old\"\n",
    "archive.extract path=\"/srv/dl/data.tar.gz\" into=\"/srv/data\"\n",
    "search pattern=/oldName\\(/ path=\"/srv/app/src\" glob=\"**/*.go\"\n|> replace with=\"newName(\"\n|> verify check=(run command=[\"go\", \"build\", \"./...\"] cwd=\"/srv/app\" |> assert exit_code=0)\n",
    "edit path=\"/srv/app/main.go\" changes=[\n  {replace: \"oldName(\", with: \"newName(\", occurrence: 1},\n  {insert: \"// updated\", line: 3, position: before},\n  {delete_lines: 10..12}\n]\n",
    "run command=[\"npm\", \"run\", \"build\"] cwd=\"/srv/app\" background=true\n",
];

/// Parameter names of the positional arguments of the reference tasks.
struct TaskCatalog;

impl PositionalNames for TaskCatalog {
    fn name(&self, command: &str, index: usize) -> Option<String> {
        let name = match (command, index) {
            ("find" | "set" | "write" | "lines" | "archive.extract" | "edit", 0)
            | ("search", 1) => "path",
            ("filter", 0) => "condition",
            ("sort", 0) => "by",
            ("first", 0) => "count",
            ("search", 0) => "pattern",
            ("replace", 0) => "with",
            ("code.symbols", 0) => "name",
            ("code.rename", 0) => "to",
            ("run", 0) => "command",
            ("write", 1) | ("insert", 0) => "text",
            ("verify", 0) => "check",
            _ => return None,
        };
        Some(name.to_owned())
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Golden files of the reference tasks named `task-NN-name<suffix>`, where
/// the name has no `.`, in task order.
fn task_files(suffix: &str) -> Vec<PathBuf> {
    let directory = workspace().join("spec/golden/ast");
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.strip_suffix(suffix)
                .is_some_and(|stem| stem.starts_with("task-") && !stem.contains('.'))
        })
        .collect();
    paths.sort();
    paths
}

fn trees(suffix: &str) -> Vec<AstDocument> {
    task_files(suffix)
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).expect("tree file");
            json::decode_surface(&text)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        })
        .collect()
}

fn a_trees() -> Vec<AstDocument> {
    trees(".json")
}

fn c_trees() -> Vec<AstDocument> {
    trees(".c.json")
}

fn hash_form(document: &AstDocument) -> serde_json::Value {
    strip_for_hash(&serde_json::to_value(document).expect("tree serializes"))
}

fn check_texts(documents: &[AstDocument], variant: Variant, expected: &[&str]) {
    assert_eq!(documents.len(), expected.len());
    let mismatches: Vec<String> = documents
        .iter()
        .zip(expected)
        .enumerate()
        .filter_map(|(index, (document, expected))| {
            let printed = print::try_print(document, variant).expect("printable tree");
            (printed != *expected)
                .then(|| format!("task {}:\n{printed}--- expected:\n{expected}", index + 1))
        })
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn prints_the_reference_tasks_in_variant_b() {
    check_texts(&a_trees(), Variant::B, &B_TEXTS);
}

#[test]
fn prints_the_reference_tasks_in_variant_c() {
    check_texts(&c_trees(), Variant::C, &C_TEXTS);
}

fn check_read_back(documents: &[AstDocument], variant: Variant, texts: &[&str]) {
    assert_eq!(documents.len(), texts.len());
    for (index, (document, text)) in documents.iter().zip(texts).enumerate() {
        let parsed = parse::parse(text, variant, &ParseOptions::default())
            .unwrap_or_else(|error| panic!("task {}: {error}\n{text}", index + 1));
        assert_eq!(
            hash_form(&parsed),
            hash_form(document),
            "task {}:\n{text}",
            index + 1
        );
        assert_eq!(print::print(&parsed, variant), *text, "task {}", index + 1);
    }
}

#[test]
fn variant_b_texts_read_back_as_the_reference_trees() {
    check_read_back(&a_trees(), Variant::B, &B_TEXTS);
}

#[test]
fn variant_c_texts_read_back_as_the_reference_trees() {
    check_read_back(&c_trees(), Variant::C, &C_TEXTS);
}

#[test]
fn formats_the_specification_texts_of_variants_b_and_c() {
    for (variant, suffix, expected) in [
        (Variant::B, ".b.nurt", &B_TEXTS),
        (Variant::C, ".c.nurt", &C_TEXTS),
    ] {
        let paths = task_files(suffix);
        assert_eq!(paths.len(), expected.len(), "{suffix}");
        for (path, expected) in paths.iter().zip(expected) {
            let text = fs::read_to_string(path).expect("program file");
            let formatted = fmt::format(&text, variant)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(formatted, *expected, "{}", path.display());
        }
    }
}

#[test]
fn named_variant_a_trees_print_as_the_variant_c_texts() {
    let named: Vec<AstDocument> = a_trees()
        .iter()
        .map(|document| {
            print::name_positional(document, &TaskCatalog).expect("every argument is named")
        })
        .collect();
    check_texts(&named, Variant::C, &C_TEXTS);
    for (named, expected) in named.iter().zip(c_trees()) {
        assert_eq!(hash_form(named), hash_form(&expected));
    }
}

fn single_stage(args: Vec<Arg>) -> AstDocument {
    AstDocument::new(Program {
        statements: vec![Statement::Pipeline(Pipeline {
            head: Head::Stage(Stage {
                name: "first".to_owned(),
                args,
                span: None,
            }),
            stages: Vec::new(),
            span: None,
        })],
    })
}

fn positional(value: i64, span: Option<Span>) -> Arg {
    Arg {
        name: None,
        value: Expr::Int {
            v: value,
            span: None,
        },
        span,
    }
}

#[test]
fn positional_argument_has_no_variant_c_form() {
    let span = Span {
        line: 1,
        column: 7,
        end_line: 1,
        end_column: 9,
    };
    let document = single_stage(vec![positional(20, Some(span))]);
    assert_eq!(
        print::try_print(&document, Variant::C),
        Err(PrintError::PositionalArgument { span: Some(span) })
    );
    assert_eq!(print::print(&document, Variant::C), "first 20\n");
}

#[test]
fn naming_fails_for_a_positional_argument_without_a_parameter() {
    let document = single_stage(vec![positional(20, None), positional(5, None)]);
    let error =
        print::name_positional(&document, &TaskCatalog).expect_err("second argument has no name");
    assert_eq!(
        error,
        PrintError::UnnamedArgument {
            command: "first".to_owned(),
            index: 1,
            span: None
        }
    );
    assert_eq!(
        error.to_string(),
        "`first` has no name for positional argument 1; write it as `name: value`"
    );
}

#[test]
fn naming_skips_named_arguments_when_counting_positions() {
    let named = Arg {
        name: Some("order".to_owned()),
        value: Expr::Ident {
            v: "desc".to_owned(),
            span: None,
        },
        span: None,
    };
    let document = single_stage(vec![named, positional(20, None)]);
    let named = print::name_positional(&document, &TaskCatalog).expect("named");
    assert_eq!(
        print::try_print(&named, Variant::C),
        Ok("first order=desc count=20\n".to_owned())
    );
}

fn argument(name: Option<&str>, value: Expr) -> Arg {
    Arg {
        name: name.map(str::to_owned),
        value,
        span: None,
    }
}

fn first(count: Option<&str>, value: i64) -> Stage {
    Stage {
        name: "first".to_owned(),
        args: vec![argument(
            count,
            Expr::Int {
                v: value,
                span: None,
            },
        )],
        span: None,
    }
}

/// `let recent = find <path> … |> first <count>`, where the named arguments
/// of `find` hold the sub-pipeline `(first <count>)` in every expression
/// position that can contain one. With `named` the positional arguments carry
/// their parameter names.
fn nested_positions(named: bool) -> AstDocument {
    let path = named.then_some("path");
    let count = named.then_some("count");
    let inner = || Expr::Sub {
        p: Pipeline {
            head: Head::Stage(first(count, 20)),
            stages: Vec::new(),
            span: None,
        },
        span: None,
    };
    let boxed = || Box::new(inner());
    let args = vec![
        argument(
            path,
            Expr::Text {
                v: "/srv".to_owned(),
                span: None,
            },
        ),
        argument(
            Some("map"),
            Expr::Map {
                entries: vec![MapEntry {
                    k: "a".to_owned(),
                    v: inner(),
                    span: None,
                }],
                span: None,
            },
        ),
        argument(
            Some("field"),
            Expr::Field {
                name: "size".to_owned(),
                of: Some(boxed()),
                span: None,
            },
        ),
        argument(
            Some("key"),
            Expr::Key {
                segs: vec![
                    KeySeg::Name("a".to_owned()),
                    KeySeg::Filter { filter: boxed() },
                ],
                span: None,
            },
        ),
        argument(
            Some("call"),
            Expr::List {
                items: vec![Expr::Call {
                    function: "f".to_owned(),
                    args: vec![inner()],
                    span: None,
                }],
                span: None,
            },
        ),
        argument(
            Some("not"),
            Expr::Un {
                op: UnOp::Not,
                x: boxed(),
                span: None,
            },
        ),
        argument(
            Some("bin"),
            Expr::Bin {
                op: BinOp::Eq,
                l: boxed(),
                r: boxed(),
                span: None,
            },
        ),
    ];
    AstDocument::new(Program {
        statements: vec![Statement::Let(Let {
            name: "recent".to_owned(),
            value: Pipeline {
                head: Head::Stage(Stage {
                    name: "find".to_owned(),
                    args,
                    span: None,
                }),
                stages: vec![first(count, 5)],
                span: None,
            },
            span: None,
        })],
    })
}

#[test]
fn naming_reaches_let_values_and_nested_expressions() {
    let named = print::name_positional(&nested_positions(false), &TaskCatalog)
        .expect("every argument is named");
    assert_eq!(named, nested_positions(true));
}

#[test]
fn naming_reports_nesting_beyond_the_limit() {
    let mut value = Expr::Int { v: 1, span: None };
    for _ in 0..=MAX_NESTING_DEPTH {
        value = Expr::Un {
            op: UnOp::Neg,
            x: Box::new(value),
            span: None,
        };
    }
    let document = single_stage(vec![argument(Some("count"), value)]);
    assert_eq!(
        print::name_positional(&document, &TaskCatalog).err(),
        Some(PrintError::TooDeep {
            limit: MAX_NESTING_DEPTH
        })
    );
}
