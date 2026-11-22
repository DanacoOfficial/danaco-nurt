// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-memory registry with the semantics of the Win32 registry.
//!
//! [`MemRegistry`] implements [`RegistryApi`] for tests of the provider and
//! of the journal on every operating system. It reproduces the behaviour the
//! provider depends on: names compared without regard to case and returned
//! in their stored spelling, subkeys and values in creation order, the time
//! of the last write of each key, volatile keys, symbolic link keys, access
//! control and the redirection of `HKLM\SOFTWARE` to
//! `HKLM\SOFTWARE\WOW6432Node` in the 32-bit view.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult};
use nurt_contracts::record::RegistryView;
use nurt_contracts::value::Instant;

use super::api::{KeyInfo, RegistryApi};
use super::path::{Hive, KeyPath, names_equal};
use super::value::{REG_LINK, RawValue};

/// Key below `HKLM` whose content the 32-bit view redirects.
pub const REDIRECTED_KEY: &str = "SOFTWARE";

/// Key below [`REDIRECTED_KEY`] that holds the 32-bit view.
pub const WOW64_NODE: &str = "WOW6432Node";

/// Value of a symbolic link key that holds its target.
pub const LINK_VALUE: &str = "SymbolicLinkValue";

#[derive(Debug, Clone)]
struct Node {
    name: String,
    class: String,
    values: Vec<(String, RawValue)>,
    subkeys: Vec<Node>,
    last_write: Instant,
    volatile: bool,
    link: bool,
    security: Option<Vec<u8>>,
    denied: bool,
}

impl Node {
    fn new(name: &str, class: &str, volatile: bool, now: Instant) -> Self {
        Self {
            name: name.to_owned(),
            class: class.to_owned(),
            values: Vec::new(),
            subkeys: Vec::new(),
            last_write: now,
            volatile,
            link: false,
            security: None,
            denied: false,
        }
    }

    fn subkey(&self, name: &str) -> Option<&Self> {
        self.subkeys
            .iter()
            .find(|node| names_equal(&node.name, name))
    }

    fn subkey_mut(&mut self, name: &str) -> Option<&mut Self> {
        self.subkeys
            .iter_mut()
            .find(|node| names_equal(&node.name, name))
    }

    fn value_index(&self, name: &str) -> Option<usize> {
        self.values
            .iter()
            .position(|(stored, _)| names_equal(stored, name))
    }
}

#[derive(Debug)]
struct State {
    roots: Vec<(Hive, Node)>,
    now: Instant,
}

impl State {
    fn root(&self, hive: Hive) -> Option<&Node> {
        self.roots
            .iter()
            .find(|(root, _)| *root == hive)
            .map(|(_, node)| node)
    }

    fn root_mut(&mut self, hive: Hive) -> Option<&mut Node> {
        self.roots
            .iter_mut()
            .find(|(root, _)| *root == hive)
            .map(|(_, node)| node)
    }

    fn node(&self, key: &KeyPath) -> FsResult<Option<&Node>> {
        let mut node = self.root(key.hive());
        for name in key.keys() {
            match node {
                Some(current) if current.denied => return Err(denied(key)),
                Some(current) => node = current.subkey(name),
                None => return Ok(None),
            }
        }
        match node {
            Some(current) if current.denied => Err(denied(key)),
            other => Ok(other),
        }
    }

    fn node_mut(&mut self, key: &KeyPath) -> FsResult<&mut Node> {
        let mut node = self.root_mut(key.hive()).ok_or_else(|| missing(key))?;
        for name in key.keys() {
            if node.denied {
                return Err(denied(key));
            }
            node = node.subkey_mut(name).ok_or_else(|| missing(key))?;
        }
        if node.denied {
            return Err(denied(key));
        }
        Ok(node)
    }

    fn existing(&self, key: &KeyPath) -> FsResult<&Node> {
        self.node(key)?.ok_or_else(|| missing(key))
    }
}

/// Registry kept in memory.
///
/// Every root key exists and is empty except `HKLM\SOFTWARE` and
/// `HKLM\SOFTWARE\WOW6432Node`, which the 32-bit view needs. The time of
/// the last write is the time set by [`MemRegistry::set_time`], so that tests
/// are deterministic.
#[derive(Debug)]
pub struct MemRegistry {
    state: Mutex<State>,
}

