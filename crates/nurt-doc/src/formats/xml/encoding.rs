// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Character encodings of XML documents.
//!
//! [`XmlEncoding::detect`] determines the encoding from the byte order mark
//! and the `encoding` pseudo-attribute of the XML declaration (XML 1.0,
//! section 4.3.3 and appendix F). Documents are decoded to text for parsing
//! and encoded back in the same encoding; the decoded text of an unchanged
//! document encodes to the original bytes.
//!
//! UTF-8, US-ASCII and ISO-8859-1 are handled directly. ISO-8859-1 follows
//! the IANA definition required by XML (every byte is the code point of the
//! same value), which differs from the WHATWG label `iso-8859-1` that names
//! windows-1252. UTF-16 and the other ASCII-compatible encodings with a
//! WHATWG label (`iso-8859-2`, `windows-1250`, `windows-1252` and others)
//! are decoded and encoded by the text codec service.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{TextCodecService, TextEncoding};

use super::dtd::{self, parse_error};
use super::entity;
use super::{XmlChange, XmlEdit, XmlTree};

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
const UTF16LE_BOM: &[u8] = &[0xFF, 0xFE];
const UTF16BE_BOM: &[u8] = &[0xFE, 0xFF];

const LATIN1_LABELS: [&str; 9] = [
    "iso-8859-1",
    "iso_8859-1",
    "iso_8859-1:1987",
    "latin1",
    "l1",
    "iso-ir-100",
    "ibm819",
    "cp819",
    "csisolatin1",
];

const ASCII_LABELS: [&str; 11] = [
    "us-ascii",
    "ascii",
    "ansi_x3.4-1968",
    "ansi_x3.4-1986",
    "iso-ir-6",
    "iso_646.irv:1991",
    "iso646-us",
    "us",
    "ibm367",
    "cp367",
    "csascii",
];

/// Highest code point of ISO-8859-1.
const LATIN1_MAX: char = '\u{FF}';

/// Highest code point of US-ASCII.
const ASCII_MAX: char = '\u{7F}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Utf8,
    Utf16Le,
    Utf16Be,
    AsciiCompatible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Charset {
    Utf8,
    Utf16Le,
    Utf16Be,
    Latin1,
    Ascii,
    Codec(String),
}

/// Place of text in a document, which decides how a character outside the
/// encoding is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlContext {
    /// Character data of an element.
    Text,
    /// Attribute value.
    Attribute,
    /// Comment.
    Comment,
    /// CDATA section.
    CData,
    /// Processing instruction.
    ProcessingInstruction,
    /// Markup other than an attribute value, such as a name.
    Name,
}

impl XmlContext {
    fn accepts_references(self) -> bool {
        matches!(self, Self::Text | Self::Attribute)
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Attribute => "an attribute value",
            Self::Comment => "a comment",
            Self::CData => "a CDATA section",
            Self::ProcessingInstruction => "a processing instruction",
            Self::Name => "a name",
        }
    }
}

/// Lexical state of markup, used to find the context of new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Markup {
    Text,
    Tag,
    Value(char),
    Comment,
    CData,
    Pi,
    /// Document type declaration; `subset` is set between `[` and `]`.
    Doctype {
        quote: Option<char>,
        subset: bool,
    },
}

const OPENERS: [(&str, Markup); 5] = [
    ("<!--", Markup::Comment),
    ("<![CDATA[", Markup::CData),
    ("<?", Markup::Pi),
    (
        "<!DOCTYPE",
        Markup::Doctype {
            quote: None,
            subset: false,
        },
    ),
    ("<", Markup::Tag),
];

impl Markup {
    fn context(self) -> XmlContext {
        match self {
            Self::Text => XmlContext::Text,
            Self::Tag | Self::Doctype { .. } => XmlContext::Name,
            Self::Value(_) => XmlContext::Attribute,
            Self::Comment => XmlContext::Comment,
            Self::CData => XmlContext::CData,
            Self::Pi => XmlContext::ProcessingInstruction,
        }
    }

