// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-sys/tests/registry_windows.rs
// Responsibility:   Tests of the `registry:` provider on the registry of a running Windows.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the `registry:` provider on the registry of a running Windows
//! system.
//!
//! The tests create keys below `HKCU\Software\NurtTest\<unique name>` and
//! delete them at the end, also when an assertion fails.

#![cfg(windows)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use std::time::{SystemTime, UNIX_EPOCH};

use nurt_contracts::fs::{
    Provider, ProviderKey, ProviderSnapshot, ProviderValue, ProviderWrite, ScopePurpose, TxScope,
};
use nurt_contracts::journal::OpId;
use nurt_contracts::record::RegistryView;
use nurt_sys::registry::RegistryProvider;
use nurt_sys::registry::path::{KeyPath, RegistryPath};

#[allow(
    clippy::disallowed_methods,
    reason = "the tests act as the journal, which owns transaction scopes"
)]
fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
}

/// Unique test key below `HKCU\Software\NurtTest`, removed on drop.
struct TestKey {
    provider: RegistryProvider,
    key: KeyPath,
}

impl TestKey {
    fn new(provider: &RegistryProvider) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("t{}-{nanos}", std::process::id());
        let key = RegistryPath::parse("registry:/HKCU/Software/NurtTest")
            .unwrap()
            .as_key()
            .unwrap()
            .child(&name);
        Self {
            provider: provider.clone(),
            key,
        }
    }

    fn value(&self, name: &str) -> ProviderKey {
        self.key.value_provider_key(name)
    }
}

impl Drop for TestKey {
    fn drop(&mut self) {
        let scope = scope();
        let _ = self.provider.remove(&scope, &self.key.to_provider_key());
        if let Some(parent) = self.key.parent() {
            let empty = self
                .provider
                .list(&parent)
                .map(|items| items.is_empty())
                .unwrap_or(false);
            if empty {
                let _ = self.provider.remove(&scope, &parent.to_provider_key());
            }
        }
    }
}

fn undo(provider: &RegistryProvider, snapshots: &[ProviderSnapshot]) {
    let scope = scope();
    for snapshot in snapshots.iter().rev() {
        provider.restore(&scope, snapshot).unwrap();
    }
}

#[test]
#[ignore = "requires the Windows registry"]
fn it_registry_hkcu_write_undo_roundtrip() {
    let provider = RegistryProvider::system();
    let test = TestKey::new(&provider);
    let target = test.value("Mode");
    let snapshot = provider.snapshot(&target).unwrap();
    provider
        .set(&scope(), &target, &ProviderValue::U32(42))
        .unwrap();
    assert_eq!(provider.get(&target).unwrap(), Some(ProviderValue::U32(42)));
    undo(&provider, &[snapshot]);
    assert_eq!(provider.get(&target).unwrap(), None);
    assert!(provider.list(&test.key).is_err());
}

#[test]
#[ignore = "requires the Windows registry"]
fn it_registry_info_of_a_key_with_many_subkeys_does_not_overflow_the_class_buffer() {
    let provider = RegistryProvider::system();
    let path = RegistryPath::parse("registry:/HKCU/Software").unwrap();
    let info = provider.info(&path).unwrap();
    assert!(matches!(info, nurt_sys::registry::RegistryItem::Key(_)));
    assert!(!provider.list(&path.as_key().unwrap()).unwrap().is_empty());
}

#[test]
#[ignore = "requires the Windows registry"]
fn it_registry_set_below_a_missing_key_creates_it_and_reads_back_by_path() {
    let provider = RegistryProvider::system();
    let test = TestKey::new(&provider);
    let target = test.value("Mode");
    provider
        .set(&scope(), &target, &ProviderValue::U32(3))
        .unwrap();
    let text = format!("{}/Mode", test.key.to_language_string());
    let value = provider.read(&RegistryPath::parse(&text).unwrap()).unwrap();
    assert_eq!(value.name, "Mode");
}

#[test]
#[ignore = "requires the Windows registry"]
fn it_registry_remove_subtree_undo() {
    let provider = RegistryProvider::system();
    let test = TestKey::new(&provider);
    let scope = scope();
    provider
        .set(
            &scope,
            &test.value(""),
            &ProviderValue::Text("default".into()),
        )
        .unwrap();
    let child = test.key.child("Child");
    provider
        .set(
            &scope,
            &child.value_provider_key("List"),
            &ProviderValue::TextList(vec!["a".into(), "b".into()]),
        )
        .unwrap();
    let before = provider.find(&test.key, None).unwrap().items;
    let snapshot = provider.snapshot(&test.key.to_provider_key()).unwrap();
    provider
        .remove(&scope, &test.key.to_provider_key())
        .unwrap();
    undo(&provider, &[snapshot]);
    let after = provider.find(&test.key, None).unwrap().items;
    assert_eq!(after.len(), before.len());
    assert_eq!(
        provider.get(&child.value_provider_key("List")).unwrap(),
        Some(ProviderValue::TextList(vec!["a".into(), "b".into()]))
    );
}

#[test]
#[ignore = "requires the Windows registry"]
fn it_registry_wow64_views_differ() {
    let provider = RegistryProvider::system();
    let path = RegistryPath::parse("registry:/HKLM/SOFTWARE").unwrap();
    let key = path.as_key().unwrap();
    let view64 = provider.list(&key).unwrap();
    let view32 = provider.in_view(RegistryView::Bits32).list(&key).unwrap();
    assert_ne!(view64, view32);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
