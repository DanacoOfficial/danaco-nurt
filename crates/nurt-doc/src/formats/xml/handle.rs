// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registered format `xml` and its document handle.
//!
//! [`XmlHandle`] presents an [`XmlTree`] as a tree of the document model
//! whose values follow [`XmlTree::to_value`]: the document is a map of the
//! XML declaration (`+p_xml`), the document type declaration
//! (`+directive`), top-level processing instructions and the document
//! element. An element without attributes, child elements and processing
//! instructions and with at most one text chunk is a scalar; any other
//! element is an element node with the members `+content` (a text node, or
//! a list of text nodes for several chunks), `+@name` (attributes), child
//! elements and `+p_target`. Keys of the model (`a.+@id`, `a.+content[1]`)
//! and keys of [`XmlTree::resolve`] (`a.@id`, `a.#text[1]`) select the same
//! nodes.
//!
//! Every change passes [`guard::commit`]. The expected value of the
//! document is derived on the value model from the requested operation,
//! independently of the markup edits, and a reader built directly on the
//! `quick-xml` event stream reads the changed bytes back for the
//! comparison. Processing instructions, the XML declaration and the
//! document type declaration are kept as written.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ByteRange, DocFormatId, LineEnding, SyntaxIssue, TextCodecService, TextDetection, TextEncoding,
    TextPosition, TextSpan,
};
use nurt_contracts::value::{Fields, Value};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::cdata::{EntitySource, is_xml_char, trim_xml};
use super::encoding::{XmlEncoding, parse_document};
use super::ns::{declared_prefix, split_qname};
use super::tree::{
    ATTRIBUTE_SEGMENT_PREFIX, Node, NodeId, Resolution, TEXT_SEGMENT, Target, XmlTree,
};
use super::{
    ATTRIBUTE_KEY_PREFIX, CONTENT_KEY, DIRECTIVE_KEY, PI_KEY_PREFIX, XmlChange, key_not_found,
};
use crate::constructs::{
    self, DOC_ROUND_TRIP_LOSS, DOC_SEMANTIC_MISMATCH, XML_DTD_INTERNAL_SUBSET,
    XML_PROCESSING_INSTRUCTION,
};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{
    DocHandle, DocTree, FormatSemantics, Layout, MapSemantics, NodeId as ModelId, NodeKind,
    NodeSpec, Repeated, Resolved, ScalarStyle, SetOptions, Span, Style, TreeBuilder, doc_parse,
};
use crate::registry::{Confidence, DocFormat, DocOptions, InputKind, issue_of};

/// Rules of key paths in XML: elements sharing a name form a group
/// addressed by index.
const SEMANTICS: XmlSemantics = XmlSemantics;

/// Characters that XML treats as whitespace around a text value.
const XML_SPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// Key rules and value normal form of XML documents.
///
/// Key paths follow [`MapSemantics::CONFLICT`]. Values are compared in the
/// form a re-read document has: scalars are untyped text without surrounding
/// whitespace, empty text is `null`, a group of one element reads as the
/// element, and an element with nothing but text reads as that text.
#[derive(Debug, Clone, Copy)]
struct XmlSemantics;

impl FormatSemantics for XmlSemantics {
    fn repeated(&self) -> Repeated {
        MapSemantics::CONFLICT.repeated()
    }

    fn canonical(&self, value: Value) -> Value {
        canonical_value(&value)
    }
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Map(fields) => {
            let mut out = Fields::new();
            for (name, member) in fields {
                let member = if name.starts_with(ATTRIBUTE_KEY_PREFIX)
                    || name.starts_with(PI_KEY_PREFIX)
                    || name == DIRECTIVE_KEY
                {
                    Some(Value::Text(scalar_text(member)))
                } else if name == CONTENT_KEY {
                    Some(canonical_value(member)).filter(|content| *content != Value::Null)
                } else {
                    canonical_group(member)
                };
                if let Some(member) = member {
                    out.insert(name.clone(), member);
                }
            }
            if out.keys().all(|name| name == CONTENT_KEY) {
                out.shift_remove(CONTENT_KEY).unwrap_or(Value::Null)
            } else {
                Value::Map(out)
            }
        }
        Value::List(items) => match items.as_slice() {
            [] => Value::Null,
            [only] => canonical_value(only),
            _ => Value::List(items.iter().map(canonical_value).collect()),
        },
        scalar => {
            let text = scalar_text(scalar);
            let trimmed = text.trim_matches(XML_SPACE);
            if trimmed.is_empty() {
                Value::Null
            } else {
                Value::Text(trimmed.to_owned())
            }
        }
    }
}

fn canonical_group(value: &Value) -> Option<Value> {
    match value {
        Value::List(items) => match items.as_slice() {
            [] => None,
            [only] => Some(canonical_value(only)),
            _ => Some(Value::List(items.iter().map(canonical_value).collect())),
        },
        other => Some(canonical_value(other)),
    }
}

/// Flag of a change that created elements or attributes.
const KEYS_CREATED_FLAG: &str = "keys_created";

/// Byte order mark of UTF-8.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Start of a UTF-16 little-endian document: byte order mark and `<`.
const UTF16_LE_START: &[u8] = b"\xFF\xFE<\x00";

/// Start of a UTF-16 big-endian document: byte order mark and `<`.
const UTF16_BE_START: &[u8] = b"\xFE\xFF\x00<";

/// Start of the XML declaration.
const DECLARATION_OPEN: &str = "<?xml";

/// End of a processing instruction.
const PI_CLOSE: &str = "?>";

/// Start of a processing instruction.
const PI_OPEN: &str = "<?";

/// Start of a markup declaration.
const DECLARATION_MARKUP_OPEN: &str = "<!";

/// End of a markup declaration.
const DECLARATION_MARKUP_CLOSE: &str = ">";

/// Start of a CDATA section.
const CDATA_OPEN: &str = "<![CDATA[";

/// End of a CDATA section.
const CDATA_CLOSE: &str = "]]>";

/// Target of the XML declaration in the document value.
const DECLARATION_TARGET: &str = "xml";

/// File name extensions of XML vocabularies.
const EXTENSIONS: [&str; 16] = [
    "xml", "svg", "xhtml", "csproj", "props", "targets", "rss", "atom", "xlf", "xliff", "xsd",
    "xsl", "xslt", "nuspec", "resx", "wsdl",
];

/// Codec of the encodings an XML document may declare beyond UTF-8,
/// US-ASCII and ISO-8859-1: UTF-16 and the encodings with a WHATWG label.
#[derive(Debug, Clone, Copy, Default)]
struct LabelCodec;

/// The codec used for every XML document.
const CODEC: LabelCodec = LabelCodec;

impl LabelCodec {
    fn lookup(label: &str) -> Result<&'static encoding_rs::Encoding> {
        encoding_rs::Encoding::for_label(label.as_bytes()).ok_or_else(|| {
            Error::new(
                ErrorCode::Encoding,
                format!("unknown encoding `{label}`; convert the file to UTF-8"),
            )
        })
    }

    /// Byte order of a UTF-16 label: `Some(true)` for big endian.
    fn utf16(label: &str) -> Option<bool> {
        match label {
            "utf-16le" => Some(false),
            "utf-16be" => Some(true),
            _ => None,
        }
    }
}

impl TextCodecService for LabelCodec {
    fn detect(&self, bytes: &[u8]) -> TextDetection {
        let encoding = match bytes {
            [0xEF, 0xBB, 0xBF, ..] => labelled("utf-8", true),
            [0xFF, 0xFE, ..] => labelled("utf-16le", true),
            [0xFE, 0xFF, ..] => labelled("utf-16be", true),
            _ => TextEncoding::utf8(),
        };
        TextDetection {
            binary: false,
            encoding: Some(encoding),
            line_ending: line_ending(bytes),
        }
    }

    fn decode(&self, bytes: &[u8], encoding: &TextEncoding) -> Result<String> {
        Self::lookup(&encoding.label)?
            .decode_without_bom_handling_and_without_replacement(bytes)
            .map(std::borrow::Cow::into_owned)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::Encoding,
                    format!(
                        "the bytes are not valid {}; check the declared encoding",
                        encoding.label
                    ),
                )
            })
    }

    fn encode(&self, text: &str, encoding: &TextEncoding) -> Result<Vec<u8>> {
        if let Some(big_endian) = Self::utf16(&encoding.label) {
            return Ok(text
                .encode_utf16()
                .flat_map(|unit| {
                    if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect());
        }
        let (bytes, _, unmappable) = Self::lookup(&encoding.label)?.encode(text);
        if unmappable {
            return Err(Error::new(
                ErrorCode::Encoding,
                format!(
                    "the text has characters that {} cannot hold; use character references",
                    encoding.label
                ),
            ));
        }
        Ok(bytes.into_owned())
    }
}

fn labelled(label: &str, bom: bool) -> TextEncoding {
    TextEncoding {
        label: label.to_owned(),
        bom,
    }
}

/// Line ending convention of ASCII-compatible bytes.
fn line_ending(bytes: &[u8]) -> LineEnding {
    let mut found: Option<LineEnding> = None;
    let mut index = 0;
    while index < bytes.len() {
        let ending = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                index += 1;
                LineEnding::CrLf
            }
            b'\r' => LineEnding::Cr,
            b'\n' => LineEnding::Lf,
            _ => {
                index += 1;
                continue;
            }
        };
        index += 1;
        match found {
            None => found = Some(ending),
            Some(previous) if previous != ending => return LineEnding::Mixed,
            Some(_) => {}
        }
    }
    found.unwrap_or(LineEnding::None)
}

