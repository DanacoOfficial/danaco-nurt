// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/hash_properties.rs
// Responsibility:   Properties of `ast_hash` on the golden trees and the number formatting of the JSON Canonicalization Scheme (RFC 8785) used by the hash.
// Dependencies:     hash_support, nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Properties of `ast_hash` on the golden trees and the number formatting of
//! the JSON Canonicalization Scheme (RFC 8785) used by the hash.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use hash_support::{Format, all_trees, decode_json, without_layout};
use nurt_contracts::ast::AstDocument;
use nurt_syntax::hash::{ast_hash, hash_input};
use serde_json::{Map, Number, Value, json};

fn hash_of_value(format: Format, value: &Value) -> String {
    decode_json(format, &value.to_string())
        .unwrap_or_else(|error| panic!("{error}: {value}"))
        .hash()
}

#[test]
fn spans_and_metadata_do_not_change_the_hash() {
    for tree in all_trees() {
        let value = tree.json();
        let stripped = without_layout(&value);
        assert_ne!(
            tree.text().contains("\"span\"") || tree.text().contains("\"meta\""),
            value == stripped,
            "{}",
            tree.path.display()
        );
        assert_eq!(
            hash_of_value(tree.format, &value),
            hash_of_value(tree.format, &stripped),
            "{}",
            tree.path.display()
        );
    }
}

fn reversed_members(value: &Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .rev()
                .map(|(name, member)| (name.clone(), reversed_members(member)))
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(reversed_members).collect()),
        other => other.clone(),
    }
}

#[test]
fn member_order_does_not_change_the_hash() {
    for tree in all_trees() {
        let value = tree.json();
        assert_eq!(
            hash_of_value(tree.format, &value),
            hash_of_value(tree.format, &reversed_members(&value)),
            "{}",
            tree.path.display()
        );
    }
}

/// Writes every member that the typed tree omits at its default value:
/// `fields: []` of a reference, `waits: false` of a bound stage and
/// `default: false` of a bound argument.
fn with_explicit_defaults(value: &mut Value, format: Format) -> usize {
    let mut added = 0;
    let mut pending = vec![value];
    while let Some(current) = pending.pop() {
        match current {
            Value::Object(members) => {
                let kind = members.get("t").and_then(Value::as_str).map(str::to_owned);
                match kind.as_deref() {
                    Some("ref") if !members.contains_key("fields") => {
                        members.insert("fields".to_owned(), json!([]));
                        added += 1;
                    }
                    Some("stage") if format == Format::Bound && !members.contains_key("waits") => {
                        members.insert("waits".to_owned(), json!(false));
                        added += 1;
                    }
                    None if format == Format::Bound
                        && members.contains_key("name")
                        && members.contains_key("value")
                        && !members.contains_key("default") =>
                    {
                        members.insert("default".to_owned(), json!(false));
                        added += 1;
                    }
                    _ => {}
                }
                pending.extend(members.values_mut());
            }
            Value::Array(items) => pending.extend(items.iter_mut()),
            _ => {}
        }
    }
    added
}

#[test]
fn members_written_with_their_default_value_do_not_change_the_hash() {
    let mut normalized = 0;
    for tree in all_trees() {
        let value = tree.json();
        let mut explicit = value.clone();
        normalized += with_explicit_defaults(&mut explicit, tree.format);
        assert_eq!(
            hash_of_value(tree.format, &value),
            hash_of_value(tree.format, &explicit),
            "{}",
            tree.path.display()
        );
    }
    assert!(normalized > 0, "no golden tree exercises the normalization");
}

#[test]
fn hash_is_stable_across_decoding_and_encoding() {
    for tree in all_trees() {
        let decoded = tree.decode();
        let again = decode_json(tree.format, &decoded.to_json().to_string())
            .unwrap_or_else(|error| panic!("{}: {error}", tree.path.display()));
        assert_eq!(decoded.hash(), again.hash(), "{}", tree.path.display());
        assert_eq!(decoded.hash(), decoded.hash(), "{}", tree.path.display());
    }
}

