// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::toml::edit
// Responsibility:   Planning and verification of changes to TOML documents.
// Dependencies:     nurt_contracts, toml_edit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Planning and verification of changes to TOML documents.

use std::collections::BTreeMap;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::{Fields, Value};
use toml_edit::{Array, ArrayOfTables, InlineTable, Item, Key, Table};

use super::encode::{dotted, inline, normalize, same};
use super::{Found, Parent, TomlDocument, header_span, value_end};
use crate::formats::ini::support::errors::{key_conflict, key_not_found, unsupported};
use crate::formats::ini::support::flat::removals;
use crate::formats::ini::support::text::{
    Splice, apply_splices, dominant_newline, line_break_before_append,
};
use crate::formats::ini::support::tree::{Node, print_path, resolve};

/// Construct name: a table assigned a value that is not a map.
const TABLE_TO_VALUE: &str = "toml.table_to_value";
/// Construct name: a structural change inside a dotted key of an inline
/// table.
const INLINE_DOTTED: &str = "toml.inline_dotted_key";
/// Construct name: a table that a header cannot address, such as a new
/// table under an element of an array of tables that is not the last one.
const TABLE_POSITION: &str = "toml.table_position";
/// Construct name: an array of tables without elements.
const EMPTY_TABLES: &str = "toml.empty_array_of_tables";
/// Construct name: an array or inline table whose layout has no byte range.
const LAYOUT: &str = "toml.layout";
/// Construct name: the changed document does not read back as expected.
const VERIFY: &str = "toml.verify";

/// Separator written in new pairs when the table has no pair to copy.
const DEFAULT_SEPARATOR: &str = " = ";

/// Text of the changed document (`None` when nothing changes) and the
/// number of values set or removed.
pub(crate) type Outcome = (Option<String>, usize);

/// Plans and checks `set` of `value` at `key`.
pub(crate) fn set(
    doc: &TomlDocument,
    key: &KeyPath,
    value: &Value,
    create: bool,
) -> Result<Outcome, Error> {
    let value = normalize(value)?;
    let mut planner = Planner::new(doc);
    let count = if let Some(parent) = append_target(doc, key) {
        planner.append(&parent, &value, create)?;
        1
    } else {
        let value_of = |node: &Node| doc.node_value(node);
        match resolve(doc.tree(), key, &value_of) {
            Ok(hits) => {
                for hit in &hits {
                    planner.replace(&hit.path, &value)?;
                }
                hits.len()
            }
            Err(error) if error.code() == ErrorCode::KeyNotFound && create && key.is_concrete() => {
                let path = creation_path(doc, key)?;
                planner.create(&path, &value)?;
                1
            }
            Err(error) => return Err(error),
        }
    };
    Ok((planner.finish()?, count))
}

/// Plans and checks removal of the values selected by `key`.
pub(crate) fn remove(doc: &TomlDocument, key: &KeyPath) -> Result<Outcome, Error> {
    if key.is_root() {
        return Err(key_conflict(
            "the document root cannot be removed; address a key or a table",
            "document root",
            &[],
        ));
    }
    let value_of = |node: &Node| doc.node_value(node);
    let hits = resolve(doc.tree(), key, &value_of)?;
    let mut planner = Planner::new(doc);
    for hit in hits.iter().rev() {
        planner.remove_path(&hit.path)?;
    }
    Ok((planner.finish()?, hits.len()))
}

