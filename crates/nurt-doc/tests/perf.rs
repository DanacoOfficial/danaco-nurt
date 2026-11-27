// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/perf.rs
// Responsibility:   Wall-clock limits of reading: a rejected entity expansion attack must fail.
// Dependencies:     nurt_contracts, u08_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Wall-clock limits of reading: a rejected entity expansion attack must fail
//! within [`ATTACK_DEADLINE`]. The binary is named `perf` so that the filter
//! `not binary(perf)` of `cargo nextest` skips it on loaded machines. The error
//! codes of the same attacks are checked by `u08_xml_entities`.
// Helper functions of this test target are not `#[test]` functions, which
// `allow-unwrap-in-tests` covers.
#![allow(clippy::unwrap_used)]

mod u08_support;

use std::time::{Duration, Instant};

use nurt_contracts::error::ErrorCode;
use nurt_doc::formats::xml::encoding::parse_document;
use nurt_doc::formats::xml::entity::{MAX_ENTITY_DEPTH, MAX_EXPANSION_BYTES};
use u08_support::Codec;

/// Time within which a rejected expansion attack must fail.
const ATTACK_DEADLINE: Duration = Duration::from_secs(5);

fn laughs(levels: usize, fan_out: usize) -> String {
    let mut subset = String::from("<!ENTITY l0 \"lol\">");
    for level in 1..levels {
        let refs = format!("&l{};", level - 1).repeat(fan_out);
        subset.push_str(&format!("<!ENTITY l{level} \"{refs}\">"));
    }
    format!("<!DOCTYPE a [{subset}]><a>&l{};</a>", levels - 1)
}

/// Parses `text` and expects the limit error within [`ATTACK_DEADLINE`].
fn rejected_within_deadline(text: &str) {
    let started = Instant::now();
    let error = parse_document(text.as_bytes(), Some(&Codec)).unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.code(), ErrorCode::Limit, "{error}");
    assert!(elapsed < ATTACK_DEADLINE, "the attack took {elapsed:?}");
}

#[test]
fn billion_laughs_fails_within_the_deadline() {
    rejected_within_deadline(&laughs(10, 10));
}

#[test]
fn exponential_expansion_within_the_depth_limit_fails_within_the_deadline() {
    rejected_within_deadline(&laughs(MAX_ENTITY_DEPTH, 10));
}

#[test]
fn quadratic_blowup_fails_within_the_deadline() {
    let block = "x".repeat(64 * 1024);
    let refs = "&big;".repeat(MAX_EXPANSION_BYTES / block.len() + 1);
    rejected_within_deadline(&format!(
        r#"<!DOCTYPE a [<!ENTITY big "{block}">]><a>{refs}</a>"#
    ));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