    /// Step inside a document type declaration: `>` inside a quoted literal,
    /// a comment or a processing instruction of the internal subset does not
    /// end it, and `>` inside the subset ends only a markup declaration.
    fn doctype_step(
        &mut self,
        rest: &str,
        first: usize,
        quote: Option<char>,
        subset: bool,
    ) -> usize {
        if let Some(open) = quote {
            if rest.starts_with(open) {
                *self = Self::Doctype {
                    quote: None,
                    subset,
                };
            }
            return first;
        }
        if subset {
            for (opener, closer) in [("<!--", "-->"), ("<?", "?>")] {
                if let Some(after) = rest.strip_prefix(opener) {
                    return after
                        .find(closer)
                        .map_or(rest.len(), |at| opener.len() + at + closer.len());
                }
            }
        }
        match rest.as_bytes().first() {
            Some(&literal @ (b'"' | b'\'')) => {
                *self = Self::Doctype {
                    quote: Some(char::from(literal)),
                    subset,
                };
            }
            Some(b'[') => {
                *self = Self::Doctype {
                    quote: None,
                    subset: true,
                };
            }
            Some(b']') => {
                *self = Self::Doctype {
                    quote: None,
                    subset: false,
                };
            }
            Some(b'>') if !subset => *self = Self::Text,
            _ => {}
        }
        first
    }

    /// Consumes the token at the start of a non-empty `rest` and returns
    /// its length in bytes.
    fn step(&mut self, rest: &str) -> usize {
        let first = rest.chars().next().map_or(1, char::len_utf8);
        let closer = match *self {
            Self::Text => {
                if let Some((open, state)) = OPENERS.iter().find(|(open, _)| rest.starts_with(open))
                {
                    *self = *state;
                    return open.len();
                }
                return first;
            }
            Self::Tag => {
                match rest.as_bytes().first() {
                    Some(&quote @ (b'"' | b'\'')) => *self = Self::Value(char::from(quote)),
                    Some(b'>') => *self = Self::Text,
                    _ => {}
                }
                return first;
            }
            Self::Value(quote) => {
                if rest.starts_with(quote) {
                    *self = Self::Tag;
                }
                return first;
            }
            Self::Doctype { quote, subset } => {
                return self.doctype_step(rest, first, quote, subset);
            }
            Self::Comment => "-->",
            Self::CData => "]]>",
            Self::Pi => "?>",
        };
        if rest.starts_with(closer) {
            *self = Self::Text;
            closer.len()
        } else {
            first
        }
    }
}

/// XML declaration of a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlDeclaration {
    /// `version` pseudo-attribute.
    pub version: String,
    /// `encoding` pseudo-attribute as written.
    pub encoding: Option<String>,
    /// `standalone` pseudo-attribute.
    pub standalone: Option<bool>,
    /// Range of the declaration in the decoded text.
    pub span: Range<usize>,
}

/// Encoding of an XML document.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::encoding::{XmlContext, XmlEncoding};
///
/// let bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xE9</a>";
/// let encoding = XmlEncoding::detect(bytes)?;
/// assert_eq!(encoding.label(), "iso-8859-1");
/// let text = encoding.decode(bytes, None)?;
/// assert!(text.ends_with("<a>\u{E9}</a>"));
/// assert_eq!(encoding.encode(&text, None)?, bytes);
/// let escaped = encoding.escape_unrepresentable("\u{20AC}5", XmlContext::Text, None)?;
/// assert_eq!(escaped, "&#x20AC;5");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlEncoding {
    charset: Charset,
    bom: bool,
    declaration: Option<XmlDeclaration>,
}

/// Decodes and parses an XML document and installs the entities of its
/// document type declaration.
///
/// # Errors
///
/// Returns the errors of [`XmlEncoding::detect`], [`XmlEncoding::decode`],
/// [`XmlTree::parse`] and [`entity::install`].
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::encoding::parse_document;
///
/// let bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>caf\xE9</a>";
/// let (encoding, tree) = parse_document(bytes, None)?;
/// assert_eq!(encoding.label(), "iso-8859-1");
/// let text = tree.children(tree.root())[0];
/// assert_eq!(tree.chunk_value(text)?.as_deref(), Some("caf\u{E9}"));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn parse_document(
    bytes: &[u8],
    codec: Option<&dyn TextCodecService>,
) -> Result<(XmlEncoding, XmlTree)> {
    let encoding = XmlEncoding::detect(bytes)?;
    let text = encoding.decode(bytes, codec)?;
    let mut tree = XmlTree::parse(&text)?;
    entity::install(&mut tree)?;
    Ok((encoding, tree))
}

