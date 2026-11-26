// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lossless XML tree: nodes with byte ranges over the unchanged source
//! text, and resolution of key paths to nodes.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};

use super::attr::Attribute;
use super::cdata::{EntitySource, ResolverSlot, trim_xml};
use super::style::XmlStyle;
use super::{key_conflict, key_not_found};

/// Key segment that addresses the text chunks of an element.
pub const TEXT_SEGMENT: &str = "#text";

/// Prefix of key segments that address attributes.
pub const ATTRIBUTE_SEGMENT_PREFIX: char = '@';

/// Most candidate keys listed in the error of an ambiguous key.
const MAX_CANDIDATES: usize = 8;

/// Identifier of a node in an [`XmlTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) usize);

impl NodeId {
    /// Position of the node in document order.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// XML declaration `<?xml … ?>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Whole declaration.
    pub span: Range<usize>,
    /// Pseudo-attributes `version`, `encoding` and `standalone`.
    pub attributes: Vec<Attribute>,
}

/// Document type declaration `<!DOCTYPE … >`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocType {
    /// Whole declaration.
    pub span: Range<usize>,
    /// Name of the document element.
    pub name: Range<usize>,
    /// Internal subset between `[` and `]`, without the brackets.
    pub internal_subset: Option<Range<usize>>,
}

/// Element with its tags, attributes and children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// From the start of the start tag to the end of the end tag; the
    /// empty-element tag for `<a/>`.
    pub span: Range<usize>,
    /// Start tag or empty-element tag.
    pub start_tag: Range<usize>,
    /// Qualified name in the start tag.
    pub name: Range<usize>,
    /// Attributes in document order.
    pub attributes: Vec<Attribute>,
    /// Whitespace between the last attribute (or the name) and the closing
    /// delimiter of the start tag.
    pub trailing: Range<usize>,
    /// Closing delimiter of the start tag: `>` or `/>`.
    pub close: Range<usize>,
    /// End tag; `None` for an empty-element tag.
    pub end_tag: Option<Range<usize>>,
    /// Child nodes in document order.
    pub children: Vec<NodeId>,
}

impl Element {
    /// `true` for an empty-element tag `<a/>`.
    pub const fn is_empty_tag(&self) -> bool {
        self.end_tag.is_none()
    }

    /// Range between the start tag and the end tag; empty at the end of the
    /// tag for an empty-element tag.
    pub fn content(&self) -> Range<usize> {
        match &self.end_tag {
            Some(end_tag) => self.start_tag.end..end_tag.start,
            None => self.start_tag.end..self.start_tag.end,
        }
    }
}

/// Character data with references, possibly spanning several reader
/// events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextNode {
    /// Raw text.
    pub span: Range<usize>,
}

/// CDATA section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CDataSection {
    /// Whole section from `<![CDATA[` to `]]>`.
    pub span: Range<usize>,
    /// Content between the delimiters.
    pub content: Range<usize>,
}

/// Comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Whole comment from `<!--` to `-->`.
    pub span: Range<usize>,
    /// Content between the delimiters.
    pub content: Range<usize>,
}

/// Processing instruction other than the XML declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingInstruction {
    /// Whole instruction from `<?` to `?>`.
    pub span: Range<usize>,
    /// Target name.
    pub target: Range<usize>,
    /// Content after the target and the whitespace that follows it.
    pub content: Range<usize>,
}

/// Node of an [`XmlTree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// XML declaration.
    Declaration(Declaration),
    /// Document type declaration.
    DocType(DocType),
    /// Element.
    Element(Element),
    /// Character data.
    Text(TextNode),
    /// CDATA section.
    CData(CDataSection),
    /// Comment.
    Comment(Comment),
    /// Processing instruction.
    ProcessingInstruction(ProcessingInstruction),
}

