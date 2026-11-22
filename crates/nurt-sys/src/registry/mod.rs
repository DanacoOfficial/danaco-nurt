// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Windows registry provider.
//!
//! The provider serves paths of the form `registry:/<root>/<key>/…/<name>`:
//! a key is shown as a directory (`entry` with `type: directory`) and a value
//! as a `registry_value` record. Reading is offered by [`RegistryProvider`]
//! directly ([`list`](RegistryProvider::list), [`find`](RegistryProvider::find),
//! [`info`](RegistryProvider::info), [`read`](RegistryProvider::read)); every
//! change goes through [`ProviderWrite`] or the transfer operations
//! [`copy`](RegistryProvider::copy) and [`rename`](RegistryProvider::rename),
//! all of which require a transaction scope. Before a change the journal
//! takes a [`Provider::snapshot`] (or [`RegistryProvider::preserve`] with a
//! size limit); undo passes the snapshot to [`ProviderWrite::restore`], which
//! restores type and bytes of values and whole subtrees of keys exactly,
//! including their absence.
//!
//! The logic works on the primitive operations of [`api::RegistryApi`],
//! implemented by the Win32 back end on Windows and by
//! [`mem::MemRegistry`] everywhere, so that it is tested on every system.
//! Outside Windows, paths of the provider are rejected while binding
//! ([`bind_path`]).

pub mod api;
pub mod broadcast;
pub mod mem;
pub mod path;
pub mod snapshot;
pub mod value;
#[cfg(windows)]
mod win32;

use std::path::Path as StdPath;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_contracts::fs::{
    FsError, FsErrorKind, FsResult, Provider, ProviderCaps, ProviderChild, ProviderKey,
    ProviderScheme, ProviderSnapshot, ProviderValue, ProviderWrite, TxScope,
};
use nurt_contracts::record::{Entry, EntryType, LinkType, RegistryValue, RegistryView};
use nurt_contracts::report::{Irreversible, IrreversibleEntry};
use nurt_contracts::value::{Path, Record, Size};

use self::api::{KeyInfo, RegistryApi};
use self::broadcast::{SettingChangeNotifier, affects_environment};
use self::mem::LINK_VALUE;
use self::path::{Address, KeyPath, PathError, RegistryPath, SCHEME, names_equal};
use self::snapshot::{Capture, Captured, SUBTREE_FORMAT, SkipReason, SubtreeDocument};
use self::value::{RawValue, decode_string};

/// Names of the subkeys and the values of one key.
type KeyContent = (Vec<String>, Vec<(String, RawValue)>);

/// Key or value returned by the reading operations of the provider.
#[derive(Debug, Clone, PartialEq)]
pub enum RegistryItem {
    /// A key, as a directory entry.
    Key(Box<Entry>),
    /// A value.
    Value(RegistryValue),
}

impl RegistryItem {
    /// Record of the item: `entry` or `registry_value`.
    pub fn into_record(self) -> Record {
        match self {
            Self::Key(entry) => Record::from(*entry),
            Self::Value(value) => Record::from(value),
        }
    }
}

/// Result of [`RegistryProvider::find`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Listing {
    /// Keys and values found, each key followed by its content.
    pub items: Vec<RegistryItem>,
    /// Keys whose content could not be read; their subtrees are missing
    /// from `items`.
    pub unreadable: Vec<KeyPath>,
}

/// Previous state of a target, taken with a size limit.
#[derive(Debug, Clone, PartialEq)]
pub struct Preimage {
    /// Snapshot for undo; `None` when the state is too large to keep.
    pub snapshot: Option<ProviderSnapshot>,
    /// Parts of the previous state that are not kept.
    pub irreversible: Vec<IrreversibleEntry>,
}

/// Provider of `registry:` paths.
///
/// One instance works in one view of the registry (64-bit by default);
/// [`RegistryProvider::in_view`] gives an instance for the other view over
/// the same registry. Snapshots record their view, so any instance restores
/// them.
#[derive(Clone)]
pub struct RegistryProvider {
    api: Arc<dyn RegistryApi>,
    view: RegistryView,
    notifier: Option<Arc<dyn SettingChangeNotifier>>,
}