impl XmlEncoding {
    /// Detects the encoding of a document.
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` for a malformed XML declaration, an encoding
    /// that the byte order mark contradicts, UCS-4 and EBCDIC documents, and
    /// an encoding without a supported label.
    pub fn detect(bytes: &[u8]) -> Result<Self> {
        let (family, bom) = sniff(bytes)?;
        let head = declaration_head(&bytes[bom..], family);
        let declaration = parse_declaration(&head)?;
        let declared = declaration
            .as_ref()
            .and_then(|declaration| declaration.encoding.as_deref());
        let charset = resolve_charset(family, bom > 0, declared)?;
        Ok(Self {
            charset,
            bom: bom > 0,
            declaration,
        })
    }

    /// Label of the encoding: the WHATWG label in lowercase, `iso-8859-1`
    /// for ISO-8859-1 and `us-ascii` for US-ASCII.
    #[must_use]
    pub fn label(&self) -> &str {
        match &self.charset {
            Charset::Utf8 => "utf-8",
            Charset::Utf16Le => "utf-16le",
            Charset::Utf16Be => "utf-16be",
            Charset::Latin1 => "iso-8859-1",
            Charset::Ascii => "us-ascii",
            Charset::Codec(label) => label,
        }
    }

    /// Whether the document starts with a byte order mark.
    #[must_use]
    pub fn has_bom(&self) -> bool {
        self.bom
    }

    /// XML declaration of the document.
    #[must_use]
    pub fn declaration(&self) -> Option<&XmlDeclaration> {
        self.declaration.as_ref()
    }

    /// Whether every Unicode character can be written in this encoding.
    #[must_use]
    pub fn is_unicode(&self) -> bool {
        matches!(
            self.charset,
            Charset::Utf8 | Charset::Utf16Le | Charset::Utf16Be
        )
    }

    /// Decodes a document to text without the byte order mark.
    ///
    /// The text codec service is required for UTF-16 and for the encodings
    /// with a WHATWG label; the decoded text must encode back to the same
    /// bytes.
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` for bytes that are not valid in the encoding and
    /// when the text codec service is required but absent, and `E_ENCODING`
    /// when the content does not encode back to the same bytes.
    pub fn decode(&self, bytes: &[u8], codec: Option<&dyn TextCodecService>) -> Result<String> {
        let bom = self.bom_bytes().len().min(bytes.len());
        let body = &bytes[bom..];
        match &self.charset {
            Charset::Utf8 => decode_utf8(body, bom),
            Charset::Ascii => decode_ascii(body, bom),
            Charset::Latin1 => Ok(body.iter().copied().map(char::from).collect()),
            Charset::Utf16Le | Charset::Utf16Be | Charset::Codec(_) => {
                let codec = self.require_codec(codec)?;
                let encoding = self.codec_encoding();
                let text = codec.decode(body, &encoding).map_err(|cause| {
                    parse_error(format!("the content is not valid {}", self.label()), 0)
                        .with_cause(cause)
                })?;
                if codec.encode(&text, &encoding).ok().as_deref() != Some(body) {
                    return Err(Error::new(
                        ErrorCode::Encoding,
                        format!(
                            "the content does not convert back to the same bytes in {}; \
                             convert the file to UTF-8 before editing it",
                            self.label()
                        ),
                    ));
                }
                Ok(text)
            }
        }
    }

    /// Encodes text in this encoding, with the byte order mark when the
    /// original document had one.
    ///
    /// # Errors
    ///
    /// Returns `E_ENCODING` for a character that the encoding cannot
    /// represent and `E_DOC_PARSE` when the text codec service is required
    /// but absent.
    pub fn encode(&self, text: &str, codec: Option<&dyn TextCodecService>) -> Result<Vec<u8>> {
        let mut bytes = self.bom_bytes().to_vec();
        match &self.charset {
            Charset::Utf8 => bytes.extend_from_slice(text.as_bytes()),
            Charset::Ascii | Charset::Latin1 => {
                let max = if self.charset == Charset::Ascii {
                    ASCII_MAX
                } else {
                    LATIN1_MAX
                };
                for c in text.chars() {
                    let byte = u8::try_from(u32::from(c))
                        .ok()
                        .filter(|_| c <= max)
                        .ok_or_else(|| self.unrepresentable(c, None))?;
                    bytes.push(byte);
                }
            }
            Charset::Utf16Le | Charset::Utf16Be | Charset::Codec(_) => {
                let codec = self.require_codec(codec)?;
                bytes.extend(codec.encode(text, &self.codec_encoding())?);
            }
        }
        Ok(bytes)
    }