impl Node {
    /// Byte range of the whole node.
    pub fn span(&self) -> Range<usize> {
        match self {
            Self::Declaration(node) => node.span.clone(),
            Self::DocType(node) => node.span.clone(),
            Self::Element(node) => node.span.clone(),
            Self::Text(node) => node.span.clone(),
            Self::CData(node) => node.span.clone(),
            Self::Comment(node) => node.span.clone(),
            Self::ProcessingInstruction(node) => node.span.clone(),
        }
    }
}

/// Node addressed by a key path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The document as a whole (`$`).
    Document,
    /// Element.
    Element(NodeId),
    /// Attribute `index` of `element`.
    Attribute {
        /// Element that holds the attribute.
        element: NodeId,
        /// Position in [`Element::attributes`].
        index: usize,
    },
    /// Text or CDATA chunk of `element`.
    Text {
        /// Element that holds the chunk.
        element: NodeId,
        /// Text or CDATA node.
        node: NodeId,
    },
}

/// Outcome of resolving a key path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
    /// Nodes addressed by the key.
    Found(Vec<Target>),
    /// Segment `segment` of the key has no node inside `parent`.
    Missing {
        /// Deepest existing element; `None` for the document.
        parent: Option<NodeId>,
        /// Index of the first unresolved segment.
        segment: usize,
    },
}

/// Lossless XML document tree.
///
/// The tree keeps the source text unchanged and records the byte range of
/// every syntactic part. Values are decoded on demand, and changes are
/// expressed as [`XmlEdit`](super::XmlEdit) replacements of byte ranges, so
/// bytes outside the changed ranges are never rewritten.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::xml::XmlTree;
///
/// let text = "<project>\n  <version>1.0</version>\n</project>\n";
/// let tree = XmlTree::parse(text)?;
/// let key = KeyPath::parse("project.version")?;
/// let change = tree.set(&key, &Value::Text("1.1".into()), false)?;
/// assert_eq!(
///     change.apply(tree.text())?,
///     "<project>\n  <version>1.1</version>\n</project>\n"
/// );
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct XmlTree {
    pub(crate) text: String,
    pub(crate) nodes: Vec<Node>,
    pub(crate) parents: Vec<Option<NodeId>>,
    pub(crate) document: Vec<NodeId>,
    pub(crate) root: NodeId,
    pub(crate) line_starts: Vec<usize>,
    pub(crate) style: XmlStyle,
    pub(crate) resolver: ResolverSlot,
}

impl XmlTree {
    /// Source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Document element.
    pub const fn root(&self) -> NodeId {
        self.root
    }

    /// Nodes outside the document element and the document element itself,
    /// in document order.
    pub fn document_nodes(&self) -> &[NodeId] {
        &self.document
    }

