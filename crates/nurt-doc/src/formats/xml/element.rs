// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Elements: value model, reading by key and edits of elements and their
//! content.
//!
//! The value model follows the XML decoding convention of `yq -p xml`:
//!
//! | XML | Value |
//! |---|---|
//! | `<a/>`, `<a></a>`, `<a>  </a>` | `null` |
//! | `<a>text</a>` | `"text"` (surrounding whitespace removed) |
//! | `<a>x<!-- c -->y</a>` | `["x", "y"]`: one item per text or CDATA chunk |
//! | `<a id="1">t</a>` | `{"+content": "t", "+@id": "1"}` |
//! | `<a><b>1</b><b>2</b><c/></a>` | `{"b": ["1", "2"], "c": null}` |
//! | `<?target data?>` inside an element | member `+p_target` |
//! | XML declaration, `<!DOCTYPE …>` | members `+p_xml`, `+directive` of the document |
//!
//! Comments are not part of the value; the comment bound to an element is
//! reported with the item. Names keep their prefixes as written, and
//! namespace declarations are ordinary `+@xmlns…` members.

use std::collections::HashMap;
use std::ops::Range;

use nurt_contracts::error::Result;
use nurt_contracts::keypath::{CompareOp, FilterExpr, FilterLiteral, KeyPath, KeySegment};
use nurt_contracts::value::{Fields, Value};

use super::cdata::{check_representable, escape_attribute, escape_text, trim_xml};
use super::ns::{declared_prefix, is_valid_name, split_qname};
use super::tree::{
    ATTRIBUTE_SEGMENT_PREFIX, Node, NodeId, Resolution, TEXT_SEGMENT, Target, XmlTree,
};
use super::{
    CONSTRUCT_DOCUMENT_ROOT, CONSTRUCT_MIXED_CONTENT, CONSTRUCT_NAMESPACE_DECLARATION,
    CONSTRUCT_NESTED_LIST, CONSTRUCT_PROCESSING_INSTRUCTION, XmlChange, XmlEdit, invalid_value,
    key_conflict, key_not_found, unsupported,
};

/// Member of an element value that holds its text.
pub const CONTENT_KEY: &str = "+content";

/// Prefix of members of an element value that hold attributes.
pub const ATTRIBUTE_KEY_PREFIX: &str = "+@";

/// Prefix of members that hold processing instructions.
pub const PI_KEY_PREFIX: &str = "+p_";

/// Member of the document value that holds the document type declaration.
pub const DIRECTIVE_KEY: &str = "+directive";

/// Target of the XML declaration in the document value.
const DECLARATION_TARGET: &str = "xml";

/// Item read from an XML document.
#[derive(Debug, Clone, PartialEq)]
pub struct XmlItem {
    /// Canonical key path of the node.
    pub key: String,
    /// Addressed node.
    pub target: Target,
    /// Value in the model described in the module documentation.
    pub value: Value,
    /// Line of the node start, from 1.
    pub line: u64,
    /// Column of the node start, from 1, in Unicode scalar values.
    pub column: u64,
    /// Comments bound to an element.
    pub comment: Option<String>,
    /// Byte range of the node.
    pub span: Range<usize>,
}

/// New value classified by the markup it produces.
enum Shape<'a> {
    Null,
    Scalar(String),
    Map(&'a Fields),
    List(&'a [Value]),
}

/// Classifies a value; scalars are converted to their text.
fn shape(value: &Value) -> Shape<'_> {
    use serde_json::Value as Json;
    match value {
        Value::Null => Shape::Null,
        Value::Text(text) => Shape::Scalar(text.clone()),
        Value::Map(fields) => Shape::Map(fields),
        Value::Record(record) => Shape::Map(&record.fields),
        Value::List(items) => Shape::List(items),
        other => match other.to_plain_json() {
            Json::Null => Shape::Null,
            Json::String(text) => Shape::Scalar(text),
            json => Shape::Scalar(json.to_string()),
        },
    }
}

/// Text of a scalar value for an attribute or a text chunk.
fn scalar_text(value: &Value, what: &str) -> Result<String> {
    match shape(value) {
        Shape::Null => Ok(String::new()),
        Shape::Scalar(text) => Ok(text),
        Shape::Map(_) | Shape::List(_) => Err(invalid_value(format!(
            "{what} holds text; pass a text, number or boolean"
        ))),
    }
}