impl Default for MemRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MemRegistry {
    /// Empty registry with the time of the last write at the Unix epoch.
    pub fn new() -> Self {
        let now = Instant::UNIX_EPOCH;
        let roots = Hive::ALL
            .into_iter()
            .map(|hive| {
                let mut root = Node::new(hive.short_name(), "", false, now);
                if hive == Hive::LocalMachine {
                    let mut software = Node::new(REDIRECTED_KEY, "", false, now);
                    software.subkeys.push(Node::new(WOW64_NODE, "", false, now));
                    root.subkeys.push(software);
                }
                (hive, root)
            })
            .collect();
        Self {
            state: Mutex::new(State { roots, now }),
        }
    }

    /// Sets the time recorded as the last write of keys changed from now on.
    ///
    /// # Errors
    ///
    /// Fails with `io` when the registry state is unavailable after a panic in another
    /// thread.
    pub fn set_time(&self, now: Instant) -> FsResult<()> {
        self.lock()?.now = now;
        Ok(())
    }

    /// Turns an existing empty key into a symbolic link to `target`, given as
    /// the native path of the target (`\REGISTRY\MACHINE\…`).
    ///
    /// # Errors
    ///
    /// Fails with `not_found` when the key does not exist.
    pub fn make_link(&self, view: RegistryView, key: &KeyPath, target: &str) -> FsResult<()> {
        let mut state = self.lock()?;
        let node = state.node_mut(&physical(view, key))?;
        node.link = true;
        node.values
            .push((LINK_VALUE.to_owned(), link_target(target)));
        Ok(())
    }

    /// Makes every access to an existing key fail with `permission_denied`.
    ///
    /// # Errors
    ///
    /// Fails with `not_found` when the key does not exist.
    pub fn deny(&self, view: RegistryView, key: &KeyPath) -> FsResult<()> {
        let mut state = self.lock()?;
        state.node_mut(&physical(view, key))?.denied = true;
        Ok(())
    }

    fn lock(&self) -> FsResult<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| FsError::new(FsErrorKind::Io, "registry", None))
    }
}

impl RegistryApi for MemRegistry {
    fn key_info(&self, view: RegistryView, key: &KeyPath) -> FsResult<Option<KeyInfo>> {
        let state = self.lock()?;
        Ok(state.node(&physical(view, key))?.map(|node| KeyInfo {
            class: node.class.clone(),
            subkeys: count(node.subkeys.len()),
            values: count(node.values.len()),
            last_write: node.last_write,
            volatile: node.volatile,
            link: node.link,
        }))
    }

    fn subkeys(&self, view: RegistryView, key: &KeyPath) -> FsResult<Vec<String>> {
        let state = self.lock()?;
        let node = state.existing(&physical(view, key))?;
        Ok(node
            .subkeys
            .iter()
            .map(|child| child.name.clone())
            .collect())
    }

    fn values(&self, view: RegistryView, key: &KeyPath) -> FsResult<Vec<(String, RawValue)>> {
        let state = self.lock()?;
        Ok(state.existing(&physical(view, key))?.values.clone())
    }

    fn value(&self, view: RegistryView, key: &KeyPath, name: &str) -> FsResult<Option<RawValue>> {
        let state = self.lock()?;
        let node = state.existing(&physical(view, key))?;
        Ok(node
            .value_index(name)
            .and_then(|index| node.values.get(index))
            .map(|(_, value)| value.clone()))
    }

    fn create_key(
        &self,
        view: RegistryView,
        key: &KeyPath,
        class: &str,
        volatile: bool,
    ) -> FsResult<bool> {
        let key = physical(view, key);
        let parent_path = key.parent().ok_or_else(|| denied(&key))?;
        let mut state = self.lock()?;
        let now = state.now;
        let parent = state.node_mut(&parent_path)?;
        if parent.subkey(key.name()).is_some() {
            return Ok(false);
        }
        if parent.volatile && !volatile {
            return Err(FsError::new(
                FsErrorKind::Unsupported,
                "create_key",
                Some(Path::new(&key.to_language_string())),
            ));
        }
        parent
            .subkeys
            .push(Node::new(key.name(), class, volatile, now));
        parent.last_write = now;
        Ok(true)
    }

    fn set_value(
        &self,
        view: RegistryView,
        key: &KeyPath,
        name: &str,
        value: &RawValue,
    ) -> FsResult<()> {
        let mut state = self.lock()?;
        let now = state.now;
        let node = state.node_mut(&physical(view, key))?;
        match node.value_index(name) {
            Some(index) => {
                if let Some((_, stored)) = node.values.get_mut(index) {
                    *stored = value.clone();
                }
            }
            None => node.values.push((name.to_owned(), value.clone())),
        }
        node.last_write = now;
        Ok(())
    }

