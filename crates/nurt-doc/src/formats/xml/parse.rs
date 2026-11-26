// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Construction of an [`XmlTree`] from the events of the `quick-xml`
//! reader.
//!
//! Every event is located by the reader position before and after it; the
//! ranges of consecutive events must tile the document. Parts of a tag are
//! located by scanning the bytes inside the event range.

use std::collections::HashSet;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Result};
use quick_xml::events::Event;
use quick_xml::reader::{Config, Reader};

use super::attr::{Attribute, scan_attributes, skip_whitespace};
use super::cdata::{ResolverSlot, is_xml_char, trim_xml};
use super::ns::{is_name_char, is_valid_name};
use super::style::{XmlStyle, detect};
use super::tree::{
    CDataSection, Comment, Declaration, DocType, Element, Node, NodeId, ProcessingInstruction,
    TextNode, XmlTree,
};
use super::{CONSTRUCT_EVENT_SPAN, doc_parse_error, located_error, unsupported};

/// Byte order mark encoded in UTF-8.
const BOM: char = '\u{FEFF}';

/// Opening delimiter of a CDATA section.
const CDATA_OPEN: &str = "<![CDATA[";

/// Closing delimiter of a CDATA section.
const CDATA_CLOSE: &str = "]]>";

/// Opening delimiter of a comment.
const COMMENT_OPEN: &str = "<!--";

/// Closing delimiter of a comment.
const COMMENT_CLOSE: &str = "-->";

/// Opening delimiter of a processing instruction.
const PI_OPEN: &str = "<?";

/// Closing delimiter of a processing instruction.
const PI_CLOSE: &str = "?>";

/// Opening delimiter of the XML declaration.
const DECLARATION_OPEN: &str = "<?xml";

/// Opening delimiter of a document type declaration.
const DOCTYPE_OPEN: &str = "<!DOCTYPE";

/// Deepest nesting of elements accepted; values and edits walk the tree
/// recursively, and this bound keeps them within the stack of a thread.
const MAX_DEPTH: usize = 512;

/// Parses `text` into a lossless tree.
pub(crate) fn parse(text: &str) -> Result<XmlTree> {
    let bom = if text.starts_with(BOM) {
        BOM.len_utf8()
    } else {
        0
    };
    check_characters(text, bom)?;
    let mut builder = Builder::new(text, bom);
    let mut reader = Reader::from_str(&text[bom..]);
    configure(reader.config_mut());
    loop {
        let start = bom + position(reader.buffer_position())?;
        let event = reader.read_event().map_err(|error| {
            let at = position(reader.error_position()).map_or(start, |offset| bom + offset);
            doc_parse_error(text, at, format!("malformed XML: {error}"))
        })?;
        let end = bom + position(reader.buffer_position())?;
        if start != builder.position || end < start {
            return Err(span_error("event"));
        }
        if matches!(event, Event::Eof) {
            break;
        }
        builder.event(&event, start..end)?;
        builder.position = end;
    }
    if builder.position != text.len() {
        return Err(span_error("end of document"));
    }
    builder.finish()
}

/// Reader settings that keep every byte inside some event.
fn configure(config: &mut Config) {
    config.trim_text(false);
    config.expand_empty_elements = false;
    config.check_end_names = true;
    config.check_comments = true;
    config.allow_dangling_amp = false;
    config.allow_unmatched_ends = false;
}

/// Converts a reader position to an offset.
fn position(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| span_error("position"))
}

/// Error for an event whose range cannot be established.
fn span_error(what: &str) -> Error {
    unsupported(
        CONSTRUCT_EVENT_SPAN,
        format!(
            "the reader reported no exact byte range for the {what}; the document cannot be edited without layout loss"
        ),
    )
}

/// Rejects characters outside the XML 1.0 `Char` production.
fn check_characters(text: &str, bom: usize) -> Result<()> {
    match text[bom..].char_indices().find(|(_, c)| !is_xml_char(*c)) {
        Some((offset, c)) => Err(doc_parse_error(
            text,
            bom + offset,
            format!(
                "character U+{:04X} is not allowed in XML 1.0; remove it",
                u32::from(c)
            ),
        )),
        None => Ok(()),
    }
}

