// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Entity and character references.
//!
//! [`DtdEntities`] replaces the five predefined entities, character
//! references and the internal general entities declared in the internal
//! subset of the document type declaration. Nested expansion is limited to
//! [`MAX_ENTITY_DEPTH`] levels and the bytes produced by entity expansion in
//! one document to [`MAX_EXPANSION_BYTES`]; both limits bound exponential
//! ("billion laughs") and quadratic expansion with `E_LIMIT`.
//!
//! External entities, parameter entities and unparsed entities are never
//! resolved. A value that depends on an external entity, or on a declaration
//! that may be in the unread external subset, fails with
//! `E_UNSUPPORTED_CONSTRUCT` and construct `xml.external_entity`; the rest
//! of the document stays readable.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::cdata::Context;
use super::dtd::{self, Dtd, EntityDefinition};
use super::tree::{Node, XmlTree};

/// Maximum nesting depth of entity references; a reference that appears
/// directly in the document has depth 1.
pub const MAX_ENTITY_DEPTH: usize = 8;

/// Maximum number of bytes that entity expansion may produce in one
/// document (1 MiB).
pub const MAX_EXPANSION_BYTES: usize = 1 << 20;

/// Construct reported when a value depends on an external entity.
pub const EXTERNAL_ENTITY: &str = "xml.external_entity";

/// Construct reported when a value depends on an internal entity whose
/// replacement text contains markup.
pub const ENTITY_MARKUP: &str = "xml.entity_markup";

/// Construct names reported by this module with `E_UNSUPPORTED_CONSTRUCT`.
pub const CONSTRUCTS: [&str; 2] = [EXTERNAL_ENTITY, ENTITY_MARKUP];

/// Name of the element that wraps a replacement text with markup when it is
/// checked as content.
const FRAGMENT_ROOT: &str = "fragment";

const PREDEFINED: [(&str, char); 5] = [
    ("lt", '<'),
    ("gt", '>'),
    ("amp", '&'),
    ("apos", '\''),
    ("quot", '"'),
];

/// Reference recognized at the start of a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reference<'a> {
    /// Character reference with the character it denotes.
    Char(char),
    /// Entity reference with the entity name.
    Named(&'a str),
}

/// Recognizes the reference at the start of `text`, which starts with `&`,
/// and returns it with its length in bytes.
///
/// Returns `None` for malformed references and for character references
/// that do not denote an XML character.
pub(super) fn reference_at(text: &str) -> Option<(Reference<'_>, usize)> {
    let body = text.strip_prefix('&')?;
    if let Some(number) = body.strip_prefix('#') {
        let (digits, radix, prefix) = match number.strip_prefix('x') {
            Some(hex) => (hex, 16, 2),
            None => (number, 10, 1),
        };
        let len = digits
            .find(|c: char| !c.is_digit(radix))
            .unwrap_or(digits.len());
        if len == 0 || !digits[len..].starts_with(';') {
            return None;
        }
        let value = u32::from_str_radix(&digits[..len], radix).ok()?;
        let c = char::from_u32(value).filter(|&c| dtd::is_xml_char(c))?;
        return Some((Reference::Char(c), 1 + prefix + len + 1));
    }
    let len = dtd::name_len(body);
    if len == 0 || !body[len..].starts_with(';') {
        return None;
    }
    Some((Reference::Named(&body[..len]), 1 + len + 1))
}

/// Whether `name` is one of the five predefined entities.
pub(super) fn is_predefined(name: &str) -> bool {
    predefined(name).is_some()
}

fn predefined(name: &str) -> Option<char> {
    PREDEFINED
        .iter()
        .find(|(entity, _)| *entity == name)
        .map(|&(_, c)| c)
}

/// General entity reference found in a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityReference {
    /// Entity name.
    pub name: String,
    /// Byte range of the reference (`&name;`) in the document.
    pub span: Range<usize>,
}

/// Value with its references replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// Value after replacement and normalization.
    pub value: String,
    /// References to declared general entities, in document order;
    /// predefined entities and character references are not listed.
    pub entities: Vec<EntityReference>,
}

