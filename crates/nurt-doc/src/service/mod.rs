// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Implementation of the document service used by other commands.
//!
//! [`DocumentService`] implements the contract trait
//! [`DocService`](nurt_contracts::services::DocService) over a [`Registry`]:
//! commands of other groups detect formats, read values by key, check the
//! syntax of content before it is written and write records as JSON, YAML,
//! TOML, CSV or TSV without depending on the document formats themselves.
//!
//! ```
//! use nurt_contracts::services::{DocFormatId, DocService, SerializeOptions};
//! use nurt_contracts::value::{Record, RecordKind, Value};
//! use nurt_doc::registry::{DocOptions, Registry};
//! use nurt_doc::service::DocumentService;
//!
//! let service = DocumentService::new(Registry::new(), DocOptions::default());
//! let record = Record::new(RecordKind::Item).with("key", Value::Text("a".to_owned()));
//! let csv = service.serialize_records(&[record], &DocFormatId::CSV, &SerializeOptions::default())?;
//! assert_eq!(csv, b"key\r\na\r\n");
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

mod serialize;

use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{
    ByteRange, DocFormatId, DocFormatInfo, DocItem, DocService, SerializeOptions, SyntaxIssue,
};
use nurt_contracts::value::Record;

use crate::model::{DocTree, Span};
use crate::registry::{
    DocFormat, DocOptions, InputKind, Registry, TextEncoding, check_size, issue_of,
};

/// Length of the UTF-8 byte order mark.
const UTF8_BOM_LEN: u64 = 3;
/// Length of a UTF-16 byte order mark and of one UTF-16 code unit.
const UTF16_UNIT_LEN: u64 = 2;

/// Document service over a registry of formats.
#[derive(Debug)]
pub struct DocumentService {
    registry: Registry,
    options: DocOptions,
}

impl DocumentService {
    /// Service over `registry` with the options of `documents.*`.
    #[must_use]
    pub const fn new(registry: Registry, options: DocOptions) -> Self {
        Self { registry, options }
    }

    /// The registry.
    #[must_use]
    pub const fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The options.
    #[must_use]
    pub const fn options(&self) -> &DocOptions {
        &self.options
    }

    fn format(&self, id: &DocFormatId) -> Result<&dyn DocFormat> {
        self.registry.get(id).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownValue,
                format!("unknown document format `{id}`"),
            )
            .with_found(id.as_str())
        })
    }
}

impl DocService for DocumentService {
    fn formats(&self) -> Vec<DocFormatInfo> {
        self.registry.infos()
    }

    fn detect_format(&self, path: Option<&Path>, bytes: &[u8]) -> Option<DocFormatId> {
        self.registry.detect(path, bytes)
    }

    fn read_keys(
        &self,
        bytes: &[u8],
        format: &DocFormatId,
        key: &KeyPath,
        depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        let document = self.registry.open(format, bytes, &self.options)?;
        let handle = &document.handle;
        let tree = handle.tree();
        let text = handle.text();
        let several = tree.roots().len() > 1;
        let offsets = FileOffsets::new(document.encoding, text);
        let items = handle
            .get(key)?
            .into_iter()
            .map(|resolved| {
                let node = &tree[resolved.node];
                let mut span = tree.text_span(text, node.value_span);
                span.bytes = offsets.range(text, node.value_span);
                DocItem {
                    key: resolved.key,
                    value: tree.value_of(resolved.node, handle.semantics(), depth),
                    span: Some(span),
                    comment: node.comment.clone(),
                    document: several.then(|| tree.document_of(resolved.node)).flatten(),
                    anchor: node.anchor.clone(),
                    alias_of: alias_name(tree, resolved.node),
                    tag: node.tag.clone(),
                }
            })
            .collect();
        Ok(items)
    }

    fn check_syntax(&self, bytes: &[u8], format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        let doc_format = self.format(format)?;
        check_size(bytes, &self.options)?;
        let text = match doc_format.input() {
            InputKind::Utf8Text => match TextEncoding::decode(bytes) {
                Ok((_, text)) => text,
                Err(error) => return Ok(vec![issue_of(&error)]),
            },
            InputKind::Bytes => bytes.to_vec(),
        };
        Ok(doc_format.check_syntax(&text, &self.options))
    }

    fn serialize_records(
        &self,
        records: &[Record],
        format: &DocFormatId,
        options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        serialize::records(records, format, options)
    }
}

