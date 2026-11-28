// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-catalog/tests/embedded.rs
// Responsibility:   Tests of the catalog data files shipped with the product.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the catalog data files shipped with the product.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by panicking"
)]

use std::collections::HashSet;
use std::path::Path;

use nurt_catalog::{NameEntry, NameKind, Registry, StageSpecExt};
use nurt_contracts::spec::{CORE_GROUP, IoSpec, Os};

/// The reference catalog directory, resolved at run time so that relocated test archives find it.
fn catalog_dir() -> std::path::PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../catalog")
}

#[test]
fn embedded_catalog_has_177_commands_in_15_groups() {
    let registry = Registry::load_embedded().unwrap();
    assert_eq!(registry.len(), 177);
    assert_eq!(registry.index().total, 177);
    assert_eq!(registry.groups().count(), 15);
    assert!(!registry.is_empty());
}

#[test]
fn embedded_catalog_equals_catalog_loaded_from_directory() {
    let embedded = Registry::load_embedded().unwrap();
    let from_disk = Registry::load_dir(catalog_dir()).unwrap();
    assert_eq!(
        embedded.catalog_rev().unwrap(),
        from_disk.catalog_rev().unwrap()
    );
    let embedded_names: Vec<&str> = embedded.commands().map(|c| c.name.as_str()).collect();
    let disk_names: Vec<&str> = from_disk.commands().map(|c| c.name.as_str()).collect();
    assert_eq!(embedded_names, disk_names);
}

#[test]
fn every_command_carries_the_group_of_its_file() {
    let registry = Registry::load_embedded().unwrap();
    for group in registry.groups() {
        assert_eq!(
            registry.index().entry(&group.name).unwrap().commands as usize,
            group.commands.len()
        );
        for command in &group.commands {
            assert_eq!(command.group, group.name);
        }
    }
}

#[test]
fn command_names_are_unique_across_groups() {
    let registry = Registry::load_embedded().unwrap();
    let mut seen = HashSet::new();
    for command in registry.commands() {
        assert!(
            seen.insert(command.name.clone()),
            "{} repeated",
            command.name
        );
    }
}

#[test]
fn verify_is_a_command_and_a_group_in_separate_namespaces() {
    let registry = Registry::load_embedded().unwrap();
    let Some(NameEntry::Command(command)) = registry.lookup("verify", NameKind::Command) else {
        panic!("verify command missing");
    };
    assert_eq!(command.group, CORE_GROUP);
    let Some(NameEntry::Group(group)) = registry.lookup("verify", NameKind::Group) else {
        panic!("verify group missing");
    };
    assert_eq!(group.commands.len(), 8);
    assert!(group.commands.iter().all(|c| c.name != "verify"));
    assert_eq!(registry.commands_in("verify").unwrap().len(), 8);
}

#[test]
fn lookup_returns_none_for_name_in_the_other_namespace() {
    let registry = Registry::load_embedded().unwrap();
    assert!(registry.lookup("core", NameKind::Command).is_none());
    assert!(registry.lookup("sort", NameKind::Group).is_none());
    assert!(registry.commands_in("sort").is_none());
}

#[test]
fn positional_parameters_come_in_position_order() {
    let registry = Registry::load_embedded().unwrap();
    for command in registry.commands() {
        let positions: Vec<u32> = command
            .positional()
            .iter()
            .filter_map(|p| p.position)
            .collect();
        let expected: Vec<u32> = (1..).take(positions.len()).collect();
        assert_eq!(positions, expected, "{}", command.name);
    }
}

#[test]
fn required_parameters_and_explicit_defaults_are_listed() {
    let registry = Registry::load_embedded().unwrap();
    let with_required = registry
        .commands()
        .filter(|c| !c.required_params().is_empty())
        .count();
    let with_defaults = registry
        .commands()
        .filter(|c| !c.explicit_defaults().is_empty())
        .count();
    assert!(with_required > 20);
    assert!(with_defaults > 20);
    for command in registry.commands() {
        for param in command.required_params() {
            assert!(param.required && param.default.is_none());
        }
        for (param, default) in command.explicit_defaults() {
            assert_eq!(param.default.as_ref(), Some(default));
        }
    }
}

#[test]
fn os_restricted_commands_are_unavailable_on_other_systems() {
    let registry = Registry::load_embedded().unwrap();
    let restricted: Vec<_> = registry.commands().filter(|c| !c.os.is_empty()).collect();
    assert!(!restricted.is_empty());
    for command in restricted {
        let all = [Os::Linux, Os::Macos, Os::Windows];
        let available = all.iter().filter(|os| command.available_on(**os)).count();
        assert!(available < all.len(), "{}", command.name);
        assert!(command.os.iter().all(|os| command.available_on(*os)));
    }
    assert!(registry.command("sort").unwrap().available_on(Os::Windows));
}

#[test]
fn external_use_and_action_effects_are_exposed() {
    let registry = Registry::load_embedded().unwrap();
    assert!(registry.commands().any(|c| c.external.is_some()));
    let with_effects = registry.commands().find(|c| !c.effects.is_empty()).unwrap();
    let action = &with_effects.effects[0].action;
    assert_eq!(
        with_effects.effect_for(action),
        Some(&with_effects.effects[0])
    );
    assert!(with_effects.effect_for("no_such_action").is_none());
}

#[test]
fn polymorphic_commands_expose_forms_by_input() {
    let registry = Registry::load_embedded().unwrap();
    let remove = registry.command("remove").unwrap();
    assert!(!remove.forms.is_empty());
    assert!(remove.is_source());
    let form = remove
        .form_for(&IoSpec::Record("entry".to_owned()))
        .unwrap();
    assert!(form.input.contains(&IoSpec::Record("entry".to_owned())));
    assert!(registry.command("count").unwrap().forms.is_empty());
}

#[test]
fn require_command_and_group_report_unknown_names() {
    let registry = Registry::load_embedded().unwrap();
    assert!(registry.require_command("count").is_ok());
    assert!(registry.require_group("core").is_ok());
    let error = registry.require_command("coutn").unwrap_err();
    assert_eq!(
        error.to_string(),
        "E_UNKNOWN_COMMAND: unknown command `coutn`"
    );
    assert_eq!(
        registry
            .require_group("nothing")
            .unwrap_err()
            .code()
            .to_string(),
        "E_UNKNOWN_NAME"
    );
}

#[test]
fn debug_output_counts_commands() {
    let registry = Registry::load_embedded().unwrap();
    let text = format!("{registry:?}");
    assert!(text.contains("commands: 177"), "{text}");
}

#[test]
fn registry_is_shareable_between_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Registry>();
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
