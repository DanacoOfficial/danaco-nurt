// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Journal scenarios of the `registry:` provider on the in-memory registry.
//!
//! Each scenario takes the snapshots the journal takes before a change,
//! applies the change, restores the snapshots in reverse order as undo does,
//! and compares the whole registry with its state before the change,
//! including value types and bytes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

use std::path::Path as StdPath;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::fs::{
    FsErrorKind, Provider, ProviderKey, ProviderScheme, ProviderSnapshot, ProviderValue,
    ProviderWrite, ScopePurpose, TxScope,
};
use nurt_contracts::journal::OpId;
use nurt_contracts::record::{EntryType, RegistryType, RegistryView};
use nurt_contracts::report::Irreversible;
use nurt_contracts::value::{Instant, RecordKind, Value};
use nurt_sys::registry::api::RegistryApi;
use nurt_sys::registry::broadcast::SettingChangeNotifier;
use nurt_sys::registry::mem::MemRegistry;
use nurt_sys::registry::path::{Address, Hive, KeyPath, RegistryPath};
use nurt_sys::registry::snapshot::{Captured, KeyTree, SubtreeDocument, capture};
use nurt_sys::registry::value::{REG_BINARY, REG_SZ, RawValue, WRITABLE_TYPES};
use nurt_sys::registry::{RegistryItem, RegistryProvider, bind_path};

const V64: RegistryView = RegistryView::Bits64;
const V32: RegistryView = RegistryView::Bits32;

/// Time of the last write recorded by the in-memory registry in the tests.
const WRITE_TIME_NANOS: i64 = 1_763_800_000_000_000_000;

#[allow(
    clippy::disallowed_methods,
    reason = "the tests act as the journal, which owns transaction scopes"
)]
fn scope() -> TxScope {
    TxScope::new_for_journal(OpId(1), ScopePurpose::Operation)
}

struct Fixture {
    registry: Arc<MemRegistry>,
    provider: RegistryProvider,
}