#[test]
fn hash_input_is_the_canonical_json_of_the_hash_form() {
    let document: AstDocument = serde_json::from_value(json!({
        "nurt": "1.0",
        "meta": {"source": "first 2.5"},
        "program": {"statements": [{
            "t": "pipeline",
            "span": [1, 1, 1, 10],
            "stages": [],
            "head": {"t": "stage", "name": "first", "span": [1, 1, 1, 10], "args": [
                {"span": [1, 7, 1, 10], "value": {"t": "num", "v": 2.5, "span": [1, 7, 1, 10]}}
            ]}
        }]}
    }))
    .unwrap();
    let input = hash_input(&document).unwrap();
    assert_eq!(
        input,
        r#"{"nurt":"1.0","program":{"statements":[{"head":{"args":[{"value":{"t":"num","v":2.5}}],"name":"first","t":"stage"},"stages":[],"t":"pipeline"}]}}"#
    );
    let digest = ast_hash(&document).unwrap();
    assert!(digest.starts_with("sha256:"));
    assert_eq!(digest.len(), "sha256:".len() + 64);
}

fn canonical_number(bits: u64) -> String {
    let number = Number::from_f64(f64::from_bits(bits)).unwrap();
    serde_json_canonicalizer::to_string(&Value::Number(number)).unwrap()
}

/// Number serialization samples of RFC 8785, appendix B: IEEE 754 bit
/// patterns and their canonical text.
const RFC_8785_NUMBERS: [(u64, &str); 24] = [
    (0x0000_0000_0000_0000, "0"),
    (0x8000_0000_0000_0000, "0"),
    (0x0000_0000_0000_0001, "5e-324"),
    (0x8000_0000_0000_0001, "-5e-324"),
    (0x7fef_ffff_ffff_ffff, "1.7976931348623157e+308"),
    (0xffef_ffff_ffff_ffff, "-1.7976931348623157e+308"),
    (0x4340_0000_0000_0000, "9007199254740992"),
    (0xc340_0000_0000_0000, "-9007199254740992"),
    (0x4430_0000_0000_0000, "295147905179352830000"),
    (0x44b5_2d02_c7e1_4af5, "9.999999999999997e+22"),
    (0x44b5_2d02_c7e1_4af6, "1e+23"),
    (0x44b5_2d02_c7e1_4af7, "1.0000000000000001e+23"),
    (0x444b_1ae4_d6e2_ef4e, "999999999999999700000"),
    (0x444b_1ae4_d6e2_ef4f, "999999999999999900000"),
    (0x444b_1ae4_d6e2_ef50, "1e+21"),
    (0x3eb0_c6f7_a0b5_ed8c, "9.999999999999997e-7"),
    (0x3eb0_c6f7_a0b5_ed8d, "0.000001"),
    (0x41b3_de43_5555_5553, "333333333.3333332"),
    (0x41b3_de43_5555_5554, "333333333.33333325"),
    (0x41b3_de43_5555_5555, "333333333.3333333"),
    (0x41b3_de43_5555_5556, "333333333.3333334"),
    (0x41b3_de43_5555_5557, "333333333.33333343"),
    (0xbecb_f647_612f_3696, "-0.0000033333333333333333"),
    (0x4314_3ff3_c1cb_0959, "1424953923781206.2"),
];

#[test]
fn canonical_numbers_follow_rfc_8785_appendix_b() {
    for (bits, expected) in RFC_8785_NUMBERS {
        assert_eq!(canonical_number(bits), expected, "{bits:#018x}");
    }
}

#[test]
fn canonical_numbers_follow_the_ecmascript_form() {
    let samples = [
        (1e30, "1e+30"),
        (4.5, "4.5"),
        (0.002, "0.002"),
        (1e-7, "1e-7"),
        (1e-27, "1e-27"),
        (1e21, "1e+21"),
        (1e20, "100000000000000000000"),
        (333_333_333.333_333_3, "333333333.3333333"),
        (3.0, "3"),
        (-0.0, "0"),
    ];
    for (number, expected) in samples {
        assert_eq!(canonical_number(f64::to_bits(number)), expected, "{number}");
    }
    let integers = json!([9_007_199_254_740_991_i64, -9_007_199_254_740_991_i64, 0]);
    assert_eq!(
        serde_json_canonicalizer::to_string(&integers).unwrap(),
        "[9007199254740991,-9007199254740991,0]"
    );
}

#[test]
fn canonical_json_sorts_members_by_utf16_code_units() {
    let value = json!({"\u{20ac}": 1, "\r": 2, "\u{fb33}": 3, "1": 4, "\u{1f600}": 5, "\u{80}": 6, "\u{f6}": 7});
    assert_eq!(
        serde_json_canonicalizer::to_string(&value).unwrap(),
        "{\"\\r\":2,\"1\":4,\"\u{80}\":6,\"\u{f6}\":7,\"\u{20ac}\":1,\"\u{1f600}\":5,\"\u{fb33}\":3}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
