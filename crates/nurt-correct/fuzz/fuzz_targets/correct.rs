// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-correct/fuzz/fuzz_targets/correct.rs
// Responsibility:   Fuzz target of the front part of correction.
// Dependencies:     libfuzzer_sys, nurt_contracts, nurt_syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fuzz target of the front part of correction: any program text, read in
//! every syntax variant with a small command catalog, must never panic, must
//! give at most the allowed number of alternatives, and the corrected tree,
//! printed and read again, must need no further correction.

#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use nurt_contracts::config::{CorrectionConfig, SyntaxConfig};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_correct::lexical::front::MAX_ALTERNATIVES;
use nurt_correct::lexical::{ArgumentShape, CatalogView, front};
use nurt_correct::tables::{Tables, load_embedded};
use nurt_syntax::print::try_print;

/// Commands known to the catalog of the target.
const COMMANDS: [&str; 14] = [
    "find", "search", "list", "read", "count", "first", "filter", "sort", "select", "replace",
    "remove", "run", "verify", "jobs",
];

/// Commands of the catalog that change the system.
const MUTATING: [&str; 3] = ["replace", "remove", "run"];

/// Commands that take a bare word as a positional argument.
const WORD_TAKING: [&str; 3] = ["filter", "sort", "select"];

/// Longest input read, in bytes; longer inputs test the parser, not the
/// correction rules.
const MAX_INPUT: usize = 4096;

struct Commands;

impl CatalogView for Commands {
    fn is_command(&self, name: &str) -> bool {
        COMMANDS.contains(&name)
    }

    fn command_names(&self) -> &[&str] {
        &COMMANDS
    }

    fn is_mutating(&self, name: &str) -> bool {
        MUTATING.contains(&name)
    }

    fn can_take_argument(&self, command: &str, _: usize, shape: ArgumentShape) -> bool {
        shape == ArgumentShape::Word && WORD_TAKING.contains(&command)
    }
}

fn tables() -> Option<&'static Tables> {
    static TABLES: OnceLock<Option<Tables>> = OnceLock::new();
    TABLES.get_or_init(|| load_embedded().ok()).as_ref()
}

fn syntax(variant: SyntaxVariant) -> SyntaxConfig {
    let mut config = SyntaxConfig::default();
    config.variant = variant;
    config
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Some(tables) = tables() else {
        return;
    };
    if text.len() > MAX_INPUT {
        return;
    }
    let correction = CorrectionConfig::default();
    for variant in [SyntaxVariant::A, SyntaxVariant::B, SyntaxVariant::C] {
        let syntax = syntax(variant);
        let Ok(result) = front(text, &syntax, &correction, tables, &Commands) else {
            continue;
        };
        assert!(result.alternatives.len() <= MAX_ALTERNATIVES);
        let Ok(printed) = try_print(&result.document, variant) else {
            continue;
        };
        let again = front(&printed, &syntax, &correction, tables, &Commands);
        assert!(again.is_ok_and(|again| again.corrections.is_empty()));
    }
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
