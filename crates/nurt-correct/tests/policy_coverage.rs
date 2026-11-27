// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/tests/policy_coverage.rs
// Responsibility:   Policy helpers on whole trees: program kind, effecting commands, and the.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Policy helpers on whole trees: program kind, effecting commands, and the
//! invariant check on every expression form.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a failed assertion is the failure of the test"
)]

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::spec::StageSpec;
use nurt_correct::policy::{
    Invariant, InvariantInput, InvariantViolation, ProgramKind, check_invariants, is_effecting,
};
use nurt_correct::tables::load_embedded;
use serde_json::{Value, json};

fn surface(statements: Value) -> AstDocument {
    serde_json::from_value(json!({ "nurt": "1.0", "program": { "statements": statements } }))
        .unwrap()
}

fn bound(statements: Value) -> BoundDocument {
    serde_json::from_value(json!({
        "nurt": "1.0",
        "bound": "1.0",
        "catalog_rev": "cat-1",
        "corr_rev": "corr-1",
        "program": { "statements": statements }
    }))
    .unwrap()
}

fn pipeline(head: Value, stages: &[Value]) -> Value {
    json!({ "t": "pipeline", "head": head, "stages": stages })
}

fn written(name: &str, args: &[(&str, Value)]) -> Value {
    let args: Vec<Value> = args
        .iter()
        .map(|(arg, value)| {
            if arg.is_empty() {
                json!({ "value": value })
            } else {
                json!({ "name": arg, "value": value })
            }
        })
        .collect();
    json!({ "t": "stage", "name": name, "args": args })
}

fn resolved(name: &str, phase: &str, args: &[(&str, Value)]) -> Value {
    let effect = if matches!(phase, "mutate" | "run") {
        "local"
    } else {
        "read"
    };
    let args: Vec<Value> = args
        .iter()
        .map(|(arg, value)| json!({ "name": arg, "value": value, "default": false }))
        .collect();
    json!({ "t": "stage", "name": name, "effect": effect, "phase": phase, "args": args })
}

fn text(v: &str) -> Value {
    json!({ "t": "text", "v": v })
}

fn path(v: &str) -> Value {
    json!({ "t": "path", "v": v })
}

fn int(v: i64) -> Value {
    json!({ "t": "int", "v": v })
}

fn spec(category: &str, effect: &str, actions: &[&str]) -> StageSpec {
    let effects: Vec<Value> = actions
        .iter()
        .map(|effect| json!({ "action": "go", "effect": effect, "reversibility": "irreversible" }))
        .collect();
    serde_json::from_value(json!({
        "name": "probe",
        "category": category,
        "reversibility": "read",
        "effect": effect,
        "implementation": "builtin",
        "input": [],
        "output": [],
        "summary": "probe",
        "effects": effects,
    }))
    .unwrap()
}

fn check(before: &AstDocument, after: &BoundDocument) -> Result<(), InvariantViolation> {
    let tables = load_embedded().unwrap();
    check_invariants(&InvariantInput {
        before: Some(before),
        after,
        corrections: &[],
        base: None,
        tables: &tables,
        effecting: &|name| name == "remove",
    })
}

#[test]
fn classifies_bound_programs_by_the_phases_of_their_stages() {
    let reading = bound(json!([pipeline(
        resolved("find", "source", &[]),
        &[resolved("first", "transform", &[])]
    )]));
    assert_eq!(ProgramKind::of_bound(&reading), ProgramKind::Read);

    let head_ref = bound(json!([
        { "t": "let", "name": "r", "value": pipeline(resolved("find", "source", &[]), &[]) },
        pipeline(json!({ "t": "ref", "name": "r" }), &[resolved("remove", "mutate", &[])]),
    ]));
    assert_eq!(ProgramKind::of_bound(&head_ref), ProgramKind::Effect);

    let only_ref = bound(json!([pipeline(json!({ "t": "ref", "name": "r" }), &[])]));
    assert_eq!(ProgramKind::of_bound(&only_ref), ProgramKind::Read);

    let running = bound(json!([pipeline(resolved("run", "run", &[]), &[])]));
    assert_eq!(ProgramKind::of_bound(&running), ProgramKind::Effect);
}

#[test]
fn recognizes_effecting_commands_by_category_and_effect() {
    assert!(is_effecting(&spec("change", "read", &[])));
    assert!(is_effecting(&spec("run", "read", &[])));
    assert!(is_effecting(&spec("system", "local", &[])));
    assert!(is_effecting(&spec("system", "read", &["local"])));
    assert!(!is_effecting(&spec("source", "read", &[])));
    assert!(!is_effecting(&spec("system", "read", &["read"])));
}

