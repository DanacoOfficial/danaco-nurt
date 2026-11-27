// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/json_kinds.rs
// Responsibility:   The JSON reader recognises exactly the node kinds that `nurt-contracts` declares, so that a newer-version error never blames a known kind.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The JSON reader recognises exactly the node kinds that `nurt-contracts`
//! declares, so that a newer-version error never blames a known kind.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::ast::SURFACE_KINDS;
use nurt_contracts::bound::BOUND_KINDS;
use nurt_syntax::json::{decode_bound, decode_surface};
use serde_json::json;

fn newer_surface(kind: &str) -> String {
    json!({"nurt": "1.99", "program": {"statements": [{"t": kind}]}}).to_string()
}

fn newer_bound(kind: &str) -> String {
    json!({"nurt": "1.0", "bound": "1.99", "program": {"statements": [{"t": kind}]}}).to_string()
}

#[test]
fn surface_reader_knows_every_declared_kind() {
    for kind in SURFACE_KINDS {
        let error = decode_surface(&newer_surface(kind)).unwrap_err();
        assert!(error.data().expected.is_empty(), "kind {kind}: {error}");
    }
    let error = decode_surface(&newer_surface("future")).unwrap_err();
    assert_eq!(error.data().expected, ["future"]);
}

#[test]
fn bound_reader_knows_every_declared_kind() {
    for kind in BOUND_KINDS {
        let error = decode_bound(&newer_bound(kind)).unwrap_err();
        assert!(error.data().expected.is_empty(), "kind {kind}: {error}");
    }
    let error = decode_bound(&newer_bound("future")).unwrap_err();
    assert_eq!(error.data().expected, ["future"]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
