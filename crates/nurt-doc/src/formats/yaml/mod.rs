// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::yaml
// Responsibility:   Reading and editing of YAML documents through a lossless model of the byte ranges of the unchanged text.
// Dependencies:     comment, indexmap, model, nurt_contracts, style
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! YAML documents.
//!
//! A document is read into a lossless [`YamlModel`]: every node, indicator,
//! property and comment is described by its byte range in the unchanged
//! text. The `saphyr` loader validates the stream; ranges of indicators,
//! anchors, tags and comments are read from the text between the nodes it
//! reports. Plain scalars are typed by YAML 1.2 unless a document declares
//! `%YAML 1.1` or `documents.yaml_version` selects 1.1.
//!
//! A change is a list of byte edits of the original text. Before it is
//! accepted, the edited text is read again and its value must equal the
//! value of the old model with the change applied, including the effect on
//! aliases of a changed anchored node and on mappings that merge it.
//! Template syntax (`{{ }}`, `{% %}`) is not YAML and fails with
//! `E_DOC_PARSE`, with a fix that points to editing the file as text.

pub mod anchor;
mod backend_saphyr;
pub mod comment;
mod map;
pub mod merge;
pub mod model;
pub mod multidoc;
mod parse;
pub mod scalar;
pub mod seq;
mod style;
pub mod tag;

use std::collections::{HashMap, HashSet};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Fields, Value};

pub use model::*;

use crate::constructs::{self, DOC_SEMANTIC_MISMATCH};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{
    DocHandle, DocTree, FormatSemantics, Layout as TreeLayout, MapSemantics, NodeKind as TreeKind,
    NodeSpec, ScalarStyle as TreeScalarStyle, SetOptions, Span, Style, TreeBuilder, doc_parse,
    key_conflict, key_not_found,
};
use crate::registry::{Confidence, DocFormat, DocOptions};
use crate::value::{apply_set, equivalent, to_document_value, value_type};
use comment::Comments;
use style::Layout;

/// Rules of key paths in YAML: a repeated key means its last occurrence.
const SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// Flag of a change that created intermediate keys.
const KEYS_CREATED_FLAG: &str = "keys_created";

/// File name extensions of YAML documents.
const EXTENSIONS: &[&str] = &["yaml", "yml"];

/// Byte order mark at the start of a text.
const UTF8_BYTE_ORDER_MARK: &[u8] = b"\xEF\xBB\xBF";

/// Lines read when the content of a file is sniffed.
const SNIFF_LINES: usize = 32;

/// The format `yaml`.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::yaml::YamlFormat;
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let handle = YamlFormat.open(b"name: demo # app\n".to_vec(), &DocOptions::default())?;
/// assert_eq!(handle.text(), b"name: demo # app\n");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct YamlFormat;

/// Formats of this module, for the registry.
#[must_use]
pub fn formats() -> Vec<Box<dyn DocFormat>> {
    vec![Box::new(YamlFormat)]
}

impl DocFormat for YamlFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::YAML
    }

    fn extensions(&self) -> &[&'static str] {
        EXTENSIONS
    }

    /// A text starting with `%YAML` or `---` is a YAML document; a text whose
    /// first content line is `key: value` probably is one.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let content = content
            .strip_prefix(UTF8_BYTE_ORDER_MARK)
            .unwrap_or(content);
        let text = String::from_utf8_lossy(content);
        let Some(first) = text
            .lines()
            .take(SNIFF_LINES)
            .map(str::trim_end)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
        else {
            return Confidence::None;
        };
        if first.starts_with("%YAML") || first == "---" || first.starts_with("--- ") {
            return Confidence::High;
        }
        let key_line = first.split_once(':').is_some_and(|(key, rest)| {
            is_simple_key(key) && (rest.is_empty() || rest.starts_with(' '))
        });
        if key_line || first.starts_with("- ") {
            Confidence::Low
        } else {
            Confidence::None
        }
    }

    fn open(&self, content: Vec<u8>, options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let text = String::from_utf8(content).map_err(|error| {
            let offset = error.utf8_error().valid_up_to();
            doc_parse(
                &DocFormatId::YAML,
                error.as_bytes(),
                Some(offset),
                "the text is not valid UTF-8; save the file as UTF-8",
            )
        })?;
        Ok(Box::new(YamlDoc::parse(text, options.yaml_version)?))
    }
}

fn is_simple_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '"' | '\''))
}

/// Reads the edited text with the `saphyr` loader and the model builder, for
/// the meaning guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct YamlReference {
    version: YamlVersion,
}

impl ReferenceParser for YamlReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let text = std::str::from_utf8(text).map_err(|error| {
            doc_parse(
                &DocFormatId::YAML,
                text,
                Some(error.valid_up_to()),
                "the text is not valid UTF-8",
            )
        })?;
        let model = YamlModel::parse_with(text, self.version)?;
        (0..model.documents().len())
            .map(|index| model.document_value(index))
            .collect()
    }
}

/// Open YAML document: the model, its node tree, and the operations that
/// change it.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::yaml::{YamlDoc, YamlVersion};
/// use nurt_doc::model::{DocHandle, SetOptions};
///
/// let mut doc = YamlDoc::parse("app:\n  replicas: 2 # scale\n", YamlVersion::V1_2)?;
/// doc.set(&KeyPath::parse("app.replicas")?, &Value::Int(3), &SetOptions::default())?;
/// doc.set(&KeyPath::parse("app.debug")?, &Value::Text("on".to_owned()), &SetOptions { create: true })?;
/// assert_eq!(doc.text(), b"app:\n  replicas: 3 # scale\n  debug: \"on\"\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct YamlDoc {
    model: YamlModel,
    tree: DocTree,
    version: YamlVersion,
}

