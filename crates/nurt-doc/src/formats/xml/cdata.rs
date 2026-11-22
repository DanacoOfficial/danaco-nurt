// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Character data: decoding of references, escaping of new values and
//! replacement of text and CDATA nodes.
//!
//! Decoding applies end-of-line normalization (XML 1.0, section 2.11) to
//! text and attribute-value normalization for attributes of type CDATA
//! (section 3.3.3): literal tab, line feed and carriage return become a
//! space, characters written as references are kept.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use nurt_contracts::error::Result;

use super::tree::{Node, NodeId, XmlTree};
use super::{CONSTRUCT_ENTITY_REFERENCE, NOTABLE_ENTITY_PREFIX};
use super::{XmlChange, XmlEdit, doc_parse_error, invalid_value, locate, unsupported};

/// Source of replacement text for general entities declared in a document
/// type definition.
///
/// The five predefined entities and character references are decoded
/// without a resolver.
pub trait EntitySource: Send + Sync {
    /// Replacement text of the general entity `name`, with nested
    /// references already expanded.
    ///
    /// # Errors
    ///
    /// Returns the error of the definition: limits of expansion, external
    /// or unparsed entities, recursion.
    fn resolve(&self, name: &str) -> Result<Option<String>>;

    /// `true` when the attribute-list declaration of `element` gives
    /// `attribute` a tokenized type (`ID`, `IDREF`, `IDREFS`, `ENTITY`,
    /// `ENTITIES`, `NMTOKEN`, `NMTOKENS`, a notation or an enumeration);
    /// values of such attributes lose leading and trailing spaces and keep
    /// one space between tokens (XML 1.0, section 3.3.3).
    fn tokenized(&self, _element: &str, _attribute: &str) -> bool {
        false
    }
}

/// Optional entity source held by a tree.
#[derive(Clone, Default)]
pub(crate) struct ResolverSlot(pub(crate) Option<Arc<dyn EntitySource>>);

impl fmt::Debug for ResolverSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() {
            "Some(EntitySource)"
        } else {
            "None"
        })
    }
}

/// Where decoded character data occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Context {
    /// Element content.
    Text,
    /// Attribute value.
    Attribute,
}

/// Decoded character data and the names of entities it references.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Decoded {
    pub(crate) value: String,
    pub(crate) entities: Vec<String>,
}

/// Predefined entities of XML 1.0, section 4.6.
const PREDEFINED: [(&str, char); 5] = [
    ("lt", '<'),
    ("gt", '>'),
    ("amp", '&'),
    ("apos", '\''),
    ("quot", '"'),
];

/// Whitespace characters of the `S` production.
pub(crate) const XML_WHITESPACE: [char; 4] = [' ', '\t', '\n', '\r'];

/// Radix of hexadecimal character references.
const HEX_RADIX: u32 = 16;

/// Radix of decimal character references.
const DECIMAL_RADIX: u32 = 10;

/// `true` for characters of the XML 1.0 `Char` production.
pub(crate) fn is_xml_char(c: char) -> bool {
    matches!(c,
        '\t' | '\n' | '\r'
        | '\u{20}'..='\u{D7FF}'
        | '\u{E000}'..='\u{FFFD}'
        | '\u{10000}'..='\u{10FFFF}')
}

/// Range of `raw` without leading and trailing XML whitespace, relative to
/// `raw`; the whole range when `raw` holds whitespace only.
pub(crate) fn core_range(raw: &str) -> Range<usize> {
    let trimmed_start = raw.len() - raw.trim_start_matches(XML_WHITESPACE).len();
    let trimmed_end = raw.trim_end_matches(XML_WHITESPACE).len();
    if trimmed_start >= trimmed_end {
        0..raw.len()
    } else {
        trimmed_start..trimmed_end
    }
}

/// Text with leading and trailing XML whitespace removed.
pub(crate) fn trim_xml(text: &str) -> &str {
    text.trim_matches(XML_WHITESPACE)
}

/// Checks that every character of a new value can be written in XML 1.0.
pub(crate) fn check_representable(value: &str) -> Result<()> {
    match value.chars().find(|c| !is_xml_char(*c)) {
        Some(c) => Err(invalid_value(format!(
            "value contains U+{:04X}, which XML 1.0 cannot represent; remove the character",
            u32::from(c)
        ))),
        None => Ok(()),
    }
}

/// Escapes text for element content; line feeds take the line ending of
/// the document.
pub(crate) fn escape_text(value: &str, eol: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str(eol),
            _ => out.push(c),
        }
    }
    out
}

