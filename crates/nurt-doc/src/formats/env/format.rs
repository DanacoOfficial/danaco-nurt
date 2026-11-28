// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::env::format
// Responsibility:   Registered format `env`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registered format `env`.

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::EnvDocument;
use crate::formats::ini::support::flat::{block_range, flat_key, single_name};
use crate::formats::ini::support::handle::{
    Backend, KeyValueHandle, build_tree, reparsed, utf8_text,
};
use crate::formats::ini::support::tree::Node;
use crate::model::{
    DocHandle, DocTree, FlatSemantics, FormatSemantics, NodeKind, NodeSpec, Span, doc_parse,
};
use crate::registry::{Confidence, DocFormat, DocOptions};

/// Key path rules of environment files: a name with periods is one name.
static SEMANTICS: FlatSemantics = FlatSemantics;

/// File name extension of environment files (`app.env`).
const FORMAT_EXTENSIONS: [&str; 1] = ["env"];

/// File names of environment files.
const FORMAT_FILE_NAMES: [&str; 2] = [".env", ".env.*"];

/// Lines read when sniffing.
const SNIFF_LINES: usize = 64;

/// The format `env`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::env::EnvFormat;
/// use nurt_doc::model::SetOptions;
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let mut handle = EnvFormat.open(b"export PORT=80 # web\n".to_vec(), &DocOptions::default())?;
/// handle.set(&KeyPath::parse("PORT")?, &Value::Int(8080), &SetOptions::default())?;
/// assert_eq!(handle.text(), b"export PORT=8080 # web\n");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EnvFormat;

impl DocFormat for EnvFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::ENV
    }

    fn extensions(&self) -> &[&'static str] {
        &FORMAT_EXTENSIONS
    }

    fn file_names(&self) -> &[&'static str] {
        &FORMAT_FILE_NAMES
    }

    /// Lines of `NAME=value`, optionally after `export`, could be an
    /// environment file.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let Ok(text) = std::str::from_utf8(content) else {
            return Confidence::None;
        };
        let mut assignments = false;
        for line in text.lines().take(SNIFF_LINES).map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((name, _)) = line.split_once('=') else {
                return Confidence::None;
            };
            if !is_variable_name(name) {
                return Confidence::None;
            }
            assignments = true;
        }
        if assignments {
            Confidence::Low
        } else {
            Confidence::None
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let text = utf8_text(content, &DocFormatId::ENV)?;
        Ok(Box::new(KeyValueHandle::new(EnvDocument::parse(&text)?)))
    }
}

fn is_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

impl Backend for EnvDocument {
    fn format(&self) -> DocFormatId {
        DocFormatId::ENV
    }

    fn bytes(&self) -> Vec<u8> {
        self.text.as_bytes().to_vec()
    }

    fn tree(&self, bytes: &[u8]) -> DocTree {
        build_tree(bytes, &self.tree, &|node, path| self.describe(node, path))
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
        let text = std::str::from_utf8(bytes).map_err(|error| {
            doc_parse(
                &DocFormatId::ENV,
                bytes,
                Some(error.valid_up_to()),
                "the text is not valid UTF-8",
            )
        })?;
        reparsed(Self::parse(text))
    }

    fn text_values(&self) -> bool {
        true
    }

    fn removal_spans(&self, key: &KeyPath) -> Vec<Span> {
        let Some(name) = flat_key(key).ok().and_then(|flat| single_name(&flat)) else {
            return Vec::new();
        };
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
                Span::new(block.start, block.end)
            })
            .collect()
    }
}

impl EnvDocument {
    /// Spans, value and comment of a node: a variable spans its lines with
    /// the comment lines directly above it.
    fn describe(&self, node: &Node, path: &[KeySegment]) -> NodeSpec {
        let whole = Span::new(0, self.text.len());
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
        let mut spec = NodeSpec::new(NodeKind::Scalar, Span::new(block.start, block.end))
            .with_key_span(Span::new(entry.key_range.start, entry.key_range.end))
            .with_value_span(Span::new(entry.value_range.start, entry.value_range.end))
            .with_value(self.node_value(node));
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
    use crate::registry::Registry;

    const CREATE: SetOptions = SetOptions { create: true };

    fn open(text: &str) -> Box<dyn DocHandle> {
        EnvFormat
            .open(text.as_bytes().to_vec(), &DocOptions::default())
            .unwrap()
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn quotes_new_values_that_need_it() {
        let mut handle = open("A=1\nB='x' # note\n");
        handle
            .set(&key("B"), &Value::Text("two words".into()), &CREATE)
            .unwrap();
        handle
            .set(&key("C"), &Value::Text("$HOME/x y".into()), &CREATE)
            .unwrap();
        let text = String::from_utf8(handle.text().to_vec()).unwrap();
        assert!(text.starts_with("A=1\nB='two words' # note\n"), "{text}");
        assert_eq!(
            handle.to_value().unwrap().get("C"),
            Some(&Value::Text("$HOME/x y".into()))
        );
    }

    #[test]
    fn removes_variables_with_their_comments() {
        let mut handle = open("# a\nA=1\nB=2\nA=3\n");
        handle.remove(&key("A")).unwrap();
        assert_eq!(handle.text(), b"B=2\n");
        let node = &handle.tree()[handle.get(&key("B")).unwrap()[0].node];
        assert_eq!(node.value, Some(Value::Text("2".into())));
    }

    #[test]
    fn detects_environment_files_by_name_and_content() {
        let registry = Registry::with([Box::new(EnvFormat) as Box<dyn DocFormat>]);
        for name in [".env", "conf/.env.local", "app.env"] {
            let found = registry.detect(Some(std::path::Path::new(name)), b"");
            assert_eq!(found, Some(DocFormatId::ENV), "{name}");
        }
        assert_eq!(EnvFormat.sniff(b"# c\nexport A_1=x\n"), Confidence::Low);
        assert_eq!(EnvFormat.sniff(b"a b = c\n"), Confidence::None);
        assert_eq!(EnvFormat.sniff(b"text\n"), Confidence::None);
        assert!(EnvFormat.open(vec![0xFF], &DocOptions::default()).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
