// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Variant C texts of the reference tasks parse to their golden trees.
//!
//! Golden files live in `tests/d1/golden/ast/c`: `<name>.nurt` is the program
//! text, `<name>.json` the expected surface tree without spans and `<name>.hash`
//! its `ast_hash`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{
    AST_HASH_PREFIX, AstDocument, Expr, Head, Pipeline, Span, Statement, strip_for_hash,
};
use nurt_syntax::parse::{ParseOptions, Variant, parse};
use sha2::{Digest, Sha256};

/// Number of reference tasks.
const TASK_COUNT: usize = 14;

fn golden_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d1/golden/ast/c")
}

fn task_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(golden_directory())
        .expect("variant C golden directory exists")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "nurt")
        })
        .map(|path| {
            path.file_stem()
                .expect("file stem")
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with("task-"))
        .collect();
    names.sort();
    names
}

fn read(name: &str, extension: &str) -> String {
    let path = golden_directory().join(format!("{name}.{extension}"));
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn parse_task(name: &str) -> (String, AstDocument) {
    let text = read(name, "nurt");
    let document = parse(&text, Variant::C, &ParseOptions::default())
        .unwrap_or_else(|error| panic!("{name} does not parse: {error:?}"));
    (text, document)
}

fn ast_hash(document: &AstDocument) -> String {
    let value = serde_json::to_value(document).expect("tree serializes");
    let canonical =
        serde_json_canonicalizer::to_string(&strip_for_hash(&value)).expect("tree canonicalizes");
    format!(
        "{AST_HASH_PREFIX}{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    )
}

/// Text covered by a span: lines and columns start at 1, columns count
/// characters and the end is exclusive.
fn covered(text: &str, span: Span) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = String::new();
    for line in span.line..=span.end_line {
        let chars = lines[usize::try_from(line - 1).expect("line index")].chars();
        let from = if line == span.line {
            span.column - 1
        } else {
            0
        };
        let taken: String = chars
            .skip(usize::try_from(from).expect("column index"))
            .take(if line == span.end_line {
                usize::try_from(span.end_column - 1 - from).expect("column count")
            } else {
                usize::MAX
            })
            .collect();
        if line != span.line {
            out.push('\n');
        }
        out.push_str(&taken);
    }
    out
}

fn check_argument_spans(text: &str, pipeline: &Pipeline, name: &str) {
    let head = match &pipeline.head {
        Head::Stage(stage) => Some(stage),
        _ => None,
    };
    for stage in head.into_iter().chain(&pipeline.stages) {
        let stage_text = covered(text, stage.span.expect("stage span"));
        assert!(
            stage_text.starts_with(&stage.name),
            "{name}: {stage_text:?}"
        );
        for argument in &stage.args {
            let parameter = argument.name.as_deref().expect("named argument");
            let argument_text = covered(text, argument.span.expect("argument span"));
            assert!(
                argument_text.starts_with(parameter),
                "{name}: {argument_text:?} does not start with {parameter}"
            );
            assert!(
                stage_text.contains(&argument_text),
                "{name}: {argument_text:?} outside its stage"
            );
            if let Expr::Sub { p, .. } = &argument.value {
                check_argument_spans(text, p, name);
            }
        }
    }
}

#[test]
fn every_reference_task_has_a_golden_text() {
    assert_eq!(task_names().len(), TASK_COUNT);
}

#[test]
fn reference_tasks_parse_to_golden_trees() {
    for name in task_names() {
        let (_, document) = parse_task(&name);
        let expected: serde_json::Value =
            serde_json::from_str(&read(&name, "json")).expect("golden tree is JSON");
        let actual = strip_for_hash(&serde_json::to_value(&document).expect("tree serializes"));
        assert_eq!(actual, strip_for_hash(&expected), "{name}");
    }
}

#[test]
fn reference_tasks_have_recorded_hashes() {
    for name in task_names() {
        let (_, document) = parse_task(&name);
        assert_eq!(
            ast_hash(&document),
            read(&name, "hash").trim_end(),
            "{name}"
        );
    }
}

#[test]
fn argument_spans_start_at_the_parameter_name() {
    for name in task_names() {
        let (text, document) = parse_task(&name);
        for statement in &document.program.statements {
            match statement {
                Statement::Let(binding) => check_argument_spans(&text, &binding.value, &name),
                Statement::Pipeline(pipeline) => check_argument_spans(&text, pipeline, &name),
                other => panic!("unexpected statement {other:?}"),
            }
        }
    }
}
