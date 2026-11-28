// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-programs/tests/record_fields.rs
// Responsibility:   Agreement of the fields the `program` stages set on records with the fields of the typed record kinds.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Agreement of the fields the `program` stages set on records with the
//! fields of the typed record kinds. The binder checks the fields of a
//! pipeline against the typed kinds, so a field a stage sets and the kinds do
//! not declare would be refused before the program runs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code fails loudly on broken fixtures"
)]

use nurt_contracts::record::TypedRecord;
use nurt_contracts::value::RecordKind;

const STAGES: &str = include_str!("../src/stages.rs");

/// Names of the fields set with `record.set("name", …)` in the stages,
/// including the calls whose arguments continue on the next line.
fn set_fields(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in source.split("record.set(").skip(1) {
        let rest = part.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
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

fn declared(kind: RecordKind) -> Vec<&'static str> {
    TypedRecord::field_types(kind)
        .unwrap()
        .iter()
        .map(|(name, _)| *name)
        .collect()
}

#[test]
fn reads_every_field_the_stages_set() {
    let names = set_fields(STAGES);
    for expected in ["version_source", "guide_etag", "score", "level", "title"] {
        assert!(names.iter().any(|name| name == expected), "{expected}");
    }
    assert!(names.len() >= 35, "{names:?}");
}

#[test]
fn declares_every_field_the_stages_set() {
    let kinds = [
        RecordKind::ProgramCard,
        RecordKind::ProgramCategory,
        RecordKind::ProgramLocation,
    ];
    let known: Vec<&str> = kinds.into_iter().flat_map(declared).collect();
    let missing: Vec<String> = set_fields(STAGES)
        .into_iter()
        .filter(|name| !known.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "fields set by the program stages and not declared by the record kinds: {missing:?}"
    );
}

#[test]
fn declares_each_field_in_the_kind_of_its_stage() {
    let find = declared(RecordKind::ProgramLocation);
    for name in ["version_source", "alias_of"] {
        assert!(find.contains(&name), "{name}");
    }
    let card = declared(RecordKind::ProgramCard);
    for name in [
        "score",
        "query",
        "instead",
        "guide_etag",
        "alias_kind",
        "help",
    ] {
        assert!(card.contains(&name), "{name}");
    }
    let category = declared(RecordKind::ProgramCategory);
    for name in ["level", "parent", "counts_by_source"] {
        assert!(category.contains(&name), "{name}");
    }
}
