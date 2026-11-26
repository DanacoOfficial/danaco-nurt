// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! TOML documents.
//!
//! Documents are parsed with `toml_edit`, which keeps the byte range of
//! every key, value and table header. Changes replace or insert byte ranges
//! of the original text; the document is never serialized again. New keys
//! are placed by the structure that holds them: at the end of the content of
//! a header table (`[a]`), as a dotted key next to the other keys of a
//! dotted-key table (`a.b = 1`), inside an inline table, or under a new
//! header at the end of the file when no table exists. `a[2].b` addresses a
//! table of an array of tables and `a[+]` appends one under a new `[[a]]`
//! header. Every change is checked by parsing the result again and
//! comparing it with the expected content.

mod edit;
mod encode;
mod format;

pub use format::TomlFormat;

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocItem;
use nurt_contracts::value::{Date, DateKind, Fields, Value};
use toml_edit::{Array, ArrayOfTables, Document, InlineTable, Item, Key, Table};

use crate::formats::ini::support::errors::{key_conflict, parse_error};
use crate::formats::ini::support::flat::block_range;
use crate::formats::ini::support::text::{Line, LineIndex, lines};
use crate::formats::ini::support::tree::{Map, Node, resolve};

/// File name extension of TOML documents.
pub const EXTENSIONS: [&str; 1] = ["toml"];

/// File names of TOML documents without the `.toml` extension, lower case.
pub const FILE_NAMES: [&str; 3] = ["cargo.lock", "pipfile", "poetry.lock"];

/// Registration function of the format `toml`.
#[must_use]
pub fn formats() -> Vec<Box<dyn crate::registry::DocFormat>> {
    vec![Box::new(TomlFormat)]
}

/// Name of the format in error messages.
const FORMAT: &str = "toml";

/// Parsed TOML document that keeps its original text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::toml::TomlDocument;
///
/// let mut doc = TomlDocument::parse("[package]\nname = 'demo' # crate\n\n[deps]\n")?;
/// let key = KeyPath::parse("package.name")?;
/// assert_eq!(doc.get(&key)?[0].value, Value::Text("demo".into()));
/// doc.set(&key, &Value::Text("other".into()), false)?;
/// doc.set(&KeyPath::parse("package.version")?, &Value::Text("1.0.0".into()), true)?;
/// assert_eq!(
///     doc.text(),
///     "[package]\nname = 'other' # crate\nversion = \"1.0.0\"\n\n[deps]\n"
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct TomlDocument {
    text: String,
    doc: Document<String>,
    tree: Node,
    leaves: Vec<Value>,
    lines: Vec<Line>,
    comment_lines: Vec<bool>,
    /// Lines of the table headers (`[a]`, `[[a]]`), in document order.
    headers: Vec<usize>,
}

