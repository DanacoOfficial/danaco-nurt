// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Correction invariants checked on surface and bound trees, and a property
//! test of path literals.

#![allow(
    clippy::unwrap_used,
    reason = "helpers of the test build trees from fixed JSON; a malformed tree fails the test"
)]

use nurt_contracts::ast::AstDocument;
use nurt_contracts::bound::BoundDocument;
use nurt_contracts::error::ErrorCode;
use nurt_correct::policy::{
    Class, Invariant, InvariantInput, InvariantViolation, PathCheck, check_invariants, path_check,
};
use nurt_correct::record::{Correction, CorrectionKind, RuleId};
use nurt_correct::tables::{Tables, load_embedded};
use proptest::prelude::*;
use serde_json::{Value, json};

const EFFECTING: [&str; 6] = ["remove", "move", "copy", "write", "create", "run"];

fn effecting(name: &str) -> bool {
    EFFECTING.contains(&name)
}

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

fn pipeline(stages: &[Value]) -> Value {
    let (head, rest) = stages.split_first().unwrap();
    json!({ "t": "pipeline", "head": head, "stages": rest })
}

/// Surface stage with `(name, value)` arguments; an empty name is positional.
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

/// Bound stage with `(name, value, default)` arguments.
fn resolved(name: &str, effect: &str, phase: &str, args: &[(&str, Value, bool)]) -> Value {
    let args: Vec<Value> = args
        .iter()
        .map(|(arg, value, default)| json!({ "name": arg, "value": value, "default": default }))
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

fn check(
    before: &AstDocument,
    after: &BoundDocument,
    corrections: &[Correction],
    base: Option<&str>,
    tables: &Tables,
) -> Result<(), InvariantViolation> {
    check_invariants(&InvariantInput {
        before: Some(before),
        after,
        corrections,
        base,
        tables,
        effecting: &effecting,
    })
}

fn violated(result: Result<(), InvariantViolation>) -> Invariant {
    result.unwrap_err().invariant
}

fn remove_program(written_path: &str, bound_path: &str) -> (AstDocument, BoundDocument) {
    let before = surface(json!([pipeline(&[written(
        "remove",
        &[("", text(written_path))]
    )])]));
    let after = bound(json!([pipeline(&[
        resolved(
            "find",
            "read",
            "source",
            &[
                ("path", path(bound_path), false),
                ("type", json!({ "t": "enum", "v": "any" }), false),
                ("depth", int(0), false),
            ]
        ),
        resolved(
            "remove",
            "local",
            "mutate",
            &[("on_missing", json!({ "t": "enum", "v": "skip" }), true)]
        ),
    ])]));
    (before, after)
}

#[test]
fn accepts_the_expansion_of_a_source_form() {
    let tables = load_embedded().unwrap();
    let (before, after) = remove_program("/srv/data/my file.txt", "/srv/data/my file.txt");
    assert_eq!(check(&before, &after, &[], None, &tables), Ok(()));
}

#[test]
fn accepts_a_corrected_command_name() {
    let tables = load_embedded().unwrap();
    let before = surface(json!([pipeline(&[written(
        "remvoe",
        &[("", text("/srv/a.log"))]
    )])]));
    let after = bound(json!([pipeline(&[
        resolved(
            "find",
            "read",
            "source",
            &[
                ("path", path("/srv/a.log"), false),
                ("depth", int(0), false)
            ]
        ),
        resolved("remove", "local", "mutate", &[]),
    ])]));
    let corrections = [Correction::new(
        RuleId::L01,
        Class::Fuzzy,
        "remvoe",
        "remove",
        CorrectionKind::Command,
    )];
    assert_eq!(check(&before, &after, &corrections, None, &tables), Ok(()));
}

#[test]
fn rejects_changed_path_bytes() {
    let tables = load_embedded().unwrap();
    let (before, after) = remove_program("/srv/Data/a.txt", "/srv/data/a.txt");
    assert_eq!(
        violated(check(&before, &after, &[], None, &tables)),
        Invariant::LiteralBytes
    );
    let (before, after) = remove_program("c:\\proj\\out\\", "C:/proj/out");
    assert_eq!(check(&before, &after, &[], None, &tables), Ok(()));
}

#[test]
fn resolves_relative_paths_only_against_an_explicit_base() {
    let tables = load_embedded().unwrap();
    let (before, after) = remove_program("./build", "/srv/app/build");
    assert_eq!(
        violated(check(&before, &after, &[], None, &tables)),
        Invariant::Base
    );
    assert_eq!(
        check(&before, &after, &[], Some("/srv/app"), &tables),
        Ok(())
    );
    assert_eq!(
        violated(check(&before, &after, &[], Some("/srv/other"), &tables)),
        Invariant::LiteralBytes
    );
}

fn first_program(written_count: Value, bound_count: i64) -> (AstDocument, BoundDocument) {
    let before = surface(json!([pipeline(&[
        written("list", &[("", text("/srv"))]),
        written("first", &[("", written_count)]),
    ])]));
    let after = bound(json!([pipeline(&[
        resolved("list", "read", "source", &[("path", path("/srv"), false)]),
        resolved(
            "first",
            "read",
            "transform",
            &[("count", int(bound_count), false)]
        ),
    ])]));
    (before, after)
}

#[test]
fn keeps_counts_of_counting_stages() {
    let tables = load_embedded().unwrap();
    let (before, after) = first_program(int(10), 10);
    assert_eq!(check(&before, &after, &[], None, &tables), Ok(()));
    let (before, after) = first_program(int(10), 1);
    assert_eq!(
        violated(check(&before, &after, &[], None, &tables)),
        Invariant::Numbers
    );
    let (before, after) = first_program(text("1,000"), 1_000);
    assert_eq!(check(&before, &after, &[], None, &tables), Ok(()));
    let (before, after) = first_program(json!({ "t": "num", "v": 40.0 }), 40);
    assert_eq!(check(&before, &after, &[], None, &tables), Ok(()));
}

#[test]
fn keeps_durations_of_timeouts() {
    let tables = load_embedded().unwrap();
    let before = surface(json!([pipeline(&[written(
        "run",
        &[
            ("", json!({ "t": "list", "items": [text("make")] })),
            ("timeout", text("2 minutes"))
        ]
    )])]));
    let after = |millis: u64| {
        bound(json!([pipeline(&[resolved(
            "run",
            "process",
            "run",
            &[
                (
                    "command",
                    json!({ "t": "list", "items": [{ "t": "text", "v": "make" }] }),
                    false
                ),
                ("timeout", json!({ "t": "dur", "v": millis }), false),
            ]
        )])]))
    };
    assert_eq!(check(&before, &after(120_000), &[], None, &tables), Ok(()));
    assert_eq!(
        violated(check(&before, &after(60_000), &[], None, &tables)),
        Invariant::Numbers
    );
}

#[test]
fn keeps_the_stages_that_select_targets() {
    let tables = load_embedded().unwrap();
    let filter = json!({ "t": "bin", "op": ">", "l": { "t": "field", "name": "size" }, "r": { "t": "size", "v": 10 } });
    let before = surface(json!([pipeline(&[
        written("find", &[("", text("/srv"))]),
        written("filter", &[("", filter)]),
        written("remove", &[]),
    ])]));
    let after = bound(json!([pipeline(&[
        resolved("find", "read", "source", &[("path", path("/srv"), false)]),
        resolved("remove", "local", "mutate", &[]),
    ])]));
    let error = check(&before, &after, &[], None, &tables).unwrap_err();
    assert_eq!(error.invariant, Invariant::Targets);
    assert_eq!(error.to_error().code(), ErrorCode::InternalCorrection);
    assert!(!error.to_string().contains("N-"));
}

#[test]
fn never_replaces_or_adds_a_changing_stage() {
    let tables = load_embedded().unwrap();
    let (before, after) = remove_program("/srv/a", "/srv/a");
    let renamed = [Correction::new(
        RuleId::L01,
        Class::Fuzzy,
        "move",
        "remove",
        CorrectionKind::Command,
    )];
    assert_eq!(
        violated(check(&before, &after, &renamed, None, &tables)),
        Invariant::EffectKind
    );
    let reading = surface(json!([pipeline(&[written(
        "find",
        &[("", text("/srv/a"))]
    )])]));
    assert_eq!(
        violated(check(&reading, &after, &[], None, &tables)),
        Invariant::EffectKind
    );
}

fn write_program(written_mode: Option<&str>, bound_mode: &str) -> (AstDocument, BoundDocument) {
    let mut args = vec![("", text("/srv/a.txt")), ("", text("x"))];
    if let Some(mode) = written_mode {
        args.push(("mode", json!({ "t": "ident", "v": mode })));
    }
    let before = surface(json!([pipeline(&[written("write", &args)])]));
    let after = bound(json!([pipeline(&[resolved(
        "write",
        "local",
        "mutate",
        &[
            ("path", path("/srv/a.txt"), false),
            ("text", text("x"), false),
            ("mode", json!({ "t": "enum", "v": bound_mode }), false),
        ]
    )])]));
    (before, after)
}

#[test]
fn takes_mode_parameters_only_from_the_program_or_a_value_correction() {
    let tables = load_embedded().unwrap();
    let (before, after) = write_program(Some("append"), "append");
    assert_eq!(check(&before, &after, &[], None, &tables), Ok(()));
    let (before, after) = write_program(None, "overwrite");
    assert_eq!(
        violated(check(&before, &after, &[], None, &tables)),
        Invariant::ModeParameters
    );
    let (before, after) = write_program(Some("truncate"), "overwrite");
    let table = [Correction::new(
        RuleId::T09,
        Class::Certain,
        "truncate",
        "overwrite",
        CorrectionKind::Value,
    )];
    assert_eq!(check(&before, &after, &table, None, &tables), Ok(()));
    let (before, after) = write_program(Some("over"), "overwrite");
    let prefix = [Correction::new(
        RuleId::L05,
        Class::Fuzzy,
        "over",
        "overwrite",
        CorrectionKind::Value,
    )];
    assert_eq!(
        violated(check(&before, &after, &prefix, None, &tables)),
        Invariant::ModeParameters
    );
    let unrelated = [Correction::new(
        RuleId::L01,
        Class::Fuzzy,
        "overwirte",
        "overwrite",
        CorrectionKind::Command,
    )];
    assert_eq!(
        violated(check(&before, &after, &unrelated, None, &tables)),
        Invariant::ModeParameters
    );
}

#[test]
fn checks_only_renamed_effects_without_a_surface_tree() {
    let tables = load_embedded().unwrap();
    let (_, after) = remove_program("/srv/a", "/srv/a");
    let input = |corrections| InvariantInput {
        before: None,
        after: &after,
        corrections,
        base: None,
        tables: &tables,
        effecting: &effecting,
    };
    assert_eq!(check_invariants(&input(&[])), Ok(()));
    let renamed = [Correction::new(
        RuleId::L01,
        Class::Fuzzy,
        "copy",
        "remove",
        CorrectionKind::Command,
    )];
    assert_eq!(
        check_invariants(&input(&renamed)).unwrap_err().invariant,
        Invariant::EffectKind
    );
}

#[test]
fn numbers_the_invariants_from_one_to_six() {
    let all = [
        Invariant::LiteralBytes,
        Invariant::Numbers,
        Invariant::Targets,
        Invariant::EffectKind,
        Invariant::ModeParameters,
        Invariant::Base,
    ];
    assert_eq!(all.map(Invariant::number), [1, 2, 3, 4, 5, 6]);
}

#[test]
fn compares_paths_with_the_allowed_rewrites() {
    assert_eq!(
        path_check("/srv//app/", "/srv/app", None),
        PathCheck::Preserved
    );
    assert_eq!(
        path_check("//server/share/", "//server/share", None),
        PathCheck::Preserved
    );
    assert_eq!(path_check("/", "/", None), PathCheck::Preserved);
    assert_eq!(path_check("C:\\", "C:/", None), PathCheck::Preserved);
    assert_eq!(
        path_check("~/notes", "/home/user/notes", None),
        PathCheck::Preserved
    );
    assert_eq!(
        path_check("~/notes", "/home/user/other", None),
        PathCheck::Changed
    );
    assert_eq!(
        path_check("registry:/HKCU/Software", "registry:/HKCU/Software", None),
        PathCheck::Preserved
    );
    assert_eq!(
        path_check(".", "/srv/app", Some("/srv/app")),
        PathCheck::Preserved
    );
    assert_eq!(
        path_check("src", "/srv/other/src", Some("/srv/app")),
        PathCheck::Changed
    );
    assert_eq!(path_check("src", "/srv/app/lib", None), PathCheck::Changed);
}

/// Allowed rewrite of a canonical path.
#[derive(Debug, Clone, Copy)]
enum Rewrite {
    Backslashes,
    DoubledSlash,
    TrailingSlash,
    LowerDrive,
}

fn rewrite(canonical: &str, rewrites: &[Rewrite]) -> String {
    let mut out = canonical.to_owned();
    for rewrite in rewrites {
        out = match rewrite {
            Rewrite::Backslashes => out.replace('/', "\\"),
            Rewrite::DoubledSlash => match out.rfind(['/', '\\']) {
                Some(index) if index > 2 => {
                    let mut doubled = out.clone();
                    doubled.insert(index, '/');
                    doubled
                }
                _ => out,
            },
            Rewrite::TrailingSlash => format!("{out}/"),
            Rewrite::LowerDrive => out.replacen("C:", "c:", 1),
        };
    }
    out
}

fn canonical_path() -> impl Strategy<Value = String> {
    (
        any::<bool>(),
        prop::collection::vec("[A-Za-z0-9 ._-]{1,8}", 1..5),
    )
        .prop_map(|(windows, segments)| {
            let joined = segments.join("/");
            if windows {
                format!("C:/{joined}")
            } else {
                format!("/{joined}")
            }
        })
}

fn rewrites() -> impl Strategy<Value = Vec<Rewrite>> {
    prop::collection::vec(
        prop_oneof![
            Just(Rewrite::Backslashes),
            Just(Rewrite::DoubledSlash),
            Just(Rewrite::TrailingSlash),
            Just(Rewrite::LowerDrive),
        ],
        0..4,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2_000, rng_seed: proptest::test_runner::RngSeed::Fixed(0x5eed), ..ProptestConfig::default() })]

    #[test]
    fn allowed_rewrites_keep_a_path(canonical in canonical_path(), steps in rewrites()) {
        let written = rewrite(&canonical, &steps);
        prop_assert_eq!(path_check(&written, &canonical, None), PathCheck::Preserved, "{}", written);
    }

    #[test]
    fn any_other_byte_change_breaks_a_path(canonical in canonical_path(), position in any::<prop::sample::Index>(), replacement in "[a-zA-Z0-9]") {
        let bytes: Vec<(usize, char)> = canonical
            .char_indices()
            .filter(|(index, c)| *c != '/' && !(canonical.starts_with("C:") && *index < 2))
            .collect();
        let (index, original) = bytes[position.index(bytes.len())];
        let replacement = replacement.chars().next().unwrap();
        prop_assume!(replacement != original);
        let mut changed = canonical.clone();
        changed.replace_range(index..index + original.len_utf8(), &replacement.to_string());
        prop_assert_eq!(path_check(&changed, &canonical, None), PathCheck::Changed, "{}", changed);
    }
}