    fn delete_value(&self, view: RegistryView, key: &KeyPath, name: &str) -> FsResult<()> {
        let mut state = self.lock()?;
        let now = state.now;
        let node = state.node_mut(&physical(view, key))?;
        let index = node.value_index(name).ok_or_else(|| missing(key))?;
        node.values.remove(index);
        node.last_write = now;
        Ok(())
    }

    fn delete_tree(&self, view: RegistryView, key: &KeyPath) -> FsResult<()> {
        let key = physical(view, key);
        let parent_path = key.parent().ok_or_else(|| denied(&key))?;
        let mut state = self.lock()?;
        let now = state.now;
        state.node_mut(&key)?;
        let parent = state.node_mut(&parent_path)?;
        parent
            .subkeys
            .retain(|node| !names_equal(&node.name, key.name()));
        parent.last_write = now;
        Ok(())
    }

    fn security(&self, view: RegistryView, key: &KeyPath) -> FsResult<Option<Vec<u8>>> {
        let state = self.lock()?;
        Ok(state.existing(&physical(view, key))?.security.clone())
    }

    fn set_security(&self, view: RegistryView, key: &KeyPath, descriptor: &[u8]) -> FsResult<()> {
        let mut state = self.lock()?;
        state.node_mut(&physical(view, key))?.security = Some(descriptor.to_vec());
        Ok(())
    }
}

/// Path of `key` in the single tree of the in-memory registry: in the
/// 32-bit view, `HKLM\SOFTWARE\…` is `HKLM\SOFTWARE\WOW6432Node\…`.
fn physical(view: RegistryView, key: &KeyPath) -> KeyPath {
    let keys = key.keys();
    let redirected = view == RegistryView::Bits32
        && key.hive() == Hive::LocalMachine
        && keys
            .first()
            .is_some_and(|first| names_equal(first, REDIRECTED_KEY))
        && !keys
            .get(1)
            .is_some_and(|second| names_equal(second, WOW64_NODE));
    if !redirected {
        return key.clone();
    }
    let mut mapped = Vec::with_capacity(keys.len() + 1);
    mapped.extend(keys.first().cloned());
    mapped.push(WOW64_NODE.to_owned());
    mapped.extend(keys.iter().skip(1).cloned());
    KeyPath::new(key.hive(), mapped)
}

fn link_target(target: &str) -> RawValue {
    let bytes = target.encode_utf16().flat_map(u16::to_le_bytes).collect();
    RawValue::new(REG_LINK, bytes)
}

fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

fn missing(key: &KeyPath) -> FsError {
    FsError::new(
        FsErrorKind::NotFound,
        "registry",
        Some(Path::new(&key.to_language_string())),
    )
}

