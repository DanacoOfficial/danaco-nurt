// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Invariants of `validate_surface`, `validate_bound` and `check_members`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use hash_support::json_depth;
use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_syntax::validate::{
    MAX_DEPTH, MAX_SAFE_INTEGER, check_members, is_canonical_path, is_identifier, is_rfc3339,
    validate_bound, validate_surface,
};
use serde_json::{Value, json};

fn surface(value: Value) -> AstDocument {
    serde_json::from_value(value).unwrap()
}

fn surface_with_arg(arg: Value) -> Value {
    json!({"nurt": "1.0", "program": {"statements": [{
        "t": "pipeline",
        "head": {"t": "stage", "name": "find", "args": [arg]},
        "stages": []
    }]}})
}

fn surface_value(value: Value) -> Value {
    surface_with_arg(json!({"value": value}))
}

/// Asserts that the surface tree is rejected and returns the JSON Pointer of
/// the violation.
fn surface_violation(value: Value) -> String {
    let error = validate_surface(&surface(value.clone()))
        .err()
        .unwrap_or_else(|| panic!("accepted: {value}"));
    let pointer = error.found.clone().unwrap_or_default();
    assert_eq!(Error::from(error).code(), ErrorCode::AstSchema);
    pointer
}

fn surface_accepts(value: Value) {
    validate_surface(&surface(value.clone())).unwrap_or_else(|error| panic!("{error}: {value}"));
}

const ARG_VALUE: &str = "/program/statements/0/head/args/0/value";

#[test]
fn accepts_a_minimal_surface_tree() {
    surface_accepts(surface_value(json!({"t": "text", "v": "/srv"})));
}

#[test]
fn rejects_an_empty_program_and_an_unsupported_version() {
    let empty = json!({"nurt": "1.0", "program": {"statements": []}});
    assert_eq!(surface_violation(empty), "/program/statements");
    for version in ["2.0", "1.01", "1.", "1.x", "01.0", ""] {
        let mut value = surface_value(json!({"t": "null"}));
        value["nurt"] = json!(version);
        assert_eq!(surface_violation(value), "/nurt", "{version}");
    }
    let mut value = surface_value(json!({"t": "null"}));
    value["nurt"] = json!("1.12");
    surface_accepts(value);
}

#[test]
fn rejects_identifiers_outside_the_pattern() {
    for name in ["", "Find", "1find", "find-all", "fínd"] {
        assert!(!is_identifier(name), "{name}");
        let value = surface_value(json!({"t": "ident", "v": name}));
        assert_eq!(surface_violation(value), format!("{ARG_VALUE}/v"));
    }
    assert!(is_identifier("find_2"));
    let named = surface_with_arg(json!({"name": "Glob", "value": {"t": "null"}}));
    assert_eq!(
        surface_violation(named),
        "/program/statements/0/head/args/0/name"
    );
    let reference = surface_value(json!({"t": "ref", "name": "recent", "fields": ["Path"]}));
    assert_eq!(
        surface_violation(reference),
        format!("{ARG_VALUE}/fields/0")
    );
    let call = surface_value(json!({"t": "call", "fn": "Ago", "args": []}));
    assert_eq!(surface_violation(call), format!("{ARG_VALUE}/fn"));
    let field = surface_value(json!({"t": "field", "name": "size-1"}));
    assert_eq!(surface_violation(field), format!("{ARG_VALUE}/name"));
    let binding = json!({"nurt": "1.0", "program": {"statements": [{
        "t": "let", "name": "Recent",
        "value": {"t": "pipeline", "head": {"t": "stage", "name": "find", "args": []}, "stages": []}
    }]}});
    assert_eq!(surface_violation(binding), "/program/statements/0/name");
}

#[test]
fn accepts_dotted_command_names_and_rejects_malformed_ones() {
    let mut value = surface_value(json!({"t": "null"}));
    value["program"]["statements"][0]["head"]["name"] = json!("git.remote.add");
    surface_accepts(value.clone());
    for name in ["git.", ".git", "git..status", "Git"] {
        value["program"]["statements"][0]["head"]["name"] = json!(name);
        assert_eq!(
            surface_violation(value.clone()),
            "/program/statements/0/head/name",
            "{name}"
        );
    }
}