/// Escapes text for an attribute value delimited by `quote`.
///
/// Tab, line feed and carriage return are written as references because
/// attribute-value normalization would turn literal ones into spaces.
pub(crate) fn escape_attribute(value: &str, quote: char) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' if quote == '"' => out.push_str("&quot;"),
            '\'' if quote == '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            _ => out.push(c),
        }
    }
    out
}

/// Content of a CDATA section holding `value`, without the delimiters.
///
/// `]]>` and carriage returns cannot occur inside one section; the content
/// is split into several sections around them.
pub(crate) fn cdata_content(value: &str, eol: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("]]>") {
            out.push_str("]]]]><![CDATA[>");
            rest = &rest["]]>".len()..];
            continue;
        }
        match c {
            '\r' => out.push_str("]]>&#13;<![CDATA["),
            '\n' => out.push_str(eol),
            _ => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

impl XmlTree {
    /// Decodes the raw character data at `range` of the source.
    pub(crate) fn decode(&self, range: Range<usize>, context: Context) -> Result<Decoded> {
        let raw = &self.text()[range.clone()];
        let mut decoded = Decoded {
            value: String::with_capacity(raw.len()),
            entities: Vec::new(),
        };
        let mut index = 0;
        while let Some(c) = raw[index..].chars().next() {
            match c {
                '&' => index = self.decode_reference(range.start, index, context, &mut decoded)?,
                '\r' => {
                    decoded.value.push(normalized_space(context));
                    index += 1;
                    if raw[index..].starts_with('\n') {
                        index += 1;
                    }
                }
                '\n' | '\t' if context == Context::Attribute => {
                    decoded.value.push(' ');
                    index += 1;
                }
                _ => {
                    decoded.value.push(c);
                    index += c.len_utf8();
                }
            }
        }
        Ok(decoded)
    }

    /// Decodes the reference starting at `index` of the raw data at `base`
    /// and returns the index after it.
    fn decode_reference(
        &self,
        base: usize,
        index: usize,
        context: Context,
        decoded: &mut Decoded,
    ) -> Result<usize> {
        let source = &self.text()[base + index..];
        let Some(length) = source.find(';') else {
            return Err(doc_parse_error(
                self.text(),
                base + index,
                "reference is not terminated; write `&amp;` for a literal ampersand",
            ));
        };
        let name = &source[1..length];
        let at = base + index;
        if let Some(number) = name.strip_prefix('#') {
            decoded.value.push(self.char_reference(number, at)?);
        } else if let Some((_, c)) = PREDEFINED.iter().find(|(entity, _)| *entity == name) {
            decoded.value.push(*c);
        } else {
            let replacement = self.entity_replacement(name, at, context)?;
            decoded.value.push_str(&replacement);
            decoded.entities.push(name.to_owned());
        }
        Ok(index + length + 1)
    }

    /// Character of a character reference body (`#65` without `&#`).
    fn char_reference(&self, number: &str, at: usize) -> Result<char> {
        let parsed = match number.strip_prefix('x') {
            Some(hex) => parse_digits(hex, HEX_RADIX),
            None => parse_digits(number, DECIMAL_RADIX),
        };
        parsed
            .and_then(char::from_u32)
            .filter(|c| is_xml_char(*c))
            .ok_or_else(|| {
                doc_parse_error(
                    self.text(),
                    at,
                    format!(
                        "`&#{number};` does not refer to an XML character; correct the reference"
                    ),
                )
            })
    }

    /// Replacement text of a general entity.
    fn entity_replacement(&self, name: &str, at: usize, context: Context) -> Result<String> {
        if !super::ns::is_valid_name(name) {
            return Err(doc_parse_error(
                self.text(),
                at,
                format!(
                    "`&{name};` is not a valid reference; write `&amp;` for a literal ampersand"
                ),
            ));
        }
        let Some(resolver) = self.resolver() else {
            return Err(self.unresolved_entity(name, at));
        };
        let Some(text) = resolver
            .resolve(name)
            .map_err(|error| locate(error, self.text(), at))?
        else {
            return Err(doc_parse_error(
                self.text(),
                at,
                format!(
                    "entity `{name}` is not declared; declare it in the document type definition or escape the ampersand"
                ),
            ));
        };
        // The entity source checks literal `<` in replacement text used in
        // attributes; the text returned here is already decoded, so a `<`
        // in it comes from a reference such as `&lt;` and is allowed.
        Ok(match context {
            Context::Text => text,
            Context::Attribute => text.replace(['\t', '\n', '\r'], " "),
        })
    }

    /// Error for an entity reference that cannot be resolved without an
    /// entity source.
    fn unresolved_entity(&self, name: &str, at: usize) -> nurt_contracts::error::Error {
        if self.doctype().is_some() {
            unsupported(
                CONSTRUCT_ENTITY_REFERENCE,
                format!(
                    "entity `{name}` is defined by the document type definition, which is not loaded"
                ),
            )
        } else {
            doc_parse_error(
                self.text(),
                at,
                format!("entity `{name}` is not declared; write `&amp;{name};` for literal text"),
            )
        }
    }

    /// Decodes every text node and attribute value, reporting the first
    /// malformed or unresolvable reference.
    ///
    /// [`XmlTree::parse`] performs this check for documents without a
    /// document type declaration; documents with one are checked after an
    /// [`EntitySource`] is installed.
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` for a malformed or undeclared reference and
    /// the errors of the entity source.
    pub fn check_references(&self) -> Result<()> {
        for id in self.node_ids() {
            match self.node(id) {
                Node::Text(text) if self.text()[text.span.clone()].contains('&') => {
                    self.decode(text.span.clone(), Context::Text)?;
                }
                Node::Element(element) => {
                    for attribute in &element.attributes {
                        self.decode(attribute.value.clone(), Context::Attribute)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Decoded value of the text chunk that starts at a text or CDATA node,
    /// with surrounding whitespace removed; `None` for other nodes.
    ///
    /// The chunk is the run of adjacent text and CDATA nodes starting at
    /// `node` (see [`XmlTree::text_chunks`]).
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` for a malformed reference and the errors of
    /// the entity source.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a>\n  x &amp; y\n</a>")?;
    /// let text = tree.children(tree.root())[0];
    /// assert_eq!(tree.chunk_value(text)?.as_deref(), Some("x & y"));
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn chunk_value(&self, node: NodeId) -> Result<Option<String>> {
        Ok(self
            .chunk_decoded(node)?
            .map(|decoded| trim_xml(&decoded.value).to_owned()))
    }

    /// Decoded text chunk that starts at `node`, without trimming.
    pub(crate) fn chunk_decoded(&self, node: NodeId) -> Result<Option<Decoded>> {
        let run = self.run(node);
        if run.is_empty() {
            return Ok(None);
        }
        let mut decoded = Decoded::default();
        for id in run {
            match self.node(id) {
                Node::Text(text) => {
                    let part = self.decode(text.span.clone(), Context::Text)?;
                    decoded.value.push_str(&part.value);
                    decoded.entities.extend(part.entities);
                }
                Node::CData(cdata) => {
                    decoded
                        .value
                        .push_str(&normalize_eol(&self.text()[cdata.content.clone()]));
                }
                _ => {}
            }
        }
        Ok(Some(decoded))
    }

    /// CDATA section of a run whose other nodes hold only whitespace.
    fn sole_cdata(&self, run: &[NodeId]) -> Option<NodeId> {
        let mut sections = run
            .iter()
            .filter(|id| matches!(self.node(**id), Node::CData(_)));
        let section = sections.next().copied();
        let blank_text = run.iter().all(|id| match self.node(*id) {
            Node::Text(text) => trim_xml(&self.text()[text.span.clone()]).is_empty(),
            _ => true,
        });
        section.filter(|_| sections.next().is_none() && blank_text)
    }

    /// Range and markup that replace the value of the text chunk `run`.
    ///
    /// A CDATA section surrounded only by whitespace keeps its delimiters
    /// and the whitespace inside them; another run with CDATA is written as
    /// one CDATA section, other runs as escaped text. Whitespace that
    /// surrounds the value stays in place.
    fn run_replacement(&self, run: &[NodeId], value: &str) -> (Range<usize>, String) {
        let eol = self.style().eol;
        if let Some(single) = self.sole_cdata(run)
            && let Node::CData(cdata) = self.node(single)
        {
            let core = core_range(&self.text()[cdata.content.clone()]);
            let start = cdata.content.start;
            return (
                start + core.start..start + core.end,
                cdata_content(value, eol),
            );
        }
        let (Some(first), Some(last)) = (run.first(), run.last()) else {
            return (0..0, String::new());
        };
        let (first_span, last_span) = (self.node(*first).span(), self.node(*last).span());
        let mut start = first_span.start;
        let mut end = last_span.end;
        if matches!(self.node(*first), Node::Text(_)) {
            let raw = &self.text()[first_span.clone()];
            start += raw.len() - raw.trim_start_matches(XML_WHITESPACE).len();
        }
        if matches!(self.node(*last), Node::Text(_)) {
            let raw = &self.text()[last_span.clone()];
            end -= raw.len() - raw.trim_end_matches(XML_WHITESPACE).len();
        }
        if start > end {
            start = first_span.start;
            end = last_span.end;
        }
        let with_cdata = run
            .iter()
            .any(|id| matches!(self.node(*id), Node::CData(_)));
        let replacement = if with_cdata {
            format!("<![CDATA[{}]]>", cdata_content(value, eol))
        } else {
            escape_text(value, eol)
        };
        (start..end, replacement)
    }

    /// Edit that replaces the value of the text chunk starting at a text or
    /// CDATA node, keeping the whitespace around the value and CDATA
    /// sections as CDATA.
    ///
    /// The change is empty when the node already holds `value`. Text that
    /// references declared entities is replaced as a whole; the entity
    /// names are listed in [`XmlChange::notable`].
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` when `value` holds characters that XML 1.0
    /// cannot represent, `E_KEY_NOT_FOUND` when `node` is neither text nor
    /// CDATA, and decoding errors of the current value.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a><![CDATA[x]]></a>")?;
    /// let cdata = tree.children(tree.root())[0];
    /// let change = tree.replace_text(cdata, "1 < 2")?;
    /// assert_eq!(change.apply(tree.text())?, "<a><![CDATA[1 < 2]]></a>");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn replace_text(&self, node: NodeId, value: &str) -> Result<XmlChange> {
        check_representable(value)?;
        let mut change = XmlChange::default();
        let Some(decoded) = self.chunk_decoded(node)? else {
            return Err(super::key_not_found(
                "the node holds no text; address an element, attribute or `#text`",
            ));
        };
        if trim_xml(&decoded.value) == value {
            return Ok(change);
        }
        let (range, replacement) = self.run_replacement(&self.run(node), value);
        change.push(XmlEdit::new(range, replacement));
        for entity in decoded.entities {
            change.note(format!("{NOTABLE_ENTITY_PREFIX}{entity}"));
        }
        Ok(change)
    }
}

