// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::registry::api
// Responsibility:   Primitive registry operations behind the provider.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Primitive registry operations behind the provider.
//!
//! [`RegistryApi`] is implemented by the Win32 back end and by the in-memory
//! registry used in tests ([`MemRegistry`](super::mem::MemRegistry)). Every
//! operation addresses a key by its path and a view; names are compared
//! without regard to case, and returned names keep the spelling stored in the
//! registry.

use nurt_contracts::fs::FsResult;
use nurt_contracts::record::RegistryView;
use nurt_contracts::value::Instant;

use super::path::KeyPath;
use super::value::RawValue;

/// Facts about one key, read without following a symbolic link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInfo {
    /// Class name given when the key was created; usually empty.
    pub class: String,
    /// Number of subkeys.
    pub subkeys: u32,
    /// Number of values.
    pub values: u32,
    /// Time of the last write to the key.
    pub last_write: Instant,
    /// The key exists only until the system restarts (`REG_OPTION_VOLATILE`).
    pub volatile: bool,
    /// The key is a symbolic link to another key (`REG_LINK`).
    pub link: bool,
}

/// Primitive operations on the registry.
///
/// Implementations open keys with `REG_OPTION_OPEN_LINK`, so a symbolic link
/// key is reported as itself and never followed.
///
/// # Errors
///
/// Every method fails with an [`FsError`](nurt_contracts::fs::FsError) whose
/// kind is `not_found` for a missing key, `permission_denied` for refused
/// access, `privilege` for a missing privilege and `io` for any other error
/// of the system.
pub trait RegistryApi: Send + Sync {
    /// Facts about a key; `None` when the key does not exist.
    fn key_info(&self, view: RegistryView, key: &KeyPath) -> FsResult<Option<KeyInfo>>;

    /// Names of the subkeys, in the order of the registry.
    fn subkeys(&self, view: RegistryView, key: &KeyPath) -> FsResult<Vec<String>>;

    /// Names and data of the values, in the order of the registry.
    fn values(&self, view: RegistryView, key: &KeyPath) -> FsResult<Vec<(String, RawValue)>>;

    /// One value; `None` when the key has no value of that name.
    fn value(&self, view: RegistryView, key: &KeyPath, name: &str) -> FsResult<Option<RawValue>>;

    /// Creates the last key of `key` below its existing parent and reports
    /// whether it was created (`false` when it already existed).
    ///
    /// A missing parent is `not_found`.
    fn create_key(
        &self,
        view: RegistryView,
        key: &KeyPath,
        class: &str,
        volatile: bool,
    ) -> FsResult<bool>;

    /// Sets a value, replacing an existing value of the same name.
    fn set_value(
        &self,
        view: RegistryView,
        key: &KeyPath,
        name: &str,
        value: &RawValue,
    ) -> FsResult<()>;

    /// Deletes a value; a missing value is `not_found`.
    fn delete_value(&self, view: RegistryView, key: &KeyPath, name: &str) -> FsResult<()>;

    /// Deletes a key with all its subkeys and values; a missing key is
    /// reported as `not_found`.
    fn delete_tree(&self, view: RegistryView, key: &KeyPath) -> FsResult<()>;

    /// Self-relative security descriptor with the discretionary access
    /// control list of the key; `None` when it cannot be read.
    fn security(&self, view: RegistryView, key: &KeyPath) -> FsResult<Option<Vec<u8>>>;

    /// Replaces the discretionary access control list of the key with the
    /// one in a self-relative security descriptor.
    fn set_security(&self, view: RegistryView, key: &KeyPath, descriptor: &[u8]) -> FsResult<()>;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
