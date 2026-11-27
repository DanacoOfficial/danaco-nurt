// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the implementation registry: registration and resolution by the
//! pair of a command and an input record kind.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by panicking"
)]

mod common;

use std::sync::Arc;

use common::Fake;
use nurt_catalog::Registry;
use nurt_contracts::error::ErrorCode;
use nurt_contracts::stage::{InputKinds, StageImpl, StageRegistrar};
use nurt_contracts::value::RecordKind;

fn registry() -> Registry {
    Registry::load_embedded().unwrap()
}

fn descriptor_kinds(stage: &Arc<dyn StageImpl>) -> InputKinds {
    stage.descriptor().input
}

#[test]
fn one_command_resolves_to_the_implementation_of_the_record_kind() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Entry]))
        .unwrap();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Job, RecordKind::Item]))
        .unwrap();
    registry.register_impl(Fake::source("remove")).unwrap();

    let entry = registry.resolve("remove", RecordKind::Entry).unwrap();
    assert_eq!(
        descriptor_kinds(&entry),
        InputKinds::Only(vec![RecordKind::Entry])
    );
    let job = registry.resolve("remove", RecordKind::Job).unwrap();
    assert_eq!(
        descriptor_kinds(&job),
        InputKinds::Only(vec![RecordKind::Job, RecordKind::Item])
    );
    assert!(
        registry
            .resolve_source("remove")
            .unwrap()
            .descriptor()
            .source
    );
    assert_eq!(registry.implementations("remove").count(), 3);
}

#[test]
fn unsupported_record_kind_is_a_type_error_listing_accepted_kinds() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Entry, RecordKind::Job]))
        .unwrap();
    let error = registry.resolve("remove", RecordKind::Match).err().unwrap();
    assert_eq!(error.code(), ErrorCode::Type);
    assert_eq!(
        error.to_string(),
        "E_TYPE: `remove` does not accept `match` records, accepted kinds: entry, job"
    );
    assert_eq!(error.found.as_deref(), Some("match"));
    assert_eq!(error.expected, Some(serde_json::json!(["entry", "job"])));
}

#[test]
fn unknown_command_is_reported_before_the_kind_is_checked() {
    let registry = registry();
    let error = registry.resolve("remvoe", RecordKind::Entry).err().unwrap();
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
    let error = registry.resolve_source("remvoe").err().unwrap();
    assert_eq!(error.code(), ErrorCode::UnknownCommand);
}

#[test]
fn command_without_implementation_is_an_internal_error() {
    let registry = registry();
    let error = registry.resolve("remove", RecordKind::Entry).err().unwrap();
    assert_eq!(error.code(), ErrorCode::Internal);
    assert!(error.to_string().contains("no registered implementation"));
}

#[test]
fn any_implementation_accepts_every_kind() {
    let mut registry = registry();
    registry.register_impl(Fake::any("count")).unwrap();
    for kind in [RecordKind::Entry, RecordKind::Match, RecordKind::Job] {
        assert!(registry.resolve("count", kind).is_ok());
    }
    assert_eq!(registry.accepted_kinds("count").kinds, vec!["any"]);
}

#[test]
fn source_resolution_fails_for_a_command_that_is_not_a_source() {
    let mut registry = registry();
    registry.register_impl(Fake::any("count")).unwrap();
    let error = registry.resolve_source("count").err().unwrap();
    assert_eq!(error.code(), ErrorCode::Type);
    assert!(
        error.to_string().contains("cannot start a pipeline"),
        "{error}"
    );
}

#[test]
fn source_only_implementation_gives_none_as_accepted_kind() {
    let mut registry = registry();
    registry.register_impl(Fake::source("list")).unwrap();
    let error = registry.resolve("list", RecordKind::Entry).err().unwrap();
    assert!(
        error.to_string().ends_with("accepted kinds: none"),
        "{error}"
    );
    assert!(registry.accepted_kinds("list").source);
}

#[test]
fn overlapping_input_kinds_are_rejected_at_registration() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Entry, RecordKind::Job]))
        .unwrap();
    assert!(
        registry
            .register_impl(Fake::only("remove", &[RecordKind::Job]))
            .is_err()
    );
    assert!(registry.register_impl(Fake::any("remove")).is_err());
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Item]))
        .unwrap();
    assert_eq!(registry.implementations("remove").count(), 2);
}

#[test]
fn second_source_for_a_command_is_rejected() {
    let mut registry = registry();
    registry.register_impl(Fake::source("remove")).unwrap();
    let error = registry.register_impl(Fake::source("remove")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
    assert!(
        error
            .to_string()
            .contains("another implementation accepts the same input")
    );
}

#[test]
fn validation_accepts_consistent_implementations() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Entry]))
        .unwrap();
    registry.register_impl(Fake::source("remove")).unwrap();
    registry.register_impl(Fake::any("count")).unwrap();
    assert!(registry.validate_implementations().is_empty());
}

