// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! XML property lists on the lossless XML tree.
//!
//! The document element `plist` holds one value element. A `dict` holds
//! pairs of a `key` element and a value element, an `array` holds value
//! elements, and the scalar elements are `string`, `integer`, `real`,
//! `true`, `false`, `date` and `data`. Key paths follow the property list
//! structure (`a.b[0]` is element 0 of the array under key `b` of the
//! dictionary under key `a`), not the XML elements.
//!
//! A scalar replaced by a value of the same element keeps its tags and only
//! its content changes. A value of another type replaces the whole value
//! element (`<integer>1</integer>` becomes `<string>x</string>`). New
//! entries take the indentation of their siblings. Comments, the XML
//! declaration and the document type declaration keep their bytes.

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Bytes, Date, DateKind, Value};

use super::PlistEdit;
use super::base64;
use super::layout::{DEFAULT_INDENT, indentation, removal, starts_line};
use super::plan::{SEMANTICS, TextSyntax, next_sibling};
use super::value::{self, Hint};
use crate::formats::xml::{NodeId as XmlNode, XmlTree};
use crate::model::{DocTree, NodeId, NodeKind, NodeSpec, Span, TreeBuilder, doc_parse};

/// Deepest nesting of arrays and dictionaries accepted.
const MAX_DEPTH: usize = 512;

/// Name of the document element.
const ROOT_ELEMENT: &str = "plist";

/// Layout facts of an XML property list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XmlSyntax {
    eol: &'static str,
    unit: String,
}

/// Parses the XML property list that starts at `base` in `text` and builds
/// its node tree with byte ranges in `text`.
///
/// # Errors
///
/// Returns `E_DOC_PARSE` for text that is not well-formed XML or not a
/// property list, and the errors of the XML tree for entity references.
pub(crate) fn build(text: &str, base: usize) -> Result<(DocTree, XmlSyntax), Error> {
    let xml = XmlTree::parse(&text[base..])?;
    let root = xml
        .document_nodes()
        .iter()
        .copied()
        .find(|node| xml.element(*node).is_some())
        .ok_or_else(|| parse_error(text, base, "the document has no element; a property list starts with <plist>"))?;
    let root_start = element_range(&xml, root)?.start + base;
    if xml.name(root) != ROOT_ELEMENT {
        let message = format!("the document element is <{}>; a property list has <plist>", xml.name(root));
        return Err(parse_error(text, root_start, &message));
    }
    let values: Vec<XmlNode> = xml.child_elements(root).collect();
    let [value] = values.as_slice() else {
        return Err(parse_error(text, root_start, "the plist element must hold exactly one value"));
    };
    let unit = if xml.style.indent_unit.is_empty() { DEFAULT_INDENT.to_owned() } else { xml.style.indent_unit.clone() };
    let syntax = XmlSyntax { eol: xml.style.eol, unit };
    let mut walk = Walk { xml: &xml, text, base, builder: TreeBuilder::new(text.as_bytes()) };
    walk.add(None, *value, None, 0)?;
    Ok((walk.builder.finish(), syntax))
}

fn element_range(xml: &XmlTree, node: XmlNode) -> Result<Range<usize>, Error> {
    xml.element(node)
        .map(|element| element.span.clone())
        .ok_or_else(|| Error::new(ErrorCode::DocParse, "invalid plist document: an element was expected"))
}

fn parse_error(text: &str, offset: usize, message: &str) -> Error {
    doc_parse(&DocFormatId::PLIST, text.as_bytes(), Some(offset), message)
}

struct Walk<'a> {
    xml: &'a XmlTree,
    text: &'a str,
    base: usize,
    builder: TreeBuilder<'a>,
}

