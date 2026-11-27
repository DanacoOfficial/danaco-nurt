// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-exec/tests/stages_catalog.rs
// Responsibility:   Agreement of the record stages with the command catalog: names, parameters, types, defaults and accepted record kinds.
// Dependencies:     nurt_catalog, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Agreement of the record stages with the command catalog: names,
//! parameters, types, defaults and accepted record kinds.

// Tests fail through unwrap, expect and panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use nurt_catalog::Registry;
use nurt_contracts::stage::{InputKinds, StageImpl};
use nurt_contracts::value::RecordKind;
use nurt_exec::stages::{COMMANDS, implementations, register};

/// One parameter as written in the catalog: name, type, default, required,
/// exclusive group.
type Param = (
    &'static str,
    &'static str,
    Option<&'static str>,
    bool,
    Option<&'static str>,
);

/// A parameter in owned form, as compared by the test.
type Owned = (String, String, Option<String>, bool, Option<String>);

const CONDITION: Option<&str> = Some("condition");

/// Signatures of the commands as written in `catalog/core.toml` and
/// `catalog/text.toml`.
const SIGNATURES: &[(&str, &[Param])] = &[
    ("filter", &[("condition", "expression", None, true, None)]),
    (
        "select",
        &[("fields", "list<field | named_expression>", None, true, None)],
    ),
    (
        "sort",
        &[
            ("by", "list<field>", None, true, None),
            ("order", "enum(asc|desc)", Some("asc"), false, None),
        ],
    ),
    ("first", &[("count", "int", None, true, None)]),
    ("last", &[("count", "int", None, true, None)]),
    ("skip", &[("count", "int", None, true, None)]),
    ("count", &[]),
    (
        "assert",
        &[
            ("count", "range", None, false, CONDITION),
            ("paths", "range", None, false, CONDITION),
            ("exit_code", "range", None, false, CONDITION),
            ("all", "expression", None, false, CONDITION),
            ("any", "expression", None, false, CONDITION),
            ("empty", "bool", None, false, CONDITION),
            ("contains", "text", None, false, CONDITION),
            ("if_match", "text", None, false, CONDITION),
        ],
    ),
    ("verify", &[("check", "pipeline", None, true, None)]),
    (
        "group",
        &[
            ("by", "field | expression", None, true, None),
            ("keep_items", "bool", Some("false"), false, None),
        ],
    ),
    (
        "summarize",
        &[
            ("fields", "list<field>", None, false, None),
            ("by", "field", None, false, None),
        ],
    ),
    ("distinct", &[("by", "list<field>", None, false, None)]),
    ("parse", &[("pattern", "regex", None, true, None)]),
    (
        "split",
        &[
            ("delimiter", "text | regex", None, true, None),
            ("names", "list<text>", None, false, None),
        ],
    ),
];

fn compact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn registry_with_stages() -> Registry {
    let mut registry = Registry::load_embedded().unwrap();
    let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
    register(&mut stages).unwrap();
    for stage in stages {
        registry.register_impl(stage).unwrap();
    }
    registry
}

#[test]
fn signatures_equal_the_catalog() {
    let registry = Registry::load_embedded().unwrap();
    assert_eq!(SIGNATURES.len(), COMMANDS.len());
    for ((command, params), expected) in SIGNATURES.iter().zip(COMMANDS) {
        assert_eq!(*command, expected);
        let spec = registry.command(command).unwrap();
        let actual: Vec<Owned> = spec
            .params
            .iter()
            .map(|param| {
                (
                    param.name.clone(),
                    compact(&param.ty.to_string()),
                    param
                        .default
                        .as_ref()
                        .map(|default| default.as_str().to_owned()),
                    param.required,
                    param.exclusive_group.clone(),
                )
            })
            .collect();
        let wanted: Vec<Owned> = params
            .iter()
            .map(|(name, ty, default, required, group)| {
                (
                    (*name).to_owned(),
                    compact(ty),
                    default.map(str::to_owned),
                    *required,
                    group.map(str::to_owned),
                )
            })
            .collect();
        assert_eq!(actual, wanted, "signature of `{command}`");
    }
    let assert = registry.command("assert").unwrap();
    assert_eq!(assert.required_groups, ["condition"]);
}

#[test]
fn every_implementation_agrees_with_the_catalog() {
    let registry = registry_with_stages();
    let issues: Vec<String> = registry
        .validate_implementations()
        .into_iter()
        .map(|issue| format!("{issue:?}"))
        .collect();
    assert!(issues.is_empty(), "{issues:#?}");
    for command in COMMANDS {
        assert!(
            !registry.unimplemented_commands().contains(&command),
            "{command}"
        );
    }
}

#[test]
fn implementations_resolve_by_record_kind() {
    let registry = registry_with_stages();
    for command in COMMANDS {
        let kind = match command {
            "parse" | "split" => RecordKind::Line,
            "verify" => RecordKind::Effect,
            _ => RecordKind::Entry,
        };
        let stage = registry.resolve(command, kind).unwrap();
        assert_eq!(stage.descriptor().command, command);
        assert!(!stage.descriptor().source, "{command} is not a source");
        assert!(stage.as_read().is_some(), "{command} reads records");
        assert!(stage.as_effect().is_none(), "{command} has no effect");
    }
    let error = registry.resolve("split", RecordKind::Entry).err().unwrap();
    assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Type);
    assert!(registry.resolve("parse", RecordKind::Content).is_ok());
    assert!(registry.resolve("parse", RecordKind::Match).is_ok());
}

#[test]
fn registration_follows_the_command_list_and_rejects_duplicates() {
    let commands: Vec<String> = implementations()
        .iter()
        .map(|stage| stage.descriptor().command)
        .collect();
    assert_eq!(commands, COMMANDS);
    let mut registry = registry_with_stages();
    let again = implementations().into_iter().next().unwrap();
    assert!(registry.register_impl(again).is_err());
    let inputs: Vec<InputKinds> = implementations()
        .iter()
        .map(|stage| stage.descriptor().input)
        .collect();
    assert_eq!(
        inputs
            .iter()
            .filter(|input| **input == InputKinds::Any)
            .count(),
        11
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