impl YamlDoc {
    /// Opens the YAML stream `text`; plain scalars of documents without a
    /// `%YAML` directive are typed by `version`.
    ///
    /// # Errors
    ///
    /// `E_DOC_PARSE` for text that is not a YAML stream and for template
    /// syntax; `E_UNSUPPORTED_CONSTRUCT` naming
    /// `yaml.alias_cycle` or `yaml.alias_expansion_limit` for aliases whose
    /// expansion does not end or grows beyond the limit.
    pub fn parse(text: impl Into<String>, version: YamlVersion) -> Result<Self> {
        let text = text.into();
        let model = YamlModel::parse_with(&text, version)?;
        anchor::check_expansion(&model)?;
        let tree = build_tree(&model);
        Ok(Self {
            model,
            tree,
            version,
        })
    }

    /// The model of the current text.
    #[must_use]
    pub const fn model(&self) -> &YamlModel {
        &self.model
    }

    /// Value of every document, in stream order.
    ///
    /// # Errors
    ///
    /// As for [`YamlModel::value`].
    pub fn values(&self) -> Result<Vec<Value>> {
        (0..self.model.documents().len())
            .map(|index| self.model.document_value(index))
            .collect()
    }

    /// Concrete keys selected by `key`: the key itself, once per document
    /// for `#*:`, when it has only names, indexes and a final `[+]`;
    /// otherwise the keys of the nodes it matches.
    fn concrete_keys(&self, key: &KeyPath) -> Result<Vec<KeyPath>> {
        let direct = key.segments().iter().all(|segment| {
            matches!(
                segment,
                KeySegment::Key(_) | KeySegment::Index(_) | KeySegment::Append
            )
        });
        if direct && key.document() == Some(DocSelector::All) {
            return (0..self.model.documents().len())
                .map(|index| {
                    let document = multidoc::selector_of(index);
                    KeyPath::new(document, key.segments().to_vec()).map_err(|error| {
                        Error::new(
                            ErrorCode::KeyConflict,
                            format!("key `{key}` is not valid: {error}"),
                        )
                    })
                })
                .collect();
        }
        if direct {
            return Ok(vec![key.clone()]);
        }
        let mut keys: Vec<KeyPath> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for resolved in self.tree.resolve(key, &SEMANTICS)? {
            if seen.insert(resolved.key.to_string()) {
                keys.push(resolved.key);
            }
        }
        Ok(keys)
    }

    fn document_root(&self, key: &KeyPath) -> Result<NodeId> {
        let index = match key.document() {
            None => 0,
            Some(DocSelector::Index(index)) => usize::try_from(index).unwrap_or(usize::MAX),
            Some(DocSelector::All) => {
                return Err(key_conflict(
                    key,
                    "pattern",
                    "selects every document; a change needs one document",
                ));
            }
        };
        self.model
            .documents()
            .get(index)
            .map(|document| document.root)
            .ok_or_else(|| multidoc::document_not_found(&self.model, key))
    }

    fn not_found(&self, key: &KeyPath) -> Error {
        key_not_found(key, &self.tree.similar_keys(key, &SEMANTICS))
    }

    /// One step of a concrete key path from `node`.
    fn step(&self, node: NodeId, segment: &KeySegment, key: &KeyPath) -> Result<Step> {
        let node = self.model.dealias(node);
        let yaml = self.model.node(node);
        if yaml.is_empty_node() {
            return Ok(Step::Empty(node));
        }
        match (&yaml.kind, segment) {
            (NodeKind::Mapping(_), KeySegment::Key(name)) => Ok(self.member_step(node, name)),
            (NodeKind::Sequence(_), KeySegment::Key(name)) if is_index(name) => {
                let index = name.parse::<i64>().unwrap_or(i64::MAX);
                self.index_step(node, index, key)
            }
            (NodeKind::Sequence(_), KeySegment::Index(index)) => self.index_step(node, *index, key),
            (NodeKind::Sequence(_), KeySegment::Append) => Ok(Step::Append(node)),
            (NodeKind::Mapping(_), KeySegment::Index(_) | KeySegment::Append) => Err(key_conflict(
                key,
                "map",
                "applies an index to a map; use a key name",
            )),
            (_, KeySegment::Key(_)) => Err(key_conflict(
                key,
                kind_name(yaml),
                format!("passes through a {}; a name needs a map", kind_name(yaml)),
            )),
            (_, KeySegment::Index(_) | KeySegment::Append) => Err(key_conflict(
                key,
                kind_name(yaml),
                format!(
                    "applies an index to a {}; an index needs a list",
                    kind_name(yaml)
                ),
            )),
            _ => Err(key_conflict(
                key,
                "pattern",
                "is not a concrete key; use names and indexes",
            )),
        }
    }

    fn member_step(&self, map: NodeId, name: &str) -> Step {
        if let Some(value) = self.model.map_get(map, name) {
            return Step::Node(value);
        }
        match merge::inherited_value(&self.model, map, name) {
            Some(value) => Step::Inherited { map, value },
            None => Step::Missing(map),
        }
    }

    fn index_step(&self, sequence: NodeId, index: i64, key: &KeyPath) -> Result<Step> {
        Ok(
            match seq::index_target(&self.model, sequence, index, key)? {
                seq::IndexTarget::Item(item) => Step::Node(item),
                seq::IndexTarget::Append => Step::Append(sequence),
            },
        )
    }