#[test]
fn rejects_regular_expression_flags_out_of_order() {
    for flags in ["", "i", "imsx", "ix", "ms"] {
        surface_accepts(surface_value(
            json!({"t": "regex", "v": "a", "flags": flags}),
        ));
    }
    for flags in ["mi", "ii", "u", "xs", "I"] {
        let value = surface_value(json!({"t": "regex", "v": "a", "flags": flags}));
        assert_eq!(
            surface_violation(value),
            format!("{ARG_VALUE}/flags"),
            "{flags}"
        );
    }
}

#[test]
fn rejects_surface_integers_outside_the_safe_non_negative_range() {
    surface_accepts(surface_value(json!({"t": "int", "v": MAX_SAFE_INTEGER})));
    for v in [json!(-1), json!(MAX_SAFE_INTEGER + 1)] {
        let value = surface_value(json!({"t": "int", "v": v}));
        assert_eq!(surface_violation(value), format!("{ARG_VALUE}/v"));
    }
    for kind in ["size", "dur"] {
        surface_accepts(surface_value(json!({"t": kind, "v": MAX_SAFE_INTEGER})));
        let value = surface_value(json!({"t": kind, "v": MAX_SAFE_INTEGER + 1}));
        assert_eq!(surface_violation(value), format!("{ARG_VALUE}/v"));
    }
    let range = surface_value(json!({"t": "range", "lo": -1}));
    assert_eq!(surface_violation(range), format!("{ARG_VALUE}/lo"));
    let range = surface_value(json!({"t": "range", "lo": 1, "hi": MAX_SAFE_INTEGER + 1}));
    assert_eq!(surface_violation(range), format!("{ARG_VALUE}/hi"));
    surface_accepts(surface_value(json!({"t": "range", "lo": 10, "hi": 12})));
    surface_accepts(surface_value(json!({"t": "num", "v": -2.5})));
}

#[test]
fn rejects_a_spread_outside_a_list() {
    let spread = json!({"t": "spread", "x": {"t": "ref", "name": "recent"}});
    assert_eq!(surface_violation(surface_value(spread.clone())), ARG_VALUE);
    surface_accepts(surface_value(
        json!({"t": "list", "items": [spread.clone()]}),
    ));
    let nested = json!({"t": "list", "items": [{"t": "un", "op": "not", "x": spread}]});
    assert_eq!(
        surface_violation(surface_value(nested)),
        format!("{ARG_VALUE}/items/0/x")
    );
}

#[test]
fn rejects_duplicate_map_keys() {
    let map = json!({"t": "map", "entries": [
        {"k": "replace", "v": {"t": "text", "v": "a"}},
        {"k": "with", "v": {"t": "text", "v": "b"}}
    ]});
    surface_accepts(surface_value(map));
    let map = json!({"t": "map", "entries": [
        {"k": "replace", "v": {"t": "text", "v": "a"}},
        {"k": "replace", "v": {"t": "text", "v": "b"}}
    ]});
    assert_eq!(
        surface_violation(surface_value(map)),
        format!("{ARG_VALUE}/entries/1/k")
    );
}

#[test]
fn checks_structured_keys() {
    let key = json!({"t": "key", "segs": ["scripts", 0, -1, {"wild": "*"},
        {"filter": {"t": "bin", "op": "==",
            "l": {"t": "field", "name": "name"}, "r": {"t": "text", "v": "x"}}}]});
    surface_accepts(surface_value(key));
    let empty = json!({"t": "key", "segs": []});
    assert_eq!(
        surface_violation(surface_value(empty)),
        format!("{ARG_VALUE}/segs")
    );
    let large = json!({"t": "key", "segs": [-MAX_SAFE_INTEGER - 1]});
    assert_eq!(
        surface_violation(surface_value(large)),
        format!("{ARG_VALUE}/segs/0")
    );
    let filter = json!({"t": "key", "segs": [{"filter": {"t": "ident", "v": "Bad"}}]});
    assert_eq!(
        surface_violation(surface_value(filter)),
        format!("{ARG_VALUE}/segs/0/filter/v")
    );
}

