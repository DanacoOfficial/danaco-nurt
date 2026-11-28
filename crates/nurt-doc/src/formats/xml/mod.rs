// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::xml
// Responsibility:   Reading and editing of XML documents through the byte ranges of every part of the markup.
// Dependencies:     attr, cdata, element, handle, ns, nurt_contracts, style, tree
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! XML documents.
//!
//! [`XmlTree`] keeps the source text and the byte range of every part of
//! the markup: declaration, document type declaration, start and end tags,
//! element names, each attribute with its name, `=`, quote and value, text,
//! CDATA sections, comments and processing instructions. Reading decodes
//! values on demand; editing produces [`XmlEdit`] replacements of byte
//! ranges, so markup outside the edited ranges is preserved byte for byte.
//!
//! # Keys
//!
//! | Key | Addresses |
//! |---|---|
//! | `project.version` | element `version` in the document element `project` |
//! | `deps.dep[2].version` | third `dep` among its siblings named `dep` |
//! | `a.b.@id` | attribute `id` |
//! | `a.b.#text`, `a.b.#text[1]` | text chunks of mixed content |
//! | `pom:project`, `["{urn:x}item"]` | names with a document prefix or a namespace |
//!
//! Without an index, a name shared by several siblings is ambiguous and
//! reported with the candidate keys. Values follow the convention of
//! `yq -p xml`, described in [`XmlTree::element_value`] and the
//! [`XmlTree::to_value`] examples: attributes are members `+@name`, text is
//! `+content`.
//!
//! # Errors
//!
//! Changes that would lose layout fail with `E_UNSUPPORTED_CONSTRUCT` and
//! one of the construct names defined in this module.

mod attr;
mod cdata;
mod comment;
pub mod dtd;
mod element;
pub mod encoding;
pub mod entity;
mod handle;
mod ns;
mod parse;
mod style;
mod tree;

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Location, Result};

pub use attr::Attribute;
pub use cdata::EntitySource;
pub use element::{ATTRIBUTE_KEY_PREFIX, CONTENT_KEY, DIRECTIVE_KEY, PI_KEY_PREFIX, XmlItem};
pub use handle::{XmlFormat, XmlHandle};
pub use ns::{XML_NAMESPACE, XMLNS_NAMESPACE, is_valid_name, split_qname};
pub use style::XmlStyle;
pub use tree::{
    ATTRIBUTE_SEGMENT_PREFIX, CDataSection, Comment, Declaration, DocType, Element, Node, NodeId,
    ProcessingInstruction, TEXT_SEGMENT, Target, TextNode, XmlTree,
};

/// Formats of this module, registered by
/// [`Registry::builtin`](crate::registry::Registry::builtin).
#[must_use]
pub fn formats() -> Vec<Box<dyn crate::registry::DocFormat>> {
    vec![Box::new(XmlFormat)]
}

/// Construct name: a reader event without an exact byte range.
pub const CONSTRUCT_EVENT_SPAN: &str = "xml.event_span";

/// Construct name: a reference to an entity of a document type definition
/// that is not loaded.
pub const CONSTRUCT_ENTITY_REFERENCE: &str = "xml.entity_reference";

/// Construct name: a change that needs a new namespace declaration.
pub const CONSTRUCT_NAMESPACE_DECLARATION: &str = "xml.namespace_declaration";

/// Construct name: replacing or removing the document or its root
/// element, or creating a second root element.
pub const CONSTRUCT_DOCUMENT_ROOT: &str = "xml.document_root";

/// Construct name: a value with text and child elements together, or with
/// several text chunks.
pub const CONSTRUCT_MIXED_CONTENT: &str = "xml.mixed_content";

/// Construct name: a list nested directly in a list.
pub const CONSTRUCT_NESTED_LIST: &str = "xml.nested_list";

/// Construct name: a value that would create a processing instruction.
pub const CONSTRUCT_PROCESSING_INSTRUCTION: &str = "xml.processing_instruction";

/// Construct names reported by this module with `E_UNSUPPORTED_CONSTRUCT`.
pub const CONSTRUCTS: [&str; 7] = [
    CONSTRUCT_EVENT_SPAN,
    CONSTRUCT_ENTITY_REFERENCE,
    CONSTRUCT_NAMESPACE_DECLARATION,
    CONSTRUCT_DOCUMENT_ROOT,
    CONSTRUCT_MIXED_CONTENT,
    CONSTRUCT_NESTED_LIST,
    CONSTRUCT_PROCESSING_INSTRUCTION,
];

/// Prefix of [`XmlChange::notable`] entries naming an entity whose
/// reference was replaced together with the surrounding text.
pub const NOTABLE_ENTITY_PREFIX: &str = "entity_replaced:";

/// Replacement of a byte range of the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlEdit {
    /// Replaced range; empty for an insertion.
    pub range: Range<usize>,
    /// New text.
    pub replacement: String,
}

impl XmlEdit {
    /// Creates an edit.
    pub fn new(range: Range<usize>, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

/// Edits of one change and what they create.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XmlChange {
    /// Non-overlapping edits.
    pub edits: Vec<XmlEdit>,
    /// Canonical keys of created nodes.
    pub keys_created: Vec<String>,
    /// Facts about the change worth reporting, such as
    /// [`NOTABLE_ENTITY_PREFIX`] entries.
    pub notable: Vec<String>,
}

impl XmlChange {
    /// Adds an edit.
    pub fn push(&mut self, edit: XmlEdit) {
        self.edits.push(edit);
    }

