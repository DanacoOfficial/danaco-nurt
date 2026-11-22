// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Equivalence of the variant A and variant C notations of the reference
//! tasks.
//!
//! The parser does not know the catalog, so a variant A surface tree keeps
//! positional arguments while the variant C tree names every argument. Once
//! the positional arguments of the variant A tree receive their parameter
//! names, both trees must have the same `ast_hash`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::ast::{
    AST_HASH_PREFIX, Arg, AstDocument, Expr, Head, MapEntry, Pipeline, Stage, Statement,
    strip_for_hash,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Positional parameters of the commands used by the reference tasks, in
/// signature order, as declared in `catalog/*.toml`.
///
/// `replace` appears only as a stage on `match` records in the reference
/// tasks; in that form its first positional parameter is the replacement
/// text `with`.
const POSITIONAL_PARAMETERS: &[(&str, &[&str])] = &[
    ("archive.extract", &["path"]),
    ("code.rename", &["to"]),
    ("code.symbols", &["name"]),
    ("edit", &["path"]),
    ("filter", &["condition"]),
    ("find", &["path", "glob"]),
    ("first", &["count"]),
    ("insert", &["text"]),
    ("lines", &["path", "range"]),
    ("move", &["from", "to"]),
    ("remove", &["path"]),
    ("replace", &["with"]),
    ("run", &["command"]),
    ("search", &["pattern", "path"]),
    ("set", &["path"]),
    ("sort", &["by"]),
    ("verify", &["check"]),
    ("write", &["path", "text"]),
];

/// Number of reference tasks.
const TASK_COUNT: usize = 14;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root exists")
}

/// Names of the reference task files, `task-NN-<slug>`, in task order.
fn task_names() -> Vec<String> {
    let directory = workspace_root().join("tests/d1/golden/ast/c");
    let mut names: Vec<String> = fs::read_dir(&directory)
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

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> T {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("cannot decode {}: {error}", path.display()))
}

fn read_hash(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
        .trim_end()
        .to_owned()
}

/// `ast_hash` of a tree document: SHA-256 of the JCS form of the document
/// without `meta` and spans.
fn ast_hash<T: Serialize>(document: &T) -> String {
    let value = serde_json::to_value(document).expect("tree serializes");
    let canonical =
        serde_json_canonicalizer::to_string(&strip_for_hash(&value)).expect("tree canonicalizes");
    format!(
        "{AST_HASH_PREFIX}{}",
        hex::encode(Sha256::digest(canonical.as_bytes()))
    )
}

fn positional_parameters(command: &str) -> &'static [&'static str] {
    POSITIONAL_PARAMETERS
        .iter()
        .find(|(name, _)| *name == command)
        .map_or(&[], |(_, parameters)| parameters)
}

fn name_document(document: &mut AstDocument) {
    for statement in &mut document.program.statements {
        match statement {
            Statement::Let(binding) => name_pipeline(&mut binding.value),
            Statement::Pipeline(pipeline) => name_pipeline(pipeline),
            other => panic!("unexpected statement {other:?}"),
        }
    }
}

fn name_pipeline(pipeline: &mut Pipeline) {
    if let Head::Stage(stage) = &mut pipeline.head {
        name_stage(stage);
    }
    for stage in &mut pipeline.stages {
        name_stage(stage);
    }
}

/// Gives the k-th positional argument of a stage the name of the k-th
/// positional parameter of its command, keeping the written order.
fn name_stage(stage: &mut Stage) {
    let parameters = positional_parameters(&stage.name);
    let mut position = 0;
    for argument in &mut stage.args {
        name_expr(&mut argument.value);
        if argument.name.is_none() {
            let name = parameters.get(position).unwrap_or_else(|| {
                panic!(
                    "no positional parameter {} for `{}`",
                    position + 1,
                    stage.name
                )
            });
            argument.name = Some((*name).to_owned());
            position += 1;
        }
    }
}

fn name_expr(expr: &mut Expr) {
    match expr {
        Expr::Sub { p, .. } => name_pipeline(p),
        Expr::List { items, .. } => items.iter_mut().for_each(name_expr),
        Expr::Map { entries, .. } => entries
            .iter_mut()
            .for_each(|MapEntry { v, .. }| name_expr(v)),
        Expr::Spread { x, .. } | Expr::Un { x, .. } => name_expr(x),
        Expr::Field { of: Some(of), .. } => name_expr(of),
        Expr::Call { args, .. } => args.iter_mut().for_each(name_expr),
        Expr::Bin { l, r, .. } => {
            name_expr(l);
            name_expr(r);
        }
        _ => {}
    }
}

fn stage_args(document: &AstDocument) -> Vec<&Arg> {
    fn collect<'a>(pipeline: &'a Pipeline, out: &mut Vec<&'a Arg>) {
        let head = match &pipeline.head {
            Head::Stage(stage) => Some(stage),
            _ => None,
        };
        for stage in head.into_iter().chain(&pipeline.stages) {
            for argument in &stage.args {
                out.push(argument);
                if let Expr::Sub { p, .. } = &argument.value {
                    collect(p, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for statement in &document.program.statements {
        match statement {
            Statement::Let(binding) => collect(&binding.value, &mut out),
            Statement::Pipeline(pipeline) => collect(pipeline, &mut out),
            other => panic!("unexpected statement {other:?}"),
        }
    }
    out
}

#[test]
fn every_reference_task_has_a_variant_c_golden_tree() {
    assert_eq!(task_names().len(), TASK_COUNT);
}

#[test]
fn variant_c_golden_trees_name_every_argument() {
    let root = workspace_root();
    for name in task_names() {
        let document: AstDocument =
            read_json(&root.join(format!("tests/d1/golden/ast/c/{name}.json")));
        for argument in stage_args(&document) {
            assert!(
                argument.name.is_some(),
                "{name}: positional argument {:?}",
                argument.value
            );
        }
    }
}

#[test]
fn variant_c_golden_trees_match_recorded_hashes() {
    let root = workspace_root();
    for name in task_names() {
        let document: AstDocument =
            read_json(&root.join(format!("tests/d1/golden/ast/c/{name}.json")));
        let recorded = read_hash(&root.join(format!("tests/d1/golden/ast/c/{name}.hash")));
        assert_eq!(ast_hash(&document), recorded, "{name}");
    }
}

#[test]
fn named_variant_a_trees_equal_variant_c_trees() {
    let root = workspace_root();
    for name in task_names() {
        let mut variant_a: AstDocument =
            read_json(&root.join(format!("spec/golden/ast/{name}.json")));
        name_document(&mut variant_a);
        let variant_c: AstDocument =
            read_json(&root.join(format!("tests/d1/golden/ast/c/{name}.json")));
        assert_eq!(ast_hash(&variant_a), ast_hash(&variant_c), "{name}");
    }
}

#[test]
fn naming_changes_the_hash_of_trees_with_positional_arguments() {
    let root = workspace_root();
    let name = "task-03-remove-temporary-files";
    let variant_a: AstDocument = read_json(&root.join(format!("spec/golden/ast/{name}.json")));
    let mut named = variant_a.clone();
    name_document(&mut named);
    assert_ne!(ast_hash(&variant_a), ast_hash(&named));
}