    fn plan_set(
        &self,
        change: &mut Change<'_>,
        key: &KeyPath,
        value: &Value,
        create: bool,
    ) -> Result<()> {
        let mut node = self.document_root(key)?;
        let segments = key.segments();
        for (position, segment) in segments.iter().enumerate() {
            let rest = &segments[position + 1..];
            match self.step(node, segment, key)? {
                Step::Node(next) => node = next,
                Step::Missing(map) if create => {
                    let name = segment_name(segment);
                    return change.add_entry(map, &name, rest, value, key);
                }
                Step::Missing(_) => return Err(self.not_found(key)),
                Step::Inherited {
                    map,
                    value: inherited,
                } => {
                    let name = segment_name(segment);
                    return change.override_key(map, &name, inherited, rest, value, key);
                }
                Step::Append(sequence) if create || rest.is_empty() => {
                    return change.append(sequence, rest, value, key);
                }
                Step::Empty(empty) if create => {
                    return change.fill(empty, &segments[position..], value, key);
                }
                Step::Append(_) | Step::Empty(_) => return Err(self.not_found(key)),
            }
        }
        change.replace(node, value)
    }

    /// Container and position of the node removed by `key`.
    fn locate_removal(&self, key: &KeyPath) -> Result<Removal> {
        let Some((last, parents)) = key.segments().split_last() else {
            return Err(Error::new(
                ErrorCode::KeyConflict,
                "the document root cannot be removed; set it to a new value instead",
            ));
        };
        let mut node = self.document_root(key)?;
        for segment in parents {
            match self.step(node, segment, key)? {
                Step::Node(next) => node = next,
                _ => return Err(self.not_found(key)),
            }
        }
        let container = self.model.dealias(node);
        match self.step(container, last, key)? {
            Step::Node(target) => self.removal_of(container, target, last, key),
            Step::Inherited { map, .. } => {
                merge::check_remove(&self.model, map, &segment_name(last))?;
                Err(self.not_found(key))
            }
            _ => Err(self.not_found(key)),
        }
    }

    fn removal_of(
        &self,
        container: NodeId,
        target: NodeId,
        last: &KeySegment,
        key: &KeyPath,
    ) -> Result<Removal> {
        let index = match &self.model.node(container).kind {
            NodeKind::Mapping(mapping) => mapping
                .entries
                .iter()
                .position(|entry| entry.value == target),
            NodeKind::Sequence(sequence) => sequence_position(sequence, target, last),
            _ => None,
        };
        index
            .map(|index| Removal {
                container,
                index,
                node: target,
                key: key.clone(),
            })
            .ok_or_else(|| self.not_found(key))
    }

    /// Checks the change with the guards and takes the new text.
    fn commit(&mut self, change: Planned) -> Result<EditReport> {
        merge::guard_key_edits(&self.model, &change.edits)?;
        let expected = self
            .model
            .documents()
            .iter()
            .map(|document| self.model.value_with(document.root, &change.overrides))
            .collect::<Result<Vec<Value>>>()?;
        let expectation = Expectation::new(expected, &SEMANTICS);
        let plan = EditPlan {
            targets: change.targets,
            edits: change.edits,
            report: change.report,
            ..EditPlan::new()
        };
        let reference = YamlReference {
            version: self.version,
        };
        let committed = guard::commit(
            self.model.source().as_bytes(),
            plan,
            &expectation,
            &reference,
        )?;
        if committed.report.is_unchanged() {
            return Ok(committed.report);
        }
        let text = String::from_utf8(committed.bytes).map_err(|_| {
            constructs::unsupported(
                DOC_SEMANTIC_MISMATCH,
                "the change would produce invalid UTF-8",
            )
        })?;
        *self = Self::parse(text, self.version).map_err(|error| {
            constructs::unsupported(DOC_SEMANTIC_MISMATCH, "the changed document does not open")
                .with_cause(error)
        })?;
        Ok(committed.report)
    }
}

impl DocHandle for YamlDoc {
    fn format(&self) -> DocFormatId {
        DocFormatId::YAML
    }

    fn text(&self) -> &[u8] {
        self.model.source().as_bytes()
    }

    fn tree(&self) -> &DocTree {
        &self.tree
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        &SEMANTICS
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        let value = to_document_value(value.clone());
        let keys = self.concrete_keys(key)?;
        let mut change = Change::new(&self.model);
        for concrete in &keys {
            self.plan_set(&mut change, concrete, &value, options.create)?;
        }
        let planned = change.into_planned();
        self.commit(planned)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let keys = self.concrete_keys(key)?;
        let mut removals = Vec::with_capacity(keys.len());
        for concrete in &keys {
            removals.push(self.locate_removal(concrete)?);
        }
        let mut change = Change::new(&self.model);
        change.remove_all(removals)?;
        let planned = change.into_planned();
        self.commit(planned)
    }

    fn to_value(&self) -> Result<Value> {
        self.model.document_value(0)
    }
}

/// Result of one step of a key path.
enum Step {
    /// Existing node.
    Node(NodeId),
    /// Missing key of a mapping.
    Missing(NodeId),
    /// Key that a mapping inherits through a merge key.
    Inherited { map: NodeId, value: NodeId },
    /// Position after the last item of a sequence.
    Append(NodeId),
    /// Empty node, which a change with `create` fills.
    Empty(NodeId),
}

/// Entry or item removed by a key.
struct Removal {
    container: NodeId,
    index: usize,
    node: NodeId,
    key: KeyPath,
}

/// Edits, targets, expected value and report of a planned change.
struct Planned {
    edits: Vec<ByteEdit>,
    targets: Vec<Span>,
    overrides: ValueOverrides,
    report: EditReport,
}

/// Edits, targets, expected value and report of a change being planned.
struct Change<'m> {
    model: &'m YamlModel,
    layout: Layout,
    edits: Vec<ByteEdit>,
    targets: Vec<Span>,
    overrides: ValueOverrides,
    report: EditReport,
}

impl<'m> Change<'m> {
    fn into_planned(self) -> Planned {
        Planned {
            edits: self.edits,
            targets: self.targets,
            overrides: self.overrides,
            report: self.report,
        }
    }

