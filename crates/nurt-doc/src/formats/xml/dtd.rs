// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Document type declaration and its internal subset.
//!
//! [`Dtd::parse`] checks a `<!DOCTYPE>` declaration against the grammar and
//! the well-formedness constraints of XML 1.0 (Fifth Edition) and keeps the
//! declarations that a non-validating processor uses: general entities and
//! attribute-list declarations. The external subset and parameter entities
//! are never read. Entity and attribute-list declarations that follow a
//! reference to a parameter entity are not processed unless the document is
//! standalone, because the unread entity could have declared the same names
//! first (XML 1.0, section 5.1).

use std::collections::HashMap;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Location, Result};

use super::entity::{self, DtdEntities, Reference};

/// Construct reported when an edit falls inside the document type
/// declaration.
pub const DTD_INTERNAL_SUBSET: &str = "xml.dtd_internal_subset";

/// External identifier of an entity, a notation or the external subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalId {
    /// Public identifier (`PUBLIC "…"`).
    pub public_id: Option<String>,
    /// System identifier; absent only for a notation declared with a public
    /// identifier alone.
    pub system_id: Option<String>,
}

/// Definition of a general entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityDefinition {
    /// Internal entity with its replacement text: character references
    /// replaced, general entity references kept, line ends normalized.
    Internal(String),
    /// External parsed entity; never read.
    External(ExternalId),
    /// External unparsed entity with the name of its notation.
    Unparsed {
        /// External identifier.
        id: ExternalId,
        /// Notation name from `NDATA`.
        notation: String,
    },
}

/// Declaration of a general entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityDecl {
    /// Entity name.
    pub name: String,
    /// Entity definition.
    pub definition: EntityDefinition,
    /// Byte range of the declaration in the document.
    pub span: Range<usize>,
}

/// Declared type of an attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeType {
    /// `CDATA`.
    CData,
    /// `ID`.
    Id,
    /// `IDREF`.
    IdRef,
    /// `IDREFS`.
    IdRefs,
    /// `ENTITY`.
    Entity,
    /// `ENTITIES`.
    Entities,
    /// `NMTOKEN`.
    NmToken,
    /// `NMTOKENS`.
    NmTokens,
    /// `NOTATION (…)` with the notation names.
    Notation(Vec<String>),
    /// Enumeration `(…)` with the name tokens.
    Enumeration(Vec<String>),
}

impl AttributeType {
    /// Whether attribute values of this type are tokenized: leading and
    /// trailing spaces are removed and runs of spaces collapse to one
    /// (XML 1.0, section 3.3.3).
    #[must_use]
    pub fn is_tokenized(&self) -> bool {
        !matches!(self, Self::CData)
    }
}

/// Default declaration of an attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeDefault {
    /// `#REQUIRED`.
    Required,
    /// `#IMPLIED`.
    Implied,
    /// Default value, optionally `#FIXED`.
    Value {
        /// Literal value as written, without quotes.
        literal: String,
        /// Byte offset of the literal value in the document.
        offset: usize,
        /// The declaration uses `#FIXED`.
        fixed: bool,
    },
}

/// Declaration of one attribute in an attribute-list declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeDecl {
    /// Element name.
    pub element: String,
    /// Attribute name.
    pub name: String,
    /// Declared type.
    pub kind: AttributeType,
    /// Default declaration.
    pub default: AttributeDefault,
}

/// Parsed document type declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dtd {
    root: String,
    external_id: Option<ExternalId>,
    span: Range<usize>,
    internal_subset: Option<Range<usize>>,
    entities: Vec<EntityDecl>,
    entity_index: HashMap<String, usize>,
    attributes: Vec<AttributeDecl>,
    attribute_index: HashMap<String, HashMap<String, usize>>,
    unread_declarations: bool,
    standalone: bool,
}