    /// Replaces the characters of new text that this encoding cannot
    /// represent with character references (`&#xNNNN;`).
    ///
    /// # Errors
    ///
    /// Returns `E_ENCODING` for such a character in a comment, a CDATA
    /// section, a processing instruction or a name, where references are not
    /// recognized, and `E_DOC_PARSE` when the text codec service is required
    /// but absent.
    pub fn escape_unrepresentable<'t>(
        &self,
        text: &'t str,
        context: XmlContext,
        codec: Option<&dyn TextCodecService>,
    ) -> Result<Cow<'t, str>> {
        if self.is_unicode() || text.is_ascii() {
            return Ok(Cow::Borrowed(text));
        }
        let mut escaper = Escaper::new(self, codec);
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            escaper.write(c, context, &mut out)?;
        }
        Ok(if escaper.changed {
            Cow::Owned(out)
        } else {
            Cow::Borrowed(text)
        })
    }

    /// Applies a change to the text of a tree and encodes the result.
    ///
    /// Characters of the replacements that the encoding cannot represent
    /// become character references in text and attribute values. Bytes
    /// outside the edited ranges are the bytes of the original document.
    ///
    /// # Errors
    ///
    /// Returns `E_UNSUPPORTED_CONSTRUCT` with construct
    /// `xml.dtd_internal_subset` for an edit of the document type
    /// declaration, `E_ENCODING` for an unrepresentable character where
    /// references are not recognized, and the errors of
    /// [`XmlChange::apply`] and [`XmlEncoding::encode`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::encoding::parse_document;
    /// use nurt_doc::formats::xml::{XmlChange, XmlEdit};
    ///
    /// let bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>x</a>";
    /// let (encoding, tree) = parse_document(bytes, None)?;
    /// let mut change = XmlChange::default();
    /// change.push(XmlEdit::new(46..47, "\u{E9}\u{20AC}"));
    /// let written = encoding.encode_change(&tree, &change, None)?;
    /// assert!(written.ends_with(b"<a>\xE9&#x20AC;</a>"));
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn encode_change(
        &self,
        tree: &XmlTree,
        change: &XmlChange,
        codec: Option<&dyn TextCodecService>,
    ) -> Result<Vec<u8>> {
        let text = tree.text();
        let doctype = tree.doctype().map(|doctype| doctype.span.clone());
        let mut edits: Vec<&XmlEdit> = change.edits.iter().collect();
        edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
        let mut escaper = Escaper::new(self, codec);
        let mut escaped = XmlChange::default();
        let mut state = Markup::Text;
        let mut position = 0;
        for edit in edits {
            if let Some(span) = &doctype {
                dtd::guard_doctype_edit(span, &edit.range)?;
            }
            position = advance(
                text,
                position,
                edit.range.start,
                &mut state,
                doctype.as_ref(),
            );
            let replacement = escaper.markup(&edit.replacement, state)?;
            escaped.push(XmlEdit::new(edit.range.clone(), replacement));
        }
        self.encode(&escaped.apply(text)?, codec)
    }

    fn represents(&self, c: char, codec: Option<&dyn TextCodecService>) -> Result<bool> {
        Ok(match &self.charset {
            Charset::Utf8 | Charset::Utf16Le | Charset::Utf16Be => true,
            Charset::Latin1 => c <= LATIN1_MAX,
            Charset::Ascii => c.is_ascii(),
            Charset::Codec(_) => {
                c.is_ascii()
                    || self
                        .require_codec(codec)?
                        .encode(c.encode_utf8(&mut [0; 4]), &self.codec_encoding())
                        .is_ok()
            }
        })
    }

    fn bom_bytes(&self) -> &'static [u8] {
        match (self.bom, &self.charset) {
            (false, _) => &[],
            (true, Charset::Utf16Le) => UTF16LE_BOM,
            (true, Charset::Utf16Be) => UTF16BE_BOM,
            (true, _) => UTF8_BOM,
        }
    }

    fn codec_encoding(&self) -> TextEncoding {
        TextEncoding {
            label: self.label().to_owned(),
            bom: false,
        }
    }

    fn require_codec<'c>(
        &self,
        codec: Option<&'c dyn TextCodecService>,
    ) -> Result<&'c dyn TextCodecService> {
        codec.ok_or_else(|| {
            parse_error(
                format!(
                    "decoding {} requires the text codec service, which is not available",
                    self.label()
                ),
                0,
            )
        })
    }

    fn unrepresentable(&self, c: char, context: Option<XmlContext>) -> Error {
        let place = context.map_or(String::new(), |context| {
            format!(" inside {}", context.describe())
        });
        Error::new(
            ErrorCode::Encoding,
            format!(
                "character U+{:04X} cannot be represented in {}{place}; use text without it \
                 or convert the file to UTF-8",
                u32::from(c),
                self.label()
            ),
        )
    }
}

/// Writer of new text that remembers which characters the encoding
/// represents.
struct Escaper<'e, 'c> {
    encoding: &'e XmlEncoding,
    codec: Option<&'c dyn TextCodecService>,
    known: HashMap<char, bool>,
    changed: bool,
}

impl<'e, 'c> Escaper<'e, 'c> {
    fn new(encoding: &'e XmlEncoding, codec: Option<&'c dyn TextCodecService>) -> Self {
        Self {
            encoding,
            codec,
            known: HashMap::new(),
            changed: false,
        }
    }