/// Role of a member of a map value written as an element.
enum Member<'a> {
    Attribute(&'a str),
    Content,
    Instruction(&'a str),
    Child(&'a str),
}

/// Role of a map member: `@name` and `+@name` are attributes, `#text` and
/// `+content` the text, `+p_target` a processing instruction, other names
/// child elements.
fn member(name: &str) -> Result<Member<'_>> {
    if let Some(attribute) = name
        .strip_prefix(ATTRIBUTE_KEY_PREFIX)
        .or_else(|| name.strip_prefix(ATTRIBUTE_SEGMENT_PREFIX))
    {
        return Ok(Member::Attribute(attribute));
    }
    if name == CONTENT_KEY || name == TEXT_SEGMENT {
        return Ok(Member::Content);
    }
    if name.starts_with(PI_KEY_PREFIX) {
        return Ok(Member::Instruction(name));
    }
    Ok(Member::Child(name))
}

/// Error for a member that would create a processing instruction.
fn instruction_error(name: &str) -> nurt_contracts::error::Error {
    unsupported(
        CONSTRUCT_PROCESSING_INSTRUCTION,
        format!(
            "member `{name}` would create a processing instruction; edit the file text with `replace`"
        ),
    )
}

/// Value that a new value has in the model after it is written.
fn model_of(value: &Value) -> Result<Value> {
    Ok(match shape(value) {
        Shape::Null => Value::Null,
        Shape::Scalar(text) => Value::Text(text),
        Shape::List(items) => Value::List(items.iter().map(model_of).collect::<Result<_>>()?),
        Shape::Map(fields) => map_model(fields)?,
    })
}

fn map_model(fields: &Fields) -> Result<Value> {
    let mut content = None;
    let mut attributes = Fields::new();
    let mut children = Fields::new();
    for (name, value) in fields {
        match member(name)? {
            Member::Content => content = Some(model_of(value)?),
            Member::Attribute(attribute) => {
                attributes.insert(
                    format!("{ATTRIBUTE_KEY_PREFIX}{attribute}"),
                    Value::Text(scalar_text(value, "an attribute")?),
                );
            }
            Member::Instruction(target) => {
                children.insert(
                    target.to_owned(),
                    Value::Text(scalar_text(value, "a processing instruction")?),
                );
            }
            Member::Child(child) => {
                children.insert(child.to_owned(), model_of(value)?);
            }
        }
    }
    let content = content.filter(|value| *value != Value::Null);
    if attributes.is_empty() && children.is_empty() {
        return Ok(content.unwrap_or(Value::Null));
    }
    let mut map = Fields::new();
    if let Some(content) = content {
        map.insert(CONTENT_KEY.to_owned(), content);
    }
    map.extend(attributes);
    map.extend(children);
    Ok(Value::Map(map))
}

/// Members in document order; a name that occurs several times becomes one
/// member holding the list of values.
fn group(entries: Vec<(String, Value)>) -> Fields {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (name, _) in &entries {
        *counts.entry(name.clone()).or_default() += 1;
    }
    let mut map = Fields::new();
    for (name, value) in entries {
        if counts.get(&name).copied().unwrap_or_default() > 1 {
            if let Value::List(items) = map.entry(name).or_insert_with(|| Value::List(Vec::new())) {
                items.push(value);
            }
        } else {
            map.insert(name, value);
        }
    }
    map
}

impl XmlTree {
    /// Value of the whole document: the XML declaration, the document type
    /// declaration, top-level processing instructions and the document
    /// element under its name.
    ///
    /// # Errors
    ///
    /// Returns decoding errors of references.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse(r#"<?xml version="1.0"?><r a="1"><b>x</b><b/></r>"#)?;
    /// let json = tree.to_value()?.to_plain_json();
    /// assert_eq!(
    ///     json,
    ///     serde_json::json!({"+p_xml": "version=\"1.0\"", "r": {"+@a": "1", "b": ["x", null]}})
    /// );
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn to_value(&self) -> Result<Value> {
        let mut entries = Vec::new();
        for id in self.document_nodes() {
            match self.node(*id) {
                Node::Declaration(declaration) => {
                    let raw = &self.text()
                        [declaration.span.start + "<?xml".len()..declaration.span.end - "?>".len()];
                    entries.push((
                        format!("{PI_KEY_PREFIX}{DECLARATION_TARGET}"),
                        Value::Text(trim_xml(raw).to_owned()),
                    ));
                }
                Node::DocType(doctype) => {
                    let raw = &self.text()[doctype.span.start + "<!".len()..doctype.span.end - 1];
                    entries.push((DIRECTIVE_KEY.to_owned(), Value::Text(raw.to_owned())));
                }
                Node::ProcessingInstruction(_) => entries.push(self.pi_member(*id)),
                Node::Element(_) => {
                    entries.push((self.name(*id).to_owned(), self.element_value(*id)?));
                }
                _ => {}
            }
        }
        Ok(Value::Map(group(entries)))
    }

    /// Member name and value of a processing instruction.
    fn pi_member(&self, id: NodeId) -> (String, Value) {
        match self.node(id) {
            Node::ProcessingInstruction(pi) => (
                format!("{PI_KEY_PREFIX}{}", &self.text()[pi.target.clone()]),
                Value::Text(trim_xml(&self.text()[pi.content.clone()]).to_owned()),
            ),
            _ => (String::new(), Value::Null),
        }
    }

    /// Value of an element in the model described in the module
    /// documentation.
    ///
    /// # Errors
    ///
    /// Returns decoding errors of references.
    pub fn element_value(&self, element: NodeId) -> Result<Value> {
        let Some(el) = self.element(element) else {
            return Ok(Value::Null);
        };
        let mut pieces = Vec::new();
        for chunk in self.text_chunks(element) {
            pieces.push(Value::Text(self.chunk_value(chunk)?.unwrap_or_default()));
        }
        let mut entries = Vec::new();
        for child in &el.children {
            match self.node(*child) {
                Node::Element(_) => {
                    entries.push((self.name(*child).to_owned(), self.element_value(*child)?));
                }
                Node::ProcessingInstruction(_) => entries.push(self.pi_member(*child)),
                _ => {}
            }
        }
        let members = group(entries);
        let content = match pieces.len() {
            0 => None,
            1 => pieces.pop(),
            _ => Some(Value::List(pieces)),
        };
        if el.attributes.is_empty() && members.is_empty() {
            return Ok(content.unwrap_or(Value::Null));
        }
        let mut map = Fields::new();
        if let Some(content) = content {
            map.insert(CONTENT_KEY.to_owned(), content);
        }
        for attribute in &el.attributes {
            map.insert(
                format!("{ATTRIBUTE_KEY_PREFIX}{}", self.attribute_name(attribute)),
                Value::Text(self.attribute_value(attribute)?),
            );
        }
        map.extend(members);
        Ok(Value::Map(map))
    }

    /// Value of a target.
    ///
    /// # Errors
    ///
    /// Returns decoding errors of references.
    pub fn target_value(&self, target: Target) -> Result<Value> {
        match target {
            Target::Document => self.to_value(),
            Target::Element(element) => self.element_value(element),
            Target::Attribute { element, index } => {
                match self
                    .element(element)
                    .and_then(|el| el.attributes.get(index))
                {
                    Some(attribute) => Ok(Value::Text(self.attribute_value(attribute)?)),
                    None => Ok(Value::Null),
                }
            }
            Target::Text { node, .. } => {
                Ok(self.chunk_value(node)?.map_or(Value::Null, Value::Text))
            }
        }
    }

    /// Byte range of a target.
    fn target_span(&self, target: Target) -> Range<usize> {
        match target {
            Target::Document => 0..self.text().len(),
            Target::Element(id) | Target::Text { node: id, .. } => self.node(id).span(),
            Target::Attribute { element, index } => self
                .element(element)
                .and_then(|el| el.attributes.get(index))
                .map_or(0..0, |attribute| attribute.span.clone()),
        }
    }

    /// Items addressed by a key path.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`XmlTree::resolve`] and decoding errors of
    /// references.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::KeyPath;
    /// use nurt_contracts::value::Value;
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a>\n  <!-- port -->\n  <p x='1'>80</p>\n</a>")?;
    /// let items = tree.read(&KeyPath::parse("a.p")?)?;
    /// assert_eq!(items[0].key, "a.p");
    /// assert_eq!(items[0].comment.as_deref(), Some("port"));
    /// assert_eq!((items[0].line, items[0].column), (3, 3));
    /// let attribute = tree.read(&KeyPath::parse("a.p.@x")?)?;
    /// assert_eq!(attribute[0].value, Value::Text("1".into()));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn read(&self, key: &KeyPath) -> Result<Vec<XmlItem>> {
        self.resolve(key)?
            .into_iter()
            .map(|target| {
                let span = self.target_span(target);
                let (line, column) = self.line_column(span.start);
                let comment = match target {
                    Target::Element(element) => self.comment_before(element),
                    _ => None,
                };
                Ok(XmlItem {
                    key: self.canonical_key(target),
                    target,
                    value: self.target_value(target)?,
                    line,
                    column,
                    comment,
                    span,
                })
            })
            .collect()
    }

    /// Edits that set the value addressed by a key path.
    ///
    /// A scalar replaces the text of an element and keeps its attributes;
    /// a map replaces the content and the attributes of the element
    /// (namespace declarations stay); a list replaces the element by one
    /// element per item. With `create`, missing elements are inserted after
    /// their last sibling with the indentation of the siblings; the
    /// segments `[+]` and `[n]` with `n` equal to the number of siblings
    /// append an element without `create`.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_NOT_FOUND` for a missing key without `create`,
    /// `E_KEY_CONFLICT` for an ambiguous key or overlapping edits,
    /// `E_INVALID_VALUE` for values that XML cannot hold at the target and
    /// `E_UNSUPPORTED_CONSTRUCT` for changes that cannot be written without
    /// layout loss.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::KeyPath;
    /// use nurt_contracts::value::Value;
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<deps>\n    <dep>a</dep>\n</deps>\n")?;
    /// let change = tree.set(&KeyPath::parse("deps.dep[+]")?, &Value::Text("b".into()), false)?;
    /// assert_eq!(change.apply(tree.text())?, "<deps>\n    <dep>a</dep>\n    <dep>b</dep>\n</deps>\n");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set(&self, key: &KeyPath, value: &Value, create: bool) -> Result<XmlChange> {
        match self.resolve_key(key)? {
            Resolution::Found(targets) => {
                let mut change = XmlChange::default();
                for target in targets {
                    change.merge(self.set_target(target, value)?);
                }
                Ok(change)
            }
            Resolution::Missing { parent, segment } => {
                self.create_missing(key, parent, segment, value, create)
            }
        }
    }

    fn set_target(&self, target: Target, value: &Value) -> Result<XmlChange> {
        match target {
            Target::Document => Err(unsupported(
                CONSTRUCT_DOCUMENT_ROOT,
                "the document as a whole cannot be replaced; set a key below the root element",
            )),
            Target::Element(element) => self.set_element_value(element, value),
            Target::Attribute { element, index } => {
                let name = self
                    .element(element)
                    .and_then(|el| el.attributes.get(index))
                    .map(|attribute| self.attribute_name(attribute).to_owned())
                    .unwrap_or_default();
                self.set_attribute(element, &name, &scalar_text(value, "an attribute")?)
            }
            Target::Text { node, .. } => {
                self.replace_text(node, &scalar_text(value, "a text chunk")?)
            }
        }
    }

    /// Edits that give an element a new value; see [`XmlTree::set`].
    ///
    /// # Errors
    ///
    /// Returns the errors described for [`XmlTree::set`].
    pub fn set_element_value(&self, element: NodeId, value: &Value) -> Result<XmlChange> {
        if model_of(value).is_ok_and(|model| self.element_value(element).ok() == Some(model)) {
            return Ok(XmlChange::default());
        }
        match shape(value) {
            Shape::Null => self.replace_content(element, "", false),
            Shape::Scalar(text) => self.set_element_text(element, &text),
            Shape::Map(fields) => self.set_element_map(element, fields, value),
            Shape::List(items) => self.replace_with_list(element, items),
        }
    }

    /// Sets the text of an element, keeping its attributes.
    fn set_element_text(&self, element: NodeId, text: &str) -> Result<XmlChange> {
        check_representable(text)?;
        let chunks = self.text_chunks(element);
        let only_text = self
            .children(element)
            .iter()
            .all(|child| matches!(self.node(*child), Node::Text(_) | Node::CData(_)));
        if let ([chunk], true) = (chunks.as_slice(), only_text) {
            return self.replace_text(*chunk, text);
        }
        let escaped = escape_text(text, self.style().eol);
        self.replace_content(element, &escaped, false)
    }

    /// Replaces the content of an element; an empty-element tag is expanded
    /// when the content is not empty. `block` places the content on its
    /// own lines in a multi-line document.
    fn replace_content(&self, element: NodeId, markup: &str, block: bool) -> Result<XmlChange> {
        let Some(el) = self.element(element) else {
            return Err(invalid_value("the key does not address an element"));
        };
        let mut change = XmlChange::default();
        let content = el.content();
        let inner = if block && !markup.is_empty() && self.style().multiline {
            let indent = self.child_indent(element);
            let outer = self.line_indentation(el.span.start);
            let eol = self.style().eol;
            format!("{eol}{indent}{markup}{eol}{outer}")
        } else {
            markup.to_owned()
        };
        if el.is_empty_tag() {
            if !inner.is_empty() {
                change.push(self.expand_empty_tag(element, &inner));
            }
        } else if self.text()[content.clone()] != inner {
            change.push(XmlEdit::new(content, inner));
        }
        Ok(change)
    }

    /// Edit that turns `<a/>` into `<a>inner</a>`.
    fn expand_empty_tag(&self, element: NodeId, inner: &str) -> XmlEdit {
        let (trailing, close) = self
            .element(element)
            .map_or((0..0, 0..0), |el| (el.trailing.clone(), el.close.clone()));
        let start = if self.text()[trailing.clone()].contains('\n') {
            close.start
        } else {
            trailing.start
        };
        XmlEdit::new(
            start..close.end,
            format!(">{inner}</{}>", self.name(element)),
        )
    }

    /// Replaces attributes and content of an element by a map value.
    fn set_element_map(
        &self,
        element: NodeId,
        fields: &Fields,
        value: &Value,
    ) -> Result<XmlChange> {
        let expected = model_of(value)?;
        if self.element_value(element)? == expected {
            return Ok(XmlChange::default());
        }
        let prefix = split_qname(self.name(element)).0.map(str::to_owned);
        let indent = self.child_indent(element);
        let render = Render::new(self, element);
        let (attributes, content, block) =
            render.members(fields, prefix.as_deref(), &indent, &[])?;
        let mut change = XmlChange::default();
        for (name, text) in &attributes {
            change.merge(self.set_attribute(element, name, text)?);
        }
        if let Some(el) = self.element(element) {
            for attribute in &el.attributes {
                let name = self.attribute_name(attribute);
                if declared_prefix(name).is_none()
                    && !attributes.iter().any(|(kept, _)| kept == name)
                {
                    change.merge(self.remove_attribute(element, name));
                }
            }
        }
        change.merge(self.replace_content(element, &content, block)?);
        Ok(change)
    }

    /// Replaces an element by one element with the same name per item.
    fn replace_with_list(&self, element: NodeId, items: &[Value]) -> Result<XmlChange> {
        if element == self.root() {
            return Err(unsupported(
                CONSTRUCT_DOCUMENT_ROOT,
                "a document has one root element; set a map or a scalar",
            ));
        }
        if items.is_empty() {
            return Ok(self.remove_node(element));
        }
        let indent = self
            .line_indentation(self.node(element).span().start)
            .to_owned();
        let parent = self.parent(element).unwrap_or(element);
        let render = Render::new(self, parent);
        let markup = render.siblings(self.name(element), items, &indent)?;
        let mut change = XmlChange::default();
        if self.text()[self.node(element).span()] != markup {
            change.push(XmlEdit::new(self.node(element).span(), markup));
        }
        Ok(change)
    }

    /// Edits that insert child element `name` with `value` into `parent`.
    ///
    /// The element follows the last child with the same name, otherwise the
    /// last child, with the indentation of its siblings; under a prefixed
    /// parent an unprefixed name receives the parent's prefix. A list value
    /// inserts one element per item.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` for an invalid name or value and
    /// `E_UNSUPPORTED_CONSTRUCT` `xml.namespace_declaration` when the
    /// namespace of the name has no prefix in scope.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::value::Value;
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a/>")?;
    /// let change = tree.insert_element(tree.root(), "b", &Value::Text("1".into()))?;
    /// assert_eq!(change.apply(tree.text())?, "<a><b>1</b></a>");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn insert_element(&self, parent: NodeId, name: &str, value: &Value) -> Result<XmlChange> {
        let written = self.new_element_name(parent, name)?;
        let indent = self.child_indent(parent);
        let render = Render::new(self, parent);
        let markup = match shape(value) {
            Shape::List(items) => render.siblings(&written, items, &indent)?,
            _ => render.element(&written, value, &indent, &[])?,
        };
        let mut change = XmlChange::default();
        change.push(self.insertion(parent, &written, &markup, &indent));
        Ok(change)
    }

    /// Written name of a new child element of `parent`.
    fn new_element_name(&self, parent: NodeId, name: &str) -> Result<String> {
        let prefix = split_qname(self.name(parent)).0;
        let written = self.written_name(parent, prefix, name, true).ok_or_else(|| {
            unsupported(
                CONSTRUCT_NAMESPACE_DECLARATION,
                format!("namespace of `{name}` has no prefix in scope; declare the prefix with an `xmlns:` attribute first"),
            )
        })?;
        if is_valid_name(&written) {
            Ok(written)
        } else {
            Err(invalid_value(format!(
                "`{written}` is not a valid element name; names start with a letter, `_` or `:`"
            )))
        }
    }

    /// Edit that inserts `markup` as the last child of `parent`, or after
    /// the last child named `name`.
    fn insertion(&self, parent: NodeId, name: &str, markup: &str, indent: &str) -> XmlEdit {
        let children = self.children(parent);
        let anchor = children
            .iter()
            .rev()
            .find(|child| self.element(**child).is_some() && self.name(**child) == name)
            .or_else(|| {
                children
                    .iter()
                    .rev()
                    .find(|child| match self.node(**child) {
                        Node::Text(text) => !trim_xml(&self.text()[text.span.clone()]).is_empty(),
                        _ => true,
                    })
            })
            .copied();
        let eol = self.style().eol;
        match anchor {
            Some(anchor) if !matches!(self.node(anchor), Node::Text(_) | Node::CData(_)) => {
                let at = self.end_with_line_comments(anchor);
                match self.own_line_indent(anchor) {
                    Some(sibling_indent) if self.style().multiline => {
                        XmlEdit::new(at..at, format!("{eol}{sibling_indent}{markup}"))
                    }
                    _ => XmlEdit::new(at..at, markup.to_owned()),
                }
            }
            Some(_) => {
                let at = self.element(parent).map_or(0, |el| el.content().end);
                XmlEdit::new(at..at, markup.to_owned())
            }
            None => self.first_child_edit(parent, markup, indent),
        }
    }

    /// Edit that writes the first child of an element without children
    /// other than whitespace.
    fn first_child_edit(&self, parent: NodeId, markup: &str, indent: &str) -> XmlEdit {
        let Some(el) = self.element(parent) else {
            return XmlEdit::new(0..0, String::new());
        };
        let inner = if self.style().multiline {
            let eol = self.style().eol;
            let outer = self.line_indentation(el.span.start);
            format!("{eol}{indent}{markup}{eol}{outer}")
        } else {
            markup.to_owned()
        };
        if el.is_empty_tag() {
            self.expand_empty_tag(parent, &inner)
        } else {
            XmlEdit::new(el.content(), inner)
        }
    }

    /// Creates the missing part of a key.
    fn create_missing(
        &self,
        key: &KeyPath,
        parent: Option<NodeId>,
        segment: usize,
        value: &Value,
        create: bool,
    ) -> Result<XmlChange> {
        let remaining = &key.segments()[segment..];
        let Some(parent) = parent else {
            return Err(self.missing_root(key, create));
        };
        let appends = self.is_append(parent, remaining)?;
        if !create && !appends {
            return Err(key_not_found(format!(
                "key `{key}` does not exist; set it with `create: true` or append with `[+]`"
            )));
        }
        let mut change = match remaining {
            [KeySegment::Key(name)] if name.starts_with(ATTRIBUTE_SEGMENT_PREFIX) => self
                .set_attribute(
                    parent,
                    &name[ATTRIBUTE_SEGMENT_PREFIX.len_utf8()..],
                    &scalar_text(value, "an attribute")?,
                )?,
            [KeySegment::Key(name), rest @ ..] if name == TEXT_SEGMENT => {
                self.insert_text(parent, rest, value)?
            }
            [KeySegment::Key(name), rest @ ..] => {
                let rest = match rest {
                    [KeySegment::Index(_) | KeySegment::Append, tail @ ..] => tail,
                    _ => rest,
                };
                let nested = nest(rest, value)?;
                self.insert_element(parent, name, &nested)?
            }
            _ => {
                return Err(key_not_found(format!(
                    "key `{key}` cannot be created; name the missing elements"
                )));
            }
        };
        change.keys_created = self.created_keys(parent, remaining);
        Ok(change)
    }

    /// Error for a key whose first segment does not name the document
    /// element.
    fn missing_root(&self, key: &KeyPath, create: bool) -> nurt_contracts::error::Error {
        if create {
            unsupported(
                CONSTRUCT_DOCUMENT_ROOT,
                format!(
                    "the document element is `{}`; a document has one root element",
                    self.name(self.root())
                ),
            )
        } else {
            key_not_found(format!(
                "key `{key}` does not exist: the document element is `{}`; start the key with it",
                self.name(self.root())
            ))
        }
    }

    /// `true` when the missing part appends an element (`name[+]` or
    /// `name[n]` with `n` equal to the number of such siblings).
    fn is_append(&self, parent: NodeId, remaining: &[KeySegment]) -> Result<bool> {
        let (name, position) = match remaining {
            [KeySegment::Key(name), KeySegment::Append] => return Ok(!name.starts_with(['@', '#'])),
            [KeySegment::Key(name), KeySegment::Index(position), ..] => (name, *position),
            _ => return Ok(false),
        };
        let count = self.matching_children(Some(parent), name).len();
        let fits = usize::try_from(position).is_ok_and(|position| position == count);
        if !fits && position >= 0 {
            return Err(key_conflict(
                format!(
                    "`{name}[{position}]` is beyond the {count} existing elements; append with `[+]` or `[{count}]`"
                ),
                Vec::new(),
            ));
        }
        Ok(fits && remaining.len() == 2)
    }

    /// Inserts the first text chunk of an element.
    fn insert_text(&self, parent: NodeId, rest: &[KeySegment], value: &Value) -> Result<XmlChange> {
        if !matches!(rest, [] | [KeySegment::Index(0)]) || !self.text_chunks(parent).is_empty() {
            return Err(key_conflict(
                "only the first text chunk of an element can be created; address an existing `#text[n]`",
                Vec::new(),
            ));
        }
        let text = scalar_text(value, "a text chunk")?;
        check_representable(&text)?;
        let escaped = escape_text(&text, self.style().eol);
        let mut change = XmlChange::default();
        match self.element(parent) {
            Some(el) if el.is_empty_tag() => change.push(self.expand_empty_tag(parent, &escaped)),
            Some(el) => change.push(XmlEdit::new(
                el.content().start..el.content().start,
                escaped,
            )),
            None => {}
        }
        Ok(change)
    }

    /// Canonical keys of the nodes created for the missing segments.
    fn created_keys(&self, parent: NodeId, remaining: &[KeySegment]) -> Vec<String> {
        let mut segments = self.element_segments(parent);
        let mut keys = Vec::new();
        for (index, segment) in remaining.iter().enumerate() {
            let KeySegment::Key(name) = segment else {
                continue;
            };
            segments.push(segment.clone());
            let existing = if index == 0 && !name.starts_with(['@', '#']) {
                self.matching_children(Some(parent), name).len()
            } else {
                0
            };
            if existing > 0 {
                segments.push(KeySegment::Index(
                    i64::try_from(existing).unwrap_or(i64::MAX),
                ));
            }
            keys.push(print_key(&segments));
        }
        keys
    }

    /// Edits that remove the nodes addressed by a key path.
    ///
    /// An element is removed with its bound comments and comments on the
    /// same line; when it fills its lines, the lines are removed. An
    /// attribute is removed with the whitespace before it, a text chunk
    /// without the whitespace around it.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`XmlTree::resolve`] and
    /// `E_UNSUPPORTED_CONSTRUCT` `xml.document_root` for the document
    /// element.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::keypath::KeyPath;
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a>\n  <b/> <!-- old -->\n  <c/>\n</a>")?;
    /// let change = tree.remove(&KeyPath::parse("a.b")?)?;
    /// assert_eq!(change.apply(tree.text())?, "<a>\n  <c/>\n</a>");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove(&self, key: &KeyPath) -> Result<XmlChange> {
        let mut change = XmlChange::default();
        for target in self.resolve(key)? {
            let part = match target {
                Target::Document => {
                    return Err(unsupported(
                        CONSTRUCT_DOCUMENT_ROOT,
                        "the document cannot be removed; remove the file instead",
                    ));
                }
                Target::Element(element) if element == self.root() => {
                    return Err(unsupported(
                        CONSTRUCT_DOCUMENT_ROOT,
                        "the root element cannot be removed; a document needs one root element",
                    ));
                }
                Target::Element(element) => self.remove_node(element),
                Target::Attribute { element, index } => {
                    let name = self
                        .element(element)
                        .and_then(|el| el.attributes.get(index))
                        .map(|attribute| self.attribute_name(attribute).to_owned())
                        .unwrap_or_default();
                    self.remove_attribute(element, &name)
                }
                Target::Text { node, .. } => self.replace_text(node, "")?,
            };
            change.merge(part);
        }
        Ok(change)
    }

    /// Edit that removes a node with its bound comments and the comments
    /// that follow it on the same line.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a><b/><c/></a>")?;
    /// let b = tree.child_elements(tree.root()).next().unwrap();
    /// assert_eq!(tree.remove_node(b).apply(tree.text())?, "<a><c/></a>");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn remove_node(&self, node: NodeId) -> XmlChange {
        let start = self
            .bound_comments(node)
            .first()
            .map_or(self.node(node).span().start, |comment| {
                self.node(*comment).span().start
            });
        let end = self.end_with_line_comments(node);
        let line_start = self.line_start(start);
        let line_end = self.line_end(end);
        let alone = trim_xml(&self.text()[line_start..start]).is_empty()
            && trim_xml(&self.text()[end..line_end]).is_empty();
        let range = if alone && line_end < self.text().len() {
            let after = self.text()[line_end..]
                .find('\n')
                .map_or(self.text().len(), |newline| line_end + newline + 1);
            line_start..after
        } else {
            start..end
        };
        let mut change = XmlChange::default();
        change.push(XmlEdit::new(range, String::new()));
        change
    }

    /// `true` when `element` satisfies a filter expression evaluated on its
    /// value.
    pub(crate) fn filter_matches(&self, element: NodeId, expr: &FilterExpr) -> Result<bool> {
        let value = self.element_value(element)?;
        Ok(truthy(evaluate(&value, expr).as_ref()))
    }
}

