// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lossless node model of a YAML stream.
//!
//! The model keeps the source text unchanged and describes every document,
//! node, indicator, property and comment by its byte range in that text.
//! Edits are replacements of byte ranges of the source, so content outside
//! the edited ranges keeps its bytes.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::{Fields, Value};

pub use nurt_contracts::config::YamlVersion;

use super::anchor::Anchors;
use super::scalar;
use crate::constructs::{self, YAML_ALIAS_EXPANSION_LIMIT};

/// Maximum number of nodes that the expansion of aliases and merge keys adds
/// to a value, beyond the nodes written in the document.
pub const ALIAS_EXPANSION_LIMIT: usize = 1_000_000;

/// Deepest nesting of collections in an expanded value; equal to the deepest
/// nesting the parser accepts, so every document that parses can be read.
/// Aliases nest collections beyond the depth of the text, and the consumers
/// of a value (comparison, drop, serialization) recurse over its depth.
pub const MAX_EXPANSION_DEPTH: usize = 512;

/// What the expansion does next.
enum Next {
    /// Evaluate a node.
    Enter(NodeId),
    /// Hand a finished value to the collection that waits for it.
    Deliver(Value),
    /// Ask the collection on top of the stack for its next node.
    Advance,
}

/// Evaluation of one node.
enum Entered<'m> {
    /// A scalar, or a node whose value an override replaces.
    Value(Value),
    /// A collection whose members are still to be evaluated.
    Frame(Work<'m>),
}

/// Request of a collection on top of the stack.
enum Advance {
    /// Evaluate this node and deliver its value.
    Enter(NodeId),
    /// The collection is complete.
    Done(Value),
}

/// What the value delivered to a mapping becomes.
enum Awaiting {
    Nothing,
    /// The value of the entry with this key.
    Key(String),
    /// A mapping merged by a `<<` key.
    Source,
}

/// Mappings merged by one `<<` key, in order of precedence.
struct Merging {
    sources: Vec<NodeId>,
    next: usize,
}

/// A collection being expanded.
enum Work<'m> {
    List {
        id: NodeId,
        sequence: &'m SequenceNode,
        next: usize,
        items: Vec<Value>,
    },
    Map {
        id: NodeId,
        mapping: &'m MappingNode,
        next: usize,
        fields: Fields,
        local: HashSet<String>,
        merging: Option<Merging>,
        awaiting: Awaiting,
    },
}

impl Work<'_> {
    fn id(&self) -> NodeId {
        match self {
            Self::List { id, .. } | Self::Map { id, .. } => *id,
        }
    }

    /// Takes the value of the member that was requested last.
    fn accept(&mut self, value: Value) {
        match self {
            Self::List { items, .. } => items.push(value),
            Self::Map {
                fields,
                local,
                awaiting,
                ..
            } => match std::mem::replace(awaiting, Awaiting::Nothing) {
                Awaiting::Key(key) => {
                    fields.insert(key, value);
                }
                Awaiting::Source => {
                    if let Value::Map(inherited) = value {
                        for (key, inherited_value) in inherited {
                            if !local.contains(&key) && !fields.contains_key(&key) {
                                fields.insert(key, inherited_value);
                            }
                        }
                    }
                }
                Awaiting::Nothing => {}
            },
        }
    }

    /// The next node to evaluate, or the finished collection.
    fn advance(&mut self, model: &YamlModel, overrides: &ValueOverrides) -> Advance {
        match self {
            Self::List {
                id,
                sequence,
                next,
                items,
            } => loop {
                let Some(item) = sequence.items.get(*next) else {
                    if let Some(extra) = overrides.appended_items.get(id) {
                        items.extend(extra.iter().cloned());
                    }
                    return Advance::Done(Value::List(std::mem::take(items)));
                };
                let index = *next;
                *next += 1;
                if !overrides.removed_items.contains(&(*id, index)) {
                    return Advance::Enter(item.node);
                }
            },
            Self::Map {
                id,
                mapping,
                next,
                fields,
                merging,
                awaiting,
                ..
            } => loop {
                if let Some(state) = merging.as_mut() {
                    if let Some(source) = state.sources.get(state.next).copied() {
                        state.next += 1;
                        *awaiting = Awaiting::Source;
                        return Advance::Enter(source);
                    }
                    *merging = None;
                }
                let Some(entry) = mapping.entries.get(*next) else {
                    if let Some(extra) = overrides.appended_entries.get(id) {
                        for (key, value) in extra {
                            // An appended key that overrides a merged key is
                            // written last in the text, so it is read last.
                            fields.shift_remove(key);
                            fields.insert(key.clone(), value.clone());
                        }
                    }
                    return Advance::Done(Value::Map(std::mem::take(fields)));
                };
                let index = *next;
                *next += 1;
                if overrides.removed_entries.contains(&(*id, index)) {
                    continue;
                }
                if entry.merge {
                    *merging = Some(Merging {
                        sources: model.merge_sources(entry.value),
                        next: 0,
                    });
                    continue;
                }
                *awaiting = Awaiting::Key(model.key_text(entry.key).unwrap_or_default());
                return Advance::Enter(entry.value);
            },
        }
    }
}