/// Map from offsets of the decoded text to offsets of the file bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ByteOffsets {
    /// Length of the byte order mark.
    bom: usize,
    /// Byte offset of every character boundary of the text, for encodings
    /// other than UTF-8; indexes between boundaries are unused.
    table: Option<Vec<usize>>,
}

impl ByteOffsets {
    fn new(encoding: &XmlEncoding, text: &str) -> Result<Self> {
        let bom = encoding.encode("", Some(&CODEC))?.len();
        if encoding.label() == "utf-8" {
            return Ok(Self { bom, table: None });
        }
        let mut table = vec![0; text.len() + 1];
        let mut at = bom;
        let mut buffer = [0_u8; 4];
        for (index, c) in text.char_indices() {
            table[index] = at;
            let encoded = encoding.encode(c.encode_utf8(&mut buffer), Some(&CODEC))?;
            at += encoded.len().saturating_sub(bom);
        }
        table[text.len()] = at;
        Ok(Self {
            bom,
            table: Some(table),
        })
    }

    fn byte(&self, offset: usize) -> usize {
        match &self.table {
            None => self.bom + offset,
            Some(table) => table
                .get(offset)
                .or_else(|| table.last())
                .copied()
                .unwrap_or(self.bom),
        }
    }

    fn span(&self, range: &Range<usize>) -> Span {
        Span::new(self.byte(range.start), self.byte(range.end))
    }
}

/// The format `xml`.
///
/// Documents are read as bytes: the encoding comes from the byte order mark
/// and the XML declaration, and edited text is written back in the same
/// encoding, with character references for characters the encoding cannot
/// hold.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::XmlFormat;
/// use nurt_doc::registry::{Confidence, DocFormat, DocOptions};
///
/// assert_eq!(XmlFormat.sniff(b"<?xml version=\"1.0\"?><a/>"), Confidence::High);
/// let handle = XmlFormat.open(b"<a x='1'/>".to_vec(), &DocOptions::default())?;
/// assert_eq!(handle.text(), b"<a x='1'/>");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct XmlFormat;

impl DocFormat for XmlFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::XML
    }

    fn extensions(&self) -> &[&'static str] {
        &EXTENSIONS
    }

    /// A text that starts with the XML declaration is a document of the
    /// format; a text that starts with another markup construct or a start
    /// tag probably is one.
    fn sniff(&self, content: &[u8]) -> Confidence {
        if content.starts_with(UTF16_LE_START) || content.starts_with(UTF16_BE_START) {
            return Confidence::Medium;
        }
        let content = content.strip_prefix(UTF8_BOM).unwrap_or(content);
        let start = content
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(content.len());
        let rest = &content[start..];
        if rest.starts_with(DECLARATION_OPEN.as_bytes())
            && rest
                .get(DECLARATION_OPEN.len())
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'?')
        {
            return Confidence::High;
        }
        match rest {
            [b'<', b'!' | b'?', ..] => Confidence::Medium,
            [b'<', next, ..]
                if next.is_ascii_alphabetic() || matches!(next, b'_' | b':') || *next >= 0x80 =>
            {
                Confidence::Medium
            }
            _ => Confidence::None,
        }
    }

    fn input(&self) -> InputKind {
        InputKind::Bytes
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        Ok(Box::new(XmlHandle::open(content)?))
    }

    /// The error of a failed opening as one issue at the line, column and
    /// byte of its location in the decoded text.
    fn check_syntax(&self, content: &[u8], options: &DocOptions) -> Vec<SyntaxIssue> {
        match self.open(content.to_vec(), options) {
            Ok(_) => Vec::new(),
            Err(error) => vec![syntax_issue(&in_file_bytes(error, content))],
        }
    }
}

/// `error` with the byte of its location counted in the bytes of the file.
///
/// Parse errors locate the decoded text, which has no byte order mark and,
/// for encodings other than UTF-8, other byte lengths than the file. Errors
/// of detecting and decoding the file already count file bytes.
fn in_file_bytes(error: Error, content: &[u8]) -> Error {
    let Ok(encoding) = XmlEncoding::detect(content) else {
        return error;
    };
    let Ok(text) = encoding.decode(content, Some(&CODEC)) else {
        return error;
    };
    let Some(mut location) = error.data().location.clone() else {
        return error;
    };
    let Some(byte) = location.byte.and_then(|byte| usize::try_from(byte).ok()) else {
        return error;
    };
    let Ok(offsets) = ByteOffsets::new(&encoding, &text) else {
        return error;
    };
    location.byte = u64::try_from(offsets.byte(byte)).ok();
    error.with_location(location)
}

/// Syntax issue of an error located by [`Error::with_location`].
fn syntax_issue(error: &Error) -> SyntaxIssue {
    let mut issue = issue_of(error);
    if let Some(location) = &error.data().location {
        let number = |value: Option<u64>| {
            value
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(1)
        };
        let position = TextPosition {
            line: number(location.line),
            column: number(location.column),
        };
        let byte = location.byte.unwrap_or_default();
        issue.span = TextSpan {
            bytes: ByteRange::new(byte, byte),
            start: position,
            end: position,
        };
    }
    issue
}

/// Node of the document an element of the model stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    Document,
    Element(NodeId),
    Attribute(NodeId, usize),
    Text(NodeId, NodeId),
    Chunks(NodeId),
    Instruction(NodeId),
    Directive(NodeId),
}

impl Slot {
    const fn of(target: Target) -> Self {
        match target {
            Target::Document => Self::Document,
            Target::Element(element) => Self::Element(element),
            Target::Attribute { element, index } => Self::Attribute(element, index),
            Target::Text { element, node } => Self::Text(element, node),
        }
    }
}

/// Open XML document.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::xml::XmlHandle;
/// use nurt_doc::model::{DocHandle, SetOptions};
///
/// let text = "<project>\n  <version>1.0</version>\n</project>\n";
/// let mut handle = XmlHandle::open(text.as_bytes().to_vec())?;
/// let key = KeyPath::parse("project.version")?;
/// handle.set(&key, &Value::Text("1.1".into()), &SetOptions::default())?;
/// assert_eq!(handle.text(), b"<project>\n  <version>1.1</version>\n</project>\n");
/// let created = KeyPath::parse("project.@id")?;
/// handle.set(&created, &Value::Int(7), &SetOptions { create: true })?;
/// assert_eq!(handle.text(), b"<project id=\"7\">\n  <version>1.1</version>\n</project>\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct XmlHandle {
    bytes: Vec<u8>,
    encoding: XmlEncoding,
    offsets: ByteOffsets,
    lossless: bool,
    xml: XmlTree,
    value: Value,
    model: DocTree,
    slots: HashMap<ModelId, Slot>,
    nodes: HashMap<Slot, ModelId>,
}

impl XmlHandle {
    /// Opens a document from the bytes of a file.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` for a document that is not well-formed or
    /// declares an unsupported encoding, `E_ENCODING` for bytes that do not
    /// convert back to themselves, `E_LIMIT` for nesting or entity
    /// expansion beyond the limits, and `E_UNSUPPORTED_CONSTRUCT` naming
    /// `xml.event_span` for a construct without an exact byte range.
    pub fn open(bytes: Vec<u8>) -> Result<Self> {
        let (encoding, xml) = parse_document(&bytes, Some(&CODEC))?;
        let lossless = encoding
            .encode(xml.text(), Some(&CODEC))
            .is_ok_and(|encoded| encoded == bytes);
        let offsets = ByteOffsets::new(&encoding, xml.text())?;
        let value = xml.to_value()?;
        let (model, slots) = ModelBuilder::new(&xml, &offsets, &bytes).build()?;
        let nodes = slots.iter().map(|(node, slot)| (*slot, *node)).collect();
        Ok(Self {
            bytes,
            encoding,
            offsets,
            lossless,
            xml,
            value,
            model,
            slots,
            nodes,
        })
    }

    /// The lossless XML tree of the decoded text.
    #[must_use]
    pub const fn xml(&self) -> &XmlTree {
        &self.xml
    }

    /// Encoding of the file.
    #[must_use]
    pub const fn encoding(&self) -> &XmlEncoding {
        &self.encoding
    }

    fn reference(&self) -> XmlReference {
        XmlReference {
            entities: self.xml.resolver.0.clone(),
        }
    }

