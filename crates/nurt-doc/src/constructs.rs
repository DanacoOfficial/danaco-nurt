// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::constructs
// Responsibility:   Names of format constructs reported by `E_UNSUPPORTED_CONSTRUCT`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Names of format constructs reported by `E_UNSUPPORTED_CONSTRUCT`.
//!
//! When a change cannot be made without losing layout or meaning, nothing is
//! written and the error names the construct in its `found` field. The names
//! are stable identifiers of the form `<format>.<construct>`; every name used
//! by the crate is listed in [`ALL`], which is published with the schema of
//! error codes. Each name is described on its constant.
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

/// A name that is not a valid environment variable name.
pub const ENV_KEY_NAME: &str = "env.key_name";

/// A map or list set in an environment file, which holds text values only.
pub const ENV_NESTED_VALUE: &str = "env.nested_value";

/// The changed environment file reads back with another value.
pub const ENV_VERIFY: &str = "env.verify";

/// A key outside any section in a file without global keys.
pub const INI_GLOBAL_KEY: &str = "ini.global_key";

/// A key name that INI syntax cannot hold (`=`, `[` or a line break).
pub const INI_KEY_NAME: &str = "ini.key_name";

/// A map or list below the level of section keys.
pub const INI_NESTED_VALUE: &str = "ini.nested_value";

/// A section name that INI syntax cannot hold.
pub const INI_SECTION_NAME: &str = "ini.section_name";

/// A scalar set in place of a section.
pub const INI_SECTION_VALUE: &str = "ini.section_value";

/// A value that INI syntax cannot hold.
pub const INI_UNREPRESENTABLE_VALUE: &str = "ini.unrepresentable_value";

/// The changed INI file reads back with another value.
pub const INI_VERIFY: &str = "ini.verify";

/// A binary property list, which has no layout to keep, would be changed in
/// place.
pub const PLIST_BINARY_LAYOUT: &str = "plist.binary_layout";

/// A map or list set in a property file, which holds text values only.
pub const PROPERTIES_NESTED_VALUE: &str = "properties.nested_value";

/// The changed property file reads back with another value.
pub const PROPERTIES_VERIFY: &str = "properties.verify";

/// An empty array of tables, which has no table header to write.
pub const TOML_EMPTY_ARRAY_OF_TABLES: &str = "toml.empty_array_of_tables";

/// A dotted key inside an inline table.
pub const TOML_INLINE_DOTTED_KEY: &str = "toml.inline_dotted_key";

/// A change whose place in the table layout of the document cannot be kept.
pub const TOML_LAYOUT: &str = "toml.layout";

/// A null value, which TOML has no syntax for.
pub const TOML_NULL: &str = "toml.null";

/// A new table that cannot be placed without moving other tables.
pub const TOML_TABLE_POSITION: &str = "toml.table_position";

/// A table written with a header replaced by a value.
pub const TOML_TABLE_TO_VALUE: &str = "toml.table_to_value";

/// A value of a type that TOML cannot hold.
pub const TOML_VALUE_TYPE: &str = "toml.value_type";

/// The changed TOML document reads back with another value.
pub const TOML_VERIFY: &str = "toml.verify";

/// Removal or replacement of the document element of an XML document.
pub const XML_DOCUMENT_ROOT: &str = "xml.document_root";

/// A change inside the internal subset of an XML document type declaration.
pub const XML_DTD_INTERNAL_SUBSET: &str = "xml.dtd_internal_subset";

/// A value of an entity whose replacement text contains markup.
pub const XML_ENTITY_MARKUP: &str = "xml.entity_markup";

/// A value written with entity references.
pub const XML_ENTITY_REFERENCE: &str = "xml.entity_reference";

/// A node whose source position the XML reader does not report reliably.
pub const XML_EVENT_SPAN: &str = "xml.event_span";

/// A value of an XML document that depends on an external entity, which is
/// never resolved.
pub const XML_EXTERNAL_ENTITY: &str = "xml.external_entity";

/// Text mixed with child elements.
pub const XML_MIXED_CONTENT: &str = "xml.mixed_content";

/// A change of a namespace declaration attribute.
pub const XML_NAMESPACE_DECLARATION: &str = "xml.namespace_declaration";

/// A list directly inside a list, which XML elements cannot hold.
pub const XML_NESTED_LIST: &str = "xml.nested_list";

/// A change of a processing instruction.
pub const XML_PROCESSING_INSTRUCTION: &str = "xml.processing_instruction";

/// A YAML alias that refers back to its own anchor.
pub const YAML_ALIAS_CYCLE: &str = "yaml.alias_cycle";

/// YAML aliases whose expansion exceeds the limit of expanded nodes.
pub const YAML_ALIAS_EXPANSION_LIMIT: &str = "yaml.alias_expansion_limit";

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
    ENV_KEY_NAME,
    ENV_NESTED_VALUE,
    ENV_VERIFY,
    INI_GLOBAL_KEY,
    INI_KEY_NAME,
    INI_NESTED_VALUE,
    INI_SECTION_NAME,
    INI_SECTION_VALUE,
    INI_UNREPRESENTABLE_VALUE,
    INI_VERIFY,
    PLIST_BINARY_LAYOUT,
    PROPERTIES_NESTED_VALUE,
    PROPERTIES_VERIFY,
    TOML_EMPTY_ARRAY_OF_TABLES,
    TOML_INLINE_DOTTED_KEY,
    TOML_LAYOUT,
    TOML_NULL,
    TOML_TABLE_POSITION,
    TOML_TABLE_TO_VALUE,
    TOML_VALUE_TYPE,
    TOML_VERIFY,
    XML_DOCUMENT_ROOT,
    XML_DTD_INTERNAL_SUBSET,
    XML_ENTITY_MARKUP,
    XML_ENTITY_REFERENCE,
    XML_EVENT_SPAN,
    XML_EXTERNAL_ENTITY,
    XML_MIXED_CONTENT,
    XML_NAMESPACE_DECLARATION,
    XML_NESTED_LIST,
    XML_PROCESSING_INSTRUCTION,
    YAML_ALIAS_CYCLE,
    YAML_ALIAS_EXPANSION_LIMIT,
    YAML_COMPLEX_KEY,
    YAML_MERGE_INHERITED_REMOVE,
];

/// Returns `true` when `name` is a listed construct name.
#[must_use]
pub fn is_known(name: &str) -> bool {
    ALL.binary_search(&name).is_ok()
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
