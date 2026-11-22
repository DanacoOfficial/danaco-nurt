// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fuzz target of the JSON decoders.
//!
//! No input may panic. A decoded tree that passes validation encodes to CBOR,
//! decodes back to the same tree and has a hash.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nurt_syntax::cbor;
use nurt_syntax::hash::ast_hash;
use nurt_syntax::json::{decode_bound, decode_surface};
use nurt_syntax::validate::{validate_bound, validate_surface};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(document) = decode_surface(text)
        && validate_surface(&document).is_ok()
    {
        let bytes = cbor::encode(&document).expect("a decoded surface tree encodes");
        assert_eq!(cbor::decode_surface(&bytes).ok(), Some(document.clone()));
        ast_hash(&document).expect("a decoded surface tree has a hash");
    }
    if let Ok(document) = decode_bound(text)
        && validate_bound(&document).is_ok()
    {
        let bytes = cbor::encode(&document).expect("a decoded bound tree encodes");
        assert_eq!(cbor::decode_bound(&bytes).ok(), Some(document.clone()));
        ast_hash(&document).expect("a decoded bound tree has a hash");
    }
});