#[test]
fn checks_operations_and_sub_pipelines() {
    let value = json!({"t": "bin", "op": "and",
        "l": {"t": "un", "op": "not", "x": {"t": "field", "name": "hidden",
            "of": {"t": "ident", "v": "file"}}},
        "r": {"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 3_600_000}]}});
    surface_accepts(surface_value(value));
    let sub = json!({"t": "sub", "p": {"t": "pipeline",
        "head": {"t": "ref", "name": "recent"},
        "stages": [{"t": "stage", "name": "Count", "args": []}]}});
    assert_eq!(
        surface_violation(surface_value(sub)),
        format!("{ARG_VALUE}/p/stages/0/name")
    );
    let bin = json!({"t": "bin", "op": "+", "l": {"t": "int", "v": 1}, "r": {"t": "int", "v": -1}});
    assert_eq!(
        surface_violation(surface_value(bin)),
        format!("{ARG_VALUE}/r/v")
    );
}

/// Surface tree whose deepest value lies at the given depth: nested `un`
/// nodes around `null`.
fn nested_surface(depth: usize) -> Value {
    let base = surface_value(json!({"t": "null"}));
    let base_depth = json_depth(&base);
    let mut expr = json!({"t": "null"});
    for _ in base_depth..depth {
        expr = json!({"t": "un", "op": "not", "x": expr});
    }
    let value = surface_value(expr);
    assert_eq!(json_depth(&value), depth);
    value
}

#[test]
fn accepts_depth_128_and_rejects_depth_129() {
    surface_accepts(nested_surface(MAX_DEPTH));
    let pointer = surface_violation(nested_surface(MAX_DEPTH + 1));
    assert!(pointer.starts_with(ARG_VALUE), "{pointer}");
}

#[test]
fn counts_spans_and_metadata_in_the_depth() {
    let mut value = nested_surface(MAX_DEPTH);
    surface_accepts(value.clone());
    let mut pointer = ARG_VALUE.to_owned();
    while value.pointer(&format!("{pointer}/x")).is_some() {
        pointer.push_str("/x");
    }
    value.pointer_mut(&pointer).unwrap()["span"] = json!([1, 1, 1, 5]);
    assert_eq!(json_depth(&value), MAX_DEPTH + 1);
    surface_violation(value);
    let mut meta = surface_value(json!({"t": "null"}));
    let mut nested = json!(1);
    for _ in 0..MAX_DEPTH - 2 {
        nested = json!([nested]);
    }
    meta["meta"] = json!({"comments": nested});
    assert_eq!(json_depth(&meta), MAX_DEPTH + 1);
    assert_eq!(surface_violation(meta), "/meta");
}

fn bound_stage(name: &str, effect: &str, phase: &str) -> Value {
    json!({"t": "stage", "name": name, "effect": effect, "phase": phase, "args": []})
}

fn bound_pipeline(head: Value, stages: Vec<Value>) -> Value {
    json!({"nurt": "1.0", "bound": "1.0", "catalog_rev": "cat-1", "corr_rev": "corr-1",
        "program": {"statements": [{"t": "pipeline", "head": head, "stages": stages}]}})
}

fn bound_violation(value: Value) -> String {
    let document: BoundDocument = serde_json::from_value(value.clone()).unwrap();
    let error = validate_bound(&document)
        .err()
        .unwrap_or_else(|| panic!("accepted: {value}"));
    let pointer = error.found.clone().unwrap_or_default();
    assert_eq!(Error::from(error).code(), ErrorCode::AstSchema);
    pointer
}

fn bound_accepts(value: Value) {
    let document: BoundDocument = serde_json::from_value(value.clone()).unwrap();
    validate_bound(&document).unwrap_or_else(|error| panic!("{error}: {value}"));
}

#[test]
fn accepts_the_phase_order_of_an_effect_pipeline() {
    bound_accepts(bound_pipeline(
        bound_stage("find", "read", "source"),
        vec![
            bound_stage("filter", "read", "transform"),
            bound_stage("assert", "read", "check"),
            bound_stage("replace", "local", "mutate"),
            bound_stage("assert", "read", "after"),
            bound_stage("verify", "read", "verify"),
        ],
    ));
    bound_accepts(bound_pipeline(
        bound_stage("run", "process", "run"),
        vec![bound_stage("assert", "read", "after")],
    ));
    bound_accepts(bound_pipeline(
        json!({"t": "ref", "name": "recent", "fields": ["path"]}),
        vec![bound_stage("remove", "local", "mutate")],
    ));
}

#[test]
fn rejects_phases_out_of_order() {
    let cases = [
        (
            bound_stage("filter", "read", "transform"),
            vec![],
            "/program/statements/0/head/phase",
        ),
        (
            bound_stage("find", "read", "source"),
            vec![bound_stage("assert", "read", "after")],
            "/program/statements/0/stages/0/phase",
        ),
        (
            bound_stage("find", "read", "source"),
            vec![
                bound_stage("remove", "local", "mutate"),
                bound_stage("run", "process", "run"),
            ],
            "/program/statements/0/stages/1/phase",
        ),
        (
            bound_stage("find", "read", "source"),
            vec![
                bound_stage("remove", "local", "mutate"),
                bound_stage("filter", "read", "transform"),
            ],
            "/program/statements/0/stages/1/phase",
        ),
        (
            bound_stage("find", "read", "source"),
            vec![
                bound_stage("remove", "local", "mutate"),
                bound_stage("verify", "read", "verify"),
                bound_stage("assert", "read", "after"),
            ],
            "/program/statements/0/stages/2/phase",
        ),
        (
            bound_stage("find", "read", "source"),
            vec![bound_stage("verify", "read", "verify")],
            "/program/statements/0/stages/0/phase",
        ),
    ];
    for (head, stages, pointer) in cases {
        assert_eq!(bound_violation(bound_pipeline(head, stages)), pointer);
    }
}

#[test]
fn rejects_an_effect_that_does_not_match_the_phase() {
    for (effect, phase) in [
        ("local", "source"),
        ("read", "mutate"),
        ("read", "run"),
        ("process", "verify"),
    ] {
        let head = bound_stage("x", effect, phase);
        assert_eq!(
            bound_violation(bound_pipeline(head, vec![])),
            "/program/statements/0/head/effect",
            "{effect} {phase}"
        );
    }
    bound_accepts(bound_pipeline(
        bound_stage("x", "external", "mutate"),
        vec![],
    ));
}

#[test]
fn rejects_an_effect_in_a_let_pipeline() {
    let value = json!({"nurt": "1.0", "bound": "1.0", "catalog_rev": "c", "corr_rev": "c",
        "program": {"statements": [{"t": "let", "name": "x", "value": {"t": "pipeline",
            "head": bound_stage("find", "read", "source"),
            "stages": [bound_stage("remove", "local", "mutate")]}}]}});
    assert_eq!(bound_violation(value), "/program/statements/0/value");
}

#[test]
fn rejects_bound_names_revisions_and_duplicate_parameters() {
    let mut value = bound_pipeline(bound_stage("git.remote.add", "local", "mutate"), vec![]);
    assert_eq!(
        bound_violation(value.clone()),
        "/program/statements/0/head/name"
    );
    value["program"]["statements"][0]["head"]["name"] = json!("git.commit");
    bound_accepts(value.clone());
    value["catalog_rev"] = json!("");
    assert_eq!(bound_violation(value.clone()), "/catalog_rev");
    value["catalog_rev"] = json!("cat-1");
    value["corr_rev"] = json!("");
    assert_eq!(bound_violation(value.clone()), "/corr_rev");
    value["corr_rev"] = json!("corr-1");
    value["bound"] = json!("2.0");
    assert_eq!(bound_violation(value.clone()), "/bound");
    value["bound"] = json!("1.0");
    value["program"]["statements"][0]["head"]["args"] = json!([
        {"name": "path", "value": {"t": "null"}, "default": true},
        {"name": "path", "value": {"t": "null"}}
    ]);
    assert_eq!(
        bound_violation(value),
        "/program/statements/0/head/args/1/name"
    );
}

fn bound_value(v: Value) -> Value {
    let mut head = bound_stage("list", "read", "source");
    head["args"] = json!([{"name": "value", "value": v}]);
    bound_pipeline(head, vec![])
}

const BOUND_VALUE: &str = "/program/statements/0/head/args/0/value";

#[test]
fn checks_bound_values() {
    for value in [
        json!({"t": "int", "v": -MAX_SAFE_INTEGER}),
        json!({"t": "path", "v": "/srv/data/my file.txt"}),
        json!({"t": "instant", "v": "2026-11-22T10:00:00.25+01:00"}),
        json!({"t": "range", "lo": -1, "hi": 1}),
        json!({"t": "list", "items": [{"t": "spread", "x": {"t": "ref", "name": "r"}}]}),
        json!({"t": "sub", "p": {"t": "pipeline", "head": {"t": "ref", "name": "r"},
            "stages": [bound_stage("count", "read", "transform")]}}),
        json!({"t": "map", "entries": [{"k": "a", "v": {"t": "enum", "v": "b"}}]}),
        json!({"t": "bin", "op": "~", "l": {"t": "field", "name": "path"},
            "r": {"t": "regex", "v": "x", "flags": "i"}}),
        json!({"t": "call", "fn": "ago", "args": [{"t": "dur", "v": 1}]}),
        json!({"t": "un", "op": "neg", "x": {"t": "num", "v": 1.5}}),
    ] {
        bound_accepts(bound_value(value));
    }
    let rejected = [
        (json!({"t": "int", "v": -MAX_SAFE_INTEGER - 1}), "/v"),
        (json!({"t": "size", "v": MAX_SAFE_INTEGER + 1}), "/v"),
        (json!({"t": "path", "v": "src/app"}), "/v"),
        (json!({"t": "instant", "v": "2026-11-22 10:00:00Z"}), "/v"),
        (json!({"t": "regex", "v": "x", "flags": "xi"}), "/flags"),
        (json!({"t": "range", "hi": MAX_SAFE_INTEGER + 1}), "/hi"),
        (json!({"t": "spread", "x": {"t": "ref", "name": "r"}}), ""),
        (
            json!({"t": "map", "entries": [{"k": "a", "v": {"t": "null"}}, {"k": "a", "v": {"t": "null"}}]}),
            "/entries/1/k",
        ),
        (
            json!({"t": "field", "name": "x", "of": {"t": "ref", "name": "R"}}),
            "/of/name",
        ),
        (
            json!({"t": "call", "fn": "x", "args": [{"t": "path", "v": "a"}]}),
            "/args/0/v",
        ),
        (
            json!({"t": "bin", "op": "+", "l": {"t": "null"}, "r": {"t": "un", "op": "neg", "x": {"t": "dur", "v": MAX_SAFE_INTEGER + 1}}}),
            "/r/x/v",
        ),
        (
            json!({"t": "sub", "p": {"t": "pipeline", "head": bound_stage("count", "read", "after"), "stages": []}}),
            "/p/head/phase",
        ),
    ];
    for (value, suffix) in rejected {
        assert_eq!(
            bound_violation(bound_value(value.clone())),
            format!("{BOUND_VALUE}{suffix}"),
            "{value}"
        );
    }
}

#[test]
fn recognizes_canonical_paths() {
    for path in [
        "/",
        "/srv",
        "/srv/app",
        "C:/",
        "C:/Users/a",
        "//server/share",
        "//server/share/a",
        "mem:/a",
    ] {
        assert!(is_canonical_path(path), "{path}");
    }
    for path in [
        "",
        "srv",
        "/srv/",
        "/srv//a",
        "/srv/./a",
        "/srv/../a",
        "c:/a",
        "C:a",
        "C:\\a",
        "//server",
        "///share",
        "/a\0b",
        "m:/a",
        "Mem:/a",
    ] {
        assert!(!is_canonical_path(path), "{path}");
    }
}

#[test]
fn recognizes_rfc3339_instants() {
    for instant in [
        "2026-11-22T10:00:00Z",
        "2026-11-22T10:00:00.123456789Z",
        "2024-02-29T23:59:60Z",
        "2026-11-22T10:00:00-05:30",
    ] {
        assert!(is_rfc3339(instant), "{instant}");
    }
    for instant in [
        "2026-11-22",
        "2026-13-01T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-11-22T24:00:00Z",
        "2026-11-22T10:60:00Z",
        "2026-11-22T10:00:61Z",
        "2026-11-22T10:00:00.Z",
        "2026-11-22T10:00:00",
        "2026-11-22T10:00:00+24:00",
        "2026-11-22T10:00:00+05",
        "2026-11-22t10:00:00z",
    ] {
        assert!(!is_rfc3339(instant), "{instant}");
    }
}

#[test]
fn check_members_reports_unknown_members_and_admits_normalized_defaults() {
    let received = json!({"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
        "head": {"t": "ref", "name": "r", "fields": []},
        "stages": [{"t": "stage", "name": "count", "args": [], "effect": "read"}]}]}});
    let mut clean = received.clone();
    clean["program"]["statements"][0]["stages"][0]
        .as_object_mut()
        .unwrap()
        .remove("effect");
    let typed = serde_json::to_value(surface(clean)).unwrap();
    let error = check_members(&received, &typed).unwrap_err();
    assert_eq!(
        error.found.as_deref(),
        Some("/program/statements/0/stages/0/effect")
    );
    let mut bound = bound_pipeline(bound_stage("find", "read", "source"), vec![]);
    bound["program"]["statements"][0]["head"]["waits"] = json!(false);
    bound["program"]["statements"][0]["head"]["args"] =
        json!([{"name": "path", "value": {"t": "null"}, "default": false}]);
    let typed =
        serde_json::to_value(serde_json::from_value::<BoundDocument>(bound.clone()).unwrap())
            .unwrap();
    check_members(&bound, &typed).unwrap();
    bound["meta"] = json!({"a/b~c": {"x": 1}});
    let typed = json!({"meta": {"a/b~c": {}}});
    let error = check_members(&bound, &typed).unwrap_err();
    assert_eq!(error.found.as_deref(), Some("/nurt"));
    let error = check_members(&json!({"meta": {"a/b~c": {"x": 1}}}), &typed).unwrap_err();
    assert_eq!(error.found.as_deref(), Some("/meta/a~1b~0c/x"));
}

