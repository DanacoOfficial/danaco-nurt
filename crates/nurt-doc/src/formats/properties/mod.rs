// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Java property files.
//!
//! A property file is a flat map: the whole key is one name, so
//! `server.port` addresses the key `"server.port"`. Separators `=`, `:` and
//! whitespace, `\` line continuations, `\uHHHH` and the other escapes of
//! `java.util.Properties`, and comments starting with `#` or `!` are read
//! and kept. A file that is valid UTF-8 is read as UTF-8, any other file as
//! ISO-8859-1; characters a file's encoding cannot hold are written as
//! `\uHHHH`. A key written more than once has the value of its last
//! occurrence.

mod escape;
mod format;
mod parse;

pub use format::PropertiesFormat;

/// Registration function of the format `properties`.
#[must_use]
pub fn formats() -> Vec<Box<dyn crate::registry::DocFormat>> {
    vec![Box::new(PropertiesFormat)]
}

use nurt_contracts::error::Error;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocItem;
use nurt_contracts::value::{Fields, Value};

use crate::formats::ini::support::errors::{key_conflict, unsupported};
use crate::formats::ini::support::flat::{
    block_range, flat_key, flat_tree, removals, scalar_text, single_name,
};
use crate::formats::ini::support::text::{
    LineIndex, Splice, apply_splices, dominant_newline, line_break_before_append,
};
use crate::formats::ini::support::tree::{Node, resolve};

use escape::{Part, escape};
use parse::{Parsed, continues, is_space, parse};

/// File name extensions of property files.
pub const EXTENSIONS: [&str; 1] = ["properties"];

/// UTF-8 byte order mark.
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

/// Construct name: a list or map assigned to a key.
const NESTED: &str = "properties.nested_value";
/// Construct name: the changed file does not read back as expected.
const VERIFY: &str = "properties.verify";

/// Character encoding of a property file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PropertiesEncoding {
    /// UTF-8, the encoding of resource bundles since Java 9.
    Utf8,
    /// ISO-8859-1, the encoding of `Properties.load(InputStream)`.
    Latin1,
}

/// Parsed property file that keeps its original bytes.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::properties::PropertiesDocument;
///
/// let mut doc = PropertiesDocument::parse(b"# port\nserver.port = 80\n")?;
/// let key = KeyPath::parse("server.port")?;
/// assert_eq!(doc.get(&key)?[0].value, Value::Text("80".into()));
/// doc.set(&key, &Value::Text("caf\u{e9}".into()), false)?;
/// assert_eq!(doc.text(), "# port\nserver.port = caf\u{e9}\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct PropertiesDocument {
    text: String,
    encoding: PropertiesEncoding,
    bom: bool,
    parsed: Parsed,
    tree: Node,
}

/// Changes to a property file and the entries expected after them.
struct Plan {
    splices: Vec<Splice>,
    count: usize,
    expected: Vec<(String, String)>,
}