/// Replacement of an entity reference in content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replacement {
    /// Character data, with every reference replaced.
    Text(String),
    /// Well-formed content with markup. Nested general entity references
    /// are expanded; predefined entities and character references remain
    /// as written.
    Markup(String),
}

#[derive(Debug)]
struct Expansion {
    source: String,
    height: usize,
    markup: bool,
}

#[derive(Debug, Default)]
struct State {
    produced: usize,
    active: Vec<String>,
    cache: HashMap<String, Arc<Expansion>>,
}

/// General entities of one document.
///
/// The expansion budget is shared by every call on the same value, so a
/// document uses one `DtdEntities` for all its values. The type is the
/// entity resolver that [`install`] sets on an [`XmlTree`].
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::dtd::Dtd;
/// use nurt_doc::formats::xml::entity::{DtdEntities, Replacement};
///
/// let dtd = Dtd::parse(r#"<!DOCTYPE a [<!ENTITY v "1.0">]>"#, 0, false).unwrap();
/// let entities = DtdEntities::new(Some(dtd));
/// let decoded = entities.decode_attribute("v&v;&#x2D;&lt;", 0, false).unwrap();
/// assert_eq!(decoded.value, "v1.0-<");
/// assert_eq!(entities.replacement("v", 0).unwrap(), Replacement::Text("1.0".to_owned()));
/// ```
#[derive(Debug)]
pub struct DtdEntities {
    dtd: Option<Dtd>,
    state: Mutex<State>,
}

impl DtdEntities {
    /// Creates the entities of a document with an optional document type
    /// declaration.
    #[must_use]
    pub fn new(dtd: Option<Dtd>) -> Self {
        Self {
            dtd,
            state: Mutex::new(State::default()),
        }
    }

    /// Document type declaration.
    #[must_use]
    pub fn dtd(&self) -> Option<&Dtd> {
        self.dtd.as_ref()
    }