    fn write(&mut self, c: char, context: XmlContext, out: &mut String) -> Result<()> {
        let representable = match self.known.get(&c) {
            Some(&representable) => representable,
            None => {
                let representable = self.encoding.represents(c, self.codec)?;
                self.known.insert(c, representable);
                representable
            }
        };
        if representable {
            out.push(c);
        } else if context.accepts_references() {
            out.push_str(&format!("&#x{:04X};", u32::from(c)));
            self.changed = true;
        } else {
            return Err(self.encoding.unrepresentable(c, Some(context)));
        }
        Ok(())
    }

    /// Escapes a replacement that starts in markup state `state`.
    fn markup(&mut self, replacement: &str, mut state: Markup) -> Result<String> {
        if self.encoding.is_unicode() || replacement.is_ascii() {
            return Ok(replacement.to_owned());
        }
        let mut out = String::with_capacity(replacement.len());
        let mut rest = replacement;
        while !rest.is_empty() {
            let context = state.context();
            let len = state.step(rest);
            for c in rest[..len].chars() {
                self.write(c, context, &mut out)?;
            }
            rest = &rest[len..];
        }
        Ok(out)
    }
}

/// Scans `text` from `position` in state `state` to `target` and returns
/// the position reached; the document type declaration is skipped.
fn advance(
    text: &str,
    mut position: usize,
    target: usize,
    state: &mut Markup,
    doctype: Option<&Range<usize>>,
) -> usize {
    while position < target && position < text.len() {
        if let Some(span) = doctype.filter(|span| span.start == position) {
            position = span.end;
            continue;
        }
        position += state.step(&text[position..]);
    }
    position
}

fn sniff(bytes: &[u8]) -> Result<(Family, usize)> {
    const UCS4: [[u8; 4]; 6] = [
        [0x00, 0x00, 0xFE, 0xFF],
        [0xFF, 0xFE, 0x00, 0x00],
        [0x00, 0x00, 0x00, 0x3C],
        [0x3C, 0x00, 0x00, 0x00],
        [0x00, 0x00, 0x3C, 0x00],
        [0x00, 0x3C, 0x00, 0x00],
    ];
    const EBCDIC: [u8; 4] = [0x4C, 0x6F, 0xA7, 0x94];
    let head = bytes.get(..4);
    if head.is_some_and(|head| UCS4.iter().any(|ucs4| head == ucs4)) {
        return Err(parse_error(
            "UCS-4 documents are not supported; convert the file to UTF-8",
            0,
        ));
    }
    if head == Some(EBCDIC.as_slice()) {
        return Err(parse_error(
            "EBCDIC documents are not supported; convert the file to UTF-8",
            0,
        ));
    }
    Ok(if bytes.starts_with(UTF8_BOM) {
        (Family::Utf8, UTF8_BOM.len())
    } else if bytes.starts_with(UTF16LE_BOM) {
        (Family::Utf16Le, UTF16LE_BOM.len())
    } else if bytes.starts_with(UTF16BE_BOM) {
        (Family::Utf16Be, UTF16BE_BOM.len())
    } else if bytes.starts_with(&[0x3C, 0x00, 0x3F, 0x00]) {
        (Family::Utf16Le, 0)
    } else if bytes.starts_with(&[0x00, 0x3C, 0x00, 0x3F]) {
        (Family::Utf16Be, 0)
    } else {
        (Family::AsciiCompatible, 0)
    })
}

/// ASCII text of the start of the document up to the first `>`, read in the
/// detected family; a non-ASCII character ends it.
fn declaration_head(body: &[u8], family: Family) -> String {
    let units: Box<dyn Iterator<Item = u32> + '_> = match family {
        Family::Utf8 | Family::AsciiCompatible => Box::new(body.iter().map(|&b| u32::from(b))),
        Family::Utf16Le => Box::new(
            body.as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| u32::from(u16::from_le_bytes(pair))),
        ),
        Family::Utf16Be => Box::new(
            body.as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| u32::from(u16::from_be_bytes(pair))),
        ),
    };
    let mut head = String::new();
    for unit in units {
        let Some(c) = char::from_u32(unit).filter(char::is_ascii) else {
            break;
        };
        head.push(c);
        if c == '>' {
            break;
        }
    }
    head
}

fn parse_declaration(head: &str) -> Result<Option<XmlDeclaration>> {
    let Some(rest) = head.strip_prefix("<?xml") else {
        return Ok(None);
    };
    if !rest.starts_with(dtd::is_space) {
        return Ok(None);
    }
    let mut parser = DeclarationParser {
        text: head,
        pos: "<?xml".len(),
    };
    parser.declaration().map(Some)
}