/// Node of the document reached by a key path.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Found<'a> {
    Table(&'a Table),
    Value(&'a toml_edit::Value),
    Tables(&'a ArrayOfTables),
}

/// Structure that holds a located node.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Parent<'a> {
    Table,
    Inline(&'a InlineTable),
    Array(&'a Array),
    Tables,
}

/// Located node with the key or index under which its parent holds it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Located<'a> {
    pub(crate) found: Found<'a>,
    pub(crate) parent: Option<Parent<'a>>,
    pub(crate) key: Option<&'a Key>,
    pub(crate) index: usize,
}

impl TomlDocument {
    /// Parses a TOML document.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` with the line and column of the first error.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let doc = Document::parse(text.to_owned()).map_err(|error| {
            let offset = error.span().map_or(0, |span| span.start);
            parse_error(FORMAT, error.message(), text, offset)
        })?;
        let line_table = lines(text);
        let mut comment_lines: Vec<bool> = line_table
            .iter()
            .map(|line| line.content(text).trim_start().starts_with('#'))
            .collect();
        let mut leaves = Vec::new();
        let root = root_table(&doc)?;
        let mut strings = Vec::new();
        string_spans(root, &mut strings);
        for span in strings {
            let first = line_table.partition_point(|line| line.end <= span.start);
            for (index, line) in line_table.iter().enumerate().skip(first + 1) {
                if line.start >= span.end {
                    break;
                }
                comment_lines[index] = false;
            }
        }
        let tree = table_node(root, text, &mut leaves);
        let mut header_spans = Vec::new();
        collect_headers(root, &mut header_spans);
        let mut document = Self {
            text: text.to_owned(),
            doc,
            tree,
            leaves,
            lines: line_table,
            comment_lines,
            headers: Vec::new(),
        };
        let mut headers: Vec<usize> = header_spans.iter().map(|&s| document.line_of(s)).collect();
        headers.sort_unstable();
        document.headers = headers;
        Ok(document)
    }

    /// Text of the document, including all changes made so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Values selected by `key`; tables are maps, arrays and arrays of
    /// tables lists, dates [`Value::Date`] with the text of the document.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing key, `E_KEY_CONFLICT` for a path
    /// through a value.
    pub fn get(&self, key: &KeyPath) -> Result<Vec<DocItem>, Error> {
        let value_of = |node: &Node| self.node_value(node);
        let hits = resolve(&self.tree, key, &value_of)?;
        let index = LineIndex::new(&self.text);
        hits.iter()
            .map(|hit| {
                let path = KeyPath::new(None, hit.path.clone())
                    .map_err(|error| key_conflict(&error.to_string(), "key path", &[]))?;
                let located = self.locate(&hit.path);
                let range = located.map_or(0..self.text.len(), |l| self.extent(&l));
                let comment = located.and_then(|l| self.comment(&l));
                Ok(DocItem {
                    key: path,
                    value: self.node_value(hit.node),
                    span: Some(index.span(&self.text, range)),
                    comment,
                    document: None,
                    anchor: None,
                    alias_of: None,
                    tag: None,
                })
            })
            .collect()
    }

    /// Content of the document.
    pub fn to_value(&self) -> Value {
        self.node_value(&self.tree)
    }

    /// Sets the value at `key`; `create` adds missing keys and tables,
    /// `[+]` appends to an array or an array of tables. Returns the number
    /// of values set.
    ///
    /// The string style, integer radix and layout of the current value are
    /// kept where they can hold the new value; lists and maps replace
    /// arrays and tables element by element and key by key.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing key without `create`,
    /// `E_KEY_CONFLICT` for paths that do not fit the document and
    /// `E_UNSUPPORTED_CONSTRUCT` with the construct name when the change
    /// cannot be made without losing layout or meaning, for example
    /// `toml.null` or `toml.table_to_value`.
    pub fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize, Error> {
        let text = edit::set(self, key, value, create)?;
        self.accept(text)
    }

    /// Removes the keys, tables or array elements selected by `key`, with
    /// the comment lines directly above them. Returns the number of values
    /// removed.
    ///
    /// # Errors
    ///
    /// The errors of [`TomlDocument::get`]; `E_KEY_CONFLICT` for the
    /// document root.
    pub fn remove(&mut self, key: &KeyPath) -> Result<usize, Error> {
        let text = edit::remove(self, key)?;
        self.accept(text)
    }

    fn accept(&mut self, (text, count): (Option<String>, usize)) -> Result<usize, Error> {
        if let Some(text) = text {
            *self = Self::parse(&text)?;
        }
        Ok(count)
    }

    pub(crate) fn root(&self) -> Option<&Table> {
        self.doc.as_item().as_table()
    }

    pub(crate) fn tree(&self) -> &Node {
        &self.tree
    }

    pub(crate) fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub(crate) fn node_value(&self, node: &Node) -> Value {
        match node {
            Node::Leaf(id) => self.leaves[*id].clone(),
            Node::Map(map) => Value::Map(
                map.members
                    .iter()
                    .map(|(name, node)| (name.clone(), self.node_value(node)))
                    .collect::<Fields>(),
            ),
            Node::List(elements) | Node::Repeated(elements) => {
                Value::List(elements.iter().map(|n| self.node_value(n)).collect())
            }
        }
    }

    /// Node reached by a concrete path of names and non-negative indices.
    pub(crate) fn locate(&self, path: &[KeySegment]) -> Option<Located<'_>> {
        let mut located = Located {
            found: Found::Table(self.root()?),
            parent: None,
            key: None,
            index: 0,
        };
        for segment in path {
            located = step(located.found, segment)?;
        }
        Some(located)
    }

    /// Index of the line that contains `offset`.
    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.lines
            .partition_point(|line| line.end <= offset)
            .min(self.lines.len().saturating_sub(1))
    }

    /// Byte range of a key and value pair: its lines with the comment lines
    /// directly above.
    pub(crate) fn pair_block(&self, key: &Key, value_end: usize) -> Option<Range<usize>> {
        let first = self.line_of(key.span()?.start);
        let last = self.line_of(value_end.saturating_sub(1));
        Some(block_range(
            &self.lines,
            &self.comment_lines,
            first,
            last,
            0,
        ))
    }

    /// Byte range of a table header with its content, up to the comment
    /// block of the next header.
    pub(crate) fn header_block(&self, header: &Range<usize>) -> Range<usize> {
        let first = self.line_of(header.start);
        let mut end = self
            .headers
            .iter()
            .copied()
            .find(|&line| line > first)
            .unwrap_or(self.lines.len());
        if end < self.lines.len() {
            while end > first + 1 && self.comment_lines[end - 1] {
                end -= 1;
            }
        }
        block_range(&self.lines, &self.comment_lines, first, end - 1, 0)
    }

    /// Byte ranges of the lines that form a table and its descendants.
    pub(crate) fn table_blocks(&self, table: &Table, out: &mut Vec<Range<usize>>) {
        if let Some(span) = header_span(table) {
            out.push(self.header_block(&span));
        }
        for (name, item) in table.iter() {
            match item {
                Item::Value(value) => {
                    let block = table
                        .get_key_value(name)
                        .map(|(key, _)| key)
                        .zip(value_end(value))
                        .and_then(|(key, end)| self.pair_block(key, end));
                    out.extend(block);
                }
                Item::Table(child) => self.table_blocks(child, out),
                Item::ArrayOfTables(tables) => {
                    for child in tables.iter() {
                        self.table_blocks(child, out);
                    }
                }
                Item::None => {}
            }
        }
    }

    /// Byte range reported for a located node.
    fn extent(&self, located: &Located<'_>) -> Range<usize> {
        match located.found {
            Found::Value(value) => value.span().unwrap_or(0..0),
            Found::Table(_) if located.parent.is_none() => 0..self.text.len(),
            Found::Table(table) => self.span_of_blocks(|out| self.table_blocks(table, out)),
            Found::Tables(tables) => self.span_of_blocks(|out| {
                for table in tables.iter() {
                    self.table_blocks(table, out);
                }
            }),
        }
    }

    fn span_of_blocks(&self, collect: impl FnOnce(&mut Vec<Range<usize>>)) -> Range<usize> {
        let mut blocks = Vec::new();
        collect(&mut blocks);
        let start = blocks.iter().map(|b| b.start).min().unwrap_or(0);
        let end = blocks.iter().map(|b| b.end).max().unwrap_or(0);
        start..end
    }

    /// Comment lines directly above the key or header of a node and the
    /// comment after its value on the same line.
    fn comment(&self, located: &Located<'_>) -> Option<String> {
        let (start, end) = match located.found {
            Found::Value(value) => (located.key?.span()?.start, value_end(value)?),
            Found::Table(table) => {
                let span = header_span(table)?;
                (span.start, span.end)
            }
            Found::Tables(tables) => {
                let span = header_span(tables.get(0)?)?;
                (span.start, span.end)
            }
        };
        let mut parts = Vec::new();
        let mut line = self.line_of(start);
        while line > 0 && self.comment_lines[line - 1] {
            line -= 1;
        }
        for index in line..self.line_of(start) {
            let content = self.lines[index].content(&self.text).trim_start();
            parts.push(content.trim_start_matches('#').trim().to_owned());
        }
        let last = self.lines[self.line_of(end.saturating_sub(1))];
        let after = self.text[end.min(last.content_end)..last.content_end].trim_start();
        if let Some(note) = after.strip_prefix('#') {
            parts.push(note.trim().to_owned());
        }
        (!parts.is_empty()).then(|| parts.join("\n"))
    }
}

