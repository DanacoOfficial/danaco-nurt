// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::json
// Responsibility:   JSON and JSONC documents.
// Dependencies:     editor, handle, indexmap, layout, nurt_contracts, syntax
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
//!
//! ```
//! use nurt_contracts::services::DocFormatId;
//! use nurt_doc::formats::json;
//! use nurt_doc::registry::Registry;
//!
//! let registry = Registry::with(json::formats());
//! let detected = registry.detect(Some(std::path::Path::new("tsconfig.json")), b"{}");
//! assert_eq!(detected, Some(DocFormatId::JSONC));
//! ```

mod editor;
mod handle;
mod layout;
mod lexeme;
mod reference;
mod render;
mod syntax;

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use indexmap::IndexMap;
use nurt_contracts::error::{Error, ErrorCode, Location};
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

pub use handle::{JsonFormat, JsonHandle};

use crate::model::{doc_parse, key_conflict, key_not_found};
use crate::registry::{DocFormat, matches_file};
use editor::{Change, Editor};
use layout::Layout;
use syntax::{NodeId, NodeKind, Span, Tree};

/// The formats `json` and `jsonc`, in this order, for registration in a
/// [`Registry`](crate::registry::Registry).
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::json;
///
/// let ids: Vec<String> = json::formats().iter().map(|format| format.id().to_string()).collect();
/// assert_eq!(ids, ["json", "jsonc"]);
/// ```
#[must_use]
pub fn formats() -> Vec<Box<dyn DocFormat>> {
    vec![
        Box::new(JsonFormat::new(Dialect::Json)),
        Box::new(JsonFormat::new(Dialect::Jsonc)),
    ]
}

/// Variant of the JSON syntax a document is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// RFC 8259 JSON.
    Json,
    /// JSON with comments and trailing commas.
    Jsonc,
}

/// Names of `.json` files that tools read as JSONC, in the pattern form of
/// [`DocFormat::file_names`]: a final `*` matches every name with the
/// prefix.
const JSONC_FILE_NAMES: [&str; 8] = [
    "tsconfig.*",
    "jsconfig.*",
    "devcontainer.json",
    ".devcontainer.json",
    ".eslintrc.json",
    ".babelrc.json",
    ".vscode/*.json",
    ".devcontainer/*.json",
];

impl Dialect {
    /// Format identifier: `json` or `jsonc`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::services::DocFormatId;
    /// use nurt_doc::formats::json::Dialect;
    ///
    /// assert_eq!(Dialect::Jsonc.format_id(), DocFormatId::JSONC);
    /// ```
    #[must_use]
    pub const fn format_id(self) -> DocFormatId {
        match self {
            Self::Json => DocFormatId::JSON,
            Self::Jsonc => DocFormatId::JSONC,
        }
    }

    /// Dialect of a file from its path: `.jsonc` files, and `.json` files
    /// that tools read with comments (`tsconfig.json`, `tsconfig.*.json`,
    /// `jsconfig.json`, `devcontainer.json`, `.eslintrc.json`,
    /// `.babelrc.json`, `.json` files in `.vscode` and `.devcontainer`), are JSONC; other `.json` files
    /// are JSON. Other names, `.json5` included, have no dialect.
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
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("jsonc") {
            return Some(Self::Jsonc);
        }
        if !extension.eq_ignore_ascii_case("json") {
            return None;
        }
        Some(if matches_file(&JsonFormat::new(Self::Jsonc), path) {
            Self::Jsonc
        } else {
            Self::Json
        })
    }
}

/// Parsed JSON or JSONC document with its text.
#[derive(Debug, Clone)]
pub struct JsonDocument {
    text: String,
    dialect: Dialect,
    tree: Tree,
    /// Positions of the members of large objects by name, built when a key
    /// path first passes through such an object.
    members: OnceLock<MemberIndex>,
}

/// Positions of the entries of an object by member name, in order, for each
/// object of at least [`INDEXED_MEMBERS`] entries.
type MemberIndex = HashMap<NodeId, HashMap<String, Vec<usize>>>;

