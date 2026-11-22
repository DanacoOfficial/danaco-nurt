// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Element tree built from the `tree-sitter-html` syntax tree.

use tree_sitter::{Node as SyntaxNode, Parser};

/// Node of the element tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Node {
    /// An element.
    Element(Element),
    /// Text with character references decoded.
    Text(String),
}

/// An element with its attributes and children.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Element {
    /// Tag name in lower case.
    pub(crate) name: String,
    /// Attributes, names in lower case, values decoded.
    pub(crate) attrs: Vec<(String, String)>,
    /// Children in document order.
    pub(crate) children: Vec<Node>,
}

impl Element {
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(attr, _)| attr == name)
            .map(|(_, value)| value.as_str())
    }

    /// Elements below this one, depth first, including this one.
    pub(crate) fn descendants(&self) -> Vec<&Element> {
        let mut out = vec![self];
        let mut index = 0;
        while index < out.len() {
            let current = out[index];
            index += 1;
            for child in &current.children {
                if let Node::Element(element) = child {
                    out.push(element);
                }
            }
        }
        out
    }

    /// First descendant with the given tag name.
    pub(crate) fn find(&self, name: &str) -> Option<&Element> {
        self.descendants()
            .into_iter()
            .find(|element| element.name == name)
    }

    /// Concatenated text of the subtree with whitespace collapsed.
    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        collect_text(self, &mut out);
        collapse_whitespace(&out)
    }
}

fn collect_text(element: &Element, out: &mut String) {
    for child in &element.children {
        match child {
            Node::Text(text) => out.push_str(text),
            Node::Element(child) if !matches!(child.name.as_str(), "script" | "style") => {
                out.push(' ');
                collect_text(child, out);
            }
            Node::Element(_) => {}
        }
    }
}

/// Replaces runs of whitespace with one space and trims both ends.
pub(crate) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses an HTML document into a synthetic root element named `#root`.
pub(crate) fn parse(source: &str) -> Element {
    let mut root = Element {
        name: "#root".to_owned(),
        ..Element::default()
    };
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_html::LANGUAGE.into())
        .is_err()
    {
        root.children.push(Node::Text(source.to_owned()));
        return root;
    }
    let Some(tree) = parser.parse(source, None) else {
        root.children.push(Node::Text(source.to_owned()));
        return root;
    };
    append_children(tree.root_node(), source, &mut root.children);
    root
}

/// Whether the source between two sibling nodes holds whitespace.
///
/// The grammar keeps whitespace out of `text` nodes, so the space between
/// `a <b>c</b>` and around character references is recovered from the gap
/// between the byte ranges of neighbouring nodes.
fn spaced(source: &str, previous_end: Option<usize>, start: usize) -> bool {
    previous_end
        .and_then(|end| source.get(end..start))
        .is_some_and(|gap| gap.chars().any(char::is_whitespace))
}

fn append_children(node: SyntaxNode<'_>, source: &str, out: &mut Vec<Node>) {
    let mut cursor = node.walk();
    let mut previous_end = None;
    for child in node.named_children(&mut cursor) {
        if spaced(source, previous_end, child.start_byte()) {
            out.push(Node::Text(" ".to_owned()));
        }
        previous_end = Some(child.end_byte());
        match child.kind() {
            "element" | "script_element" | "style_element" => {
                out.push(Node::Element(element(child, source)));
            }
            "text" | "raw_text" => out.push(Node::Text(decode_entities(slice(child, source)))),
            "entity" => out.push(Node::Text(decode_entities(slice(child, source)))),
            _ => {}
        }
    }
}

fn element(node: SyntaxNode<'_>, source: &str) -> Element {
    let mut element = Element::default();
    let mut cursor = node.walk();
    let mut previous_end = None;
    for child in node.named_children(&mut cursor) {
        let raw = matches!(element.name.as_str(), "script" | "style");
        if !raw && spaced(source, previous_end, child.start_byte()) {
            element.children.push(Node::Text(" ".to_owned()));
        }
        previous_end = Some(child.end_byte());
        match child.kind() {
            "start_tag" | "self_closing_tag" => read_tag(child, source, &mut element),
            "end_tag" | "erroneous_end_tag" | "comment" => {}
            "element" | "script_element" | "style_element" => {
                element
                    .children
                    .push(Node::Element(self::element(child, source)));
            }
            "raw_text" if matches!(element.name.as_str(), "script" | "style") => {
                element
                    .children
                    .push(Node::Text(slice(child, source).to_owned()));
            }
            "text" | "raw_text" | "entity" => element
                .children
                .push(Node::Text(decode_entities(slice(child, source)))),
            _ => {}
        }
    }
    element
}