    /// Number of bytes produced by entity expansion so far.
    #[must_use]
    pub fn produced(&self) -> usize {
        self.state().produced
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replacement of the general entity reference `&name;` that appears in
    /// content at byte `offset`.
    ///
    /// # Errors
    ///
    /// Returns `E_UNSUPPORTED_CONSTRUCT` with construct `xml.external_entity`
    /// when the value depends on an external entity, or on a declaration
    /// that may be in the unread external subset or parameter entity.
    /// Returns `E_DOC_PARSE` for an undeclared entity, an unparsed entity, a
    /// recursive entity and a replacement text that is not well-formed
    /// content. Returns `E_LIMIT` when the nesting depth exceeds
    /// [`MAX_ENTITY_DEPTH`] or the expansion exceeds [`MAX_EXPANSION_BYTES`].
    pub fn replacement(&self, name: &str, offset: usize) -> Result<Replacement> {
        self.content_replacement(name, Some(offset))
    }

    fn content_replacement(&self, name: &str, at: Option<usize>) -> Result<Replacement> {
        if let Some(c) = predefined(name) {
            return Ok(Replacement::Text(c.to_string()));
        }
        let expansion = self.expand(name, Context::Text, 1, at)?;
        self.charge(expansion.source.len(), at)?;
        if !expansion.markup {
            return Ok(Replacement::Text(decode_source(&expansion.source, false)));
        }
        let fragment = format!("<{FRAGMENT_ROOT}>{}</{FRAGMENT_ROOT}>", expansion.source);
        if let Err(cause) = XmlTree::parse(&fragment) {
            return Err(located(
                ErrorCode::DocParse,
                format!("the replacement text of entity `{name}` is not well-formed content"),
                at,
            )
            .with_cause(cause));
        }
        Ok(Replacement::Markup(expansion.source.clone()))
    }

    /// Decodes an attribute value as written between its quotes, starting at
    /// byte `offset` of the document.
    ///
    /// The value is normalized as XML 1.0 section 3.3.3 requires: line ends
    /// and white space characters become spaces, references are replaced,
    /// and for a `tokenized` (non-`CDATA`) type leading and trailing spaces
    /// are removed and runs of spaces collapse to one.
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` for `<` in the value or in a replacement text, a
    /// malformed reference, a character outside XML, and a reference to an
    /// undeclared, external, unparsed or recursive entity. Returns
    /// `E_UNSUPPORTED_CONSTRUCT` with construct `xml.external_entity` for a
    /// reference to an entity that may be declared in the unread external
    /// subset or parameter entity, and `E_LIMIT` as
    /// [`DtdEntities::replacement`] does.
    pub fn decode_attribute(&self, raw: &str, offset: usize, tokenized: bool) -> Result<Decoded> {
        let mut decoded = Decoded {
            value: String::with_capacity(raw.len()),
            entities: Vec::new(),
        };
        let mut index = 0;
        while let Some(c) = raw[index..].chars().next() {
            let at = offset + index;
            index += match c {
                '&' => self.attribute_reference(&raw[index..], at, &mut decoded)?,
                '<' => {
                    return Err(dtd::parse_error(
                        "`<` is not allowed in attribute values; write `&lt;`",
                        at,
                    ));
                }
                '\r' if raw[index + 1..].starts_with('\n') => {
                    decoded.value.push(' ');
                    2
                }
                '\t' | '\n' | '\r' => {
                    decoded.value.push(' ');
                    1
                }
                _ if dtd::is_xml_char(c) => {
                    decoded.value.push(c);
                    c.len_utf8()
                }
                _ => return Err(invalid_char(c, at)),
            };
        }
        if tokenized {
            decoded.value = tokenize(&decoded.value);
        }
        Ok(decoded)
    }

    fn attribute_reference(&self, text: &str, at: usize, decoded: &mut Decoded) -> Result<usize> {
        let Some((reference, len)) = reference_at(text) else {
            return Err(dtd::parse_error(
                "`&` must start an entity or character reference; write `&amp;`",
                at,
            ));
        };
        match reference {
            Reference::Char(c) => decoded.value.push(c),
            Reference::Named(name) => match predefined(name) {
                Some(c) => decoded.value.push(c),
                None => {
                    let value = self.attribute_replacement(name, Some(at))?;
                    decoded.value.push_str(&value);
                    decoded.entities.push(EntityReference {
                        name: name.to_owned(),
                        span: at..at + len,
                    });
                }
            },
        }
        Ok(len)
    }

    fn attribute_replacement(&self, name: &str, at: Option<usize>) -> Result<String> {
        let expansion = self.expand(name, Context::Attribute, 1, at)?;
        self.charge(expansion.source.len(), at)?;
        Ok(decode_source(&expansion.source, true))
    }

    fn expand(
        &self,
        name: &str,
        context: Context,
        depth: usize,
        at: Option<usize>,
    ) -> Result<Arc<Expansion>> {
        if depth > MAX_ENTITY_DEPTH {
            return Err(depth_error(at));
        }
        let cached = self.state().cache.get(name).cloned();
        let expansion = match cached {
            Some(expansion) => expansion,
            None => {
                let replacement = self.internal_replacement(name, context, at)?;
                let expansion =
                    Arc::new(self.expand_replacement(name, replacement, context, depth, at)?);
                self.state()
                    .cache
                    .insert(name.to_owned(), Arc::clone(&expansion));
                expansion
            }
        };
        if depth + expansion.height - 1 > MAX_ENTITY_DEPTH {
            return Err(depth_error(at));
        }
        if context == Context::Attribute && expansion.markup {
            return Err(located(
                ErrorCode::DocParse,
                format!(
                    "the replacement text of entity `{name}` contains `<`, which is not allowed \
                     in attribute values"
                ),
                at,
            ));
        }
        Ok(expansion)
    }

    fn internal_replacement(
        &self,
        name: &str,
        context: Context,
        at: Option<usize>,
    ) -> Result<&str> {
        let Some(decl) = self.dtd.as_ref().and_then(|dtd| dtd.entity(name)) else {
            return Err(self.undeclared(name, at));
        };
        match (&decl.definition, context) {
            (EntityDefinition::Internal(text), _) => Ok(text),
            (EntityDefinition::External(_), Context::Text) => Err(located(
                ErrorCode::UnsupportedConstruct,
                format!("external entity `{name}` is never resolved; its value cannot be read"),
                at,
            )
            .with_found(EXTERNAL_ENTITY)),
            (EntityDefinition::External(_), Context::Attribute) => Err(located(
                ErrorCode::DocParse,
                format!("external entity `{name}` cannot be referenced in an attribute value"),
                at,
            )),
            (EntityDefinition::Unparsed { .. }, _) => Err(located(
                ErrorCode::DocParse,
                format!(
                    "unparsed entity `{name}` can be named only in an attribute of type `ENTITY` \
                     or `ENTITIES`, not referenced"
                ),
                at,
            )),
        }
    }

    /// Whether a reference to an undeclared `name` cannot be decided because
    /// declarations exist that are never read.
    fn may_be_unread(&self) -> bool {
        self.dtd
            .as_ref()
            .is_some_and(|dtd| dtd.has_unread_declarations() && !dtd.is_standalone())
    }

    fn undeclared(&self, name: &str, at: Option<usize>) -> Error {
        if self.may_be_unread() {
            return located(
                ErrorCode::UnsupportedConstruct,
                format!(
                    "entity `{name}` may be declared in the external subset or in a parameter \
                     entity, which are never read"
                ),
                at,
            )
            .with_found(EXTERNAL_ENTITY);
        }
        located(
            ErrorCode::DocParse,
            format!(
                "entity `{name}` is not declared; declare it in the internal subset or use a \
                 character reference"
            ),
            at,
        )
    }

    fn expand_replacement(
        &self,
        name: &str,
        replacement: &str,
        context: Context,
        depth: usize,
        at: Option<usize>,
    ) -> Result<Expansion> {
        if self.state().active.iter().any(|active| active == name) {
            return Err(located(
                ErrorCode::DocParse,
                format!("entity `{name}` refers to itself"),
                at,
            ));
        }
        self.state().active.push(name.to_owned());
        let result = self.substitute(name, replacement, context, depth, at);
        self.state().active.pop();
        result
    }

    fn substitute(
        &self,
        name: &str,
        replacement: &str,
        context: Context,
        depth: usize,
        at: Option<usize>,
    ) -> Result<Expansion> {
        let mut expansion = Expansion {
            source: String::with_capacity(replacement.len()),
            height: 1,
            markup: replacement.contains('<'),
        };
        let mut rest = replacement;
        while let Some(index) = rest.find('&') {
            expansion.source.push_str(&rest[..index]);
            rest = &rest[index..];
            let Some((reference, len)) = reference_at(rest) else {
                return Err(located(
                    ErrorCode::DocParse,
                    format!(
                        "the replacement text of entity `{name}` contains `&` that does not \
                         start a reference; write `&#38;#38;` in the entity value"
                    ),
                    at,
                ));
            };
            match reference {
                Reference::Named(child) if !is_predefined(child) => {
                    let nested = self.expand(child, context, depth + 1, at)?;
                    self.charge(nested.source.len(), at)?;
                    expansion.source.push_str(&nested.source);
                    expansion.height = expansion.height.max(nested.height + 1);
                    expansion.markup |= nested.markup;
                }
                _ => expansion.source.push_str(&rest[..len]),
            }
            rest = &rest[len..];
        }
        expansion.source.push_str(rest);
        Ok(expansion)
    }

    fn charge(&self, bytes: usize, at: Option<usize>) -> Result<()> {
        let mut state = self.state();
        let produced = state.produced.saturating_add(bytes);
        if produced > MAX_EXPANSION_BYTES {
            return Err(located(
                ErrorCode::Limit,
                format!("entity expansion exceeds {MAX_EXPANSION_BYTES} bytes"),
                at,
            ));
        }
        state.produced = produced;
        Ok(())
    }
}

