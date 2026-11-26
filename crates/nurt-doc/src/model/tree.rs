// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::collections::HashMap;
use std::ops::Index;

use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::services::{TextPosition, TextSpan};
use nurt_contracts::value::{Fields, Value};

use super::semantics::{FormatSemantics, Repeated};
use super::{LineIndex, Span};

/// Longest chain of aliases followed before a node is treated as having no
/// target; guards against cycles in malformed documents.
const MAX_ALIAS_CHAIN: usize = 64;

/// Kind of a document node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    /// Single value: text, number, boolean, null or date.
    Scalar,
    /// Members addressed by name.
    Map,
    /// Elements addressed by index.
    List,
    /// XML element: attributes, text and child elements addressed by name.
    Element,
    /// XML attribute.
    Attribute,
    /// Text content of an XML element.
    Text,
    /// Comment; never selected by a key path.
    Comment,
}

impl NodeKind {
    /// Returns `true` for maps and elements, whose children have names.
    #[must_use]
    pub const fn is_named_container(self) -> bool {
        matches!(self, Self::Map | Self::Element)
    }

    /// Returns `true` for maps, lists and elements.
    #[must_use]
    pub const fn is_container(self) -> bool {
        matches!(self, Self::Map | Self::List | Self::Element)
    }

    /// Name of the kind in errors, matching the `value_type` names of
    /// records where one exists.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Map => "map",
            Self::List => "list",
            Self::Element => "element",
            Self::Attribute => "attribute",
            Self::Text => "text",
            Self::Comment => "comment",
        }
    }
}

/// How a scalar is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ScalarStyle {
    /// Without quotes.
    #[default]
    Plain,
    /// In single quotes.
    SingleQuoted,
    /// In double quotes.
    DoubleQuoted,
    /// YAML literal block (`|`).
    Literal,
    /// YAML folded block (`>`).
    Folded,
}

impl ScalarStyle {
    /// Quote character of a quoted style.
    #[must_use]
    pub const fn quote(self) -> Option<char> {
        match self {
            Self::SingleQuoted => Some('\''),
            Self::DoubleQuoted => Some('"'),
            Self::Plain | Self::Literal | Self::Folded => None,
        }
    }
}

/// How a map or list is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Layout {
    /// One member per line, structure given by indentation or headers.
    #[default]
    Block,
    /// Inline with brackets and separators (`{a: 1}`, `[1, 2]`, JSON).
    Flow,
}

/// Layout description of a node, used to write new content in the same
/// style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    /// Scalar style.
    pub scalar: ScalarStyle,
    /// Container layout.
    pub layout: Layout,
    /// Indentation of the node in columns, when the format has indentation.
    pub indent: Option<usize>,
}

/// Identifier of a node within its [`DocTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// Position of the node in the tree storage.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Description of a node passed to [`TreeBuilder::node`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeSpec {
    /// Kind; scalar when not set.
    pub kind: Option<NodeKind>,
    /// Name under which the parent holds the node (map key, element or
    /// attribute name; attributes are named `@name`, text nodes `#text`).
    pub name: Option<String>,
    /// The whole node with its key and key separator.
    pub span: Span,
    /// The key, when the node has one in the text.
    pub key_span: Option<Span>,
    /// The value; equal to `span` when not set.
    pub value_span: Option<Span>,
    /// Value of a scalar, attribute or text node.
    pub value: Option<Value>,
    /// Comment attached to the node, without comment markers.
    pub comment: Option<String>,
    /// Layout of the node.
    pub style: Style,
    /// Anchor defined on the node (YAML).
    pub anchor: Option<String>,
    /// Explicit tag (YAML).
    pub tag: Option<String>,
}

impl NodeSpec {
    /// Node of `kind` covering `span`.
    #[must_use]
    pub fn new(kind: NodeKind, span: Span) -> Self {
        Self {
            kind: Some(kind),
            span,
            ..Self::default()
        }
    }

    /// Sets the name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the key span.
    #[must_use]
    pub const fn with_key_span(mut self, span: Span) -> Self {
        self.key_span = Some(span);
        self
    }

    /// Sets the value span.
    #[must_use]
    pub const fn with_value_span(mut self, span: Span) -> Self {
        self.value_span = Some(span);
        self
    }

    /// Sets the scalar value.
    #[must_use]
    pub fn with_value(mut self, value: Value) -> Self {
        self.value = Some(value);
        self
    }

