// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! JSON and JSONC documents.
//!
//! JSON follows RFC 8259 strictly: comments, trailing commas and the
//! extensions of JSON5 are syntax errors. JSONC adds `//` and `/* */`
//! comments and trailing commas, as read by Visual Studio Code and the
//! TypeScript compiler.
//!
//! Edits replace byte spans of the original text. Unchanged values keep
//! their bytes, including number lexemes such as `1.10` or `1e400` and
//! string escapes such as `é` or `\/`. New content copies the style of
//! its neighbours: indentation, the separator after member names, spacing
//! after commas, line endings and, in JSONC, trailing commas.
//!
//! An object with a repeated member name reads as its last occurrence;
//! changing the name changes the last occurrence and removing it removes
//! every occurrence.

mod editor;
mod layout;
mod lexeme;
mod reference;
mod render;
mod syntax;

use std::path::Path;

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Location};
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::value::Value;

use editor::{Change, Editor};
use layout::Layout;
use syntax::{NodeId, NodeKind, Tree};

/// Variant of the JSON syntax a document is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// RFC 8259 JSON.
    Json,
    /// JSON with comments and trailing commas.
    Jsonc,
}

/// File names read as JSONC although their extension is `.json`.
const JSONC_FILE_NAMES: [&str; 6] = [
    "tsconfig.json",
    "jsconfig.json",
    "devcontainer.json",
    ".devcontainer.json",
    ".eslintrc.json",
    ".babelrc.json",
];

/// Directory whose `.json` files are JSONC (editor settings).
const JSONC_DIRECTORY: &str = ".vscode";

impl Dialect {
    /// Format identifier: `json` or `jsonc`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::json::Dialect;
    ///
    /// assert_eq!(Dialect::Jsonc.id(), "jsonc");
    /// ```
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Jsonc => "jsonc",
        }
    }

    /// Dialect of a file from its name: `.jsonc` files, and `.json` files
    /// that tools read with comments (`tsconfig.json`, `tsconfig.*.json`,
    /// `jsconfig.json`, `devcontainer.json`, `.eslintrc.json`, files in
    /// `.vscode`), are JSONC; other `.json` files are JSON. Other names,
    /// `.json5` included, have no dialect.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// use nurt_doc::formats::json::Dialect;
    ///
    /// assert_eq!(Dialect::detect(Path::new("package.json")), Some(Dialect::Json));
    /// assert_eq!(Dialect::detect(Path::new(".vscode/settings.json")), Some(Dialect::Jsonc));
    /// assert_eq!(Dialect::detect(Path::new("config.json5")), None);
    /// ```
    #[must_use]
    pub fn detect(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("jsonc") {
            return Some(Self::Jsonc);
        }
        if !extension.eq_ignore_ascii_case("json") {
            return None;
        }
        let lower = name.to_ascii_lowercase();
        let in_editor_settings = path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == JSONC_DIRECTORY);
        let tool_config = JSONC_FILE_NAMES.contains(&lower.as_str())
            || lower.starts_with("tsconfig.")
            || lower.starts_with("jsconfig.");
        Some(if in_editor_settings || tool_config { Self::Jsonc } else { Self::Json })
    }
}

/// Parsed JSON or JSONC document with its text.
#[derive(Debug, Clone)]
pub struct JsonDocument {
    text: String,
    dialect: Dialect,
    tree: Tree,
}

/// Node addressed by a key path, or the place where it would be created.
enum Target {
    /// Existing value; `duplicate` is set when a member name on the way
    /// occurs more than once in its object.
    Node { id: NodeId, duplicate: bool },
    /// The segment at `segment` names nothing inside `container`.
    Missing { container: NodeId, segment: usize },
    /// Position after the last element of an array.
    Append(NodeId),
}

/// One step of a key path from a node.
enum Step {
    Child { id: NodeId, duplicate: bool },
    Missing,
    Append,
}