impl super::EntityResolver for DtdEntities {
    fn resolve(&self, name: &str) -> Result<Option<String>> {
        let declared = self
            .dtd
            .as_ref()
            .is_some_and(|dtd| dtd.entity(name).is_some());
        if !declared && !self.may_be_unread() {
            return Ok(None);
        }
        match self.content_replacement(name, None)? {
            Replacement::Text(text) => Ok(Some(text)),
            Replacement::Markup(_) => Err(Error::new(
                ErrorCode::UnsupportedConstruct,
                format!(
                    "entity `{name}` expands to markup; its value is read only from the \
                     unexpanded text"
                ),
            )
            .with_found(ENTITY_MARKUP)),
        }
    }
}

/// Parses the document type declaration of a tree, installs its entities
/// and checks every reference of the document.
///
/// A reference whose value cannot be read (`E_UNSUPPORTED_CONSTRUCT`) does
/// not make the document unreadable; the error is reported when that value
/// is read.
///
/// # Errors
///
/// Returns `E_DOC_PARSE` for a malformed document type declaration or
/// reference and `E_LIMIT` when entity expansion exceeds its limits.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::xml::XmlTree;
/// use nurt_doc::formats::xml::entity::install;
///
/// let mut tree = XmlTree::parse(r#"<!DOCTYPE a [<!ENTITY n "Nurt">]><a>&n;</a>"#)?;
/// install(&mut tree)?;
/// let text = tree.children(tree.root())[0];
/// assert_eq!(tree.chunk_value(text)?.as_deref(), Some("Nurt"));
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
pub fn install(tree: &mut XmlTree) -> Result<()> {
    let Some(doctype) = tree.doctype() else {
        return Ok(());
    };
    let span = doctype.span.clone();
    let dtd = Dtd::parse(&tree.text()[span.clone()], span.start, is_standalone(tree))?;
    tree.set_entity_resolver(Arc::new(DtdEntities::new(Some(dtd))));
    check_references(tree)
}

