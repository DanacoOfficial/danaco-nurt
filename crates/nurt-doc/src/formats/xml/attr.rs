// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::xml::attr
// Responsibility:   Attributes: byte ranges of every part, values and edits.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Attributes: byte ranges of every part, values and edits.
//!
//! The ranges come from a scan of the start tag bytes delimited by the
//! event range of the reader; the reader itself exposes no attribute
//! positions.

use std::ops::Range;

use nurt_contracts::error::Result;

use super::cdata::{Context, check_representable, escape_attribute};
use super::ns::{is_name_char, is_valid_name, parse_clark, split_qname};
use super::tree::{NodeId, XmlTree};
use super::{CONSTRUCT_NAMESPACE_DECLARATION, XmlChange, XmlEdit, doc_parse_error, invalid_value};

/// Attribute of a start tag or pseudo-attribute of the XML declaration.
///
/// All ranges are byte ranges of the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Whitespace between the previous part of the tag and the name.
    pub leading: Range<usize>,
    /// Qualified name as written.
    pub name: Range<usize>,
    /// From the end of the name to the opening quote: `=` with the
    /// whitespace around it.
    pub equals: Range<usize>,
    /// Quote character, `"` or `'`.
    pub quote: char,
    /// Raw value between the quotes, references not decoded.
    pub value: Range<usize>,
    /// From the first character of the name to the closing quote.
    pub span: Range<usize>,
}

/// Attributes of a tag and the whitespace that follows the last one.
#[derive(Debug)]
pub(crate) struct ScannedAttributes {
    pub(crate) attributes: Vec<Attribute>,
    pub(crate) trailing: Range<usize>,
}

/// Scans the attributes in `text[start..end]`, which holds the part of a
/// tag after its name and before the closing delimiter.
pub(crate) fn scan_attributes(text: &str, start: usize, end: usize) -> Result<ScannedAttributes> {
    let mut attributes = Vec::new();
    let mut pos = start;
    loop {
        let leading_start = pos;
        pos = skip_whitespace(text, pos, end);
        if pos == end {
            return Ok(ScannedAttributes {
                attributes,
                trailing: leading_start..end,
            });
        }
        if pos == leading_start {
            return Err(doc_parse_error(
                text,
                pos,
                "attributes must be separated by whitespace; insert a space",
            ));
        }
        let attribute = scan_attribute(text, leading_start..pos, end)?;
        pos = attribute.span.end;
        attributes.push(attribute);
    }
}

/// Scans one attribute whose name starts at `leading.end`.
fn scan_attribute(text: &str, leading: Range<usize>, end: usize) -> Result<Attribute> {
    let name_start = leading.end;
    let name_end = text[name_start..end]
        .find(|c: char| c == '=' || c.is_ascii_whitespace())
        .map_or(end, |offset| name_start + offset);
    let name = &text[name_start..name_end];
    if !is_valid_name(name) {
        return Err(doc_parse_error(
            text,
            name_start,
            format!(
                "`{name}` is not a valid attribute name; names start with a letter, `_` or `:`"
            ),
        ));
    }
    let equals_at = skip_whitespace(text, name_end, end);
    if !text[equals_at..end].starts_with('=') {
        return Err(doc_parse_error(
            text,
            equals_at,
            format!("attribute `{name}` has no value; write `{name}=\"…\"`"),
        ));
    }
    let quote_at = skip_whitespace(text, equals_at + 1, end);
    let quote = match text[quote_at..end].chars().next() {
        Some(c @ ('"' | '\'')) => c,
        _ => {
            return Err(doc_parse_error(
                text,
                quote_at,
                format!("value of attribute `{name}` must be quoted"),
            ));
        }
    };
    let value_start = quote_at + 1;
    let Some(length) = text[value_start..end].find(quote) else {
        return Err(doc_parse_error(
            text,
            quote_at,
            format!("value of attribute `{name}` is not terminated; add the closing quote"),
        ));
    };
    let value = value_start..value_start + length;
    if let Some(offset) = text[value.clone()].find('<') {
        return Err(doc_parse_error(
            text,
            value.start + offset,
            "`<` is not allowed in an attribute value; write `&lt;`",
        ));
    }
    Ok(Attribute {
        leading,
        name: name_start..name_end,
        equals: name_end..quote_at,
        quote,
        value: value.clone(),
        span: name_start..value.end + 1,
    })
}