/// Parent of a key ending with `[+]`, or with an index equal to the length
/// of an existing array.
fn append_target(doc: &TomlDocument, key: &KeyPath) -> Option<KeyPath> {
    let (last, rest) = key.segments().split_last()?;
    let parent = KeyPath::new(key.document(), rest.to_vec()).ok()?;
    match last {
        KeySegment::Append => Some(parent),
        KeySegment::Index(index) => {
            let value_of = |node: &Node| doc.node_value(node);
            let hits = resolve(doc.tree(), &parent, &value_of).ok()?;
            match hits.as_slice() {
                [hit] => match hit.node {
                    Node::List(elements)
                        if usize::try_from(*index).ok() == Some(elements.len()) =>
                    {
                        Some(parent)
                    }
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

/// Concrete path of a key to create: the longest existing prefix resolved
/// to its concrete path, followed by the new names.
fn creation_path(doc: &TomlDocument, key: &KeyPath) -> Result<Vec<KeySegment>, Error> {
    let segments = key.segments();
    let value_of = |node: &Node| doc.node_value(node);
    for depth in (0..segments.len()).rev() {
        let Ok(prefix) = KeyPath::new(None, segments[..depth].to_vec()) else {
            continue;
        };
        if let Ok(hits) = resolve(doc.tree(), &prefix, &value_of)
            && let [hit] = hits.as_slice()
        {
            let rest = &segments[depth..];
            if rest.iter().any(|s| !matches!(s, KeySegment::Key(_))) {
                return Err(key_conflict(
                    "a new key is addressed by names; remove indices and wildcards after the existing part",
                    "segment",
                    &[],
                ));
            }
            let mut path = hit.path.clone();
            path.extend_from_slice(rest);
            return Ok(path);
        }
    }
    Ok(segments.to_vec())
}

fn names_of(path: &[KeySegment]) -> Option<Vec<String>> {
    path.iter()
        .map(|segment| match segment {
            KeySegment::Key(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn extend(path: &[KeySegment], segment: KeySegment) -> Vec<KeySegment> {
    let mut path = path.to_vec();
    path.push(segment);
    path
}

fn value_at<'v>(value: &'v Value, path: &[KeySegment]) -> Option<&'v Value> {
    path.iter()
        .try_fold(value, |current, segment| match (current, segment) {
            (Value::Map(map), KeySegment::Key(name)) => map.get(name),
            (Value::List(list), KeySegment::Index(index)) => {
                list.get(usize::try_from(*index).ok()?)
            }
            _ => None,
        })
}

/// Value at `path`, with missing map members created as empty maps.
fn slot<'v>(value: &'v mut Value, path: &[KeySegment]) -> Option<&'v mut Value> {
    path.iter()
        .try_fold(value, |current, segment| match (current, segment) {
            (Value::Map(map), KeySegment::Key(name)) => Some(
                map.entry(name.clone())
                    .or_insert_with(|| Value::Map(Fields::new())),
            ),
            (Value::List(list), KeySegment::Index(index)) => {
                list.get_mut(usize::try_from(*index).ok()?)
            }
            _ => None,
        })
}

/// Kind of a table for the placement of new keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableKind {
    Root,
    Header,
    Dotted,
    Implicit,
}

fn table_kind(path: &[KeySegment], table: &Table) -> TableKind {
    if path.is_empty() {
        TableKind::Root
    } else if header_span(table).is_some() {
        TableKind::Header
    } else if table.is_dotted() {
        TableKind::Dotted
    } else {
        TableKind::Implicit
    }
}

/// Last key and value pair of a table, including the pairs of its dotted
/// tables: the leaf key and the end of the value.
fn last_pair(table: &Table) -> Option<(&Key, usize)> {
    let mut best: Option<(&Key, usize)> = None;
    for (name, item) in table.iter() {
        let candidate = match item {
            Item::Value(value) => table
                .get_key_value(name)
                .map(|(key, _)| key)
                .zip(value_end(value)),
            Item::Table(child) if child.is_dotted() => last_pair(child),
            _ => None,
        };
        if let Some((key, end)) = candidate
            && best.is_none_or(|(_, b)| end > b)
        {
            best = Some((key, end));
        }
    }
    best
}

struct Planner<'a> {
    doc: &'a TomlDocument,
    text: &'a str,
    newline: &'static str,
    original: Value,
    expected: Value,
    splices: Vec<Splice>,
    removed: Vec<Range<usize>>,
    inserts: BTreeMap<usize, String>,
}

impl<'a> Planner<'a> {
    fn new(doc: &'a TomlDocument) -> Self {
        let original = doc.to_value();
        Self {
            doc,
            text: doc.text(),
            newline: dominant_newline(doc.text()),
            expected: original.clone(),
            original,
            splices: Vec::new(),
            removed: Vec::new(),
            inserts: BTreeMap::new(),
        }
    }

    fn finish(mut self) -> Result<Option<String>, Error> {
        let mut splices = std::mem::take(&mut self.splices);
        splices.extend(removals(std::mem::take(&mut self.removed)));
        splices.extend(
            std::mem::take(&mut self.inserts)
                .into_iter()
                .map(|(offset, text)| Splice::insert(offset, text)),
        );
        splices.retain(|s| self.text.get(s.range.clone()) != Some(s.text.as_str()));
        if splices.is_empty() {
            return Ok(None);
        }
        let text = apply_splices(self.text, &splices).ok_or_else(|| {
            key_conflict(
                "the changes overlap; address keys that do not contain each other",
                "overlapping changes",
                &[],
            )
        })?;
        let failed = || {
            unsupported(
                VERIFY,
                "the changed document does not read back with the expected content; nothing was changed",
            )
        };
        let changed = TomlDocument::parse(&text).map_err(|_| failed())?;
        if same(&changed.to_value(), &self.expected) {
            Ok(Some(text))
        } else {
            Err(failed())
        }
    }

    fn expect_set(&mut self, path: &[KeySegment], value: &Value) {
        if let Some(target) = slot(&mut self.expected, path) {
            *target = value.clone();
        }
    }

    fn expect_remove(&mut self, path: &[KeySegment]) {
        let Some((last, parent)) = path.split_last() else {
            return;
        };
        match (slot(&mut self.expected, parent), last) {
            (Some(Value::Map(map)), KeySegment::Key(name)) => {
                map.shift_remove(name);
            }
            (Some(Value::List(list)), KeySegment::Index(index)) => {
                if let Some(index) = usize::try_from(*index).ok().filter(|&i| i < list.len()) {
                    list.remove(index);
                }
            }
            _ => {}
        }
    }

    fn insert(&mut self, offset: usize, text: &str) {
        self.inserts.entry(offset).or_default().push_str(text);
    }

    /// Inserts `line` as a new line after the line `index`.
    fn insert_after_line(&mut self, index: usize, line: &str) {
        let target = self.doc.lines()[index];
        let text = if target.end == target.content_end {
            format!("{}{line}", self.newline)
        } else {
            format!("{line}{}", self.newline)
        };
        self.insert(target.end, &text);
    }

    fn missing(path: &[KeySegment]) -> Error {
        key_not_found(&print_path(path), &[])
    }

    fn replace(&mut self, path: &[KeySegment], value: &Value) -> Result<(), Error> {
        let Some(old) = value_at(&self.original, path).cloned() else {
            return Err(Self::missing(path));
        };
        if same(&old, value) {
            return Ok(());
        }
        let located = self.doc.locate(path).ok_or_else(|| Self::missing(path))?;
        match (located.found, value) {
            (Found::Table(_), Value::Map(fields))
            | (Found::Value(toml_edit::Value::InlineTable(_)), Value::Map(fields)) => {
                self.replace_fields(path, &old, fields)
            }
            (Found::Tables(tables), Value::List(items)) => self.replace_tables(path, tables, items),
            (Found::Value(toml_edit::Value::Array(array)), Value::List(items)) => {
                self.replace_elements(path, array, items)
            }
            (Found::Table(_) | Found::Tables(_), _) => Err(unsupported(
                TABLE_TO_VALUE,
                "a table holds keys; set a map, or remove the table and set the key again",
            )),
            (Found::Value(current), _) => self.replace_value(path, current, value),
        }
    }

    fn replace_value(
        &mut self,
        path: &[KeySegment],
        current: &toml_edit::Value,
        value: &Value,
    ) -> Result<(), Error> {
        let span = current.span().ok_or_else(Self::layout)?;
        let scalar = !matches!(
            current,
            toml_edit::Value::Array(_) | toml_edit::Value::InlineTable(_)
        );
        let old_raw = scalar.then(|| &self.text[span.clone()]);
        self.splices.push(Splice::new(span, inline(value, old_raw)));
        self.expect_set(path, value);
        Ok(())
    }

    fn layout() -> Error {
        unsupported(
            LAYOUT,
            "the layout of the array or inline table cannot be changed in place",
        )
    }

    fn replace_fields(
        &mut self,
        path: &[KeySegment],
        old: &Value,
        fields: &Fields,
    ) -> Result<(), Error> {
        let old_names: Vec<String> = match old {
            Value::Map(map) => map.keys().cloned().collect(),
            _ => Vec::new(),
        };
        for (name, value) in fields {
            let child = extend(path, KeySegment::Key(name.clone()));
            if old_names.contains(name) {
                self.replace(&child, value)?;
            } else {
                self.create(&child, value)?;
            }
        }
        for name in old_names.iter().filter(|n| !fields.contains_key(*n)) {
            self.remove_path(&extend(path, KeySegment::Key(name.clone())))?;
        }
        Ok(())
    }

    fn replace_elements(
        &mut self,
        path: &[KeySegment],
        array: &Array,
        items: &[Value],
    ) -> Result<(), Error> {
        let spans = array_spans(array)?;
        for (index, item) in items.iter().enumerate().take(spans.len()) {
            let index = i64::try_from(index).unwrap_or(i64::MAX);
            self.replace(&extend(path, KeySegment::Index(index)), item)?;
        }
        if items.len() < spans.len() {
            let from = items.len();
            let start = if from == 0 {
                spans[0].start
            } else {
                spans[from - 1].end
            };
            self.removed.push(start..spans[spans.len() - 1].end);
        } else if items.len() > spans.len() {
            self.array_insert(array, &items[spans.len()..])?;
        }
        self.expect_set(path, &Value::List(items.to_vec()));
        Ok(())
    }

    fn replace_tables(
        &mut self,
        path: &[KeySegment],
        tables: &ArrayOfTables,
        items: &[Value],
    ) -> Result<(), Error> {
        if items.is_empty() {
            return Err(unsupported(
                EMPTY_TABLES,
                "an array of tables cannot be empty; remove the key instead",
            ));
        }
        let old_len = tables.len();
        for (index, item) in items.iter().enumerate() {
            let Value::Map(fields) = item else {
                return Err(unsupported(
                    TABLE_TO_VALUE,
                    "elements of an array of tables are maps; write a map for each element",
                ));
            };
            if index < old_len {
                let index = i64::try_from(index).unwrap_or(i64::MAX);
                self.replace(&extend(path, KeySegment::Index(index)), item)?;
            } else {
                self.append_table(path, tables, fields)?;
            }
        }
        for table in tables.iter().skip(items.len()) {
            let mut blocks = Vec::new();
            self.doc.table_blocks(table, &mut blocks);
            self.removed.extend(blocks);
        }
        self.expect_set(path, &Value::List(items.to_vec()));
        Ok(())
    }

    fn append(&mut self, parent: &KeyPath, value: &Value, create: bool) -> Result<(), Error> {
        let value_of = |node: &Node| self.doc.node_value(node);
        match resolve(self.doc.tree(), parent, &value_of) {
            Ok(hits) => {
                for hit in hits {
                    self.append_at(&hit.path, value)?;
                }
                Ok(())
            }
            Err(error) if error.code() == ErrorCode::KeyNotFound && create => {
                let path = creation_path(self.doc, parent)?;
                self.create(&path, &Value::List(vec![value.clone()]))
            }
            Err(error) => Err(error),
        }
    }

    fn append_at(&mut self, path: &[KeySegment], value: &Value) -> Result<(), Error> {
        let located = self.doc.locate(path).ok_or_else(|| Self::missing(path))?;
        match (located.found, value) {
            (Found::Value(toml_edit::Value::Array(array)), _) => {
                self.array_insert(array, std::slice::from_ref(value))?;
            }
            (Found::Tables(tables), Value::Map(fields)) => {
                self.append_table(path, tables, fields)?
            }
            (Found::Tables(_), _) => {
                return Err(unsupported(
                    TABLE_TO_VALUE,
                    "elements of an array of tables are maps; write a map",
                ));
            }
            _ => {
                return Err(key_conflict(
                    &format!(
                        "`{}` is not an array; `[+]` adds an element to an array",
                        print_path(path)
                    ),
                    "not an array",
                    &[],
                ));
            }
        }
        if let Some(Value::List(list)) = slot(&mut self.expected, path) {
            list.push(value.clone());
        }
        Ok(())
    }

    /// Names of a table path written in a header, when a header can
    /// address it: every index selects the last table of its array.
    fn header_names(&self, path: &[KeySegment]) -> Option<Vec<String>> {
        let mut names = Vec::new();
        for (depth, segment) in path.iter().enumerate() {
            match segment {
                KeySegment::Key(name) => names.push(name.clone()),
                KeySegment::Index(index) => {
                    let located = self.doc.locate(&path[..depth])?;
                    let Found::Tables(tables) = located.found else {
                        return None;
                    };
                    if usize::try_from(*index).ok()? + 1 != tables.len() {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        Some(names)
    }

    fn pair_lines(&self, fields: &Fields) -> String {
        fields
            .iter()
            .map(|(name, value)| {
                format!(
                    "{}{DEFAULT_SEPARATOR}{}{}",
                    super::encode::key(name),
                    inline(value, None),
                    self.newline
                )
            })
            .collect()
    }

    /// Text that starts a new block at `offset`: a line break when the text
    /// before does not end with one, and a blank line before the block.
    fn block_opening(&self, offset: usize) -> String {
        let before = &self.text[..offset];
        let mut opening = line_break_before_append(before, self.newline).to_owned();
        let blank_before = before.is_empty()
            || before.ends_with("\n\n")
            || before.ends_with("\n\r\n")
            || before.trim_end_matches(['\n', '\r']).is_empty();
        if !blank_before {
            opening.push_str(self.newline);
        }
        opening
    }

    fn append_table(
        &mut self,
        path: &[KeySegment],
        tables: &ArrayOfTables,
        fields: &Fields,
    ) -> Result<(), Error> {
        let names = self.header_names(path).ok_or_else(|| {
            unsupported(
                TABLE_POSITION,
                "a header cannot address this array of tables; change the last element of the enclosing array instead",
            )
        })?;
        let mut blocks = Vec::new();
        if let Some(last) = tables.iter().last() {
            self.doc.table_blocks(last, &mut blocks);
        }
        let offset = blocks
            .iter()
            .map(|b| b.end)
            .max()
            .unwrap_or(self.text.len());
        let separation = if offset < self.text.len() {
            self.newline
        } else {
            ""
        };
        let block = format!(
            "{}[[{}]]{}{}{separation}",
            self.block_opening(offset),
            dotted(&names),
            self.newline,
            self.pair_lines(fields)
        );
        self.insert(offset, &block);
        Ok(())
    }

    fn new_header(&mut self, names: &[String], fields: &Fields) {
        let offset = self.text.len();
        let block = format!(
            "{}[{}]{}{}",
            self.block_opening(offset),
            dotted(names),
            self.newline,
            self.pair_lines(fields)
        );
        self.insert(offset, &block);
    }

    fn create(&mut self, path: &[KeySegment], value: &Value) -> Result<(), Error> {
        let depth = (0..path.len())
            .rev()
            .find(|&depth| self.doc.locate(&path[..depth]).is_some())
            .unwrap_or(0);
        let anchor = self
            .doc
            .locate(&path[..depth])
            .ok_or_else(|| Self::missing(path))?;
        let rest = names_of(&path[depth..]).ok_or_else(|| Self::missing(path))?;
        match anchor.found {
            Found::Table(table) => self.create_in_table(&path[..depth], table, &rest, value)?,
            Found::Value(toml_edit::Value::InlineTable(table)) => {
                self.create_in_inline(table, &rest, value)?;
            }
            _ => {
                return Err(key_conflict(
                    &format!(
                        "key path passes through a value at `{}`; address a table",
                        print_path(&path[..depth])
                    ),
                    "value",
                    &[],
                ));
            }
        }
        self.expect_set(path, value);
        Ok(())
    }

    fn create_in_table(
        &mut self,
        table_path: &[KeySegment],
        table: &Table,
        rest: &[String],
        value: &Value,
    ) -> Result<(), Error> {
        let kind = table_kind(table_path, table);
        let full = || -> Option<Vec<String>> {
            let mut names = self.header_names(table_path)?;
            names.extend_from_slice(rest);
            Some(names)
        };
        match (kind, value) {
            (TableKind::Root | TableKind::Header | TableKind::Implicit, Value::Map(fields)) => {
                if let Some(names) = full() {
                    self.new_header(&names, fields);
                    return Ok(());
                }
                self.insert_pair(table_path, table, kind, rest, value)
            }
            (TableKind::Root, _) if rest.len() > 1 => {
                let mut names = rest.to_vec();
                let leaf = names.pop().unwrap_or_default();
                let mut fields = Fields::new();
                fields.insert(leaf, value.clone());
                self.new_header(&names, &fields);
                Ok(())
            }
            (TableKind::Implicit, _) => {
                let mut names = full().ok_or_else(|| {
                    unsupported(
                        TABLE_POSITION,
                        "a header cannot address this table; add the key to a table with a header",
                    )
                })?;
                let leaf = names.pop().unwrap_or_default();
                let mut fields = Fields::new();
                fields.insert(leaf, value.clone());
                self.new_header(&names, &fields);
                Ok(())
            }
            _ => self.insert_pair(table_path, table, kind, rest, value),
        }
    }

    /// Dotted key prefix of a table relative to the nearest table with a
    /// header (or the root).
    fn dotted_prefix(&self, table_path: &[KeySegment]) -> Vec<String> {
        for depth in (0..table_path.len()).rev() {
            let header = match self.doc.locate(&table_path[..depth]).map(|l| l.found) {
                Some(Found::Table(table)) => depth == 0 || header_span(table).is_some(),
                _ => true,
            };
            if header {
                return names_of(&table_path[depth..]).unwrap_or_default();
            }
        }
        Vec::new()
    }

    fn insert_pair(
        &mut self,
        table_path: &[KeySegment],
        table: &Table,
        kind: TableKind,
        rest: &[String],
        value: &Value,
    ) -> Result<(), Error> {
        let mut names = if kind == TableKind::Dotted {
            self.dotted_prefix(table_path)
        } else {
            Vec::new()
        };
        names.extend_from_slice(rest);
        let reference = last_pair(table);
        let (indent, separator) = reference.map_or_else(
            || (String::new(), DEFAULT_SEPARATOR.to_owned()),
            |(key, _)| self.pair_style(key, table),
        );
        let line = format!(
            "{indent}{}{separator}{}",
            dotted(&names),
            inline(value, None)
        );
        match (reference, kind) {
            (Some((_, end)), _) => {
                self.insert_after_line(self.doc.line_of(end.saturating_sub(1)), &line)
            }
            (None, TableKind::Header) => {
                let header = header_span(table).ok_or_else(Self::layout)?;
                self.insert_after_line(self.doc.line_of(header.start), &line);
            }
            (None, _) => self.insert_at_root(&line),
        }
        Ok(())
    }

    /// Indentation and separator of the pair whose leaf key is `key`.
    fn pair_style(&self, key: &Key, table: &Table) -> (String, String) {
        let Some(span) = key.span() else {
            return (String::new(), DEFAULT_SEPARATOR.to_owned());
        };
        let line = self.doc.lines()[self.doc.line_of(span.start)];
        let content = line.content(self.text);
        let indent = &content[..content.len() - content.trim_start().len()];
        let value_start = find_value_start(table, key);
        let separator = value_start
            .and_then(|start| self.text.get(span.end..start))
            .filter(|s| s.contains('=') && !s.contains(['\n', '\r']))
            .unwrap_or(DEFAULT_SEPARATOR);
        (indent.to_owned(), separator.to_owned())
    }

    /// Inserts a pair of the root table, which has no pairs yet: before the
    /// comment block of the first header, or at the end of the document.
    fn insert_at_root(&mut self, line: &str) {
        match self.doc.headers.first() {
            Some(&first) => {
                let mut index = first;
                while index > 0 && self.doc.comment_lines[index - 1] {
                    index -= 1;
                }
                let offset = self.doc.lines()[index].start;
                let text = format!("{line}{0}{0}", self.newline);
                self.insert(offset, &text);
            }
            None => {
                let text = format!(
                    "{}{line}{}",
                    line_break_before_append(self.text, self.newline),
                    self.newline
                );
                self.insert(self.text.len(), &text);
            }
        }
    }

    fn create_in_inline(
        &mut self,
        table: &InlineTable,
        rest: &[String],
        value: &Value,
    ) -> Result<(), Error> {
        if table.is_dotted() {
            return Err(unsupported(
                INLINE_DOTTED,
                "keys cannot be added inside a dotted key of an inline table; write the inline table again",
            ));
        }
        let member = format!("{}{DEFAULT_SEPARATOR}{}", dotted(rest), inline(value, None));
        let span = table.span().ok_or_else(Self::layout)?;
        let members = member_spans(table)?;
        self.list_insert(span, &members, &[member], true)
    }

    fn array_insert(&mut self, array: &Array, values: &[Value]) -> Result<(), Error> {
        let span = array.span().ok_or_else(Self::layout)?;
        let elements = array_spans(array)?;
        let items: Vec<String> = values.iter().map(|v| inline(v, None)).collect();
        self.list_insert(span, &elements, &items, false)
    }

    /// Inserts items after the last element of an array or inline table
    /// spanning `span`, in the layout of the existing elements.
    fn list_insert(
        &mut self,
        span: Range<usize>,
        elements: &[Range<usize>],
        items: &[String],
        braces: bool,
    ) -> Result<(), Error> {
        let close = span.end.checked_sub(1).ok_or_else(Self::layout)?;
        let Some(last) = elements.last() else {
            let joined = items.join(", ");
            let text = if braces {
                format!(" {joined} ")
            } else {
                joined
            };
            self.insert(span.start + 1, &text);
            return Ok(());
        };
        let between = self.text.get(last.end..close).ok_or_else(Self::layout)?;
        let multiline = self.text[span.start..elements[0].start].contains('\n');
        let indent = if multiline {
            let line = self.doc.lines()[self.doc.line_of(last.start)];
            let content = line.content(self.text);
            content[..content.len() - content.trim_start().len()].to_owned()
        } else {
            String::new()
        };
        let newline = self.newline;
        let comma = between
            .find(',')
            .filter(|&c| between.find('#').is_none_or(|h| c < h));
        let (offset, text) = match comma {
            Some(c) => (
                last.end + c + 1,
                items
                    .iter()
                    .map(|i| {
                        if multiline {
                            format!("{newline}{indent}{i},")
                        } else {
                            format!(" {i},")
                        }
                    })
                    .collect::<String>(),
            ),
            None => (
                last.end,
                items
                    .iter()
                    .map(|i| {
                        if multiline {
                            format!(",{newline}{indent}{i}")
                        } else {
                            format!(", {i}")
                        }
                    })
                    .collect::<String>(),
            ),
        };
        self.insert(offset, &text);
        Ok(())
    }

    fn remove_path(&mut self, path: &[KeySegment]) -> Result<(), Error> {
        let located = self.doc.locate(path).ok_or_else(|| Self::missing(path))?;
        match (located.parent, located.found) {
            (None, _) => {
                return Err(key_conflict(
                    "the document root cannot be removed; address a key or a table",
                    "document root",
                    &[],
                ));
            }
            (Some(Parent::Array(array)), _) => {
                let spans = array_spans(array)?;
                self.removed
                    .push(element_range(self.text, &spans, located.index)?);
            }
            (Some(Parent::Inline(table)), found) => {
                let dotted_member = matches!(found, Found::Value(toml_edit::Value::InlineTable(t)) if t.is_dotted());
                if table.is_dotted() || dotted_member {
                    return Err(unsupported(
                        INLINE_DOTTED,
                        "keys cannot be removed inside a dotted key of an inline table; write the inline table again",
                    ));
                }
                let spans = member_spans(table)?;
                let start = located.key.and_then(Key::span).map(|s| s.start);
                let index = spans
                    .iter()
                    .position(|s| Some(s.start) == start)
                    .ok_or_else(Self::layout)?;
                self.removed.push(element_range(self.text, &spans, index)?);
            }
            (Some(Parent::Tables | Parent::Table), found) => {
                self.remove_blocks(located.key, found)?
            }
        }
        self.expect_remove(path);
        Ok(())
    }

    fn remove_blocks(&mut self, key: Option<&Key>, found: Found<'_>) -> Result<(), Error> {
        let mut blocks = Vec::new();
        match found {
            Found::Value(value) => {
                let block = key
                    .zip(value_end(value))
                    .and_then(|(key, end)| self.doc.pair_block(key, end))
                    .ok_or_else(Self::layout)?;
                blocks.push(block);
            }
            Found::Table(table) => self.doc.table_blocks(table, &mut blocks),
            Found::Tables(tables) => {
                for table in tables.iter() {
                    self.doc.table_blocks(table, &mut blocks);
                }
            }
        }
        self.removed.extend(blocks);
        Ok(())
    }
}

/// Start of the value of the pair whose leaf key is `key`, searching the
/// table and its dotted tables.
fn find_value_start(table: &Table, key: &Key) -> Option<usize> {
    let target = key.span()?;
    for (name, item) in table.iter() {
        match item {
            Item::Value(value) => {
                let (candidate, _) = table.get_key_value(name)?;
                if candidate.span() == Some(target.clone()) {
                    return value.span().map(|s| s.start);
                }
            }
            Item::Table(child) if child.is_dotted() => {
                if let Some(start) = find_value_start(child, key) {
                    return Some(start);
                }
            }
            _ => {}
        }
    }
    None
}

fn array_spans(array: &Array) -> Result<Vec<Range<usize>>, Error> {
    array
        .iter()
        .map(|value| value.span().ok_or_else(Planner::layout))
        .collect()
}

/// Byte ranges of the members of an inline table, from the key to the end
/// of the value, in document order.
fn member_spans(table: &InlineTable) -> Result<Vec<Range<usize>>, Error> {
    let mut spans = table
        .iter()
        .map(|(name, value)| {
            let start = table
                .get_key_value(name)
                .and_then(|(key, _)| key.span())
                .map(|s| s.start);
            start
                .zip(value_end(value))
                .map(|(start, end)| start..end)
                .ok_or_else(Planner::layout)
        })
        .collect::<Result<Vec<_>, _>>()?;
    spans.sort_by_key(|s| s.start);
    Ok(spans)
}

/// Byte range removed with the element `index` of an array or inline
/// table, kept within the lines of the element so that the lines of the
/// other elements keep their bytes.
///
/// The element goes with the comma that follows it on its line. The last
/// element without such a comma goes with the comma before it on its line;
/// when that comma ends the previous line, it stays as a trailing comma,
/// which arrays and multi-line inline tables accept. A line left without
/// content is removed whole.
///
/// # Errors
///
/// `E_UNSUPPORTED_CONSTRUCT` (`toml.layout`) when the separator of the
/// element is on another line.
fn element_range(text: &str, spans: &[Range<usize>], index: usize) -> Result<Range<usize>, Error> {
    let bytes = text.as_bytes();
    let span = spans[index].clone();
    let line_start = text[..span.start].rfind('\n').map_or(0, |at| at + 1);
    let alone = text[line_start..span.start]
        .bytes()
        .all(|b| b == b' ' || b == b'\t');
    let after = skip_blanks(bytes, span.end);
    if bytes.get(after) == Some(&b',') {
        let end = skip_blanks(bytes, after + 1);
        return Ok(own_line(bytes, line_start, alone, end).unwrap_or(span.start..end));
    }
    if index + 1 < spans.len() {
        return Err(Planner::layout());
    }
    if let Some(line) = own_line(bytes, line_start, alone, after) {
        return Ok(line);
    }
    let before = text[line_start..span.start].trim_end_matches([' ', '\t']);
    if before.ends_with(',') {
        Ok(line_start + before.len() - 1..span.end)
    } else if alone || spans.len() == 1 {
        Ok(span.start..after)
    } else {
        Err(Planner::layout())
    }
}

/// Offset of the first byte at or after `from` that is not a space or a
/// tab.
fn skip_blanks(bytes: &[u8], from: usize) -> usize {
    bytes[from..]
        .iter()
        .position(|b| *b != b' ' && *b != b'\t')
        .map_or(bytes.len(), |at| from + at)
}

/// The whole line from `line_start` with its line break, when the element
/// is `alone` on it and nothing but the line break follows `end`.
fn own_line(bytes: &[u8], line_start: usize, alone: bool, end: usize) -> Option<Range<usize>> {
    if !alone {
        return None;
    }
    match &bytes[end..] {
        [] => Some(line_start..end),
        [b'\n', ..] => Some(line_start..end + 1),
        [b'\r', b'\n', ..] => Some(line_start..end + 2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    fn set(text: &str, path: &str, value: Value) -> String {
        let mut doc = TomlDocument::parse(text).unwrap();
        doc.set(&key(path), &value, true).unwrap();
        doc.text().to_owned()
    }

    fn removed(text: &str, path: &str) -> String {
        let mut doc = TomlDocument::parse(text).unwrap();
        doc.remove(&key(path)).unwrap();
        doc.text().to_owned()
    }

    #[test]
    fn places_new_keys_by_the_table_that_holds_them() {
        let text = "a = 1\n\n[t]\nx = 1\n\n[u]\n";
        assert_eq!(
            set(text, "t.y", Value::Int(2)),
            "a = 1\n\n[t]\nx = 1\ny = 2\n\n[u]\n"
        );
        assert_eq!(
            set(text, "b", Value::Int(2)),
            "a = 1\nb = 2\n\n[t]\nx = 1\n\n[u]\n"
        );
        assert_eq!(
            set(text, "u.z", Value::Int(3)),
            "a = 1\n\n[t]\nx = 1\n\n[u]\nz = 3\n"
        );
        assert_eq!(
            set(text, "n.m", Value::Bool(true)),
            "a = 1\n\n[t]\nx = 1\n\n[u]\n\n[n]\nm = true\n"
        );
        assert_eq!(
            set("[t]\nx = 1", "t.d.e", Value::Int(4)),
            "[t]\nx = 1\nd.e = 4"
        );
    }

    #[test]
    fn extends_dotted_key_tables_with_dotted_keys() {
        let text = "[tool]\n  build.target = \"x\"\n  other = 1\n";
        assert_eq!(
            set(text, "tool.build.jobs", Value::Int(4)),
            "[tool]\n  build.target = \"x\"\n  build.jobs = 4\n  other = 1\n"
        );
    }

    #[test]
    fn adds_root_keys_before_the_first_header() {
        assert_eq!(set("# c\n[a]\n", "k", Value::Int(1)), "k = 1\n\n# c\n[a]\n");
        assert_eq!(set("", "k", Value::Int(1)), "k = 1\n");
    }

    #[test]
    fn implicit_tables_get_a_header() {
        let text = "[a.b]\nx = 1\n";
        assert_eq!(
            set(text, "a.y", Value::Int(2)),
            "[a.b]\nx = 1\n\n[a]\ny = 2\n"
        );
    }

    #[test]
    fn edits_arrays_and_inline_tables_in_place() {
        assert_eq!(
            set("a = [1, 2]\n", "a[+]", Value::Int(3)),
            "a = [1, 2, 3]\n"
        );
        assert_eq!(set("a = []\n", "a[+]", Value::Int(3)), "a = [3]\n");
        assert_eq!(
            set("a = [\n  1,\n  2,\n]\n", "a[2]", Value::Int(3)),
            "a = [\n  1,\n  2,\n  3,\n]\n"
        );
        assert_eq!(
            set("p = { x = 1 }\n", "p.y", Value::Int(2)),
            "p = { x = 1, y = 2 }\n"
        );
        assert_eq!(set("p = {}\n", "p.y", Value::Int(2)), "p = { y = 2 }\n");
        assert_eq!(
            set(
                "a = [1, 2, 3]\n",
                "a",
                Value::List(vec![Value::Int(1), Value::Int(9)])
            ),
            "a = [1, 9]\n"
        );
    }

    #[test]
    fn appends_to_arrays_of_tables_with_a_new_header() {
        let text = "[[bin]]\nname = \"a\"\n[bin.cfg]\nx = 1\n\n[other]\n";
        let mut fields = Fields::new();
        fields.insert("name".into(), Value::Text("b".into()));
        assert_eq!(
            set(text, "bin[+]", Value::Map(fields)),
            "[[bin]]\nname = \"a\"\n[bin.cfg]\nx = 1\n\n[[bin]]\nname = \"b\"\n\n[other]\n"
        );
        assert_eq!(
            set(text, "bin[0].name", Value::Text("z".into())),
            "[[bin]]\nname = \"z\"\n[bin.cfg]\nx = 1\n\n[other]\n"
        );
    }

    #[test]
    fn removes_elements_of_multi_line_arrays_within_their_lines() {
        let text = "a = [\n  1,\n  2,\n  3\n]\n";
        assert_eq!(removed(text, "a[0]"), "a = [\n  2,\n  3\n]\n");
        assert_eq!(removed(text, "a[2]"), "a = [\n  1,\n  2,\n]\n");
        assert_eq!(removed("b = [1,\n  2]\n", "b[0]"), "b = [\n  2]\n");
        assert_eq!(removed("b = [1,\n  2]\n", "b[1]"), "b = [1,\n  ]\n");
        assert_eq!(removed("b = [1\n, 2]\n", "b[1]"), "b = [1\n]\n");
        assert_eq!(removed("c = { x = 1, }\n", "c.x"), "c = { }\n");
        let error = TomlDocument::parse("d = [\n  1 # one\n  , 2\n]\n")
            .unwrap()
            .remove(&key("d[0]"))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnsupportedConstruct);
    }

    #[test]
    fn removes_pairs_tables_and_elements() {
        let text = "a = 1\n# about b\nb = 2\n\n[t]\nx = 1\n[t.sub]\ny = 2\n\n[u]\nz = [1, 2, 3]\n";
        assert_eq!(
            removed(text, "b"),
            "a = 1\n\n[t]\nx = 1\n[t.sub]\ny = 2\n\n[u]\nz = [1, 2, 3]\n"
        );
        assert_eq!(
            removed(text, "t"),
            "a = 1\n# about b\nb = 2\n\n[u]\nz = [1, 2, 3]\n"
        );
        assert_eq!(removed(text, "u.z[1]"), text.replace("[1, 2, 3]", "[1, 3]"));
        assert_eq!(removed(text, "u.z[2]"), text.replace("[1, 2, 3]", "[1, 2]"));
        assert_eq!(removed("p = { x = 1, y = 2 }\n", "p.x"), "p = { y = 2 }\n");
        assert!(
            TomlDocument::parse(text)
                .unwrap()
                .remove(&KeyPath::root())
                .is_err()
        );
    }

    #[test]
    fn same_value_changes_no_byte() {
        let text = "n = 0x1F\nf = 1.50\ns = '''\nx'''\nd = 1979-05-27\n";
        let mut doc = TomlDocument::parse(text).unwrap();
        let value = doc.to_value();
        let Value::Map(fields) = value else {
            panic!("map expected")
        };
        for (name, value) in fields {
            doc.set(&key(&name), &value, false).unwrap();
        }
        assert_eq!(doc.text(), text);
    }

    #[test]
    fn rejects_changes_that_toml_cannot_hold() {
        let mut doc = TomlDocument::parse("[t]\nx = 1\n[[a]]\n").unwrap();
        let null = doc.set(&key("t.x"), &Value::Null, false).unwrap_err();
        assert_eq!(null.found.as_deref(), Some("toml.null"));
        let table = doc.set(&key("t"), &Value::Int(1), false).unwrap_err();
        assert_eq!(table.found.as_deref(), Some(TABLE_TO_VALUE));
        let empty = doc.set(&key("a"), &Value::List(vec![]), false).unwrap_err();
        assert_eq!(empty.found.as_deref(), Some(EMPTY_TABLES));
        let through = doc.set(&key("t.x.y"), &Value::Int(1), true).unwrap_err();
        assert_eq!(through.code(), ErrorCode::KeyConflict);
        let dotted = TomlDocument::parse("p = { a.b = 1 }\n")
            .unwrap()
            .remove(&key("p.a.b"))
            .unwrap_err();
        assert_eq!(dotted.found.as_deref(), Some(INLINE_DOTTED));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