/// Registry with `HKCU/Software/Example`: a `REG_SZ` default value stored
/// without terminator, a `REG_DWORD` value `Mode`, a class, a security
/// descriptor and the subkey `Child` with a `REG_BINARY` value.
fn fixture() -> Fixture {
    let registry = Arc::new(MemRegistry::new());
    registry
        .set_time(Instant::from_unix_nanos(WRITE_TIME_NANOS))
        .unwrap();
    let example = key("registry:/HKCU/Software/Example");
    registry
        .create_key(V64, &example.prefix(1), "", false)
        .unwrap();
    registry
        .create_key(V64, &example, "ExampleClass", false)
        .unwrap();
    registry
        .set_value(V64, &example, "", &RawValue::new(REG_SZ, utf16("plain")))
        .unwrap();
    registry
        .set_value(V64, &example, "Mode", &RawValue::dword(1))
        .unwrap();
    registry
        .set_security(V64, &example, &[1, 0, 4, 128, 20])
        .unwrap();
    let child = example.child("Child");
    registry.create_key(V64, &child, "", false).unwrap();
    registry
        .set_value(
            V64,
            &child,
            "Blob",
            &RawValue::new(REG_BINARY, vec![0, 255]),
        )
        .unwrap();
    let provider = RegistryProvider::new(registry.clone());
    Fixture { registry, provider }
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn key(text: &str) -> KeyPath {
    RegistryPath::parse(text).unwrap().as_key().unwrap()
}

fn value_key(text: &str) -> ProviderKey {
    let (key, name) = RegistryPath::parse(text).unwrap().as_value().unwrap();
    key.value_provider_key(&name)
}

/// Whole content of every root key in the 64-bit view, which in the
/// in-memory registry includes the 32-bit view below `WOW6432Node`.
fn dump(registry: &MemRegistry) -> Vec<KeyTree> {
    Hive::ALL
        .into_iter()
        .map(|hive| {
            let root = KeyPath::root(hive);
            match capture(registry, V64, &root, hive.short_name(), None).unwrap() {
                Captured::Tree(capture) => capture.tree.unwrap(),
                Captured::TooLarge => panic!("no budget was given"),
            }
        })
        .collect()
}

/// Snapshots of `targets`, taken in declaration order as the journal does.
fn preserve(targets: &[(&RegistryProvider, ProviderKey)]) -> Vec<ProviderSnapshot> {
    targets
        .iter()
        .map(|(owner, key)| owner.snapshot(key).unwrap())
        .collect()
}

/// Restores snapshots in reverse order, as undo does.
fn undo(provider: &RegistryProvider, snapshots: &[ProviderSnapshot]) {
    let scope = scope();
    for snapshot in snapshots.iter().rev() {
        provider.restore(&scope, snapshot).unwrap();
    }
}

#[derive(Default)]
struct CountingNotifier(AtomicUsize);

impl SettingChangeNotifier for CountingNotifier {
    fn environment_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn registry_list_keys_as_directory_entries_with_modified() {
    let Fixture { provider, .. } = fixture();
    let items = provider
        .list(&key("registry:/HKCU/software/EXAMPLE"))
        .unwrap();
    assert_eq!(items.len(), 3);
    let RegistryItem::Key(entry) = &items[0] else {
        panic!("keys come first");
    };
    assert_eq!(entry.kind, EntryType::Directory);
    assert_eq!(entry.name, "Child");
    assert_eq!(entry.modified, Instant::from_unix_nanos(WRITE_TIME_NANOS));
    assert_eq!(entry.path.as_str(), "registry:/HKCU/software/EXAMPLE/Child");
    let RegistryItem::Value(default) = &items[1] else {
        panic!("values follow the keys");
    };
    assert_eq!(default.name, "");
    assert_eq!(default.kind, RegistryType::String);
    assert_eq!(default.value, Value::Text("plain".into()));
    assert_eq!(default.size.bytes(), 10);
    assert_eq!(default.view, Some(V64));
    let RegistryItem::Value(mode) = &items[2] else {
        panic!("values follow the keys");
    };
    assert_eq!(mode.value, Value::Int(1));
    let kinds: Vec<RecordKind> = items
        .into_iter()
        .map(|item| item.into_record().kind)
        .collect();
    assert_eq!(
        kinds,
        [
            RecordKind::Entry,
            RecordKind::RegistryValue,
            RecordKind::RegistryValue
        ]
    );
    assert_eq!(
        provider
            .list(&key("registry:/HKCU/Missing"))
            .unwrap_err()
            .kind,
        FsErrorKind::NotFound
    );
}

#[test]
fn registry_find_does_not_follow_links() {
    let Fixture { registry, provider } = fixture();
    let link = key("registry:/HKCU/Software/Example/Link");
    registry.create_key(V64, &link, "", false).unwrap();
    registry
        .create_key(V64, &link.child("Behind"), "", false)
        .unwrap();
    registry
        .make_link(V64, &link, "\\REGISTRY\\MACHINE\\SOFTWARE\\Target")
        .unwrap();
    let listing = provider
        .find(&key("registry:/HKCU/Software"), None)
        .unwrap();
    let names: Vec<String> = listing
        .items
        .iter()
        .filter_map(|item| match item {
            RegistryItem::Key(entry) => entry.relative.clone(),
            RegistryItem::Value(_) => None,
        })
        .collect();
    assert_eq!(names, ["Example", "Example/Child", "Example/Link"]);
    let link_entry = listing
        .items
        .iter()
        .find_map(|item| match item {
            RegistryItem::Key(entry) if entry.name == "Link" => Some(entry),
            _ => None,
        })
        .unwrap();
    assert_eq!(link_entry.kind, EntryType::Link);
    assert_eq!(link_entry.depth, Some(2));
    assert_eq!(
        link_entry.link_target.as_ref().unwrap().as_str(),
        "\\REGISTRY\\MACHINE\\SOFTWARE\\Target"
    );
    let shallow = provider
        .find(&key("registry:/HKCU/Software"), Some(1))
        .unwrap();
    assert_eq!(shallow.items.len(), 1);
    registry
        .deny(V64, &key("registry:/HKCU/Software/Example/Child"))
        .unwrap();
    let partial = provider
        .find(&key("registry:/HKCU/Software"), None)
        .unwrap();
    assert_eq!(
        partial.unreadable,
        [key("registry:/HKCU/Software/Example/Child")]
    );
}

#[test]
fn registry_write_new_value_undo_removes_it() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let target = value_key("registry:/HKCU/Software/Example/Editor");
    let snapshots = preserve(&[(&provider, target.clone())]);
    assert!(!snapshots[0].existed);
    provider
        .set(&scope(), &target, &ProviderValue::Text("code".into()))
        .unwrap();
    assert_eq!(
        provider.get(&target).unwrap(),
        Some(ProviderValue::Text("code".into()))
    );
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    assert_eq!(provider.get(&target).unwrap(), None);
}

#[test]
fn registry_write_overwrite_changes_type_undo_restores_type_and_bytes() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let target = value_key("registry:/HKCU/Software/Example/(default)");
    let snapshots = preserve(&[(&provider, target.clone())]);
    assert_eq!(
        snapshots[0].value,
        Some(ProviderValue::Binary {
            raw_type: REG_SZ,
            bytes: utf16("plain")
        })
    );
    provider
        .set(&scope(), &target, &ProviderValue::U32(7))
        .unwrap();
    let read = provider
        .read(&RegistryPath::parse("registry:/HKCU/Software/Example/(default)").unwrap())
        .unwrap();
    assert_eq!(read.kind, RegistryType::Dword);
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    let example = key("registry:/HKCU/Software/Example");
    assert_eq!(
        registry.value(V64, &example, "").unwrap(),
        Some(RawValue::new(REG_SZ, utf16("plain")))
    );
}