    fn new(model: &'m YamlModel) -> Self {
        Self {
            model,
            layout: Layout::of(model),
            edits: Vec::new(),
            targets: Vec::new(),
            overrides: ValueOverrides::default(),
            report: EditReport::default(),
        }
    }

    fn push(&mut self, edits: Vec<ByteEdit>) {
        for edit in edits {
            self.targets.push(edit.span);
            self.edits.push(edit);
        }
        self.report.replacements += 1;
    }

    fn shared(&mut self, node: NodeId) {
        anchor::note_shared(&mut self.report, &anchor::shared_by(self.model, node));
    }

    fn created(&mut self, key: &KeyPath) {
        self.report
            .note(Notable::KeysCreated, Some(key.to_string()));
        self.report.flag(KEYS_CREATED_FLAG);
    }

    /// Replaces the value of an existing node; setting an equal value of the
    /// same type changes nothing.
    fn replace(&mut self, node: NodeId, value: &Value) -> Result<()> {
        let current = self.model.value(node)?;
        if value_type(&current) == value_type(value) && equivalent(&current, value) {
            return Ok(());
        }
        let mut edits = if let Value::Bytes(data) = value {
            let text = style::separated(
                self.model,
                self.model.node(node).span.start,
                tag::render_binary(self.model, node, data),
            );
            let span = &self.model.node(node).span;
            vec![ByteEdit::replace(Span::new(span.start, span.end), text)]
        } else {
            map::replace(self.model, node, value, &self.layout)?
        };
        let tag_plan = tag::plan_set(self.model, node, value);
        if tag_plan.removed.is_some() {
            self.report.flag(tag::TAG_REMOVED_FLAG);
        }
        edits.extend(tag_plan.edits);
        self.push(edits);
        self.shared(node);
        self.overrides.replaced.insert(node, value.clone());
        Ok(())
    }

    fn add_entry(
        &mut self,
        map: NodeId,
        name: &str,
        rest: &[KeySegment],
        value: &Value,
        key: &KeyPath,
    ) -> Result<()> {
        let nested = nest(rest, value)?;
        let edits = map::insert_entry(self.model, map, name, &nested, &self.layout)?;
        self.push(edits);
        if !rest.is_empty() {
            self.created(key);
        }
        self.shared(map);
        self.overrides
            .appended_entries
            .entry(map)
            .or_default()
            .push((name.to_owned(), nested));
        Ok(())
    }

    fn override_key(
        &mut self,
        map: NodeId,
        name: &str,
        inherited: NodeId,
        rest: &[KeySegment],
        value: &Value,
        key: &KeyPath,
    ) -> Result<()> {
        let local = if rest.is_empty() {
            value.clone()
        } else {
            let mut merged = self.model.value(inherited)?;
            let path = KeyPath::new(None, rest.to_vec()).map_err(|error| {
                Error::new(
                    ErrorCode::KeyConflict,
                    format!("key `{key}` is not valid: {error}"),
                )
            })?;
            apply_set(&mut merged, &path, value.clone(), &SEMANTICS)?;
            merged
        };
        let edits = map::insert_entry(self.model, map, name, &local, &self.layout)?;
        self.push(edits);
        self.report.flag(merge::MERGE_OVERRIDE_FLAG);
        self.shared(map);
        self.overrides
            .appended_entries
            .entry(map)
            .or_default()
            .push((name.to_owned(), local));
        Ok(())
    }

    fn append(
        &mut self,
        sequence: NodeId,
        rest: &[KeySegment],
        value: &Value,
        key: &KeyPath,
    ) -> Result<()> {
        let nested = nest(rest, value)?;
        let plan = seq::plan_append(self.model, sequence, &nested, key)?;
        self.push(plan.edits);
        if !rest.is_empty() {
            self.created(key);
        }
        self.shared(plan.target);
        if self.model.node(plan.target).is_empty_node() {
            self.overrides
                .replaced
                .insert(plan.target, Value::List(vec![nested]));
        } else {
            self.overrides
                .appended_items
                .entry(plan.target)
                .or_default()
                .push(nested);
        }
        Ok(())
    }

    fn fill(
        &mut self,
        node: NodeId,
        segments: &[KeySegment],
        value: &Value,
        key: &KeyPath,
    ) -> Result<()> {
        let nested = nest(segments, value)?;
        let edits = map::replace(self.model, node, &nested, &self.layout)?;
        self.push(edits);
        if segments.len() > 1 {
            self.created(key);
        }
        self.shared(node);
        self.overrides.replaced.insert(node, nested);
        Ok(())
    }

    /// Removes the selected entries and items; a collection that loses every
    /// member becomes `{}` or `[]`.
    fn remove_all(&mut self, mut removals: Vec<Removal>) -> Result<()> {
        let removed: HashSet<NodeId> = removals.iter().map(|removal| removal.node).collect();
        removals.retain(|removal| !self.inside_any(removal.container, &removed));
        let mut groups: HashMap<NodeId, Vec<Removal>> = HashMap::new();
        let mut order = Vec::new();
        for removal in removals {
            if !groups.contains_key(&removal.container) {
                order.push(removal.container);
            }
            groups.entry(removal.container).or_default().push(removal);
        }
        for container in order {
            let mut group = groups.remove(&container).unwrap_or_default();
            group.sort_by_key(|removal| removal.index);
            group.dedup_by_key(|removal| removal.index);
            self.remove_group(container, &group)?;
        }
        Ok(())
    }