fn is_standalone(tree: &XmlTree) -> bool {
    tree.declaration().is_some_and(|declaration| {
        declaration.attributes.iter().any(|attribute| {
            &tree.text()[attribute.name.clone()] == "standalone"
                && &tree.text()[attribute.value.clone()] == "yes"
        })
    })
}

fn check_references(tree: &XmlTree) -> Result<()> {
    for id in tree.node_ids() {
        let checked = match tree.node(id) {
            Node::Text(text) => tree.decode(text.span.clone(), Context::Text).map(drop),
            Node::Element(element) => element.attributes.iter().try_for_each(|attribute| {
                tree.decode(attribute.value.clone(), Context::Attribute)
                    .map(drop)
            }),
            _ => Ok(()),
        };
        match checked {
            Err(error) if error.code() != ErrorCode::UnsupportedConstruct => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// Replaces the predefined entities and character references of an expanded
/// replacement text; in an attribute value, white space characters written
/// literally become spaces.
fn decode_source(source: &str, attribute: bool) -> String {
    let mut value = String::with_capacity(source.len());
    let mut index = 0;
    while let Some(c) = source[index..].chars().next() {
        if c == '&'
            && let Some((reference, len)) = reference_at(&source[index..])
        {
            match reference {
                Reference::Char(c) => value.push(c),
                Reference::Named(name) => value.push(predefined(name).unwrap_or('&')),
            }
            index += len;
            continue;
        }
        value.push(if attribute && dtd::is_space(c) {
            ' '
        } else {
            c
        });
        index += c.len_utf8();
    }
    value
}

fn tokenize(value: &str) -> String {
    value
        .split(' ')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn invalid_char(c: char, at: usize) -> Error {
    dtd::parse_error(
        format!("character U+{:04X} is not allowed in XML", u32::from(c)),
        at,
    )
}

fn depth_error(at: Option<usize>) -> Error {
    located(
        ErrorCode::Limit,
        format!("entity references are nested deeper than {MAX_ENTITY_DEPTH} levels"),
        at,
    )
}

fn located(code: ErrorCode, message: String, at: Option<usize>) -> Error {
    let error = Error::new(code, message);
    match at {
        Some(at) => error.with_location(dtd::byte_location(at)),
        None => error,
    }
}