impl PropertiesDocument {
    /// Parses a property file, detecting its encoding.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` for a malformed `\uHHHH` escape.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        match std::str::from_utf8(bytes) {
            Ok(_) => Self::parse_with(bytes, PropertiesEncoding::Utf8),
            Err(_) => Self::parse_with(bytes, PropertiesEncoding::Latin1),
        }
    }

    /// Parses a property file in the given encoding; bytes that are not
    /// UTF-8 are read as ISO-8859-1.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` for a malformed `\uHHHH` escape.
    pub fn parse_with(bytes: &[u8], encoding: PropertiesEncoding) -> Result<Self, Error> {
        let (text, encoding, bom) = match (encoding, std::str::from_utf8(bytes)) {
            (PropertiesEncoding::Utf8, Ok(text)) => match text.strip_prefix('\u{feff}') {
                Some(rest) => (rest.to_owned(), PropertiesEncoding::Utf8, true),
                None => (text.to_owned(), PropertiesEncoding::Utf8, false),
            },
            _ => (
                bytes.iter().map(|&b| char::from(b)).collect(),
                PropertiesEncoding::Latin1,
                false,
            ),
        };
        Self::from_text(text, encoding, bom)
    }

    fn from_text(text: String, encoding: PropertiesEncoding, bom: bool) -> Result<Self, Error> {
        let parsed = parse(&text)?;
        let tree = flat_tree(parsed.entries.iter().map(|e| e.key.as_str()));
        Ok(Self {
            text,
            encoding,
            bom,
            parsed,
            tree,
        })
    }

    /// Encoding of the file.
    pub fn encoding(&self) -> PropertiesEncoding {
        self.encoding
    }

    /// Text of the file, including all changes made so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Bytes of the file in its encoding, including all changes made so far.
    pub fn bytes(&self) -> Vec<u8> {
        match self.encoding {
            PropertiesEncoding::Utf8 => {
                let prefix: &[u8] = if self.bom { UTF8_BOM } else { &[] };
                [prefix, self.text.as_bytes()].concat()
            }
            PropertiesEncoding::Latin1 => self
                .text
                .chars()
                .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                .collect(),
        }
    }

    /// Values selected by `key`; consecutive names of the key path form one
    /// property name.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing key with the closest names,
    /// `E_KEY_CONFLICT` for indices.
    pub fn get(&self, key: &KeyPath) -> Result<Vec<DocItem>, Error> {
        let key = flat_key(key)?;
        let value_of = |node: &Node| self.node_value(node);
        let hits = resolve(&self.tree, &key, &value_of)?;
        let index = LineIndex::new(&self.text);
        hits.iter()
            .map(|hit| {
                let path = KeyPath::new(None, hit.path.clone())
                    .map_err(|error| key_conflict(&error.to_string(), "key path", &[]))?;
                let (range, comment) = match hit.node {
                    Node::Leaf(id) => {
                        let entry = &self.parsed.entries[*id];
                        (entry.value_range.clone(), entry.comment.clone())
                    }
                    _ => (0..self.text.len(), None),
                };
                let mut span = index.span(&self.text, range.clone());
                span.bytes.start = self.file_offset(range.start);
                span.bytes.end = self.file_offset(range.end);
                Ok(DocItem {
                    key: path,
                    value: self.node_value(hit.node),
                    span: Some(span),
                    comment,
                    document: None,
                    anchor: None,
                    alias_of: None,
                    tag: None,
                })
            })
            .collect()
    }

    /// Content of the file as a map in the order of first occurrence.
    pub fn to_value(&self) -> Value {
        self.node_value(&self.tree)
    }

    /// Sets the value of the keys selected by `key`; `create` adds a missing
    /// key at the end of the file. Returns the number of keys set.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing key without `create`,
    /// `E_UNSUPPORTED_CONSTRUCT` `properties.nested_value` for a list or
    /// map value.
    pub fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize, Error> {
        let text = scalar_text(value)
            .ok_or_else(|| unsupported(NESTED, "property values are text; write a scalar value"))?;
        let flat = flat_key(key)?;
        let mut plan = self.plan();
        let value_of = |node: &Node| self.node_value(node);
        match resolve(&self.tree, &flat, &value_of) {
            Ok(hits) => {
                for hit in hits {
                    match hit.node {
                        Node::Leaf(id) => self.plan_value(&mut plan, *id, &text),
                        _ => {
                            return Err(unsupported(
                                NESTED,
                                "a property file is a flat map; address a single key",
                            ));
                        }
                    }
                }
            }
            Err(error) => match (create, single_name(&flat)) {
                (true, Some(name))
                    if error.code() == nurt_contracts::error::ErrorCode::KeyNotFound =>
                {
                    self.plan_new(&mut plan, &name, &text);
                }
                _ => return Err(error),
            },
        }
        self.commit(plan)
    }

    /// Removes every occurrence of the keys selected by `key` with the
    /// comment lines directly above them. Returns the number of keys
    /// removed.
    ///
    /// # Errors
    ///
    /// The errors of [`PropertiesDocument::get`].
    pub fn remove(&mut self, key: &KeyPath) -> Result<usize, Error> {
        let flat = flat_key(key)?;
        let value_of = |node: &Node| self.node_value(node);
        let hits = resolve(&self.tree, &flat, &value_of)?;
        let names: Vec<&str> = hits
            .iter()
            .filter_map(|hit| match hit.node {
                Node::Leaf(id) => Some(self.parsed.entries[*id].key.as_str()),
                _ => None,
            })
            .collect();
        let mut plan = self.plan();
        let ranges = self
            .parsed
            .entries
            .iter()
            .filter(|entry| names.contains(&entry.key.as_str()))
            .map(|entry| {
                block_range(
                    &self.parsed.lines,
                    &self.parsed.comment_lines,
                    entry.first_line,
                    entry.last_line,
                    0,
                )
            })
            .collect();
        plan.splices = removals(ranges);
        plan.expected
            .retain(|(name, _)| !names.contains(&name.as_str()));
        plan.count = names.len();
        self.commit(plan)
    }

    fn node_value(&self, node: &Node) -> Value {
        match node {
            Node::Leaf(id) => Value::Text(self.parsed.entries[*id].value.clone()),
            Node::Map(map) => Value::Map(
                map.members
                    .iter()
                    .map(|(name, node)| (name.clone(), self.node_value(node)))
                    .collect::<Fields>(),
            ),
            Node::List(_) | Node::Repeated(_) => Value::Null,
        }
    }

    fn file_offset(&self, offset: usize) -> u64 {
        let bytes = match self.encoding {
            PropertiesEncoding::Latin1 => self.text[..offset].chars().count(),
            PropertiesEncoding::Utf8 => offset + if self.bom { UTF8_BOM.len() } else { 0 },
        };
        u64::try_from(bytes).unwrap_or(u64::MAX)
    }

    fn latin1(&self) -> bool {
        self.encoding == PropertiesEncoding::Latin1
    }

    fn plan(&self) -> Plan {
        Plan {
            splices: Vec::new(),
            count: 0,
            expected: self
                .parsed
                .entries
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect(),
        }
    }

    fn plan_value(&self, plan: &mut Plan, id: usize, value: &str) {
        plan.count += 1;
        let entry = &self.parsed.entries[id];
        if entry.value == value {
            return;
        }
        plan.expected[id].1 = value.to_owned();
        let raw = self.continued_value(id, value);
        plan.splices
            .push(Splice::new(entry.value_range.clone(), raw));
    }

    /// Raw text of `value` for the entry `id`, broken over lines like the
    /// current value: after the same delimiter character when every line of
    /// the current value ends with it, otherwise at the width of its longest
    /// line.
    fn continued_value(&self, id: usize, value: &str) -> String {
        let entry = &self.parsed.entries[id];
        let lines = &self.parsed.lines[entry.first_line..=entry.last_line];
        let latin1 = self.latin1();
        if lines.len() == 1 {
            return escape(value, Part::Value, latin1);
        }
        let parts: Vec<&str> = lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let from = if i == 0 {
                    entry.value_range.start
                } else {
                    line.start
                };
                let content = &self.text[from.min(line.content_end)..line.content_end];
                let content = content.trim_start_matches(is_space);
                if continues(content) {
                    &content[..content.len() - 1]
                } else {
                    content
                }
            })
            .collect();
        let second = lines[1];
        let second_text = &self.text[second.start..second.content_end];
        let indent =
            &second_text[..second_text.len() - second_text.trim_start_matches(is_space).len()];
        let newline = &self.text[lines[0].content_end..lines[0].end];
        let chunks = split_like(value, &parts);
        chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                let part = if i == 0 {
                    Part::Value
                } else {
                    Part::Continuation
                };
                escape(chunk, part, latin1)
            })
            .collect::<Vec<_>>()
            .join(&format!("\\{newline}{indent}"))
    }

    fn plan_new(&self, plan: &mut Plan, name: &str, value: &str) {
        plan.count += 1;
        plan.expected.push((name.to_owned(), value.to_owned()));
        let separator = self
            .parsed
            .entries
            .iter()
            .rev()
            .map(|e| &self.text[e.key_range.end..e.value_range.start])
            .find(|s| !s.is_empty() && !s.contains(['\n', '\r', '\\']))
            .unwrap_or("=");
        let newline = dominant_newline(&self.text);
        let line = format!(
            "{}{}{separator}{}{newline}",
            line_break_before_append(&self.text, newline),
            escape(name, Part::Key, self.latin1()),
            escape(value, Part::Value, self.latin1()),
        );
        plan.splices.push(Splice::insert(self.text.len(), line));
    }

    fn commit(&mut self, plan: Plan) -> Result<usize, Error> {
        let splices: Vec<Splice> = plan
            .splices
            .into_iter()
            .filter(|s| self.text.get(s.range.clone()) != Some(s.text.as_str()))
            .collect();
        if splices.is_empty() {
            return Ok(plan.count);
        }
        let failed = || {
            unsupported(
                VERIFY,
                "the changed property file does not read back with the expected keys; nothing was changed",
            )
        };
        let text = apply_splices(&self.text, &splices).ok_or_else(failed)?;
        let changed = Self::from_text(text, self.encoding, self.bom).map_err(|_| failed())?;
        let found: Vec<(String, String)> = changed
            .parsed
            .entries
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect();
        if found != plan.expected {
            return Err(failed());
        }
        *self = changed;
        Ok(plan.count)
    }
}