    fn inside_any(&self, node: NodeId, removed: &HashSet<NodeId>) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if removed.contains(&id) {
                return true;
            }
            current = self.model.parent(id);
        }
        false
    }

    fn remove_group(&mut self, container: NodeId, group: &[Removal]) -> Result<()> {
        let yaml = self.model.node(container);
        let members = match &yaml.kind {
            NodeKind::Mapping(mapping) => mapping.entries.len(),
            NodeKind::Sequence(sequence) => sequence.items.len(),
            _ => 0,
        };
        let mapping = yaml.as_mapping().is_some();
        if group.len() == members {
            let edits = map::clear(self.model, container, &self.layout);
            self.edits.extend(edits.iter().cloned());
            self.targets.extend(edits.iter().map(|edit| edit.span));
        }
        for removal in group {
            if group.len() < members {
                let edits = if mapping {
                    map::remove_entry(self.model, container, removal.index, &self.layout)
                } else {
                    let index = i64::try_from(removal.index).unwrap_or(i64::MAX);
                    seq::plan_remove(self.model, container, index, &removal.key)?.edits
                };
                self.edits.extend(edits.iter().cloned());
                self.targets.extend(edits.iter().map(|edit| edit.span));
            }
            self.report.replacements += 1;
            self.shared(removal.node);
            if mapping {
                self.overrides
                    .removed_entries
                    .insert((container, removal.index));
            } else {
                self.overrides
                    .removed_items
                    .insert((container, removal.index));
            }
        }
        Ok(())
    }
}

/// Position of the item `target` in `sequence`. The segment that selected the
/// item names its position, which is checked first so that removing every
/// item of a long list does not search the list for each of them.
fn sequence_position(
    sequence: &SequenceNode,
    target: NodeId,
    selected_by: &KeySegment,
) -> Option<usize> {
    let hint = match selected_by {
        KeySegment::Index(index) if *index >= 0 => usize::try_from(*index).ok(),
        KeySegment::Index(index) => usize::try_from(index.unsigned_abs())
            .ok()
            .and_then(|back| sequence.items.len().checked_sub(back)),
        KeySegment::Key(name) if is_index(name) => name.parse().ok(),
        _ => None,
    };
    hint.filter(|position| {
        sequence
            .items
            .get(*position)
            .is_some_and(|item| item.node == target)
    })
    .or_else(|| sequence.items.iter().position(|item| item.node == target))
}

/// Value of a missing path: `value` wrapped in a map for each name and in a
/// list for each `[0]` or `[+]` of `segments`.
fn nest(segments: &[KeySegment], value: &Value) -> Result<Value> {
    let mut nested = value.clone();
    for segment in segments.iter().rev() {
        nested = match segment {
            KeySegment::Key(name) => {
                let mut fields = Fields::new();
                fields.insert(name.clone(), nested);
                Value::Map(fields)
            }
            KeySegment::Index(0) | KeySegment::Append => Value::List(vec![nested]),
            _ => {
                return Err(Error::new(
                    ErrorCode::KeyConflict,
                    "only key names, [0] and [+] can create missing values; set the existing parent instead",
                ));
            }
        };
    }
    Ok(nested)
}

fn segment_name(segment: &KeySegment) -> String {
    match segment {
        KeySegment::Key(name) => name.clone(),
        KeySegment::Index(index) => index.to_string(),
        _ => String::new(),
    }
}

fn is_index(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

fn kind_name(node: &YamlNode) -> &'static str {
    match &node.kind {
        NodeKind::Mapping(_) => "map",
        NodeKind::Sequence(_) => "list",
        NodeKind::Alias(_) => "alias",
        NodeKind::Scalar(_) if node.is_empty_node() => "null",
        NodeKind::Scalar(_) => "scalar",
    }
}

/// Node of the tree to add: the model node and how its parent holds it.
struct Pending {
    node: NodeId,
    parent: Option<crate::model::NodeId>,
    entry: Option<(String, std::ops::Range<usize>, std::ops::Range<usize>)>,
    /// The tree node is a member inherited through a merge key, an alias of
    /// `node`.
    merged: bool,
}

/// Node tree of the model: map members carry their key, merged keys appear
/// as members that alias the merged value, aliases point to their anchors.
fn build_tree(model: &YamlModel) -> DocTree {
    let mut builder = TreeBuilder::new(model.source().as_bytes());
    let comments = Comments::new(model);
    let mut ids: HashMap<NodeId, crate::model::NodeId> = HashMap::new();
    let mut aliases: Vec<(crate::model::NodeId, NodeId)> = Vec::new();
    for document in model.documents() {
        let mut pending = vec![Pending {
            node: document.root,
            parent: None,
            entry: None,
            merged: false,
        }];
        while let Some(item) = pending.pop() {
            if item.merged {
                let id = builder.node(item.parent, merged_spec(&item));
                aliases.push((id, item.node));
                continue;
            }
            let spec = node_spec(model, &comments, &item);
            let id = builder.node(item.parent, spec);
            ids.insert(item.node, id);
            if let Some(target) = model.anchor_target(item.node) {
                aliases.push((id, target));
            }
            pending.extend(children(model, item.node, id).into_iter().rev());
        }
    }
    for (alias, target) in aliases {
        if let Some(&target) = ids.get(&target) {
            builder.set_alias(alias, target);
        }
    }
    builder.finish()
}