    /// Span of the file bytes that a target may change.
    fn target_span(&self, target: Target) -> Span {
        match target {
            Target::Document => Span::new(0, self.bytes.len()),
            Target::Element(element) => self.offsets.span(&element_range(&self.xml, element)),
            Target::Attribute { element, index } => self
                .xml
                .element(element)
                .and_then(|el| el.attributes.get(index))
                .map_or_else(
                    || self.offsets.span(&element_range(&self.xml, element)),
                    |attribute| self.offsets.span(&attribute.span),
                ),
            Target::Text { node, .. } => self.offsets.span(&run_range(&self.xml, node)),
        }
    }

    /// Model node of a target; a text chunk of a scalar element is the
    /// element itself.
    fn node_of(&self, target: Target) -> Option<ModelId> {
        let slot = Slot::of(target);
        self.nodes.get(&slot).copied().or_else(|| match target {
            Target::Text { element, .. } => self.nodes.get(&Slot::Element(element)).copied(),
            _ => None,
        })
    }

    fn set_special(&self, key: &KeyPath, value: &Value, create: bool) -> Result<EditReport> {
        let found = self.model.resolve(key, &SEMANTICS)?;
        if found.is_empty() {
            return Err(if create {
                kept_as_written(false)
            } else {
                key_not_found(format!(
                    "key {key} does not exist; list the document keys to find the name"
                ))
            });
        }
        for resolved in &found {
            let current = self.model[resolved.node].value.as_ref();
            if !current.is_some_and(|current| same_text(value, current)) {
                return Err(self.special_error(resolved.node));
            }
        }
        Ok(EditReport::default())
    }

    fn special_error(&self, node: ModelId) -> Error {
        kept_as_written(matches!(self.slots.get(&node), Some(Slot::Directive(_))))
    }

    /// Checks a change with the guards and takes the new bytes.
    fn commit(
        &mut self,
        change: &XmlChange,
        targets: Vec<Span>,
        expected: Value,
        mut report: EditReport,
    ) -> Result<EditReport> {
        for fact in &change.notable {
            report.flag(fact);
        }
        let rewritten = if change.is_empty() {
            self.bytes.clone()
        } else if self.lossless {
            self.encoding
                .encode_change(&self.xml, change, Some(&CODEC))?
        } else {
            return Err(constructs::unsupported(
                DOC_ROUND_TRIP_LOSS,
                "the decoded text of the file does not convert back to the same bytes",
            ));
        };
        let mut plan = EditPlan::from_rewrite(&self.bytes, &rewritten, targets);
        if let Some(edits) = self.exact_edits(change, &rewritten)? {
            plan.edits = edits;
        }
        plan.report = report;
        let expectation = Expectation::new(vec![expected], &SEMANTICS);
        let committed = guard::commit(&self.bytes, plan, &expectation, &self.reference())?;
        if committed.report.is_unchanged() {
            return Ok(committed.report);
        }
        *self = Self::open(committed.bytes)
            .map_err(|error| mismatch("the changed document does not open").with_cause(error))?;
        Ok(committed.report)
    }

    /// Byte edits that replace exactly the ranges of `change`, or `None`
    /// when they do not rebuild `rewritten`.
    ///
    /// A difference of the two texts is ambiguous where the removed or
    /// inserted bytes repeat their surroundings, so it can place an edit
    /// outside the changed node.
    fn exact_edits(&self, change: &XmlChange, rewritten: &[u8]) -> Result<Option<Vec<ByteEdit>>> {
        let mut edits = Vec::with_capacity(change.edits.len());
        for edit in &change.edits {
            let single = XmlChange {
                edits: vec![edit.clone()],
                ..XmlChange::default()
            };
            let bytes = self
                .encoding
                .encode_change(&self.xml, &single, Some(&CODEC))?;
            let span = self.offsets.span(&edit.range);
            let Some(tail) = self.bytes.len().checked_sub(span.end) else {
                return Ok(None);
            };
            let Some(end) = bytes
                .len()
                .checked_sub(tail)
                .filter(|end| *end >= span.start)
            else {
                return Ok(None);
            };
            if bytes[..span.start] != self.bytes[..span.start]
                || bytes[end..] != self.bytes[span.end..]
            {
                return Ok(None);
            }
            edits.push(ByteEdit::replace(span, &bytes[span.start..end]));
        }
        edits.sort_by_key(|edit| edit.span.start);
        let rebuilt = crate::edit::apply(&self.bytes, &edits)?;
        Ok((rebuilt.bytes == rewritten).then_some(edits))
    }

    fn expected_set(&self, resolution: &Resolution, key: &KeyPath, value: &Value) -> Result<Value> {
        let model = ValueModel { xml: &self.xml };
        let mut document = self.value.clone();
        match resolution {
            Resolution::Found(targets) => {
                for target in self.reverse_order(targets) {
                    model.set(&mut document, target, value)?;
                }
            }
            Resolution::Missing { parent, segment } => {
                let parent = parent.ok_or_else(|| mismatch("the key has no parent element"))?;
                let remaining = key.segments().get(*segment..).unwrap_or_default();
                model.create(&mut document, parent, remaining, value)?;
            }
        }
        Ok(document)
    }

    fn expected_remove(&self, targets: &[Target]) -> Result<Value> {
        let model = ValueModel { xml: &self.xml };
        let mut document = self.value.clone();
        for target in self.reverse_order(targets) {
            model.remove(&mut document, target)?;
        }
        Ok(document)
    }

    /// Targets from the last in the document to the first, so that a
    /// change of one keeps the indexes of the others.
    fn reverse_order(&self, targets: &[Target]) -> Vec<Target> {
        let mut ordered = targets.to_vec();
        ordered.sort_by_key(|target| std::cmp::Reverse(self.target_span(*target).start));
        ordered
    }
}

impl DocHandle for XmlHandle {
    fn format(&self) -> DocFormatId {
        DocFormatId::XML
    }

    fn text(&self) -> &[u8] {
        &self.bytes
    }

    fn tree(&self) -> &DocTree {
        &self.model
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        &SEMANTICS
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        let Some(xml_key) = xml_key(key) else {
            return self.set_special(key, value, options.create);
        };
        let resolution = self.xml.resolve_key(&xml_key)?;
        if matches!(&resolution, Resolution::Found(targets) if targets == &[Target::Document])
            && crate::value::equivalent(value, &self.value)
        {
            return Ok(EditReport::default());
        }
        let change = self.xml.set(&xml_key, value, options.create)?;
        let (targets, count): (Vec<Span>, usize) = match &resolution {
            Resolution::Found(targets) => (
                targets.iter().map(|t| self.target_span(*t)).collect(),
                targets.len(),
            ),
            Resolution::Missing { parent, .. } => (
                vec![parent.map_or(Span::new(0, self.bytes.len()), |parent| {
                    self.target_span(Target::Element(parent))
                })],
                1,
            ),
        };
        let expected = if change.is_empty() {
            self.value.clone()
        } else {
            self.expected_set(&resolution, &xml_key, value)?
        };
        let mut report = EditReport {
            replacements: u64::try_from(count).unwrap_or(u64::MAX),
            ..EditReport::default()
        };
        if !change.keys_created.is_empty() {
            report.note(Notable::KeysCreated, Some(change.keys_created.join(", ")));
            report.flag(KEYS_CREATED_FLAG);
        }
        self.commit(&change, targets, expected, report)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let Some(xml_key) = xml_key(key) else {
            let found = self.model.resolve(key, &SEMANTICS)?;
            return Err(found.first().map_or_else(
                || {
                    key_not_found(format!(
                        "key {key} does not exist; list the document keys to find the name"
                    ))
                },
                |resolved| self.special_error(resolved.node),
            ));
        };
        let targets = self.xml.resolve(&xml_key)?;
        let change = self.xml.remove(&xml_key)?;
        let expected = self.expected_remove(&targets)?;
        let spans = targets.iter().map(|t| self.target_span(*t)).collect();
        let report = EditReport {
            replacements: u64::try_from(targets.len()).unwrap_or(u64::MAX),
            ..EditReport::default()
        };
        self.commit(&change, spans, expected, report)
    }

    /// Nodes selected by `key`, in model form (`+@id`, `+content`) or in
    /// the key syntax of [`XmlTree::resolve`] (`@id`, `#text`, prefixes and
    /// `{uri}name`).
    fn get(&self, key: &KeyPath) -> Result<Vec<Resolved>> {
        let Some(xml_key) = xml_key(key) else {
            return self.model.resolve(key, &SEMANTICS);
        };
        Ok(self
            .xml
            .resolve(&xml_key)?
            .into_iter()
            .filter_map(|target| self.node_of(target))
            .map(|node| Resolved {
                node,
                key: self.model.path_of(node, &SEMANTICS),
            })
            .collect())
    }

    fn to_value(&self) -> Result<Value> {
        Ok(self.value.clone())
    }
}

