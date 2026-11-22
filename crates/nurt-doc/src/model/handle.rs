// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::Result;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::{DocTree, FormatSemantics, Resolved};
use crate::edit::EditReport;

/// Options of [`DocHandle::set`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SetOptions {
    /// Create missing keys, including intermediate maps (`a.b` creates `a`
    /// as a map, `a[0]` creates a list).
    pub create: bool,
}

/// Open document of one format: its text, its node tree and the operations
/// that change it while preserving the layout of unchanged content.
///
/// The text is UTF-8 without a byte order mark for formats that read text;
/// the registry decodes and re-encodes other encodings. Every change goes
/// through the guards of [`crate::edit::guard`]; a failed change leaves the
/// document unchanged.
pub trait DocHandle: Send {
    /// Format of the document.
    fn format(&self) -> DocFormatId;

    /// Current text, including all changes made through this handle.
    fn text(&self) -> &[u8];

    /// Node tree of the current text.
    fn tree(&self) -> &DocTree;

    /// Key path rules of the format.
    fn semantics(&self) -> &dyn FormatSemantics;

    /// Sets the value at `key` in every node it selects; `[+]` appends to a
    /// list. Setting a value equal to the current one changes no byte.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` when the key selects nothing and
    /// `create` is off, with `E_KEY_CONFLICT` for paths through nodes of
    /// another kind or ambiguous names, and with `E_UNSUPPORTED_CONSTRUCT`
    /// when the change cannot be made without losing layout or meaning.
    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport>;

    /// Removes every node selected by `key` with its separator and the
    /// comment on the same line.
    ///
    /// # Errors
    ///
    /// As for [`DocHandle::set`].
    fn remove(&mut self, key: &KeyPath) -> Result<EditReport>;

    /// Nodes selected by `key`.
    ///
    /// # Errors
    ///
    /// As for [`DocTree::resolve`].
    fn get(&self, key: &KeyPath) -> Result<Vec<Resolved>> {
        self.tree().resolve(key, self.semantics())
    }

    /// Value of the first document; `null` for an empty file.
    ///
    /// # Errors
    ///
    /// The default implementation does not fail; formats whose values need
    /// further decoding may.
    fn to_value(&self) -> Result<Value> {
        let tree = self.tree();
        Ok(tree
            .roots()
            .first()
            .map(|root| tree.value_of(*root, self.semantics(), None))
            .unwrap_or_default())
    }
}