#[test]
fn validation_reports_inconsistent_implementations() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("count", &[RecordKind::Entry]))
        .unwrap();
    registry.register_impl(Fake::source("sort")).unwrap();
    registry.register_impl(Fake::any("verify")).unwrap();
    registry
        .register_impl(Fake::only("ghost", &[RecordKind::Entry]))
        .unwrap();
    registry
        .register_impl(Arc::new(Fake {
            command: "first",
            source: false,
            input: InputKinds::None,
        }))
        .unwrap();
    let messages: Vec<String> = registry
        .validate_implementations()
        .iter()
        .map(ToString::to_string)
        .collect();
    let has = |fragment: &str| messages.iter().any(|message| message.contains(fragment));
    assert!(
        has("sort: source implementation, but the catalog input has no `none`"),
        "{messages:?}"
    );
    assert!(
        has("verify: implementation accepts any kind"),
        "{messages:?}"
    );
    assert!(
        has("ghost: implementation has no command in the catalog"),
        "{messages:?}"
    );
    assert!(
        has("first: implementation accepts no input and is not a source"),
        "{messages:?}"
    );
    assert_eq!(messages.len(), 4, "{messages:?}");
}

#[test]
fn validation_reports_kind_missing_from_the_catalog_input() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("verify", &[RecordKind::Entry]))
        .unwrap();
    let messages: Vec<String> = registry
        .validate_implementations()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        messages,
        vec!["verify: input kind `entry` is not in the catalog input"]
    );
}

#[test]
fn unimplemented_commands_shrink_as_implementations_register() {
    let mut registry = registry();
    assert_eq!(registry.unimplemented_commands().len(), 166);
    registry.register_impl(Fake::any("count")).unwrap();
    let remaining = registry.unimplemented_commands();
    assert_eq!(remaining.len(), 165);
    assert!(!remaining.contains(&"count"));
}

#[test]
fn verify_resolves_as_a_command_and_group_members_resolve_separately() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("verify", &[RecordKind::Effect]))
        .unwrap();
    registry
        .register_impl(Fake::only("test.run", &[RecordKind::Exec]))
        .unwrap();
    assert!(registry.resolve("verify", RecordKind::Effect).is_ok());
    assert!(registry.resolve("test.run", RecordKind::Exec).is_ok());
    assert!(registry.resolve("verify", RecordKind::Exec).is_err());
}

#[test]
fn registrar_trait_registers_into_the_registry_and_reports_conflicts() {
    let mut registry = registry();
    let registrar: &mut dyn StageRegistrar = &mut registry;
    registrar.register(Fake::any("count")).unwrap();
    let error = registrar.register(Fake::any("count")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Catalog);
    assert!(registry.resolve("count", RecordKind::Entry).is_ok());
    assert_eq!(registry.implementations("count").count(), 1);
}

use nurt_catalog::descriptors_overlap;
use nurt_contracts::stage::StageDescriptor;

fn descriptor(command: &str, source: bool, input: InputKinds) -> StageDescriptor {
    StageDescriptor {
        command: command.to_owned(),
        source,
        input,
    }
}

#[test]
fn descriptors_of_different_commands_never_overlap() {
    let any = descriptor("count", false, InputKinds::Any);
    let other = descriptor("sort", false, InputKinds::Any);
    assert!(!descriptors_overlap(&any, &other));
    let source = descriptor("find", true, InputKinds::None);
    assert!(!descriptors_overlap(
        &source,
        &descriptor("list", true, InputKinds::None)
    ));
}

#[test]
fn descriptors_overlap_when_both_are_sources_or_share_a_kind() {
    let source = descriptor("remove", true, InputKinds::None);
    assert!(descriptors_overlap(&source, &source.clone()));
    let entries = descriptor("remove", false, InputKinds::Only(vec![RecordKind::Entry]));
    let jobs = descriptor(
        "remove",
        false,
        InputKinds::Only(vec![RecordKind::Job, RecordKind::Item]),
    );
    let both = descriptor(
        "remove",
        false,
        InputKinds::Only(vec![RecordKind::Item, RecordKind::Entry]),
    );
    assert!(!descriptors_overlap(&entries, &jobs));
    assert!(descriptors_overlap(&entries, &both));
    assert!(descriptors_overlap(&jobs, &both));
    assert!(descriptors_overlap(&entries, &entries.clone()));
}

#[test]
fn descriptor_without_input_overlaps_no_record_descriptor() {
    let source = descriptor("remove", true, InputKinds::None);
    let any = descriptor("remove", false, InputKinds::Any);
    let entries = descriptor("remove", false, InputKinds::Only(vec![RecordKind::Entry]));
    assert!(!descriptors_overlap(&source, &any));
    assert!(!descriptors_overlap(&entries, &source));
    assert!(descriptors_overlap(&any, &entries));
    assert!(descriptors_overlap(&any, &any.clone()));
}

#[test]
fn accepted_kinds_of_an_unknown_command_are_empty() {
    let registry = registry();
    let accepted = registry.accepted_kinds("no_such_command");
    assert!(accepted.kinds.is_empty());
    assert!(!accepted.source);
}

#[test]
fn accepted_kinds_list_each_kind_once_in_registration_order() {
    let mut registry = registry();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Entry, RecordKind::Job]))
        .unwrap();
    registry
        .register_impl(Fake::only("remove", &[RecordKind::Item]))
        .unwrap();
    let accepted = registry.accepted_kinds("remove");
    assert_eq!(accepted.kinds, vec!["entry", "job", "item"]);
    assert!(!accepted.source);
}