/// Identifier of a node in a [`YamlModel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// Position of the node in the arena of the model.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_index(index: usize) -> Option<Self> {
        u32::try_from(index).ok().map(Self)
    }
}

/// Directive preceding a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    /// Range of the directive without the line break and without a trailing
    /// comment.
    pub span: Range<usize>,
    /// Content of the directive.
    pub kind: DirectiveKind,
}

/// Content of a [`Directive`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveKind {
    /// `%YAML <version>`.
    Yaml {
        /// Version as written, for example `1.1`.
        version: String,
    },
    /// `%TAG <handle> <prefix>`.
    Tag {
        /// Tag handle, for example `!e!`.
        handle: String,
        /// Tag prefix, for example `tag:example.com,2000:`.
        prefix: String,
    },
    /// Directive with a reserved name.
    Reserved {
        /// Name of the directive without `%`.
        name: String,
    },
}

/// Document of a YAML stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlDocument {
    /// Range from the first directive, marker or content byte to the end of
    /// the end marker or of the content.
    pub span: Range<usize>,
    /// Directives in source order.
    pub directives: Vec<Directive>,
    /// Range of the `---` marker.
    pub start_marker: Option<Range<usize>>,
    /// Range of the `...` marker.
    pub end_marker: Option<Range<usize>>,
    /// Root node; an empty document has an empty plain scalar as its root.
    pub root: NodeId,
    /// Version that governs the typing of plain scalars.
    pub version: YamlVersion,
}

/// Anchor or alias name with its range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Name without the `&` or `*` indicator.
    pub name: String,
    /// Range including the indicator.
    pub span: Range<usize>,
}

/// Tag of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagProperty {
    /// Tag as written, for example `!!str` or `!Ref`.
    pub text: String,
    /// Tag after resolution of its handle and percent escapes, for example
    /// `tag:yaml.org,2002:str`; `!` for the non-specific tag.
    pub resolved: String,
    /// Range of the tag as written.
    pub span: Range<usize>,
}

/// Presentation style of a scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarStyle {
    /// Unquoted scalar.
    Plain,
    /// Scalar in `'…'`.
    SingleQuoted,
    /// Scalar in `"…"`.
    DoubleQuoted,
    /// Block scalar introduced by `|`.
    Literal,
    /// Block scalar introduced by `>`.
    Folded,
}

impl ScalarStyle {
    /// Whether the style is a block scalar style.
    #[must_use]
    pub const fn is_block(self) -> bool {
        matches!(self, Self::Literal | Self::Folded)
    }
}

/// Chomping indicator of a block scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Chomping {
    /// `-`: no final line break.
    Strip,
    /// No indicator: one final line break.
    Clip,
    /// `+`: every trailing line break.
    Keep,
}

/// Header of a block scalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockHeader {
    /// Range of the indicators, from `|` or `>` to the last indicator.
    pub span: Range<usize>,
    /// Chomping indicator.
    pub chomping: Chomping,
    /// Explicit indentation indicator.
    pub indent_indicator: Option<u8>,
    /// Indentation of the content lines in spaces.
    pub content_indent: usize,
}