/// Nested value that the missing segments after the first one describe.
fn nest(rest: &[KeySegment], value: &Value) -> Result<Value> {
    let mut nested = value.clone();
    for segment in rest.iter().rev() {
        match segment {
            KeySegment::Key(name) => {
                let mut map = Fields::new();
                map.insert(name.clone(), nested);
                nested = Value::Map(map);
            }
            KeySegment::Index(0) | KeySegment::Append => {}
            _ => {
                return Err(key_not_found(
                    "missing elements are created from names only; remove indexes and wildcards from the missing part",
                ));
            }
        }
    }
    Ok(nested)
}

/// Canonical text of key segments.
fn print_key(segments: &[KeySegment]) -> String {
    KeyPath::new(None, segments.to_vec())
        .map(|key| key.to_string())
        .unwrap_or_default()
}

/// Written attributes, content markup and whether the content holds
/// elements.
type Members = (Vec<(String, String)>, String, bool);

/// Markup writer for new elements in the scope of an existing element.
struct Render<'a> {
    tree: &'a XmlTree,
    scope: NodeId,
}

impl<'a> Render<'a> {
    fn new(tree: &'a XmlTree, scope: NodeId) -> Self {
        Self { tree, scope }
    }

    /// Elements named `name`, one per item, separated as siblings at
    /// `indent`.
    fn siblings(&self, name: &str, items: &[Value], indent: &str) -> Result<String> {
        let separator = if self.tree.style().multiline {
            format!("{}{indent}", self.tree.style().eol)
        } else {
            String::new()
        };
        let mut parts = Vec::with_capacity(items.len());
        for item in items {
            if matches!(item, Value::List(_)) {
                return Err(unsupported(
                    CONSTRUCT_NESTED_LIST,
                    "a list item that is itself a list has no XML form; wrap the inner list in a map",
                ));
            }
            parts.push(self.element(name, item, indent, &[])?);
        }
        Ok(parts.join(&separator))
    }

