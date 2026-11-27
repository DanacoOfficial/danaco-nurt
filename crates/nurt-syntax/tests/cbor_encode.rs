// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/cbor_encode.rs
// Responsibility:   Deterministic CBOR encoding (RFC 8949, section 4.2.1).
// Dependencies:     hash_support, nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Deterministic CBOR encoding (RFC 8949, section 4.2.1).
//!
//! The encodings of all golden trees are checked by an independent walker of
//! the byte stream: definite lengths, shortest arguments, shortest
//! floating-point forms, no tags and map keys in the bytewise order of their
//! encodings.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use hash_support::all_trees;
use nurt_contracts::ast::{AstDocument, Expr, Statement};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_syntax::cbor::{decode_surface, encode, read};
use serde_json::json;

/// Walks one data item at `bytes[*at..]` and checks the deterministic
/// encoding; returns the encoded item.
fn walk(bytes: &[u8], at: &mut usize) -> Vec<u8> {
    let start = *at;
    let initial = bytes[*at];
    *at += 1;
    let major = initial >> 5;
    let info = initial & 0x1f;
    assert!(info != 31, "indefinite length at byte {start}");
    assert!(major != 6, "tag at byte {start}");
    let argument: u64 = match info {
        0..=23 => u64::from(info),
        24..=27 => {
            let size = 1 << (info - 24);
            let mut value = 0_u64;
            for byte in &bytes[*at..*at + size] {
                value = (value << 8) | u64::from(*byte);
            }
            *at += size;
            value
        }
        _ => panic!("reserved additional information at byte {start}"),
    };
    if major == 7 {
        check_simple(info, argument, start);
        return bytes[start..*at].to_vec();
    }
    let shortest = match argument {
        0..=23 => info == u8::try_from(argument).unwrap(),
        24..=0xff => info == 24,
        0x100..=0xffff => info == 25,
        0x1_0000..=0xffff_ffff => info == 26,
        _ => info == 27,
    };
    assert!(
        shortest,
        "argument {argument} not in its shortest form at byte {start}"
    );
    let length = usize::try_from(argument).unwrap();
    match major {
        2 | 3 => *at += length,
        4 => {
            for _ in 0..length {
                walk(bytes, at);
            }
        }
        5 => {
            let mut previous: Option<Vec<u8>> = None;
            for _ in 0..length {
                let key = walk(bytes, at);
                assert_eq!(key[0] >> 5, 3, "key other than text at byte {start}");
                if let Some(previous) = &previous {
                    assert!(previous < &key, "map keys out of order at byte {start}");
                }
                previous = Some(key);
                walk(bytes, at);
            }
        }
        _ => {}
    }
    bytes[start..*at].to_vec()
}

fn check_simple(info: u8, bits: u64, start: usize) {
    match info {
        20..=22 => {}
        25 => {}
        26 => {
            let value = f32::from_bits(u32::try_from(bits).unwrap());
            assert_ne!(
                encode(&f64::from(value)).unwrap()[0],
                HALF_FLOAT,
                "32-bit float {value} fits 16 bits at byte {start}"
            );
        }
        27 => {
            let value = f64::from_bits(bits);
            #[allow(clippy::cast_possible_truncation)] // The truncation is the property tested.
            let narrowed = value as f32;
            assert!(
                f64::from(narrowed) != value,
                "64-bit float {value} fits 32 bits at byte {start}"
            );
        }
        _ => panic!("simple value {info} at byte {start}"),
    }
}

/// Initial byte of a 16-bit float.
const HALF_FLOAT: u8 = 0xf9;

fn assert_deterministic(bytes: &[u8]) {
    let mut at = 0;
    walk(bytes, &mut at);
    assert_eq!(at, bytes.len(), "data after the document");
}

#[test]
fn encodes_every_golden_tree_deterministically() {
    for tree in all_trees() {
        let decoded = tree.decode();
        let bytes = decoded.cbor();
        assert_deterministic(&bytes);
        assert_eq!(bytes, decoded.cbor(), "{}", tree.path.display());
        assert_eq!(
            encode(&tree.json()).unwrap(),
            encode(&decoded.to_json()).unwrap(),
            "{}",
            tree.path.display()
        );
    }
}

