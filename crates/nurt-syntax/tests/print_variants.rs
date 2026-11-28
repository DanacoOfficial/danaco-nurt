// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/print_variants.rs
// Responsibility:   Printing of the reference tasks in variants B and C and naming of positional arguments.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
use nurt_syntax::print::{self, MAX_NESTING_DEPTH, PositionalForm, PositionalNames, PrintError};
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
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../..")
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
fn print_errors_report_the_span_of_the_offending_node() {
    let span = Span {
        line: 1,
        column: 7,
        end_line: 1,
        end_column: 9,
    };
    let errors = [
        PrintError::PositionalArgument { span: Some(span) },
        PrintError::UnnamedArgument {
            command: "first".to_owned(),
            index: 0,
            span: Some(span),
        },
        PrintError::Unprintable {
            node: "ident",
            problem: "is not an identifier",
            span: Some(span),
        },
    ];
    for error in errors {
        assert_eq!(error.span(), Some(span), "{error}");
    }
    let too_deep = PrintError::TooDeep {
        limit: MAX_NESTING_DEPTH,
    };
    assert_eq!(too_deep.span(), None);
    assert_eq!(
        too_deep.to_string(),
        "the tree is nested deeper than 128 levels; split the program"
    );
    assert_eq!(
        PrintError::Unprintable {
            node: "ident",
            problem: "is not an identifier",
            span: None,
        }
        .to_string(),
        "`ident` node is not an identifier"
    );
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

/// Positional forms of the commands of the golden programs, transcribed from
/// `catalog/*.toml` in catalog order: command, whether the form applies to the
/// first stage of a pipeline (input `none`), whether it applies after `|>`
/// (any other input kind), positional parameters and the index of the
/// repeated parameter. A command without forms has one form made of its
/// parameters with a position.
type FormRow = (
    &'static str,
    bool,
    bool,
    &'static [&'static str],
    Option<usize>,
);

const CATALOG_FORMS: &[FormRow] = &[
    ("archive.create", false, true, &["to"], None),
    ("archive.extract", true, false, &["path"], None),
    ("code.references", true, false, &["path"], None),
    ("code.references", false, true, &[], None),
    ("code.rename", false, true, &["to"], None),
    ("code.rename", true, false, &["to"], None),
    ("code.symbols", true, false, &["name"], None),
    ("copy", true, false, &["from", "to"], Some(0)),
    ("copy", false, true, &["to"], None),
    ("create", true, false, &["path"], Some(0)),
    ("edit", true, false, &["path"], None),
    ("filter", false, true, &["condition"], None),
    ("find", true, false, &["path", "glob"], None),
    ("first", false, true, &["count"], None),
    ("git.commit", true, false, &["repo"], None),
    ("git.status", true, false, &["repo"], Some(0)),
    ("http.download", true, false, &["url", "to"], None),
    ("http.request", true, false, &["url"], None),
    ("info", true, false, &["path"], Some(0)),
    ("insert", true, false, &["text"], None),
    ("insert", false, true, &["text"], None),
    ("jobs", true, false, &["id"], None),
    ("lines", true, false, &["path", "range"], None),
    ("lines", false, true, &["range"], None),
    ("move", true, false, &["from", "to"], Some(0)),
    ("move", false, true, &["to"], None),
    ("notebook.replace", true, false, &["path"], None),
    ("program.search", true, false, &["query"], None),
    ("read", true, false, &["path"], Some(0)),
    ("read", false, true, &[], None),
    ("remove", true, false, &["path"], Some(0)),
    ("remove", false, true, &[], None),
    ("replace", true, false, &["pattern", "with"], None),
    ("replace", false, true, &["with"], None),
    ("replace", false, true, &["pattern", "with"], None),
    ("run", true, true, &["command"], None),
    ("schedule.create", true, false, &["label"], None),
    ("search", true, false, &["pattern", "path"], Some(1)),
    ("search", false, true, &["pattern"], None),
    ("select", false, true, &["fields"], Some(0)),
    ("set", true, false, &["path"], None),
    ("set", false, true, &["value"], None),
    ("sort", false, true, &["by"], Some(0)),
    ("sql", true, false, &["path", "query"], None),
    ("test.run", true, false, &["path"], None),
    ("undo", true, false, &["id"], None),
    ("verify", false, true, &["check"], None),
    ("write", true, true, &["path", "text"], None),
];

/// Number of reference tasks.
const TASK_COUNT: usize = 14;
/// Number of examples.
const EXAMPLE_COUNT: usize = 48;

/// Parameter names from the forms of [`CATALOG_FORMS`].
struct CatalogForms;

impl PositionalNames for CatalogForms {
    fn name(&self, command: &str, index: usize) -> Option<String> {
        self.forms(command).first()?.parameters.get(index).cloned()
    }

    fn forms(&self, command: &str) -> Vec<PositionalForm> {
        CATALOG_FORMS
            .iter()
            .filter(|(name, ..)| *name == command)
            .map(
                |&(_, head, follower, parameters, repeated)| PositionalForm {
                    parameters: parameters.iter().map(|&name| name.to_owned()).collect(),
                    repeated,
                    head,
                    follower,
                },
            )
            .collect()
    }
}

fn variant_c_golden_dir() -> PathBuf {
    workspace().join("tests/d1/golden/ast/c")
}