/// Key in the syntax of [`XmlTree::resolve`] for a key of the model:
/// `+@name` becomes `@name` and `+content` becomes `#text`. `None` for keys
/// through a processing instruction or the document type declaration.
fn xml_key(key: &KeyPath) -> Option<KeyPath> {
    let mut segments = Vec::with_capacity(key.segments().len());
    for segment in key.segments() {
        let translated = match segment {
            KeySegment::Key(name) if name.starts_with(PI_KEY_PREFIX) || name == DIRECTIVE_KEY => {
                return None;
            }
            KeySegment::Key(name) if name == CONTENT_KEY => {
                KeySegment::Key(TEXT_SEGMENT.to_owned())
            }
            KeySegment::Key(name) => match name.strip_prefix(ATTRIBUTE_KEY_PREFIX) {
                Some(attribute) => {
                    KeySegment::Key(format!("{ATTRIBUTE_SEGMENT_PREFIX}{attribute}"))
                }
                None => segment.clone(),
            },
            other => other.clone(),
        };
        segments.push(translated);
    }
    KeyPath::new(key.document(), segments).ok()
}

/// Error for a change of a construct that is kept as written.
fn kept_as_written(directive: bool) -> Error {
    if directive {
        constructs::unsupported(
            XML_DTD_INTERNAL_SUBSET,
            "the document type declaration is kept as written",
        )
    } else {
        constructs::unsupported(
            XML_PROCESSING_INSTRUCTION,
            "processing instructions and the XML declaration are kept as written",
        )
    }
}

fn mismatch(message: &str) -> Error {
    constructs::unsupported(DOC_SEMANTIC_MISMATCH, message)
}

/// Returns `true` when `value` written as text equals the text `current`,
/// exactly or without surrounding whitespace.
fn same_text(value: &Value, current: &Value) -> bool {
    let text = scalar_text(value);
    *current == Value::Text(text.clone()) || *current == Value::Text(trim_xml(&text).to_owned())
}

/// Range of an element in the model: from its bound comments to the end of
/// the comments that follow it on its last line.
fn element_range(xml: &XmlTree, element: NodeId) -> Range<usize> {
    let start = xml
        .bound_comments(element)
        .first()
        .map_or(xml.node(element).span().start, |comment| {
            xml.node(*comment).span().start
        });
    start..xml.end_with_line_comments(element)
}

/// Range of the run of text and CDATA nodes that starts at `node`.
fn run_range(xml: &XmlTree, node: NodeId) -> Range<usize> {
    let run = xml.run(node);
    let start = run.first().map_or(xml.node(node).span().start, |first| {
        xml.node(*first).span().start
    });
    let end = run
        .last()
        .map_or(xml.node(node).span().end, |last| xml.node(*last).span().end);
    start..end
}

/// Builder of the model tree of a document.
struct ModelBuilder<'a> {
    xml: &'a XmlTree,
    offsets: &'a ByteOffsets,
    length: usize,
    builder: TreeBuilder<'a>,
    slots: HashMap<ModelId, Slot>,
}

impl<'a> ModelBuilder<'a> {
    fn new(xml: &'a XmlTree, offsets: &'a ByteOffsets, bytes: &'a [u8]) -> Self {
        Self {
            xml,
            offsets,
            length: bytes.len(),
            builder: TreeBuilder::new(bytes),
            slots: HashMap::new(),
        }
    }

    fn build(mut self) -> Result<(DocTree, HashMap<ModelId, Slot>)> {
        let root = self.builder.node(
            None,
            NodeSpec::new(NodeKind::Map, Span::new(0, self.length)),
        );
        self.slots.insert(root, Slot::Document);
        for id in self.xml.document_nodes() {
            self.document_member(root, *id)?;
        }
        Ok((self.builder.finish(), self.slots))
    }

    fn document_member(&mut self, root: ModelId, id: NodeId) -> Result<()> {
        let xml = self.xml;
        let text = xml.text();
        match xml.node(id) {
            Node::Declaration(declaration) => {
                let raw = between(&text[declaration.span.clone()], DECLARATION_OPEN, PI_CLOSE);
                let spec = self
                    .scalar(&declaration.span, trim_xml(raw))
                    .with_name(format!("{PI_KEY_PREFIX}{DECLARATION_TARGET}"));
                self.add(root, spec, Slot::Instruction(id));
            }
            Node::DocType(doctype) => {
                let raw = between(
                    &text[doctype.span.clone()],
                    DECLARATION_MARKUP_OPEN,
                    DECLARATION_MARKUP_CLOSE,
                );
                let spec = self.scalar(&doctype.span, raw).with_name(DIRECTIVE_KEY);
                self.add(root, spec, Slot::Directive(id));
            }
            Node::ProcessingInstruction(_) => self.instruction(root, id),
            Node::Element(_) => self.element(root, id)?,
            _ => {}
        }
        Ok(())
    }

    fn scalar(&self, range: &Range<usize>, text: &str) -> NodeSpec {
        NodeSpec::new(NodeKind::Scalar, self.offsets.span(range))
            .with_value(Value::Text(text.to_owned()))
    }

    fn add(&mut self, parent: ModelId, spec: NodeSpec, slot: Slot) -> ModelId {
        let id = self.builder.node(Some(parent), spec);
        self.slots.insert(id, slot);
        id
    }

    fn instruction(&mut self, parent: ModelId, id: NodeId) {
        let xml = self.xml;
        if let Node::ProcessingInstruction(pi) = xml.node(id) {
            let text = xml.text();
            let spec = self
                .scalar(&pi.span, trim_xml(&text[pi.content.clone()]))
                .with_name(format!("{PI_KEY_PREFIX}{}", &text[pi.target.clone()]));
            self.add(parent, spec, Slot::Instruction(id));
        }
    }

    fn element(&mut self, parent: ModelId, element: NodeId) -> Result<()> {
        let xml = self.xml;
        let Some(el) = xml.element(element) else {
            return Ok(());
        };
        let chunks = xml.text_chunks(element);
        let structured = !el.attributes.is_empty()
            || el.children.iter().any(|child| {
                matches!(
                    xml.node(*child),
                    Node::Element(_) | Node::ProcessingInstruction(_)
                )
            })
            || chunks.len() > 1;
        let mut spec = NodeSpec::new(
            if structured {
                NodeKind::Element
            } else {
                NodeKind::Scalar
            },
            self.offsets.span(&element_range(xml, element)),
        )
        .with_name(xml.name(element))
        .with_key_span(self.offsets.span(&el.name));
        if let Some(comment) = xml.comment_before(element) {
            spec = spec.with_comment(comment);
        }
        if !structured {
            let value_span = if el.is_empty_tag() {
                el.span.clone()
            } else {
                el.content()
            };
            spec = spec
                .with_value_span(self.offsets.span(&value_span))
                .with_value(xml.element_value(element)?);
            self.add(parent, spec, Slot::Element(element));
            return Ok(());
        }
        spec = spec
            .with_value_span(self.offsets.span(&el.span))
            .with_style(Style {
                layout: Layout::Block,
                ..Style::default()
            });
        let id = self.add(parent, spec, Slot::Element(element));
        self.content(id, element, &chunks)?;
        self.attributes(id, element)?;
        for child in &el.children {
            match xml.node(*child) {
                Node::Element(_) => self.element(id, *child)?,
                Node::ProcessingInstruction(_) => self.instruction(id, *child),
                _ => {}
            }
        }
        Ok(())
    }

    fn content(&mut self, parent: ModelId, element: NodeId, chunks: &[NodeId]) -> Result<()> {
        match chunks {
            [] => Ok(()),
            [chunk] => self.chunk(parent, element, *chunk, true),
            [first, .., last] => {
                let range = run_range(self.xml, *first).start..run_range(self.xml, *last).end;
                let spec =
                    NodeSpec::new(NodeKind::List, self.offsets.span(&range)).with_name(CONTENT_KEY);
                let list = self.add(parent, spec, Slot::Chunks(element));
                for chunk in chunks {
                    self.chunk(list, element, *chunk, false)?;
                }
                Ok(())
            }
        }
    }

    fn chunk(
        &mut self,
        parent: ModelId,
        element: NodeId,
        chunk: NodeId,
        named: bool,
    ) -> Result<()> {
        let value = self.xml.chunk_value(chunk)?.unwrap_or_default();
        let mut spec = NodeSpec::new(
            NodeKind::Text,
            self.offsets.span(&run_range(self.xml, chunk)),
        )
        .with_value(Value::Text(value));
        if named {
            spec = spec.with_name(CONTENT_KEY);
        }
        self.add(parent, spec, Slot::Text(element, chunk));
        Ok(())
    }

    fn attributes(&mut self, parent: ModelId, element: NodeId) -> Result<()> {
        let xml = self.xml;
        let Some(el) = xml.element(element) else {
            return Ok(());
        };
        for (index, attribute) in el.attributes.iter().enumerate() {
            let scalar = if attribute.quote == '\'' {
                ScalarStyle::SingleQuoted
            } else {
                ScalarStyle::DoubleQuoted
            };
            let spec = NodeSpec::new(NodeKind::Attribute, self.offsets.span(&attribute.span))
                .with_name(format!(
                    "{ATTRIBUTE_KEY_PREFIX}{}",
                    xml.attribute_name(attribute)
                ))
                .with_key_span(self.offsets.span(&attribute.name))
                .with_value_span(self.offsets.span(&attribute.value))
                .with_value(Value::Text(xml.attribute_value(attribute)?))
                .with_style(Style {
                    scalar,
                    ..Style::default()
                });
            self.add(parent, spec, Slot::Attribute(element, index));
        }
        Ok(())
    }
}