    /// Element `name` with `value`, its first line at `indent`.
    fn element(
        &self,
        name: &str,
        value: &Value,
        indent: &str,
        declared: &[String],
    ) -> Result<String> {
        match shape(value) {
            Shape::Null => Ok(format!("<{name}/>")),
            Shape::Scalar(text) => {
                check_representable(&text)?;
                Ok(format!(
                    "<{name}>{}</{name}>",
                    escape_text(&text, self.tree.style().eol)
                ))
            }
            Shape::List(_) => Err(unsupported(
                CONSTRUCT_NESTED_LIST,
                "a list item that is itself a list has no XML form; wrap the inner list in a map",
            )),
            Shape::Map(fields) => self.map_element(name, fields, indent, declared),
        }
    }

    fn map_element(
        &self,
        name: &str,
        fields: &Fields,
        indent: &str,
        declared: &[String],
    ) -> Result<String> {
        let mut scope: Vec<String> = declared.to_vec();
        for member_name in fields.keys() {
            if let Ok(Member::Attribute(attribute)) = member(member_name)
                && let Some(Some(prefix)) = declared_prefix(attribute)
            {
                scope.push(prefix.to_owned());
            }
        }
        let child_indent = format!("{indent}{}", self.tree.style().indent_unit);
        let prefix = split_qname(name).0;
        let (attributes, content, block) = self.members(fields, prefix, &child_indent, &scope)?;
        let quote = self.tree.style().quote;
        let mut out = format!("<{name}");
        for (attribute, text) in &attributes {
            out.push_str(&format!(
                " {attribute}={quote}{}{quote}",
                escape_attribute(text, quote)
            ));
        }
        if content.is_empty() {
            out.push_str("/>");
        } else if block && self.tree.style().multiline {
            let eol = self.tree.style().eol;
            out.push_str(&format!(
                ">{eol}{child_indent}{content}{eol}{indent}</{name}>"
            ));
        } else {
            out.push_str(&format!(">{content}</{name}>"));
        }
        Ok(out)
    }