/// Scalar node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarNode {
    /// Presentation style.
    pub style: ScalarStyle,
    /// Value after unescaping, line folding and chomping.
    pub value: String,
    /// Header of a block scalar.
    pub block: Option<BlockHeader>,
}

/// Presentation style of a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollectionStyle {
    /// Indentation-based collection.
    Block,
    /// Collection in `[…]` or `{…}`.
    Flow,
    /// Single-pair mapping written as an entry of a flow sequence, as in
    /// `[a: 1]`.
    FlowPair,
}

/// Brackets of a flow collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowDelimiters {
    /// Range of `[` or `{`.
    pub open: Range<usize>,
    /// Range of `]` or `}`.
    pub close: Range<usize>,
}

/// Entry of a mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    /// Key node.
    pub key: NodeId,
    /// Value node; a missing value is an empty plain scalar.
    pub value: NodeId,
    /// Range of the `?` indicator of an explicit key.
    pub question: Option<Range<usize>>,
    /// Range of the `:` indicator.
    pub colon: Option<Range<usize>>,
    /// Range of the `,` that follows the entry in a flow mapping.
    pub comma: Option<Range<usize>>,
    /// Whether the key is the merge key `<<`.
    pub merge: bool,
    /// Range of the entry; in a block mapping it ends at the end of the last
    /// line of the value, including a comment on that line and excluding the
    /// line break.
    pub span: Range<usize>,
}

/// Mapping node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingNode {
    /// Presentation style.
    pub style: CollectionStyle,
    /// Brackets of a flow mapping.
    pub delimiters: Option<FlowDelimiters>,
    /// Column of the first entry, counted from zero.
    pub indent: usize,
    /// Entries in source order.
    pub entries: Vec<MapEntry>,
}

/// Item of a sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqItem {
    /// Item node.
    pub node: NodeId,
    /// Range of the `-` indicator of a block sequence item.
    pub dash: Option<Range<usize>>,
    /// Range of the `,` that follows the item in a flow sequence.
    pub comma: Option<Range<usize>>,
    /// Range of the item; in a block sequence it ends at the end of the last
    /// line of the item, including a comment on that line and excluding the
    /// line break.
    pub span: Range<usize>,
}

/// Sequence node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceNode {
    /// Presentation style; never [`CollectionStyle::FlowPair`].
    pub style: CollectionStyle,
    /// Brackets of a flow sequence.
    pub delimiters: Option<FlowDelimiters>,
    /// Column of the first `-` of a block sequence or of `[`, counted from
    /// zero.
    pub indent: usize,
    /// Items in source order.
    pub items: Vec<SeqItem>,
}

/// Content of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Scalar.
    Scalar(ScalarNode),
    /// Mapping.
    Mapping(MappingNode),
    /// Sequence.
    Sequence(SequenceNode),
    /// Alias `*name`.
    Alias(Property),
}

/// Node of a YAML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlNode {
    /// Content of the node.
    pub kind: NodeKind,
    /// Range of the content without properties; empty for an empty node.
    pub span: Range<usize>,
    /// Anchor `&name`.
    pub anchor: Option<Property>,
    /// Tag.
    pub tag: Option<TagProperty>,
    /// Parent collection; `None` for the root of a document.
    pub parent: Option<NodeId>,
    /// Index of the document in the stream.
    pub document: usize,
    /// Whether the node is inside `[…]` or `{…}`.
    pub in_flow: bool,
}

impl YamlNode {
    /// Range of the node including its anchor and tag.
    #[must_use]
    pub fn full_span(&self) -> Range<usize> {
        let mut start = self.span.start;
        if let Some(anchor) = &self.anchor {
            start = start.min(anchor.span.start);
        }
        if let Some(tag) = &self.tag {
            start = start.min(tag.span.start);
        }
        start..self.span.end
    }

    /// Scalar content, when the node is a scalar.
    #[must_use]
    pub const fn as_scalar(&self) -> Option<&ScalarNode> {
        match &self.kind {
            NodeKind::Scalar(scalar) => Some(scalar),
            _ => None,
        }
    }