/// New value classified by the markup it produces, as the tree writes it.
enum Shape<'v> {
    Null,
    Scalar(String),
    Map(&'v Fields),
    List(&'v [Value]),
}

fn classify(value: &Value) -> Shape<'_> {
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

/// Text of a scalar value; empty for other values.
fn scalar_text(value: &Value) -> String {
    match classify(value) {
        Shape::Scalar(text) => text,
        Shape::Null | Shape::Map(_) | Shape::List(_) => String::new(),
    }
}

/// Text chunk an element holds after `text` is written into it; `None`
/// when only whitespace remains.
fn chunk_text(text: &str) -> Option<String> {
    let trimmed = trim_xml(text);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Value of a group of members: absent, one value or a list.
fn sequence(mut items: Vec<Value>) -> Option<Value> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(Value::List(items)),
    }
}

/// Element value from its text content and its other members.
fn element_form(content: Option<Value>, mut members: Fields) -> Value {
    if members.is_empty() {
        return content.unwrap_or(Value::Null);
    }
    if let Some(content) = content {
        members.insert(CONTENT_KEY.to_owned(), content);
    }
    Value::Map(members)
}

/// Collapses an element map without attributes, children and processing
/// instructions to its content.
fn collapse(slot: &mut Value) {
    let content = match slot {
        Value::Map(fields) if fields.keys().all(|name| name == CONTENT_KEY) => {
            fields.shift_remove(CONTENT_KEY).unwrap_or(Value::Null)
        }
        _ => return,
    };
    *slot = content;
}

/// The members of an element value, turning a scalar element into a map
/// with its content.
fn as_map(slot: &mut Value) -> Result<&mut Fields> {
    if !matches!(slot, Value::Map(_)) {
        let content = std::mem::take(slot);
        let mut fields = Fields::new();
        if content != Value::Null {
            fields.insert(CONTENT_KEY.to_owned(), content);
        }
        *slot = Value::Map(fields);
    }
    match slot {
        Value::Map(fields) => Ok(fields),
        _ => Err(mismatch("the changed element has no members")),
    }
}

/// Replaces the text chunks of an element value.
fn set_content(slot: &mut Value, chunks: Vec<String>) {
    let content = sequence(chunks.into_iter().map(Value::Text).collect());
    match slot {
        Value::Map(fields) => {
            match content {
                Some(content) => fields.insert(CONTENT_KEY.to_owned(), content),
                None => fields.shift_remove(CONTENT_KEY),
            };
            collapse(slot);
        }
        _ => *slot = content.unwrap_or(Value::Null),
    }
}

/// Member of the value at `segments`.
fn member<'v>(document: &'v mut Value, segments: &[KeySegment]) -> Result<&'v mut Value> {
    let mut current = document;
    for segment in segments {
        let next = match (current, segment) {
            (Value::Map(fields), KeySegment::Key(name)) => fields.get_mut(name),
            (Value::List(items), KeySegment::Index(index)) => usize::try_from(*index)
                .ok()
                .and_then(|index| items.get_mut(index)),
            _ => None,
        };
        current = next.ok_or_else(|| mismatch("the value of the document has no changed node"))?;
    }
    Ok(current)
}

/// Value wrapped in maps for the missing names after the first one.
fn nest(rest: &[KeySegment], value: &Value) -> Result<Value> {
    let mut nested = value.clone();
    for segment in rest.iter().rev() {
        match segment {
            KeySegment::Key(name) => {
                let mut fields = Fields::new();
                fields.insert(name.clone(), nested);
                nested = Value::Map(fields);
            }
            KeySegment::Index(0) | KeySegment::Append => {}
            _ => return Err(mismatch("the missing part of the key is not a name")),
        }
    }
    Ok(nested)
}

/// Operations of the value model: the value a document has after a change,
/// derived from the value before it without the markup edits.
struct ValueModel<'a> {
    xml: &'a XmlTree,
}

impl ValueModel<'_> {
    fn set(&self, document: &mut Value, target: Target, value: &Value) -> Result<()> {
        match target {
            Target::Document => Err(mismatch("the document as a whole cannot change")),
            Target::Element(element) => self.set_element(document, element, value),
            Target::Attribute { element, index } => {
                self.set_attribute(document, element, index, Some(scalar_text(value)))
            }
            Target::Text { element, node } => {
                self.set_chunk(document, element, node, &scalar_text(value))
            }
        }
    }

    fn remove(&self, document: &mut Value, target: Target) -> Result<()> {
        match target {
            Target::Document => Err(mismatch("the document cannot be removed")),
            Target::Element(element) => self.splice(document, element, Vec::new()),
            Target::Attribute { element, index } => {
                self.set_attribute(document, element, index, None)
            }
            Target::Text { element, node } => self.set_chunk(document, element, node, ""),
        }
    }

    fn set_attribute(
        &self,
        document: &mut Value,
        element: NodeId,
        index: usize,
        value: Option<String>,
    ) -> Result<()> {
        let name = self
            .xml
            .element(element)
            .and_then(|el| el.attributes.get(index))
            .map(|attribute| self.xml.attribute_name(attribute).to_owned())
            .ok_or_else(|| mismatch("the attribute does not exist"))?;
        let slot = member(document, &self.xml.element_segments(element))?;
        let fields = as_map(slot)?;
        let key = format!("{ATTRIBUTE_KEY_PREFIX}{name}");
        match value {
            Some(value) => {
                fields.insert(key, Value::Text(value));
            }
            None => {
                fields.shift_remove(&key);
            }
        }
        collapse(slot);
        Ok(())
    }

    fn set_chunk(
        &self,
        document: &mut Value,
        element: NodeId,
        node: NodeId,
        text: &str,
    ) -> Result<()> {
        let mut chunks = Vec::new();
        for chunk in self.xml.text_chunks(element) {
            if chunk == node {
                chunks.extend(chunk_text(text));
            } else {
                chunks.push(self.xml.chunk_value(chunk)?.unwrap_or_default());
            }
        }
        let slot = member(document, &self.xml.element_segments(element))?;
        set_content(slot, chunks);
        Ok(())
    }

    fn set_element(&self, document: &mut Value, element: NodeId, value: &Value) -> Result<()> {
        let prefix = split_qname(self.xml.name(element)).0;
        let shape = classify(value);
        if let Shape::List(items) = &shape {
            let scope = self.xml.parent(element).unwrap_or(element);
            let written = items
                .iter()
                .map(|item| self.written(item, scope, prefix))
                .collect();
            return self.splice(document, element, written);
        }
        let slot = member(document, &self.xml.element_segments(element))?;
        let namespaces_only = matches!(shape, Shape::Map(_));
        let kept: Fields = match &*slot {
            Value::Map(fields) => fields
                .iter()
                .filter(|(name, _)| {
                    name.strip_prefix(ATTRIBUTE_KEY_PREFIX)
                        .is_some_and(|attribute| {
                            !namespaces_only || declared_prefix(attribute).is_some()
                        })
                })
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            _ => Fields::new(),
        };
        let fresh = match shape {
            Shape::Map(fields) => self.written_map(fields, element, prefix),
            Shape::Scalar(text) => chunk_text(&text).map_or(Value::Null, Value::Text),
            Shape::Null | Shape::List(_) => Value::Null,
        };
        *slot = with_attributes(fresh, kept);
        Ok(())
    }

    /// Replaces `element` in its group of same-named siblings by `items`.
    fn splice(&self, document: &mut Value, element: NodeId, items: Vec<Value>) -> Result<()> {
        let parent = self
            .xml
            .parent(element)
            .ok_or_else(|| mismatch("the document element cannot be replaced"))?;
        let name = self.xml.name(element);
        let same: Vec<NodeId> = self
            .xml
            .child_elements(parent)
            .filter(|child| self.xml.name(*child) == name)
            .collect();
        let position = same
            .iter()
            .position(|child| *child == element)
            .unwrap_or_default();
        let slot = member(document, &self.xml.element_segments(parent))?;
        let fields = as_map(slot)?;
        let mut group = match fields.shift_remove(name) {
            Some(Value::List(list)) if same.len() > 1 => list,
            Some(single) => vec![single],
            None => Vec::new(),
        };
        if position >= group.len() {
            return Err(mismatch("the group of the changed element is incomplete"));
        }
        group.splice(position..=position, items);
        if let Some(value) = sequence(group) {
            fields.insert(name.to_owned(), value);
        }
        collapse(slot);
        Ok(())
    }

    /// Adds the node created for the missing `remaining` segments below
    /// `parent`.
    fn create(
        &self,
        document: &mut Value,
        parent: NodeId,
        remaining: &[KeySegment],
        value: &Value,
    ) -> Result<()> {
        let slot = member(document, &self.xml.element_segments(parent))?;
        match remaining {
            [KeySegment::Key(name)] if name.starts_with(ATTRIBUTE_SEGMENT_PREFIX) => {
                let attribute = &name[ATTRIBUTE_SEGMENT_PREFIX.len_utf8()..];
                let written = self
                    .xml
                    .written_name(parent, None, attribute, false)
                    .unwrap_or_else(|| attribute.to_owned());
                as_map(slot)?.insert(
                    format!("{ATTRIBUTE_KEY_PREFIX}{written}"),
                    Value::Text(scalar_text(value)),
                );
                Ok(())
            }
            [KeySegment::Key(name), ..] if name == TEXT_SEGMENT => {
                set_content(slot, chunk_text(&scalar_text(value)).into_iter().collect());
                Ok(())
            }
            [KeySegment::Key(name), rest @ ..] => {
                let rest = match rest {
                    [KeySegment::Index(_) | KeySegment::Append, tail @ ..] => tail,
                    _ => rest,
                };
                let nested = nest(rest, value)?;
                let parent_prefix = split_qname(self.xml.name(parent)).0;
                let written = self
                    .xml
                    .written_name(parent, parent_prefix, name, true)
                    .ok_or_else(|| mismatch("the new element has no namespace prefix"))?;
                let prefix = split_qname(&written).0;
                let items = match classify(&nested) {
                    Shape::List(items) => items
                        .iter()
                        .map(|item| self.written(item, parent, prefix))
                        .collect(),
                    _ => vec![self.written(&nested, parent, prefix)],
                };
                let existing = self
                    .xml
                    .child_elements(parent)
                    .filter(|child| self.xml.name(*child) == written)
                    .count();
                add_children(slot, &written, existing, items)
            }
            _ => Err(mismatch("the missing part of the key is not a name")),
        }
    }

    /// Value that an element written from `value` reads back with; `prefix`
    /// is the prefix of the element name and `scope` the element in whose
    /// namespace scope it is written.
    fn written(&self, value: &Value, scope: NodeId, prefix: Option<&str>) -> Value {
        match classify(value) {
            Shape::Null => Value::Null,
            Shape::Scalar(text) => chunk_text(&text).map_or(Value::Null, Value::Text),
            Shape::List(items) => Value::List(
                items
                    .iter()
                    .map(|item| self.written(item, scope, prefix))
                    .collect(),
            ),
            Shape::Map(fields) => self.written_map(fields, scope, prefix),
        }
    }

    fn written_map(&self, fields: &Fields, scope: NodeId, prefix: Option<&str>) -> Value {
        let mut content = None;
        let mut members = Fields::new();
        for (name, value) in fields {
            if let Some(attribute) = name
                .strip_prefix(ATTRIBUTE_KEY_PREFIX)
                .or_else(|| name.strip_prefix(ATTRIBUTE_SEGMENT_PREFIX))
            {
                let written = self
                    .xml
                    .written_name(scope, None, attribute, false)
                    .unwrap_or_else(|| attribute.to_owned());
                members.insert(
                    format!("{ATTRIBUTE_KEY_PREFIX}{written}"),
                    Value::Text(scalar_text(value)),
                );
            } else if name == CONTENT_KEY || name == TEXT_SEGMENT {
                content = chunk_text(&scalar_text(value)).map(Value::Text);
            } else if name.starts_with(PI_KEY_PREFIX) {
                members.insert(
                    name.clone(),
                    Value::Text(trim_xml(&scalar_text(value)).to_owned()),
                );
            } else {
                let written = self
                    .xml
                    .written_name(scope, prefix, name, true)
                    .unwrap_or_else(|| name.clone());
                let child_prefix = split_qname(&written).0;
                let child = match classify(value) {
                    Shape::List(items) => sequence(
                        items
                            .iter()
                            .map(|item| self.written(item, scope, child_prefix))
                            .collect(),
                    ),
                    _ => Some(self.written(value, scope, child_prefix)),
                };
                if let Some(child) = child {
                    members.insert(written, child);
                }
            }
        }
        element_form(content, members)
    }
}