    /// Adds a notable fact once.
    pub fn note(&mut self, fact: String) {
        if !self.notable.contains(&fact) {
            self.notable.push(fact);
        }
    }

    /// Adds the edits, keys and facts of another change.
    pub fn merge(&mut self, other: Self) {
        self.edits.extend(other.edits);
        self.keys_created.extend(other.keys_created);
        for fact in other.notable {
            self.note(fact);
        }
    }

    /// `true` when the change modifies no byte.
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Applies the edits to `text`.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_CONFLICT` when edits overlap or insert at the same
    /// position, and `E_INVALID_VALUE` when an edit lies outside `text` or
    /// splits a character.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::{XmlChange, XmlEdit};
    ///
    /// let mut change = XmlChange::default();
    /// change.push(XmlEdit::new(3..4, "2"));
    /// assert_eq!(change.apply("<a>1</a>")?, "<a>2</a>");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn apply(&self, text: &str) -> Result<String> {
        let mut edits: Vec<&XmlEdit> = self.edits.iter().collect();
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        let mut out = String::with_capacity(text.len());
        let mut position = 0;
        let mut previous: Option<&Range<usize>> = None;
        for edit in edits {
            if let Some(previous) = previous
                && (edit.range.start < previous.end
                    || (edit.range.start == previous.start
                        && edit.range.is_empty()
                        && previous.is_empty()))
            {
                return Err(key_conflict(
                    "the change edits one part of the document twice; set the overlapping keys in separate calls",
                    Vec::new(),
                ));
            }
            let Some(kept) = text
                .get(position..edit.range.start)
                .filter(|_| edit.range.end <= text.len())
            else {
                return Err(invalid_value(
                    "edit range lies outside the document or splits a character",
                ));
            };
            if !text.is_char_boundary(edit.range.end) {
                return Err(invalid_value(
                    "edit range lies outside the document or splits a character",
                ));
            }
            out.push_str(kept);
            out.push_str(&edit.replacement);
            position = edit.range.end;
            previous = Some(&edit.range);
        }
        out.push_str(&text[position..]);
        Ok(out)
    }
}

impl XmlTree {
    /// Parses an XML document held as UTF-8 text.
    ///
    /// The document must be well-formed: one root element, matching tags,
    /// valid names, quoted attribute values without `<` and without
    /// repetition, and only declared references. References to entities of
    /// a document type definition are checked after
    /// [`XmlTree::set_entity_source`] with [`XmlTree::check_references`].
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` with the location for a document that is not
    /// well-formed and `E_UNSUPPORTED_CONSTRUCT` `xml.event_span` when the
    /// reader reports an event without an exact byte range.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<?xml version='1.0'?>\n<a x='1'/>\n")?;
    /// assert_eq!(tree.name(tree.root()), "a");
    /// assert!(XmlTree::parse("<a><b></a>").is_err());
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self> {
        parse::parse(text)
    }
}

/// `E_DOC_PARSE` error located at a byte offset of `text`.
pub(crate) fn doc_parse_error(text: &str, offset: usize, message: impl Into<String>) -> Error {
    located_error(ErrorCode::DocParse, text, offset, message)
}

/// Error with the given code located at a byte offset of `text`.
pub(crate) fn located_error(
    code: ErrorCode,
    text: &str,
    offset: usize,
    message: impl Into<String>,
) -> Error {
    Error::new(code, message).with_location(location_at(text, offset))
}

/// `error` located at a byte offset of `text` unless it carries a location.
pub(crate) fn locate(error: Error, text: &str, offset: usize) -> Error {
    if error.data().location.is_some() {
        error
    } else {
        error.with_location(location_at(text, offset))
    }
}

/// Line, column and byte of an offset of `text`.
fn location_at(text: &str, offset: usize) -> Location {
    let offset = offset.min(text.len());
    let prefix = text.get(..offset).unwrap_or(text);
    let line = prefix.matches('\n').count() + 1;
    let column = prefix
        .rfind('\n')
        .map_or(prefix, |newline| &prefix[newline + 1..])
        .chars()
        .count()
        + 1;
    Location {
        path: String::new(),
        line: u64::try_from(line).ok(),
        column: u64::try_from(column).ok(),
        end_line: None,
        end_column: None,
        byte: u64::try_from(offset).ok(),
    }
}

/// `E_UNSUPPORTED_CONSTRUCT` error for the named construct.
pub(crate) fn unsupported(construct: &str, message: impl Into<String>) -> Error {
    Error::new(ErrorCode::UnsupportedConstruct, message).with_found(construct)
}

/// `E_KEY_NOT_FOUND` error.
pub(crate) fn key_not_found(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::KeyNotFound, message)
}

/// `E_KEY_CONFLICT` error with the canonical keys of the candidates.
pub(crate) fn key_conflict(message: impl Into<String>, candidates: Vec<String>) -> Error {
    let mut error = Error::new(ErrorCode::KeyConflict, message);
    if !candidates.is_empty() {
        error = error.with_detail(
            "candidates",
            serde_json::Value::Array(
                candidates
                    .iter()
                    .cloned()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
        for candidate in candidates {
            error = error.with_sample(candidate);
        }
    }
    error
}

/// `E_INVALID_VALUE` error.
pub(crate) fn invalid_value(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidValue, message)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
