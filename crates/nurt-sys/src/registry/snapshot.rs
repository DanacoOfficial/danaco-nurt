// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Preserved state of registry keys and values.
//!
//! A snapshot of a key holds its whole subtree: class, values with their
//! type codes and exact bytes, subkeys and the discretionary access control
//! list. The subtree is serialized as JSON in the format
//! `nurt-registry-subtree/1` and stored in
//! [`ProviderSnapshot::subtree`](nurt_contracts::fs::ProviderSnapshot::subtree).
//! A snapshot of a value keeps the value itself in
//! [`ProviderSnapshot::value`](nurt_contracts::fs::ProviderSnapshot::value)
//! and uses the document only for the view and the keys that did not exist.
//!
//! Volatile keys and symbolic link keys are not preserved: their content
//! does not survive a restart, and links cannot be recreated through the
//! registry interface. They are listed in [`Capture::skipped`].

use nurt_contracts::fs::{FsError, FsErrorKind, FsResult};
use nurt_contracts::record::RegistryView;
use nurt_contracts::value::Bytes;
use serde::{Deserialize, Serialize};

use super::api::RegistryApi;
use super::path::KeyPath;
use super::value::RawValue;

/// Format identifier of the serialized subtree.
pub const SUBTREE_FORMAT: &str = "nurt-registry-subtree/1";

/// Key with its values and subkeys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTree {
    /// Name of the key as stored in the registry.
    pub name: String,
    /// Class name of the key; usually empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub class: String,
    /// Values in the order of the registry.
    pub values: Vec<ValueEntry>,
    /// Subkeys in the order of the registry.
    pub subkeys: Vec<KeyTree>,
    /// Self-relative security descriptor with the discretionary access
    /// control list, when it was readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<Bytes>,
}

/// One value of a [`KeyTree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueEntry {
    /// Name; empty for the default value.
    pub name: String,
    /// Type code.
    pub raw_type: u32,
    /// Data exactly as stored, base64 in JSON.
    pub data: Bytes,
}

impl ValueEntry {
    /// Raw form of the value.
    pub fn to_raw(&self) -> RawValue {
        RawValue::new(self.raw_type, self.data.as_slice().to_vec())
    }
}

/// Serialized content of a registry snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtreeDocument {
    /// Always [`SUBTREE_FORMAT`].
    pub format: String,
    /// View the snapshot was taken in.
    pub view: RegistryView,
    /// Number of keys of the path that existed, when a key of the path was
    /// missing; undo deletes the first missing key, which the operation
    /// created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_depth: Option<usize>,
    /// Subtree of a key; absent for values, missing keys and keys that are
    /// not preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<KeyTree>,
}

impl SubtreeDocument {
    /// Serializes the document as JSON.
    ///
    /// # Errors
    ///
    /// Fails with `io` when serialization fails.
    pub fn to_bytes(&self) -> FsResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(|_| corrupt())
    }

    /// Parses a document and checks its format identifier.
    ///
    /// # Errors
    ///
    /// Fails with `io` when the bytes are not a document of the expected format.
    pub fn from_bytes(bytes: &[u8]) -> FsResult<Self> {
        let document: Self = serde_json::from_slice(bytes).map_err(|_| corrupt())?;
        if document.format == SUBTREE_FORMAT {
            Ok(document)
        } else {
            Err(corrupt())
        }
    }
}

/// Why a key was left out of a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The key is volatile.
    Volatile,
    /// The key is a symbolic link.
    Link,
}

/// Key left out of a capture, with its subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Key.
    pub path: KeyPath,
    /// Reason.
    pub reason: SkipReason,
}

/// Result of [`capture`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Captured {
    /// The subtree was read completely.
    Tree(Capture),
    /// The subtree exceeds the byte budget; reading stopped.
    TooLarge,
}

/// Subtree read by [`capture`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// Subtree; `None` when the key itself was skipped.
    pub tree: Option<KeyTree>,
    /// Keys left out.
    pub skipped: Vec<Skipped>,
    /// Logical size of the preserved data in bytes: names, classes, value
    /// data and security descriptors.
    pub bytes: u64,
}

struct Reader<'a> {
    api: &'a dyn RegistryApi,
    view: RegistryView,
    budget: Option<u64>,
    bytes: u64,
    skipped: Vec<Skipped>,
}

/// Reads the subtree of an existing key.
///
/// The `budget` limits the logical size; reading stops as soon as it is
/// exceeded.
///
/// # Errors
///
/// Fails with `not_found` when the key does not exist, and the errors of
/// [`RegistryApi`] for keys that cannot be read.
pub fn capture(
    api: &dyn RegistryApi,
    view: RegistryView,
    key: &KeyPath,
    name: &str,
    budget: Option<u64>,
) -> FsResult<Captured> {
    let mut reader = Reader {
        api,
        view,
        budget,
        bytes: 0,
        skipped: Vec::new(),
    };
    match reader.read(key, name)? {
        Step::OverBudget => Ok(Captured::TooLarge),
        Step::Done(tree) => Ok(Captured::Tree(Capture {
            tree,
            skipped: reader.skipped,
            bytes: reader.bytes,
        })),
    }
}

enum Step {
    Done(Option<KeyTree>),
    OverBudget,
}

