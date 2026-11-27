// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::registry::access
// Responsibility:   The registry as seen by the file commands.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The registry as seen by the file commands.
//!
//! [`RegistryProvider`] implements [`RegistryAccess`], the interface the file
//! commands use for `registry:` paths: the reading operations of the provider
//! with records as results, and the translation of a path into the provider
//! key that a change declares to the journal.

use std::path::Path as StdPath;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    FsError, FsErrorKind, RegistryAccess, RegistryKind, RegistryNode, RegistryNodeKind,
    RegistryTarget,
};
use nurt_contracts::record::EntryType;
use nurt_contracts::value::Record;

use super::path::{Address, KeyPath, RegistryPath, escape_segment};
use super::{RegistryItem, RegistryProvider, relative_path};

/// Name of the default value in a relative path.
const DEFAULT_VALUE: &str = "(default)";

impl RegistryAccess for RegistryProvider {
    fn read_value(&self, path: &str) -> Result<Record> {
        let value = RegistryProvider::read(self, &parse(path)?)?;
        Ok(Record::from(value))
    }

    fn describe(&self, path: &str) -> Result<Option<Record>> {
        match RegistryProvider::info(self, &parse(path)?) {
            Ok(item) => Ok(Some(item.into_record())),
            Err(error) if error.kind == FsErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn walk(&self, path: &str, max_depth: Option<u32>) -> Result<Vec<RegistryNode>> {
        let root = match self.resolve(&parse(path)?)? {
            Address::Key(key) => key,
            Address::Value(..) => {
                return Err(FsError::new(
                    FsErrorKind::NotADirectory,
                    "walk",
                    Some(StdPath::new(path)),
                )
                .into());
            }
        };
        let depth = max_depth.map(|depth| usize::try_from(depth).unwrap_or(usize::MAX));
        let listing = RegistryProvider::find(self, &root, depth)?;
        Ok(listing
            .items
            .into_iter()
            .map(|item| node(&root, item))
            .collect())
    }

    fn locate(&self, path: &str) -> Result<RegistryTarget> {
        let address = self.resolve(&parse(path)?)?;
        Ok(target(&address))
    }

    fn address(&self, path: &str, kind: RegistryKind) -> Result<RegistryTarget> {
        let parsed = parse(path)?;
        let address = match kind {
            RegistryKind::Key => parsed.as_key().map(Address::Key),
            RegistryKind::Value => parsed
                .as_value()
                .map(|(key, name)| Address::Value(key, name)),
        };
        address.map(|address| target(&address)).ok_or_else(|| {
            let wanted = match kind {
                RegistryKind::Key => "a key",
                RegistryKind::Value => "a value",
            };
            Error::new(
                ErrorCode::InvalidValue,
                format!("`{path}` cannot name {wanted}: check the end of the path"),
            )
            .with_found(path)
        })
    }
}

fn parse(text: &str) -> Result<RegistryPath> {
    RegistryPath::parse(text)
        .map_err(|error| Error::new(ErrorCode::InvalidValue, error.to_string()).with_found(text))
}

fn target(address: &Address) -> RegistryTarget {
    RegistryTarget {
        key: address.to_provider_key(),
        kind: match address {
            Address::Key(_) => RegistryKind::Key,
            Address::Value(..) => RegistryKind::Value,
        },
    }
}

fn node(root: &KeyPath, item: RegistryItem) -> RegistryNode {
    match item {
        RegistryItem::Key(entry) => RegistryNode {
            kind: if entry.kind == EntryType::Link {
                RegistryNodeKind::Link
            } else {
                RegistryNodeKind::Key
            },
            name: entry.name.clone(),
            relative: entry.relative.clone().unwrap_or_default(),
            record: Record::from(*entry),
        },
        RegistryItem::Value(value) => {
            let key_relative = RegistryPath::parse(value.path.as_str())
                .ok()
                .and_then(|path| path.as_key())
                .map(|key| relative_path(root, &key))
                .unwrap_or_default();
            let leaf = if value.name.is_empty() {
                DEFAULT_VALUE.to_owned()
            } else {
                escape_segment(&value.name)
            };
            RegistryNode {
                kind: RegistryNodeKind::Value,
                name: value.name.clone(),
                relative: if key_relative.is_empty() {
                    leaf
                } else {
                    format!("{key_relative}/{leaf}")
                },
                record: Record::from(value),
            }
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