fn alias_name(tree: &DocTree, node: crate::model::NodeId) -> Option<String> {
    let target = tree[node].alias_of?;
    tree.get(target).and_then(|target| target.anchor.clone())
}

/// Bytes of the decoded text between two entries of the offset index.
const OFFSET_BLOCK: usize = 1024;

/// Conversion of offsets in the decoded text to offsets in the file.
///
/// For UTF-16 the index holds the number of code units before every
/// [`OFFSET_BLOCK`]th byte of the text, so that an offset costs one block
/// instead of a scan from the start of the text.
struct FileOffsets {
    encoding: TextEncoding,
    units_before_block: Vec<u64>,
}

impl FileOffsets {
    fn new(encoding: TextEncoding, text: &[u8]) -> Self {
        let mut units_before_block = Vec::new();
        if matches!(encoding, TextEncoding::Utf16Le | TextEncoding::Utf16Be) {
            let mut total = 0_u64;
            units_before_block.push(total);
            for block in text.chunks(OFFSET_BLOCK) {
                total += utf16_units(block);
                units_before_block.push(total);
            }
        }
        Self {
            encoding,
            units_before_block,
        }
    }

    fn offset(&self, text: &[u8], at: usize) -> u64 {
        let at = at.min(text.len());
        let at_u64 = u64::try_from(at).unwrap_or(u64::MAX);
        match self.encoding {
            TextEncoding::Utf8 => at_u64,
            TextEncoding::Utf8Bom => at_u64 + UTF8_BOM_LEN,
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
                let block = at / OFFSET_BLOCK;
                let before = self.units_before_block.get(block).copied().unwrap_or(0);
                let start = block * OFFSET_BLOCK;
                let units = before + utf16_units(text.get(start..at).unwrap_or_default());
                UTF16_UNIT_LEN + UTF16_UNIT_LEN * units
            }
        }
    }

    /// Byte range of `span` of the decoded `text` in the file.
    fn range(&self, text: &[u8], span: Span) -> ByteRange {
        ByteRange::new(self.offset(text, span.start), self.offset(text, span.end))
    }
}

/// UTF-16 code units of the characters that start in `bytes`, a part of
/// valid UTF-8 text: a leading byte of four bytes gives a surrogate pair,
/// other leading bytes one unit, continuation bytes none.
fn utf16_units(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .map(|byte| match byte {
            0x80..=0xBF => 0,
            0xF0..=0xFF => 2,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_count_code_units_across_blocks() {
        let text = format!("{}é😀x", "a".repeat(OFFSET_BLOCK * 3 - 2));
        let offsets = FileOffsets::new(TextEncoding::Utf16Le, text.as_bytes());
        let at = |needle: &str| text.find(needle).unwrap();
        let expected = |prefix: &str| 2 + 2 * u64::try_from(prefix.encode_utf16().count()).unwrap();
        for needle in ["é", "😀", "x"] {
            let prefix = &text[..at(needle)];
            assert_eq!(
                offsets.offset(text.as_bytes(), at(needle)),
                expected(prefix)
            );
        }
        assert_eq!(offsets.offset(text.as_bytes(), text.len()), expected(&text));
        assert_eq!(offsets.offset(text.as_bytes(), usize::MAX), expected(&text));
    }

    #[test]
    fn utf8_offsets_account_for_the_byte_order_mark() {
        let text = "zażółć";
        let plain = FileOffsets::new(TextEncoding::Utf8, text.as_bytes());
        let marked = FileOffsets::new(TextEncoding::Utf8Bom, text.as_bytes());
        assert_eq!(plain.offset(text.as_bytes(), 3), 3);
        assert_eq!(marked.offset(text.as_bytes(), 3), 6);
        let range = marked.range(text.as_bytes(), Span::new(1, 2));
        assert_eq!(range, ByteRange::new(4, 5));
    }

    #[test]
    fn syntax_check_refuses_documents_above_the_size_limit() {
        let options = DocOptions {
            max_bytes: 3,
            ..DocOptions::default()
        };
        let service = DocumentService::new(Registry::new(), options);
        let error = service
            .check_syntax(b"1234", &DocFormatId::JSON)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownValue);
        let service = DocumentService::new(Registry::builtin(), options);
        let error = service
            .check_syntax(b"1234", &DocFormatId::JSON)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Limit);
        assert!(
            service
                .check_syntax(b"123", &DocFormatId::JSON)
                .unwrap()
                .is_empty()
        );
    }
}