fn root_table(doc: &Document<String>) -> Result<&Table, Error> {
    doc.as_item().as_table().ok_or_else(|| {
        Error::new(
            ErrorCode::DocParse,
            "toml document has no root table; check the document",
        )
    })
}

/// Header span of a table written with a `[a]` or `[[a]]` header.
pub(crate) fn header_span(table: &Table) -> Option<Range<usize>> {
    if table.is_implicit() || table.is_dotted() {
        None
    } else {
        table.span()
    }
}

/// Offset after the last byte of a value, including the values of a dotted
/// inline table.
pub(crate) fn value_end(value: &toml_edit::Value) -> Option<usize> {
    match value {
        toml_edit::Value::InlineTable(table) if table.is_dotted() => {
            table.iter().filter_map(|(_, v)| value_end(v)).max()
        }
        other => other.span().map(|span| span.end),
    }
}

/// Byte ranges of the string values of a table and its descendants. The
/// lines that a string spans after its first one are text, not comments.
fn string_spans(table: &Table, out: &mut Vec<Range<usize>>) {
    for (_, item) in table.iter() {
        match item {
            Item::Value(value) => value_string_spans(value, out),
            Item::Table(child) => string_spans(child, out),
            Item::ArrayOfTables(tables) => {
                for child in tables.iter() {
                    string_spans(child, out);
                }
            }
            Item::None => {}
        }
    }
}

