// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Bound golden trees of `tests/d1/golden/bound/<variant>/` against the
//! published schema `nurt-bound/1` and the invariants of `validate_bound`.

mod hash_support;

use hash_support::{Tree, d1_bound_trees};
use nurt_syntax::schema::{AST_SCHEMA, BOUND_SCHEMA};
use nurt_syntax::validate::validate_bound;
use serde_json::Value;

fn validator(schema: &str) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(schema).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

#[test]
fn bound_golden_trees_conform_to_the_bound_schema() {
    let bound = validator(BOUND_SCHEMA);
    for tree in d1_bound_trees() {
        let errors: Vec<String> = bound
            .iter_errors(&tree.json())
            .map(|error| format!("{} at {}", error, error.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{}: {errors:#?}", tree.path.display());
    }
}

#[test]
fn bound_golden_trees_pass_the_bound_invariants() {
    for tree in d1_bound_trees() {
        let Tree::Bound(document) = tree.decode() else {
            panic!("{}: not a bound tree", tree.path.display());
        };
        validate_bound(&document)
            .unwrap_or_else(|error| panic!("{}: {error}", tree.path.display()));
    }
}

#[test]
fn bound_golden_trees_are_rejected_by_the_surface_schema() {
    let surface = validator(AST_SCHEMA);
    for tree in d1_bound_trees() {
        assert!(!surface.is_valid(&tree.json()), "{}", tree.path.display());
    }
}
