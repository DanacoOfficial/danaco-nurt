// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/tests/parse_depth_limit.rs
// Responsibility:   The depth limit of the parser and of the surface decoders agree: a program the parser accepts always encodes to a document the decoders accept.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The depth limit of the parser and of the surface decoders agree: a program
//! the parser accepts always encodes to a document the decoders accept.

use nurt_contracts::error::ErrorCode;
use nurt_syntax::json::{decode_surface, encode};
use nurt_syntax::parse::{ParseOptions, Variant, parse};

fn nested_lists(depth: usize) -> String {
    format!(
        "filter condition={}x{}",
        "[".repeat(depth),
        "]".repeat(depth)
    )
}

#[test]
fn every_program_the_parser_accepts_encodes_and_decodes() {
    let mut accepted = 0;
    let mut rejected = 0;
    for depth in 1..=140 {
        let text = nested_lists(depth);
        match parse(&text, Variant::C, &ParseOptions::default()) {
            Ok(document) => {
                accepted += 1;
                let json = encode(&document).expect("an accepted program encodes");
                decode_surface(&json)
                    .unwrap_or_else(|error| panic!("depth {depth} does not decode: {error:?}"));
            }
            Err(error) => {
                rejected += 1;
                assert_eq!(error.code, ErrorCode::Syntax, "depth {depth}");
                assert_eq!(error.problem.as_deref(), Some("nesting_too_deep"));
            }
        }
    }
    assert!(accepted > 20, "{accepted} accepted");
    assert!(rejected > 20, "{rejected} rejected");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
