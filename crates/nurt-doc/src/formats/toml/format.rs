// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::toml::format
// Responsibility:   Registered format `toml`.
// Dependencies:     nurt_contracts, toml_edit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Registered format `toml`.
//!
//! The meaning guard reads changed documents with the `toml` crate, a
//! parser independent of the `toml_edit` tree that the edits are made on.

use nurt_contracts::error::Result;
use nurt_contracts::keypath::KeySegment;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Date, DateKind, Fields, Value};
use toml_edit::Key;

use super::{EXTENSIONS, FILE_NAMES, Found, Located, Parent, TomlDocument, value_end};
use crate::formats::ini::support::handle::{Backend, KeyValueHandle, build_tree, utf8_text};
use crate::formats::ini::support::tree::Node;
use crate::model::{
    DocHandle, DocTree, FormatSemantics, Layout, MapSemantics, NodeSpec, Span, Style, doc_parse,
};
use crate::registry::{Confidence, DocFormat, DocOptions};

/// Key path rules of TOML: names are unique within a table.
static SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// The format `toml`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::toml::TomlFormat;
/// use nurt_doc::model::SetOptions;
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let mut handle = TomlFormat.open(b"[server]\nport = 80\n".to_vec(), &DocOptions::default())?;
/// handle.set(&KeyPath::parse("server.port")?, &Value::Int(8080), &SetOptions::default())?;
/// assert_eq!(handle.text(), b"[server]\nport = 8080\n");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TomlFormat;

impl DocFormat for TomlFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::TOML
    }

    fn extensions(&self) -> &[&'static str] {
        &EXTENSIONS
    }

    fn file_names(&self) -> &[&'static str] {
        &FILE_NAMES
    }

    /// A text with a key or a table that parses as TOML is probably a
    /// document; one that starts with a table header but does not parse,
    /// possibly because it is only the start of a file, could be one.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let Ok(text) = std::str::from_utf8(content) else {
            return Confidence::None;
        };
        match toml_edit::Document::parse(text) {
            Ok(document) if !document.as_table().is_empty() => Confidence::Medium,
            Ok(_) => Confidence::None,
            Err(_) if text.trim_start().starts_with('[') => Confidence::Low,
            Err(_) => Confidence::None,
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let text = utf8_text(content, &DocFormatId::TOML)?;
        Ok(Box::new(KeyValueHandle::new(TomlDocument::parse(&text)?)))
    }
}

impl Backend for TomlDocument {
    fn format(&self) -> DocFormatId {
        DocFormatId::TOML
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

    fn set(
        &mut self,
        key: &nurt_contracts::keypath::KeyPath,
        value: &Value,
        create: bool,
    ) -> Result<usize> {
        Self::set(self, key, value, create)
    }

    fn remove(&mut self, key: &nurt_contracts::keypath::KeyPath) -> Result<usize> {
        Self::remove(self, key)
    }

    fn reference(&self, bytes: &[u8]) -> Result<Vec<Value>> {
        reference(bytes)
    }
}

impl TomlDocument {
    /// Spans, value and comment of the node at `path`.
    ///
    /// A key and value pair spans its lines with the comment lines directly
    /// above it, which a removal takes with it; a member of an inline table
    /// spans its key and value, an array element its value, a table the
    /// lines of its header, keys and subtables.
    fn describe(&self, node: &Node, path: &[KeySegment]) -> NodeSpec {
        let whole = Span::new(0, self.text.len());
        let mut spec = NodeSpec::new(crate::model::NodeKind::Map, whole);
        if let Node::Leaf(id) = node {
            spec = spec.with_value(normalized_date(&self.leaves[*id]));
        }
        if path.is_empty() {
            return spec;
        }
        let Some(located) = self.locate(path) else {
            return spec;
        };
        if let Some(comment) = self.comment(&located) {
            spec = spec.with_comment(comment);
        }
        match located.found {
            Found::Value(value) => self.describe_value(spec, &located, value),
            Found::Table(_) | Found::Tables(_) => {
                let extent = self.extent(&located);
                spec.span = Span::new(extent.start, extent.end);
                spec.with_value_span(Span::new(extent.start, extent.end))
            }
        }
    }

    fn describe_value(
        &self,
        mut spec: NodeSpec,
        located: &Located<'_>,
        value: &toml_edit::Value,
    ) -> NodeSpec {
        let span = value.span().unwrap_or(0..0);
        let end = value_end(value).unwrap_or(span.end);
        let key = located.key.and_then(Key::span);
        let whole = match (located.parent, located.key) {
            (Some(Parent::Table), Some(key)) => self.pair_block(key, end),
            _ => None,
        }
        .or_else(|| key.as_ref().map(|key| key.start..end))
        .unwrap_or_else(|| span.clone());
        spec.span = Span::new(whole.start, whole.end);
        if let Some(key) = key {
            spec = spec.with_key_span(Span::new(key.start, key.end));
        }
        let layout = match value {
            toml_edit::Value::Array(_) | toml_edit::Value::InlineTable(_) => Layout::Flow,
            _ => Layout::Block,
        };
        spec.with_value_span(Span::new(span.start, end))
            .with_style(Style {
                layout,
                ..Style::default()
            })
    }
}

/// A date with the text of its canonical form, as the reference reader
/// writes it.
fn normalized_date(value: &Value) -> Value {
    match value {
        Value::Date(date) => match date.text.parse::<toml_edit::Datetime>() {
            Ok(datetime) => Value::Date(Date {
                kind: date.kind,
                text: datetime.to_string(),
            }),
            Err(_) => value.clone(),
        },
        other => other.clone(),
    }
}

/// Reads a TOML document with the `toml` crate.
fn reference(bytes: &[u8]) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        doc_parse(
            &DocFormatId::TOML,
            bytes,
            Some(error.valid_up_to()),
            "the text is not valid UTF-8",
        )
    })?;
    let table = text.parse::<toml::Table>().map_err(|error| {
        doc_parse(
            &DocFormatId::TOML,
            bytes,
            error.span().map(|span| span.start),
            error.message(),
        )
    })?;
    Ok(vec![Value::Map(table_value(&table))])
}