    /// Node with the given identifier.
    ///
    /// # Panics
    ///
    /// Panics when `id` belongs to another tree and is out of range.
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0]
    }

    /// All node identifiers in document order.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.nodes.len()).map(NodeId)
    }

    /// Element with the given identifier; `None` for other nodes.
    pub fn element(&self, id: NodeId) -> Option<&Element> {
        match self.nodes.get(id.0) {
            Some(Node::Element(element)) => Some(element),
            _ => None,
        }
    }

    /// Parent element; `None` for nodes outside the document element and
    /// for the document element.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parents.get(id.0).copied().flatten()
    }

    /// Child nodes of an element; empty for other nodes.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.element(id).map_or(&[], |element| &element.children)
    }

    /// Child elements of an element.
    pub fn child_elements(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(|child| self.element(*child).is_some())
    }

    /// Qualified name of an element as written; empty for other nodes.
    pub fn name(&self, id: NodeId) -> &str {
        self.element(id)
            .map_or("", |element| &self.text[element.name.clone()])
    }

    /// XML declaration, if present.
    pub fn declaration(&self) -> Option<&Declaration> {
        self.document.iter().find_map(|id| match self.node(*id) {
            Node::Declaration(declaration) => Some(declaration),
            _ => None,
        })
    }

    /// Document type declaration, if present.
    pub fn doctype(&self) -> Option<&DocType> {
        self.document.iter().find_map(|id| match self.node(*id) {
            Node::DocType(doctype) => Some(doctype),
            _ => None,
        })
    }

    /// Installs the resolver of general entities declared in the document
    /// type definition.
    pub fn set_entity_source(&mut self, resolver: Arc<dyn EntitySource>) {
        self.resolver = ResolverSlot(Some(resolver));
    }

    /// Installed entity source.
    pub(crate) fn resolver(&self) -> Option<&dyn EntitySource> {
        self.resolver.0.as_deref()
    }

    /// Layout conventions detected in the document.
    pub const fn style(&self) -> &XmlStyle {
        &self.style
    }

    /// Line (from 1) and column (from 1, in Unicode scalar values) of a byte
    /// offset.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a>\n  <ü/><b/>\n</a>")?;
    /// let b = tree.child_elements(tree.root()).nth(1).unwrap();
    /// let start = tree.node(b).span().start;
    /// assert_eq!(tree.line_column(start), (2, 7));
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn line_column(&self, offset: usize) -> (u64, u64) {
        let line = self.line_starts.partition_point(|start| *start <= offset);
        let line_start = self
            .line_starts
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(0);
        let column = self
            .text
            .get(line_start..offset)
            .map_or(0, |prefix| prefix.chars().count());
        (as_u64(line), as_u64(column + 1))
    }

    /// Text chunks of an element that hold more than whitespace, each
    /// identified by its first node, in document order.
    ///
    /// A chunk is a maximal run of adjacent text and CDATA nodes, since
    /// CDATA boundaries carry no meaning; the chunks are the items
    /// addressed by `#text[n]`.
    pub fn text_chunks(&self, element: NodeId) -> Vec<NodeId> {
        let children = self.children(element);
        let mut chunks = Vec::new();
        let mut index = 0;
        while index < children.len() {
            let run = self.run_at(children, index);
            if run.is_empty() {
                index += 1;
                continue;
            }
            if run.iter().any(|node| self.has_character_content(*node)) {
                chunks.push(run[0]);
            }
            index += run.len();
        }
        chunks
    }

    /// Adjacent text and CDATA nodes of `siblings` from `index` on.
    fn run_at<'s>(&self, siblings: &'s [NodeId], index: usize) -> &'s [NodeId] {
        let length = siblings[index..]
            .iter()
            .take_while(|node| matches!(self.node(**node), Node::Text(_) | Node::CData(_)))
            .count();
        &siblings[index..index + length]
    }

    /// Run of adjacent text and CDATA nodes that starts at `node`; empty
    /// for other nodes.
    pub(crate) fn run(&self, node: NodeId) -> Vec<NodeId> {
        let siblings = self.siblings(node);
        siblings
            .iter()
            .position(|sibling| *sibling == node)
            .map(|index| self.run_at(siblings, index).to_vec())
            .unwrap_or_default()
    }

    /// `true` for a text or CDATA node that holds more than whitespace.
    fn has_character_content(&self, node: NodeId) -> bool {
        match self.node(node) {
            Node::Text(text) => !trim_xml(&self.text[text.span.clone()]).is_empty(),
            Node::CData(cdata) => !trim_xml(&self.text[cdata.content.clone()]).is_empty(),
            _ => false,
        }
    }

    /// Key segments of an element from the document element down.
    ///
    /// An index follows a name when the parent has several child elements
    /// with that name.
    pub(crate) fn element_segments(&self, element: NodeId) -> Vec<KeySegment> {
        let mut chain = Vec::new();
        let mut current = Some(element);
        while let Some(id) = current {
            chain.push(id);
            current = self.parent(id);
        }
        let mut segments = Vec::with_capacity(chain.len());
        for id in chain.into_iter().rev() {
            let name = self.name(id);
            segments.push(KeySegment::Key(name.to_owned()));
            if let Some(parent) = self.parent(id) {
                let same: Vec<NodeId> = self
                    .child_elements(parent)
                    .filter(|sibling| self.name(*sibling) == name)
                    .collect();
                if same.len() > 1 {
                    let position = same.iter().position(|sibling| *sibling == id).unwrap_or(0);
                    segments.push(KeySegment::Index(as_i64(position)));
                }
            }
        }
        segments
    }

    /// Canonical key path of a target.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::KeyPath;
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<deps><dep/><dep id='x'/></deps>")?;
    /// let targets = tree.resolve(&KeyPath::parse("deps.dep[1].@id")?)?;
    /// assert_eq!(tree.canonical_key(targets[0]), "deps.dep[1].@id");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn canonical_key(&self, target: Target) -> String {
        let segments = match target {
            Target::Document => Vec::new(),
            Target::Element(element) => self.element_segments(element),
            Target::Attribute { element, index } => {
                let mut segments = self.element_segments(element);
                if let Some(attribute) = self
                    .element(element)
                    .and_then(|el| el.attributes.get(index))
                {
                    segments.push(KeySegment::Key(format!(
                        "{ATTRIBUTE_SEGMENT_PREFIX}{}",
                        self.attribute_name(attribute)
                    )));
                }
                segments
            }
            Target::Text { element, node } => {
                let mut segments = self.element_segments(element);
                segments.push(KeySegment::Key(TEXT_SEGMENT.to_owned()));
                let chunks = self.text_chunks(element);
                if chunks.len() > 1 {
                    let position = chunks.iter().position(|chunk| *chunk == node).unwrap_or(0);
                    segments.push(KeySegment::Index(as_i64(position)));
                }
                segments
            }
        };
        KeyPath::new(None, segments)
            .map(|key| key.to_string())
            .unwrap_or_default()
    }

    /// Nodes addressed by a key path.
    ///
    /// The first segment names the document element. Element segments
    /// match child elements by name (see [`XmlTree::lookup_namespace`] for
    /// prefixes and `{uri}local`); `[n]` selects among siblings with that
    /// name, and a name shared by several siblings without an index or a
    /// wildcard is ambiguous. A wildcard directly after a name (`dep[*]`,
    /// `dep.*`) selects every sibling with that name; a wildcard after an
    /// index or at the start of the key selects child elements
    /// (`deps[0].*`). `@name` addresses an attribute and `#text` the text
    /// chunks of an element.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_NOT_FOUND` when no node matches and `E_KEY_CONFLICT`
    /// with the candidate keys when the key is ambiguous.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::KeyPath;
    /// use nurt_doc::formats::xml::{Target, XmlTree};
    ///
    /// let tree = XmlTree::parse("<a><b>1</b><b>2</b></a>")?;
    /// let error = tree.resolve(&KeyPath::parse("a.b")?).unwrap_err();
    /// assert_eq!(error.code().as_str(), "E_KEY_CONFLICT");
    /// let all = tree.resolve(&KeyPath::parse("a.b[*]")?)?;
    /// assert_eq!(all.len(), 2);
    /// assert!(matches!(all[0], Target::Element(_)));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn resolve(&self, key: &KeyPath) -> Result<Vec<Target>> {
        match self.resolve_key(key)? {
            Resolution::Found(targets) => Ok(targets),
            Resolution::Missing { segment, .. } => Err(key_not_found(format!(
                "key `{key}` does not exist: segment {} has no match; read the parent key to list members, or set with `create: true`",
                segment + 1
            ))),
        }
    }

    /// Resolves a key path, reporting the deepest existing element when a
    /// segment has no match.
    pub(crate) fn resolve_key(&self, key: &KeyPath) -> Result<Resolution> {
        if !matches!(key.document(), None | Some(DocSelector::Index(0))) {
            return Err(key_not_found(format!(
                "key `{key}` selects a document of a stream; an XML file holds one document, remove the `#n:` selector"
            )));
        }
        let segments = key.segments();
        let mut walk = Walk {
            tree: self,
            segments,
            cursors: vec![None],
            multi: false,
        };
        let mut index = 0;
        while index < segments.len() {
            match walk.step(index)? {
                Step::Next(next) => index = next,
                Step::Done(targets) => return Ok(Resolution::Found(targets)),
                Step::Missing(parent) => {
                    return Ok(Resolution::Missing {
                        parent,
                        segment: index,
                    });
                }
            }
        }
        if walk.cursors.is_empty() {
            return Err(key_not_found(format!("key `{key}` matches no element")));
        }
        Ok(Resolution::Found(
            walk.cursors
                .into_iter()
                .map(|cursor| cursor.map_or(Target::Document, Target::Element))
                .collect(),
        ))
    }

    /// Child elements of a cursor: the document element for the document.
    fn cursor_children(&self, cursor: Option<NodeId>) -> Vec<NodeId> {
        match cursor {
            Some(element) => self.child_elements(element).collect(),
            None => vec![self.root],
        }
    }

    /// Elements of the subtree of a cursor in document order, the cursor
    /// first.
    fn subtree(&self, cursor: Option<NodeId>) -> Vec<Option<NodeId>> {
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = match cursor {
            Some(element) => {
                out.push(Some(element));
                self.child_elements(element).collect()
            }
            None => {
                out.push(None);
                vec![self.root]
            }
        };
        stack.reverse();
        while let Some(id) = stack.pop() {
            out.push(Some(id));
            let mut children: Vec<NodeId> = self.child_elements(id).collect();
            children.reverse();
            stack.extend(children);
        }
        out
    }

    /// Child elements of `cursor` that match the key segment `name`.
    pub(crate) fn matching_children(&self, cursor: Option<NodeId>, name: &str) -> Vec<NodeId> {
        let children = self.cursor_children(cursor);
        let direct: Vec<NodeId> = children
            .iter()
            .copied()
            .filter(|child| self.name_matches(*child, name))
            .collect();
        if !direct.is_empty() || name.contains(':') || name.starts_with('{') {
            return direct;
        }
        children
            .into_iter()
            .filter(|child| self.matches_in_parent_prefix(*child, name))
            .collect()
    }
}

