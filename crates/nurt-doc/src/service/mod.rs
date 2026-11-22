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
use crate::registry::{DocFormat, DocOptions, InputKind, Registry, TextEncoding, issue_of};

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
        let items = handle
            .get(key)?
            .into_iter()
            .map(|resolved| {
                let node = &tree[resolved.node];
                let mut span = tree.text_span(text, node.value_span);
                span.bytes = file_range(document.encoding, text, node.value_span);
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

/// Byte range of `span` of the decoded `text` in the file.
fn file_range(encoding: TextEncoding, text: &[u8], span: Span) -> ByteRange {
    let offset = |at: usize| -> u64 {
        let at_u64 = u64::try_from(at).unwrap_or(u64::MAX);
        match encoding {
            TextEncoding::Utf8 => at_u64,
            TextEncoding::Utf8Bom => at_u64 + UTF8_BOM_LEN,
            TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
                let units = String::from_utf8_lossy(text.get(..at).unwrap_or_default())
                    .encode_utf16()
                    .count();
                UTF16_UNIT_LEN + UTF16_UNIT_LEN * u64::try_from(units).unwrap_or(u64::MAX / 2)
            }
        }
    };
    ByteRange::new(offset(span.start), offset(span.end))
}