impl Dtd {
    /// Parses a document type declaration.
    ///
    /// The declaration text runs from `<!DOCTYPE` to the closing `>`;
    /// `offset` is its byte offset in the document and `standalone` the
    /// value of the `standalone` pseudo-attribute of the XML declaration.
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` with the byte offset in `location` when the
    /// declaration is not well-formed, including attribute defaults that
    /// refer to undeclared, external or unparsed entities. Returns `E_LIMIT`
    /// when an attribute default exceeds the entity expansion limits.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::dtd::{Dtd, EntityDefinition};
    ///
    /// let text = r#"<!DOCTYPE note [<!ENTITY org "Example &#38; Co">]>"#;
    /// let dtd = Dtd::parse(text, 0, false).unwrap();
    /// assert_eq!(dtd.root(), "note");
    /// let org = dtd.entity("org").unwrap();
    /// assert_eq!(org.definition, EntityDefinition::Internal("Example & Co".to_owned()));
    /// ```
    pub fn parse(declaration: &str, offset: usize, standalone: bool) -> Result<Self> {
        let mut parser = Parser::new(declaration, offset, standalone);
        parser.doctype()?;
        let dtd = parser.dtd;
        dtd.check_attribute_defaults()?;
        Ok(dtd)
    }

    /// Name of the root element.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// External identifier of the external subset.
    #[must_use]
    pub fn external_id(&self) -> Option<&ExternalId> {
        self.external_id.as_ref()
    }

    /// Byte range of the whole declaration in the document.
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    /// Byte range of the internal subset, between `[` and `]`.
    #[must_use]
    pub fn internal_subset(&self) -> Option<Range<usize>> {
        self.internal_subset.clone()
    }

    /// Processed general entity declarations in document order; the first
    /// declaration of a name is binding.
    #[must_use]
    pub fn entities(&self) -> &[EntityDecl] {
        &self.entities
    }

    /// Binding declaration of a general entity.
    #[must_use]
    pub fn entity(&self, name: &str) -> Option<&EntityDecl> {
        self.entity_index
            .get(name)
            .and_then(|index| self.entities.get(*index))
    }

    /// Binding declaration of an attribute of an element.
    #[must_use]
    pub fn attribute(&self, element: &str, name: &str) -> Option<&AttributeDecl> {
        self.attribute_index
            .get(element)
            .and_then(|names| names.get(name))
            .and_then(|index| self.attributes.get(*index))
    }

    fn add_entity(&mut self, decl: EntityDecl) {
        if !self.entity_index.contains_key(&decl.name) {
            self.entity_index
                .insert(decl.name.clone(), self.entities.len());
            self.entities.push(decl);
        }
    }

    fn add_attribute(&mut self, decl: AttributeDecl) {
        let names = self
            .attribute_index
            .entry(decl.element.clone())
            .or_default();
        if !names.contains_key(&decl.name) {
            names.insert(decl.name.clone(), self.attributes.len());
            self.attributes.push(decl);
        }
    }

