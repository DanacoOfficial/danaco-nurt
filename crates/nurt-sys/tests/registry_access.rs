// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sys/tests/registry_access.rs
// Responsibility:   The registry as the file commands see it ([`RegistryAccess`]), on the in-memory registry.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The registry as the file commands see it ([`RegistryAccess`]), on the
//! in-memory registry: records of values and keys, listings with the path
//! below the listed key, and the provider keys that changes declare.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use std::sync::Arc;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{RegistryAccess, RegistryKind, RegistryNodeKind};
use nurt_contracts::record::RegistryView;
use nurt_contracts::value::{RecordKind, Value};
use nurt_sys::registry::RegistryProvider;
use nurt_sys::registry::api::RegistryApi;
use nurt_sys::registry::mem::MemRegistry;
use nurt_sys::registry::path::RegistryPath;
use nurt_sys::registry::value::{REG_SZ, RawValue};

const V64: RegistryView = RegistryView::Bits64;

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Registry with `HKCU/Software/Example`, its values `Mode` (`REG_DWORD`)
/// and the default value, and the subkey `Child` with the value `Name`.
fn access() -> Arc<dyn RegistryAccess> {
    let registry = Arc::new(MemRegistry::new());
    let key = |text: &str| RegistryPath::parse(text).unwrap().as_key().unwrap();
    let example = key("registry:/HKCU/Software/Example");
    registry
        .create_key(V64, &example.prefix(1), "", false)
        .unwrap();
    registry.create_key(V64, &example, "", false).unwrap();
    registry
        .set_value(V64, &example, "Mode", &RawValue::dword(3))
        .unwrap();
    registry
        .set_value(V64, &example, "", &RawValue::new(REG_SZ, utf16("plain")))
        .unwrap();
    let child = example.child("Child");
    registry.create_key(V64, &child, "", false).unwrap();
    registry
        .set_value(V64, &child, "Name", &RawValue::new(REG_SZ, utf16("leaf")))
        .unwrap();
    Arc::new(RegistryProvider::new(registry))
}

#[test]
fn read_value_returns_the_record_and_refuses_a_key() {
    let access = access();
    let record = access
        .read_value("registry:/hkcu/software/example/mode")
        .unwrap();
    assert_eq!(record.kind, RecordKind::RegistryValue);
    assert_eq!(record.get("value"), Some(&Value::Int(3)));
    let default = access
        .read_value("registry:/HKCU/Software/Example/(default)")
        .unwrap();
    assert_eq!(default.get("value"), Some(&Value::Text("plain".to_owned())));
    assert!(
        access
            .read_value("registry:/HKCU/Software/Example")
            .is_err()
    );
    let missing = access
        .read_value("registry:/HKCU/Software/Example/Absent")
        .unwrap_err();
    assert_eq!(missing.code(), ErrorCode::NotFound);
}

#[test]
fn describe_gives_none_for_a_missing_path_and_rejects_bad_syntax() {
    let access = access();
    let key = access
        .describe("registry:/HKCU/Software/Example")
        .unwrap()
        .unwrap();
    assert_eq!(key.kind, RecordKind::Entry);
    assert_eq!(key.get("exists"), Some(&Value::Bool(true)));
    assert!(
        access
            .describe("registry:/HKCU/Software/Gone")
            .unwrap()
            .is_none()
    );
    let error = access.describe("registry:/NOPE/Software").unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    let error = access.describe("registry:/HKCU/a//b").unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn walk_reports_the_path_below_the_listed_key() {
    let access = access();
    let nodes = access
        .walk("registry:/HKCU/Software/Example", None)
        .unwrap();
    let mut found: Vec<(RegistryNodeKind, &str, &str)> = nodes
        .iter()
        .map(|node| (node.kind, node.name.as_str(), node.relative.as_str()))
        .collect();
    found.sort_by_key(|(_, _, relative)| *relative);
    assert_eq!(
        found,
        [
            (RegistryNodeKind::Value, "", "(default)"),
            (RegistryNodeKind::Key, "Child", "Child"),
            (RegistryNodeKind::Value, "Name", "Child/Name"),
            (RegistryNodeKind::Value, "Mode", "Mode"),
        ]
    );
    let shallow = access
        .walk("registry:/HKCU/Software/Example", Some(1))
        .unwrap();
    assert_eq!(
        shallow.len(),
        3,
        "the value of the subkey is one level down"
    );
    assert_eq!(
        access
            .walk("registry:/HKCU/Software/Example/Mode", None)
            .unwrap_err()
            .code(),
        ErrorCode::Io
    );
}

#[test]
fn locate_gives_the_provider_key_of_an_existing_address() {
    let access = access();
    let value = access
        .locate("registry:/hkcu/Software/Example/Mode")
        .unwrap();
    assert_eq!(value.kind, RegistryKind::Value);
    assert_eq!(value.key.segments, ["HKCU", "Software", "Example"]);
    assert_eq!(value.key.value.as_deref(), Some("Mode"));
    let key = access.locate("registry:/HKCU/Software/Example").unwrap();
    assert_eq!(key.kind, RegistryKind::Key);
    assert_eq!(key.key.value, None);
    let default = access
        .locate("registry:/HKCU/Software/Example/(default)")
        .unwrap();
    assert_eq!(default.key.value.as_deref(), Some(""));
    assert_eq!(
        access
            .locate("registry:/HKCU/Software/Gone")
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
}

#[test]
fn address_reads_a_path_as_the_requested_kind_without_requiring_it_to_exist() {
    let access = access();
    let value = access
        .address("registry:/HKCU/Software/New/Name", RegistryKind::Value)
        .unwrap();
    assert_eq!(value.key.segments, ["HKCU", "Software", "New"]);
    assert_eq!(value.key.value.as_deref(), Some("Name"));
    let key = access
        .address(
            "registry:/HKEY_CURRENT_USER/Software/New",
            RegistryKind::Key,
        )
        .unwrap();
    assert_eq!(key.key.segments, ["HKCU", "Software", "New"]);
    assert_eq!(
        access
            .address("registry:/HKCU", RegistryKind::Value)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidValue
    );
    assert_eq!(
        access
            .address("registry:/HKCU/Software/(default)", RegistryKind::Key)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidValue
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