    /// Mapping content, when the node is a mapping.
    #[must_use]
    pub const fn as_mapping(&self) -> Option<&MappingNode> {
        match &self.kind {
            NodeKind::Mapping(mapping) => Some(mapping),
            _ => None,
        }
    }

    /// Sequence content, when the node is a sequence.
    #[must_use]
    pub const fn as_sequence(&self) -> Option<&SequenceNode> {
        match &self.kind {
            NodeKind::Sequence(sequence) => Some(sequence),
            _ => None,
        }
    }

    /// Whether the node is the empty node written as nothing at all.
    #[must_use]
    pub fn is_empty_node(&self) -> bool {
        self.span.is_empty() && self.as_scalar().is_some()
    }

    /// Whether the node is a block mapping or block sequence.
    #[must_use]
    pub fn is_block_collection(&self) -> bool {
        match &self.kind {
            NodeKind::Mapping(mapping) => mapping.style == CollectionStyle::Block,
            NodeKind::Sequence(sequence) => sequence.style == CollectionStyle::Block,
            _ => false,
        }
    }
}

/// Line break style of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineEnding {
    /// `\n`.
    #[default]
    Lf,
    /// `\r\n`.
    CrLf,
}

impl LineEnding {
    /// Text of the line break.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// Changes applied to the value of a model without changing its text.
///
/// The guard of every edit compares the value read back from the new text
/// with the value of the old model under these overrides; aliases of a
/// changed anchored node therefore see the change as well.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValueOverrides {
    /// Nodes whose value is replaced.
    pub replaced: HashMap<NodeId, Value>,
    /// Entries appended to mappings.
    pub appended_entries: HashMap<NodeId, Vec<(String, Value)>>,
    /// Items appended to sequences.
    pub appended_items: HashMap<NodeId, Vec<Value>>,
    /// Removed entries of mappings, as pairs of mapping and entry index.
    pub removed_entries: HashSet<(NodeId, usize)>,
    /// Removed items of sequences, as pairs of sequence and item index.
    pub removed_items: HashSet<(NodeId, usize)>,
}

impl ValueOverrides {
    /// Whether no override is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.replaced.is_empty()
            && self.appended_entries.is_empty()
            && self.appended_items.is_empty()
            && self.removed_entries.is_empty()
            && self.removed_items.is_empty()
    }
}

/// Anchor resolution of a model, computed on first use. Two models are equal
/// whether or not the resolution was computed.
#[derive(Debug, Clone, Default)]
pub(crate) struct AnchorCache(OnceLock<Anchors>);

impl PartialEq for AnchorCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for AnchorCache {}

/// Lossless model of a YAML stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlModel {
    pub(crate) anchor_cache: AnchorCache,
    pub(crate) source: String,
    pub(crate) bom: bool,
    pub(crate) documents: Vec<YamlDocument>,
    pub(crate) nodes: Vec<YamlNode>,
    pub(crate) comments: Vec<Range<usize>>,
    pub(crate) line_starts: Vec<usize>,
    pub(crate) alias_targets: HashMap<NodeId, NodeId>,
}

impl YamlModel {
    /// Targets of the aliases of the stream, resolved once per model.
    #[must_use]
    pub fn anchors(&self) -> &Anchors {
        self.anchor_cache.0.get_or_init(|| Anchors::new(self))
    }

    /// Source text, including a byte order mark.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether the source starts with a byte order mark.
    #[must_use]
    pub const fn has_bom(&self) -> bool {
        self.bom
    }

    /// Documents in stream order.
    #[must_use]
    pub fn documents(&self) -> &[YamlDocument] {
        &self.documents
    }

    /// Node with the given identifier.
    ///
    /// # Panics
    ///
    /// Panics when the identifier belongs to another model.
    #[must_use]
    pub fn node(&self, id: NodeId) -> &YamlNode {
        &self.nodes[id.index()]
    }