/// Position of the first non-whitespace byte in `text[pos..end]`.
pub(crate) fn skip_whitespace(text: &str, pos: usize, end: usize) -> usize {
    text[pos..end]
        .find(|c: char| !matches!(c, ' ' | '\t' | '\n' | '\r'))
        .map_or(end, |offset| pos + offset)
}

impl XmlTree {
    /// Decoded value of an attribute, with attribute-value normalization.
    ///
    /// # Errors
    ///
    /// Returns `E_DOC_PARSE` for a malformed reference and the errors of
    /// the entity resolver.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse(r#"<a href="x?a=1&amp;b=2"/>"#)?;
    /// let attribute = &tree.element(tree.root()).unwrap().attributes[0];
    /// assert_eq!(tree.attribute_value(attribute)?, "x?a=1&b=2");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn attribute_value(&self, attribute: &Attribute) -> Result<String> {
        let value = self
            .decode(attribute.value.clone(), Context::Attribute)?
            .value;
        let tokenized = self.resolver().is_some_and(|entities| {
            entities.tokenized(self.owner_name(attribute), self.attribute_name(attribute))
        });
        Ok(if tokenized {
            value
                .split(' ')
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            value
        })
    }

    /// Name of the element whose start tag holds `attribute`. Attribute
    /// values contain no `<`, so the nearest `<` before the attribute opens
    /// the tag.
    fn owner_name(&self, attribute: &Attribute) -> &str {
        let text = self.text();
        let open = text[..attribute.span.start]
            .rfind('<')
            .map_or(0, |at| at + 1);
        let end = text[open..]
            .find(|c: char| !is_name_char(c))
            .map_or(text.len(), |length| open + length);
        &text[open..end]
    }

    /// Qualified name of an attribute as written.
    pub fn attribute_name(&self, attribute: &Attribute) -> &str {
        &self.text()[attribute.name.clone()]
    }

    /// Index of the attribute of `element` addressed by `name`.
    ///
    /// `name` is a qualified name as written, `prefix:local` with the
    /// prefix resolved in the scope of the element, or `{uri}local`.
    /// Unprefixed attributes have no namespace.
    pub fn find_attribute(&self, element: NodeId, name: &str) -> Option<usize> {
        let attributes = &self.element(element)?.attributes;
        if let Some(index) = attributes
            .iter()
            .position(|attribute| self.attribute_name(attribute) == name)
        {
            return Some(index);
        }
        let (namespace, local) = match parse_clark(name) {
            Some(expanded) => (expanded.namespace.map(str::to_owned), expanded.local),
            None => match split_qname(name) {
                (Some(prefix), local) => {
                    (Some(self.lookup_namespace(element, Some(prefix))?), local)
                }
                (None, _) => return None,
            },
        };
        attributes.iter().position(|attribute| {
            let (prefix, candidate_local) = split_qname(self.attribute_name(attribute));
            let candidate_namespace = prefix.and_then(|p| self.lookup_namespace(element, Some(p)));
            candidate_local == local && candidate_namespace == namespace
        })
    }

    /// Edit that sets attribute `name` of `element` to `value`.
    ///
    /// An existing attribute keeps its quote character; references are
    /// written for the quote, `&`, `<`, tab and line breaks. A new
    /// attribute follows the last one with the same leading whitespace and
    /// `=` spacing, so attributes written one per line stay aligned.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` for an invalid name or a value that XML
    /// 1.0 cannot represent, and `E_UNSUPPORTED_CONSTRUCT`
    /// `xml.namespace_declaration` when the namespace of a new attribute
    /// has no prefix in scope.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse("<a\n   x='1'\n   y='2'/>")?;
    /// let change = tree.set_attribute(tree.root(), "z", "it's")?;
    /// assert_eq!(change.apply(tree.text())?, "<a\n   x='1'\n   y='2'\n   z='it&apos;s'/>");
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn set_attribute(&self, element: NodeId, name: &str, value: &str) -> Result<XmlChange> {
        check_representable(value)?;
        let mut change = XmlChange::default();
        let Some(el) = self.element(element) else {
            return Err(invalid_value(
                "attributes belong to elements; address an element",
            ));
        };
        if let Some(index) = self.find_attribute(element, name) {
            let attribute = &el.attributes[index];
            if self.attribute_value(attribute)? != value {
                change.push(XmlEdit::new(
                    attribute.value.clone(),
                    escape_attribute(value, attribute.quote),
                ));
            }
            return Ok(change);
        }
        let written = self.new_attribute_name(element, name)?;
        let markup = match el.attributes.last() {
            Some(last) => format!(
                "{}{written}{}{q}{}{q}",
                &self.text()[last.leading.clone()],
                &self.text()[last.equals.clone()],
                escape_attribute(value, last.quote),
                q = last.quote,
            ),
            None => {
                let q = self.style().quote;
                format!(" {written}={q}{}{q}", escape_attribute(value, q))
            }
        };
        let at = el
            .attributes
            .last()
            .map_or(el.name.end, |last| last.span.end);
        change.push(XmlEdit::new(at..at, markup));
        Ok(change)
    }

    /// Written name for a new attribute of `element`.
    fn new_attribute_name(&self, element: NodeId, name: &str) -> Result<String> {
        let written = self.written_name(element, None, name, false).ok_or_else(|| {
            super::unsupported(
                CONSTRUCT_NAMESPACE_DECLARATION,
                format!("namespace of attribute `{name}` has no prefix in scope; declare the prefix with an `xmlns:` attribute first"),
            )
        })?;
        if !is_valid_name(&written) {
            return Err(invalid_value(format!(
                "`{written}` is not a valid attribute name; names start with a letter, `_` or `:`"
            )));
        }
        Ok(written)
    }

    /// Edit that removes attribute `name` of `element` together with the
    /// whitespace before it.
    ///
    /// The change is empty when the attribute does not exist.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::xml::XmlTree;
    ///
    /// let tree = XmlTree::parse(r#"<a x="1" y="2"/>"#)?;
    /// let change = tree.remove_attribute(tree.root(), "x");
    /// assert_eq!(change.apply(tree.text())?, r#"<a y="2"/>"#);
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn remove_attribute(&self, element: NodeId, name: &str) -> XmlChange {
        let mut change = XmlChange::default();
        if let (Some(el), Some(index)) = (self.element(element), self.find_attribute(element, name))
        {
            let attribute = &el.attributes[index];
            change.push(XmlEdit::new(
                attribute.leading.start..attribute.span.end,
                String::new(),
            ));
        }
        change
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_records_every_part_of_an_attribute() {
        let text = r#"<a  x = "1" y='2' >"#;
        let scanned = scan_attributes(text, 2, text.len() - 1).unwrap();
        let x = &scanned.attributes[0];
        assert_eq!(&text[x.leading.clone()], "  ");
        assert_eq!(&text[x.name.clone()], "x");
        assert_eq!(&text[x.equals.clone()], " = ");
        assert_eq!(x.quote, '"');
        assert_eq!(&text[x.value.clone()], "1");
        assert_eq!(&text[x.span.clone()], r#"x = "1""#);
        assert_eq!(scanned.attributes[1].quote, '\'');
        assert_eq!(&text[scanned.trailing.clone()], " ");
    }

    #[test]
    fn undeclared_prefix_does_not_find_an_unprefixed_attribute() {
        let tree = XmlTree::parse(r#"<a id="1" xmlns:p="urn:p" p:id="2"/>"#).unwrap();
        let root = tree.root();
        assert_eq!(tree.find_attribute(root, "id"), Some(0));
        assert_eq!(tree.find_attribute(root, "q:id"), None);
        assert_eq!(tree.find_attribute(root, "p:id"), Some(2));
        assert_eq!(tree.find_attribute(root, "{urn:p}id"), Some(2));
        assert_eq!(tree.find_attribute(root, "{}id"), Some(0));
    }

    #[test]
    fn set_attribute_with_undeclared_prefix_does_not_overwrite_unprefixed_one() {
        let tree = XmlTree::parse(r#"<a id="1"/>"#).unwrap();
        assert!(tree.set_attribute(tree.root(), "q:id", "2").is_err());
    }

    #[test]
    fn scan_rejects_malformed_attributes() {
        for text in [
            r#"<a x="1"y="2">"#,
            "<a x>",
            "<a x=1>",
            r#"<a x="1>"#,
            r#"<a x="<">"#,
            r#"<a 1x="1">"#,
        ] {
            assert!(
                scan_attributes(text, 2, text.len() - 1).is_err(),
                "accepted {text}"
            );
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
