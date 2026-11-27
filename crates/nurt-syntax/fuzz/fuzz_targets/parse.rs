// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-syntax/fuzz/fuzz_targets/parse.rs
// Responsibility:   Fuzz target of the parsers: program text in every variant, parsed.
// Dependencies:     libfuzzer_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fuzz target of the parsers: program text in every variant, parsed
//! strictly and tolerantly, must never panic, and a tree accepted strictly
//! must be accepted tolerantly unchanged.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nurt_syntax::parse::{NoRepair, ParseOptions, Variant, parse, parse_tolerant};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let options = ParseOptions::default();
    for variant in [Variant::A, Variant::B, Variant::C] {
        let strict = parse(text, variant, &options);
        let tolerant = parse_tolerant(text, variant, &options, &NoRepair);
        if let Ok(document) = strict {
            let repaired = tolerant.ok().map(|repaired| repaired.document);
            assert_eq!(repaired.as_ref(), Some(&document));
            assert!(serde_json::to_string(&document).is_ok());
        }
    }
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