struct DeclarationParser<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> DeclarationParser<'a> {
    fn error(&self, message: &str) -> Error {
        parse_error(format!("{message} in the XML declaration"), self.pos)
    }

    fn skip_space(&mut self) -> bool {
        let rest = &self.text[self.pos..];
        let len = rest.find(|c| !dtd::is_space(c)).unwrap_or(rest.len());
        self.pos += len;
        len > 0
    }

    fn pseudo_attribute(&mut self, name: &str) -> Result<Option<&'a str>> {
        let save = self.pos;
        if !self.skip_space() || !self.text[self.pos..].starts_with(name) {
            self.pos = save;
            return Ok(None);
        }
        self.pos += name.len();
        self.skip_space();
        if !self.text[self.pos..].starts_with('=') {
            return Err(self.error(&format!("expected `=` after `{name}`")));
        }
        self.pos += 1;
        self.skip_space();
        let quote = match self.text[self.pos..].chars().next() {
            Some(quote @ ('"' | '\'')) => quote,
            _ => return Err(self.error(&format!("expected a quoted value for `{name}`"))),
        };
        self.pos += 1;
        let Some(len) = self.text[self.pos..].find(quote) else {
            return Err(self.error(&format!("the value of `{name}` is not closed")));
        };
        let value = &self.text[self.pos..self.pos + len];
        self.pos += len + 1;
        Ok(Some(value))
    }

    fn declaration(&mut self) -> Result<XmlDeclaration> {
        let version = self
            .pseudo_attribute("version")?
            .ok_or_else(|| self.error("expected `version`"))?;
        if !is_version(version) {
            return Err(self.error(&format!("version `{version}` is not of the form `1.x`")));
        }
        let encoding = self.pseudo_attribute("encoding")?;
        if let Some(name) = encoding.filter(|name| !is_encoding_name(name)) {
            return Err(self.error(&format!("`{name}` is not an encoding name")));
        }
        let standalone = match self.pseudo_attribute("standalone")? {
            None => None,
            Some("yes") => Some(true),
            Some("no") => Some(false),
            Some(other) => {
                return Err(
                    self.error(&format!("standalone value `{other}` must be `yes` or `no`"))
                );
            }
        };
        self.skip_space();
        if !self.text[self.pos..].starts_with("?>") {
            return Err(self.error("expected `?>`"));
        }
        Ok(XmlDeclaration {
            version: version.to_owned(),
            encoding: encoding.map(str::to_owned),
            standalone,
            span: 0..self.pos + "?>".len(),
        })
    }
}

fn is_version(version: &str) -> bool {
    version
        .strip_prefix("1.")
        .is_some_and(|minor| !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()))
}

/// `EncName` production of XML 1.0.
fn is_encoding_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn charset_for_label(label: &str) -> Option<Charset> {
    let lower = label.to_ascii_lowercase();
    if LATIN1_LABELS.contains(&lower.as_str()) {
        return Some(Charset::Latin1);
    }
    if ASCII_LABELS.contains(&lower.as_str()) {
        return Some(Charset::Ascii);
    }
    let encoding = encoding_rs::Encoding::for_label(lower.as_bytes())?;
    if encoding == encoding_rs::UTF_8 {
        Some(Charset::Utf8)
    } else if encoding == encoding_rs::UTF_16LE {
        Some(Charset::Utf16Le)
    } else if encoding == encoding_rs::UTF_16BE {
        Some(Charset::Utf16Be)
    } else if encoding.is_ascii_compatible()
        && encoding != encoding_rs::REPLACEMENT
        && encoding != encoding_rs::X_USER_DEFINED
    {
        Some(Charset::Codec(encoding.name().to_ascii_lowercase()))
    } else {
        None
    }
}

fn resolve_charset(family: Family, bom: bool, declared: Option<&str>) -> Result<Charset> {
    let Some(label) = declared else {
        return Ok(match family {
            Family::Utf16Le => Charset::Utf16Le,
            Family::Utf16Be => Charset::Utf16Be,
            Family::Utf8 | Family::AsciiCompatible => Charset::Utf8,
        });
    };
    let charset = if label.eq_ignore_ascii_case("utf-16") {
        None
    } else {
        Some(charset_for_label(label).ok_or_else(|| {
            parse_error(
                format!(
                    "encoding `{label}` is not supported; convert the file to UTF-8, UTF-16 \
                     or an ASCII-compatible encoding such as ISO-8859-2 or windows-1250"
                ),
                0,
            )
        })?)
    };
    let resolved = match (family, charset) {
        (Family::Utf16Le, None | Some(Charset::Utf16Le)) => Some(Charset::Utf16Le),
        (Family::Utf16Be, None | Some(Charset::Utf16Be)) => Some(Charset::Utf16Be),
        (Family::Utf8, Some(Charset::Utf8)) => Some(Charset::Utf8),
        (Family::AsciiCompatible, Some(charset))
            if !matches!(charset, Charset::Utf16Le | Charset::Utf16Be) =>
        {
            Some(charset)
        }
        _ => None,
    };
    resolved.ok_or_else(|| contradiction(family, bom, label))
}

