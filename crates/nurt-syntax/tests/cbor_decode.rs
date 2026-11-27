// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/cbor_decode.rs
// Responsibility:   Decoding of CBOR: the JSON data model, the rejected items, the nesting limit, length headers and the strict decoders.
// Dependencies:     hash_support, nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Decoding of CBOR: the JSON data model, the rejected items, the nesting
//! limit, length headers and the strict decoders.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod hash_support;

use hash_support::json_depth;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_syntax::cbor::{
    MAX_DEPTH, decode_bound, decode_surface, decode_surface_strict, encode, read,
};
use serde_json::{Value, json};

/// Asserts that the input is rejected with `E_AST_SCHEMA` at the byte offset
/// and returns the message.
fn rejected_at(bytes: &[u8], offset: usize) -> String {
    let error = read(bytes)
        .err()
        .unwrap_or_else(|| panic!("accepted {bytes:02x?}"));
    assert_eq!(
        error.found.as_deref(),
        Some(format!("byte {offset}").as_str()),
        "{bytes:02x?}"
    );
    let message = error.message.clone();
    assert_eq!(Error::from(error).code(), ErrorCode::AstSchema);
    message
}

#[test]
fn reads_the_json_data_model() {
    let bytes = [
        0xa6, 0x61, b'a', 0x01, 0x61, b'b', 0x20, 0x61, b'c', 0xf9, 0x3e, 0x00, 0x61, b'd', 0x82,
        0xf5, 0xf4, 0x61, b'e', 0xf6, 0x61, b'f', 0x62, 0xc3, 0xb3,
    ];
    assert_eq!(
        read(&bytes).unwrap(),
        json!({"a": 1, "b": -1, "c": 1.5, "d": [true, false], "e": null, "f": "ó"})
    );
    assert_eq!(
        read(&[0xfa, 0x47, 0xc3, 0x50, 0x00]).unwrap(),
        json!(100_000.0)
    );
    assert_eq!(
        read(&[0xf9, 0x00, 0x01]).unwrap(),
        json!(5.960_464_477_539_063e-8)
    );
    assert_eq!(read(&[0xf9, 0x80, 0x00]).unwrap(), json!(-0.0));
    assert_eq!(
        read(&[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a]).unwrap(),
        json!(1.1)
    );
}