/// Reference tasks and examples with a variant C golden tree, sorted. The
/// trees are the variant A trees named by the catalog table of the variant C
/// equivalence test, so equal trees show that the forms name every program
/// as that table does.
fn variant_c_programs() -> Vec<String> {
    let directory = variant_c_golden_dir();
    let mut names: Vec<String> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .filter_map(|entry| {
            let path = entry.expect("directory entry").path();
            let name = path.file_name()?.to_str()?.strip_suffix(".json")?;
            (!name.contains('.')).then(|| name.to_owned())
        })
        .collect();
    names.sort();
    names
}

fn read_tree(path: &Path) -> AstDocument {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    json::decode_surface(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn named_by_catalog_forms(name: &str) -> AstDocument {
    let variant_a = read_tree(&workspace().join(format!("spec/golden/ast/{name}.json")));
    print::name_positional(&variant_a, &CatalogForms)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

#[test]
fn catalog_forms_name_the_golden_programs_as_their_variant_c_trees() {
    let programs = variant_c_programs();
    let tasks = programs
        .iter()
        .filter(|name| name.starts_with("task-"))
        .count();
    assert_eq!((tasks, programs.len() - tasks), (TASK_COUNT, EXAMPLE_COUNT));
    let mismatches: Vec<String> = programs
        .iter()
        .filter_map(|name| {
            let named = named_by_catalog_forms(name);
            let expected = read_tree(&variant_c_golden_dir().join(format!("{name}.json")));
            (hash_form(&named) != hash_form(&expected))
                .then(|| format!("{name}: {}", print::print(&named, Variant::C)))
        })
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn catalog_forms_print_the_examples_as_their_variant_c_texts() {
    for name in variant_c_programs()
        .iter()
        .filter(|name| name.starts_with("example-"))
    {
        let path = variant_c_golden_dir().join(format!("{name}.nurt"));
        let expected = fs::read_to_string(&path).expect("variant C text");
        let printed = print::try_print(&named_by_catalog_forms(name), Variant::C)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(printed, expected, "{name}");
    }
}

fn catalog_variant_c(text: &str) -> Result<String, PrintError> {
    let document =
        parse::parse(text, Variant::A, &ParseOptions::default()).expect("variant A text parses");
    print::name_positional(&document, &CatalogForms)
        .and_then(|named| print::try_print(&named, Variant::C))
}

#[test]
fn argument_count_selects_the_form_of_a_command() {
    for (text, expected) in [
        (
            "search /old/ \"/src\"\n|> replace \"new\"\n",
            "search pattern=/old/ path=\"/src\"\n|> replace with=\"new\"\n",
        ),
        (
            "lines \"/f\"\n|> replace \"old\" \"new\"\n",
            "lines path=\"/f\"\n|> replace pattern=\"old\" with=\"new\"\n",
        ),
        (
            "replace \"old\" \"new\" in: [\"/src\"]\n",
            "replace pattern=\"old\" with=\"new\" in=[\"/src\"]\n",
        ),
    ] {
        assert_eq!(catalog_variant_c(text).as_deref(), Ok(expected), "{text}");
    }
}

#[test]
fn place_of_the_stage_selects_the_form_of_a_command() {
    assert_eq!(
        catalog_variant_c("lines \"/f\"\n").as_deref(),
        Ok("lines path=\"/f\"\n")
    );
    assert_eq!(
        catalog_variant_c("find \"/srv\"\n|> lines 1..3\n").as_deref(),
        Ok("find path=\"/srv\"\n|> lines range=1..3\n")
    );
}

#[test]
fn surplus_arguments_fill_the_repeated_parameter() {
    for (text, expected) in [
        (
            "find \"/srv\"\n|> select pid name memory\n",
            "find path=\"/srv\"\n|> select fields=[pid, name, memory]\n",
        ),
        (
            "copy \"/a\" overwrite: true \"/b\" \"/dst\"\n",
            "copy from=[\"/a\", \"/b\"] overwrite=true to=\"/dst\"\n",
        ),
        ("copy \"/a\" \"/dst\"\n", "copy from=\"/a\" to=\"/dst\"\n"),
        ("copy \"/a\"\n", "copy from=\"/a\"\n"),
    ] {
        assert_eq!(catalog_variant_c(text).as_deref(), Ok(expected), "{text}");
    }
}

fn head_stage(document: &AstDocument) -> &Stage {
    match document.program.statements.first() {
        Some(Statement::Pipeline(Pipeline {
            head: Head::Stage(stage),
            ..
        })) => stage,
        other => panic!("no head stage in {other:?}"),
    }
}

#[test]
fn list_of_surplus_arguments_spans_its_items() {
    let document = parse::parse(
        "copy \"/a\" \"/b\" \"/dst\"\n",
        Variant::A,
        &ParseOptions::default(),
    )
    .expect("variant A text parses");
    let spans: Vec<Span> = head_stage(&document)
        .args
        .iter()
        .map(|arg| arg.span.expect("parsed argument has a span"))
        .collect();
    let expected = Span {
        line: spans[0].line,
        column: spans[0].column,
        end_line: spans[1].end_line,
        end_column: spans[1].end_column,
    };
    let named = print::name_positional(&document, &CatalogForms).expect("named");
    let list = &head_stage(&named).args[0];
    assert_eq!(list.span, Some(expected));
    assert!(
        matches!(list.value, Expr::List { span: Some(span), .. } if span == expected),
        "{list:?}"
    );
    assert_eq!(head_stage(&named).args[1].span, Some(spans[2]));
}

#[test]
fn surplus_arguments_without_a_repeated_parameter_have_no_name() {
    let error = catalog_variant_c("find \"/srv\" \"*.go\" 2\n").expect_err("third argument");
    assert!(
        matches!(
            &error,
            PrintError::UnnamedArgument { command, index: 2, span: Some(_) } if command == "find"
        ),
        "{error:?}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