#[test]
fn registry_write_creates_missing_keys_undo_removes_created_keys() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let target = value_key("registry:/HKCU/Software/Vendor/App/Settings/Theme");
    let snapshots = preserve(&[(&provider, target.clone())]);
    let document = SubtreeDocument::from_bytes(snapshots[0].subtree.as_deref().unwrap()).unwrap();
    assert_eq!(document.existing_depth, Some(1));
    provider
        .set(&scope(), &target, &ProviderValue::Text("dark".into()))
        .unwrap();
    assert!(
        registry
            .key_info(V64, &key("registry:/HKCU/Software/Vendor/App/Settings"))
            .unwrap()
            .is_some()
    );
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    assert!(
        registry
            .key_info(V64, &key("registry:/HKCU/Software/Vendor"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn registry_remove_key_with_subtree_undo_restores_all() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let target = key("registry:/HKCU/Software/Example").to_provider_key();
    let snapshots = preserve(&[(&provider, target.clone())]);
    provider.remove(&scope(), &target).unwrap();
    assert!(
        registry
            .key_info(V64, &key("registry:/HKCU/Software/Example"))
            .unwrap()
            .is_none()
    );
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    let example = key("registry:/HKCU/Software/Example");
    assert_eq!(
        registry.security(V64, &example).unwrap(),
        Some(vec![1, 0, 4, 128, 20])
    );
    assert_eq!(
        registry.key_info(V64, &example).unwrap().unwrap().class,
        "ExampleClass"
    );
}

#[test]
fn registry_remove_value_undo_restores_it() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let target = value_key("registry:/HKCU/Software/Example/Child/Blob");
    let snapshots = preserve(&[(&provider, target.clone())]);
    provider.remove(&scope(), &target).unwrap();
    assert_eq!(provider.get(&target).unwrap(), None);
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    let missing = value_key("registry:/HKCU/Software/Absent/Value");
    assert_eq!(
        provider.remove(&scope(), &missing).unwrap_err().kind,
        FsErrorKind::NotFound
    );
    assert_eq!(
        provider
            .remove(
                &scope(),
                &KeyPath::root(Hive::CurrentUser).to_provider_key()
            )
            .unwrap_err()
            .kind,
        FsErrorKind::PermissionDenied
    );
}

#[test]
fn registry_move_key_is_single_operation_undo_both_sides() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let from = key("registry:/HKCU/Software/Example").to_provider_key();
    let to = key("registry:/HKCU/Software/Moved/Example").to_provider_key();
    let snapshots = preserve(&[(&provider, from.clone()), (&provider, to.clone())]);
    let scope = scope();
    provider.rename(&scope, &from, &to, V64).unwrap();
    assert!(
        registry
            .key_info(V64, &key("registry:/HKCU/Software/Example"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        registry
            .value(
                V64,
                &key("registry:/HKCU/Software/Moved/Example/Child"),
                "Blob"
            )
            .unwrap(),
        Some(RawValue::new(REG_BINARY, vec![0, 255]))
    );
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    assert_eq!(
        provider
            .rename(
                &scope,
                &KeyPath::root(Hive::Users).to_provider_key(),
                &to,
                V64
            )
            .unwrap_err()
            .kind,
        FsErrorKind::PermissionDenied
    );
}

#[test]
fn registry_move_value_undo_restores_both_sides() {
    let Fixture { registry, provider } = fixture();
    let before = dump(&registry);
    let from = value_key("registry:/HKCU/Software/Example/Mode");
    let to = value_key("registry:/HKCU/Software/Other/Mode");
    let snapshots = preserve(&[(&provider, from.clone()), (&provider, to.clone())]);
    provider.rename(&scope(), &from, &to, V64).unwrap();
    assert_eq!(provider.get(&to).unwrap(), Some(ProviderValue::U32(1)));
    assert_eq!(provider.get(&from).unwrap(), None);
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
}

#[test]
fn registry_copy_between_views_32_64() {
    let Fixture { registry, provider } = fixture();
    let vendor = key("registry:/HKLM/SOFTWARE/Vendor");
    registry.create_key(V64, &vendor, "", false).unwrap();
    registry
        .set_value(V64, &vendor, "Version", &RawValue::string("2.0"))
        .unwrap();
    let before = dump(&registry);
    let view32 = provider.in_view(V32);
    assert_eq!(view32.view(), V32);
    let target = vendor.to_provider_key();
    let snapshots = preserve(&[(&view32, target.clone())]);
    provider.copy(&scope(), &target, &target, V32).unwrap();
    let copied = view32
        .read(&RegistryPath::parse("registry:/HKLM/SOFTWARE/Vendor/Version").unwrap())
        .unwrap();
    assert_eq!(copied.view, Some(V32));
    assert_eq!(copied.value, Value::Text("2.0".into()));
    assert!(
        registry
            .key_info(V64, &key("registry:/HKLM/SOFTWARE/WOW6432Node/Vendor"))
            .unwrap()
            .is_some()
    );
    undo(&provider, &snapshots);
    assert_eq!(dump(&registry), before);
    assert_eq!(
        provider
            .copy(&scope(), &target, &target, V64)
            .unwrap_err()
            .kind,
        FsErrorKind::InvalidPath
    );
}

#[test]
fn copy_rejects_existing_destination_and_mixed_addresses() {
    let Fixture { provider, .. } = fixture();
    let example = key("registry:/HKCU/Software/Example");
    let software = key("registry:/HKCU/Software");
    let scope = scope();
    assert_eq!(
        provider
            .copy(
                &scope,
                &example.child("Child").to_provider_key(),
                &example.to_provider_key(),
                V64
            )
            .unwrap_err()
            .kind,
        FsErrorKind::Exists
    );
    let mode = example.value_provider_key("Mode");
    assert_eq!(
        provider
            .copy(&scope, &mode, &example.value_provider_key(""), V64)
            .unwrap_err()
            .kind,
        FsErrorKind::Exists
    );
    assert_eq!(
        provider
            .copy(&scope, &mode, &software.to_provider_key(), V64)
            .unwrap_err()
            .kind,
        FsErrorKind::InvalidPath
    );
    assert_eq!(
        provider
            .copy(
                &scope,
                &example.value_provider_key("None"),
                &software.value_provider_key("x"),
                V64
            )
            .unwrap_err()
            .kind,
        FsErrorKind::NotFound
    );
    let copy = software.child("Copy");
    provider
        .copy(&scope, &mode, &copy.value_provider_key("Mode"), V64)
        .unwrap();
    assert_eq!(
        provider.get(&copy.value_provider_key("Mode")).unwrap(),
        Some(ProviderValue::U32(1))
    );
}

#[test]
fn registry_snapshot_too_large_marks_irreversible() {
    let Fixture { registry, provider } = fixture();
    let example = key("registry:/HKCU/Software/Example").to_provider_key();
    let preimage = provider.preserve(&example, Some(16)).unwrap();
    assert!(preimage.snapshot.is_none());
    assert_eq!(preimage.irreversible.len(), 1);
    assert_eq!(
        preimage.irreversible[0].reason,
        Irreversible::TooLargeForJournal
    );
    assert_eq!(
        preimage.irreversible[0].path.as_deref(),
        Some("registry:/HKCU/Software/Example")
    );
    let blob = value_key("registry:/HKCU/Software/Example/Child/Blob");
    assert!(
        provider
            .preserve(&blob, Some(1))
            .unwrap()
            .snapshot
            .is_none()
    );
    assert!(
        provider
            .preserve(&blob, Some(2))
            .unwrap()
            .snapshot
            .is_some()
    );
    let session = key("registry:/HKCU/Software/Example/Session");
    registry.create_key(V64, &session, "", true).unwrap();
    let preimage = provider.preserve(&example, Some(1 << 20)).unwrap();
    assert!(preimage.snapshot.is_some());
    assert_eq!(
        preimage.irreversible[0].reason,
        Irreversible::PreviousContentNotKept
    );
    assert_eq!(
        preimage.irreversible[0].detail.as_deref(),
        Some("volatile key")
    );
}

#[test]
fn snapshot_of_link_key_is_kept_unchanged_by_undo() {
    let Fixture { registry, provider } = fixture();
    let link = key("registry:/HKCU/Software/Example/Link");
    registry.create_key(V64, &link, "", false).unwrap();
    registry
        .make_link(V64, &link, "\\REGISTRY\\USER\\Target")
        .unwrap();
    let preimage = provider.preserve(&link.to_provider_key(), None).unwrap();
    assert_eq!(
        preimage.irreversible[0].detail.as_deref(),
        Some("symbolic link key")
    );
    let before = dump(&registry);
    undo(&provider, &[preimage.snapshot.unwrap()]);
    assert_eq!(dump(&registry), before);
}

#[test]
fn registry_environment_write_triggers_broadcast_flag() {
    let registry = Arc::new(MemRegistry::new());
    let notifier = Arc::new(CountingNotifier::default());
    let provider = RegistryProvider::new(registry.clone()).with_notifier(notifier.clone());
    let editor = value_key("registry:/HKCU/Environment/EDITOR");
    let snapshots = preserve(&[(&provider, editor.clone())]);
    provider
        .set(&scope(), &editor, &ProviderValue::Text("code".into()))
        .unwrap();
    assert_eq!(notifier.0.load(Ordering::SeqCst), 1);
    provider
        .set(
            &scope(),
            &value_key("registry:/HKCU/Software/Example/x"),
            &ProviderValue::U32(1),
        )
        .unwrap();
    assert_eq!(notifier.0.load(Ordering::SeqCst), 1);
    undo(&provider, &snapshots);
    assert_eq!(notifier.0.load(Ordering::SeqCst), 2);
    let machine =
        value_key("registry:/HKLM/SYSTEM/CurrentControlSet/Control/Session Manager/Environment/X");
    provider
        .set(&scope(), &machine, &ProviderValue::Text("1".into()))
        .unwrap();
    provider.remove(&scope(), &machine).unwrap();
    assert_eq!(notifier.0.load(Ordering::SeqCst), 4);
}

#[cfg(not(windows))]
#[test]
fn registry_path_on_linux_os_only() {
    let error = bind_path("registry:/HKCU/Software/Example").unwrap_err();
    assert_eq!(error.code(), ErrorCode::OsOnly);
    let malformed = bind_path("registry:/NOPE").unwrap_err();
    assert_eq!(malformed.code(), ErrorCode::OsOnly);
}

#[cfg(windows)]
#[test]
fn registry_path_binds_on_windows() {
    assert!(bind_path("registry:/HKCU/Software/Example").is_ok());
    let error = bind_path("registry:/NOPE").unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn resolve_info_and_read_follow_stored_spelling() {
    let Fixture { provider, .. } = fixture();
    let path = RegistryPath::parse("registry:/hkcu/software/example/child").unwrap();
    assert!(matches!(provider.resolve(&path).unwrap(), Address::Key(_)));
    let RegistryItem::Key(entry) = provider.info(&path).unwrap() else {
        panic!("a key is described by an entry");
    };
    assert_eq!(entry.path.as_str(), "registry:/HKCU/Software/Example/Child");
    assert_eq!(entry.exists, Some(true));
    assert_eq!(
        entry.parent.unwrap().as_str(),
        "registry:/HKCU/Software/Example"
    );
    let value = RegistryPath::parse("registry:/HKCU/Software/Example/mode").unwrap();
    let RegistryItem::Value(record) = provider.info(&value).unwrap() else {
        panic!("a value is described by a registry_value");
    };
    assert_eq!(record.name, "Mode");
    assert_eq!(record.path.as_str(), "registry:/HKCU/Software/Example");
    assert_eq!(
        provider.read(&path).unwrap_err().kind,
        FsErrorKind::IsADirectory
    );
    let missing = RegistryPath::parse("registry:/HKCU/Software/Example/Missing").unwrap();
    assert_eq!(
        provider.resolve(&missing).unwrap_err().kind,
        FsErrorKind::NotFound
    );
    let default = RegistryPath::parse("registry:/HKCU/Software/Example/Child/(default)").unwrap();
    assert_eq!(
        provider.read(&default).unwrap_err().kind,
        FsErrorKind::NotFound
    );
}

#[test]
fn provider_interface_lists_and_reads() {
    let Fixture { provider, .. } = fixture();
    assert_eq!(provider.scheme(), ProviderScheme("registry".into()));
    assert!(provider.caps().writable && provider.caps().hierarchical);
    let example = key("registry:/HKCU/Software/Example");
    let children = Provider::list(&provider, &example.to_provider_key()).unwrap();
    assert_eq!(children.len(), 3);
    assert!(children[0].is_key);
    assert_eq!(children[1].key.value.as_deref(), Some(""));
    assert_eq!(
        Provider::list(&provider, &example.value_provider_key("Mode"))
            .unwrap_err()
            .kind,
        FsErrorKind::NotADirectory
    );
    assert_eq!(
        provider.get(&example.to_provider_key()).unwrap_err().kind,
        FsErrorKind::IsADirectory
    );
    assert_eq!(
        provider
            .set(&scope(), &example.to_provider_key(), &ProviderValue::U32(1))
            .unwrap_err()
            .kind,
        FsErrorKind::IsADirectory
    );
    let foreign = ProviderKey {
        scheme: ProviderScheme("env".into()),
        segments: vec!["PATH".into()],
        value: None,
    };
    assert_eq!(
        provider.get(&foreign).unwrap_err().kind,
        FsErrorKind::InvalidPath
    );
    assert_eq!(
        provider.get(&value_key("registry:/HKCU/Absent/x")).unwrap(),
        None
    );
    assert!(format!("{provider:?}").contains("RegistryProvider"));
}

#[test]
fn restore_rejects_corrupt_snapshots() {
    let Fixture { provider, .. } = fixture();
    let mut snapshot = provider
        .snapshot(&key("registry:/HKCU/Software/Example").to_provider_key())
        .unwrap();
    snapshot.subtree = Some(b"{}".to_vec());
    assert_eq!(
        provider.restore(&scope(), &snapshot).unwrap_err().kind,
        FsErrorKind::Io
    );
    snapshot.subtree = None;
    assert_eq!(
        provider.restore(&scope(), &snapshot).unwrap_err().kind,
        FsErrorKind::Io
    );
    let root = provider
        .snapshot(&KeyPath::root(Hive::Users).to_provider_key())
        .unwrap();
    assert!(root.existed);
    assert_eq!(
        provider.restore(&scope(), &root).unwrap_err().kind,
        FsErrorKind::PermissionDenied
    );
}

#[test]
fn subtree_document_matches_fixture() {
    let registry = Arc::new(MemRegistry::new());
    let example = key("registry:/HKCU/Software/Example");
    registry
        .create_key(V64, &example.prefix(1), "", false)
        .unwrap();
    registry.create_key(V64, &example, "", false).unwrap();
    registry
        .set_value(V64, &example, "Mode", &RawValue::dword(1))
        .unwrap();
    registry
        .create_key(V64, &example.child("Child"), "", false)
        .unwrap();
    registry
        .set_value(V64, &example.child("Child"), "", &RawValue::string("x"))
        .unwrap();
    let provider = RegistryProvider::new(registry);
    let snapshot = provider.snapshot(&example.to_provider_key()).unwrap();
    let actual: serde_json::Value = serde_json::from_slice(&snapshot.subtree.unwrap()).unwrap();
    let fixture = std::fs::read(
        StdPath::new(
            std::env::var("CARGO_MANIFEST_DIR")
                .as_deref()
                .unwrap_or(env!("CARGO_MANIFEST_DIR")),
        )
        .join("tests/fixtures/registry/subtree.json"),
    )
    .unwrap();
    let expected: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn catalog_signatures_match_frozen() {
    let root = StdPath::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../catalog");
    let core = command_table(&root.join("core.toml"));
    let write = &core["write"];
    let value_type = param(write, "value_type");
    assert_eq!(
        value_type["type"].as_str(),
        Some("enum(string|dword|qword|multi_string|expand_string|binary)")
    );
    let values: Vec<&str> = value_type["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(values, WRITABLE_TYPES);
    for name in ["list", "find", "read", "info", "remove", "copy", "move"] {
        assert!(core.contains_key(name), "core command {name}");
    }
    let admin = command_table(&root.join("administration.toml"));
    let scope = param(&admin["services"], "scope");
    assert_eq!(scope["type"].as_str(), Some("enum(system|user)"));
    assert_eq!(scope["default"].as_str(), Some("system"));
    for name in ["start", "restart"] {
        let wait = param(&admin[name], "wait");
        assert_eq!(wait["type"].as_str(), Some("duration"));
        assert_eq!(wait["default"].as_str(), Some("10s"));
    }
    for name in ["start", "restart", "enable", "disable"] {
        let input = admin[name]["input"].as_array().unwrap();
        assert_eq!(input[0].as_str(), Some("service"), "input of {name}");
    }
}

fn command_table(path: &StdPath) -> toml::Table {
    let text = std::fs::read_to_string(path).unwrap();
    let catalog: toml::Table = toml::from_str(&text).unwrap();
    catalog["command"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| {
            let name = command["name"].as_str().unwrap().to_owned();
            (name, command.clone())
        })
        .collect()
}

fn param<'a>(command: &'a toml::Value, name: &str) -> &'a toml::Value {
    command["param"]
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["name"].as_str() == Some(name))
        .unwrap_or_else(|| panic!("parameter {name}"))
}