/// Smallest number of entries of an object whose member names are indexed;
/// smaller objects are searched directly.
const INDEXED_MEMBERS: usize = 32;

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
    /// Spans of the original text the changes may alter: the replaced
    /// value, or the object or array that gains or loses an entry.
    pub(crate) targets: Vec<Span>,
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
    /// syntax error in its location and details.
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
        let tree =
            syntax::parse(&text, dialect).map_err(|error| parse_error(&text, dialect, &error))?;
        Ok(Self {
            text,
            dialect,
            tree,
            members: OnceLock::new(),
        })
    }

    /// Positions of the entries named `name` in the object `id` when the
    /// object is large; `None` when it is searched directly.
    fn indexed_members(&self, id: NodeId, name: &str) -> Option<&[usize]> {
        let index = self.members.get_or_init(|| {
            let mut index = MemberIndex::new();
            for (id, node) in self.tree.nodes.iter().enumerate() {
                let NodeKind::Object(entries) = &node.kind else {
                    continue;
                };
                if entries.len() < INDEXED_MEMBERS {
                    continue;
                }
                let names = index.entry(id).or_default();
                for (position, entry) in entries.iter().enumerate() {
                    if let Some(key) = &entry.key {
                        names.entry(key.text.clone()).or_default().push(position);
                    }
                }
            }
            index
        });
        let names = index.get(&id)?;
        Some(names.get(name).map_or(&[][..], Vec::as_slice))
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
            NodeKind::Array(entries) => {
                Value::List(entries.iter().map(|e| self.value_of(e.value)).collect())
            }
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
    /// Setting the current value changes nothing. With `create` a missing
    /// member is added at the end of its object, together with missing
    /// intermediate members; `[+]` and an index equal to the length of a
    /// list append without it.
    pub(crate) fn plan_set(
        &self,
        key: &KeyPath,
        value: &Value,
        create: bool,
    ) -> Result<Outcome, Error> {
        let editor = Editor::new(Layout::new(&self.text, &self.tree));
        match self.locate(key)? {
            Target::Node { id, duplicate } => {
                let changes = if self.value_of(id) == *value {
                    Vec::new()
                } else {
                    editor.replace(id, value)?
                };
                Ok(Outcome {
                    changes,
                    targets: vec![self.tree.nodes[id].span],
                    duplicate_key: duplicate,
                    keys_created: false,
                })
            }
            Target::Append(array) => Ok(Outcome {
                changes: editor.append(array, None, value)?,
                targets: vec![self.tree.nodes[array].span],
                ..Outcome::default()
            }),
            Target::Missing { container, segment } => {
                let rest = &key.segments()[segment..];
                let explicit_position = rest.len() == 1 && matches!(rest[0], KeySegment::Index(_));
                if !create && !explicit_position {
                    return Err(key_not_found(key, &[]));
                }
                let (name, nested) = self.nested_value(container, rest, value, key)?;
                Ok(Outcome {
                    changes: editor.append(container, name.as_deref(), &nested)?,
                    targets: vec![self.tree.nodes[container].span],
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
        let (first, repeated) = self.plan_remove_once(key)?;
        if !repeated {
            return Ok(first);
        }
        let mut text = editor::apply(&self.text, &first.changes)?;
        loop {
            let current = Self::parse(text, self.dialect)?;
            let (next, again) = current.plan_remove_once(key)?;
            text = editor::apply(&current.text, &next.changes)?;
            if !again {
                break;
            }
        }
        Ok(Outcome {
            changes: vec![editor::difference(&self.text, &text)],
            targets: first.targets,
            duplicate_key: true,
            keys_created: false,
        })
    }

    /// Comment attached to the value `id` with the entry `entry` in its
    /// container (`None` for the root): the comment lines directly above the
    /// entry, or else a comment after it on the same line, without comment
    /// markers.
    pub(crate) fn comment_of(&self, id: NodeId, entry: Option<&syntax::Entry>) -> Option<String> {
        let layout = Layout::new(&self.text, &self.tree);
        let start = entry.map_or(self.tree.nodes[id].span.start, |entry| {
            entry.start(&self.tree)
        });
        let block = layout
            .leading_comments(start)
            .or_else(|| entry.and_then(|e| layout.trailing_comment(e)))?;
        let texts: Vec<&str> = layout
            .comments_in(block)
            .map(|span| layout::comment_text(&self.text[span.start..span.end]))
            .collect();
        Some(texts.join("\n"))
    }

    /// Offset of the comment lines directly above `entry`; such comments are
    /// removed together with the entry.
    pub(crate) fn leading_comment_start(&self, entry: &syntax::Entry) -> Option<usize> {
        let layout = Layout::new(&self.text, &self.tree);
        layout
            .leading_comments(entry.start(&self.tree))
            .map(|block| block.start)
    }

    /// Plans removing one value at `key`; for a repeated member name the
    /// last occurrence. The flag tells whether other occurrences of the
    /// name remain.
    fn plan_remove_once(&self, key: &KeyPath) -> Result<(Outcome, bool), Error> {
        let Target::Node { id, duplicate } = self.locate(key)? else {
            return Err(key_not_found(key, &[]));
        };
        let Some(container) = self.tree.nodes[id].parent else {
            return Err(key_conflict(
                key,
                "root",
                "selects the document root, which cannot be removed; set it to a new value instead",
            ));
        };
        let entries = self.tree.nodes[container].entries().unwrap_or_default();
        let index = entries
            .iter()
            .position(|entry| entry.value == id)
            .unwrap_or_default();
        let name = entries[index].key.as_ref().map(|name| name.text.as_str());
        let repeated = name.is_some_and(|name| {
            entries
                .iter()
                .filter(|entry| entry.key.as_ref().is_some_and(|key| key.text == name))
                .count()
                > 1
        });
        let editor = Editor::new(Layout::new(&self.text, &self.tree));
        let outcome = Outcome {
            changes: editor.remove(container, index),
            targets: vec![self.tree.nodes[container].span],
            duplicate_key: duplicate,
            keys_created: false,
        };
        Ok((outcome, repeated))
    }

    /// Name of the entry to add to `container` and its value, with the
    /// remaining segments turned into nested maps and lists.
    fn nested_value(
        &self,
        container: NodeId,
        rest: &[KeySegment],
        value: &Value,
        key: &KeyPath,
    ) -> Result<(Option<String>, Value), Error> {
        let mut nested = value.clone();
        for segment in rest[1..].iter().rev() {
            nested = match segment {
                KeySegment::Key(name) => Value::Map(IndexMap::from([(name.clone(), nested)])),
                KeySegment::Index(0) | KeySegment::Append => Value::List(vec![nested]),
                _ => return Err(cannot_create(key)),
            };
        }
        let is_object = matches!(self.tree.nodes[container].kind, NodeKind::Object(_));
        match &rest[0] {
            KeySegment::Key(name) if is_object => Ok((Some(name.clone()), nested)),
            KeySegment::Key(_) | KeySegment::Index(_) => Ok((None, nested)),
            _ => Err(cannot_create(key)),
        }
    }

    /// Follows a concrete key path (keys, indices and a final `[+]`).
    fn locate(&self, key: &KeyPath) -> Result<Target, Error> {
        if matches!(key.document(), Some(DocSelector::Index(index)) if index > 0) {
            return Err(Error::new(
                ErrorCode::KeyNotFound,
                format!(
                    "key `{key}` selects a document after the first; a JSON file holds one document, remove the document selector"
                ),
            ));
        }
        let mut id = self.tree.root;
        let mut duplicate = false;
        for (position, segment) in key.segments().iter().enumerate() {
            match self.step(id, segment, key)? {
                Step::Child {
                    id: child,
                    duplicate: repeated,
                } => {
                    id = child;
                    duplicate |= repeated;
                }
                Step::Missing => {
                    return Ok(Target::Missing {
                        container: id,
                        segment: position,
                    });
                }
                Step::Append => return Ok(Target::Append(id)),
            }
        }
        Ok(Target::Node { id, duplicate })
    }

    fn step(&self, id: NodeId, segment: &KeySegment, key: &KeyPath) -> Result<Step, Error> {
        let node = &self.tree.nodes[id];
        match (&node.kind, segment) {
            (NodeKind::Object(entries), KeySegment::Key(name)) => {
                if let Some(positions) = self.indexed_members(id, name) {
                    let Some(&last) = positions.last() else {
                        return Ok(Step::Missing);
                    };
                    return Ok(Step::Child {
                        id: entries[last].value,
                        duplicate: positions.len() > 1,
                    });
                }
                let mut matches = entries
                    .iter()
                    .filter(|e| e.key.as_ref().is_some_and(|k| &k.text == name));
                let Some(last) = matches.next_back() else {
                    return Ok(Step::Missing);
                };
                Ok(Step::Child {
                    id: last.value,
                    duplicate: matches.next().is_some(),
                })
            }
            (NodeKind::Array(entries), KeySegment::Key(name)) => match name.parse::<i64>() {
                Ok(index) if !name.starts_with(['-', '+']) => {
                    index_step(entries.len(), index, key).map(|step| element_step(step, entries))
                }
                _ => Err(key_conflict(
                    key,
                    "list",
                    "passes through a list; use an index such as [0]",
                )),
            },
            (NodeKind::Array(entries), KeySegment::Index(index)) => {
                index_step(entries.len(), *index, key).map(|step| element_step(step, entries))
            }
            (NodeKind::Array(_), KeySegment::Append) => Ok(Step::Append),
            (NodeKind::Object(_), KeySegment::Index(_) | KeySegment::Append) => Err(key_conflict(
                key,
                "map",
                "passes through a map; use a member name",
            )),
            (NodeKind::Object(_) | NodeKind::Array(_), _) => Err(key_conflict(
                key,
                "pattern",
                "does not address a single value; use member names and indices",
            )),
            _ => Err(key_conflict(
                key,
                "scalar",
                "passes through a scalar; set the value itself or replace it with a map",
            )),
        }
    }
}

fn element_step(step: Option<usize>, entries: &[syntax::Entry]) -> Step {
    step.map_or(Step::Missing, |index| Step::Child {
        id: entries[index].value,
        duplicate: false,
    })
}

/// Position of an index in a list of `len` elements: `Some` for an
/// element, `None` for the position after the last one.
fn index_step(len: usize, index: i64, key: &KeyPath) -> Result<Option<usize>, Error> {
    let len_signed = i64::try_from(len).unwrap_or(i64::MAX);
    let resolved = if index < 0 { len_signed + index } else { index };
    match usize::try_from(resolved) {
        Ok(position) if position < len => Ok(Some(position)),
        Ok(position) if position == len && index >= 0 => Ok(None),
        _ => Err(key_conflict(
            key,
            "list",
            format!("has index {index} outside a list of {len} elements; use [+] to append"),
        )),
    }
}

fn cannot_create(key: &KeyPath) -> Error {
    key_conflict(
        key,
        "pattern",
        "can create missing values only with member names, [0] and [+]; set the existing parent instead",
    )
}

/// `E_DOC_PARSE` with the position of a syntax error in its message,
/// details and location.
fn parse_error(text: &str, dialect: Dialect, error: &syntax::SyntaxError) -> Error {
    let offset = error.offset.min(text.len());
    let parsed = doc_parse(
        &dialect.format_id(),
        text.as_bytes(),
        Some(offset),
        &error.message,
    );
    let detail = |name: &str| parsed.details.get(name).and_then(serde_json::Value::as_u64);
    let location = Location {
        path: String::new(),
        line: detail("line"),
        column: detail("column"),
        end_line: None,
        end_column: None,
        byte: detail("byte"),
    };
    parsed.with_location(location)
}

#[cfg(test)]
mod corpus_tests;
#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