/// Incremental construction of the node arena.
struct Builder<'a> {
    text: &'a str,
    bom: usize,
    nodes: Vec<Node>,
    parents: Vec<Option<NodeId>>,
    document: Vec<NodeId>,
    stack: Vec<NodeId>,
    root: Option<NodeId>,
    position: usize,
}

impl<'a> Builder<'a> {
    fn new(text: &'a str, bom: usize) -> Self {
        Self {
            text,
            bom,
            nodes: Vec::new(),
            parents: Vec::new(),
            document: Vec::new(),
            stack: Vec::new(),
            root: None,
            position: bom,
        }
    }

    fn error(&self, at: usize, message: impl Into<String>) -> Error {
        doc_parse_error(self.text, at, message)
    }

    fn event(&mut self, event: &Event<'_>, span: Range<usize>) -> Result<()> {
        match event {
            Event::Start(_) => self.start_tag(span, false),
            Event::Empty(_) => self.start_tag(span, true),
            Event::End(_) => self.end_tag(span),
            Event::Text(_) | Event::GeneralRef(_) => self.text_event(span),
            Event::CData(_) => self.cdata(span),
            Event::Comment(_) => self.comment(span),
            Event::Decl(_) => self.declaration(span),
            Event::PI(_) => self.processing_instruction(span),
            Event::DocType(_) => self.doctype(span),
            Event::Eof => Ok(()),
        }
    }