fn every_form_surface() -> AstDocument {
    let args = vec![
        (
            "",
            json!({ "t": "list", "items": [
                text("/srv/a"), text("*.rs"),
                json!({ "t": "ident", "v": "id" }),
                json!({ "t": "regex", "v": "re", "flags": "" }),
                json!({ "t": "bool", "v": true }),
                text("1,000"), text("5-9"), text("90 minutes"),
            ] }),
        ),
        (
            "limit",
            json!({ "t": "map", "entries": [{ "k": "k", "v": int(5) }] }),
        ),
        ("spread", json!({ "t": "spread", "x": text("s") })),
        (
            "un",
            json!({ "t": "un", "op": "not", "x": { "t": "bool", "v": true } }),
        ),
        (
            "field",
            json!({ "t": "field", "name": "f", "of": text("o") }),
        ),
        (
            "call",
            json!({ "t": "call", "fn": "lower", "args": [{ "t": "num", "v": 3.0 }, { "t": "num", "v": 2.5 }] }),
        ),
        (
            "bin",
            json!({ "t": "bin", "op": "+", "l": int(1), "r": int(2) }),
        ),
        ("r", json!({ "t": "range", "lo": 1, "hi": 9 })),
        ("timeout", json!({ "t": "dur", "v": 1000 })),
        (
            "sub",
            json!({ "t": "sub", "p": pipeline(written("list", &[("", text("/sub"))]), &[]) }),
        ),
        ("ref", json!({ "t": "ref", "name": "r" })),
    ];
    surface(json!([
        { "t": "let", "name": "r", "value": pipeline(written("find", &args), &[]) },
        pipeline(json!({ "t": "ref", "name": "r" }), &[written("first", &[("", int(3))])]),
    ]))
}

fn every_form_bound() -> BoundDocument {
    let args = vec![
        (
            "",
            json!({ "t": "list", "items": [
                path("/srv/a"),
                json!({ "t": "glob", "v": "*.rs" }),
                text("id"),
                json!({ "t": "regex", "v": "re", "flags": "" }),
            ] }),
        ),
        (
            "limit",
            json!({ "t": "map", "entries": [{ "k": "k", "v": int(5) }] }),
        ),
        ("spread", json!({ "t": "spread", "x": text("s") })),
        (
            "un",
            json!({ "t": "un", "op": "not", "x": { "t": "bool", "v": true } }),
        ),
        (
            "field",
            json!({ "t": "field", "name": "f", "of": text("o") }),
        ),
        (
            "call",
            json!({ "t": "call", "fn": "lower", "args": [{ "t": "int", "v": 3 }] }),
        ),
        (
            "bin",
            json!({ "t": "bin", "op": "+", "l": int(1), "r": int(2) }),
        ),
        ("r", json!({ "t": "range", "lo": 5, "hi": 9 })),
        ("timeout", json!({ "t": "dur", "v": 1000 })),
        ("timeout", json!({ "t": "dur", "v": 5_400_000 })),
        ("limit", int(1000)),
        (
            "sub",
            json!({ "t": "sub", "p": pipeline(resolved("list", "source", &[("", path("/sub"))]), &[]) }),
        ),
    ];
    bound(json!([
        { "t": "let", "name": "r", "value": pipeline(resolved("find", "source", &args), &[]) },
        pipeline(
            json!({ "t": "ref", "name": "r" }),
            &[resolved("first", "transform", &[("", int(3))])]
        ),
    ]))
}

#[test]
fn accepts_literals_and_numbers_taken_from_every_expression_form() {
    assert_eq!(check(&every_form_surface(), &every_form_bound()), Ok(()));
}

#[test]
fn rejects_a_literal_that_is_not_in_the_written_program() {
    let before = surface(json!([pipeline(written("find", &[("", text("/a"))]), &[])]));
    let after = bound(json!([pipeline(
        resolved("find", "source", &[("", text("invented"))]),
        &[]
    )]));
    let violation = check(&before, &after).unwrap_err();
    assert_eq!(violation.invariant, Invariant::LiteralBytes);
}

#[test]
fn rejects_a_changed_number_inside_a_nested_pipeline() {
    let before = surface(json!([pipeline(
        written(
            "find",
            &[(
                "sub",
                json!({ "t": "sub", "p": pipeline(written("first", &[("", int(3))]), &[]) })
            )]
        ),
        &[]
    )]));
    let after = bound(json!([pipeline(
        resolved(
            "find",
            "source",
            &[(
                "sub",
                json!({ "t": "sub", "p": pipeline(resolved("first", "transform", &[("", int(4))]), &[]) })
            )]
        ),
        &[]
    )]));
    assert_eq!(
        check(&before, &after).unwrap_err().invariant,
        Invariant::Numbers
    );
}

#[test]
fn rejects_a_changed_statement_count() {
    let before = surface(json!([pipeline(written("find", &[]), &[])]));
    let after = bound(json!([
        pipeline(resolved("find", "source", &[]), &[]),
        pipeline(resolved("list", "source", &[]), &[]),
    ]));
    assert_eq!(
        check(&before, &after).unwrap_err().invariant,
        Invariant::Targets
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
