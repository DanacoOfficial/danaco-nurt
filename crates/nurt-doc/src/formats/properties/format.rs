// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::properties::format
// Responsibility:   Registered format `properties`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registered format `properties`.
//!
//! The format reads the bytes of the file: it detects UTF-8 and
//! ISO-8859-1 itself. Node spans are byte offsets in the file, so that an
//! ISO-8859-1 character counts as one byte.

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::{EXTENSIONS, PropertiesDocument, PropertiesEncoding, UTF8_BOM};
use crate::formats::ini::support::flat::{block_range, flat_key, single_name};
use crate::formats::ini::support::handle::{Backend, KeyValueHandle, build_tree, reparsed};
use crate::formats::ini::support::tree::Node;
use crate::model::{DocHandle, DocTree, FlatSemantics, FormatSemantics, NodeKind, NodeSpec, Span};
use crate::registry::{Confidence, DocFormat, DocOptions, InputKind};

/// Key path rules of property files: `server.port` is one name.
static SEMANTICS: FlatSemantics = FlatSemantics;

/// Lines read when sniffing.
const SNIFF_LINES: usize = 64;

/// The format `properties`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::properties::PropertiesFormat;
/// use nurt_doc::model::SetOptions;
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let mut handle = PropertiesFormat.open(b"server.port=80\n".to_vec(), &DocOptions::default())?;
/// handle.set(&KeyPath::parse("server.port")?, &Value::Int(8080), &SetOptions::default())?;
/// assert_eq!(handle.text(), b"server.port=8080\n");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PropertiesFormat;

impl DocFormat for PropertiesFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::PROPERTIES
    }

    fn extensions(&self) -> &[&'static str] {
        &EXTENSIONS
    }

    /// Key lines and `!` comments could be a property file; content alone
    /// never makes it probable, because environment and INI files share
    /// the syntax.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let text = String::from_utf8_lossy(content);
        let mut pairs = false;
        for line in text.lines().take(SNIFF_LINES).map(str::trim_start) {
            if line.is_empty() || line.starts_with(['#', '!']) {
                continue;
            }
            if line.starts_with('[') {
                return Confidence::None;
            }
            pairs |= line.contains(['=', ':']);
        }
        if pairs {
            Confidence::Low
        } else {
            Confidence::None
        }
    }

    fn input(&self) -> InputKind {
        InputKind::Bytes
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        Ok(Box::new(KeyValueHandle::new(PropertiesDocument::parse(
            &content,
        )?)))
    }
}

/// Map from offsets in the decoded text to offsets in the file.
struct Offsets {
    shift: usize,
    chars: Option<Vec<usize>>,
}

impl Offsets {
    fn of(document: &PropertiesDocument) -> Self {
        let text = &document.text;
        match document.encoding {
            PropertiesEncoding::Latin1 if !text.is_ascii() => {
                let mut chars = vec![0; text.len() + 1];
                let mut count = 0;
                for (offset, _) in text.char_indices() {
                    chars[offset] = count;
                    count += 1;
                }
                chars[text.len()] = count;
                Self {
                    shift: 0,
                    chars: Some(chars),
                }
            }
            PropertiesEncoding::Utf8 if document.bom => Self {
                shift: UTF8_BOM.len(),
                chars: None,
            },
            _ => Self {
                shift: 0,
                chars: None,
            },
        }
    }

    fn at(&self, offset: usize) -> usize {
        let local = self
            .chars
            .as_ref()
            .and_then(|chars| chars.get(offset).copied())
            .unwrap_or(offset);
        local + self.shift
    }

    fn span(&self, range: &std::ops::Range<usize>) -> Span {
        Span::new(self.at(range.start), self.at(range.end))
    }
}

impl Backend for PropertiesDocument {
    fn format(&self) -> DocFormatId {
        DocFormatId::PROPERTIES
    }

    fn bytes(&self) -> Vec<u8> {
        Self::bytes(self)
    }

    fn tree(&self, bytes: &[u8]) -> DocTree {
        let offsets = Offsets::of(self);
        let whole = Span::new(0, bytes.len());
        build_tree(bytes, &self.tree, &|node, path| {
            self.describe(node, path, &offsets, whole)
        })
    }