impl std::fmt::Debug for RegistryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryProvider")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl RegistryProvider {
    /// Provider over `api` in the 64-bit view, without notification of
    /// changed environment variables.
    pub fn new(api: Arc<dyn RegistryApi>) -> Self {
        Self {
            api,
            view: RegistryView::Bits64,
            notifier: None,
        }
    }

    /// Provider over the registry of the system, announcing changes of
    /// persistent environment variables with `WM_SETTINGCHANGE`.
    #[cfg(windows)]
    pub fn system() -> Self {
        Self::new(Arc::new(win32::Win32Registry)).with_notifier(Arc::new(broadcast::Win32Broadcast))
    }

    /// The same provider with a receiver of the notification sent after a
    /// change below an environment key.
    #[must_use]
    pub fn with_notifier(mut self, notifier: Arc<dyn SettingChangeNotifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    /// Provider over the same registry in `view`.
    #[must_use]
    pub fn in_view(&self, view: RegistryView) -> Self {
        Self {
            view,
            ..self.clone()
        }
    }

    /// View of this instance.
    pub fn view(&self) -> RegistryView {
        self.view
    }

    /// Decides whether a path names a key or a value.
    ///
    /// The last segment names a value when the key before it has a value of
    /// that name, otherwise a key; `(default)` always names the default
    /// value. The result keeps the spelling of the path.
    ///
    /// # Errors
    ///
    /// Fails with `not_found` when neither exists, and the errors of the registry.
    pub fn resolve(&self, path: &RegistryPath) -> FsResult<Address> {
        if let Some((key, name)) = path.as_value() {
            let value_exists = !path.is_default_value()
                && self.key_exists(&key)?
                && self.api.value(self.view, &key, &name)?.is_some();
            if value_exists || path.is_default_value() {
                return Ok(Address::Value(key, name));
            }
        }
        match path.as_key() {
            Some(key) if self.key_exists(&key)? => Ok(Address::Key(key)),
            _ => Err(not_found("resolve", &path.to_string())),
        }
    }

    /// Subkeys and values of a key, keys first, in the order of the
    /// registry.
    ///
    /// # Errors
    ///
    /// Fails with `not_found` for a missing key, and the errors of the registry.
    pub fn list(&self, key: &KeyPath) -> FsResult<Vec<RegistryItem>> {
        self.require_key(key)?;
        let mut items = Vec::new();
        for name in self.api.subkeys(self.view, key)? {
            let child = key.child(&name);
            if let Some(info) = self.api.key_info(self.view, &child)? {
                items.push(self.key_item(&child, &info, None));
            }
        }
        for (name, raw) in self.api.values(self.view, key)? {
            items.push(RegistryItem::Value(self.value_record(key, &name, &raw)));
        }
        Ok(items)
    }

    /// Keys and values below a key, depth first, down to `max_depth` levels, or without
    /// a limit for `None`.
    ///
    /// Symbolic link keys are reported but not entered. Keys whose content
    /// cannot be read are listed in [`Listing::unreadable`].
    ///
    /// # Errors
    ///
    /// Fails with `not_found` for a missing key, and the errors of the registry for the
    /// key itself.
    pub fn find(&self, key: &KeyPath, max_depth: Option<usize>) -> FsResult<Listing> {
        self.require_key(key)?;
        let mut listing = Listing::default();
        self.find_below(key, key, 1, max_depth, &mut listing)?;
        Ok(listing)
    }

    fn find_below(
        &self,
        root: &KeyPath,
        key: &KeyPath,
        depth: usize,
        max_depth: Option<usize>,
        listing: &mut Listing,
    ) -> FsResult<()> {
        if max_depth.is_some_and(|max| depth > max) {
            return Ok(());
        }
        let (names, values) = match self.read_content(key) {
            Ok(content) => content,
            Err(error) if error.kind == FsErrorKind::PermissionDenied && key != root => {
                listing.unreadable.push(key.clone());
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        for (name, raw) in values {
            listing
                .items
                .push(RegistryItem::Value(self.value_record(key, &name, &raw)));
        }
        for name in names {
            let child = key.child(&name);
            let info = match self.api.key_info(self.view, &child) {
                Ok(Some(info)) => info,
                Ok(None) => continue,
                Err(error) if error.kind == FsErrorKind::PermissionDenied => {
                    listing.unreadable.push(child);
                    continue;
                }
                Err(error) => return Err(error),
            };
            listing
                .items
                .push(self.key_item(&child, &info, Some((root, depth))));
            if !info.link {
                self.find_below(root, &child, depth + 1, max_depth, listing)?;
            }
        }
        Ok(())
    }

    fn read_content(&self, key: &KeyPath) -> FsResult<KeyContent> {
        Ok((
            self.api.subkeys(self.view, key)?,
            self.api.values(self.view, key)?,
        ))
    }

    /// Description of a key or value: an `entry` for a key, a
    /// `registry_value` for a value.
    ///
    /// # Errors
    ///
    /// Fails with `not_found` when the path names nothing, and the errors of the
    /// registry.
    pub fn info(&self, path: &RegistryPath) -> FsResult<RegistryItem> {
        match self.resolve(path)? {
            Address::Key(key) => {
                let stored = self.stored_spelling(&key)?;
                let info = self.require_key(&stored)?;
                let mut item = self.key_item(&stored, &info, None);
                if let RegistryItem::Key(entry) = &mut item {
                    entry.exists = Some(true);
                }
                Ok(item)
            }
            Address::Value(key, name) => self.read_value(&key, &name).map(RegistryItem::Value),
        }
    }

    /// Value named by a path.
    ///
    /// # Errors
    ///
    /// Fails with `is_a_directory` when the path names a key, `not_found` when it names
    /// nothing, and the errors of the registry.
    pub fn read(&self, path: &RegistryPath) -> FsResult<RegistryValue> {
        match self.resolve(path)? {
            Address::Value(key, name) => self.read_value(&key, &name),
            Address::Key(key) => Err(fs_error(
                FsErrorKind::IsADirectory,
                "read",
                &key.to_language_string(),
            )),
        }
    }

    fn read_value(&self, key: &KeyPath, name: &str) -> FsResult<RegistryValue> {
        let stored = self.stored_spelling(key)?;
        let values = self.api.values(self.view, &stored)?;
        let (stored_name, raw) = values
            .into_iter()
            .find(|(candidate, _)| names_equal(candidate, name))
            .ok_or_else(|| not_found("read", &key.value_language_string(name)))?;
        Ok(self.value_record(&stored, &stored_name, &raw))
    }

    /// Takes a snapshot of a key or value for the journal, keeping at most
    /// `max_bytes` of data (`journal.max_op_size`).
    ///
    /// A larger state is not kept: the snapshot is `None` and the preimage
    /// carries `too_large_for_journal`. Volatile and symbolic link keys
    /// inside a subtree are left out and carry `previous_content_not_kept`.
    ///
    /// # Errors
    ///
    /// Fails with `permission_denied` or `io` when the state cannot be read.
    pub fn preserve(&self, key: &ProviderKey, max_bytes: Option<u64>) -> FsResult<Preimage> {
        let address = address_of(key)?;
        match address {
            Address::Value(ref path, ref name) => self.preserve_value(key, path, name, max_bytes),
            Address::Key(ref path) => self.preserve_key(key, path, max_bytes),
        }
    }

    fn preserve_value(
        &self,
        key: &ProviderKey,
        path: &KeyPath,
        name: &str,
        max_bytes: Option<u64>,
    ) -> FsResult<Preimage> {
        let existing_depth = self.existing_depth(path)?;
        let raw = if existing_depth.is_none() {
            self.api.value(self.view, path, name)?
        } else {
            None
        };
        if let (Some(raw), Some(limit)) = (&raw, max_bytes)
            && raw.size() > limit
        {
            return Ok(too_large(&path.value_language_string(name)));
        }
        let document = self.document(existing_depth, None);
        Ok(Preimage {
            snapshot: Some(ProviderSnapshot {
                key: key.clone(),
                existed: raw.is_some(),
                value: raw.as_ref().map(RawValue::to_exact_provider_value),
                subtree: Some(document.to_bytes()?),
            }),
            irreversible: Vec::new(),
        })
    }

    fn preserve_key(
        &self,
        key: &ProviderKey,
        path: &KeyPath,
        max_bytes: Option<u64>,
    ) -> FsResult<Preimage> {
        let existing_depth = self.existing_depth(path)?;
        let mut capture = Capture {
            tree: None,
            skipped: Vec::new(),
            bytes: 0,
        };
        if existing_depth.is_none() {
            let stored = self.stored_spelling(path)?;
            match snapshot::capture(&*self.api, self.view, &stored, stored.name(), max_bytes)? {
                Captured::Tree(read) => capture = read,
                Captured::TooLarge => return Ok(too_large(&path.to_language_string())),
            }
        }
        let irreversible = capture.skipped.iter().map(skipped_entry).collect();
        let document = self.document(existing_depth, capture.tree);
        Ok(Preimage {
            snapshot: Some(ProviderSnapshot {
                key: key.clone(),
                existed: existing_depth.is_none(),
                value: None,
                subtree: Some(document.to_bytes()?),
            }),
            irreversible,
        })
    }

    fn document(
        &self,
        existing_depth: Option<usize>,
        tree: Option<snapshot::KeyTree>,
    ) -> SubtreeDocument {
        SubtreeDocument {
            format: SUBTREE_FORMAT.to_owned(),
            view: self.view,
            existing_depth,
            tree,
        }
    }

    /// Copies a key with its subtree, or a value, to `to` in `to_view`;
    /// missing keys above the destination are created.
    ///
    /// The journal declares both the destination and the keys it creates
    /// through a snapshot of `to` taken before the copy.
    ///
    /// # Errors
    ///
    /// Fails with `not_found` for a missing source, `exists` for an existing
    /// destination, `invalid_path` when a key is copied into itself or a key
    /// onto a value, and the errors of the registry.
    pub fn copy(
        &self,
        _scope: &TxScope,
        from: &ProviderKey,
        to: &ProviderKey,
        to_view: RegistryView,
    ) -> FsResult<()> {
        let target = self.in_view(to_view);
        match (address_of(from)?, address_of(to)?) {
            (Address::Value(key, name), Address::Value(to_key, to_name)) => {
                self.copy_value(&target, (&key, &name), (&to_key, &to_name))
            }
            (Address::Key(key), Address::Key(to_key)) => self.copy_key(&target, &key, &to_key),
            (source, _) => Err(fs_error(
                FsErrorKind::InvalidPath,
                "copy",
                &source.to_language_string(),
            )),
        }?;
        target.notify(address_of(to)?.key());
        Ok(())
    }

    fn copy_value(
        &self,
        target: &Self,
        (key, name): (&KeyPath, &str),
        (to_key, to_name): (&KeyPath, &str),
    ) -> FsResult<()> {
        let raw = self
            .key_value(key, name)?
            .ok_or_else(|| not_found("copy", &key.value_language_string(name)))?;
        if target.key_value(to_key, to_name)?.is_some() {
            return Err(fs_error(
                FsErrorKind::Exists,
                "copy",
                &to_key.value_language_string(to_name),
            ));
        }
        target.ensure_keys(to_key)?;
        target.api.set_value(target.view, to_key, to_name, &raw)
    }

    fn copy_key(&self, target: &Self, key: &KeyPath, to_key: &KeyPath) -> FsResult<()> {
        let source = self.stored_spelling(key)?;
        if self.view == target.view && to_key.starts_with(&source) {
            return Err(fs_error(
                FsErrorKind::InvalidPath,
                "copy",
                &to_key.to_language_string(),
            ));
        }
        if target.key_exists(to_key)? {
            return Err(fs_error(
                FsErrorKind::Exists,
                "copy",
                &to_key.to_language_string(),
            ));
        }
        let Captured::Tree(capture) =
            snapshot::capture(&*self.api, self.view, &source, source.name(), None)?
        else {
            return Err(fs_error(
                FsErrorKind::Io,
                "copy",
                &source.to_language_string(),
            ));
        };
        let tree = capture.tree.ok_or_else(|| {
            fs_error(
                FsErrorKind::Unsupported,
                "copy",
                &source.to_language_string(),
            )
        })?;
        let parent = to_key
            .parent()
            .ok_or_else(|| fs_error(FsErrorKind::Exists, "copy", &to_key.to_language_string()))?;
        target.ensure_keys(&parent)?;
        snapshot::rebuild(&*target.api, target.view, to_key, &tree)
    }

    /// Moves a key with its subtree, or a value, to `to` in `to_view`: a copy
    /// followed by the removal of the source, within one operation of the
    /// journal.
    ///
    /// # Errors
    ///
    /// As [`RegistryProvider::copy`], and `permission_denied` for a root key.
    pub fn rename(
        &self,
        scope: &TxScope,
        from: &ProviderKey,
        to: &ProviderKey,
        to_view: RegistryView,
    ) -> FsResult<()> {
        if let Address::Key(key) = address_of(from)?
            && key.is_root()
        {
            return Err(fs_error(
                FsErrorKind::PermissionDenied,
                "move",
                &key.to_language_string(),
            ));
        }
        self.copy(scope, from, to, to_view)?;
        self.remove(scope, from)
    }

    fn key_exists(&self, key: &KeyPath) -> FsResult<bool> {
        Ok(self.api.key_info(self.view, key)?.is_some())
    }

    fn key_value(&self, key: &KeyPath, name: &str) -> FsResult<Option<RawValue>> {
        if self.key_exists(key)? {
            self.api.value(self.view, key, name)
        } else {
            Ok(None)
        }
    }

    fn require_key(&self, key: &KeyPath) -> FsResult<KeyInfo> {
        self.api
            .key_info(self.view, key)?
            .ok_or_else(|| not_found("list", &key.to_language_string()))
    }

    /// Number of keys of the path that exist, or `None` when the whole path
    /// exists.
    fn existing_depth(&self, key: &KeyPath) -> FsResult<Option<usize>> {
        for depth in 1..=key.keys().len() {
            if !self.key_exists(&key.prefix(depth))? {
                return Ok(Some(depth - 1));
            }
        }
        Ok(None)
    }

    fn ensure_keys(&self, key: &KeyPath) -> FsResult<()> {
        for depth in 1..=key.keys().len() {
            self.api
                .create_key(self.view, &key.prefix(depth), "", false)?;
        }
        Ok(())
    }

    /// The path with every name in the spelling stored in the registry.
    fn stored_spelling(&self, key: &KeyPath) -> FsResult<KeyPath> {
        let mut stored = KeyPath::root(key.hive());
        for name in key.keys() {
            let names = self.api.subkeys(self.view, &stored)?;
            let found = names
                .into_iter()
                .find(|candidate| names_equal(candidate, name))
                .ok_or_else(|| not_found("info", &key.to_language_string()))?;
            stored = stored.child(&found);
        }
        Ok(stored)
    }

    fn key_item(
        &self,
        key: &KeyPath,
        info: &KeyInfo,
        listing: Option<(&KeyPath, usize)>,
    ) -> RegistryItem {
        let mut entry = Entry {
            path: Path::new(key.to_language_string()),
            name: key.name().to_owned(),
            parent: key
                .parent()
                .map(|parent| Path::new(parent.to_language_string())),
            kind: EntryType::Directory,
            size: Size::ZERO,
            modified: info.last_write,
            ..Entry::default()
        };
        if info.link {
            entry.kind = EntryType::Link;
            entry.link_type = Some(LinkType::Symbolic);
            entry.link_target = self.link_target(key);
        }
        if let Some((root, depth)) = listing {
            entry.depth = Some(u64::try_from(depth).unwrap_or(u64::MAX));
            entry.relative = Some(relative_path(root, key));
        }
        RegistryItem::Key(Box::new(entry))
    }

    fn link_target(&self, key: &KeyPath) -> Option<Path> {
        let raw = self.api.value(self.view, key, LINK_VALUE).ok()??;
        Some(Path::new(decode_string(&raw.bytes)))
    }

    fn value_record(&self, key: &KeyPath, name: &str, raw: &RawValue) -> RegistryValue {
        RegistryValue {
            path: Path::new(key.to_language_string()),
            name: name.to_owned(),
            kind: raw.registry_type(),
            value: raw.to_language_value(),
            size: Size::from_bytes(raw.size()),
            view: Some(self.view),
        }
    }

    fn notify(&self, key: &KeyPath) {
        if let Some(notifier) = &self.notifier
            && affects_environment(key)
        {
            notifier.environment_changed();
        }
    }

    fn restore_value(
        &self,
        snapshot: &ProviderSnapshot,
        document: &SubtreeDocument,
        key: &KeyPath,
        name: &str,
    ) -> FsResult<()> {
        match &snapshot.value {
            Some(value) if snapshot.existed => {
                self.ensure_keys(key)?;
                self.api
                    .set_value(self.view, key, name, &RawValue::from_provider_value(value))
            }
            _ => {
                if self.key_exists(key)? {
                    ignore_missing(self.api.delete_value(self.view, key, name))?;
                }
                self.delete_created(key, document.existing_depth)
            }
        }
    }

    fn restore_key(
        &self,
        snapshot: &ProviderSnapshot,
        document: &SubtreeDocument,
        key: &KeyPath,
    ) -> FsResult<()> {
        if key.is_root() {
            return Err(fs_error(
                FsErrorKind::PermissionDenied,
                "restore",
                &key.to_language_string(),
            ));
        }
        let Some(tree) = document.tree.as_ref().filter(|_| snapshot.existed) else {
            return if snapshot.existed {
                Ok(())
            } else {
                self.delete_created(key, document.existing_depth)
            };
        };
        if self.key_exists(key)? {
            self.api.delete_tree(self.view, key)?;
        }
        let parent = key.parent().unwrap_or_else(|| KeyPath::root(key.hive()));
        self.ensure_keys(&parent)?;
        snapshot::rebuild(&*self.api, self.view, &parent.child(&tree.name), tree)
    }

    /// Deletes the first key of the path that did not exist when the
    /// snapshot was taken.
    fn delete_created(&self, key: &KeyPath, existing_depth: Option<usize>) -> FsResult<()> {
        match existing_depth {
            Some(depth) if depth < key.keys().len() => {
                let created = key.prefix(depth + 1);
                if self.key_exists(&created)? {
                    ignore_missing(self.api.delete_tree(self.view, &created))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl Provider for RegistryProvider {
    fn scheme(&self) -> ProviderScheme {
        ProviderScheme(SCHEME.to_owned())
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            writable: true,
            hierarchical: true,
            persistent: true,
        }
    }

    fn get(&self, key: &ProviderKey) -> FsResult<Option<ProviderValue>> {
        match address_of(key)? {
            Address::Value(path, name) => Ok(self
                .key_value(&path, &name)?
                .map(|raw| raw.to_provider_value())),
            Address::Key(path) => Err(fs_error(
                FsErrorKind::IsADirectory,
                "get",
                &path.to_language_string(),
            )),
        }
    }

    fn list(&self, key: &ProviderKey) -> FsResult<Vec<ProviderChild>> {
        let Address::Key(path) = address_of(key)? else {
            return Err(fs_error(
                FsErrorKind::NotADirectory,
                "list",
                &address_of(key)?.to_language_string(),
            ));
        };
        self.require_key(&path)?;
        let keys = self
            .api
            .subkeys(self.view, &path)?
            .into_iter()
            .map(|name| ProviderChild {
                key: path.child(&name).to_provider_key(),
                is_key: true,
            });
        let values = self
            .api
            .values(self.view, &path)?
            .into_iter()
            .map(|(name, _)| ProviderChild {
                key: path.value_provider_key(&name),
                is_key: false,
            });
        Ok(keys.chain(values).collect())
    }

    fn snapshot(&self, key: &ProviderKey) -> FsResult<ProviderSnapshot> {
        self.preserve(key, None)?
            .snapshot
            .ok_or_else(|| fs_error(FsErrorKind::Io, "snapshot", &format_key(key)))
    }
}

impl ProviderWrite for RegistryProvider {
    fn set(&self, _scope: &TxScope, key: &ProviderKey, value: &ProviderValue) -> FsResult<()> {
        let Address::Value(path, name) = address_of(key)? else {
            return Err(fs_error(FsErrorKind::IsADirectory, "set", &format_key(key)));
        };
        self.ensure_keys(&path)?;
        self.api.set_value(
            self.view,
            &path,
            &name,
            &RawValue::from_provider_value(value),
        )?;
        self.notify(&path);
        Ok(())
    }

    fn remove(&self, _scope: &TxScope, key: &ProviderKey) -> FsResult<()> {
        let address = address_of(key)?;
        match &address {
            Address::Value(path, name) => {
                if !self.key_exists(path)? {
                    return Err(not_found("remove", &address.to_language_string()));
                }
                self.api.delete_value(self.view, path, name)?;
            }
            Address::Key(path) if path.is_root() => {
                return Err(fs_error(
                    FsErrorKind::PermissionDenied,
                    "remove",
                    &path.to_language_string(),
                ));
            }
            Address::Key(path) => self.api.delete_tree(self.view, path)?,
        }
        self.notify(address.key());
        Ok(())
    }

    fn restore(&self, _scope: &TxScope, snapshot: &ProviderSnapshot) -> FsResult<()> {
        let document = snapshot
            .subtree
            .as_deref()
            .ok_or_else(|| fs_error(FsErrorKind::Io, "restore", &format_key(&snapshot.key)))
            .and_then(SubtreeDocument::from_bytes)?;
        let target = self.in_view(document.view);
        let address = address_of(&snapshot.key)?;
        match &address {
            Address::Value(key, name) => target.restore_value(snapshot, &document, key, name)?,
            Address::Key(key) => target.restore_key(snapshot, &document, key)?,
        }
        target.notify(address.key());
        Ok(())
    }
}

/// Checks a `registry:` path while binding a program.
///
/// # Errors
///
/// Fails with `E_OS_ONLY` outside Windows, whatever the path, and `E_INVALID_VALUE`
/// for a path that does not parse.
pub fn bind_path(text: &str) -> Result<RegistryPath, Error> {
    if !cfg!(windows) {
        return Err(Error::new(
            ErrorCode::OsOnly,
            format!("`{text}` names the Windows registry, which exists only on Windows"),
        )
        .with_stage(Stage::Bind)
        .with_found(text)
        .with_detail("os", serde_json::json!(["windows"])));
    }
    RegistryPath::parse(text).map_err(|error| path_error(text, &error))
}

fn path_error(text: &str, error: &PathError) -> Error {
    Error::new(ErrorCode::InvalidValue, error.to_string())
        .with_stage(Stage::Bind)
        .with_found(text)
}

fn address_of(key: &ProviderKey) -> FsResult<Address> {
    Address::from_provider_key(key)
        .map_err(|_| fs_error(FsErrorKind::InvalidPath, "registry", &format_key(key)))
}

fn format_key(key: &ProviderKey) -> String {
    format!("{}:/{}", key.scheme.0, key.segments.join("/"))
}

fn relative_path(root: &KeyPath, key: &KeyPath) -> String {
    key.keys()
        .iter()
        .skip(root.keys().len())
        .map(|name| path::escape_segment(name))
        .collect::<Vec<_>>()
        .join("/")
}

fn too_large(path: &str) -> Preimage {
    let mut entry = IrreversibleEntry::new(Irreversible::TooLargeForJournal);
    entry.path = Some(path.to_owned());
    Preimage {
        snapshot: None,
        irreversible: vec![entry],
    }
}

fn skipped_entry(skipped: &snapshot::Skipped) -> IrreversibleEntry {
    let mut entry = IrreversibleEntry::new(Irreversible::PreviousContentNotKept);
    entry.path = Some(skipped.path.to_language_string());
    entry.detail = Some(
        match skipped.reason {
            SkipReason::Volatile => "volatile key",
            SkipReason::Link => "symbolic link key",
        }
        .to_owned(),
    );
    entry
}

fn ignore_missing(result: FsResult<()>) -> FsResult<()> {
    match result {
        Err(error) if error.kind == FsErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn fs_error(kind: FsErrorKind, op: &str, path: &str) -> FsError {
    FsError::new(kind, op, Some(StdPath::new(path)))
}

fn not_found(op: &str, path: &str) -> FsError {
    fs_error(FsErrorKind::NotFound, op, path)
}