fn table_value(table: &toml::Table) -> Fields {
    table
        .iter()
        .map(|(name, item)| (name.clone(), item_value(item)))
        .collect()
}

fn item_value(item: &toml::Value) -> Value {
    match item {
        toml::Value::String(text) => Value::Text(text.clone()),
        toml::Value::Integer(number) => Value::Int(*number),
        toml::Value::Float(number) => Value::Number(*number),
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Datetime(datetime) => {
            let kind = match (datetime.date, datetime.time, datetime.offset) {
                (Some(_), Some(_), Some(_)) => DateKind::OffsetDateTime,
                (Some(_), Some(_), None) => DateKind::LocalDateTime,
                (Some(_), None, _) => DateKind::LocalDate,
                _ => DateKind::LocalTime,
            };
            Value::Date(Date {
                kind,
                text: datetime.to_string(),
            })
        }
        toml::Value::Array(items) => Value::List(items.iter().map(item_value).collect()),
        toml::Value::Table(table) => Value::Map(table_value(table)),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::keypath::KeyPath;

    use super::*;
    use crate::model::SetOptions;
    use crate::registry::Registry;

    const CREATE: SetOptions = SetOptions { create: true };

    fn open(text: &str) -> Box<dyn DocHandle> {
        TomlFormat
            .open(text.as_bytes().to_vec(), &DocOptions::default())
            .unwrap()
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn builds_a_tree_with_line_spans_comments_and_values() {
        let text =
            "# name\nname = \"a\" # inline\n\n[server]\nport = 80\nhost = { ip = \"::1\" }\n";
        let handle = open(text);
        let found = handle.get(&key("name")).unwrap();
        let node = &handle.tree()[found[0].node];
        assert_eq!(
            &text[node.span.start..node.span.end],
            "# name\nname = \"a\" # inline\n"
        );
        assert_eq!(node.comment.as_deref(), Some("name\ninline"));
        assert_eq!(node.value, Some(Value::Text("a".into())));
        let ip = handle.get(&key("server.host.ip")).unwrap();
        let ip = &handle.tree()[ip[0].node];
        assert_eq!(&text[ip.span.start..ip.span.end], "ip = \"::1\"");
        let server = handle.get(&key("server")).unwrap();
        let server = &handle.tree()[server[0].node];
        assert_eq!(
            &text[server.span.start..server.span.end],
            "[server]\nport = 80\nhost = { ip = \"::1\" }\n"
        );
    }

    #[test]
    fn sets_and_creates_keys_through_the_guards() {
        let mut handle = open("[a]\nx = 1\n\n[b]\ny = 2\n");
        let report = handle
            .set(&key("a.z"), &Value::Text("new".into()), &CREATE)
            .unwrap();
        assert_eq!(report.flags, ["keys_created"]);
        assert_eq!(handle.text(), b"[a]\nx = 1\nz = \"new\"\n\n[b]\ny = 2\n");
        let same = handle.set(&key("b.y"), &Value::Int(2), &CREATE).unwrap();
        assert!(same.is_unchanged());
        assert_eq!(same.replacements, 0);
    }

    #[test]
    fn removes_tables_and_array_elements() {
        let mut handle = open("list = [1, 2, 3]\n\n# gone\n[gone]\nk = 1\n");
        handle.remove(&key("gone")).unwrap();
        handle.remove(&key("list[1]")).unwrap();
        assert_eq!(handle.text(), b"list = [1, 3]\n\n");
        let error = handle.remove(&key("missing")).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyNotFound);
    }

    #[test]
    fn dates_are_compared_in_canonical_form() {
        let mut handle = open("when = 1979-05-27 07:32:00Z\nn = 1\n");
        handle.set(&key("n"), &Value::Int(2), &CREATE).unwrap();
        assert_eq!(handle.text(), b"when = 1979-05-27 07:32:00Z\nn = 2\n");
        let value = handle.to_value().unwrap();
        assert_eq!(
            value.get("when"),
            Some(&Value::Date(Date {
                kind: DateKind::OffsetDateTime,
                text: "1979-05-27T07:32:00Z".into()
            }))
        );
    }

    #[test]
    fn reference_reader_reads_every_type() {
        let values = reference(b"a = [1, 2.5, true, 07:00:00]\n[t]\ns = 's'\n").unwrap();
        let Value::Map(map) = &values[0] else {
            panic!("map expected");
        };
        assert_eq!(map.len(), 2);
        assert!(reference(b"a = ").is_err());
        assert!(reference(&[0xFF]).is_err());
    }

    #[test]
    fn detects_documents_by_name_extension_and_content() {
        let registry = Registry::with([Box::new(TomlFormat) as Box<dyn DocFormat>]);
        let by_name = registry.detect(Some(std::path::Path::new("x/Cargo.lock")), b"");
        assert_eq!(by_name, Some(DocFormatId::TOML));
        assert_eq!(TomlFormat.sniff(b"[a]\nb = 1\n"), Confidence::Medium);
        assert_eq!(TomlFormat.sniff(b"[a]\nb = "), Confidence::Low);
        assert_eq!(TomlFormat.sniff(b"plain text"), Confidence::None);
        assert_eq!(TomlFormat.sniff(b""), Confidence::None);
        assert!(TomlFormat.open(vec![0xFF], &DocOptions::default()).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
