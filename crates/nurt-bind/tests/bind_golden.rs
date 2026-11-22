// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden bound trees of the conformance programs of `spec/golden/ast`.
//!
//! Every surface tree is bound with the reference catalog and the embedded
//! correction tables. A tree that binds is compared with
//! `tests/d1/golden/bound/<variant>/<name>.json` and its hash with
//! `<name>.hash`; a tree that fails is listed with its error code in
//! `tests/d1/bind/golden-errors.tsv`. With `NURT_REGENERATE_GOLDEN=1` the
//! test writes these files instead of comparing them.

// Test failures report through panics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bind_support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use bind_support::{bind_with, corr_rev, hash, workspace};
use nurt_bind::BindContext;
use nurt_contracts::ast::AstDocument;
use nurt_contracts::spec::Os;
use nurt_syntax::validate::validate_bound;
use serde_json::Value;

/// Environment variable that switches the test to writing golden files.
const REGENERATE: &str = "NURT_REGENERATE_GOLDEN";

/// Programs written for a Windows host; the other programs bind on Linux.
const WINDOWS_PROGRAMS: [&str; 2] = [
    "example-15-tree-transport",
    "example-16-legacy-console-encoding",
];

/// Word in the name of a program written for a Windows host.
const WINDOWS_MARK: &str = "windows";

/// Index of the conformance programs that do not bind, relative to the
/// workspace.
const ERRORS_FILE: &str = "tests/d1/bind/golden-errors.tsv";

/// Header of the index of failing programs.
const ERRORS_HEADER: &str = "variant\tname\tcode";

/// Number of conformance programs in variant A: 14 tasks and 48 examples.
const MIN_PROGRAMS: usize = 62;

/// Suffix of the surface tree of a program written in variant C.
const VARIANT_C_SUFFIX: &str = ".c";

/// File of the published schema among the bound trees of the specification.
const SCHEMA_FILE: &str = "schema.generated.json";

struct Program {
    variant: &'static str,
    name: String,
    document: AstDocument,
}

fn regenerate() -> bool {
    std::env::var(REGENERATE).is_ok_and(|value| value == "1")
}

fn spec_golden() -> PathBuf {
    workspace().join("spec").join("golden")
}

fn bound_golden(variant: &str) -> PathBuf {
    workspace().join("tests/d1/golden/bound").join(variant)
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
                && path.file_name().is_some_and(|name| name != SCHEMA_FILE)
        })
        .collect();
    files.sort();
    files
}

fn programs() -> Vec<Program> {
    json_files(&spec_golden().join("ast"))
        .into_iter()
        .map(|path| {
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = fs::read_to_string(&path).unwrap();
            let document: AstDocument = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            match stem.strip_suffix(VARIANT_C_SUFFIX) {
                Some(name) => Program {
                    variant: "c",
                    name: name.to_owned(),
                    document,
                },
                None => Program {
                    variant: "a",
                    name: stem,
                    document,
                },
            }
        })
        .collect()
}

fn context(name: &str) -> BindContext {
    let windows = WINDOWS_PROGRAMS.contains(&name) || name.contains(WINDOWS_MARK);
    let os = if windows { Os::Windows } else { Os::Linux };
    BindContext::new(os).with_corr_rev(corr_rev())
}