impl Reader<'_> {
    fn read(&mut self, key: &KeyPath, name: &str) -> FsResult<Step> {
        let info = self
            .api
            .key_info(self.view, key)?
            .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "snapshot", None))?;
        let reason = if info.link {
            Some(SkipReason::Link)
        } else if info.volatile {
            Some(SkipReason::Volatile)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.skipped.push(Skipped {
                path: key.clone(),
                reason,
            });
            return Ok(Step::Done(None));
        }
        let mut tree = KeyTree {
            name: name.to_owned(),
            class: info.class,
            values: Vec::new(),
            subkeys: Vec::new(),
            security: self.api.security(self.view, key)?.map(Bytes::new),
        };
        let own = text_len(&tree.name)
            + text_len(&tree.class)
            + tree
                .security
                .as_ref()
                .map_or(0, |bytes| len(bytes.as_slice()));
        if !self.charge(own) {
            return Ok(Step::OverBudget);
        }
        for (value_name, raw) in self.api.values(self.view, key)? {
            if !self.charge(text_len(&value_name) + raw.size()) {
                return Ok(Step::OverBudget);
            }
            tree.values.push(ValueEntry {
                name: value_name,
                raw_type: raw.raw_type,
                data: Bytes::new(raw.bytes),
            });
        }
        for child in self.api.subkeys(self.view, key)? {
            match self.read(&key.child(&child), &child)? {
                Step::OverBudget => return Ok(Step::OverBudget),
                Step::Done(Some(subtree)) => tree.subkeys.push(subtree),
                Step::Done(None) => {}
            }
        }
        Ok(Step::Done(Some(tree)))
    }

    fn charge(&mut self, bytes: u64) -> bool {
        self.bytes = self.bytes.saturating_add(bytes);
        self.budget.is_none_or(|budget| self.bytes <= budget)
    }
}

/// Creates `key` below its existing parent with the content of `tree`.
///
/// Subkeys are created before the access control list of their parent is
/// applied, so that a restrictive list does not block the restoration.
///
/// # Errors
///
/// The errors of [`RegistryApi`]; `not_found` when the parent is missing.
pub fn rebuild(
    api: &dyn RegistryApi,
    view: RegistryView,
    key: &KeyPath,
    tree: &KeyTree,
) -> FsResult<()> {
    api.create_key(view, key, &tree.class, false)?;
    for value in &tree.values {
        api.set_value(view, key, &value.name, &value.to_raw())?;
    }
    for subtree in &tree.subkeys {
        rebuild(api, view, &key.child(&subtree.name), subtree)?;
    }
    if let Some(security) = &tree.security {
        api.set_security(view, key, security.as_slice())?;
    }
    Ok(())
}

fn text_len(text: &str) -> u64 {
    len(text.as_bytes())
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

fn corrupt() -> FsError {
    FsError::new(FsErrorKind::Io, "restore", None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::mem::MemRegistry;
    use crate::registry::path::Hive;

    const VIEW: RegistryView = RegistryView::Bits64;

    fn tree_fixture() -> (MemRegistry, KeyPath) {
        let registry = MemRegistry::new();
        let app = KeyPath::new(Hive::CurrentUser, vec!["App".to_owned()]);
        registry.create_key(VIEW, &app, "AppClass", false).unwrap();
        registry
            .set_value(VIEW, &app, "", &RawValue::string("default"))
            .unwrap();
        registry
            .create_key(VIEW, &app.child("Sub"), "", false)
            .unwrap();
        registry
            .set_value(VIEW, &app.child("Sub"), "n", &RawValue::dword(3))
            .unwrap();
        registry.set_security(VIEW, &app, &[1, 0, 4, 128]).unwrap();
        (registry, app)
    }

    #[test]
    fn capture_and_rebuild_reproduce_subtree() {
        let (registry, app) = tree_fixture();
        let Captured::Tree(first) = capture(&registry, VIEW, &app, "App", None).unwrap() else {
            panic!("unexpected budget overflow");
        };
        assert!(first.bytes > 0);
        let tree = first.tree.unwrap();
        assert_eq!(tree.class, "AppClass");
        assert_eq!(tree.subkeys.len(), 1);
        let copy = KeyPath::new(Hive::CurrentUser, vec!["Copy".to_owned()]);
        rebuild(&registry, VIEW, &copy, &tree).unwrap();
        let Captured::Tree(again) = capture(&registry, VIEW, &copy, "App", None).unwrap() else {
            panic!("unexpected budget overflow");
        };
        assert_eq!(again.tree.unwrap(), tree);
    }

    #[test]
    fn capture_stops_at_budget() {
        let (registry, app) = tree_fixture();
        assert_eq!(
            capture(&registry, VIEW, &app, "App", Some(4)).unwrap(),
            Captured::TooLarge
        );
        assert_eq!(
            capture(&registry, VIEW, &app, "App", Some(20)).unwrap(),
            Captured::TooLarge
        );
        let missing = KeyPath::new(Hive::CurrentUser, vec!["None".to_owned()]);
        assert_eq!(
            capture(&registry, VIEW, &missing, "None", None)
                .unwrap_err()
                .kind,
            FsErrorKind::NotFound
        );
    }

    #[test]
    fn document_round_trips_and_checks_format() {
        let document = SubtreeDocument {
            format: SUBTREE_FORMAT.to_owned(),
            view: RegistryView::Bits32,
            existing_depth: Some(1),
            tree: None,
        };
        let bytes = document.to_bytes().unwrap();
        assert_eq!(SubtreeDocument::from_bytes(&bytes).unwrap(), document);
        let other = br#"{"format":"other/1","view":"64"}"#;
        assert_eq!(
            SubtreeDocument::from_bytes(other).unwrap_err().kind,
            FsErrorKind::Io
        );
        assert!(SubtreeDocument::from_bytes(b"[").is_err());
    }
}