fn denied(key: &KeyPath) -> FsError {
    FsError::new(
        FsErrorKind::PermissionDenied,
        "registry",
        Some(Path::new(&key.to_language_string())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::value::RawValue;

    const V64: RegistryView = RegistryView::Bits64;
    const V32: RegistryView = RegistryView::Bits32;

    fn key(hive: Hive, keys: &[&str]) -> KeyPath {
        KeyPath::new(hive, keys.iter().map(|name| (*name).to_owned()).collect())
    }

    #[test]
    fn names_are_case_insensitive_and_keep_spelling() {
        let registry = MemRegistry::new();
        let software = key(Hive::CurrentUser, &["Software"]);
        assert!(registry.create_key(V64, &software, "", false).unwrap());
        assert!(
            !registry
                .create_key(V64, &key(Hive::CurrentUser, &["SOFTWARE"]), "", false)
                .unwrap()
        );
        registry
            .set_value(V64, &software, "Mode", &RawValue::dword(1))
            .unwrap();
        registry
            .set_value(
                V64,
                &key(Hive::CurrentUser, &["software"]),
                "MODE",
                &RawValue::dword(2),
            )
            .unwrap();
        let values = registry.values(V64, &software).unwrap();
        assert_eq!(values, [("Mode".to_owned(), RawValue::dword(2))]);
        assert_eq!(
            registry
                .subkeys(V64, &KeyPath::root(Hive::CurrentUser))
                .unwrap(),
            ["Software"]
        );
        assert_eq!(
            registry.value(V64, &software, "mode").unwrap(),
            Some(RawValue::dword(2))
        );
        assert_eq!(registry.value(V64, &software, "none").unwrap(), None);
    }

    #[test]
    fn view_32_redirects_software() {
        let registry = MemRegistry::new();
        let vendor = key(Hive::LocalMachine, &["SOFTWARE", "Vendor"]);
        registry.create_key(V32, &vendor, "", false).unwrap();
        assert!(registry.key_info(V64, &vendor).unwrap().is_none());
        assert!(
            registry
                .key_info(
                    V64,
                    &key(Hive::LocalMachine, &["SOFTWARE", "WOW6432Node", "Vendor"])
                )
                .unwrap()
                .is_some()
        );
        assert!(
            registry
                .key_info(
                    V32,
                    &key(Hive::LocalMachine, &["SOFTWARE", "WOW6432Node", "Vendor"])
                )
                .unwrap()
                .is_some()
        );
        let user = key(Hive::CurrentUser, &["SOFTWARE"]);
        registry.create_key(V32, &user, "", false).unwrap();
        assert!(registry.key_info(V64, &user).unwrap().is_some());
    }

    #[test]
    fn missing_and_denied_keys_report_errors() {
        let registry = MemRegistry::new();
        let absent = key(Hive::Users, &["absent", "child"]);
        assert!(registry.key_info(V64, &absent).unwrap().is_none());
        let not_found = registry.subkeys(V64, &absent).unwrap_err();
        assert_eq!(not_found.kind, FsErrorKind::NotFound);
        assert_eq!(
            registry
                .create_key(V64, &absent, "", false)
                .unwrap_err()
                .kind,
            FsErrorKind::NotFound
        );
        assert_eq!(
            registry
                .delete_tree(V64, &KeyPath::root(Hive::Users))
                .unwrap_err()
                .kind,
            FsErrorKind::PermissionDenied
        );
        let secret = key(Hive::Users, &["secret"]);
        registry.create_key(V64, &secret, "", false).unwrap();
        registry.deny(V64, &secret).unwrap();
        assert_eq!(
            registry.values(V64, &secret).unwrap_err().kind,
            FsErrorKind::PermissionDenied
        );
        assert_eq!(
            registry.key_info(V64, &secret.child("x")).unwrap_err().kind,
            FsErrorKind::PermissionDenied
        );
        assert_eq!(
            registry
                .create_key(V64, &secret.child("x"), "", false)
                .unwrap_err()
                .kind,
            FsErrorKind::PermissionDenied
        );
        assert_eq!(
            registry
                .delete_value(V64, &KeyPath::root(Hive::Users), "x")
                .unwrap_err()
                .kind,
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn volatile_parent_requires_volatile_children() {
        let registry = MemRegistry::new();
        let session = key(Hive::CurrentUser, &["Session"]);
        registry.create_key(V64, &session, "", true).unwrap();
        let error = registry
            .create_key(V64, &session.child("persistent"), "", false)
            .unwrap_err();
        assert_eq!(error.kind, FsErrorKind::Unsupported);
        assert!(
            registry
                .create_key(V64, &session.child("volatile"), "", true)
                .unwrap()
        );
        assert!(registry.key_info(V64, &session).unwrap().unwrap().volatile);
    }

    #[test]
    fn writes_update_last_write_time() {
        let registry = MemRegistry::new();
        let app = key(Hive::CurrentUser, &["App"]);
        registry.create_key(V64, &app, "", false).unwrap();
        let later = Instant::from_unix_nanos(5_000);
        registry.set_time(later).unwrap();
        registry
            .set_value(V64, &app, "x", &RawValue::dword(1))
            .unwrap();
        assert_eq!(
            registry.key_info(V64, &app).unwrap().unwrap().last_write,
            later
        );
        registry.delete_value(V64, &app, "X").unwrap();
        assert!(registry.values(V64, &app).unwrap().is_empty());
        registry.set_security(V64, &app, &[1, 2]).unwrap();
        assert_eq!(registry.security(V64, &app).unwrap(), Some(vec![1, 2]));
        registry.delete_tree(V64, &app).unwrap();
        assert!(registry.key_info(V64, &app).unwrap().is_none());
        assert_eq!(
            registry.delete_tree(V64, &app).unwrap_err().kind,
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn link_keys_carry_their_target() {
        let registry = MemRegistry::default();
        let link = key(Hive::CurrentUser, &["Link"]);
        registry.create_key(V64, &link, "", false).unwrap();
        registry
            .make_link(V64, &link, "\\REGISTRY\\USER\\S-1-5-18")
            .unwrap();
        let info = registry.key_info(V64, &link).unwrap().unwrap();
        assert!(info.link);
        let target = registry.value(V64, &link, LINK_VALUE).unwrap().unwrap();
        assert_eq!(target.raw_type, REG_LINK);
    }
}