/// Result of one resolution step.
enum Step {
    /// Continue with the segment at this index.
    Next(usize),
    /// The key ends with these targets.
    Done(Vec<Target>),
    /// No match inside this parent.
    Missing(Option<NodeId>),
}

/// State of a key path resolution.
struct Walk<'a> {
    tree: &'a XmlTree,
    segments: &'a [KeySegment],
    cursors: Vec<Option<NodeId>>,
    multi: bool,
}

impl Walk<'_> {
    fn step(&mut self, index: usize) -> Result<Step> {
        match &self.segments[index] {
            KeySegment::Key(name) if name.starts_with(ATTRIBUTE_SEGMENT_PREFIX) => {
                self.attribute(index, &name[ATTRIBUTE_SEGMENT_PREFIX.len_utf8()..])
            }
            KeySegment::Key(name) if name == TEXT_SEGMENT => self.text(index),
            KeySegment::Key(name) => self.elements(index, name),
            KeySegment::Wildcard => {
                self.multi = true;
                self.cursors = self.all_children(|_| true);
                Ok(Step::Next(index + 1))
            }
            KeySegment::Recursive => {
                self.multi = true;
                let cursors = std::mem::take(&mut self.cursors);
                let mut seen = HashSet::new();
                for cursor in cursors {
                    for id in self.tree.subtree(cursor) {
                        if seen.insert(id) {
                            self.cursors.push(id);
                        }
                    }
                }
                Ok(Step::Next(index + 1))
            }
            KeySegment::Filter(expr) => {
                self.multi = true;
                let candidates = self.all_children(|_| true);
                self.cursors = self.filter(candidates, expr)?;
                Ok(Step::Next(index + 1))
            }
            _ => Err(key_not_found(format!(
                "segment {} of the key selects no element; use a name, `*`, `**` or a filter",
                index + 1
            ))),
        }
    }

    /// Child elements of every cursor that satisfy `keep`.
    fn all_children(&self, keep: impl Fn(NodeId) -> bool) -> Vec<Option<NodeId>> {
        self.cursors
            .iter()
            .flat_map(|cursor| self.tree.cursor_children(*cursor))
            .filter(|id| keep(*id))
            .map(Some)
            .collect()
    }

    fn filter(
        &self,
        candidates: Vec<Option<NodeId>>,
        expr: &nurt_contracts::keypath::FilterExpr,
    ) -> Result<Vec<Option<NodeId>>> {
        let mut kept = Vec::new();
        for candidate in candidates.into_iter().flatten() {
            if self.tree.filter_matches(candidate, expr)? {
                kept.push(Some(candidate));
            }
        }
        Ok(kept)
    }

    /// The only cursor, when resolution has not branched.
    fn single(&self) -> Option<Option<NodeId>> {
        match self.cursors.as_slice() {
            [cursor] if !self.multi => Some(*cursor),
            _ => None,
        }
    }

    fn is_last(&self, index: usize) -> bool {
        index + 1 == self.segments.len()
    }

    fn attribute(&self, index: usize, name: &str) -> Result<Step> {
        if !self.is_last(index) {
            return Err(key_not_found(
                "an attribute has no members; end the key at the `@` segment",
            ));
        }
        let mut targets = Vec::new();
        for element in self.cursors.iter().flatten() {
            if let Some(found) = self.tree.find_attribute(*element, name) {
                targets.push(Target::Attribute {
                    element: *element,
                    index: found,
                });
            }
        }
        self.finish(targets)
    }

    fn text(&self, index: usize) -> Result<Step> {
        let position = match self.segments.get(index + 1) {
            Some(KeySegment::Index(position)) if index + 2 == self.segments.len() => {
                Some(*position)
            }
            None => None,
            Some(_) => {
                return Err(key_not_found(
                    "`#text` has no members; end the key at `#text` or `#text[n]`",
                ));
            }
        };
        let mut targets = Vec::new();
        for element in self.cursors.iter().flatten() {
            let chunks = self.tree.text_chunks(*element);
            let picked = match position {
                Some(position) => pick(&chunks, position).into_iter().collect(),
                None if chunks.len() > 1 && !self.multi => {
                    return Err(self.ambiguous(chunks.into_iter().map(|node| Target::Text {
                        element: *element,
                        node,
                    })));
                }
                None => chunks,
            };
            targets.extend(picked.into_iter().map(|node| Target::Text {
                element: *element,
                node,
            }));
        }
        self.finish(targets)
    }

    /// Targets of a final segment, or the missing parent when nothing
    /// matched along an unbranched path.
    fn finish(&self, targets: Vec<Target>) -> Result<Step> {
        if !targets.is_empty() {
            return Ok(Step::Done(targets));
        }
        match self.single() {
            Some(parent) => Ok(Step::Missing(parent)),
            None => Err(key_not_found("key matches no node")),
        }
    }

    fn elements(&mut self, index: usize, name: &str) -> Result<Step> {
        let next = self.segments.get(index + 1);
        let mut found = Vec::new();
        for cursor in &self.cursors {
            let matches = self.tree.matching_children(*cursor, name);
            match next {
                Some(KeySegment::Index(position)) => match pick(&matches, *position) {
                    Some(id) => found.push(Some(id)),
                    None if self.single().is_some() => return Ok(Step::Missing(*cursor)),
                    None => {}
                },
                Some(KeySegment::Append) => return Ok(Step::Missing(*cursor)),
                Some(KeySegment::Wildcard | KeySegment::Filter(_)) => {
                    found.extend(matches.into_iter().map(Some));
                }
                _ if matches.len() > 1 && !self.multi => {
                    return Err(self.ambiguous(matches.into_iter().map(Target::Element)));
                }
                _ if matches.is_empty() && self.single().is_some() => {
                    return Ok(Step::Missing(*cursor));
                }
                _ => found.extend(matches.into_iter().map(Some)),
            }
        }
        let consumed = match next {
            Some(KeySegment::Filter(expr)) => {
                self.multi = true;
                found = self.filter(found, expr)?;
                2
            }
            Some(KeySegment::Wildcard) => {
                self.multi = true;
                2
            }
            Some(KeySegment::Index(_)) => 2,
            _ => 1,
        };
        self.cursors = found;
        Ok(Step::Next(index + consumed))
    }

    fn ambiguous(&self, candidates: impl Iterator<Item = Target>) -> nurt_contracts::error::Error {
        let all: Vec<Target> = candidates.collect();
        let keys: Vec<String> = all
            .iter()
            .take(MAX_CANDIDATES)
            .map(|target| self.tree.canonical_key(*target))
            .collect();
        key_conflict(
            format!(
                "key matches {} nodes; add an index such as `{}` or `[*]` for all",
                all.len(),
                keys.first().map_or("", String::as_str)
            ),
            keys,
        )
    }
}