    /// Sets the attached comment.
    #[must_use]
    pub fn with_comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Sets the style.
    #[must_use]
    pub const fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Sets the anchor.
    #[must_use]
    pub fn with_anchor(mut self, anchor: impl Into<String>) -> Self {
        self.anchor = Some(anchor.into());
        self
    }

    /// Sets the tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

/// Node of a [`DocTree`].
#[derive(Debug, Clone, PartialEq)]
pub struct DocNode {
    /// Kind.
    pub kind: NodeKind,
    /// Name under which the parent holds the node.
    pub name: Option<String>,
    /// The whole node with its key and key separator.
    pub span: Span,
    /// The key.
    pub key_span: Option<Span>,
    /// The value.
    pub value_span: Span,
    /// Position of the first character of the value.
    pub start: TextPosition,
    /// Position after the last character of the value.
    pub end: TextPosition,
    /// Value of a scalar, attribute or text node.
    pub value: Option<Value>,
    /// Comment attached to the node.
    pub comment: Option<String>,
    /// Layout.
    pub style: Style,
    /// Anchor defined on the node.
    pub anchor: Option<String>,
    /// Node this node is an alias of.
    pub alias_of: Option<NodeId>,
    /// Explicit tag.
    pub tag: Option<String>,
    /// Parent; `None` for document roots.
    pub parent: Option<NodeId>,
    /// Children in document order.
    pub children: Vec<NodeId>,
    /// Position of the node among the selectable members of its parent.
    pub(crate) member_slot: usize,
    /// Number of selectable members among the children added so far.
    pub(crate) member_total: usize,
    /// Position of the node among the selectable siblings that share its name
    /// and the number of such siblings, set when the tree is finished.
    pub(crate) same_name: (usize, usize),
}

/// Builder of a [`DocTree`]; nodes are added parent first.
#[derive(Debug)]
pub struct TreeBuilder<'a> {
    text: &'a [u8],
    nodes: Vec<DocNode>,
    roots: Vec<NodeId>,
}

impl<'a> TreeBuilder<'a> {
    /// Builder for a tree over `text`.
    #[must_use]
    pub const fn new(text: &'a [u8]) -> Self {
        Self {
            text,
            nodes: Vec::new(),
            roots: Vec::new(),
        }
    }

    /// Adds a node as the last child of `parent`, or as the root of the next
    /// document when `parent` is `None`.
    ///
    /// # Panics
    ///
    /// Panics when `parent` was not returned by this builder.
    pub fn node(&mut self, parent: Option<NodeId>, spec: NodeSpec) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        let value_span = spec.value_span.unwrap_or(spec.span);
        let kind = spec.kind.unwrap_or(NodeKind::Scalar);
        let member_slot = parent.map_or(0, |parent| {
            let holder = &mut self.nodes[parent.index()];
            let slot = holder.member_total;
            if kind != NodeKind::Comment {
                holder.member_total += 1;
            }
            slot
        });
        self.nodes.push(DocNode {
            kind,
            name: spec.name,
            span: spec.span,
            key_span: spec.key_span,
            value_span,
            start: TextPosition { line: 1, column: 1 },
            end: TextPosition { line: 1, column: 1 },
            value: spec.value,
            comment: spec.comment,
            style: spec.style,
            anchor: spec.anchor,
            alias_of: None,
            tag: spec.tag,
            parent,
            children: Vec::new(),
            member_slot,
            member_total: 0,
            same_name: (0, 1),
        });
        match parent {
            Some(parent) => self.nodes[parent.index()].children.push(id),
            None => self.roots.push(id),
        }
        id
    }

    /// Marks `node` as an alias of `target`.
    ///
    /// # Panics
    ///
    /// Panics when `node` was not returned by this builder.
    pub fn set_alias(&mut self, node: NodeId, target: NodeId) {
        self.nodes[node.index()].alias_of = Some(target);
    }

    /// Node added earlier, for adjusting spans after its children are known.
    ///
    /// # Panics
    ///
    /// Panics when `node` was not returned by this builder.
    pub fn node_mut(&mut self, node: NodeId) -> &mut DocNode {
        &mut self.nodes[node.index()]
    }

    /// Computes line and column positions and returns the tree.
    #[must_use]
    pub fn finish(mut self) -> DocTree {
        let lines = LineIndex::new(self.text);
        for node in &mut self.nodes {
            node.start = lines.position(self.text, node.value_span.start);
            node.end = lines.position(self.text, node.value_span.end);
        }
        for parent in 0..self.nodes.len() {
            let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
            for child in &self.nodes[parent].children {
                let node = &self.nodes[child.index()];
                if node.kind != NodeKind::Comment
                    && let Some(name) = node.name.as_deref()
                {
                    groups.entry(name).or_default().push(child.index());
                }
            }
            let ranks: Vec<(usize, (usize, usize))> = groups
                .values()
                .flat_map(|group| {
                    group
                        .iter()
                        .enumerate()
                        .map(|(rank, index)| (*index, (rank, group.len())))
                })
                .collect();
            for (index, same_name) in ranks {
                self.nodes[index].same_name = same_name;
            }
        }
        DocTree {
            nodes: self.nodes,
            roots: self.roots,
            lines,
        }
    }
}