    /// Attributes, content markup and whether the content holds elements,
    /// for a map value of an element whose prefix is `prefix`.
    fn members(
        &self,
        fields: &Fields,
        prefix: Option<&str>,
        child_indent: &str,
        declared: &[String],
    ) -> Result<Members> {
        let mut attributes = Vec::new();
        let mut text = String::new();
        let mut children = Vec::new();
        for (name, value) in fields {
            match member(name)? {
                Member::Attribute(attribute) => {
                    let written = self.attribute_name(attribute, declared)?;
                    let value = scalar_text(value, "an attribute")?;
                    check_representable(&value)?;
                    attributes.push((written, value));
                }
                Member::Content => text = self.content_text(value)?,
                Member::Instruction(target) => return Err(instruction_error(target)),
                Member::Child(child) => {
                    let written = self.child_name(child, prefix, declared)?;
                    match shape(value) {
                        Shape::List(items) => {
                            for item in items {
                                if matches!(item, Value::List(_)) {
                                    return Err(unsupported(
                                        CONSTRUCT_NESTED_LIST,
                                        "a list item that is itself a list has no XML form; wrap the inner list in a map",
                                    ));
                                }
                                children.push(self.element(
                                    &written,
                                    item,
                                    child_indent,
                                    declared,
                                )?);
                            }
                        }
                        _ => {
                            children.push(self.element(&written, value, child_indent, declared)?)
                        }
                    }
                }
            }
        }
        if !text.is_empty() && !children.is_empty() {
            return Err(unsupported(
                CONSTRUCT_MIXED_CONTENT,
                "text together with child elements has no defined layout; set the text and the elements with separate keys",
            ));
        }
        let block = !children.is_empty();
        let separator = if self.tree.style().multiline {
            format!("{}{child_indent}", self.tree.style().eol)
        } else {
            String::new()
        };
        let content = if block {
            children.join(&separator)
        } else {
            text
        };
        Ok((attributes, content, block))
    }