/// Changes to the text made by one operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) changes: Vec<Change>,
    /// A repeated member name was involved.
    pub(crate) duplicate_key: bool,
    /// Intermediate keys were created.
    pub(crate) keys_created: bool,
}

impl JsonDocument {
    /// Parses UTF-8 text without a byte order mark.
    ///
    /// # Errors
    ///
    /// `E_DOC_PARSE` with the line, column and byte offset of the first
    /// syntax error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::json::{Dialect, JsonDocument};
    ///
    /// assert!(JsonDocument::parse("{\"a\": 1}", Dialect::Json).is_ok());
    /// assert!(JsonDocument::parse("{\"a\": 1, // note\n}", Dialect::Json).is_err());
    /// assert!(JsonDocument::parse("{\"a\": 1, // note\n}", Dialect::Jsonc).is_ok());
    /// ```
    pub fn parse(text: impl Into<String>, dialect: Dialect) -> Result<Self, Error> {
        let text = text.into();
        let tree = syntax::parse(&text, dialect).map_err(|error| parse_error(&text, &error))?;
        Ok(Self { text, dialect, tree })
    }

    /// Current text of the document.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Dialect the document is read in.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// The whole document as a value; for repeated member names the last
    /// occurrence wins.
    #[must_use]
    pub fn to_value(&self) -> Value {
        self.value_of(self.tree.root)
    }

    fn value_of(&self, id: NodeId) -> Value {
        let node = &self.tree.nodes[id];
        match &node.kind {
            NodeKind::Null => Value::Null,
            NodeKind::Bool(flag) => Value::Bool(*flag),
            NodeKind::Number => lexeme::number_value(&self.text[node.span.start..node.span.end]),
            NodeKind::String(text) => Value::Text(text.clone()),
            NodeKind::Array(entries) => Value::List(entries.iter().map(|e| self.value_of(e.value)).collect()),
            NodeKind::Object(entries) => {
                let mut fields = IndexMap::with_capacity(entries.len());
                for entry in entries {
                    if let Some(key) = &entry.key {
                        fields.insert(key.text.clone(), self.value_of(entry.value));
                    }
                }
                Value::Map(fields)
            }
        }
    }

    /// Plans setting the value at `key`.
    ///
    /// Setting the current value changes nothing. A missing final member
    /// or element is added at the end of its object or array, and with
    /// `create` missing intermediate members are added as well.
    pub(crate) fn plan_set(&self, key: &KeyPath, value: &Value, create: bool) -> Result<Outcome, Error> {
        let editor = Editor::new(Layout::new(&self.text, &self.tree));
        match self.locate(key)? {
            Target::Node { id, duplicate } => {
                let changes = if self.value_of(id) == *value { Vec::new() } else { editor.replace(id, value)? };
                Ok(Outcome {
                    changes,
                    duplicate_key: duplicate,
                    keys_created: false,
                })
            }
            Target::Append(array) => Ok(Outcome {
                changes: editor.append(array, None, value)?,
                ..Outcome::default()
            }),
            Target::Missing { container, segment } => {
                let rest = &key.segments()[segment..];
                let explicit_position = rest.len() == 1 && matches!(rest[0], KeySegment::Index(_));
                if !create && !explicit_position {
                    return Err(not_found(key, "add create: true to create the missing keys"));
                }
                let (name, nested) = self.nested_value(container, rest, value)?;
                Ok(Outcome {
                    changes: editor.append(container, name.as_deref(), &nested)?,
                    duplicate_key: false,
                    keys_created: rest.len() > 1,
                })
            }
        }
    }

    /// Plans removing the value at `key`. A repeated member name loses all
    /// its occurrences; the change then covers the region between the first
    /// and the last removed byte.
    pub(crate) fn plan_remove(&self, key: &KeyPath) -> Result<Outcome, Error> {
        let first = self.plan_remove_once(key)?;
        if !first.duplicate_key {
            return Ok(first);
        }
        let mut current = self.clone();
        let mut outcome = first;
        while outcome.duplicate_key {
            let text = editor::apply(&current.text, &outcome.changes)?;
            current = Self::parse(text, self.dialect)?;
            outcome = current.plan_remove_once(key)?;
        }
        let text = editor::apply(&current.text, &outcome.changes)?;
        Ok(Outcome {
            changes: vec![editor::difference(&self.text, &text)],
            duplicate_key: true,
            keys_created: false,
        })
    }