/// Tree of the nodes of a document; one root per document of the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocTree {
    nodes: Vec<DocNode>,
    roots: Vec<NodeId>,
    lines: LineIndex,
}

impl Index<NodeId> for DocTree {
    type Output = DocNode;

    /// # Panics
    ///
    /// Panics when `id` belongs to another tree and is out of range.
    fn index(&self, id: NodeId) -> &DocNode {
        &self.nodes[id.index()]
    }
}

impl DocTree {
    /// Node with the identifier, `None` for an identifier of another tree
    /// that is out of range.
    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&DocNode> {
        self.nodes.get(id.index())
    }

    /// Roots of the documents in file order.
    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Returns `true` for a tree without nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Every node with its identifier, in creation order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &DocNode)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (NodeId(u32::try_from(index).unwrap_or(u32::MAX)), node))
    }

    /// Line index of the text the tree was built from.
    #[must_use]
    pub const fn lines(&self) -> &LineIndex {
        &self.lines
    }

    /// Region of `span` in `text` (the text the tree was built from).
    #[must_use]
    pub fn text_span(&self, text: &[u8], span: Span) -> TextSpan {
        self.lines.text_span(text, span)
    }

    /// The node an alias refers to, following chains of aliases; the node
    /// itself when it is not an alias. A cyclic chain yields the last node
    /// reached.
    #[must_use]
    pub fn target(&self, id: NodeId) -> NodeId {
        let mut current = id;
        for _ in 0..MAX_ALIAS_CHAIN {
            match self.get(current).and_then(|node| node.alias_of) {
                Some(next) => current = next,
                None => return current,
            }
        }
        current
    }

    /// Children that can be selected by a key path (all except comments).
    pub fn members(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.get(id)
            .map(|node| node.children.as_slice())
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|child| self.nodes[child.index()].kind != NodeKind::Comment)
    }

    /// Zero-based index of the document whose root contains `id`.
    #[must_use]
    pub fn document_of(&self, id: NodeId) -> Option<u32> {
        let mut current = id;
        while let Some(parent) = self.get(current)?.parent {
            current = parent;
        }
        let position = self.roots.iter().position(|root| *root == current)?;
        u32::try_from(position).ok()
    }

    /// Concrete key path of the node in canonical form.
    ///
    /// Members of maps and elements are addressed by name and list elements
    /// by index. Under [`Repeated::Conflict`] a name shared by several
    /// siblings gets the index among them (`dep[2]`); flat formats address a
    /// member by its dotted name.
    #[must_use]
    pub fn path_of(&self, id: NodeId, semantics: &dyn FormatSemantics) -> KeyPath {
        let mut segments = Vec::new();
        let mut current = id;
        while let Some(parent) = self.get(current).and_then(|node| node.parent) {
            self.push_segments(parent, current, semantics, &mut segments);
            current = parent;
        }
        segments.reverse();
        let document = self
            .roots
            .iter()
            .position(|root| *root == current)
            .and_then(|position| u32::try_from(position).ok())
            .map(DocSelector::Index);
        KeyPath::new(document, segments).unwrap_or_default()
    }

    /// Pushes, in reverse order, the segments that select `child` in
    /// `parent`.
    fn push_segments(
        &self,
        parent: NodeId,
        child: NodeId,
        semantics: &dyn FormatSemantics,
        reversed: &mut Vec<KeySegment>,
    ) {
        let parent_node = &self[parent];
        let name = self[child].name.as_deref();
        match (parent_node.kind, name) {
            (NodeKind::List, _) | (_, None) => {
                reversed.push(KeySegment::Index(to_index(self[child].member_slot)));
            }
            (_, Some(name)) => {
                if semantics.repeated() == Repeated::Conflict {
                    let (position, total) = self[child].same_name;
                    if total > 1 {
                        reversed.push(KeySegment::Index(to_index(position)));
                    }
                }
                if semantics.flat_keys() && name.split('.').all(|part| !part.is_empty()) {
                    reversed.extend(
                        name.rsplit('.')
                            .map(|part| KeySegment::Key(part.to_owned())),
                    );
                } else {
                    reversed.push(KeySegment::Key(name.to_owned()));
                }
            }
        }
    }

    /// Value of the node and its descendants.
    ///
    /// Aliases yield the value of their target. Members sharing a name keep
    /// the last value under [`Repeated::LastWins`] and become a list under
    /// [`Repeated::Conflict`]. `depth` limits the levels of maps and lists
    /// expanded; containers below the limit are returned empty.
    #[must_use]
    pub fn value_of(
        &self,
        id: NodeId,
        semantics: &dyn FormatSemantics,
        depth: Option<u32>,
    ) -> Value {
        self.value_at(id, semantics, depth, 0)
    }

    fn value_at(
        &self,
        id: NodeId,
        semantics: &dyn FormatSemantics,
        depth: Option<u32>,
        chain: usize,
    ) -> Value {
        let target = self.target(id);
        let Some(node) = self.get(target) else {
            return Value::Null;
        };
        let chain = chain + usize::from(target != id);
        if chain > MAX_ALIAS_CHAIN {
            return Value::Null;
        }
        let expand = depth.is_none_or(|limit| limit > 0);
        let inner = depth.map(|limit| limit.saturating_sub(1));
        match node.kind {
            NodeKind::List => {
                let items = if expand {
                    self.members(target)
                        .map(|member| self.value_at(member, semantics, inner, chain))
                        .collect()
                } else {
                    Vec::new()
                };
                Value::List(items)
            }
            NodeKind::Map | NodeKind::Element => {
                if expand {
                    Value::Map(self.map_value(target, semantics, inner, chain))
                } else {
                    Value::Map(Fields::new())
                }
            }
            NodeKind::Scalar | NodeKind::Attribute | NodeKind::Text | NodeKind::Comment => {
                node.value.clone().unwrap_or_default()
            }
        }
    }

    fn map_value(
        &self,
        id: NodeId,
        semantics: &dyn FormatSemantics,
        depth: Option<u32>,
        chain: usize,
    ) -> Fields {
        let mut fields = Fields::new();
        let mut repeated: Vec<String> = Vec::new();
        for (position, member) in self.members(id).enumerate() {
            let name = self[member]
                .name
                .clone()
                .unwrap_or_else(|| position.to_string());
            let value = self.value_at(member, semantics, depth, chain);
            match (semantics.repeated(), fields.get_mut(&name)) {
                (Repeated::Conflict, Some(existing)) => {
                    if repeated.contains(&name) {
                        if let Value::List(items) = existing {
                            items.push(value);
                        }
                    } else {
                        let first = std::mem::take(existing);
                        *existing = Value::List(vec![first, value]);
                        repeated.push(name);
                    }
                }
                _ => {
                    fields.insert(name, value);
                }
            }
        }
        fields
    }
}