fn contradiction(family: Family, bom: bool, label: &str) -> Error {
    let detected = match family {
        Family::Utf8 if bom => "a UTF-8 byte order mark",
        Family::Utf16Le | Family::Utf16Be => "UTF-16 content",
        Family::Utf8 | Family::AsciiCompatible => "single-byte content",
    };
    parse_error(
        format!(
            "the declared encoding `{label}` contradicts {detected}; correct the `encoding` \
             declaration"
        ),
        0,
    )
}

fn decode_utf8(body: &[u8], offset: usize) -> Result<String> {
    match std::str::from_utf8(body) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) => Err(parse_error(
            "the content is not valid UTF-8; declare the actual encoding in the XML declaration",
            offset + error.valid_up_to(),
        )),
    }
}

fn decode_ascii(body: &[u8], offset: usize) -> Result<String> {
    match body.iter().position(|b| !b.is_ascii()) {
        Some(index) => Err(parse_error(
            "the content is not valid US-ASCII; declare the actual encoding in the XML \
             declaration",
            offset + index,
        )),
        None => Ok(body.iter().copied().map(char::from).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_state_follows_comments_cdata_and_attribute_values() {
        let text = "<a x='1>'><!-- > --><![CDATA[ < ]]>t<?p ?></a>";
        let mut state = Markup::Text;
        let mut contexts = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            contexts.push(state.context());
            let len = state.step(rest);
            rest = &rest[len..];
        }
        assert_eq!(state, Markup::Text);
        assert!(contexts.contains(&XmlContext::Attribute));
        assert!(contexts.contains(&XmlContext::Comment));
        assert!(contexts.contains(&XmlContext::CData));
        assert!(contexts.contains(&XmlContext::ProcessingInstruction));
    }

    #[test]
    fn document_type_declaration_ends_at_its_own_closing_bracket() {
        let declaration = "<!DOCTYPE a [<!ENTITY e \"x>y\"><!-- > \" --><?p > ?><!ENTITY f 'a>']]>";
        let text = format!("{declaration}<a/>");
        let mut state = Markup::Text;
        let mut rest = text.as_str();
        let mut consumed = 0;
        while consumed < declaration.len() {
            let len = state.step(rest);
            consumed += len;
            rest = &rest[len..];
        }
        assert_eq!(consumed, declaration.len());
        assert_eq!(state, Markup::Text);
        while !rest.is_empty() {
            let len = state.step(rest);
            rest = &rest[len..];
        }
        assert_eq!(state, Markup::Text);
    }

    #[test]
    fn declaration_without_an_internal_subset_ends_at_the_first_bracket() {
        let mut state = Markup::Text;
        let mut rest = "<!DOCTYPE a SYSTEM \"x>y.dtd\">t";
        while rest.len() > 1 {
            let len = state.step(rest);
            rest = &rest[len..];
        }
        assert_eq!(state, Markup::Text);
        assert_eq!(rest, "t");
    }

    #[test]
    fn declaration_head_reads_utf16_units() {
        let bytes: Vec<u8> = "<?xml version='1.0'?>"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        assert_eq!(
            declaration_head(&bytes, Family::Utf16Be),
            "<?xml version='1.0'?>"
        );
    }

    #[test]
    fn byte_order_mark_contradicting_the_declaration_is_rejected() {
        let bytes = b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a/>";
        let error = XmlEncoding::detect(bytes).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse);
    }

    #[test]
    fn ucs4_and_ebcdic_documents_are_rejected() {
        for bytes in [
            [0x00, 0x00, 0x00, 0x3C],
            [0x3C, 0x00, 0x00, 0x00],
            [0x4C, 0x6F, 0xA7, 0x94],
        ] {
            assert!(XmlEncoding::detect(&bytes).is_err());
        }
    }

    #[test]
    fn malformed_declarations_are_rejected() {
        for bytes in [
            &b"<?xml version=\"2.0\"?><a/>"[..],
            b"<?xml version=\"1.0\" standalone=\"maybe\"?><a/>",
            b"<?xml encoding=\"UTF-8\"?><a/>",
            b"<?xml version=\"1.0\" encoding=\"9x\"?><a/>",
            b"<?xml version=\"1.0\" encoding=\"no-such-encoding\"?><a/>",
        ] {
            assert!(XmlEncoding::detect(bytes).is_err());
        }
    }

    #[test]
    fn undeclared_encoding_defaults_to_utf8_and_reports_invalid_bytes() {
        let encoding = XmlEncoding::detect(b"<a/>").unwrap();
        assert_eq!(encoding.label(), "utf-8");
        assert!(!encoding.has_bom());
        let error = encoding.decode(b"<a>\xFF</a>", None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse);
    }

    #[test]
    fn utf8_byte_order_mark_is_kept_when_encoding_back() {
        let bytes = b"\xEF\xBB\xBF<a>\xC3\xA9</a>";
        let encoding = XmlEncoding::detect(bytes).unwrap();
        assert!(encoding.has_bom());
        let text = encoding.decode(bytes, None).unwrap();
        assert_eq!(text, "<a>\u{E9}</a>");
        assert_eq!(encoding.encode(&text, None).unwrap(), bytes);
    }

    #[test]
    fn latin1_document_round_trips_and_rejects_other_characters() {
        let bytes = b"<?xml version=\"1.0\" encoding=\"latin1\"?><a>\xE9</a>";
        let encoding = XmlEncoding::detect(bytes).unwrap();
        assert_eq!(encoding.label(), "iso-8859-1");
        let text = encoding.decode(bytes, None).unwrap();
        assert_eq!(encoding.encode(&text, None).unwrap(), bytes);
        let error = encoding.encode("<a>\u{20AC}</a>", None).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
    }

    #[test]
    fn ascii_document_with_a_high_byte_is_rejected() {
        let bytes = b"<?xml version=\"1.0\" encoding=\"us-ascii\"?><a>\xE9</a>";
        let encoding = XmlEncoding::detect(bytes).unwrap();
        assert_eq!(
            encoding.decode(bytes, None).unwrap_err().code(),
            ErrorCode::DocParse
        );
    }

    #[test]
    fn codec_encoding_without_the_codec_service_reports_a_parse_error() {
        let bytes = b"<?xml version=\"1.0\" encoding=\"windows-1250\"?><a/>";
        let encoding = XmlEncoding::detect(bytes).unwrap();
        assert_eq!(encoding.label(), "windows-1250");
        assert_eq!(
            encoding.decode(bytes, None).unwrap_err().code(),
            ErrorCode::DocParse
        );
    }

    #[test]
    fn unrepresentable_characters_become_references_only_where_references_exist() {
        let bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a/>";
        let encoding = XmlEncoding::detect(bytes).unwrap();
        let text = encoding
            .escape_unrepresentable("\u{20AC}5", XmlContext::Attribute, None)
            .unwrap();
        assert_eq!(text, "&#x20AC;5");
        for context in [
            XmlContext::Comment,
            XmlContext::CData,
            XmlContext::ProcessingInstruction,
            XmlContext::Name,
        ] {
            let error = encoding
                .escape_unrepresentable("\u{20AC}", context, None)
                .unwrap_err();
            assert_eq!(error.code(), ErrorCode::Encoding);
        }
    }

    #[test]
    fn encode_change_escapes_attribute_values_and_rejects_comments_in_latin1() {
        let bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a x='1'><!--c--></a>";
        let (encoding, tree) = parse_document(bytes, None).unwrap();
        let text = tree.text();
        let value_at = text.find("'1'").unwrap() + 1;
        let mut change = XmlChange::default();
        change.push(XmlEdit::new(value_at..value_at + 1, "\u{20AC}"));
        let written = encoding.encode_change(&tree, &change, None).unwrap();
        assert!(written.ends_with(b"<a x='&#x20AC;'><!--c--></a>"));
        let comment_at = text.find("c-->").unwrap();
        let mut in_comment = XmlChange::default();
        in_comment.push(XmlEdit::new(comment_at..comment_at + 1, "\u{20AC}"));
        let error = encoding
            .encode_change(&tree, &in_comment, None)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Encoding);
    }

    #[test]
    fn labels_map_to_charsets() {
        assert_eq!(charset_for_label("Latin1"), Some(Charset::Latin1));
        assert_eq!(charset_for_label("US-ASCII"), Some(Charset::Ascii));
        assert_eq!(
            charset_for_label("cp1250"),
            Some(Charset::Codec("windows-1250".to_owned()))
        );
        assert_eq!(charset_for_label("replacement"), None);
        assert_eq!(charset_for_label("x-user-defined"), None);
        assert_eq!(charset_for_label("no-such-encoding"), None);
    }
}