    /// Binding attribute declarations of an element in document order.
    pub fn attributes_of<'a>(
        &'a self,
        element: &'a str,
    ) -> impl Iterator<Item = &'a AttributeDecl> + 'a {
        self.attributes
            .iter()
            .filter(move |decl| decl.element == element)
    }

    /// Whether declarations may exist that are never read: an external
    /// subset, or a parameter entity reference in the internal subset.
    #[must_use]
    pub fn has_unread_declarations(&self) -> bool {
        self.unread_declarations
    }

    /// Whether the XML declaration of the document has `standalone="yes"`.
    #[must_use]
    pub fn is_standalone(&self) -> bool {
        self.standalone
    }

    /// Rejects an edit that touches the document type declaration.
    ///
    /// An insertion (empty range) at either end of the declaration is
    /// outside it.
    ///
    /// # Errors
    ///
    /// Returns `E_UNSUPPORTED_CONSTRUCT` with construct `xml.dtd_internal_subset`
    /// when `edit` overlaps the declaration.
    pub fn guard_edit(&self, edit: &Range<usize>) -> Result<()> {
        guard_doctype_edit(&self.span, edit)
    }

    /// Checks the well-formedness constraints of attribute defaults. A
    /// default that depends on unread declarations is reported only when it
    /// is used.
    fn check_attribute_defaults(&self) -> Result<()> {
        let resolver = DtdEntities::new(Some(self.clone()));
        for decl in &self.attributes {
            if let AttributeDefault::Value {
                literal, offset, ..
            } = &decl.default
            {
                match resolver.decode_attribute(literal, *offset, decl.kind.is_tokenized()) {
                    Err(error) if error.code() != ErrorCode::UnsupportedConstruct => {
                        return Err(error);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// Rejects an edit that touches the document type declaration at `span`;
/// an insertion at either end of the declaration is outside it.
pub(super) fn guard_doctype_edit(span: &Range<usize>, edit: &Range<usize>) -> Result<()> {
    let touches = if edit.is_empty() {
        span.start < edit.start && edit.start < span.end
    } else {
        edit.start < span.end && span.start < edit.end
    };
    if touches {
        return Err(Error::new(
            ErrorCode::UnsupportedConstruct,
            "the document type declaration and its internal subset cannot be edited; \
             change the file with a text edit",
        )
        .with_found(DTD_INTERNAL_SUBSET)
        .with_location(byte_location(edit.start)));
    }
    Ok(())
}

/// `E_DOC_PARSE` error at a byte offset of the document.
pub(super) fn parse_error(message: impl Into<String>, byte: usize) -> Error {
    Error::new(ErrorCode::DocParse, message).with_location(byte_location(byte))
}

/// Location of a byte offset; the path is set by the caller that knows it.
pub(super) fn byte_location(byte: usize) -> Location {
    Location {
        path: String::new(),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        byte: u64::try_from(byte).ok(),
    }
}

/// `Char` production of XML 1.0.
pub(super) fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | ' '..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// `S` production of XML 1.0.
pub(super) fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `NameStartChar` production of XML 1.0.
pub(super) fn is_name_start_char(c: char) -> bool {
    matches!(c,
        ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// `NameChar` production of XML 1.0.
pub(super) fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Length in bytes of the `Name` at the start of `text`, or 0.
pub(super) fn name_len(text: &str) -> usize {
    let mut chars = text.char_indices();
    match chars.next() {
        Some((_, c)) if is_name_start_char(c) => {}
        _ => return 0,
    }
    chars
        .find(|&(_, c)| !is_name_char(c))
        .map_or(text.len(), |(index, _)| index)
}

/// `PubidChar` production of XML 1.0.
fn is_pubid_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || " \r\n-'()+,./:=?;!*#@$_%".contains(c)
}

/// Deepest nesting of groups in a content model; the group grammar is parsed
/// recursively, and this bound keeps untrusted input within the thread stack.
const MAX_GROUP_DEPTH: usize = 64;

const ATTRIBUTE_TYPES: [(&str, AttributeType); 8] = [
    ("CDATA", AttributeType::CData),
    ("IDREFS", AttributeType::IdRefs),
    ("IDREF", AttributeType::IdRef),
    ("ID", AttributeType::Id),
    ("ENTITIES", AttributeType::Entities),
    ("ENTITY", AttributeType::Entity),
    ("NMTOKENS", AttributeType::NmTokens),
    ("NMTOKEN", AttributeType::NmToken),
];

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    base: usize,
    processing: bool,
    depth: usize,
    dtd: Dtd,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, base: usize, standalone: bool) -> Self {
        Self {
            text,
            pos: 0,
            base,
            processing: true,
            depth: 0,
            dtd: Dtd {
                root: String::new(),
                external_id: None,
                span: base..base + text.len(),
                internal_subset: None,
                entities: Vec::new(),
                entity_index: HashMap::new(),
                attributes: Vec::new(),
                attribute_index: HashMap::new(),
                unread_declarations: false,
                standalone,
            },
        }
    }

    fn error(&self, message: impl Into<String>) -> Error {
        parse_error(message, self.base + self.pos)
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn eat(&mut self, token: &str) -> bool {
        let found = self.rest().starts_with(token);
        if found {
            self.pos += token.len();
        }
        found
    }

    fn expect(&mut self, token: &str, context: &str) -> Result<()> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.error(format!("expected `{token}` {context}")))
        }
    }

    fn skip_space(&mut self) -> bool {
        let start = self.pos;
        let rest = self.rest();
        let len = rest.find(|c| !is_space(c)).unwrap_or(rest.len());
        self.pos += len;
        self.pos > start
    }

    fn require_space(&mut self, context: &str) -> Result<()> {
        if self.skip_space() {
            Ok(())
        } else {
            Err(self.error(format!("expected white space {context}")))
        }
    }

    fn name(&mut self, context: &str) -> Result<&'a str> {
        let len = name_len(self.rest());
        if len == 0 {
            return Err(self.error(format!("expected a name {context}")));
        }
        let name = &self.rest()[..len];
        self.pos += len;
        Ok(name)
    }

    fn nmtoken(&mut self) -> Result<&'a str> {
        let rest = self.rest();
        let len = rest.find(|c| !is_name_char(c)).unwrap_or(rest.len());
        if len == 0 {
            return Err(self.error("expected a name token in the enumeration"));
        }
        self.pos += len;
        Ok(&rest[..len])
    }

    fn check_chars(&self, text: &str, at: usize) -> Result<()> {
        match text.char_indices().find(|&(_, c)| !is_xml_char(c)) {
            Some((index, c)) => Err(parse_error(
                format!("character U+{:04X} is not allowed in XML", u32::from(c)),
                self.base + at + index,
            )),
            None => Ok(()),
        }
    }

    fn quote(&mut self, context: &str) -> Result<char> {
        match self.peek() {
            Some(quote @ ('"' | '\'')) => {
                self.pos += 1;
                Ok(quote)
            }
            _ => Err(self.error(format!("expected a quoted literal {context}"))),
        }
    }

    fn literal(&mut self, context: &str) -> Result<(&'a str, usize)> {
        let quote = self.quote(context)?;
        let start = self.pos;
        let Some(len) = self.rest().find(quote) else {
            return Err(self.error(format!("literal {context} is not closed; add `{quote}`")));
        };
        let value = &self.rest()[..len];
        self.check_chars(value, start)?;
        self.pos += len + 1;
        Ok((value, start))
    }

    fn doctype(&mut self) -> Result<()> {
        self.expect("<!DOCTYPE", "at the start of the document type declaration")?;
        self.require_space("after `<!DOCTYPE`")?;
        self.dtd.root = self.name("for the root element")?.to_owned();
        if self.skip_space() && matches!(self.peek(), Some('S' | 'P')) {
            self.dtd.external_id = Some(self.external_id(false)?);
            self.dtd.unread_declarations = true;
            self.skip_space();
        }
        if self.eat("[") {
            let start = self.pos;
            self.internal_subset()?;
            self.dtd.internal_subset = Some(self.base + start..self.base + self.pos);
            self.pos += 1;
            self.skip_space();
        }
        self.expect(">", "at the end of the document type declaration")?;
        if self.pos != self.text.len() {
            return Err(self.error("unexpected text after the document type declaration"));
        }
        Ok(())
    }

    fn external_id(&mut self, public_alone: bool) -> Result<ExternalId> {
        if self.eat("SYSTEM") {
            self.require_space("after `SYSTEM`")?;
            let (system, _) = self.literal("of the system identifier")?;
            return Ok(ExternalId {
                public_id: None,
                system_id: Some(system.to_owned()),
            });
        }
        if !self.eat("PUBLIC") {
            return Err(self.error("expected `SYSTEM` or `PUBLIC`"));
        }
        self.require_space("after `PUBLIC`")?;
        let public = self.pubid_literal()?;
        let spaced = self.skip_space();
        let system = if spaced && matches!(self.peek(), Some('"' | '\'')) {
            Some(self.literal("of the system identifier")?.0.to_owned())
        } else if public_alone {
            None
        } else if spaced {
            return Err(self.error("expected the system identifier after the public identifier"));
        } else {
            return Err(self.error("expected white space after the public identifier"));
        };
        Ok(ExternalId {
            public_id: Some(public),
            system_id: system,
        })
    }

    fn pubid_literal(&mut self) -> Result<String> {
        let quote = self.quote("of the public identifier")?;
        let start = self.pos;
        let rest = self.rest();
        let Some(len) = rest.find(quote) else {
            return Err(self.error(format!("public identifier is not closed; add `{quote}`")));
        };
        if let Some((index, c)) = rest[..len].char_indices().find(|&(_, c)| !is_pubid_char(c)) {
            return Err(parse_error(
                format!("character `{c}` is not allowed in a public identifier"),
                self.base + start + index,
            ));
        }
        self.pos += len + 1;
        Ok(rest[..len].to_owned())
    }

    fn internal_subset(&mut self) -> Result<()> {
        loop {
            self.skip_space();
            match self.peek() {
                None => return Err(self.error("the internal subset is not closed; add `]>`")),
                Some(']') => return Ok(()),
                Some('%') => self.parameter_reference()?,
                Some('<') => self.markup_declaration()?,
                Some(c) => {
                    return Err(
                        self.error(format!("unexpected character `{c}` in the internal subset"))
                    );
                }
            }
        }
    }

    fn parameter_reference(&mut self) -> Result<()> {
        self.pos += 1;
        self.name("in the parameter entity reference")?;
        self.expect(";", "after the parameter entity name")?;
        self.dtd.unread_declarations = true;
        if !self.dtd.standalone {
            self.processing = false;
        }
        Ok(())
    }

    fn markup_declaration(&mut self) -> Result<()> {
        let start = self.pos;
        if self.eat("<!--") {
            self.comment()
        } else if self.eat("<?") {
            self.processing_instruction()
        } else if self.eat("<!ENTITY") {
            self.entity_declaration(start)
        } else if self.eat("<!ELEMENT") {
            self.element_declaration()
        } else if self.eat("<!ATTLIST") {
            self.attlist_declaration()
        } else if self.eat("<!NOTATION") {
            self.notation_declaration()
        } else {
            Err(self.error(
                "unknown markup declaration; the internal subset accepts `<!ENTITY`, \
                 `<!ELEMENT`, `<!ATTLIST`, `<!NOTATION`, comments and processing instructions",
            ))
        }
    }

    fn comment(&mut self) -> Result<()> {
        let start = self.pos;
        let Some(len) = self.rest().find("--") else {
            return Err(self.error("comment is not closed; add `-->`"));
        };
        self.check_chars(&self.rest()[..len], start)?;
        self.pos += len + 2;
        if !self.eat(">") {
            return Err(self.error("`--` is not allowed inside a comment"));
        }
        Ok(())
    }

    fn processing_instruction(&mut self) -> Result<()> {
        let target = self.name("for the processing instruction target")?;
        if target.eq_ignore_ascii_case("xml") {
            return Err(self.error(format!(
                "processing instruction target `{target}` is reserved"
            )));
        }
        if self.eat("?>") {
            return Ok(());
        }
        self.require_space("after the processing instruction target")?;
        let start = self.pos;
        let Some(len) = self.rest().find("?>") else {
            return Err(self.error("processing instruction is not closed; add `?>`"));
        };
        self.check_chars(&self.rest()[..len], start)?;
        self.pos += len + 2;
        Ok(())
    }

    fn entity_declaration(&mut self, start: usize) -> Result<()> {
        self.require_space("after `<!ENTITY`")?;
        let parameter = self.eat("%");
        if parameter {
            self.require_space("after `%` in the parameter entity declaration")?;
        }
        let name = self.name("for the entity")?;
        self.require_space("after the entity name")?;
        let definition = if matches!(self.peek(), Some('"' | '\'')) {
            EntityDefinition::Internal(self.entity_value()?)
        } else {
            self.external_entity(parameter)?
        };
        self.skip_space();
        self.expect(">", "at the end of the entity declaration")?;
        if !parameter && self.processing {
            let span = self.base + start..self.base + self.pos;
            self.dtd.add_entity(EntityDecl {
                name: name.to_owned(),
                definition,
                span,
            });
        }
        Ok(())
    }

    fn external_entity(&mut self, parameter: bool) -> Result<EntityDefinition> {
        let id = self.external_id(false)?;
        let save = self.pos;
        let spaced = self.skip_space();
        if !self.eat("NDATA") {
            self.pos = save;
            return Ok(EntityDefinition::External(id));
        }
        if !spaced {
            return Err(self.error("expected white space before `NDATA`"));
        }
        if parameter {
            return Err(self.error("a parameter entity cannot be unparsed; remove `NDATA`"));
        }
        self.require_space("after `NDATA`")?;
        let notation = self.name("for the notation")?.to_owned();
        Ok(EntityDefinition::Unparsed { id, notation })
    }

    fn entity_value(&mut self) -> Result<String> {
        let quote = self.quote("of the entity value")?;
        let mut value = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.error(format!("entity value is not closed; add `{quote}`")));
            };
            match c {
                _ if c == quote => {
                    self.pos += 1;
                    return Ok(value);
                }
                '%' => {
                    return Err(self.error(
                        "parameter entity references are not allowed inside declarations \
                         of the internal subset",
                    ));
                }
                '&' => self.entity_value_reference(&mut value)?,
                '\r' => {
                    self.pos += 1;
                    self.eat("\n");
                    value.push('\n');
                }
                _ if is_xml_char(c) => {
                    self.pos += c.len_utf8();
                    value.push(c);
                }
                _ => {
                    return Err(self.error(format!(
                        "character U+{:04X} is not allowed in XML",
                        u32::from(c)
                    )));
                }
            }
        }
    }

    fn entity_value_reference(&mut self, value: &mut String) -> Result<()> {
        match entity::reference_at(self.rest()) {
            Some((Reference::Char(c), len)) => {
                value.push(c);
                self.pos += len;
                Ok(())
            }
            Some((Reference::Named(_), len)) => {
                value.push_str(&self.rest()[..len]);
                self.pos += len;
                Ok(())
            }
            None => Err(self.error(
                "`&` must start an entity or character reference; write `&#38;#38;` for `&`",
            )),
        }
    }

    fn element_declaration(&mut self) -> Result<()> {
        self.require_space("after `<!ELEMENT`")?;
        self.name("for the element")?;
        self.require_space("after the element name")?;
        if !self.eat("EMPTY") && !self.eat("ANY") {
            self.expect("(", "or `EMPTY` or `ANY` for the content model")?;
            self.skip_space();
            if self.eat("#PCDATA") {
                self.mixed_content()?;
            } else {
                self.children_group()?;
            }
        }
        self.skip_space();
        self.expect(">", "at the end of the element declaration")
    }

    fn mixed_content(&mut self) -> Result<()> {
        let mut names = false;
        loop {
            self.skip_space();
            if self.eat(")") {
                if names && !self.eat("*") {
                    return Err(self.error("mixed content with element names must end with `)*`"));
                }
                if !names {
                    self.eat("*");
                }
                return Ok(());
            }
            self.expect("|", "between names of mixed content")?;
            self.skip_space();
            self.name("in mixed content")?;
            names = true;
        }
    }

    fn children_group(&mut self) -> Result<()> {
        self.content_particle()?;
        let mut separator = None;
        loop {
            self.skip_space();
            if self.eat(")") {
                break;
            }
            let found = match self.peek() {
                Some(c @ ('|' | ',')) => c,
                _ => return Err(self.error("expected `|`, `,` or `)` in the content model")),
            };
            if separator.is_some_and(|known| known != found) {
                return Err(self.error("a content model group cannot mix `|` and `,`"));
            }
            separator = Some(found);
            self.pos += 1;
            self.content_particle()?;
        }
        self.content_modifier();
        Ok(())
    }

    fn content_particle(&mut self) -> Result<()> {
        self.skip_space();
        if self.eat("(") {
            self.skip_space();
            if self.rest().starts_with("#PCDATA") {
                return Err(self.error("`#PCDATA` is allowed only first in the outer group"));
            }
            if self.depth >= MAX_GROUP_DEPTH {
                return Err(Error::new(
                    ErrorCode::Limit,
                    format!(
                        "content model groups are nested deeper than {MAX_GROUP_DEPTH} levels; flatten the content model"
                    ),
                )
                .with_location(byte_location(self.base + self.pos)));
            }
            self.depth += 1;
            let group = self.children_group();
            self.depth -= 1;
            return group;
        }
        self.name("in the content model")?;
        self.content_modifier();
        Ok(())
    }

    fn content_modifier(&mut self) {
        if matches!(self.peek(), Some('?' | '*' | '+')) {
            self.pos += 1;
        }
    }

    fn notation_declaration(&mut self) -> Result<()> {
        self.require_space("after `<!NOTATION`")?;
        self.name("for the notation")?;
        self.require_space("after the notation name")?;
        self.external_id(true)?;
        self.skip_space();
        self.expect(">", "at the end of the notation declaration")
    }

    fn attlist_declaration(&mut self) -> Result<()> {
        self.require_space("after `<!ATTLIST`")?;
        let element = self.name("for the element")?;
        loop {
            let spaced = self.skip_space();
            if self.eat(">") {
                return Ok(());
            }
            if !spaced {
                return Err(self.error("expected white space before the attribute name"));
            }
            let name = self.name("for the attribute")?;
            self.require_space("after the attribute name")?;
            let kind = self.attribute_type()?;
            self.require_space("after the attribute type")?;
            let default = self.attribute_default()?;
            if self.processing {
                self.dtd.add_attribute(AttributeDecl {
                    element: element.to_owned(),
                    name: name.to_owned(),
                    kind,
                    default,
                });
            }
        }
    }

    fn attribute_type(&mut self) -> Result<AttributeType> {
        for (keyword, kind) in &ATTRIBUTE_TYPES {
            if self.eat(keyword) {
                return Ok(kind.clone());
            }
        }
        if self.eat("NOTATION") {
            self.require_space("after `NOTATION`")?;
            self.expect("(", "after `NOTATION`")?;
            return Ok(AttributeType::Notation(self.enumeration(true)?));
        }
        if self.eat("(") {
            return Ok(AttributeType::Enumeration(self.enumeration(false)?));
        }
        Err(self.error("expected an attribute type such as `CDATA`, `ID` or `(a|b)`"))
    }

    fn enumeration(&mut self, names: bool) -> Result<Vec<String>> {
        let mut values = Vec::new();
        loop {
            self.skip_space();
            let value = if names {
                self.name("in the notation list")?
            } else {
                self.nmtoken()?
            };
            values.push(value.to_owned());
            self.skip_space();
            if self.eat(")") {
                return Ok(values);
            }
            self.expect("|", "between enumerated values")?;
        }
    }

    fn attribute_default(&mut self) -> Result<AttributeDefault> {
        if self.eat("#REQUIRED") {
            return Ok(AttributeDefault::Required);
        }
        if self.eat("#IMPLIED") {
            return Ok(AttributeDefault::Implied);
        }
        let fixed = self.eat("#FIXED");
        if fixed {
            self.require_space("after `#FIXED`")?;
        }
        let (literal, start) = self.literal("of the attribute default")?;
        self.check_attribute_literal(literal, start)?;
        Ok(AttributeDefault::Value {
            literal: literal.to_owned(),
            offset: self.base + start,
            fixed,
        })
    }

    fn check_attribute_literal(&self, literal: &str, start: usize) -> Result<()> {
        for (index, c) in literal.char_indices() {
            let at = self.base + start + index;
            if c == '<' {
                return Err(parse_error(
                    "`<` is not allowed in attribute values; write `&lt;`",
                    at,
                ));
            }
            if c != '&' {
                continue;
            }
            match entity::reference_at(&literal[index..]) {
                Some((Reference::Named(name), _)) => self.check_declared_before(name, at)?,
                Some((Reference::Char(_), _)) => {}
                None => {
                    return Err(parse_error(
                        "`&` must start an entity or character reference; write `&amp;`",
                        at,
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_declared_before(&self, name: &str, at: usize) -> Result<()> {
        if entity::is_predefined(name)
            || self.dtd.entity(name).is_some()
            || self.dtd.unread_declarations && !self.dtd.standalone
        {
            return Ok(());
        }
        Err(parse_error(
            format!("entity `{name}` must be declared before the attribute default that uses it"),
            at,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element_declaration(model: &str) -> String {
        format!("<!DOCTYPE a [<!ELEMENT a {model}>]>")
    }

    #[test]
    fn nested_content_model_groups_up_to_the_limit_are_accepted() {
        let depth = MAX_GROUP_DEPTH + 1;
        let model = format!("{}b{}", "(".repeat(depth), ")".repeat(depth));
        assert!(Dtd::parse(&element_declaration(&model), 0, false).is_ok());
    }

    #[test]
    fn content_model_groups_nested_beyond_the_limit_report_a_limit_error() {
        let depth = MAX_GROUP_DEPTH + 2;
        let model = format!("{}b{}", "(".repeat(depth), ")".repeat(depth));
        let error = Dtd::parse(&element_declaration(&model), 0, false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
    }

    #[test]
    fn unclosed_deep_content_model_does_not_exhaust_the_stack() {
        let model = "(".repeat(200_000);
        let error = Dtd::parse(&element_declaration(&model), 0, false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
    }

    #[test]
    fn first_entity_declaration_is_binding() {
        let dtd = Dtd::parse(
            r#"<!DOCTYPE a [<!ENTITY e "one"><!ENTITY e "two">]>"#,
            0,
            false,
        )
        .unwrap();
        assert_eq!(
            dtd.entity("e").unwrap().definition,
            EntityDefinition::Internal("one".to_owned())
        );
        assert_eq!(dtd.entities().len(), 1);
    }

    #[test]
    fn first_attribute_declaration_is_binding_and_lookup_is_by_name() {
        let text = "<!DOCTYPE a [<!ATTLIST e x CDATA #IMPLIED y ID #IMPLIED>\
                    <!ATTLIST e x ID #IMPLIED><!ATTLIST f x NMTOKEN #IMPLIED>]>";
        let dtd = Dtd::parse(text, 0, false).unwrap();
        assert_eq!(dtd.attribute("e", "x").unwrap().kind, AttributeType::CData);
        assert_eq!(dtd.attribute("e", "y").unwrap().kind, AttributeType::Id);
        assert_eq!(dtd.attribute("f", "x").unwrap().kind, AttributeType::NmToken);
        assert!(dtd.attribute("g", "x").is_none());
        assert!(dtd.attribute("e", "z").is_none());
        assert_eq!(dtd.attributes_of("e").count(), 2);
    }

    #[test]
    fn many_entity_declarations_are_looked_up_by_name() {
        let declarations: String = (0..3000)
            .map(|index| format!("<!ENTITY e{index} \"v{index}\">"))
            .collect();
        let dtd = Dtd::parse(&format!("<!DOCTYPE a [{declarations}]>"), 0, false).unwrap();
        assert_eq!(dtd.entities().len(), 3000);
        assert_eq!(
            dtd.entity("e2999").unwrap().definition,
            EntityDefinition::Internal("v2999".to_owned())
        );
        assert!(dtd.entity("e3000").is_none());
    }

    #[test]
    fn external_entity_is_recorded_but_never_read() {
        let dtd = Dtd::parse(
            r#"<!DOCTYPE a [<!ENTITY x SYSTEM "file:///etc/passwd">]>"#,
            0,
            false,
        )
        .unwrap();
        assert!(matches!(
            dtd.entity("x").unwrap().definition,
            EntityDefinition::External(_)
        ));
    }

    #[test]
    fn declarations_after_a_parameter_reference_are_ignored_unless_standalone() {
        let text = r#"<!DOCTYPE a [%p;<!ENTITY e "v">]>"#;
        let dtd = Dtd::parse(text, 0, false).unwrap();
        assert!(dtd.entity("e").is_none());
        assert!(dtd.has_unread_declarations());
        assert!(Dtd::parse(text, 0, true).unwrap().entity("e").is_some());
    }

    #[test]
    fn edit_guard_allows_insertions_at_both_ends_only() {
        let guard = |range: Range<usize>| guard_doctype_edit(&(10..20), &range);
        assert!(guard(10..10).is_ok());
        assert!(guard(20..20).is_ok());
        assert!(guard(15..15).is_err());
        assert!(guard(0..10).is_ok());
        assert!(guard(19..25).is_err());
    }
}