#[test]
fn encodes_integers_in_their_shortest_form() {
    let cases: [(i64, &[u8]); 12] = [
        (0, &[0x00]),
        (23, &[0x17]),
        (24, &[0x18, 0x18]),
        (255, &[0x18, 0xff]),
        (256, &[0x19, 0x01, 0x00]),
        (65_535, &[0x19, 0xff, 0xff]),
        (65_536, &[0x1a, 0x00, 0x01, 0x00, 0x00]),
        (
            1 << 32,
            &[0x1b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        ),
        (-1, &[0x20]),
        (-24, &[0x37]),
        (-25, &[0x38, 0x18]),
        (-256, &[0x38, 0xff]),
    ];
    for (value, expected) in cases {
        assert_eq!(encode(&value).unwrap(), expected, "{value}");
    }
}

#[test]
fn encodes_floats_in_the_shortest_exact_form() {
    let cases: [(f64, &[u8]); 8] = [
        (3.0, &[0xf9, 0x42, 0x00]),
        (0.5, &[0xf9, 0x38, 0x00]),
        (-0.0, &[0xf9, 0x00, 0x00]),
        (65_504.0, &[0xf9, 0x7b, 0xff]),
        (100_000.0, &[0xfa, 0x47, 0xc3, 0x50, 0x00]),
        (1.1, &[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a]),
        (5.960_464_477_539_063e-8, &[0xf9, 0x00, 0x01]),
        (
            1e300,
            &[0xfb, 0x7e, 0x37, 0xe4, 0x3c, 0x88, 0x00, 0x75, 0x9c],
        ),
    ];
    for (value, expected) in cases {
        assert_eq!(encode(&value).unwrap(), expected, "{value}");
    }
}

#[test]
fn sorts_map_keys_by_their_encoding() {
    let value = json!({"b": 1, "aa": 3, "a": 2});
    assert_eq!(
        encode(&value).unwrap(),
        [
            0xa3, 0x61, b'a', 0x02, 0x61, b'b', 0x01, 0x62, b'a', b'a', 0x03
        ]
    );
    let long = "x".repeat(24);
    let value = json!({long.clone(): 1, "y": 2});
    let mut expected = vec![0xa2, 0x61, b'y', 0x02, 0x78, 0x18];
    expected.extend(long.bytes());
    expected.push(0x01);
    assert_eq!(encode(&value).unwrap(), expected);
}

#[test]
fn an_integral_num_node_stays_a_float() {
    let document: AstDocument = serde_json::from_value(json!({"nurt": "1.0", "program": {
        "statements": [{"t": "pipeline", "stages": [],
            "head": {"t": "stage", "name": "first", "args": [
                {"value": {"t": "num", "v": 3.0}}, {"value": {"t": "int", "v": 3}}]}}]}}))
    .unwrap();
    let bytes = encode(&document).unwrap();
    assert_deterministic(&bytes);
    let value = read(&bytes).unwrap();
    let args = &value["program"]["statements"][0]["head"]["args"];
    assert!(args[0]["value"]["v"].is_f64());
    assert!(args[1]["value"]["v"].is_u64());
    let decoded = decode_surface(&bytes).unwrap();
    assert_eq!(decoded, document);
    let Statement::Pipeline(pipeline) = &decoded.program.statements[0] else {
        panic!("not a pipeline");
    };
    let nurt_contracts::ast::Head::Stage(stage) = &pipeline.head else {
        panic!("not a stage");
    };
    assert!(matches!(stage.args[0].value, Expr::Num { v, .. } if v == 3.0));
    assert!(matches!(stage.args[1].value, Expr::Int { v: 3, .. }));
}

fn rejected(result: Result<Vec<u8>, nurt_syntax::error::SyntaxError>) -> String {
    let error = result.unwrap_err();
    let message = error.message.clone();
    assert_eq!(Error::from(error).code(), ErrorCode::AstSchema);
    message
}

#[test]
fn rejects_values_outside_the_json_data_model() {
    let safe = (1_i64 << 53) - 1;
    assert!(encode(&safe).is_ok());
    assert!(encode(&-safe).is_ok());
    rejected(encode(&(safe + 1)));
    rejected(encode(&-(safe + 1)));
    rejected(encode(&u64::MAX));
    rejected(encode(&f64::NAN));
    rejected(encode(&f64::INFINITY));
    rejected(encode(&f64::NEG_INFINITY));
    rejected(encode(&ciborium::Value::Bytes(vec![1, 2])));
    rejected(encode(&ciborium::Value::Tag(
        1,
        Box::new(ciborium::Value::Null),
    )));
    rejected(encode(&ciborium::Value::Map(vec![(
        ciborium::Value::Integer(1.into()),
        ciborium::Value::Null,
    )])));
    let duplicate = ciborium::Value::Map(vec![
        (ciborium::Value::Text("a".into()), ciborium::Value::Null),
        (
            ciborium::Value::Text("a".into()),
            ciborium::Value::Bool(true),
        ),
    ]);
    assert!(rejected(encode(&duplicate)).contains("duplicate"));
}

#[test]
fn rejects_nesting_deeper_than_the_limit() {
    let mut value = json!(null);
    for _ in 0..127 {
        value = json!([value]);
    }
    assert!(encode(&value).is_ok());
    let value = json!([value]);
    assert!(rejected(encode(&value)).contains("nesting"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