    /// Escaped text of a `+content` member.
    fn content_text(&self, value: &Value) -> Result<String> {
        match shape(value) {
            Shape::Null => Ok(String::new()),
            Shape::Scalar(text) => {
                check_representable(&text)?;
                Ok(escape_text(&text, self.tree.style().eol))
            }
            Shape::Map(_) | Shape::List(_) => Err(unsupported(
                CONSTRUCT_MIXED_CONTENT,
                "several text chunks need markup between them; set each `#text[n]` separately",
            )),
        }
    }

    fn attribute_name(&self, name: &str, declared: &[String]) -> Result<String> {
        if let (Some(prefix), _) = split_qname(name)
            && declared.iter().any(|known| known == prefix)
        {
            return Ok(name.to_owned());
        }
        let written = self.tree.written_name(self.scope, None, name, false).ok_or_else(|| {
            unsupported(
                CONSTRUCT_NAMESPACE_DECLARATION,
                format!("namespace of attribute `{name}` has no prefix in scope; declare it with an `+@xmlns:` member"),
            )
        })?;
        valid_name(written)
    }

    fn child_name(&self, name: &str, prefix: Option<&str>, declared: &[String]) -> Result<String> {
        if let (Some(own), _) = split_qname(name)
            && declared.iter().any(|known| known == own)
        {
            return valid_name(name.to_owned());
        }
        let written = self
            .tree
            .written_name(self.scope, prefix, name, true)
            .ok_or_else(|| {
                unsupported(
                    CONSTRUCT_NAMESPACE_DECLARATION,
                    format!("namespace of `{name}` has no prefix in scope; declare it with an `+@xmlns:` member"),
                )
            })?;
        valid_name(written)
    }
}