/// Element value `fresh` with the attributes `kept` that a change leaves
/// in place.
fn with_attributes(fresh: Value, kept: Fields) -> Value {
    if kept.is_empty() {
        return fresh;
    }
    let mut fields = match fresh {
        Value::Map(fields) => fields,
        Value::Null => Fields::new(),
        content => {
            let mut fields = Fields::new();
            fields.insert(CONTENT_KEY.to_owned(), content);
            fields
        }
    };
    for (name, value) in kept {
        fields.entry(name).or_insert(value);
    }
    Value::Map(fields)
}

/// Adds new child elements `name` to an element value that holds
/// `existing` children with that name.
fn add_children(slot: &mut Value, name: &str, existing: usize, items: Vec<Value>) -> Result<()> {
    let fields = as_map(slot)?;
    let mut group = match fields.shift_remove(name) {
        Some(Value::List(list)) if existing > 1 => list,
        Some(single) => vec![single],
        None => Vec::new(),
    };
    group.extend(items);
    if let Some(value) = sequence(group) {
        fields.insert(name.to_owned(), value);
    }
    Ok(())
}

/// Part of `raw` between the delimiters `open` and `close`.
fn between<'r>(raw: &'r str, open: &str, close: &str) -> &'r str {
    raw.get(open.len()..raw.len().saturating_sub(close.len()))
        .unwrap_or_default()
}

/// Reads XML into the value model of [`XmlTree::to_value`] from the
/// `quick-xml` event stream, without the lossless tree and its key rules.
#[derive(Clone, Default)]
struct XmlReference {
    entities: Option<Arc<dyn EntitySource>>,
}

impl ReferenceParser for XmlReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let encoding = XmlEncoding::detect(text)?;
        let decoded = encoding.decode(text, Some(&CODEC))?;
        let reader = EventReader {
            source: &decoded,
            entities: self.entities.as_deref(),
        };
        Ok(vec![reader.read()?])
    }
}

/// Element being read.
#[derive(Debug, Default)]
struct Frame {
    name: String,
    attributes: Vec<(String, String)>,
    chunks: Vec<String>,
    members: Vec<(String, Value)>,
}

impl Frame {
    fn value(self) -> Value {
        let mut chunks = self.chunks;
        let content = match chunks.len() {
            0 => None,
            1 => chunks.pop().map(Value::Text),
            _ => Some(Value::List(chunks.into_iter().map(Value::Text).collect())),
        };
        let members = group(self.members);
        if self.attributes.is_empty() && members.is_empty() {
            return content.unwrap_or(Value::Null);
        }
        let mut map = Fields::new();
        if let Some(content) = content {
            map.insert(CONTENT_KEY.to_owned(), content);
        }
        for (name, value) in self.attributes {
            map.insert(format!("{ATTRIBUTE_KEY_PREFIX}{name}"), Value::Text(value));
        }
        map.extend(members);
        Value::Map(map)
    }
}

/// Adjacent character data being read.
#[derive(Debug, Default)]
struct Run {
    text: String,
    substantive: bool,
}

impl Run {
    fn literal(&mut self, raw: &str) {
        self.substantive |= !trim_xml(raw).is_empty();
        self.text.push_str(&normalize_eol(raw));
    }

    fn reference(&mut self, text: &str) {
        self.substantive = true;
        self.text.push_str(text);
    }
}

/// Members of the document and the elements being read.
#[derive(Debug, Default)]
struct ReadState {
    document: Vec<(String, Value)>,
    stack: Vec<Frame>,
    run: Run,
    roots: usize,
}