    fn push(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.nodes.len());
        let parent = self.stack.last().copied();
        self.nodes.push(node);
        self.parents.push(parent);
        match parent.and_then(|parent| self.element_mut(parent)) {
            Some(element) => element.children.push(id),
            None => self.document.push(id),
        }
        id
    }

    fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        match self.nodes.get_mut(id.0) {
            Some(Node::Element(element)) => Some(element),
            _ => None,
        }
    }

    /// Checks that `span` starts with `open` and ends with `close`.
    fn delimited(&self, span: &Range<usize>, open: &str, close: &str) -> Result<()> {
        let raw = &self.text[span.clone()];
        if raw.len() >= open.len() + close.len() && raw.starts_with(open) && raw.ends_with(close) {
            Ok(())
        } else {
            Err(span_error("markup delimiters"))
        }
    }

    fn start_tag(&mut self, span: Range<usize>, empty: bool) -> Result<()> {
        let close = if empty { "/>" } else { ">" };
        self.delimited(&span, "<", close)?;
        if self.stack.is_empty() && self.root.is_some() {
            return Err(self.error(
                span.start,
                "document has more than one root element; wrap the elements in a single root",
            ));
        }
        if self.stack.len() >= MAX_DEPTH {
            return Err(located_error(
                ErrorCode::Limit,
                self.text,
                span.start,
                format!("elements are nested deeper than {MAX_DEPTH} levels; flatten the document"),
            ));
        }
        let body_end = span.end - close.len();
        let name_end = self.text[span.start + 1..body_end]
            .find(|c: char| !is_name_char(c))
            .map_or(body_end, |offset| span.start + 1 + offset);
        let name = span.start + 1..name_end;
        if !is_valid_name(&self.text[name.clone()]) {
            return Err(self.error(
                span.start + 1,
                "element name is missing or invalid; names start with a letter, `_` or `:`",
            ));
        }
        let scanned = scan_attributes(self.text, name_end, body_end)?;
        self.check_duplicates(&scanned.attributes)?;
        let element = Element {
            span: span.clone(),
            start_tag: span.clone(),
            name,
            attributes: scanned.attributes,
            trailing: scanned.trailing,
            close: body_end..span.end,
            end_tag: None,
            children: Vec::new(),
        };
        let id = self.push(Node::Element(element));
        if self.stack.is_empty() {
            self.root = Some(id);
        }
        if !empty {
            self.stack.push(id);
        }
        Ok(())
    }

    fn check_duplicates(&self, attributes: &[Attribute]) -> Result<()> {
        let mut seen: HashSet<&str> = HashSet::with_capacity(attributes.len());
        for attribute in attributes {
            let name = &self.text[attribute.name.clone()];
            if !seen.insert(name) {
                return Err(self.error(
                    attribute.name.start,
                    format!("attribute `{name}` is repeated; keep one occurrence"),
                ));
            }
        }
        Ok(())
    }

    fn end_tag(&mut self, span: Range<usize>) -> Result<()> {
        self.delimited(&span, "</", ">")?;
        let Some(id) = self.stack.pop() else {
            return Err(self.error(span.start, "end tag without a start tag; remove it"));
        };
        let written =
            self.text[span.start + 2..span.end - 1].trim_end_matches([' ', '\t', '\n', '\r']);
        let text = self.text;
        let element = self.element_mut(id).ok_or_else(|| span_error("end tag"))?;
        if &text[element.name.clone()] != written {
            return Err(span_error("end tag name"));
        }
        element.end_tag = Some(span.clone());
        element.span.end = span.end;
        Ok(())
    }

    fn text_event(&mut self, span: Range<usize>) -> Result<()> {
        let raw = &self.text[span.clone()];
        if self.stack.is_empty() {
            if !trim_xml(raw).is_empty() {
                return Err(self.error(
                    span.start,
                    "text outside the root element; move it into an element or a comment",
                ));
            }
        } else if let Some(offset) = raw.find("]]>") {
            return Err(self.error(
                span.start + offset,
                "`]]>` is not allowed in text; write `]]&gt;`",
            ));
        }
        let previous = match self.stack.last() {
            Some(parent) => self
                .element_mut(*parent)
                .and_then(|element| element.children.last().copied()),
            None => self.document.last().copied(),
        };
        if let Some(Node::Text(text)) = previous.and_then(|id| self.nodes.get_mut(id.0))
            && text.span.end == span.start
        {
            text.span.end = span.end;
            return Ok(());
        }
        self.push(Node::Text(TextNode { span }));
        Ok(())
    }

    fn require_element(&self, at: usize, what: &str) -> Result<()> {
        if self.stack.is_empty() {
            Err(self.error(
                at,
                format!("{what} outside the root element; move it into an element"),
            ))
        } else {
            Ok(())
        }
    }

    fn cdata(&mut self, span: Range<usize>) -> Result<()> {
        self.delimited(&span, CDATA_OPEN, CDATA_CLOSE)?;
        self.require_element(span.start, "CDATA section")?;
        let content = span.start + CDATA_OPEN.len()..span.end - CDATA_CLOSE.len();
        self.push(Node::CData(CDataSection { span, content }));
        Ok(())
    }

    fn comment(&mut self, span: Range<usize>) -> Result<()> {
        self.delimited(&span, COMMENT_OPEN, COMMENT_CLOSE)?;
        let content = span.start + COMMENT_OPEN.len()..span.end - COMMENT_CLOSE.len();
        if self.text[content.clone()].ends_with('-') {
            return Err(self.error(
                span.start,
                "comment must not end with `--->`; remove the hyphen",
            ));
        }
        self.push(Node::Comment(Comment { span, content }));
        Ok(())
    }

    fn declaration(&mut self, span: Range<usize>) -> Result<()> {
        self.delimited(&span, DECLARATION_OPEN, PI_CLOSE)?;
        if span.start != self.bom || !self.nodes.is_empty() {
            return Err(self.error(
                span.start,
                "XML declaration must be at the start of the document; move or remove it",
            ));
        }
        let scanned = scan_attributes(
            self.text,
            span.start + DECLARATION_OPEN.len(),
            span.end - PI_CLOSE.len(),
        )?;
        let first = scanned
            .attributes
            .first()
            .map(|attribute| &self.text[attribute.name.clone()]);
        if first != Some("version") {
            return Err(self.error(
                span.start,
                "XML declaration must start with `version=\"1.0\"`",
            ));
        }
        self.push(Node::Declaration(Declaration {
            span,
            attributes: scanned.attributes,
        }));
        Ok(())
    }

    fn processing_instruction(&mut self, span: Range<usize>) -> Result<()> {
        self.delimited(&span, PI_OPEN, PI_CLOSE)?;
        let body_end = span.end - PI_CLOSE.len();
        let target_start = span.start + PI_OPEN.len();
        let target_end = self.text[target_start..body_end]
            .find(|c: char| !is_name_char(c))
            .map_or(body_end, |offset| target_start + offset);
        let target = &self.text[target_start..target_end];
        if !is_valid_name(target) || target.eq_ignore_ascii_case("xml") {
            return Err(self.error(
                target_start,
                format!("`{target}` is not a valid processing instruction target"),
            ));
        }
        let content_start = skip_whitespace(self.text, target_end, body_end);
        if content_start == target_end && target_end != body_end {
            return Err(self.error(
                target_end,
                "processing instruction target must be followed by whitespace",
            ));
        }
        self.push(Node::ProcessingInstruction(ProcessingInstruction {
            span,
            target: target_start..target_end,
            content: content_start..body_end,
        }));
        Ok(())
    }

    fn doctype(&mut self, span: Range<usize>) -> Result<()> {
        if !self.text[span.clone()].starts_with(DOCTYPE_OPEN) {
            return Err(self.error(
                span.start,
                "the keyword of a document type declaration is `DOCTYPE` in upper case; correct the declaration",
            ));
        }
        self.delimited(&span, DOCTYPE_OPEN, ">")?;
        let misplaced = self.root.is_some()
            || self
                .nodes
                .iter()
                .any(|node| matches!(node, Node::DocType(_)));
        if misplaced {
            return Err(self.error(
                span.start,
                "document type declaration must precede the root element and occur once",
            ));
        }
        let body_end = span.end - 1;
        let name_start = skip_whitespace(self.text, span.start + DOCTYPE_OPEN.len(), body_end);
        let name_end = self.text[name_start..body_end]
            .find(|c: char| !is_name_char(c))
            .map_or(body_end, |offset| name_start + offset);
        if name_start == span.start + DOCTYPE_OPEN.len()
            || !is_valid_name(&self.text[name_start..name_end])
        {
            return Err(self.error(
                span.start,
                "document type declaration needs the root element name",
            ));
        }
        let internal_subset = internal_subset(self.text, name_end, body_end);
        self.push(Node::DocType(DocType {
            span,
            name: name_start..name_end,
            internal_subset,
        }));
        Ok(())
    }

    fn finish(self) -> Result<XmlTree> {
        if let Some(open) = self.stack.last() {
            let at = self.nodes[open.0].span().start;
            return Err(self.error(at, "element is not closed; add the end tag"));
        }
        let Some(root) = self.root else {
            return Err(self.error(self.bom, "document has no root element"));
        };
        let mut line_starts = vec![0];
        line_starts.extend(self.text.match_indices('\n').map(|(offset, _)| offset + 1));
        let mut tree = XmlTree {
            text: self.text.to_owned(),
            nodes: self.nodes,
            parents: self.parents,
            document: self.document,
            root,
            line_starts,
            style: XmlStyle::default(),
            resolver: ResolverSlot::default(),
        };
        tree.style = detect(&tree);
        if tree.doctype().is_none() {
            tree.check_references()?;
        }
        Ok(tree)
    }
}

