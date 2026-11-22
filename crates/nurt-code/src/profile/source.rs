// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Access to the content of manifests: raw bytes and structured values.
//!
//! JSON and TOML manifests are read through the document service of the
//! execution context. Without it, JSON is read directly and TOML manifests
//! contribute only what their presence implies.

use std::collections::BTreeSet;
use std::path::Path;

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{DocFormatId, DocService};
use nurt_contracts::value::Value;

use super::files::{TreeFiles, absolute};

/// Largest manifest read, in bytes.
const MANIFEST_BYTES_MAX: u64 = 4 * 1024 * 1024;

/// Files of the tree and readers of their content.
pub(crate) struct Source<'a> {
    root: &'a Path,
    files: &'a [String],
    present: BTreeSet<&'a str>,
    tree: &'a dyn TreeFiles,
    doc: Option<&'a dyn DocService>,
    unreadable: std::cell::RefCell<BTreeSet<String>>,
}

impl<'a> Source<'a> {
    /// Source over the listed files.
    pub(crate) fn new(
        root: &'a Path,
        files: &'a [String],
        tree: &'a dyn TreeFiles,
        doc: Option<&'a dyn DocService>,
    ) -> Self {
        Self {
            root,
            files,
            present: files.iter().map(String::as_str).collect(),
            tree,
            doc,
            unreadable: std::cell::RefCell::default(),
        }
    }

    /// Relative paths of every listed file.
    pub(crate) fn files(&self) -> &'a [String] {
        self.files
    }

    /// Returns `true` when the file is listed.
    pub(crate) fn has(&self, path: &str) -> bool {
        self.present.contains(path)
    }

    /// Returns `true` when a listed file lies below the directory.
    pub(crate) fn has_under(&self, dir: &str) -> bool {
        let prefix = format!("{dir}/");
        self.present
            .range(prefix.as_str()..)
            .next()
            .is_some_and(|path| path.starts_with(&prefix))
    }

    /// Content of a listed file as text.
    pub(crate) fn text(&self, path: &str) -> Option<String> {
        let bytes = self
            .tree
            .read(&absolute(self.root, path), MANIFEST_BYTES_MAX)?;
        String::from_utf8(bytes).ok()
    }

    /// Structured content of a JSON, JSONC or TOML manifest.
    pub(crate) fn document(&self, path: &str, format: &DocFormatId) -> Option<Value> {
        let bytes = self
            .tree
            .read(&absolute(self.root, path), MANIFEST_BYTES_MAX)?;
        let value = match self.doc {
            Some(doc) => doc
                .read_keys(&bytes, format, &KeyPath::root(), None)
                .ok()
                .and_then(|items| items.into_iter().next())
                .map(|item| item.value),
            None if *format == DocFormatId::JSON => serde_json::from_slice(&bytes)
                .ok()
                .map(Value::from_plain_json),
            None => None,
        };
        if value.is_none() {
            self.unreadable.borrow_mut().insert(path.to_owned());
        }
        value
    }

    /// Manifests whose structured content could not be read.
    pub(crate) fn unreadable(&self) -> Vec<String> {
        self.unreadable.borrow().iter().cloned().collect()
    }
}

/// Value at a path of map keys.
pub(crate) fn field<'v>(value: &'v Value, keys: &[&str]) -> Option<&'v Value> {
    keys.iter().try_fold(value, |current, key| match current {
        Value::Map(fields) => fields.get(*key),
        Value::Record(record) => record.fields.get(*key),
        _ => None,
    })
}

/// Text at a path of map keys.
pub(crate) fn text_field<'v>(value: &'v Value, keys: &[&str]) -> Option<&'v str> {
    field(value, keys).and_then(Value::as_str)
}

/// Entries of the map at a path of map keys, in document order.
pub(crate) fn map_entries<'v>(value: &'v Value, keys: &[&str]) -> Vec<(&'v str, &'v Value)> {
    match field(value, keys) {
        Some(Value::Map(fields)) => fields
            .iter()
            .map(|(key, value)| (key.as_str(), value))
            .collect(),
        _ => Vec::new(),
    }
}