/// Checks a written name.
fn valid_name(name: String) -> Result<String> {
    if is_valid_name(&name) {
        Ok(name)
    } else {
        Err(invalid_value(format!(
            "`{name}` is not a valid XML name; names start with a letter, `_` or `:`"
        )))
    }
}

/// Value of a filter operand; `None` when a field does not exist.
fn evaluate(value: &Value, expr: &FilterExpr) -> Option<Value> {
    match expr {
        FilterExpr::Field { path } => field(value, path),
        FilterExpr::Literal { value: literal } => Some(literal_value(literal)),
        FilterExpr::Compare { op, left, right } => Some(Value::Bool(compare(
            *op,
            evaluate(value, left).as_ref(),
            evaluate(value, right).as_ref(),
        ))),
        FilterExpr::And { left, right } => Some(Value::Bool(
            truthy(evaluate(value, left).as_ref()) && truthy(evaluate(value, right).as_ref()),
        )),
        FilterExpr::Or { left, right } => Some(Value::Bool(
            truthy(evaluate(value, left).as_ref()) || truthy(evaluate(value, right).as_ref()),
        )),
        FilterExpr::Not { operand } => {
            Some(Value::Bool(!truthy(evaluate(value, operand).as_ref())))
        }
        _ => None,
    }
}