    /// Every node, parents before their children.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len()).filter_map(NodeId::from_index)
    }

    /// Ranges of every comment, from `#` to the end of the line, in source
    /// order.
    #[must_use]
    pub fn comments(&self) -> &[Range<usize>] {
        &self.comments
    }

    /// Parent collection of a node.
    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).parent
    }

    /// Text of a range of the source.
    ///
    /// # Panics
    ///
    /// Panics when the range is outside the source or not on character
    /// boundaries.
    #[must_use]
    pub fn text(&self, span: &Range<usize>) -> &str {
        &self.source[span.clone()]
    }

    /// One-based line and column of a byte offset; the column counts Unicode
    /// scalar values.
    #[must_use]
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let mut start = self.line_starts[line.saturating_sub(1)];
        if start == 0 && self.bom {
            start = '\u{FEFF}'.len_utf8().min(offset);
        }
        let end = offset.min(self.source.len());
        let column = self
            .source
            .get(start..end)
            .map_or(0, |text| text.chars().count());
        (line, column + 1)
    }

    /// Column of a byte offset counted from zero in Unicode scalar values.
    #[must_use]
    pub fn column(&self, offset: usize) -> usize {
        self.line_col(offset).1 - 1
    }

    /// Byte offset of the start of the line that contains `offset`.
    #[must_use]
    pub fn line_start(&self, offset: usize) -> usize {
        let line = self.line_starts.partition_point(|&start| start <= offset);
        self.line_starts[line.saturating_sub(1)]
    }

    /// Byte offset of the line break that ends the line containing `offset`,
    /// or the length of the source on the last line.
    #[must_use]
    pub fn line_end(&self, offset: usize) -> usize {
        let bytes = self.source.as_bytes();
        let mut end = offset.min(bytes.len());
        while end < bytes.len() && bytes[end] != b'\n' && bytes[end] != b'\r' {
            end += 1;
        }
        end
    }

    /// Byte offset immediately after the line break that ends the line containing
    /// `offset`, or the length of the source on the last line.
    #[must_use]
    pub fn next_line_start(&self, offset: usize) -> usize {
        let bytes = self.source.as_bytes();
        let end = self.line_end(offset);
        match bytes.get(end) {
            Some(b'\r') if bytes.get(end + 1) == Some(&b'\n') => end + 2,
            Some(_) => end + 1,
            None => end,
        }
    }

    /// Indentation in spaces of the line that contains `offset`.
    #[must_use]
    pub fn line_indent(&self, offset: usize) -> usize {
        let start = self.line_start(offset);
        self.source.as_bytes()[start..]
            .iter()
            .take_while(|&&byte| byte == b' ')
            .count()
    }

    /// Whether only spaces and tabs precede `offset` on its line.
    #[must_use]
    pub fn starts_line(&self, offset: usize) -> bool {
        let start = self.line_start(offset);
        let skip = if start == 0 && self.bom {
            '\u{FEFF}'.len_utf8()
        } else {
            0
        };
        self.source.as_bytes()[(start + skip).min(offset)..offset]
            .iter()
            .all(|&byte| byte == b' ' || byte == b'\t')
    }

    /// Dominant line break of the source; `\n` when the source has none.
    #[must_use]
    pub fn line_ending(&self) -> LineEnding {
        let bytes = self.source.as_bytes();
        let crlf = bytes.windows(2).filter(|pair| pair == b"\r\n").count();
        let lf = bytes.iter().filter(|&&byte| byte == b'\n').count();
        if crlf * 2 > lf {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        }
    }

    /// Node defined by the anchor that an alias refers to: the closest
    /// preceding node of the same document with that anchor name.
    #[must_use]
    pub fn anchor_target(&self, alias: NodeId) -> Option<NodeId> {
        self.alias_targets.get(&alias).copied()
    }

    /// Node itself, or the target of an alias.
    #[must_use]
    pub fn dealias(&self, id: NodeId) -> NodeId {
        self.anchor_target(id).unwrap_or(id)
    }

    /// Index of the last entry of a mapping whose key has the given text,
    /// without merge keys; aliases used as keys are followed.
    #[must_use]
    pub fn entry_index(&self, map: NodeId, key: &str) -> Option<usize> {
        self.node(map)
            .as_mapping()?
            .entries
            .iter()
            .rposition(|entry| !entry.merge && self.key_text(entry.key).as_deref() == Some(key))
    }

    /// Value of the last entry with the given key text in a mapping, without
    /// merge keys.
    #[must_use]
    pub fn map_get(&self, map: NodeId, key: &str) -> Option<NodeId> {
        let index = self.entry_index(map, key)?;
        self.node(map)
            .as_mapping()
            .map(|mapping| mapping.entries[index].value)
    }

    /// Mapping entry whose value is the given node.
    #[must_use]
    pub fn entry_of(&self, value: NodeId) -> Option<&MapEntry> {
        self.entry_position(value).map(|(_, entry)| entry)
    }

    /// Mapping entry whose value is the given node, with its index in the
    /// mapping.
    #[must_use]
    pub fn entry_position(&self, value: NodeId) -> Option<(usize, &MapEntry)> {
        let parent = self.parent(value)?;
        self.node(parent)
            .as_mapping()?
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.value == value)
    }

    /// Sequence item whose node is the given node, with its index.
    #[must_use]
    pub fn item_of(&self, node: NodeId) -> Option<(usize, &SeqItem)> {
        let parent = self.parent(node)?;
        self.node(parent)
            .as_sequence()?
            .items
            .iter()
            .enumerate()
            .find(|(_, item)| item.node == node)
    }

    /// Text of a key: the value of a scalar key, the value of the target of
    /// an alias key, or the source text of a collection key.
    #[must_use]
    pub fn key_text(&self, key: NodeId) -> Option<String> {
        let node = self.node(self.dealias(key));
        match &node.kind {
            NodeKind::Scalar(scalar) => Some(scalar.value.clone()),
            NodeKind::Alias(_) => None,
            NodeKind::Mapping(_) | NodeKind::Sequence(_) => Some(self.text(&node.span).to_owned()),
        }
    }

    /// Version that governs the typing of the node.
    #[must_use]
    pub fn version_of(&self, id: NodeId) -> YamlVersion {
        self.documents
            .get(self.node(id).document)
            .map_or_else(YamlVersion::default, |document| document.version)
    }

    /// Value of a node with aliases expanded and merge keys applied.
    ///
    /// # Errors
    ///
    /// `E_UNSUPPORTED_CONSTRUCT` naming `yaml.alias_expansion_limit` when the
    /// expansion of aliases adds more than [`ALIAS_EXPANSION_LIMIT`] nodes;
    /// `E_DOC_PARSE` when an alias refers to an undefined anchor.
    pub fn value(&self, id: NodeId) -> Result<Value> {
        self.value_with(id, &ValueOverrides::default())
    }

    /// Value of a document root; `null` for an index past the last document.
    ///
    /// # Errors
    ///
    /// As for [`YamlModel::value`].
    pub fn document_value(&self, index: usize) -> Result<Value> {
        self.documents
            .get(index)
            .map_or(Ok(Value::Null), |document| self.value(document.root))
    }

    /// Value of a node with the given overrides applied.
    ///
    /// # Errors
    ///
    /// As for [`YamlModel::value`].
    pub fn value_with(&self, id: NodeId, overrides: &ValueOverrides) -> Result<Value> {
        let mut budget = self.nodes.len().saturating_add(ALIAS_EXPANSION_LIMIT);
        self.expand(id, overrides, &mut budget)
    }

    /// Value of a node, computed with an explicit work stack: a collection
    /// is a frame on the stack, so the depth of a document never reaches the
    /// call stack.
    fn expand(&self, root: NodeId, overrides: &ValueOverrides, budget: &mut usize) -> Result<Value> {
        let mut stack: Vec<Work<'_>> = Vec::new();
        let mut next = Next::Enter(root);
        loop {
            next = match next {
                Next::Enter(node) => match self.enter(node, overrides, budget, &stack)? {
                    Entered::Value(value) => Next::Deliver(value),
                    Entered::Frame(frame) => {
                        stack.push(frame);
                        Next::Advance
                    }
                },
                Next::Deliver(value) => match stack.last_mut() {
                    None => return Ok(value),
                    Some(top) => {
                        top.accept(value);
                        Next::Advance
                    }
                },
                Next::Advance => {
                    let Some(top) = stack.last_mut() else {
                        return Err(Error::new(
                            ErrorCode::Internal,
                            "the expansion of a value lost its work stack",
                        ));
                    };
                    match top.advance(self, overrides) {
                        Advance::Enter(node) => Next::Enter(node),
                        Advance::Done(value) => {
                            stack.pop();
                            Next::Deliver(value)
                        }
                    }
                }
            };
        }
    }

    /// Value of a scalar or the frame of a collection; aliases are followed.
    fn enter<'m>(
        &'m self,
        mut id: NodeId,
        overrides: &ValueOverrides,
        budget: &mut usize,
        stack: &[Work<'m>],
    ) -> Result<Entered<'m>> {
        loop {
            if *budget == 0 {
                return Err(constructs::unsupported(
                    YAML_ALIAS_EXPANSION_LIMIT,
                    format!(
                        "the expansion of aliases adds more than {ALIAS_EXPANSION_LIMIT} nodes"
                    ),
                ));
            }
            *budget -= 1;
            if let Some(value) = overrides.replaced.get(&id) {
                return Ok(Entered::Value(value.clone()));
            }
            let node = self.node(id);
            let depth_error = || {
                constructs::unsupported(
                    YAML_ALIAS_EXPANSION_LIMIT,
                    format!(
                        "the expansion of aliases nests collections deeper than {MAX_EXPANSION_DEPTH} levels"
                    ),
                )
            };
            match &node.kind {
                NodeKind::Scalar(content) => {
                    return Ok(Entered::Value(scalar::scalar_value(
                        content,
                        node.tag.as_ref(),
                        self.version_of(id),
                    )));
                }
                NodeKind::Alias(name) => {
                    let target = self.anchor_target(id).ok_or_else(|| {
                        Error::new(
                            ErrorCode::DocParse,
                            format!("alias `*{}` refers to an undefined anchor", name.name),
                        )
                    })?;
                    if stack.iter().any(|work| work.id() == target) {
                        return Err(Error::new(
                            ErrorCode::DocParse,
                            format!("alias `*{}` refers to its own ancestor", name.name),
                        ));
                    }
                    id = target;
                }
                NodeKind::Sequence(sequence) => {
                    if stack.len() >= MAX_EXPANSION_DEPTH {
                        return Err(depth_error());
                    }
                    return Ok(Entered::Frame(Work::List {
                        id,
                        sequence,
                        next: 0,
                        items: Vec::with_capacity(sequence.items.len()),
                    }));
                }
                NodeKind::Mapping(mapping) => {
                    if stack.len() >= MAX_EXPANSION_DEPTH {
                        return Err(depth_error());
                    }
                    let live = |index: &usize| !overrides.removed_entries.contains(&(id, *index));
                    let local: HashSet<String> = mapping
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(index, entry)| live(index) && !entry.merge)
                        .filter_map(|(_, entry)| self.key_text(entry.key))
                        .collect();
                    return Ok(Entered::Frame(Work::Map {
                        id,
                        mapping,
                        next: 0,
                        fields: Fields::new(),
                        local,
                        merging: None,
                        awaiting: Awaiting::Nothing,
                    }));
                }
            }
        }
    }

    /// Mappings merged by a `<<` value: an alias or mapping, or a sequence of
    /// them, in order of precedence.
    #[must_use]
    pub fn merge_sources(&self, value: NodeId) -> Vec<NodeId> {
        let node = self.dealias(value);
        match &self.node(node).kind {
            NodeKind::Mapping(_) => vec![node],
            NodeKind::Sequence(sequence) => sequence
                .items
                .iter()
                .map(|item| self.dealias(item.node))
                .filter(|&source| self.node(source).as_mapping().is_some())
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn push_node(&mut self, node: YamlNode) -> NodeId {
        let id = NodeId::from_index(self.nodes.len()).unwrap_or(NodeId(u32::MAX));
        self.nodes.push(node);
        id
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> &mut YamlNode {
        &mut self.nodes[id.index()]
    }
}