#[test]
fn rejects_metadata_integers_outside_the_safe_range() {
    let mut value = surface_value(json!({"t": "null"}));
    value["meta"] = json!({"count": MAX_SAFE_INTEGER, "ratio": 1e300, "low": -MAX_SAFE_INTEGER});
    surface_accepts(value.clone());
    for number in [
        json!(MAX_SAFE_INTEGER + 1),
        json!(-MAX_SAFE_INTEGER - 1),
        json!(u64::MAX),
    ] {
        value["meta"] = json!({"nested": [{"count": number}]});
        assert_eq!(surface_violation(value.clone()), "/meta");
    }
}

#[test]
fn rejects_metadata_integers_outside_the_safe_range_in_a_bound_tree() {
    let mut value = bound_pipeline(bound_stage("find", "read", "source"), vec![]);
    value["meta"] = json!({"count": MAX_SAFE_INTEGER + 1});
    assert_eq!(bound_violation(value), "/meta");
}

#[test]
fn rejects_empty_regular_expression_patterns() {
    for flags in ["", "i"] {
        let value = surface_value(json!({"t": "regex", "v": "", "flags": flags}));
        assert_eq!(surface_violation(value), format!("{ARG_VALUE}/v"), "{flags}");
        let value = json!({"t": "regex", "v": "", "flags": flags});
        assert_eq!(
            bound_violation(bound_value(value)),
            format!("{BOUND_VALUE}/v"),
            "{flags}"
        );
    }
    surface_accepts(surface_value(json!({"t": "regex", "v": " ", "flags": ""})));
}

#[test]
fn rejects_empty_bound_globs_and_keys() {
    for kind in ["glob", "key"] {
        let value = json!({"t": kind, "v": ""});
        assert_eq!(
            bound_violation(bound_value(value)),
            format!("{BOUND_VALUE}/v"),
            "{kind}"
        );
        bound_accepts(bound_value(json!({"t": kind, "v": "a"})));
    }
}

#[test]
fn rejects_reserved_words_and_overlong_identifiers() {
    use nurt_syntax::validate::is_reserved_word;
    assert!(is_reserved_word("and"));
    assert!(!is_reserved_word("android"));
    assert!(is_identifier(&"a".repeat(64)));
    assert!(!is_identifier(&"a".repeat(65)));
}
