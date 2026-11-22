// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! INI documents and their dialects.
//!
//! The parser keeps every byte of the document: comments, blank lines,
//! spacing around separators, quoting and continuation lines. Keys are
//! addressed as `section.key`, `["section with.dot"].key` or, for keys before
//! the first section, `key`. Git configuration adds the subsection:
//! `remote.origin.url`. A key written more than once is a list in the
//! `systemd` and `gitconfig` profiles and for PHP `key[]`; in the other
//! profiles each occurrence is selected by index (`section.key[1]`).
//!
//! Changes replace only the bytes of the affected values or lines; the
//! result is parsed again and compared with the expected content before it
//! is accepted.

mod edit;
mod format;
mod parse;
mod profile;
pub(crate) mod support;
mod value;

pub use format::IniFormat;

/// Registration function of the format `ini`.
#[must_use]
pub fn formats() -> Vec<Box<dyn crate::registry::DocFormat>> {
    vec![Box::new(IniFormat::new())]
}

use std::ops::Range;

use nurt_contracts::error::Error;
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocItem;
use nurt_contracts::value::{Fields, Value};

pub use profile::{IniProfile, UnknownIniProfile};

use parse::{Parsed, parse};
use profile::Repeat;
use support::errors::parse_error;
use support::text::LineIndex;
use support::tree::{Hit, Map, Node, resolve};

/// File name extensions read as INI documents of the generic profile unless
/// the name selects another profile.
pub const EXTENSIONS: [&str; 3] = ["ini", "cfg", "conf"];

/// Byte order mark, accepted before the first line.
const BOM: char = '\u{feff}';

/// Length of the byte order mark in UTF-8.
const BOM_LEN: usize = BOM.len_utf8();

/// Parsed INI document that keeps its original text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::ini::{IniDocument, IniProfile};
///
/// let mut doc = IniDocument::parse("[server]\nport = 80 \n", IniProfile::Generic)?;
/// let key = KeyPath::parse("server.port")?;
/// assert_eq!(doc.get(&key)?[0].value, Value::Text("80".into()));
/// doc.set(&key, &Value::Int(8080), false)?;
/// assert_eq!(doc.text(), "[server]\nport = 8080 \n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct IniDocument {
    text: String,
    profile: IniProfile,
    parsed: Parsed,
    tree: Node,
}

impl IniDocument {
    /// Parses `text` with the rules of `profile`.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` with the line and column of the first malformed line.
    pub fn parse(text: &str, profile: IniProfile) -> Result<Self, Error> {
        let parsed = parse(text, profile)?;
        let tree = build_tree(text, &parsed, profile)?;
        Ok(Self {
            text: text.to_owned(),
            profile,
            parsed,
            tree,
        })
    }

    /// Text of the document, including all changes made so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Profile the document was parsed with.
    pub fn profile(&self) -> IniProfile {
        self.profile
    }

    /// Values selected by `key`, with their positions and comments.
    ///
    /// Sections are maps; a key written more than once in a list profile is
    /// a list; a key without separator is `null`, or `true` in Git
    /// configuration.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing key, `E_KEY_CONFLICT` for a repeated
    /// key without index or a path through a value.
    pub fn get(&self, key: &KeyPath) -> Result<Vec<DocItem>, Error> {
        let value_of = |node: &Node| self.node_value(node);
        let hits = resolve(&self.tree, key, &value_of)?;
        let index = LineIndex::new(&self.text);
        hits.iter().map(|hit| self.item(hit, &index)).collect()
    }

    /// Content of the document as a map of sections and global keys.
    pub fn to_value(&self) -> Value {
        self.node_value(&self.tree)
    }

    /// Sets the value at `key`; `create` allows new keys and sections.
    ///
    /// Returns the number of values changed. A list value replaces all
    /// occurrences of a repeated key in a list profile; a map value replaces
    /// the keys of a section.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing key without `create`,
    /// `E_KEY_CONFLICT` for paths that do not fit the document and
    /// `E_UNSUPPORTED_CONSTRUCT` for values the profile cannot hold on one
    /// line or for nesting deeper than sections.
    pub fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize, Error> {
        let plan = edit::plan_set(self, key, value, create)?;
        self.commit(plan)
    }

    /// Removes the keys or sections selected by `key`, with their comment
    /// lines.
    ///
    /// Returns the number of values removed.
    ///
    /// # Errors
    ///
    /// The errors of [`IniDocument::get`]; `E_KEY_CONFLICT` for the document
    /// root.
    pub fn remove(&mut self, key: &KeyPath) -> Result<usize, Error> {
        let plan = edit::plan_remove(self, key)?;
        self.commit(plan)
    }

    fn commit(&mut self, plan: edit::Plan) -> Result<usize, Error> {
        let count = plan.count;
        if let Some(text) = edit::apply_and_verify(self, &plan)? {
            *self = Self::parse(&text, self.profile)?;
        }
        Ok(count)
    }

    fn node_value(&self, node: &Node) -> Value {
        match node {
            Node::Leaf(id) => self.entry_value(*id),
            Node::Map(map) => Value::Map(
                map.members
                    .iter()
                    .map(|(name, node)| (name.clone(), self.node_value(node)))
                    .collect::<Fields>(),
            ),
            Node::List(elements) => {
                Value::List(elements.iter().map(|n| self.node_value(n)).collect())
            }
            Node::Repeated(elements) => elements
                .last()
                .map_or(Value::Null, |last| self.node_value(last)),
        }
    }