/// Parses the digits of a character reference.
fn parse_digits(digits: &str, radix: u32) -> Option<u32> {
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    u32::from_str_radix(digits, radix).ok()
}

/// Character that replaces a line break in the given context.
const fn normalized_space(context: Context) -> char {
    match context {
        Context::Text => '\n',
        Context::Attribute => ' ',
    }
}

/// Applies end-of-line normalization.
pub(crate) fn normalize_eol(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_escaping_covers_markup_characters() {
        assert_eq!(escape_text("a<b>&c", "\n"), "a&lt;b&gt;&amp;c");
        assert_eq!(escape_text("x]]>y", "\n"), "x]]&gt;y");
        assert_eq!(escape_text("a\nb\r", "\r\n"), "a\r\nb&#13;");
    }

    #[test]
    fn attribute_escaping_depends_on_quote() {
        assert_eq!(escape_attribute(r#"a"b'c"#, '"'), "a&quot;b'c");
        assert_eq!(escape_attribute(r#"a"b'c"#, '\''), "a\"b&apos;c");
        assert_eq!(escape_attribute("a\tb\nc<&", '"'), "a&#9;b&#10;c&lt;&amp;");
    }

    #[test]
    fn cdata_content_splits_terminator_and_carriage_return() {
        assert_eq!(cdata_content("a]]>b", "\n"), "a]]]]><![CDATA[>b");
        assert_eq!(cdata_content("a\rb", "\n"), "a]]>&#13;<![CDATA[b");
        assert_eq!(cdata_content("a\nb", "\r\n"), "a\r\nb");
    }

    #[test]
    fn core_range_skips_surrounding_whitespace() {
        assert_eq!(core_range("\n  1.0\n"), 3..6);
        assert_eq!(core_range("   "), 0..3);
        assert_eq!(core_range("x"), 0..1);
    }

    #[test]
    fn unrepresentable_characters_are_rejected() {
        assert!(check_representable("a\u{1}b").is_err());
        assert!(check_representable("a\u{FFFE}").is_err());
        assert!(check_representable("tab\tok").is_ok());
    }

    #[test]
    fn character_reference_digits_are_validated() {
        assert_eq!(parse_digits("41", HEX_RADIX), Some(0x41));
        assert_eq!(parse_digits("", DECIMAL_RADIX), None);
        assert_eq!(parse_digits("1a", DECIMAL_RADIX), None);
    }
}