    /// Comment attached to the value `id`: the comment lines directly above
    /// its entry, or else a comment after it on the same line, without
    /// comment markers.
    pub(crate) fn comment_of(&self, id: NodeId) -> Option<String> {
        let layout = Layout::new(&self.text, &self.tree);
        let entry = self.tree.nodes[id]
            .parent
            .and_then(|parent| self.tree.nodes[parent].entries())
            .and_then(|entries| entries.iter().find(|entry| entry.value == id));
        let start = entry.map_or(self.tree.nodes[id].span.start, |entry| entry.start(&self.tree));
        let block = layout.leading_comments(start).or_else(|| entry.and_then(|e| layout.trailing_comment(e)))?;
        let texts: Vec<&str> = layout
            .comments_in(block)
            .map(|span| layout::comment_text(&self.text[span.start..span.end]))
            .collect();
        Some(texts.join("\n"))
    }

    /// Plans removing one value at `key`; for a repeated member name the
    /// last occurrence.
    fn plan_remove_once(&self, key: &KeyPath) -> Result<Outcome, Error> {
        let Target::Node { id, duplicate } = self.locate(key)? else {
            return Err(not_found(key, "read the document to list its keys"));
        };
        let Some(container) = self.tree.nodes[id].parent else {
            return Err(Error::new(
                ErrorCode::KeyConflict,
                "the document root cannot be removed; set it to a new value instead",
            ));
        };
        let entries = self.tree.nodes[container].entries().unwrap_or_default();
        let index = entries.iter().position(|entry| entry.value == id).unwrap_or_default();
        let editor = Editor::new(Layout::new(&self.text, &self.tree));
        Ok(Outcome {
            changes: editor.remove(container, index),
            duplicate_key: duplicate,
            keys_created: false,
        })
    }

    /// Name of the entry to add to `container` and its value, with the
    /// remaining segments turned into nested maps and lists.
    fn nested_value(
        &self,
        container: NodeId,
        rest: &[KeySegment],
        value: &Value,
    ) -> Result<(Option<String>, Value), Error> {
        let mut nested = value.clone();
        for segment in rest[1..].iter().rev() {
            nested = match segment {
                KeySegment::Key(name) => Value::Map(IndexMap::from([(name.clone(), nested)])),
                KeySegment::Index(0) | KeySegment::Append => Value::List(vec![nested]),
                _ => return Err(cannot_create()),
            };
        }
        let is_object = matches!(self.tree.nodes[container].kind, NodeKind::Object(_));
        match &rest[0] {
            KeySegment::Key(name) if is_object => Ok((Some(name.clone()), nested)),
            KeySegment::Key(_) | KeySegment::Index(_) => Ok((None, nested)),
            _ => Err(cannot_create()),
        }
    }

    /// Follows a concrete key path (keys, indices and a final `[+]`).
    fn locate(&self, key: &KeyPath) -> Result<Target, Error> {
        if matches!(key.document(), Some(DocSelector::Index(index)) if index > 0) {
            return Err(not_found(key, "a JSON file holds one document; remove the document selector"));
        }
        let mut id = self.tree.root;
        let mut duplicate = false;
        for (position, segment) in key.segments().iter().enumerate() {
            match self.step(id, segment, key)? {
                Step::Child { id: child, duplicate: repeated } => {
                    id = child;
                    duplicate |= repeated;
                }
                Step::Missing => return Ok(Target::Missing { container: id, segment: position }),
                Step::Append => return Ok(Target::Append(id)),
            }
        }
        Ok(Target::Node { id, duplicate })
    }