fn to_index(position: usize) -> i64 {
    i64::try_from(position).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MapSemantics;

    fn spec(kind: NodeKind) -> NodeSpec {
        NodeSpec::new(kind, Span::new(0, 0))
    }

    #[test]
    fn value_of_keeps_documents_nested_deeper_than_the_alias_chain_limit() {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::Map));
        let mut parent = root;
        for _ in 0..(MAX_ALIAS_CHAIN * 2) {
            parent = builder.node(Some(parent), spec(NodeKind::Map).with_name("k"));
        }
        builder.node(
            Some(parent),
            spec(NodeKind::Scalar)
                .with_name("leaf")
                .with_value(Value::Int(1)),
        );
        let tree = builder.finish();
        let value = tree.value_of(root, &MapSemantics::LAST_WINS, None);
        let mut current = &value;
        for _ in 0..(MAX_ALIAS_CHAIN * 2) {
            current = current.get("k").unwrap();
        }
        assert_eq!(current.get("leaf"), Some(&Value::Int(1)));
    }

    #[test]
    fn value_of_follows_aliases_and_stops_at_a_cycle() {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::Map));
        let target = builder.node(
            Some(root),
            spec(NodeKind::Scalar)
                .with_name("a")
                .with_value(Value::Int(7)),
        );
        let alias = builder.node(Some(root), spec(NodeKind::Scalar).with_name("b"));
        builder.set_alias(alias, target);
        let first = builder.node(Some(root), spec(NodeKind::Scalar).with_name("c"));
        let second = builder.node(Some(root), spec(NodeKind::Scalar).with_name("d"));
        builder.set_alias(first, second);
        builder.set_alias(second, first);
        let tree = builder.finish();
        let value = tree.value_of(root, &MapSemantics::LAST_WINS, None);
        assert_eq!(value.get("b"), Some(&Value::Int(7)));
        assert!(value.get("c").is_some());
    }

    #[test]
    fn repeated_names_form_a_list_only_under_conflict_semantics() {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::Map));
        for number in [1, 2] {
            builder.node(
                Some(root),
                spec(NodeKind::Scalar)
                    .with_name("dep")
                    .with_value(Value::Int(number)),
            );
        }
        let tree = builder.finish();
        let last = tree.value_of(root, &MapSemantics::LAST_WINS, None);
        assert_eq!(last.get("dep"), Some(&Value::Int(2)));
        let group = tree.value_of(root, &MapSemantics::CONFLICT, None);
        assert_eq!(
            group.get("dep"),
            Some(&Value::List(vec![Value::Int(1), Value::Int(2)]))
        );
    }

    #[test]
    fn depth_limit_returns_empty_containers_below_it() {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::Map));
        let inner = builder.node(Some(root), spec(NodeKind::List).with_name("l"));
        builder.node(
            Some(inner),
            spec(NodeKind::Scalar).with_value(Value::Int(1)),
        );
        let tree = builder.finish();
        let value = tree.value_of(root, &MapSemantics::LAST_WINS, Some(1));
        assert_eq!(value.get("l"), Some(&Value::List(Vec::new())));
    }

    #[test]
    fn list_positions_skip_comments_and_do_not_depend_on_the_length() {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::List));
        builder.node(Some(root), spec(NodeKind::Comment));
        let mut last = root;
        for _ in 0..3000 {
            last = builder.node(Some(root), spec(NodeKind::Scalar));
        }
        let tree = builder.finish();
        let semantics = MapSemantics::LAST_WINS;
        assert_eq!(tree.path_of(last, &semantics).to_string(), "[2999]");
        let first = tree.members(root).next().unwrap();
        assert_eq!(tree.path_of(first, &semantics).to_string(), "[0]");
    }

    #[test]
    fn path_of_names_members_and_indexes_repeated_names() {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::Map));
        let first = builder.node(Some(root), spec(NodeKind::Scalar).with_name("dep"));
        let second = builder.node(Some(root), spec(NodeKind::Scalar).with_name("dep"));
        let tree = builder.finish();
        assert_eq!(
            tree.path_of(second, &MapSemantics::CONFLICT).to_string(),
            "dep[1]"
        );
        assert_eq!(
            tree.path_of(first, &MapSemantics::LAST_WINS).to_string(),
            "dep"
        );
        assert_eq!(tree.document_of(second), Some(0));
    }

    /// Time of the paths of all members of a container with `count` members
    /// that share one name, under the semantics that index repeated names.
    fn conflict_paths_time(count: usize) -> std::time::Duration {
        let mut builder = TreeBuilder::new(b"");
        let root = builder.node(None, spec(NodeKind::Map));
        let ids: Vec<NodeId> = (0..count)
            .map(|_| builder.node(Some(root), spec(NodeKind::Scalar).with_name("dep")))
            .collect();
        let tree = builder.finish();
        let start = std::time::Instant::now();
        for (position, id) in ids.iter().enumerate() {
            assert_eq!(
                tree.path_of(*id, &MapSemantics::CONFLICT).to_string(),
                format!("dep[{position}]")
            );
        }
        start.elapsed()
    }

    #[test]
    fn the_paths_of_repeated_names_cost_time_proportional_to_their_number() {
        let fastest = |count: usize| (0..3).map(|_| conflict_paths_time(count)).min().unwrap();
        let small = fastest(2_000);
        let large = fastest(8_000);
        let ratio = large.as_secs_f64() / small.as_secs_f64().max(1e-4);
        assert!(
            ratio < 9.0,
            "{small:?} for 2000, {large:?} for 8000: ratio {ratio:.1}"
        );
    }
}