impl ReadState {
    fn member(&mut self, name: String, value: Value) {
        match self.stack.last_mut() {
            Some(frame) => frame.members.push((name, value)),
            None => self.document.push((name, value)),
        }
    }
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

/// Line breaks normalized to `\n` (XML 1.0, 2.11).
fn normalize_eol(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Appends `text` with tabs and line breaks replaced by spaces (XML 1.0,
/// 3.3.3).
fn push_spaced(out: &mut String, text: &str) {
    out.extend(text.chars().map(|c| {
        if matches!(c, '\t' | '\n' | '\r') {
            ' '
        } else {
            c
        }
    }));
}

fn to_offset(position: u64) -> usize {
    usize::try_from(position).unwrap_or(usize::MAX)
}

/// Event reader of one decoded document.
struct EventReader<'a> {
    source: &'a str,
    entities: Option<&'a dyn EntitySource>,
}

impl EventReader<'_> {
    fn read(&self) -> Result<Value> {
        let source = self.source.strip_prefix('\u{FEFF}').unwrap_or(self.source);
        let base = self.source.len() - source.len();
        let mut reader = Reader::from_str(source);
        let config = reader.config_mut();
        config.trim_text(false);
        config.expand_empty_elements = false;
        config.check_end_names = true;
        config.check_comments = true;
        config.allow_dangling_amp = false;
        config.allow_unmatched_ends = false;
        let mut state = ReadState::default();
        loop {
            let start = to_offset(reader.buffer_position());
            let event = match reader.read_event() {
                Ok(Event::Eof) => break,
                Ok(event) => event,
                Err(error) => {
                    let at = base + to_offset(reader.error_position());
                    return Err(self.error(at, &error.to_string()));
                }
            };
            let end = to_offset(reader.buffer_position());
            let raw = source.get(start..end).unwrap_or_default();
            self.event(&event, raw, base + start, &mut state)?;
        }
        self.flush(&mut state, self.source.len())?;
        if !state.stack.is_empty() {
            return Err(self.error(self.source.len(), "an element is not closed"));
        }
        if state.roots == 0 {
            return Err(self.error(self.source.len(), "the document has no element"));
        }
        Ok(Value::Map(group(state.document)))
    }

    fn event(&self, event: &Event<'_>, raw: &str, at: usize, state: &mut ReadState) -> Result<()> {
        match event {
            Event::Text(_) => state.run.literal(raw),
            Event::GeneralRef(_) => {
                let name = between(raw, "&", ";");
                let text = self.entity(name, at)?;
                state.run.reference(&text);
            }
            Event::CData(_) => {
                let content = between(raw, CDATA_OPEN, CDATA_CLOSE);
                state.run.substantive |= !trim_xml(content).is_empty();
                state.run.text.push_str(&normalize_eol(content));
            }
            Event::Start(tag) => {
                self.flush(state, at)?;
                let frame = self.open(tag, at)?;
                state.stack.push(frame);
            }
            Event::Empty(tag) => {
                self.flush(state, at)?;
                let frame = self.open(tag, at)?;
                self.close(state, frame, at)?;
            }
            Event::End(_) => {
                self.flush(state, at)?;
                let frame = state
                    .stack
                    .pop()
                    .ok_or_else(|| self.error(at, "an end tag has no start tag"))?;
                self.close(state, frame, at)?;
            }
            Event::Comment(_) => self.flush(state, at)?,
            Event::PI(_) => {
                self.flush(state, at)?;
                let inner = between(raw, PI_OPEN, PI_CLOSE);
                let split = inner.find([' ', '\t', '\r', '\n']).unwrap_or(inner.len());
                state.member(
                    format!("{PI_KEY_PREFIX}{}", &inner[..split]),
                    Value::Text(trim_xml(&inner[split..]).to_owned()),
                );
            }
            Event::Decl(_) => state.document.push((
                format!("{PI_KEY_PREFIX}{DECLARATION_TARGET}"),
                Value::Text(trim_xml(between(raw, DECLARATION_OPEN, PI_CLOSE)).to_owned()),
            )),
            Event::DocType(_) => state.document.push((
                DIRECTIVE_KEY.to_owned(),
                Value::Text(
                    between(raw, DECLARATION_MARKUP_OPEN, DECLARATION_MARKUP_CLOSE).to_owned(),
                ),
            )),
            Event::Eof => {}
        }
        Ok(())
    }

    /// Ends the current run of character data.
    fn flush(&self, state: &mut ReadState, at: usize) -> Result<()> {
        let run = std::mem::take(&mut state.run);
        if !run.substantive {
            return Ok(());
        }
        match state.stack.last_mut() {
            Some(frame) => {
                frame.chunks.push(trim_xml(&run.text).to_owned());
                Ok(())
            }
            None => Err(self.error(at, "text outside the document element")),
        }
    }

    fn close(&self, state: &mut ReadState, frame: Frame, at: usize) -> Result<()> {
        if state.stack.is_empty() {
            if state.roots > 0 {
                return Err(self.error(at, "the document has a second root element"));
            }
            state.roots += 1;
        }
        let name = frame.name.clone();
        state.member(name, frame.value());
        Ok(())
    }

    fn open(&self, tag: &BytesStart<'_>, at: usize) -> Result<Frame> {
        let name: String = tag.name().as_ref().to_owned();
        let mut attributes = Vec::new();
        for attribute in tag.attributes() {
            let attribute = attribute.map_err(|error| self.error(at, &error.to_string()))?;
            let key: &str = attribute.key.as_ref();
            let raw: &str = &attribute.value;
            let value = self.attribute_value(raw, &name, key, at)?;
            attributes.push((key.to_owned(), value));
        }
        Ok(Frame {
            name,
            attributes,
            ..Frame::default()
        })
    }

    /// Normalized value of an attribute (XML 1.0, 3.3.3).
    fn attribute_value(&self, raw: &str, element: &str, name: &str, at: usize) -> Result<String> {
        let normalized = normalize_eol(raw);
        let mut out = String::with_capacity(raw.len());
        let mut rest = normalized.as_str();
        while let Some(amp) = rest.find('&') {
            push_spaced(&mut out, &rest[..amp]);
            let tail = &rest[amp + 1..];
            let end = tail.find(';').ok_or_else(|| {
                self.error(at, "a reference in an attribute value is not terminated")
            })?;
            let reference = &tail[..end];
            let replacement = self.entity(reference, at)?;
            if reference.starts_with('#') {
                out.push_str(&replacement);
            } else {
                push_spaced(&mut out, &replacement);
            }
            rest = &tail[end + 1..];
        }
        push_spaced(&mut out, rest);
        if self
            .entities
            .is_some_and(|entities| entities.tokenized(element, name))
        {
            out = out
                .split(' ')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
        }
        Ok(out)
    }

    /// Replacement text of a character or entity reference.
    fn entity(&self, name: &str, at: usize) -> Result<String> {
        if let Some(number) = name.strip_prefix('#') {
            let code = match number.strip_prefix('x') {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => number.parse().ok(),
            };
            return code
                .and_then(char::from_u32)
                .filter(|c| is_xml_char(*c))
                .map(String::from)
                .ok_or_else(|| self.error(at, "a character reference is not a valid character"));
        }
        let predefined = match name {
            "lt" => Some("<"),
            "gt" => Some(">"),
            "amp" => Some("&"),
            "apos" => Some("'"),
            "quot" => Some("\""),
            _ => None,
        };
        if let Some(text) = predefined {
            return Ok(text.to_owned());
        }
        self.entities
            .map(|entities| entities.resolve(name))
            .transpose()?
            .flatten()
            .ok_or_else(|| self.error(at, &format!("the entity `{name}` is not declared")))
    }

    fn error(&self, at: usize, message: &str) -> Error {
        doc_parse(&DocFormatId::XML, self.source.as_bytes(), Some(at), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREATE: SetOptions = SetOptions { create: true };

    fn open(text: &str) -> XmlHandle {
        XmlHandle::open(text.as_bytes().to_vec()).unwrap()
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    fn text(handle: &XmlHandle) -> &str {
        std::str::from_utf8(handle.text()).unwrap()
    }

    #[test]
    fn model_tree_values_match_the_value_of_the_document() {
        let handle = open(
            "<?xml version=\"1.0\"?>\n<?style a?>\n<r a=\"1\">\n  <!-- first -->\n  <b>x</b>\n  <b/>\n  <c>t<d/></c>\n  <?pi data?>\n</r>\n",
        );
        let tree = handle.tree();
        let root = tree.roots()[0];
        assert_eq!(
            tree.value_of(root, &SEMANTICS, None),
            handle.to_value().unwrap()
        );
        let found = handle.get(&key("r.b[0]")).unwrap();
        let node = &tree[found[0].node];
        assert_eq!(node.comment.as_deref(), Some("first"));
        assert_eq!(node.start.line, 5);
        assert_eq!(found[0].key, key("r.b[0]"));
    }

    #[test]
    fn model_and_xml_keys_select_the_same_nodes() {
        let handle = open("<a id=\"7\"><b>x<!-- c -->y</b></a>");
        let model = handle.get(&key("a.+@id")).unwrap();
        let xml = handle.get(&key("a.@id")).unwrap();
        assert_eq!(model[0].node, xml[0].node);
        let chunk = handle.get(&key("a.b.#text[1]")).unwrap();
        assert_eq!(
            handle.tree()[chunk[0].node].value,
            Some(Value::Text("y".into()))
        );
        let again = handle.get(&chunk[0].key).unwrap();
        assert_eq!(again[0].node, chunk[0].node);
        assert_eq!(handle.tree()[chunk[0].node].name, None);
    }

    #[test]
    fn sets_text_attributes_and_creates_elements() {
        let mut handle = open("<p:a xmlns:p=\"urn:p\">\n  <p:b>1</p:b>\n</p:a>\n");
        handle
            .set(&key("p:a.p:b"), &Value::Int(2), &SetOptions::default())
            .unwrap();
        let report = handle
            .set(&key("p:a.c"), &Value::Bool(true), &CREATE)
            .unwrap();
        assert_eq!(report.flags, vec![KEYS_CREATED_FLAG.to_owned()]);
        assert_eq!(report.notable[0].code, Notable::KeysCreated);
        handle
            .set(&key("p:a.p:b.@n"), &Value::Text("x\"y".into()), &CREATE)
            .unwrap();
        assert_eq!(
            text(&handle),
            "<p:a xmlns:p=\"urn:p\">\n  <p:b n=\"x&quot;y\">2</p:b>\n  <p:c>true</p:c>\n</p:a>\n"
        );
    }

    #[test]
    fn removes_one_of_repeated_elements() {
        let mut handle = open("<deps>\n  <dep>a</dep>\n  <dep>b</dep>\n</deps>\n");
        let report = handle.remove(&key("deps.dep[0]")).unwrap();
        assert_eq!(report.replacements, 1);
        assert_eq!(text(&handle), "<deps>\n  <dep>b</dep>\n</deps>\n");
        let error = handle.remove(&key("deps")).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(constructs::XML_DOCUMENT_ROOT));
    }

    #[test]
    fn replaces_an_element_by_a_list_and_a_map() {
        let mut handle = open("<a>\n  <b>1</b>\n  <c id=\"k\" xmlns:q=\"urn:q\">x</c>\n</a>\n");
        let list = Value::List(vec![Value::Text("1".into()), Value::Text("2".into())]);
        handle
            .set(&key("a.b"), &list, &SetOptions::default())
            .unwrap();
        let mut fields = Fields::new();
        fields.insert("@n".into(), Value::Int(1));
        fields.insert("d".into(), Value::Null);
        handle
            .set(&key("a.c"), &Value::Map(fields), &SetOptions::default())
            .unwrap();
        assert_eq!(
            text(&handle),
            "<a>\n  <b>1</b>\n  <b>2</b>\n  <c xmlns:q=\"urn:q\" n=\"1\">\n    <d/>\n  </c>\n</a>\n"
        );
    }

    #[test]
    fn setting_the_current_value_changes_nothing() {
        let source = "<?xml version=\"1.0\"?>\n<a x=\"1\"><?pi d?><b> t </b></a>\n";
        let mut handle = open(source);
        let whole = handle.to_value().unwrap();
        assert!(
            handle
                .set(&key("a"), &whole, &SetOptions::default())
                .is_err()
        );
        let report = handle
            .set(&KeyPath::default(), &whole, &SetOptions::default())
            .unwrap();
        assert!(report.is_unchanged());
        let report = handle
            .set(
                &key("a.+p_pi"),
                &Value::Text("d".into()),
                &SetOptions::default(),
            )
            .unwrap();
        assert!(report.is_unchanged());
        let report = handle
            .set(
                &key("a.b"),
                &Value::Text("t".into()),
                &SetOptions::default(),
            )
            .unwrap();
        assert!(report.is_unchanged());
        assert_eq!(text(&handle), source);
    }

    #[test]
    fn instructions_and_declarations_are_kept_as_written() {
        let mut handle = open("<?xml version=\"1.0\"?>\n<!DOCTYPE a>\n<a><?pi d?></a>");
        let error = handle
            .set(
                &key("a.+p_pi"),
                &Value::Text("e".into()),
                &SetOptions::default(),
            )
            .unwrap_err();
        assert_eq!(error.found.as_deref(), Some(XML_PROCESSING_INSTRUCTION));
        let error = handle.remove(&key("+directive")).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(XML_DTD_INTERNAL_SUBSET));
        let error = handle
            .set(&key("a.+p_new"), &Value::Text("e".into()), &CREATE)
            .unwrap_err();
        assert_eq!(error.found.as_deref(), Some(XML_PROCESSING_INSTRUCTION));
        assert_eq!(
            handle.remove(&key("a.+p_x")).unwrap_err().code,
            ErrorCode::KeyNotFound
        );
    }

    #[test]
    fn edits_documents_in_single_byte_and_utf16_encodings() {
        let latin = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<a>caf\xE9</a>\n".to_vec();
        let mut handle = XmlHandle::open(latin).unwrap();
        let node = handle.get(&key("a")).unwrap()[0].node;
        let span = handle.tree()[node].span;
        assert_eq!(&handle.text()[span.start..span.end], b"<a>caf\xE9</a>");
        handle
            .set(
                &key("a"),
                &Value::Text("th\u{E9} \u{20AC}".into()),
                &SetOptions::default(),
            )
            .unwrap();
        assert!(handle.text().ends_with(b"<a>th\xE9 &#x20AC;</a>\n"));

        let source = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a>x</a>";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(source.encode_utf16().flat_map(u16::to_le_bytes));
        let mut handle = XmlHandle::open(bytes).unwrap();
        handle
            .set(&key("a"), &Value::Text("y".into()), &SetOptions::default())
            .unwrap();
        let decoded: Vec<u16> = handle.text()[2..]
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(
            String::from_utf16(&decoded).unwrap(),
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?><a>y</a>"
        );
        assert_eq!(handle.encoding().label(), "utf-16le");
    }

    #[test]
    fn sniffs_xml_documents() {
        assert_eq!(
            XmlFormat.sniff(b"\xEF\xBB\xBF <?xml version='1.0'?>"),
            Confidence::High
        );
        assert_eq!(XmlFormat.sniff(b"<!-- c --><a/>"), Confidence::Medium);
        assert_eq!(XmlFormat.sniff(b"<svg/>"), Confidence::Medium);
        assert_eq!(XmlFormat.sniff(b"\xFF\xFE<\x00a\x00"), Confidence::Medium);
        assert_eq!(XmlFormat.sniff(b"<?xml-stylesheet x?>"), Confidence::Medium);
        assert_eq!(XmlFormat.sniff(b"{\"a\": 1}"), Confidence::None);
        assert_eq!(XmlFormat.sniff(b"< a"), Confidence::None);
        assert_eq!(XmlFormat.input(), InputKind::Bytes);
        assert!(XmlFormat.extensions().contains(&"csproj"));
    }

    #[test]
    fn reference_reader_follows_the_value_model() {
        let text = "<?xml version=\"1.0\"?><!DOCTYPE r [<!ENTITY e \"E\">]>\r\n<r a=\"x\ty &amp; &#10;\"><b>1</b><b><![CDATA[ 2 ]]></b>t&e;<!-- c -->u<?p q?></r>";
        let handle = open(text);
        let values = handle.reference().parse(text.as_bytes()).unwrap();
        assert_eq!(values, vec![handle.to_value().unwrap()]);
        let reference = XmlReference::default();
        for invalid in [
            "<a><b></a>",
            "<a/><b/>",
            "text<a/>",
            "<a>&undeclared;</a>",
            "<a>&#0;</a>",
            "",
        ] {
            assert!(reference.parse(invalid.as_bytes()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn rejects_changes_whose_meaning_differs() {
        let mut handle = open("<a>x<b/>y</a>");
        let error = handle.remove(&key("a.b")).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(DOC_SEMANTIC_MISMATCH));
        assert_eq!(text(&handle), "<a>x<b/>y</a>");
    }

    fn error_byte(bytes: &[u8]) -> u64 {
        let error = XmlHandle::open(bytes.to_vec()).unwrap_err();
        error
            .data()
            .location
            .as_ref()
            .and_then(|location| location.byte)
            .unwrap()
    }

    #[test]
    fn syntax_issue_counts_bytes_of_the_file_after_a_byte_order_mark() {
        let plain = b"<a><b></a>".to_vec();
        let marked = [UTF8_BOM, plain.as_slice()].concat();
        let issues = XmlFormat.check_syntax(&marked, &DocOptions::default());
        let expected = error_byte(&plain) + 3;
        assert_eq!(issues[0].span.bytes, ByteRange::new(expected, expected));
        let issues = XmlFormat.check_syntax(&plain, &DocOptions::default());
        let expected = error_byte(&plain);
        assert_eq!(issues[0].span.bytes, ByteRange::new(expected, expected));
    }

    #[test]
    fn syntax_issue_counts_bytes_of_the_file_in_a_single_byte_encoding() {
        let bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xE9<b></a>".to_vec();
        let in_text = error_byte(&bytes);
        let issues = XmlFormat.check_syntax(&bytes, &DocOptions::default());
        assert_eq!(
            issues[0].span.bytes,
            ByteRange::new(in_text - 1, in_text - 1)
        );
    }

    #[test]
    fn syntax_issue_of_a_decoding_error_keeps_the_offset_of_the_file() {
        let issues = XmlFormat.check_syntax(b"<a>\xFF</a>", &DocOptions::default());
        assert_eq!(issues[0].span.bytes, ByteRange::new(3, 3));
    }

    #[test]
    fn codec_detects_byte_order_marks_and_line_endings() {
        let detection = CODEC.detect(b"\xFF\xFEa\x00");
        assert_eq!(
            detection.encoding.map(|e| e.label),
            Some("utf-16le".to_owned())
        );
        assert_eq!(line_ending(b"a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(line_ending(b"a\nb\r"), LineEnding::Mixed);
        assert_eq!(line_ending(b"a"), LineEnding::None);
        assert!(
            CODEC
                .encode("\u{20AC}", &labelled("iso-8859-2", false))
                .is_err()
        );
        assert!(
            CODEC
                .decode(b"a", &labelled("no-such-label", false))
                .is_err()
        );
    }
}