    fn semantics(&self) -> &'static dyn FormatSemantics {
        &SEMANTICS
    }

    fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize> {
        Self::set(self, key, value, create)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<usize> {
        Self::remove(self, key)
    }

    fn reference(&self, bytes: &[u8]) -> Result<Vec<Value>> {
        reparsed(Self::parse_with(bytes, self.encoding))
    }

    fn text_values(&self) -> bool {
        true
    }

    fn removal_spans(&self, key: &KeyPath) -> Vec<Span> {
        let Some(name) = flat_key(key).ok().and_then(|flat| single_name(&flat)) else {
            return Vec::new();
        };
        let offsets = Offsets::of(self);
        self.parsed
            .entries
            .iter()
            .filter(|entry| entry.key == name)
            .map(|entry| {
                let block = block_range(
                    &self.parsed.lines,
                    &self.parsed.comment_lines,
                    entry.first_line,
                    entry.last_line,
                    0,
                );
                offsets.span(&block)
            })
            .collect()
    }
}

impl PropertiesDocument {
    /// Spans, value and comment of a node: an entry spans its lines with
    /// the comment lines directly above it.
    fn describe(
        &self,
        node: &Node,
        path: &[KeySegment],
        offsets: &Offsets,
        whole: Span,
    ) -> NodeSpec {
        let Node::Leaf(id) = node else {
            return NodeSpec::new(NodeKind::Map, whole);
        };
        if path.is_empty() {
            return NodeSpec::new(NodeKind::Map, whole);
        }
        let entry = &self.parsed.entries[*id];
        let block = block_range(
            &self.parsed.lines,
            &self.parsed.comment_lines,
            entry.first_line,
            entry.last_line,
            0,
        );
        let mut spec = NodeSpec::new(NodeKind::Scalar, offsets.span(&block))
            .with_key_span(offsets.span(&entry.key_range))
            .with_value_span(offsets.span(&entry.value_range))
            .with_value(Value::Text(entry.value.clone()));
        if let Some(comment) = &entry.comment {
            spec = spec.with_comment(comment.clone());
        }
        spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SetOptions;

    const CREATE: SetOptions = SetOptions { create: true };

    fn open(bytes: &[u8]) -> Box<dyn DocHandle> {
        PropertiesFormat
            .open(bytes.to_vec(), &DocOptions::default())
            .unwrap()
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn spans_are_offsets_in_the_file() {
        let handle = open(b"a=\xe9\xe9\nb=2\n");
        let found = handle.get(&key("b")).unwrap();
        let node = &handle.tree()[found[0].node];
        assert_eq!((node.value_span.start, node.value_span.end), (7, 8));
        let bom = open(b"\xef\xbb\xbfk=v\n");
        let found = bom.get(&key("k")).unwrap();
        assert_eq!(bom.tree()[found[0].node].value_span.start, 5);
    }

    #[test]
    fn keeps_the_encoding_of_the_file_when_setting() {
        let mut handle = open(b"name=caf\xe9\nport=1\n");
        handle.set(&key("port"), &Value::Int(2), &CREATE).unwrap();
        assert_eq!(handle.text(), b"name=caf\xe9\nport=2\n");
        handle
            .set(&key("server.host"), &Value::Text("x".into()), &CREATE)
            .unwrap();
        assert_eq!(handle.text(), b"name=caf\xe9\nport=2\nserver.host=x\n");
    }

    #[test]
    fn removes_every_occurrence_of_a_repeated_key() {
        let mut handle = open(b"a=1\n# b\nb=2\na=3\n");
        assert_eq!(
            handle.to_value().unwrap().get("a"),
            Some(&Value::Text("3".into()))
        );
        handle.remove(&key("a")).unwrap();
        assert_eq!(handle.text(), b"# b\nb=2\n");
    }

    #[test]
    fn sniffs_key_lines_without_sections() {
        assert_eq!(PropertiesFormat.sniff(b"! c\na=1\n"), Confidence::Low);
        assert_eq!(PropertiesFormat.sniff(b"[a]\nb=1\n"), Confidence::None);
        assert_eq!(PropertiesFormat.sniff(b"# only\n"), Confidence::None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