/// Children of a collection in source order; members inherited through a
/// merge key are added at once as aliases of the merged values.
fn children(model: &YamlModel, node: NodeId, tree_id: crate::model::NodeId) -> Vec<Pending> {
    match &model.node(node).kind {
        NodeKind::Mapping(mapping) => {
            let mut children: Vec<Pending> = mapping
                .entries
                .iter()
                .filter(|entry| !entry.merge)
                .map(|entry| Pending {
                    node: entry.value,
                    parent: Some(tree_id),
                    entry: Some((
                        key_name(model, entry.key),
                        model.node(entry.key).full_span(),
                        entry.span.clone(),
                    )),
                    merged: false,
                })
                .collect();
            if mapping.entries.iter().any(|entry| entry.merge) {
                children.extend(inherited(model, node, tree_id));
            }
            children
        }
        NodeKind::Sequence(sequence) => sequence
            .items
            .iter()
            .map(|item| Pending {
                node: item.node,
                parent: Some(tree_id),
                entry: None,
                merged: false,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Name of a mapping key: the value of a scalar key, otherwise the name
/// given by the merge rules (alias keys, complex keys), which resolve
/// anchors over the whole model and are only used where needed.
fn key_name(model: &YamlModel, key: NodeId) -> String {
    match model.node(key).as_scalar() {
        Some(scalar) => scalar.value.clone(),
        None => merge::key_name(model, key).unwrap_or_default(),
    }
}

/// Members of `map` inherited through its merge keys, placed at the first
/// merge key.
fn inherited(model: &YamlModel, map: NodeId, tree_id: crate::model::NodeId) -> Vec<Pending> {
    let merge_span = model
        .node(map)
        .as_mapping()
        .and_then(|mapping| mapping.entries.iter().find(|entry| entry.merge))
        .map_or(0..0, |entry| entry.span.clone());
    merge::inherited_members(model, map)
        .into_iter()
        .map(|member| Pending {
            node: member.value,
            parent: Some(tree_id),
            entry: Some((member.name, merge_span.clone(), merge_span.clone())),
            merged: true,
        })
        .collect()
}

fn merged_spec(item: &Pending) -> NodeSpec {
    let (name, span) = item
        .entry
        .as_ref()
        .map_or((String::new(), 0..0), |(name, _, span)| {
            (name.clone(), span.clone())
        });
    NodeSpec::new(TreeKind::Scalar, span_of(&span))
        .with_name(name)
        .with_key_span(span_of(&span))
}

fn node_spec(model: &YamlModel, comments: &Comments<'_>, item: &Pending) -> NodeSpec {
    let yaml = model.node(item.node);
    let value_span = span_of(&yaml.span);
    let (kind, value, scalar_style, layout) = match &yaml.kind {
        NodeKind::Scalar(scalar) => (
            TreeKind::Scalar,
            Some(scalar::scalar_value(
                scalar,
                yaml.tag.as_ref(),
                model.version_of(item.node),
            )),
            tree_style(scalar.style),
            TreeLayout::Block,
        ),
        NodeKind::Mapping(mapping) => (
            TreeKind::Map,
            None,
            TreeScalarStyle::Plain,
            collection_layout(mapping.style),
        ),
        NodeKind::Sequence(sequence) => (
            TreeKind::List,
            None,
            TreeScalarStyle::Plain,
            collection_layout(sequence.style),
        ),
        NodeKind::Alias(_) => (
            TreeKind::Scalar,
            None,
            TreeScalarStyle::Plain,
            TreeLayout::Block,
        ),
    };
    let whole = item.entry.as_ref().map_or_else(
        || span_of(&yaml.full_span()),
        |(_, _, entry)| span_of(entry),
    );
    let mut spec = NodeSpec::new(kind, whole)
        .with_value_span(value_span)
        .with_style(Style {
            scalar: scalar_style,
            layout: if yaml.in_flow {
                TreeLayout::Flow
            } else {
                layout
            },
            indent: Some(model.column(yaml.span.start)),
        });
    let start = item
        .entry
        .as_ref()
        .map_or(yaml.full_span().start, |(_, _, entry)| entry.start);
    if let Some((name, key_span, _)) = &item.entry {
        spec = spec
            .with_name(name.clone())
            .with_key_span(span_of(key_span));
    }
    if let Some(value) = value {
        spec = spec.with_value(value);
    }
    let content_end = if yaml.is_block_collection() {
        item.entry.as_ref().map_or(start, |(_, key, _)| key.end + 1)
    } else {
        model
            .line_end(yaml.span.start)
            .min(yaml.span.end.max(yaml.span.start))
    };
    if let Some(comment) = comments.describe(start, content_end) {
        spec = spec.with_comment(comment);
    }
    if let Some(anchor) = &yaml.anchor {
        spec = spec.with_anchor(anchor.name.clone());
    }
    if let Some(tag) = &yaml.tag {
        spec = spec.with_tag(tag.text.clone());
    }
    spec
}

const fn span_of(range: &std::ops::Range<usize>) -> Span {
    Span::new(range.start, range.end)
}

const fn tree_style(style: ScalarStyle) -> TreeScalarStyle {
    match style {
        ScalarStyle::Plain => TreeScalarStyle::Plain,
        ScalarStyle::SingleQuoted => TreeScalarStyle::SingleQuoted,
        ScalarStyle::DoubleQuoted => TreeScalarStyle::DoubleQuoted,
        ScalarStyle::Literal => TreeScalarStyle::Literal,
        ScalarStyle::Folded => TreeScalarStyle::Folded,
    }
}

const fn collection_layout(style: CollectionStyle) -> TreeLayout {
    match style {
        CollectionStyle::Block => TreeLayout::Block,
        CollectionStyle::Flow | CollectionStyle::FlowPair => TreeLayout::Flow,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::*;

    fn doc(text: &str) -> YamlDoc {
        YamlDoc::parse(text, YamlVersion::V1_2).unwrap()
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    const CREATE: SetOptions = SetOptions { create: true };

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn builds_a_tree_with_keys_comments_and_styles() {
        let doc = doc("# the name\nname: 'x'\nlist: [1, 2]\n");
        let found = doc.get(&key("name")).unwrap();
        let node = &doc.tree()[found[0].node];
        assert_eq!(node.comment.as_deref(), Some("the name"));
        assert_eq!(node.value, Some(text("x")));
        assert_eq!(node.style.scalar, TreeScalarStyle::SingleQuoted);
        let list = doc.get(&key("list[1]")).unwrap();
        assert_eq!(doc.tree()[list[0].node].value, Some(Value::Int(2)));
        assert_eq!(doc.to_value().unwrap(), doc.values().unwrap()[0]);
    }

    #[test]
    fn resolves_merged_keys_and_aliases_through_the_tree() {
        let doc = doc("base: &b {x: 1, y: 2}\nitem:\n  <<: *b\n  y: 3\nref: *b\n");
        let x = doc.get(&key("item.x")).unwrap();
        assert_eq!(
            doc.tree().value_of(x[0].node, &SEMANTICS, None),
            Value::Int(1)
        );
        let refs = doc.get(&key("ref.y")).unwrap();
        assert_eq!(
            doc.tree().value_of(refs[0].node, &SEMANTICS, None),
            Value::Int(2)
        );
    }

    #[test]
    fn sets_scalars_keeping_comments_and_style() {
        let mut doc = doc("a: 'old' # keep\nb: 1\n");
        doc.set(&key("a"), &text("new"), &SetOptions::default())
            .unwrap();
        assert_eq!(doc.text(), b"a: 'new' # keep\nb: 1\n");
        let report = doc
            .set(&key("b"), &Value::Int(1), &SetOptions::default())
            .unwrap();
        assert!(report.is_unchanged());
    }

    #[test]
    fn creates_missing_keys_only_with_create() {
        let mut doc = doc("a: 1\n");
        let error = doc
            .set(&key("b.c"), &Value::Int(2), &SetOptions::default())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyNotFound);
        let report = doc.set(&key("b.c"), &Value::Int(2), &CREATE).unwrap();
        assert_eq!(report.flags, vec![KEYS_CREATED_FLAG.to_owned()]);
        assert_eq!(doc.text(), b"a: 1\nb:\n  c: 2\n");
    }

    #[test]
    fn fills_empty_values() {
        let mut doc = doc("a:\nb: 2\n");
        doc.set(&key("a.x"), &Value::Int(1), &CREATE).unwrap();
        assert_eq!(doc.text(), b"a:\n  x: 1\nb: 2\n");
    }

    #[test]
    fn overrides_merged_keys_with_local_keys() {
        let mut doc = doc("base: &b\n  x: 1\nitem:\n  <<: *b\n  y: 2\n");
        let report = doc
            .set(&key("item.x"), &Value::Int(5), &SetOptions::default())
            .unwrap();
        assert!(
            report
                .flags
                .contains(&merge::MERGE_OVERRIDE_FLAG.to_owned())
        );
        assert_eq!(
            doc.text(),
            b"base: &b\n  x: 1\nitem:\n  <<: *b\n  y: 2\n  x: 5\n"
        );
        let mut doc = self::doc("base: &b\n  x: 1\nitem:\n  <<: *b\n");
        let error = doc.remove(&key("item.x")).unwrap_err();
        assert_eq!(
            error.found.as_deref(),
            Some(constructs::YAML_MERGE_INHERITED_REMOVE)
        );
    }

    #[test]
    fn removes_keys_and_list_items() {
        let mut doc = doc("a: 1 # one\nb:\n  - x\n  - y\nc: 3\n");
        doc.remove(&key("a")).unwrap();
        doc.remove(&key("b[0]")).unwrap();
        assert_eq!(doc.text(), b"b:\n  - y\nc: 3\n");
        doc.remove(&key("b[*]")).unwrap();
        assert_eq!(doc.text(), b"b: []\nc: 3\n");
        let error = doc.remove(&KeyPath::root()).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
    }

    #[test]
    fn removes_every_entry_matched_by_a_pattern() {
        let mut doc = doc("m:\n  a: 1\n  b: 2\nz: 0\n");
        let report = doc.remove(&key("m.*")).unwrap();
        assert_eq!(report.replacements, 2);
        assert_eq!(doc.text(), b"m: {}\nz: 0\n");
    }

    #[test]
    fn removes_every_item_of_a_long_list_and_selected_items_by_position() {
        let items: String = (0..5000).map(|index| format!("  - {index}\n")).collect();
        let mut doc = doc(&format!("l:\n{items}z: 0\n"));
        let report = doc.remove(&key("l[*]")).unwrap();
        assert_eq!(report.replacements, 5000);
        assert_eq!(doc.text(), b"l: []\nz: 0\n");
        let mut doc = self::doc("l: [a, b, c, d]\n");
        doc.remove(&key("l[-1]")).unwrap();
        doc.remove(&key("l.0")).unwrap();
        assert_eq!(doc.text(), b"l: [b, c]\n");
    }

    #[test]
    fn reads_a_flow_document_nested_to_the_parser_limit() {
        // The scanner of saphyr-parser counts flow levels in a `u8`, so a flow
        // document nests up to 255 levels; block documents reach MAX_DEPTH.
        let depth = usize::from(u8::MAX).min(backend_saphyr::MAX_DEPTH);
        let text = format!("{}1{}\n", "[".repeat(depth), "]".repeat(depth));
        let doc = YamlDoc::parse(text, YamlVersion::V1_2).unwrap();
        let mut value = doc.values().unwrap().remove(0);
        for level in 0..depth {
            let Value::List(mut items) = value else {
                panic!("level {level} is not a list");
            };
            value = items.remove(0);
        }
        assert_eq!(value, Value::Int(1));
    }

    #[test]
    fn reads_a_block_document_nested_to_the_parser_limit() {
        let depth = backend_saphyr::MAX_DEPTH;
        let mut text: String = (0..depth - 1)
            .map(|level| format!("{}a:\n", " ".repeat(level)))
            .collect();
        text.push_str(&format!("{}a: 1\n", " ".repeat(depth - 1)));
        let doc = YamlDoc::parse(text, YamlVersion::V1_2).unwrap();
        let mut value = doc.values().unwrap().remove(0);
        for level in 0..depth {
            let Value::Map(mut fields) = value else {
                panic!("level {level} is not a map");
            };
            value = fields.swap_remove("a").unwrap();
        }
        assert_eq!(value, Value::Int(1));
    }

    #[test]
    fn expansion_deeper_than_the_parser_limit_through_aliases_is_refused() {
        let mut text = String::from("l0: &l0 [x]\n");
        for level in 1..=backend_saphyr::MAX_DEPTH + 2 {
            text.push_str(&format!("l{level}: &l{level} [*l{}]\n", level - 1));
        }
        let error = YamlDoc::parse(text, YamlVersion::V1_2)
            .and_then(|doc| doc.values())
            .unwrap_err();
        assert_eq!(
            error.found.as_deref(),
            Some(crate::constructs::YAML_ALIAS_EXPANSION_LIMIT)
        );
    }

    #[test]
    fn many_merge_keys_resolve_against_their_anchors() {
        let mut text = String::from("base: &b {x: 1}\n");
        for index in 0..2000 {
            text.push_str(&format!("m{index}:\n  <<: *b\n  y: {index}\n"));
        }
        let doc = doc(&text);
        let found = doc.get(&key("m1999.x")).unwrap();
        assert_eq!(
            doc.tree().value_of(found[0].node, &SEMANTICS, None),
            Value::Int(1)
        );
        let anchors = doc.model().anchors();
        let target = doc
            .model()
            .node_ids()
            .find_map(|id| anchors.target(id))
            .unwrap();
        assert_eq!(anchors.aliases_of(target).len(), 2000);
    }

    #[test]
    fn appends_to_lists() {
        let mut doc = doc("l: [a]\n");
        doc.set(&key("l[+]"), &text("b"), &SetOptions::default())
            .unwrap();
        assert_eq!(doc.text(), b"l: [a, b]\n");
    }

    #[test]
    fn changes_shared_anchors_for_every_alias() {
        let mut doc = doc("a: &x 1\nb: *x\n");
        let report = doc
            .set(&key("a"), &Value::Int(2), &SetOptions::default())
            .unwrap();
        assert_eq!(doc.text(), b"a: &x 2\nb: *x\n");
        assert!(report.flags.contains(&anchor::ALIAS_SHARED_FLAG.to_owned()));
    }

    #[test]
    fn addresses_documents_of_a_stream() {
        let mut doc = doc("a: 1\n---\na: 2\n");
        doc.set(&key("#1:a"), &Value::Int(3), &SetOptions::default())
            .unwrap();
        assert_eq!(doc.text(), b"a: 1\n---\na: 3\n");
        doc.set(&key("#*:b"), &Value::Int(0), &CREATE).unwrap();
        assert_eq!(doc.text(), b"a: 1\nb: 0\n---\na: 3\nb: 0\n");
        assert!(doc.set(&key("#5:a"), &Value::Int(1), &CREATE).is_err());
    }

    #[test]
    fn rejects_paths_through_scalars() {
        let mut doc = doc("a: 1\n");
        let error = doc.set(&key("a.b"), &Value::Int(1), &CREATE).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
        let error = doc.set(&key("[0]"), &Value::Int(1), &CREATE).unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyConflict);
    }

    #[test]
    fn sniffs_yaml_content() {
        assert_eq!(
            YamlFormat.sniff(b"%YAML 1.2\n---\na: 1\n"),
            Confidence::High
        );
        assert_eq!(
            YamlFormat.sniff(b"\xEF\xBB\xBF# c\n---\n"),
            Confidence::High
        );
        assert_eq!(YamlFormat.sniff(b"name: x\n"), Confidence::Low);
        assert_eq!(YamlFormat.sniff(b"- a\n"), Confidence::Low);
        assert_eq!(YamlFormat.sniff(b"{\"a\": 1}"), Confidence::None);
        assert_eq!(YamlFormat.extensions(), EXTENSIONS);
        assert_eq!(formats().len(), 1);
    }

    #[test]
    fn opens_documents_and_rejects_invalid_text() {
        let options = DocOptions::default();
        assert!(YamlFormat.open(b"a: [1\n".to_vec(), &options).is_err());
        assert!(
            YamlFormat
                .open(vec![b'a', b':', b' ', 0xFF], &options)
                .is_err()
        );
        let handle = YamlFormat
            .open(
                b"a: yes\n".to_vec(),
                &DocOptions {
                    yaml_version: YamlVersion::V1_1,
                    ..options
                },
            )
            .unwrap();
        let expected = Value::Map(IndexMap::from([("a".to_owned(), Value::Bool(true))]));
        assert_eq!(handle.to_value().unwrap(), expected);
    }

    #[test]
    fn rejects_exponential_alias_expansion_as_an_unsupported_construct() {
        let mut text = String::from("l0: &l0 [x, x, x, x, x, x, x, x, x, x]\n");
        for level in 1..9 {
            let previous = level - 1;
            let items = vec![format!("*l{previous}"); 10].join(", ");
            text.push_str(&format!("l{level}: &l{level} [{items}]\n"));
        }
        let Err(error) = YamlFormat.open(text.into_bytes(), &DocOptions::default()) else {
            panic!("an exponential expansion was accepted");
        };
        assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
        assert_eq!(
            error.found.as_deref(),
            Some(crate::constructs::YAML_ALIAS_EXPANSION_LIMIT)
        );
    }

    #[test]
    fn reference_parser_reads_every_document() {
        let values = YamlReference {
            version: YamlVersion::V1_2,
        }
        .parse(b"a: 1\n--- 2\n")
        .unwrap();
        assert_eq!(values.len(), 2);
        assert!(
            YamlReference {
                version: YamlVersion::V1_2
            }
            .parse(&[0xFF])
            .is_err()
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