#[test]
fn reads_integers_within_the_safe_range() {
    let max = [0x1b, 0x00, 0x1f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(read(&max).unwrap(), json!(9_007_199_254_740_991_i64));
    let min = [0x3b, 0x00, 0x1f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe];
    assert_eq!(read(&min).unwrap(), json!(-9_007_199_254_740_991_i64));
    rejected_at(&[0x1b, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], 0);
    rejected_at(&[0x3b, 0x00, 0x1f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 0);
    rejected_at(&[0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 0);
    rejected_at(&[0x3b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 0);
}

#[test]
fn rejects_items_outside_the_json_data_model() {
    assert!(rejected_at(&[0xc0, 0x60], 0).contains("tag"));
    assert!(rejected_at(&[0xd8, 0x20, 0x60], 0).contains("tag"));
    assert!(rejected_at(&[0x81, 0x40], 1).contains("byte string"));
    assert!(rejected_at(&[0xf7], 0).contains("undefined"));
    rejected_at(&[0xf0], 0);
    rejected_at(&[0xf8, 0x20], 0);
    assert!(rejected_at(&[0xa1, 0x01, 0x02], 1).contains("text"));
    assert!(rejected_at(&[0xa1, 0x81, 0x01, 0x02], 1).contains("text"));
    let duplicate = [0xa2, 0x61, b'a', 0x01, 0x61, b'a', 0x02];
    assert!(rejected_at(&duplicate, 4).contains("duplicate"));
    for nan in [
        &[0xf9, 0x7e, 0x00][..],
        &[0xf9, 0x7c, 0x00],
        &[0xf9, 0xfc, 0x00],
        &[0xfa, 0x7f, 0xc0, 0x00, 0x00],
        &[0xfb, 0x7f, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    ] {
        assert!(rejected_at(nan, 0).contains("finite"));
    }
}

#[test]
fn rejects_malformed_input() {
    assert!(rejected_at(&[0x01, 0x02], 1).contains("after the document"));
    assert!(rejected_at(&[], 0).contains("truncated"));
    rejected_at(&[0x19, 0x01], 1);
    assert!(rejected_at(&[0x82, 0x01], 0).contains("exceeds"));
    assert!(rejected_at(&[0x1c], 0).contains("well-formed"));
    assert!(rejected_at(&[0xff], 0).contains("break"));
    assert!(rejected_at(&[0x1f], 0).contains("indefinite"));
    assert!(rejected_at(&[0x62, 0xc3, 0x28], 0).contains("UTF-8"));
    assert!(rejected_at(&[0x7f, 0x41, 0x61, 0xff], 1).contains("chunk"));
    assert!(rejected_at(&[0xbf, 0x61, b'a', 0xff], 3).contains("break"));
    assert!(rejected_at(&[0x82, 0x01, 0xff], 2).contains("break"));
}

#[test]
fn reads_indefinite_lengths() {
    assert_eq!(
        read(&[0x9f, 0x01, 0x9f, 0xff, 0xff]).unwrap(),
        json!([1, []])
    );
    assert_eq!(
        read(&[
            0xbf, 0x61, b'a', 0x7f, 0x61, b'x', 0x62, b'y', b'z', 0xff, 0xff
        ])
        .unwrap(),
        json!({"a": "xyz"})
    );
}

fn nested_arrays(depth: usize) -> Vec<u8> {
    let mut bytes = vec![0x81; depth - 1];
    bytes.push(0x80);
    bytes
}

#[test]
fn accepts_depth_128_and_rejects_depth_129() {
    let value = read(&nested_arrays(MAX_DEPTH)).unwrap();
    assert_eq!(json_depth(&value), MAX_DEPTH);
    assert!(rejected_at(&nested_arrays(MAX_DEPTH + 1), MAX_DEPTH).contains("nesting"));
    let mut scalar = vec![0x81; MAX_DEPTH];
    scalar.push(0x01);
    assert!(rejected_at(&scalar, MAX_DEPTH).contains("nesting"));
    let mut maps = Vec::new();
    for _ in 0..MAX_DEPTH {
        maps.extend([0xa1, 0x61, b'k']);
    }
    maps.push(0xf6);
    assert!(rejected_at(&maps, 3 * MAX_DEPTH).contains("nesting"));
}

/// Surface tree whose deepest value lies at the given depth.
fn nested_tree(depth: usize) -> Value {
    let wrap = |expr: Value| {
        json!({"nurt": "1.0", "program": {"statements": [{"t": "pipeline", "stages": [],
            "head": {"t": "stage", "name": "filter", "args": [{"value": expr}]}}]}})
    };
    let base = json_depth(&wrap(json!({"t": "null"})));
    let mut expr = json!({"t": "null"});
    for _ in base..depth {
        expr = json!({"t": "un", "op": "not", "x": expr});
    }
    let tree = wrap(expr);
    assert_eq!(json_depth(&tree), depth);
    tree
}

#[test]
fn decodes_a_tree_of_depth_128_and_rejects_depth_129() {
    let tree = nested_tree(MAX_DEPTH);
    let bytes = encode(&tree).unwrap();
    assert_eq!(
        serde_json::to_value(decode_surface(&bytes).unwrap()).unwrap(),
        tree
    );
    let deeper = nested_tree(MAX_DEPTH + 1);
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&deeper, &mut bytes).unwrap();
    let error = decode_surface(&bytes).unwrap_err();
    assert!(error.message.contains("nesting"), "{error}");
}

#[test]
fn rejects_length_headers_beyond_the_input_without_allocating() {
    let headers: [&[u8]; 9] = [
        &[0x7a, 0xff, 0xff, 0xff, 0xff],
        &[0x7b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[0x7b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        &[0x9a, 0xff, 0xff, 0xff, 0xff],
        &[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[0x9b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        &[0xba, 0xff, 0xff, 0xff, 0xff],
        &[0xbb, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        &[0xbb, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    ];
    for header in headers {
        assert!(rejected_at(header, 0).contains("exceeds"), "{header:02x?}");
        let mut nested = vec![0xa1, 0x61, b'k'];
        nested.extend(header);
        nested.extend([0x00; 16]);
        assert!(rejected_at(&nested, 3).contains("exceeds"), "{header:02x?}");
    }
    assert!(rejected_at(&[0xa1, 0x61], 0).contains("exceeds"));
}

fn small_tree() -> Value {
    json!({"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
        "head": {"t": "stage", "name": "first", "args": [{"value": {"t": "int", "v": 20}}]},
        "stages": []}]}})
}

#[test]
fn strict_decoding_requires_the_deterministic_encoding() {
    let bytes = encode(&small_tree()).unwrap();
    assert_eq!(
        decode_surface_strict(&bytes).unwrap(),
        decode_surface(&bytes).unwrap()
    );
    let mut indefinite = bytes.clone();
    assert_eq!(indefinite[0], 0xa2);
    indefinite[0] = 0xbf;
    indefinite.push(0xff);
    assert_eq!(
        decode_surface(&indefinite).unwrap(),
        decode_surface(&bytes).unwrap()
    );
    let error = decode_surface_strict(&indefinite).unwrap_err();
    assert_eq!(error.found.as_deref(), Some("byte 0"));
    let position = bytes.windows(2).position(|pair| pair == [0x61, b'v']);
    let position = position.unwrap() + 2;
    assert_eq!(bytes[position], 0x14);
    let mut long_integer = bytes[..position].to_vec();
    long_integer.extend([0x18, 0x14]);
    long_integer.extend(&bytes[position + 1..]);
    assert!(decode_surface(&long_integer).is_ok());
    let error = decode_surface_strict(&long_integer).unwrap_err();
    assert_eq!(error.found, Some(format!("byte {position}")));
    let mut unsorted = small_tree();
    unsorted["meta"] = json!({"b": 1, "a": 2});
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&unsorted, &mut bytes).unwrap();
    assert!(decode_surface(&bytes).is_ok());
    assert!(decode_surface_strict(&bytes).is_err());
    let mut truncated = encode(&small_tree()).unwrap();
    truncated.push(0x00);
    assert!(decode_surface_strict(&truncated).is_err());
}

#[test]
fn decoders_reject_documents_of_the_other_kind_and_unknown_versions() {
    let bytes = encode(&small_tree()).unwrap();
    assert_eq!(
        Error::from(decode_bound(&bytes).unwrap_err()).code(),
        ErrorCode::AstSchema
    );
    let mut newer = small_tree();
    newer["nurt"] = json!("1.9");
    let error = decode_surface(&encode(&newer).unwrap()).unwrap_err();
    assert_eq!(Error::from(error).code(), ErrorCode::AstVersion);
    let error = decode_surface(&encode(&json!([1])).unwrap()).unwrap_err();
    assert_eq!(Error::from(error).code(), ErrorCode::AstSchema);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