fn value_string_spans(value: &toml_edit::Value, out: &mut Vec<Range<usize>>) {
    match value {
        toml_edit::Value::String(_) => out.extend(value.span()),
        toml_edit::Value::Array(array) => {
            for element in array.iter() {
                value_string_spans(element, out);
            }
        }
        toml_edit::Value::InlineTable(table) => {
            for (_, member) in table.iter() {
                value_string_spans(member, out);
            }
        }
        _ => {}
    }
}

fn collect_headers(table: &Table, out: &mut Vec<usize>) {
    for (_, item) in table.iter() {
        match item {
            Item::Table(child) => {
                out.extend(header_span(child).map(|s| s.start));
                collect_headers(child, out);
            }
            Item::ArrayOfTables(tables) => {
                for child in tables.iter() {
                    out.extend(child.span().map(|s| s.start));
                    collect_headers(child, out);
                }
            }
            Item::Value(_) | Item::None => {}
        }
    }
}

fn found_of(item: &Item) -> Option<Found<'_>> {
    match item {
        Item::Value(value) => Some(Found::Value(value)),
        Item::Table(table) => Some(Found::Table(table)),
        Item::ArrayOfTables(tables) => Some(Found::Tables(tables)),
        Item::None => None,
    }
}

fn step<'a>(found: Found<'a>, segment: &KeySegment) -> Option<Located<'a>> {
    match (found, segment) {
        (Found::Table(table), KeySegment::Key(name)) => {
            let (key, item) = table.get_key_value(name)?;
            Some(Located {
                found: found_of(item)?,
                parent: Some(Parent::Table),
                key: Some(key),
                index: 0,
            })
        }
        (Found::Value(toml_edit::Value::InlineTable(table)), KeySegment::Key(name)) => {
            let (key, item) = table.get_key_value(name)?;
            Some(Located {
                found: found_of(item)?,
                parent: Some(Parent::Inline(table)),
                key: Some(key),
                index: 0,
            })
        }
        (Found::Value(toml_edit::Value::Array(array)), KeySegment::Index(index)) => {
            let index = usize::try_from(*index).ok()?;
            Some(Located {
                found: Found::Value(array.get(index)?),
                parent: Some(Parent::Array(array)),
                key: None,
                index,
            })
        }
        (Found::Tables(tables), KeySegment::Index(index)) => {
            let index = usize::try_from(*index).ok()?;
            Some(Located {
                found: Found::Table(tables.get(index)?),
                parent: Some(Parent::Tables),
                key: None,
                index,
            })
        }
        _ => None,
    }
}

fn table_node(table: &Table, text: &str, leaves: &mut Vec<Value>) -> Node {
    let mut map = Map::new(false);
    for (name, item) in table.iter() {
        let node = match item {
            Item::Value(value) => value_node(value, text, leaves),
            Item::Table(child) => table_node(child, text, leaves),
            Item::ArrayOfTables(tables) => Node::List(
                tables
                    .iter()
                    .map(|child| table_node(child, text, leaves))
                    .collect(),
            ),
            Item::None => continue,
        };
        map.push(name.to_owned(), node);
    }
    Node::Map(map)
}

fn value_node(value: &toml_edit::Value, text: &str, leaves: &mut Vec<Value>) -> Node {
    match value {
        toml_edit::Value::Array(array) => {
            Node::List(array.iter().map(|v| value_node(v, text, leaves)).collect())
        }
        toml_edit::Value::InlineTable(table) => {
            let mut map = Map::new(false);
            for (name, child) in table.iter() {
                map.push(name.to_owned(), value_node(child, text, leaves));
            }
            Node::Map(map)
        }
        scalar => {
            leaves.push(scalar_value(scalar, text));
            Node::Leaf(leaves.len() - 1)
        }
    }
}