impl Walk<'_> {
    fn add(&mut self, parent: Option<NodeId>, element: XmlNode, key: Option<(String, Range<usize>)>, depth: usize) -> Result<(), Error> {
        let range = element_range(self.xml, element)?;
        let start = range.start + self.base;
        if depth > MAX_DEPTH {
            return Err(parse_error(self.text, start, "arrays and dictionaries are nested deeper than 512 levels"));
        }
        let value_span = Span::new(start, range.end + self.base);
        let name = self.xml.name(element);
        let kind = match name {
            "dict" => NodeKind::Map,
            "array" => NodeKind::List,
            _ => NodeKind::Scalar,
        };
        let span = key.as_ref().map_or(value_span, |(_, key)| Span::new(key.start + self.base, value_span.end));
        let mut spec = NodeSpec::new(kind, span).with_value_span(value_span);
        if let Some((key_text, key_range)) = &key {
            spec = spec.with_name(key_text.clone()).with_key_span(Span::new(key_range.start + self.base, key_range.end + self.base));
        }
        if kind == NodeKind::Scalar {
            spec = spec.with_value(self.scalar(name, element, start)?);
        } else if !self.text_of(element)?.trim().is_empty() {
            return Err(parse_error(self.text, start, &format!("<{name}> holds text; it may hold only elements")));
        }
        let node = self.builder.node(parent, spec);
        let children: Vec<XmlNode> = self.xml.child_elements(element).collect();
        match kind {
            NodeKind::Map => self.entries(node, &children, depth),
            NodeKind::List => {
                for child in children {
                    self.add(Some(node), child, None, depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn entries(&mut self, node: NodeId, children: &[XmlNode], depth: usize) -> Result<(), Error> {
        for pair in children.chunks(2) {
            let key = pair[0];
            let key_range = element_range(self.xml, key)?;
            if self.xml.name(key) != "key" {
                let message = format!("expected <key> in <dict>, found <{}>", self.xml.name(key));
                return Err(parse_error(self.text, key_range.start + self.base, &message));
            }
            let Some(&value) = pair.get(1) else {
                return Err(parse_error(self.text, key_range.end + self.base, "the last <key> of <dict> has no value"));
            };
            let name = self.text_of(key)?;
            self.add(Some(node), value, Some((name, key_range)), depth + 1)?;
        }
        Ok(())
    }

    /// Text content of an element with references and CDATA decoded.
    fn text_of(&self, element: XmlNode) -> Result<String, Error> {
        let mut text = String::new();
        for chunk in self.xml.text_chunks(element) {
            if let Some(value) = self.xml.chunk_value(chunk)? {
                text.push_str(&value);
            }
        }
        Ok(text)
    }

    fn scalar(&self, name: &str, element: XmlNode, start: usize) -> Result<Value, Error> {
        let content = self.text_of(element)?;
        let trimmed = content.trim();
        let invalid = |what: &str| parse_error(self.text, start, &format!("<{name}> does not hold {what}"));
        let value = match name {
            "string" => Value::Text(content.clone()),
            "integer" => parse_integer(trimmed).ok_or_else(|| invalid("an integer"))?,
            "real" => trimmed.parse().map(Value::Number).map_err(|_| invalid("a number"))?,
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "date" => {
                value::parse_date(trimmed).map_err(|_| invalid("an RFC 3339 date such as 2026-01-31T12:00:00Z"))?;
                Value::Date(Date { kind: DateKind::OffsetDateTime, text: trimmed.to_owned() })
            }
            "data" => Value::Bytes(Bytes::new(base64::decode(trimmed).ok_or_else(|| invalid("Base64 data"))?)),
            other => {
                return Err(parse_error(
                    self.text,
                    start,
                    &format!("<{other}> is not a property list element; use dict, array, string, integer, real, true, false, date or data"),
                ));
            }
        };
        Ok(value)
    }
}

/// Integer in decimal or with the prefix `0x`.
fn parse_integer(text: &str) -> Option<Value> {
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let magnitude: u64 = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    if negative {
        let value = 0i64.checked_sub_unsigned(magnitude)?;
        Some(Value::Int(value))
    } else {
        Some(i64::try_from(magnitude).map_or_else(|_| Value::BigInt(magnitude.to_string()), Value::Int))
    }
}

impl XmlSyntax {
    /// Element of `value`, indented for the line that starts with `indent`.
    fn render(&self, value: &plist::Value, indent: &str) -> Result<String, Error> {
        let inner = format!("{indent}{}", self.unit);
        let eol = self.eol;
        let text = match value {
            plist::Value::Array(items) if items.is_empty() => "<array/>".to_owned(),
            plist::Value::Array(items) => {
                let mut out = String::from("<array>");
                for item in items {
                    out.push_str(&format!("{eol}{inner}{}", self.render(item, &inner)?));
                }
                format!("{out}{eol}{indent}</array>")
            }
            plist::Value::Dictionary(dictionary) if dictionary.is_empty() => "<dict/>".to_owned(),
            plist::Value::Dictionary(dictionary) => {
                let mut out = String::from("<dict>");
                for (key, item) in dictionary {
                    out.push_str(&format!("{eol}{inner}<key>{}</key>{eol}{inner}{}", escape(key), self.render(item, &inner)?));
                }
                format!("{out}{eol}{indent}</dict>")
            }
            plist::Value::Boolean(true) => "<true/>".to_owned(),
            plist::Value::Boolean(false) => "<false/>".to_owned(),
            other => {
                let content = scalar_content(other).ok_or_else(|| {
                    Error::new(ErrorCode::Type, "an XML property list cannot hold this value; pass text, a number, a date, bytes, a list or a map")
                })?;
                let name = element_for(other);
                format!("<{name}>{content}</{name}>")
            }
        };
        Ok(text)
    }

    /// New member of a container: the value element, preceded by its key
    /// element in a dictionary.
    fn member(&self, key: Option<&str>, value: &plist::Value, indent: &str, own_line: bool) -> Result<String, Error> {
        let rendered = self.render(value, indent)?;
        Ok(match key {
            Some(key) if own_line => format!("<key>{}</key>{}{indent}{rendered}", escape(key), self.eol),
            Some(key) => format!("<key>{}</key>{rendered}", escape(key)),
            None => rendered,
        })
    }

    fn add_member(&self, text: &str, tree: &DocTree, container: NodeId, key: Option<&str>, value: &Value) -> Result<PlistEdit, Error> {
        let new = value::from_value(value, Hint::Other)?;
        let node = &tree[container];
        if let Some(&last) = node.children.last() {
            let last = &tree[last];
            let indent = indentation(text, last.span.start);
            let own_line = starts_line(text, last.span.start);
            let member = self.member(key, &new, indent, own_line)?;
            let inserted = if own_line { format!("{}{indent}{member}", self.eol) } else { member };
            return Ok(edit(last.span.end..last.span.end, &inserted));
        }
        let span = node.value_span;
        let parent_indent = indentation(text, span.start);
        let inner = format!("{parent_indent}{}", self.unit);
        let body = format!("{}{inner}{}{}{parent_indent}", self.eol, self.member(key, &new, &inner, true)?, self.eol);
        let old = &text[span.start..span.end];
        let name = element_name(old);
        if old.ends_with("/>") {
            return Ok(edit(span.start..span.end, &format!("<{name}>{body}</{name}>")));
        }
        let close = span.start + old.rfind("</").unwrap_or(old.len());
        let content = &text[span.start..close];
        let trailing = content.len() - content.trim_end().len();
        Ok(edit(close - trailing..close, &body))
    }
}

impl TextSyntax for XmlSyntax {
    fn replace(&self, text: &str, tree: &DocTree, node: NodeId, value: &Value) -> Result<Option<PlistEdit>, Error> {
        let span = tree[node].value_span;
        let old = &text[span.start..span.end];
        let old_name = element_name(old);
        let hint = if old_name == "real" { Hint::Real } else { Hint::Other };
        let new = value::from_value(value, hint)?;
        if value::to_value(&new) == tree.value_of(node, &SEMANTICS, None) {
            return Ok(None);
        }
        if element_for(&new) == old_name {
            if let Some(content) = scalar_content(&new) {
                if old.ends_with("/>") {
                    return Ok(Some(edit(span.start..span.end, &format!("<{old_name}>{content}</{old_name}>"))));
                }
                let open = span.start + old.find('>').map_or(0, |index| index + 1);
                let close = span.start + old.rfind("</").unwrap_or(old.len());
                return Ok(Some(edit(open..close, &content)));
            }
        }
        let rendered = self.render(&new, indentation(text, span.start))?;
        Ok(Some(edit(span.start..span.end, &rendered)))
    }

    fn insert(&self, text: &str, tree: &DocTree, map: NodeId, name: &str, value: &Value) -> Result<PlistEdit, Error> {
        self.add_member(text, tree, map, Some(name), value)
    }

    fn append(&self, text: &str, tree: &DocTree, list: NodeId, value: &Value) -> Result<PlistEdit, Error> {
        self.add_member(text, tree, list, None, value)
    }

    fn removal(&self, text: &str, tree: &DocTree, node: NodeId) -> Range<usize> {
        let span = tree[node].span;
        let next = next_sibling(tree, node).map(|next| tree[next].span.start);
        removal(text, &(span.start..span.end), next)
    }
}

fn edit(range: Range<usize>, text: &str) -> PlistEdit {
    PlistEdit { range, replacement: text.as_bytes().to_vec() }
}

/// Name of the element written in `text`, which starts with `<`.
fn element_name(text: &str) -> &str {
    let name = text.get(1..).unwrap_or_default();
    let end = name.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').unwrap_or(name.len());
    &name[..end]
}

/// Element name of a value.
const fn element_for(value: &plist::Value) -> &'static str {
    match value {
        plist::Value::Array(_) => "array",
        plist::Value::Dictionary(_) => "dict",
        plist::Value::Boolean(true) => "true",
        plist::Value::Boolean(false) => "false",
        plist::Value::Data(_) => "data",
        plist::Value::Date(_) => "date",
        plist::Value::Real(_) => "real",
        plist::Value::Integer(_) | plist::Value::Uid(_) => "integer",
        _ => "string",
    }
}

/// Content of a scalar element; `None` for containers and truth values.
fn scalar_content(value: &plist::Value) -> Option<String> {
    Some(match value {
        plist::Value::String(text) => escape(text),
        plist::Value::Integer(integer) => integer.to_string(),
        plist::Value::Uid(uid) => uid.get().to_string(),
        plist::Value::Real(number) => real_text(*number),
        plist::Value::Date(date) => date.to_xml_format(),
        plist::Value::Data(bytes) => base64::encode(bytes),
        _ => return None,
    })
}

fn real_text(number: f64) -> String {
    if number.is_nan() {
        "nan".to_owned()
    } else if number.is_infinite() {
        if number > 0.0 { "+infinity" } else { "-infinity" }.to_owned()
    } else {
        number.to_string()
    }
}

/// Text with `&`, `<` and `>` escaped, which also keeps `]]>` out of
/// character data.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            character => out.push(character),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::keypath::KeyPath;
    use nurt_contracts::value::Fields;

    use crate::formats::plist::plan::{plan_remove, plan_set, text_remove, text_set};

    const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n";

    fn document(body: &str) -> String {
        format!("{HEAD}<plist version=\"1.0\">\n{body}\n</plist>\n")
    }

    fn set(text: &str, key: &str, value: &Value, create: bool) -> String {
        let (tree, syntax) = build(text, 0).unwrap();
        let key = KeyPath::parse(key).unwrap();
        let ops = plan_set(&tree, &key, value, create).unwrap();
        apply(text, &text_set(&syntax, text, &tree, &key, &ops).unwrap())
    }

    fn remove(text: &str, key: &str) -> String {
        let (tree, syntax) = build(text, 0).unwrap();
        let key = KeyPath::parse(key).unwrap();
        let nodes = plan_remove(&tree, &key).unwrap();
        apply(text, &text_remove(&syntax, text, &tree, &key, &nodes).unwrap())
    }

    fn apply(text: &str, edits: &[PlistEdit]) -> String {
        let mut out = text.as_bytes().to_vec();
        for edit in edits.iter().rev() {
            out.splice(edit.range.clone(), edit.replacement.iter().copied());
        }
        String::from_utf8(out).unwrap()
    }

    fn sample() -> String {
        document(
            "<dict>\n\t<key>Name</key>\n\t<string>App &amp; Co</string>\n\t<key>Version</key>\n\t<integer>3</integer>\n\t<key>Scale</key>\n\t<real>1.5</real>\n\t<key>Tags</key>\n\t<array>\n\t\t<string>a</string>\n\t</array>\n\t<key>Empty</key>\n\t<dict/>\n</dict>",
        )
    }

    #[test]
    fn reads_values_in_property_list_terms() {
        let text = sample();
        let (tree, _) = build(&text, 0).unwrap();
        let Value::Map(fields) = tree.value_of(tree.roots()[0], &SEMANTICS, None) else { panic!("map expected") };
        assert_eq!(fields["Name"], Value::Text("App & Co".into()));
        assert_eq!(fields["Version"], Value::Int(3));
        assert_eq!(fields["Scale"], Value::Number(1.5));
        assert_eq!(fields["Tags"], Value::List(vec![Value::Text("a".into())]));
        assert_eq!(fields["Empty"], Value::Map(Fields::new()));
    }

    #[test]
    fn scalars_keep_their_tags_and_types_change_the_element() {
        let text = sample();
        assert_eq!(set(&text, "Name", &Value::Text("A<B".into()), false), text.replace("App &amp; Co", "A&lt;B"));
        assert_eq!(set(&text, "Version", &Value::Int(4), false), text.replace("<integer>3</integer>", "<integer>4</integer>"));
        assert_eq!(set(&text, "Scale", &Value::Int(2), false), text.replace("<real>1.5</real>", "<real>2</real>"));
        assert_eq!(set(&text, "Version", &Value::Text("4".into()), false), text.replace("<integer>3</integer>", "<string>4</string>"));
        assert_eq!(set(&text, "Version", &Value::Bool(true), false), text.replace("<integer>3</integer>", "<true/>"));
        assert_eq!(set(&text, "Version", &Value::Int(3), false), text);
    }

    #[test]
    fn new_entries_take_sibling_indentation() {
        let text = sample();
        let added = set(&text, "Build", &Value::Int(7), true);
        assert!(added.contains("<dict/>\n\t<key>Build</key>\n\t<integer>7</integer>\n</dict>"), "{added}");
        let appended = set(&text, "Tags[+]", &Value::Text("b".into()), false);
        assert!(appended.contains("<string>a</string>\n\t\t<string>b</string>\n\t</array>"), "{appended}");
        let nested = set(&text, "Empty.k", &Value::List(vec![Value::Int(1)]), true);
        assert!(nested.contains("<dict>\n\t\t<key>k</key>\n\t\t<array>\n\t\t\t<integer>1</integer>\n\t\t</array>\n\t</dict>"), "{nested}");
    }

    #[test]
    fn removing_entries_takes_their_lines() {
        let text = sample();
        let removed = remove(&text, "Version");
        assert!(!removed.contains("Version") && !removed.contains("<integer>3"), "{removed}");
        assert!(removed.contains("</string>\n\t<key>Scale</key>"), "{removed}");
        let removed = remove(&text, "Tags[0]");
        assert!(removed.contains("<array>\n\t</array>"), "{removed}");
    }

    #[test]
    fn rejects_documents_that_are_not_property_lists() {
        for body in ["<dict><key>a</key></dict>", "<dict><string>a</string><string>b</string></dict>", "<integer>x</integer>", "<foo/>", "<date>yesterday</date>", "<data>@@</data>", "<array>text</array>"] {
            let error = build(&document(body), 0).unwrap_err();
            assert_eq!(error.code().as_str(), "E_DOC_PARSE", "body {body}");
        }
        assert_eq!(build("<root/>", 0).unwrap_err().code().as_str(), "E_DOC_PARSE");
        assert_eq!(build("<plist><a/><b/></plist>", 0).unwrap_err().code().as_str(), "E_DOC_PARSE");
    }

    #[test]
    fn integers_dates_and_data_are_decoded() {
        assert_eq!(parse_integer("0x1F"), Some(Value::Int(31)));
        assert_eq!(parse_integer("-5"), Some(Value::Int(-5)));
        assert_eq!(parse_integer("18446744073709551615"), Some(Value::BigInt("18446744073709551615".into())));
        assert_eq!(parse_integer("x"), None);
        let text = document("<array><date>2026-01-31T12:00:00Z</date><data>AAE=</data><real>nan</real></array>");
        let (tree, _) = build(&text, 0).unwrap();
        let Value::List(items) = tree.value_of(tree.roots()[0], &SEMANTICS, None) else { panic!("list expected") };
        assert_eq!(items[1], Value::Bytes(Bytes::new(vec![0, 1])));
        assert_eq!(real_text(f64::INFINITY), "+infinity");
        assert_eq!(escape("a&b<c>"), "a&amp;b&lt;c&gt;");
    }
}