fn read_or_empty(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

/// Compares or writes one golden file; returns the problem found.
fn golden_file(path: &Path, content: &str, write: bool) -> Option<String> {
    if write {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        return None;
    }
    (read_or_empty(path) != content)
        .then(|| format!("{} differs from the bound tree", path.display()))
}

#[test]
fn bound_trees_match_the_golden_files() {
    let write = regenerate();
    let mut problems = Vec::new();
    let mut failures = BTreeSet::new();
    for program in programs() {
        match bind_with(&program.document, &context(&program.name)) {
            Ok(bound) => {
                if let Err(error) = validate_bound(&bound) {
                    problems.push(format!("{} {}: {error:?}", program.variant, program.name));
                }
                let dir = bound_golden(program.variant);
                let json = format!("{}\n", serde_json::to_string_pretty(&bound).unwrap());
                let digest = format!("{}\n", hash(&bound));
                problems.extend(golden_file(
                    &dir.join(format!("{}.json", program.name)),
                    &json,
                    write,
                ));
                problems.extend(golden_file(
                    &dir.join(format!("{}.hash", program.name)),
                    &digest,
                    write,
                ));
            }
            Err(error) => {
                failures.insert(format!(
                    "{}\t{}\t{}",
                    program.variant,
                    program.name,
                    error.code()
                ));
            }
        }
    }
    let mut index = format!("{ERRORS_HEADER}\n");
    for line in &failures {
        index.push_str(line);
        index.push('\n');
    }
    problems.extend(golden_file(&workspace().join(ERRORS_FILE), &index, write));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_conformance_program_is_bound() {
    let count = programs()
        .iter()
        .filter(|program| program.variant == "a")
        .count();
    assert!(
        count >= MIN_PROGRAMS,
        "{count} programs, {MIN_PROGRAMS} expected"
    );
}

#[test]
fn variants_a_and_c_bind_to_one_tree() {
    let all = programs();
    let mut problems = Vec::new();
    let mut compared = 0;
    for program in all.iter().filter(|program| program.variant == "c") {
        let Some(word) = all
            .iter()
            .find(|other| other.variant == "a" && other.name == program.name)
        else {
            problems.push(format!("{}: no program in variant A", program.name));
            continue;
        };
        let context = context(&program.name);
        match (
            bind_with(&word.document, &context),
            bind_with(&program.document, &context),
        ) {
            (Ok(a), Ok(c)) if hash(&a) == hash(&c) => compared += 1,
            (Ok(_), Ok(_)) => problems.push(format!("{}: the bound trees differ", program.name)),
            (a, c) => problems.push(format!(
                "{}: variant A {:?}, variant C {:?}",
                program.name,
                a.err().map(|error| error.code()),
                c.err().map(|error| error.code())
            )),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(compared > 0, "no program in variant C");
}

#[test]
fn agrees_with_the_bound_trees_of_the_specification() {
    let mut problems = Vec::new();
    for path in json_files(&spec_golden().join("bound")) {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let name = file.trim_end_matches(".json");
        let surface = spec_golden().join("ast").join(&file);
        let Ok(text) = fs::read_to_string(&surface) else {
            continue;
        };
        let document: AstDocument = serde_json::from_str(&text).unwrap();
        let reference: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        match bind_with(&document, &context(name)) {
            Ok(bound) => {
                let mine = serde_json::to_value(&bound).unwrap();
                compare(&reference["program"], &mine["program"], name, &mut problems);
            }
            Err(error) => problems.push(format!("{name}: {error}")),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Compares two bound trees: stages by name, effect, phase, `waits` and the
/// value of every parameter both trees have. Parameters present in one tree
/// only, and defaults the specification records as `null` where the catalog
/// now gives a value, follow revisions of the catalog and are not compared.
fn compare(reference: &Value, mine: &Value, at: &str, problems: &mut Vec<String>) {
    match (reference, mine) {
        (Value::Object(left), Value::Object(_)) if left.get("t") == Some(&Value::from("stage")) => {
            compare_stage(reference, mine, at, problems);
        }
        (Value::Object(left), Value::Object(right)) => {
            let keys: BTreeSet<&String> = left
                .keys()
                .chain(right.keys())
                .filter(|key| key.as_str() != "span")
                .collect();
            for key in keys {
                let null = Value::Null;
                compare(
                    left.get(key).unwrap_or(&null),
                    right.get(key).unwrap_or(&null),
                    &format!("{at}.{key}"),
                    problems,
                );
            }
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                compare(left, right, &format!("{at}[{index}]"), problems);
            }
        }
        (left, right) if left == right => {}
        (left, right) => problems.push(format!("{at}: specification {left}, binder {right}")),
    }
}

fn compare_stage(reference: &Value, mine: &Value, at: &str, problems: &mut Vec<String>) {
    let at = format!("{at}<{}>", reference["name"].as_str().unwrap_or_default());
    for member in ["name", "effect", "phase"] {
        compare(
            &reference[member],
            &mine[member],
            &format!("{at}.{member}"),
            problems,
        );
    }
    let waits = |stage: &Value| stage["waits"].as_bool().unwrap_or(false);
    if waits(reference) != waits(mine) {
        problems.push(format!(
            "{at}.waits: specification {}, binder {}",
            waits(reference),
            waits(mine)
        ));
    }
    let empty = Vec::new();
    let mine_args = mine["args"].as_array().unwrap_or(&empty);
    for arg in reference["args"].as_array().unwrap_or(&empty) {
        let name = arg["name"].as_str().unwrap_or_default();
        let Some(other) = mine_args.iter().find(|other| other["name"] == arg["name"]) else {
            continue;
        };
        let default = |arg: &Value| arg["default"].as_bool().unwrap_or(false);
        if default(arg) != default(other) {
            problems.push(format!(
                "{at}.{name}.default: specification {}, binder {}",
                default(arg),
                default(other)
            ));
        }
        let added_default = default(arg) && arg["value"]["t"] == "null";
        if !added_default {
            compare(
                &arg["value"],
                &other["value"],
                &format!("{at}.{name}"),
                problems,
            );
        }
    }
}