fn scalar_value(value: &toml_edit::Value, text: &str) -> Value {
    match value {
        toml_edit::Value::String(s) => Value::Text(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::Int(*i.value()),
        toml_edit::Value::Float(f) => Value::Number(*f.value()),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => {
            let datetime = d.value();
            let kind = match (datetime.date, datetime.time, datetime.offset) {
                (Some(_), Some(_), Some(_)) => DateKind::OffsetDateTime,
                (Some(_), Some(_), None) => DateKind::LocalDateTime,
                (Some(_), None, _) => DateKind::LocalDate,
                _ => DateKind::LocalTime,
            };
            let raw = d.span().and_then(|span| text.get(span));
            Value::Date(Date {
                kind,
                text: raw.map_or_else(|| datetime.to_string(), str::to_owned),
            })
        }
        toml_edit::Value::Array(_) | toml_edit::Value::InlineTable(_) => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    const SAMPLE: &str = "\
# top
title = \"demo\"
when = 1979-05-27T07:32:00-08:00
day = 1979-05-27
time = 07:32:00
local = 1979-05-27T07:32:00
hex = 0xDEAD_BEEF
inf = -inf
point = { x = 1, y = 2 }
list = [1, 2, 3]

[server] # main
host = 'localhost'
dotted.port = 80

[[fruit]]
name = \"apple\"
[fruit.color]
red = 1

[[fruit]]
name = \"banana\"
";

    #[test]
    fn reads_scalars_with_their_kinds() {
        let doc = TomlDocument::parse(SAMPLE).unwrap();
        let value = doc.to_value();
        let kinds: Vec<DateKind> = ["when", "day", "time", "local"]
            .iter()
            .map(|name| match value.get(name) {
                Some(Value::Date(date)) => date.kind,
                other => panic!("date expected, found {other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                DateKind::OffsetDateTime,
                DateKind::LocalDate,
                DateKind::LocalTime,
                DateKind::LocalDateTime
            ]
        );
        assert_eq!(value.get("hex"), Some(&Value::Int(0xDEAD_BEEF)));
        assert_eq!(value.get("inf"), Some(&Value::Number(f64::NEG_INFINITY)));
    }

    #[test]
    fn addresses_tables_arrays_of_tables_and_inline_tables() {
        let doc = TomlDocument::parse(SAMPLE).unwrap();
        let get = |k: &str| doc.get(&key(k)).unwrap();
        assert_eq!(get("fruit[1].name")[0].value, Value::Text("banana".into()));
        assert_eq!(get("fruit[0].color.red")[0].value, Value::Int(1));
        assert_eq!(get("point.y")[0].value, Value::Int(2));
        assert_eq!(get("server.dotted.port")[0].value, Value::Int(80));
        assert_eq!(get("fruit[*].name").len(), 2);
        assert_eq!(get("list[-1]")[0].value, Value::Int(3));
    }

    #[test]
    fn reports_positions_and_comments() {
        let doc = TomlDocument::parse(SAMPLE).unwrap();
        let title = &doc.get(&key("title")).unwrap()[0];
        assert_eq!(title.comment.as_deref(), Some("top"));
        let span = title.span.unwrap();
        assert_eq!((span.start.line, span.start.column), (2, 9));
        let server = &doc.get(&key("server")).unwrap()[0];
        assert_eq!(server.comment.as_deref(), Some("main"));
        assert_eq!(server.span.unwrap().start.line, 12);
    }

    #[test]
    fn rejects_invalid_documents_with_their_position() {
        let error = TomlDocument::parse("a = 1\nb = \n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse);
        assert_eq!(error.location.as_ref().and_then(|l| l.line), Some(2));
        assert!(TomlDocument::parse("a = 1\na = 2\n").is_err());
    }

    #[test]
    fn lines_inside_multi_line_strings_are_not_comments() {
        let text = "a = 1\n# real\ns = \"\"\"\nx\n# not a comment\n  # nor this\n\"\"\"\nb = '''\n# lit\n'''\nc = [\n  # inside array\n  1,\n]\n";
        let doc = TomlDocument::parse(text).unwrap();
        let flags: Vec<bool> = doc.comment_lines.clone();
        assert_eq!(
            flags,
            [
                false, true, false, false, false, false, false, false, false, false, false, true,
                false, false
            ]
        );
    }

    #[test]
    fn removing_a_key_keeps_the_text_of_the_string_above() {
        let mut doc = TomlDocument::parse("s = \"\"\"\nline\n# kept\"\"\"\nk = 1\n").unwrap();
        doc.remove(&key("k")).unwrap();
        assert_eq!(doc.text(), "s = \"\"\"\nline\n# kept\"\"\"\n");
        let value = doc.to_value();
        assert_eq!(value.get("s"), Some(&Value::Text("line\n# kept".into())));
    }

    #[test]
    fn header_blocks_end_at_the_comment_block_of_the_next_header() {
        let text = "[a]\nx = 1\n\n# about b\n[b]\ny = 2";
        let doc = TomlDocument::parse(text).unwrap();
        let block = doc.header_block(&(0..3));
        assert_eq!(&text[block], "[a]\nx = 1\n\n");
        let last = doc.header_block(&(text.find("[b]").unwrap()..text.len()));
        assert_eq!(&text[last], "\n# about b\n[b]\ny = 2");
    }
}
