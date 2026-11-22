// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Names of format constructs reported by `E_UNSUPPORTED_CONSTRUCT`.
//!
//! When a change cannot be made without losing layout or meaning, nothing is
//! written and the error names the construct in its `found` field. The names
//! are stable identifiers of the form `<format>.<construct>`; every name used
//! by the crate is listed in [`ALL`], which is published with the schema of
//! error codes.
//!
//! | Name | Construct |
//! |---|---|
//! | `doc.out_of_scope_change` | a change would alter bytes outside the changed node |
//! | `doc.round_trip_loss` | the document would not be kept byte for byte |
//! | `doc.semantic_mismatch` | the changed document reads back with a value other than the one requested |
//! | `plist.binary_layout` | a binary property list is changed in place |
//! | `xml.dtd_internal_subset` | a change inside the internal subset of an XML document type declaration |
//! | `xml.external_entity` | a value of an XML document that depends on an external entity |
//! | `yaml.complex_key` | a YAML key written as `? key`, changed as a key |
//! | `yaml.merge_inherited_remove` | removal of a YAML key that exists only through a merge key `<<` |
//!
//! ```
//! use nurt_contracts::error::ErrorCode;
//! use nurt_doc::constructs::{self, YAML_COMPLEX_KEY};
//!
//! let error = constructs::unsupported(YAML_COMPLEX_KEY, "the key `? [a, b]` cannot be changed");
//! assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
//! assert_eq!(error.found.as_deref(), Some("yaml.complex_key"));
//! assert!(constructs::is_known(YAML_COMPLEX_KEY));
//! ```

use nurt_contracts::error::{Error, ErrorCode};

/// A change would alter bytes outside the node being changed and its
/// separators.
pub const DOC_OUT_OF_SCOPE_CHANGE: &str = "doc.out_of_scope_change";

/// Opening the document, or setting a value equal to the current one, would
/// not keep the bytes unchanged.
pub const DOC_ROUND_TRIP_LOSS: &str = "doc.round_trip_loss";

/// The changed document, read back with the reference parser of the format,
/// differs from the value the change should produce.
pub const DOC_SEMANTIC_MISMATCH: &str = "doc.semantic_mismatch";

/// A binary property list, which has no layout to keep, would be changed in
/// place.
pub const PLIST_BINARY_LAYOUT: &str = "plist.binary_layout";

/// A change inside the internal subset of an XML document type declaration.
pub const XML_DTD_INTERNAL_SUBSET: &str = "xml.dtd_internal_subset";

/// A value of an XML document that depends on an external entity, which is
/// never resolved.
pub const XML_EXTERNAL_ENTITY: &str = "xml.external_entity";

/// A YAML mapping key written in the explicit form `? key` whose key itself
/// would change.
pub const YAML_COMPLEX_KEY: &str = "yaml.complex_key";

/// Removal of a YAML key that a mapping only inherits through a merge key
/// (`<<: *base`); the removal would change the shared anchor.
pub const YAML_MERGE_INHERITED_REMOVE: &str = "yaml.merge_inherited_remove";

/// Every construct name, sorted.
pub const ALL: &[&str] = &[
    DOC_OUT_OF_SCOPE_CHANGE,
    DOC_ROUND_TRIP_LOSS,
    DOC_SEMANTIC_MISMATCH,
    PLIST_BINARY_LAYOUT,
    XML_DTD_INTERNAL_SUBSET,
    XML_EXTERNAL_ENTITY,
    YAML_COMPLEX_KEY,
    YAML_MERGE_INHERITED_REMOVE,
];

/// Returns `true` when `name` is a listed construct name.
#[must_use]
pub fn is_known(name: &str) -> bool {
    ALL.contains(&name)
}

/// Error `E_UNSUPPORTED_CONSTRUCT` naming `construct`; `message` describes
/// the change that was refused.
#[must_use]
pub fn unsupported(construct: &str, message: impl AsRef<str>) -> Error {
    Error::new(
        ErrorCode::UnsupportedConstruct,
        format!(
            "{}; nothing was changed, edit the text with `replace` or `edit` instead",
            message.as_ref()
        ),
    )
    .with_found(construct)
}