/// Internal subset of a document type declaration: from the first `[`
/// outside quoted literals to the last `]`.
fn internal_subset(text: &str, start: usize, end: usize) -> Option<Range<usize>> {
    let mut quote: Option<char> = None;
    for (offset, c) in text[start..end].char_indices() {
        match (quote, c) {
            (Some(open), _) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => {
                let open = start + offset + 1;
                let close = text[open..end].rfind(']')?;
                return Some(open..open + close);
            }
            (None, _) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_subset_ignores_brackets_in_literals() {
        let text = r#"<!DOCTYPE a SYSTEM "x[1].dtd" [ <!ENTITY e "]"> ]>"#;
        let subset = internal_subset(text, 11, text.len() - 1).unwrap();
        assert_eq!(&text[subset], r#" <!ENTITY e "]"> "#);
        assert_eq!(internal_subset("<!DOCTYPE a>", 11, 11), None);
    }

    #[test]
    fn repeated_attribute_names_are_rejected_wherever_they_are() {
        assert!(parse(r#"<a x="1" y="2" x="3"/>"#).is_err());
        assert!(parse(r#"<a x="1" X="2"/>"#).is_ok());
        assert!(parse(r#"<a p:x="1" q:x="2"/>"#).is_ok());
        let many: String = (0..2000).map(|index| format!(" a{index}=\"1\"")).collect();
        assert!(parse(&format!("<r{many}/>")).is_ok());
        assert!(parse(&format!("<r{many} a1999=\"2\"/>")).is_err());
    }

    #[test]
    fn nesting_beyond_the_limit_reports_a_limit_error() {
        let depth = MAX_DEPTH + 1;
        let text = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
        assert_eq!(parse(&text).unwrap_err().code(), ErrorCode::Limit);
    }
}