/// Member of an element value at a filter path; `@name` and `#text`
/// address attributes and text.
fn field(value: &Value, path: &[String]) -> Option<Value> {
    let mut current = value.clone();
    for name in path {
        current = match (&current, name.as_str()) {
            (Value::Text(_), TEXT_SEGMENT) => current.clone(),
            (Value::Map(map), TEXT_SEGMENT) => map.get(CONTENT_KEY)?.clone(),
            (Value::Map(map), name) if name.starts_with(ATTRIBUTE_SEGMENT_PREFIX) => map
                .get(&format!(
                    "{ATTRIBUTE_KEY_PREFIX}{}",
                    &name[ATTRIBUTE_SEGMENT_PREFIX.len_utf8()..]
                ))?
                .clone(),
            (Value::Map(map), name) => map.get(name)?.clone(),
            _ => return None,
        };
    }
    Some(current)
}

fn literal_value(literal: &FilterLiteral) -> Value {
    match literal {
        FilterLiteral::Text(text) => Value::Text(text.clone()),
        FilterLiteral::Int(number) => Value::Int(*number),
        FilterLiteral::Number(number) => Value::Number(*number),
        FilterLiteral::Bool(flag) => Value::Bool(*flag),
        _ => Value::Null,
    }
}

/// `true` for values other than missing, `null` and `false`.
fn truthy(value: Option<&Value>) -> bool {
    !matches!(value, None | Some(Value::Null | Value::Bool(false)))
}

/// Number held by a value; XML text is compared as a number when it
/// parses as one.
fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Int(number) => number.to_string().parse().ok(),
        Value::Number(number) => Some(*number),
        Value::Text(text) => trim_xml(text).parse().ok(),
        _ => None,
    }
}

/// Text form of a scalar for comparison.
fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Int(number) => Some(number.to_string()),
        _ => None,
    }
}

fn compare(op: CompareOp, left: Option<&Value>, right: Option<&Value>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        let both_null = !truthy_null(left) && !truthy_null(right);
        return match op {
            CompareOp::Eq => both_null,
            CompareOp::Ne => !both_null,
            _ => false,
        };
    };
    let ordering = match (as_number(left), as_number(right)) {
        (Some(a), Some(b)) => a.partial_cmp(&b),
        _ => match (as_text(left), as_text(right)) {
            (Some(a), Some(b)) => Some(a.cmp(&b)),
            _ => (left == right).then_some(std::cmp::Ordering::Equal),
        },
    };
    match op {
        CompareOp::Eq => ordering == Some(std::cmp::Ordering::Equal),
        CompareOp::Ne => ordering != Some(std::cmp::Ordering::Equal),
        CompareOp::Lt => ordering == Some(std::cmp::Ordering::Less),
        CompareOp::Le => matches!(
            ordering,
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ),
        CompareOp::Gt => ordering == Some(std::cmp::Ordering::Greater),
        CompareOp::Ge => matches!(
            ordering,
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        ),
    }
}

/// `true` when an operand is present and not `null`.
fn truthy_null(value: Option<&Value>) -> bool {
    !matches!(value, None | Some(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_members_become_lists() {
        let chunks = Value::List(vec![Value::Text("x".into()), Value::Text("y".into())]);
        let map = group(vec![
            ("a".into(), chunks.clone()),
            ("b".into(), Value::Null),
            ("a".into(), Value::Text("3".into())),
        ]);
        assert_eq!(map["a"], Value::List(vec![chunks, Value::Text("3".into())]));
        assert_eq!(map["b"], Value::Null);
    }

    #[test]
    fn model_of_map_orders_content_attributes_children() {
        let mut fields = Fields::new();
        fields.insert("child".into(), Value::Int(1));
        fields.insert("@id".into(), Value::Bool(true));
        fields.insert("#text".into(), Value::Text("t".into()));
        let model = model_of(&Value::Map(fields)).unwrap();
        let Value::Map(map) = model else {
            panic!("expected a map")
        };
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        assert_eq!(keys, ["+content", "+@id", "child"]);
        assert_eq!(map["child"], Value::Text("1".into()));
    }

    #[test]
    fn filter_compares_text_as_number() {
        let value = Value::Text("10".into());
        assert!(compare(CompareOp::Gt, Some(&value), Some(&Value::Int(9))));
        assert!(compare(
            CompareOp::Eq,
            Some(&value),
            Some(&Value::Text("10".into()))
        ));
        assert!(compare(CompareOp::Eq, None, Some(&Value::Null)));
        assert!(!compare(CompareOp::Lt, None, Some(&Value::Int(1))));
    }

    #[test]
    fn nest_wraps_value_in_missing_elements() {
        let rest = [KeySegment::Key("b".into()), KeySegment::Key("@id".into())];
        let nested = nest(&rest, &Value::Text("x".into())).unwrap();
        assert_eq!(
            nested.to_plain_json(),
            serde_json::json!({"b": {"@id": "x"}})
        );
    }
}