    fn step(&self, id: NodeId, segment: &KeySegment, key: &KeyPath) -> Result<Step, Error> {
        let node = &self.tree.nodes[id];
        match (&node.kind, segment) {
            (NodeKind::Object(entries), KeySegment::Key(name)) => {
                let mut matches = entries.iter().filter(|e| e.key.as_ref().is_some_and(|k| &k.text == name));
                let Some(last) = matches.next_back() else {
                    return Ok(Step::Missing);
                };
                Ok(Step::Child {
                    id: last.value,
                    duplicate: matches.next().is_some(),
                })
            }
            (NodeKind::Array(entries), KeySegment::Key(name)) => match name.parse::<i64>() {
                Ok(index) if !name.starts_with(['-', '+']) => index_step(entries.len(), index, key)
                    .map(|step| self.element_step(step, entries)),
                _ => Err(kind_conflict(key, "a list", "use an index such as [0]")),
            },
            (NodeKind::Array(entries), KeySegment::Index(index)) => {
                index_step(entries.len(), *index, key).map(|step| self.element_step(step, entries))
            }
            (NodeKind::Array(_), KeySegment::Append) => Ok(Step::Append),
            (NodeKind::Object(_), KeySegment::Index(_) | KeySegment::Append) => {
                Err(kind_conflict(key, "a map", "use a member name"))
            }
            (NodeKind::Object(_) | NodeKind::Array(_), _) => Err(Error::new(
                ErrorCode::KeyConflict,
                format!("key path '{key}' does not address a single value; use member names and indices"),
            )),
            _ => Err(kind_conflict(key, "a scalar", "set the value itself or replace it with a map")),
        }
    }

    fn element_step(&self, step: Option<usize>, entries: &[syntax::Entry]) -> Step {
        step.map_or(Step::Missing, |index| Step::Child {
            id: entries[index].value,
            duplicate: false,
        })
    }
}

/// Position of an index in a list of `len` elements: `Some` for an
/// element, `None` for the position after the last one.
fn index_step(len: usize, index: i64, key: &KeyPath) -> Result<Option<usize>, Error> {
    let len_signed = i64::try_from(len).unwrap_or(i64::MAX);
    let resolved = if index < 0 { len_signed + index } else { index };
    match usize::try_from(resolved) {
        Ok(position) if position < len => Ok(Some(position)),
        Ok(position) if position == len && index >= 0 => Ok(None),
        _ => Err(Error::new(
            ErrorCode::KeyConflict,
            format!("index {index} in '{key}' is outside a list of {len} elements; use [+] to append"),
        )),
    }
}

fn kind_conflict(key: &KeyPath, kind: &str, fix: &str) -> Error {
    Error::new(ErrorCode::KeyConflict, format!("key path '{key}' passes through {kind}; {fix}"))
}

fn not_found(key: &KeyPath, fix: &str) -> Error {
    Error::new(ErrorCode::KeyNotFound, format!("key path '{key}' does not exist; {fix}"))
}

fn cannot_create() -> Error {
    Error::new(
        ErrorCode::KeyConflict,
        "only member names, [0] and [+] can create missing values; set the existing parent instead",
    )
}

/// `E_DOC_PARSE` with the position of a syntax error.
fn parse_error(text: &str, error: &syntax::SyntaxError) -> Error {
    let offset = error.offset.min(text.len());
    let before = &text.as_bytes()[..offset];
    let line_start = before.iter().rposition(|&b| b == b'\n').map_or(0, |pos| pos + 1);
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let column = String::from_utf8_lossy(&before[line_start..]).chars().count() + 1;
    let to_u64 = |number: usize| u64::try_from(number).unwrap_or(u64::MAX);
    Error::new(ErrorCode::DocParse, error.message.clone()).with_location(Location {
        path: String::new(),
        line: Some(to_u64(line)),
        column: Some(to_u64(column)),
        end_line: None,
        end_column: None,
        byte: Some(to_u64(offset)),
    })
}

#[cfg(test)]
mod corpus_tests;
#[cfg(test)]
mod tests;