/// Element of `items` at `position`; negative positions count from the end.
fn pick(items: &[NodeId], position: i64) -> Option<NodeId> {
    let index = if position < 0 {
        items
            .len()
            .checked_sub(usize::try_from(position.unsigned_abs()).ok()?)?
    } else {
        usize::try_from(position).ok()?
    };
    items.get(index).copied()
}

/// Converts a count to `u64`; counts of in-memory text always fit.
fn as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Converts a position to `i64`; positions of in-memory text always fit.
fn as_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_positions_count_from_the_end() {
        let items = [NodeId(1), NodeId(2), NodeId(3)];
        assert_eq!(pick(&items, -1), Some(NodeId(3)));
        assert_eq!(pick(&items, -3), Some(NodeId(1)));
        assert_eq!(pick(&items, -4), None);
        assert_eq!(pick(&items, 3), None);
    }

    #[test]
    fn recursive_segment_lists_every_element_once_in_document_order() {
        let tree = XmlTree::parse("<a><b><c/></b><b/></a>").unwrap();
        let key = KeyPath::parse("a.**.**").unwrap();
        let targets = tree.resolve(&key).unwrap();
        let keys: Vec<String> = targets
            .into_iter()
            .map(|target| tree.canonical_key(target))
            .collect();
        assert_eq!(keys, ["a", "a.b[0]", "a.b[0].c", "a.b[1]"]);
    }

    #[test]
    fn line_column_counts_scalar_values_after_multibyte_characters() {
        let tree = XmlTree::parse("<a>\r\n  <ü/><b/>\r\n</a>").unwrap();
        let b = tree.child_elements(tree.root()).nth(1).unwrap();
        assert_eq!(tree.line_column(tree.node(b).span().start), (2, 7));
        assert_eq!(tree.line_column(0), (1, 1));
    }

    #[test]
    fn ambiguous_key_lists_a_bounded_number_of_candidates() {
        let items = "<b/>".repeat(MAX_CANDIDATES + 4);
        let tree = XmlTree::parse(&format!("<a>{items}</a>")).unwrap();
        let error = tree.resolve(&KeyPath::parse("a.b").unwrap()).unwrap_err();
        assert_eq!(error.code().as_str(), "E_KEY_CONFLICT");
        assert!(
            error.message.contains("matches 12 nodes"),
            "{}",
            error.message
        );
        assert!(error.message.contains("a.b[0]"), "{}", error.message);
        let listed = error.details["candidates"].as_array().map(Vec::len);
        assert_eq!(listed, Some(MAX_CANDIDATES));
    }

    #[test]
    fn selector_of_another_document_is_rejected() {
        let tree = XmlTree::parse("<a/>").unwrap();
        let key = KeyPath::parse("#1:a").unwrap();
        assert!(tree.resolve(&key).is_err());
    }
}