fn read_tag(tag: SyntaxNode<'_>, source: &str, element: &mut Element) {
    let mut cursor = tag.walk();
    for part in tag.named_children(&mut cursor) {
        match part.kind() {
            "tag_name" => element.name = slice(part, source).to_ascii_lowercase(),
            "attribute" => element.attrs.push(attribute(part, source)),
            _ => {}
        }
    }
}

fn attribute(node: SyntaxNode<'_>, source: &str) -> (String, String) {
    let mut name = String::new();
    let mut value = String::new();
    let mut cursor = node.walk();
    for part in node.named_children(&mut cursor) {
        match part.kind() {
            "attribute_name" => name = slice(part, source).to_ascii_lowercase(),
            "attribute_value" => value = decode_entities(slice(part, source)),
            "quoted_attribute_value" => {
                let raw = slice(part, source);
                let inner = raw
                    .strip_prefix(['"', '\''])
                    .and_then(|rest| rest.strip_suffix(['"', '\'']))
                    .unwrap_or(raw);
                value = decode_entities(inner);
            }
            _ => {}
        }
    }
    (name, value)
}

fn slice<'a>(node: SyntaxNode<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or_default()
}

/// Named character references decoded by the converter; others stay as
/// written.
const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", "\u{a0}"),
    ("shy", ""),
    ("copy", "©"),
    ("reg", "®"),
    ("trade", "™"),
    ("hellip", "…"),
    ("mdash", "—"),
    ("ndash", "–"),
    ("lsquo", "‘"),
    ("rsquo", "’"),
    ("sbquo", "‚"),
    ("ldquo", "“"),
    ("rdquo", "”"),
    ("bdquo", "„"),
    ("laquo", "«"),
    ("raquo", "\u{bb}"),
    ("bull", "•"),
    ("middot", "·"),
    ("euro", "€"),
    ("pound", "£"),
    ("yen", "¥"),
    ("cent", "¢"),
    ("sect", "§"),
    ("para", "¶"),
    ("deg", "°"),
    ("plusmn", "±"),
    ("times", "×"),
    ("divide", "÷"),
    ("frac12", "½"),
    ("frac14", "\u{bc}"),
    ("frac34", "¾"),
    ("ensp", "\u{2002}"),
    ("emsp", "\u{2003}"),
    ("thinsp", "\u{2009}"),
    ("zwnj", "\u{200c}"),
    ("zwj", "\u{200d}"),
    ("larr", "←"),
    ("rarr", "→"),
    ("uarr", "↑"),
    ("darr", "↓"),
    ("oacute", "\u{f3}"),
    ("Oacute", "\u{d3}"),
    ("eacute", "é"),
    ("aacute", "á"),
    ("uuml", "ü"),
    ("ouml", "ö"),
    ("auml", "ä"),
    ("szlig", "ß"),
];

/// Longest named reference in [`ENTITIES`] plus the terminating `;`.
const MAX_ENTITY: usize = 8;

/// Decodes numeric and the named character references of [`ENTITIES`].
pub(crate) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        match decode_one(rest) {
            Some((decoded, used)) => {
                out.push_str(&decoded);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_one(text: &str) -> Option<(String, usize)> {
    let end = text.get(1..)?.find(';')? + 1;
    if end > MAX_ENTITY + 2 {
        return None;
    }
    let name = &text[1..end];
    let decoded = if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        char::from_u32(code)
            .unwrap_or(char::REPLACEMENT_CHARACTER)
            .to_string()
    } else {
        ENTITIES
            .iter()
            .find(|(entity, _)| *entity == name)
            .map(|(_, value)| (*value).to_owned())?
    };
    Some((decoded, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_named_and_numeric_references() {
        assert_eq!(
            decode_entities("a &amp; b &#261;&#x142; &unknown; &"),
            "a & b \u{105}\u{142} &unknown; &"
        );
    }

    #[test]
    fn builds_elements_with_attributes_and_text() {
        let root = parse(
            "<html lang=\"pl\"><body><p class='x'>Tom &amp; Jerry<br>next</p>\
             <script>var a = '<b>';</script></body></html>",
        );
        let html = root.find("html").unwrap();
        assert_eq!(html.attr("lang"), Some("pl"));
        let p = root.find("p").unwrap();
        assert_eq!(p.attr("class"), Some("x"));
        assert_eq!(p.text(), "Tom & Jerry next");
        let script = root.find("script").unwrap();
        assert!(matches!(&script.children[0], Node::Text(text) if text.contains("<b>")));
    }
}