/// Splits `value` into lines like `parts`, the decoded lines of the
/// current value.
fn split_like(value: &str, parts: &[&str]) -> Vec<String> {
    let delimiter = parts[..parts.len() - 1]
        .iter()
        .map(|p| {
            p.trim_end_matches(is_space)
                .chars()
                .last()
                .filter(|c| !c.is_alphanumeric())
        })
        .reduce(|a, b| if a == b { a } else { None })
        .flatten();
    let mut chunks = Vec::new();
    match delimiter {
        Some(delimiter) => {
            let mut current = String::new();
            for c in value.chars() {
                current.push(c);
                if c == delimiter {
                    chunks.push(std::mem::take(&mut current));
                }
            }
            if !current.is_empty() || chunks.is_empty() {
                chunks.push(current);
            }
        }
        None => {
            let width = parts
                .iter()
                .map(|p| p.chars().count())
                .max()
                .unwrap_or(1)
                .max(1);
            let chars: Vec<char> = value.chars().collect();
            chunks.extend(chars.chunks(width).map(|c| c.iter().collect::<String>()));
            if chunks.is_empty() {
                chunks.push(String::new());
            }
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn breaks_a_continued_value_after_the_same_delimiter() {
        let mut doc = PropertiesDocument::parse(b"list = a,\\\n       b\nx=1\n").unwrap();
        doc.set(&key("list"), &Value::Text("p,q,r".into()), false)
            .unwrap();
        assert_eq!(doc.text(), "list = p,\\\n       q,\\\n       r\nx=1\n");
    }

    #[test]
    fn breaks_a_continued_value_at_the_old_width() {
        let mut doc = PropertiesDocument::parse(b"t = abc\\\n  de\n").unwrap();
        doc.set(&key("t"), &Value::Text("12 4567".into()), false)
            .unwrap();
        assert_eq!(doc.text(), "t = 12 \\\n  456\\\n  7\n");
        assert_eq!(
            doc.to_value().get("t"),
            Some(&Value::Text("12 4567".into()))
        );
    }

    #[test]
    fn writes_characters_outside_latin1_as_unicode_escapes() {
        let mut doc = PropertiesDocument::parse(b"name=caf\xe9\n").unwrap();
        assert_eq!(doc.encoding(), PropertiesEncoding::Latin1);
        doc.set(&key("name"), &Value::Text("λöd€ é".into()), false)
            .unwrap();
        assert_eq!(doc.bytes(), b"name=\\u03BB\xf6d\\u20AC \xe9\n");
    }

    #[test]
    fn creates_keys_with_the_separator_of_the_file() {
        let mut doc = PropertiesDocument::parse(b"a: 1").unwrap();
        doc.set(&key("new.key"), &Value::Int(2), true).unwrap();
        assert_eq!(doc.text(), "a: 1\nnew.key: 2\n");
        assert!(doc.set(&key("other"), &Value::Int(2), false).is_err());
    }

    #[test]
    fn removes_every_occurrence_with_its_comment() {
        let mut doc = PropertiesDocument::parse(b"# a\na=1\nb=2\na=3\n").unwrap();
        assert_eq!(
            doc.get(&key("a")).unwrap()[0].value,
            Value::Text("3".into())
        );
        assert_eq!(doc.remove(&key("a")).unwrap(), 1);
        assert_eq!(doc.text(), "b=2\n");
    }

    #[test]
    fn same_value_changes_no_byte_and_nested_values_fail() {
        let original = b"\xef\xbb\xbfk = v\n";
        let mut doc = PropertiesDocument::parse(original).unwrap();
        doc.set(&key("k"), &Value::Text("v".into()), false).unwrap();
        assert_eq!(doc.bytes(), original);
        let error = doc.set(&key("k"), &Value::List(vec![]), false).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(NESTED));
        let item = &doc.get(&key("k")).unwrap()[0];
        assert_eq!(item.span.unwrap().bytes.start, 7);
    }
}
