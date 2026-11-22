// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden trees and negative cases against the published schemas and the
//! decoders of the crate.
//!
//! `tests/d1/golden/invalid/expected.tsv` lists every negative case with the
//! expected error code and the layer that rejects it: `schema` (the
//! published schema), `validate` (an invariant beyond the schema) or
//! `accepted` (a document that the published contract admits).

mod hash_support;

use std::fs;

use hash_support::{
    Format, all_trees, d1_golden, decode_json, read_text, spec_schemas, spec_trees,
};
use nurt_contracts::ast::{AST_FORMAT, AST_VERSION};
use nurt_contracts::bound::BOUND_FORMAT;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_syntax::schema::{AST_SCHEMA, AST_SCHEMA_NAME, BOUND_SCHEMA, BOUND_SCHEMA_NAME, by_name};
use serde_json::{Value, json};

/// Number of negative cases of `tests/d1/golden/invalid/`.
const INVALID_CASES: usize = 16;

fn validator(schema: &str) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(schema).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

fn validator_for(format: Format) -> jsonschema::Validator {
    match format {
        Format::Surface => validator(AST_SCHEMA),
        Format::Bound => validator(BOUND_SCHEMA),
    }
}

fn schema_errors(validator: &jsonschema::Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect()
}

#[test]
fn exported_schemas_are_the_published_files() {
    let ast = read_text(&spec_schemas().join("nurt-ast-1.schema.json"));
    let bound = read_text(&spec_schemas().join("nurt-bound-1.schema.json"));
    assert_eq!(AST_SCHEMA, ast);
    assert_eq!(BOUND_SCHEMA, bound);
    let ast: Value = serde_json::from_str(AST_SCHEMA).unwrap();
    let bound: Value = serde_json::from_str(BOUND_SCHEMA).unwrap();
    assert_eq!(ast["title"], AST_FORMAT);
    assert_eq!(bound["title"], BOUND_FORMAT);
}

#[test]
fn schemas_are_found_by_short_and_format_name() {
    assert_eq!(by_name(AST_SCHEMA_NAME), Some(AST_SCHEMA));
    assert_eq!(by_name(AST_FORMAT), Some(AST_SCHEMA));
    assert_eq!(by_name(BOUND_SCHEMA_NAME), Some(BOUND_SCHEMA));
    assert_eq!(by_name(BOUND_FORMAT), Some(BOUND_SCHEMA));
    assert_eq!(by_name("nurt-report/1"), None);
    assert_eq!(by_name(""), None);
}

#[test]
fn every_golden_tree_conforms_to_the_exported_schema() {
    let surface = validator_for(Format::Surface);
    let bound = validator_for(Format::Bound);
    let trees = all_trees();
    assert!(trees.len() > spec_trees().len() - 1);
    for tree in trees {
        let validator = match tree.format {
            Format::Surface => &surface,
            Format::Bound => &bound,
        };
        let errors = schema_errors(validator, &tree.json());
        assert!(errors.is_empty(), "{}: {errors:#?}", tree.path.display());
    }
}

#[derive(Debug)]
struct Case {
    file: String,
    format: Format,
    code: Option<ErrorCode>,
    layer: String,
}

fn cases() -> Vec<Case> {
    let index = read_text(&d1_golden().join("invalid/expected.tsv"));
    index
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let columns: Vec<&str> = line.split('\t').collect();
            assert_eq!(columns.len(), 5, "{line}");
            let format = match columns[1] {
                "ast" => Format::Surface,
                "bound" => Format::Bound,
                other => panic!("unknown kind {other}"),
            };
            let code = match columns[2] {
                "-" => None,
                code => Some(serde_json::from_value(json!(code)).unwrap()),
            };
            Case {
                file: columns[0].to_owned(),
                format,
                code,
                layer: columns[3].to_owned(),
            }
        })
        .collect()
}

#[test]
fn expected_index_lists_every_negative_case() {
    let listed: Vec<String> = cases().into_iter().map(|case| case.file).collect();
    let mut present: Vec<String> = fs::read_dir(d1_golden().join("invalid"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .map(|name| format!("invalid/{name}"))
        .collect();
    present.sort();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(sorted, present);
    assert_eq!(listed.len(), INVALID_CASES);
}

#[test]
fn negative_cases_are_rejected_by_the_expected_layer_with_the_expected_code() {
    for case in cases() {
        let path = d1_golden().join(&case.file);
        let text = read_text(&path);
        let instance: Value = serde_json::from_str(&text).unwrap();
        let schema_valid = validator_for(case.format).is_valid(&instance);
        let decoded = decode_json(case.format, &text);
        match case.layer.as_str() {
            "schema" => assert!(!schema_valid, "{}: the schema accepts it", case.file),
            "validate" | "accepted" => {
                let errors = schema_errors(&validator_for(case.format), &instance);
                assert!(errors.is_empty(), "{}: {errors:#?}", case.file);
            }
            other => panic!("{}: unknown layer {other}", case.file),
        }
        match (case.code, decoded) {
            (None, Ok(_)) => {}
            (None, Err(error)) => panic!("{}: rejected with {error}", case.file),
            (Some(code), Ok(_)) => panic!("{}: accepted, expected {code}", case.file),
            (Some(code), Err(error)) => {
                assert_eq!(Error::from(error).code(), code, "{}", case.file);
            }
        }
    }
}

fn first_surface_tree() -> Value {
    spec_trees()
        .into_iter()
        .find(|tree| tree.format == Format::Surface && tree.stem().starts_with("task-01"))
        .unwrap()
        .json()
}

fn surface_rejects(instance: &Value, what: &str) {
    let surface = validator_for(Format::Surface);
    assert!(
        !surface.is_valid(instance),
        "the surface schema accepts {what}"
    );
    let error = decode_json(Format::Surface, &instance.to_string())
        .err()
        .unwrap_or_else(|| panic!("the surface decoder accepts {what}"));
    assert_eq!(Error::from(error).code(), ErrorCode::AstSchema, "{what}");
}

#[test]
fn surface_schema_rejects_members_of_the_bound_tree() {
    let tree = first_surface_tree();
    assert!(validator_for(Format::Surface).is_valid(&tree));
    for (member, value) in [
        ("bound", json!(AST_VERSION)),
        ("catalog_rev", json!("cat-1")),
        ("corr_rev", json!("corr-1")),
    ] {
        let mut instance = tree.clone();
        instance[member] = value;
        surface_rejects(&instance, &format!("the root member {member}"));
    }
    for (member, value) in [("effect", json!("read")), ("phase", json!("source"))] {
        let mut instance = tree.clone();
        instance["program"]["statements"][0]["head"][member] = value.clone();
        surface_rejects(&instance, &format!("the head member {member}"));
        let mut instance = tree.clone();
        instance["program"]["statements"][0]["stages"][0][member] = value;
        surface_rejects(&instance, &format!("the stage member {member}"));
    }
    let mut instance = tree;
    instance["program"]["statements"][0]["head"]["args"][0]["default"] = json!(true);
    surface_rejects(&instance, "the argument member default");
}

#[test]
fn surface_schema_rejects_every_bound_golden_tree() {
    let surface = validator_for(Format::Surface);
    for tree in all_trees()
        .into_iter()
        .filter(|tree| tree.format == Format::Bound)
    {
        assert!(!surface.is_valid(&tree.json()), "{}", tree.path.display());
    }
}
