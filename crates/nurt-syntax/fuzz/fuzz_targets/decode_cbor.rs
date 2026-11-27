// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/fuzz/fuzz_targets/decode_cbor.rs
// Responsibility:   Fuzz target of the CBOR decoders.
// Dependencies:     libfuzzer_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fuzz target of the CBOR decoders.
//!
//! No input may panic. A decoded tree encodes again to the same tree and has
//! a hash.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nurt_syntax::cbor::{decode_bound, decode_surface, encode, read};
use nurt_syntax::hash::ast_hash;

fuzz_target!(|data: &[u8]| {
    let _ = read(data);
    if let Ok(document) = decode_surface(data) {
        let bytes = encode(&document).expect("a decoded surface tree encodes");
        assert_eq!(decode_surface(&bytes).ok(), Some(document.clone()));
        ast_hash(&document).expect("a decoded surface tree has a hash");
    }
    if let Ok(document) = decode_bound(data) {
        let bytes = encode(&document).expect("a decoded bound tree encodes");
        assert_eq!(decode_bound(&bytes).ok(), Some(document.clone()));
        ast_hash(&document).expect("a decoded bound tree has a hash");
    }
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
