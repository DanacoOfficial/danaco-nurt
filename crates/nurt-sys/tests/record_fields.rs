// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sys/tests/record_fields.rs
// Responsibility:   Agreement of the fields set on typed records in src/cim/invoke.rs with the fields of their record kinds.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Agreement of the fields set on typed records in `src/cim/invoke.rs` with the fields of
//! the record kinds `effect`. The binder checks the fields of a pipeline against
//! the typed kinds, so a field set here and not declared would be refused by
//! `select`, `filter`, `sort` and `evaluate` before the program runs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

use nurt_contracts::record::TypedRecord;
use nurt_contracts::value::RecordKind;

const SOURCE: &str = include_str!("../src/cim/invoke.rs");
const KINDS: &[RecordKind] = &[RecordKind::Effect];

/// Names of the fields set with `record.set("name", …)` in the source.
fn set_fields(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in source.split("record.set(").skip(1) {
        let Some(rest) = part.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            let name = rest[..end].to_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

#[test]
fn reads_the_fields_the_source_sets() {
    let names = set_fields(SOURCE);
    assert!(!names.is_empty(), "no field set in the source");
    assert!(names.iter().any(|name| name == "return_value"));
}

#[test]
fn declares_every_field_the_source_sets() {
    let known: Vec<&str> = KINDS
        .iter()
        .flat_map(|kind| TypedRecord::field_types(*kind).unwrap().iter())
        .map(|(name, _)| *name)
        .collect();
    let missing: Vec<String> = set_fields(SOURCE)
        .into_iter()
        .filter(|name| !known.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "fields set in the source and not declared by the record kinds: {missing:?}"
    );
}