    fn entry_value(&self, id: usize) -> Value {
        match &self.parsed.entries[id].value {
            Some(text) => Value::Text(text.clone()),
            None if self.profile == IniProfile::Gitconfig => Value::Bool(true),
            None => Value::Null,
        }
    }

    fn item(&self, hit: &Hit<'_>, index: &LineIndex) -> Result<DocItem, Error> {
        let key = KeyPath::new(None, hit.path.clone())
            .map_err(|error| parse_error("ini", &error.to_string(), &self.text, 0))?;
        let (range, comment) = if hit.path.is_empty() {
            (0..self.text.len(), None)
        } else {
            self.node_extent(hit.node)
        };
        Ok(DocItem {
            key,
            value: self.node_value(hit.node),
            span: Some(index.span(&self.text, range)),
            comment,
            document: None,
            anchor: None,
            alias_of: None,
            tag: None,
        })
    }

    /// Byte range and comment of a node: the raw value of an entry, or the
    /// lines of the sections and entries that form a map or list.
    fn node_extent(&self, node: &Node) -> (Range<usize>, Option<String>) {
        match node {
            Node::Leaf(id) => {
                let entry = &self.parsed.entries[*id];
                (entry.value_range.clone(), entry.comment.clone())
            }
            Node::Map(map) => {
                let first = map.ids.first().map(|&s| &self.parsed.sections[s]);
                let comment = first.and_then(|s| s.comment.clone());
                (self.lines_range(node), comment)
            }
            Node::List(_) | Node::Repeated(_) => (self.lines_range(node), None),
        }
    }

    fn lines_range(&self, node: &Node) -> Range<usize> {
        let mut lines = Vec::new();
        collect_lines(&self.parsed, node, &mut lines);
        let start = lines
            .iter()
            .min()
            .map_or(0, |&l| self.parsed.lines[l].start);
        let end = lines
            .iter()
            .max()
            .map_or(0, |&l| self.parsed.lines[l].content_end);
        start..end
    }
}

/// Line indices of the headers and entries that form a node.
fn collect_lines(parsed: &Parsed, node: &Node, out: &mut Vec<usize>) {
    match node {
        Node::Leaf(id) => {
            let entry = &parsed.entries[*id];
            out.extend(entry.first_line..=entry.last_line);
        }
        Node::Map(map) => {
            out.extend(
                map.ids
                    .iter()
                    .filter_map(|&s| parsed.sections[s].header_line),
            );
            for (_, child) in &map.members {
                collect_lines(parsed, child, out);
            }
        }
        Node::List(elements) | Node::Repeated(elements) => {
            for element in elements {
                collect_lines(parsed, element, out);
            }
        }
    }
}

/// Builds the key tree: sections are maps under the root, Git subsections
/// maps under their section, entries leaves.
fn build_tree(text: &str, parsed: &Parsed, profile: IniProfile) -> Result<Node, Error> {
    let rules = profile.rules();
    let mut root = Map::new(rules.fold_section_names);
    for (id, section) in parsed.sections.iter().enumerate() {
        let mut map = &mut root;
        for name in &section.path {
            let node = map.entry(name, || Node::Map(Map::new(rules.fold_key_names)));
            map = match node {
                Node::Map(child) => child,
                _ => {
                    return Err(name_clash(text, parsed, section.header_line, name));
                }
            };
        }
        if section.header_line.is_some() {
            map.ids.push(id);
        }
        for &entry_id in &section.entries {
            add_entry(map, parsed, entry_id, profile).map_err(|name| {
                name_clash(
                    text,
                    parsed,
                    Some(parsed.entries[entry_id].first_line),
                    &name,
                )
            })?;
        }
    }
    Ok(Node::Map(root))
}

fn name_clash(text: &str, parsed: &Parsed, line: Option<usize>, name: &str) -> Error {
    let offset = line.map_or(0, |l| parsed.lines[l].start);
    parse_error(
        "ini",
        &format!("`{name}` names both a key and a section; rename one of them"),
        text,
        offset,
    )
}

fn add_entry(map: &mut Map, parsed: &Parsed, id: usize, profile: IniProfile) -> Result<(), String> {
    let entry = &parsed.entries[id];
    let rules = profile.rules();
    let name = if rules.fold_key_names {
        entry.name.to_ascii_lowercase()
    } else {
        entry.name.clone()
    };
    let list = rules.repeat == Repeat::List || entry.list_suffix;
    match map.position(&name) {
        None => {
            let node = if entry.list_suffix {
                Node::List(vec![Node::Leaf(id)])
            } else {
                Node::Leaf(id)
            };
            map.members.push((name, node));
            Ok(())
        }
        Some(position) => {
            let slot = &mut map.members[position].1;
            match slot {
                Node::List(elements) | Node::Repeated(elements) => {
                    elements.push(Node::Leaf(id));
                    Ok(())
                }
                Node::Leaf(previous) => {
                    let elements = vec![Node::Leaf(*previous), Node::Leaf(id)];
                    *slot = if list {
                        Node::List(elements)
                    } else {
                        Node::Repeated(elements)
                    };
                    Ok(())
                }
                Node::Map(_) => Err(name),
            }
        }
    }
}
