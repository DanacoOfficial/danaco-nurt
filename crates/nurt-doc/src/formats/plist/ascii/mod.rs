// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-doc::formats::plist::ascii
// Responsibility:   ASCII (OpenStep) property lists: node tree and changes of byte ranges.
// Dependencies:     nurt_contracts, parse
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! ASCII (OpenStep) property lists: node tree and changes of byte ranges.
//!
//! The form has no types besides strings, data, arrays and dictionaries, so
//! a number or a truth value is written as a string (`12`, `YES`) unless the
//! replaced value is a GNUstep typed value (`<*I12>`), which keeps its form.
//! A replaced string keeps its quoting: a quoted string stays quoted with
//! the same quote character, an unquoted string stays unquoted when the new
//! text allows it. New entries follow the indentation, the separator around
//! `=` and the trailing comma style of their siblings.

mod parse;

use std::fmt::Write as _;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Value;

use super::PlistEdit;
use super::layout::{indent_unit, indentation, line_break, removal, starts_line};
use super::plan::{SEMANTICS, TextSyntax, next_sibling, previous_sibling};
use crate::model::{DocTree, NodeId, NodeKind, NodeSpec, Span, TreeBuilder};
use parse::{Body, Item, is_unquoted};

/// Separator between a key and its value in new entries of a dictionary
/// without entries.
const DEFAULT_SEPARATOR: &str = " = ";

/// Bytes of data written in one group of hexadecimal digits when the
/// replaced data is grouped.
const DATA_GROUP_BYTES: usize = 4;

/// Layout facts of an ASCII property list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AsciiSyntax {
    eol: &'static str,
    unit: String,
}

/// Member of a container while the tree is built.
struct Member {
    name: Option<String>,
    span: Range<usize>,
    key_span: Option<Range<usize>>,
}

/// Parses the ASCII property list that starts at `base` in `text` and
/// builds its node tree with byte ranges in `text`.
///
/// # Errors
///
/// Returns `E_DOC_PARSE` for text that is not an ASCII property list.
pub(crate) fn build(text: &str, base: usize) -> Result<(DocTree, AsciiSyntax), Error> {
    let root = parse::parse(&text.as_bytes()[base..])?;
    let mut builder = TreeBuilder::new(text.as_bytes());
    let member = Member {
        name: None,
        span: root.span.clone(),
        key_span: None,
    };
    add(&mut builder, None, &root, member, base);
    let syntax = AsciiSyntax {
        eol: line_break(text),
        unit: indent_unit(text),
    };
    Ok((builder.finish(), syntax))
}

fn add(
    builder: &mut TreeBuilder<'_>,
    parent: Option<NodeId>,
    item: &Item,
    member: Member,
    base: usize,
) {
    let shift = |range: &Range<usize>| Span::new(range.start + base, range.end + base);
    let kind = match &item.body {
        Body::Array(_) => NodeKind::List,
        Body::Dict(_) => NodeKind::Map,
        _ => NodeKind::Scalar,
    };
    let mut spec = NodeSpec::new(kind, shift(&member.span)).with_value_span(shift(&item.span));
    if let Some(name) = member.name {
        spec = spec.with_name(name);
    }
    if let Some(key_span) = &member.key_span {
        spec = spec.with_key_span(shift(key_span));
    }
    if kind == NodeKind::Scalar {
        spec = spec.with_value(item.to_value());
    }
    let node = builder.node(parent, spec);
    match &item.body {
        Body::Array(elements) => {
            for element in elements {
                let end = element
                    .comma
                    .map_or(element.value.span.end, |comma| comma + 1);
                let member = Member {
                    name: None,
                    span: element.value.span.start..end,
                    key_span: None,
                };
                add(builder, Some(node), &element.value, member, base);
            }
        }
        Body::Dict(entries) => {
            for entry in entries {
                let member = Member {
                    name: Some(entry.name.clone()),
                    span: entry.key.span.start..entry.end,
                    key_span: Some(entry.key.span.clone()),
                };
                add(builder, Some(node), &entry.value, member, base);
            }
        }
        _ => {}
    }
}

/// Form of a replaced value that a new value keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Form {
    /// Quote character of a quoted string.
    quote: Option<char>,
    /// Letter of a GNUstep typed value.
    typed: Option<u8>,
    /// Data written in groups separated by spaces.
    grouped: bool,
}

impl Form {
    fn of(text: &str) -> Self {
        let bytes = text.as_bytes();
        match bytes.first() {
            Some(quote @ (b'"' | b'\'')) => Self {
                quote: Some(char::from(*quote)),
                ..Self::default()
            },
            Some(b'<') if bytes.get(1) == Some(&b'*') => Self {
                typed: bytes.get(2).copied(),
                ..Self::default()
            },
            Some(b'<') => Self {
                grouped: bytes.iter().any(u8::is_ascii_whitespace),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

impl AsciiSyntax {
    /// Text of `value` written in the form of the replaced value; containers
    /// span several lines when `multiline` is set, indented one unit deeper
    /// than `indent`.
    fn render(
        &self,
        value: &Value,
        form: Form,
        indent: &str,
        multiline: bool,
    ) -> Result<String, Error> {
        let text = match value {
            Value::Text(text)
            | Value::Path(text)
            | Value::Glob(text)
            | Value::Key(text)
            | Value::Enum(text) => string(text, form),
            Value::Int(number) => typed_or_string(form, b'I', &number.to_string()),
            Value::BigInt(text) | Value::Decimal(text) => string(text, form),
            Value::Number(number) => typed_or_string(form, b'R', &number.to_string()),
            Value::Size(size) => string(&size.bytes().to_string(), form),
            Value::Duration(duration) => string(&duration.as_millis().to_string(), form),
            Value::Bool(flag) => match form.typed {
                Some(b'B') => format!("<*B{}>", if *flag { 'Y' } else { 'N' }),
                _ => string(if *flag { "YES" } else { "NO" }, form),
            },
            Value::Date(date) => typed_or_string(form, b'D', &date.text),
            Value::Instant(instant) => typed_or_string(form, b'D', &instant.to_string()),
            Value::Bytes(bytes) => data(bytes.as_slice(), form.grouped),
            Value::List(items) => self.array(items, indent, multiline)?,
            Value::Map(fields) => {
                let entries: Vec<(&str, &Value)> = fields
                    .iter()
                    .map(|(key, item)| (key.as_str(), item))
                    .collect();
                self.dictionary(&entries, indent, multiline)?
            }
            Value::Record(record) => {
                let entries: Vec<(&str, &Value)> = record
                    .field_names()
                    .filter_map(|name| Some((name, record.get(name)?)))
                    .collect();
                self.dictionary(&entries, indent, multiline)?
            }
            other => {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "an ASCII property list cannot hold a value of type {}; pass text, a number, bytes, a list or a map",
                        other.type_name()
                    ),
                ));
            }
        };
        Ok(text)
    }

    fn array(&self, items: &[Value], indent: &str, multiline: bool) -> Result<String, Error> {
        if items.is_empty() {
            return Ok("()".to_owned());
        }
        let inner = format!("{indent}{}", self.unit);
        let rendered: Vec<String> = items
            .iter()
            .map(|item| self.render(item, Form::default(), &inner, multiline))
            .collect::<Result<_, _>>()?;
        if multiline {
            let eol = self.eol;
            let body: String = rendered
                .iter()
                .map(|item| format!("{eol}{inner}{item},"))
                .collect();
            Ok(format!("({body}{eol}{indent})"))
        } else {
            Ok(format!("({})", rendered.join(", ")))
        }
    }

    fn dictionary(
        &self,
        entries: &[(&str, &Value)],
        indent: &str,
        multiline: bool,
    ) -> Result<String, Error> {
        if entries.is_empty() {
            return Ok("{}".to_owned());
        }
        let inner = format!("{indent}{}", self.unit);
        let mut out = String::from("{");
        for (key, item) in entries {
            let rendered = self.render(item, Form::default(), &inner, multiline)?;
            if multiline {
                let _ = write!(
                    out,
                    "{}{inner}{} = {rendered};",
                    self.eol,
                    string(key, Form::default())
                );
            } else {
                let _ = write!(out, " {} = {rendered};", string(key, Form::default()));
            }
        }
        if multiline {
            let _ = write!(out, "{}{indent}}}", self.eol);
        } else {
            out.push_str(" }");
        }
        Ok(out)
    }

    /// Value the document holds after `value` is written over `node`, or as
    /// a new member when `node` is `None`: numbers and truth values become
    /// strings unless the replaced value is typed.
    ///
    /// # Errors
    ///
    /// Returns `E_TYPE` for a value the form cannot hold.
    pub(crate) fn stored(
        &self,
        text: &str,
        tree: &DocTree,
        node: Option<NodeId>,
        value: &Value,
    ) -> Result<Value, Error> {
        let form = node.map_or_else(Form::default, |node| {
            let span = tree[node].value_span;
            Form::of(&text[span.start..span.end])
        });
        let rendered = self.render(value, form, "", false)?;
        let wrapped = parse::parse(format!("({rendered})").as_bytes())?;
        match wrapped.to_value() {
            Value::List(mut items) if items.len() == 1 => Ok(items.remove(0)),
            other => Ok(other),
        }
    }

    /// Text inserted after the last member of a container, or inside an
    /// empty one; `member` is the new member without its separator.
    fn add_member(
        &self,
        text: &str,
        tree: &DocTree,
        container: NodeId,
        member: &dyn Fn(&str) -> Result<String, Error>,
        trailing: &str,
    ) -> Result<PlistEdit, Error> {
        let node = &tree[container];
        let span = node.value_span;
        if let Some(&last) = node.children.last() {
            return self.after_last(text, tree, last, member, trailing);
        }
        let braced = text[span.start..span.end].starts_with(['{', '(']);
        if !braced {
            let prefix = if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                self.eol
            };
            let new = format!("{prefix}{}{trailing}{}", member("")?, self.eol);
            return Ok(edit(text.len()..text.len(), &new));
        }
        let interior = span.start + 1..span.end - 1;
        let parent_indent = indentation(text, span.start);
        let new = if text[interior.clone()].contains('\n') {
            let inner = format!("{parent_indent}{}", self.unit);
            format!(
                "{}{inner}{}{trailing}{}{parent_indent}",
                self.eol,
                member(&inner)?,
                self.eol
            )
        } else if trailing.is_empty() {
            member(parent_indent)?
        } else {
            format!(" {}{trailing} ", member(parent_indent)?)
        };
        Ok(edit(interior, &new))
    }

    fn after_last(
        &self,
        text: &str,
        tree: &DocTree,
        last: NodeId,
        member: &dyn Fn(&str) -> Result<String, Error>,
        trailing: &str,
    ) -> Result<PlistEdit, Error> {
        let node = &tree[last];
        let own_line = starts_line(text, node.span.start);
        let indent = indentation(text, node.span.start);
        if !trailing.is_empty() {
            let new = if own_line {
                format!("{}{indent}{}{trailing}", self.eol, member(indent)?)
            } else {
                format!(" {}{trailing}", member(indent)?)
            };
            return Ok(edit(node.span.end..node.span.end, &new));
        }
        let element = member(indent)?;
        if text[node.span.start..node.span.end].ends_with(',') {
            let new = if own_line {
                format!("{}{indent}{element},", self.eol)
            } else {
                format!(" {element},")
            };
            return Ok(edit(node.span.end..node.span.end, &new));
        }
        let new = if own_line {
            format!(",{}{indent}{element}", self.eol)
        } else {
            format!(", {element}")
        };
        Ok(edit(node.value_span.end..node.value_span.end, &new))
    }
}

impl TextSyntax for AsciiSyntax {
    fn replace(
        &self,
        text: &str,
        tree: &DocTree,
        node: NodeId,
        value: &Value,
    ) -> Result<Option<PlistEdit>, Error> {
        if tree.value_of(node, &SEMANTICS, None) == *value {
            return Ok(None);
        }
        let span = tree[node].value_span;
        let old = &text[span.start..span.end];
        let new = self.render(
            value,
            Form::of(old),
            indentation(text, span.start),
            old.contains('\n'),
        )?;
        Ok((new != old).then(|| edit(span.start..span.end, &new)))
    }

    fn insert(
        &self,
        text: &str,
        tree: &DocTree,
        map: NodeId,
        name: &str,
        value: &Value,
    ) -> Result<PlistEdit, Error> {
        let separator = tree[map]
            .children
            .last()
            .and_then(|last| {
                let node = &tree[*last];
                node.key_span
                    .map(|key| &text[key.end..node.value_span.start])
            })
            .unwrap_or(DEFAULT_SEPARATOR);
        let key = string(name, Form::default());
        let member = |indent: &str| -> Result<String, Error> {
            let rendered = self.render(value, Form::default(), indent, false)?;
            Ok(format!("{key}{separator}{rendered}"))
        };
        self.add_member(text, tree, map, &member, ";")
    }

    fn append(
        &self,
        text: &str,
        tree: &DocTree,
        list: NodeId,
        value: &Value,
    ) -> Result<PlistEdit, Error> {
        let member = |indent: &str| self.render(value, Form::default(), indent, false);
        self.add_member(text, tree, list, &member, "")
    }

    fn removal(&self, text: &str, tree: &DocTree, node: NodeId) -> Range<usize> {
        let span = tree[node].span;
        let next = next_sibling(tree, node).map(|next| tree[next].span.start);
        let mut range = removal(text, &(span.start..span.end), next);
        let in_array = tree[node]
            .parent
            .is_some_and(|parent| tree[parent].kind == NodeKind::List);
        let trailing_comma = text[span.start..span.end].ends_with(',');
        if in_array
            && next.is_none()
            && !trailing_comma
            && !starts_line(text, span.start)
            && let Some(previous) = previous_sibling(tree, node)
        {
            let comma = tree[previous].span.end - 1;
            range.start = range.start.min(comma);
        }
        range
    }
}

fn edit(range: Range<usize>, text: &str) -> PlistEdit {
    PlistEdit {
        range,
        replacement: text.as_bytes().to_vec(),
    }
}

/// String in the given form: quoted with the form's quote character, or
/// unquoted when the text allows it, or quoted with `"`.
fn string(text: &str, form: Form) -> String {
    match form.quote {
        Some(quote) => quoted(text, quote),
        None if unquoted_allowed(text) => text.to_owned(),
        None => quoted(text, '"'),
    }
}

fn typed_or_string(form: Form, letter: u8, text: &str) -> String {
    match form.typed {
        Some(typed) if typed == letter || (letter == b'I' && typed == b'R') => {
            format!("<*{}{text}>", char::from(typed))
        }
        _ => string(text, form),
    }
}

fn unquoted_allowed(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(is_unquoted)
        && !text.starts_with("//")
        && !text.starts_with("/*")
}

fn quoted(text: &str, quote: char) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            character if character == quote => {
                out.push('\\');
                out.push(character);
            }
            character if character.is_control() => {
                let _ = write!(out, "\\U{:04x}", u32::from(character));
            }
            character => out.push(character),
        }
    }
    out.push(quote);
    out
}

fn data(bytes: &[u8], grouped: bool) -> String {
    let mut out = String::from("<");
    for (index, byte) in bytes.iter().enumerate() {
        if grouped && index > 0 && index % DATA_GROUP_BYTES == 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out.push('>');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::keypath::KeyPath;
    use nurt_contracts::value::{Bytes, Fields};

    use crate::formats::plist::plan::{Op, plan_remove, plan_set, text_remove, text_set};

    fn set(text: &str, key: &str, value: &Value, create: bool) -> String {
        let (tree, syntax) = build(text, 0).unwrap();
        let key = KeyPath::parse(key).unwrap();
        let ops = plan_set(&tree, &key, value, create).unwrap();
        let edits = text_set(&syntax, text, &tree, &key, &ops).unwrap();
        apply(text, &edits)
    }

    fn remove(text: &str, key: &str) -> String {
        let (tree, syntax) = build(text, 0).unwrap();
        let key = KeyPath::parse(key).unwrap();
        let nodes = plan_remove(&tree, &key).unwrap();
        apply(
            text,
            &text_remove(&syntax, text, &tree, &key, &nodes).unwrap(),
        )
    }

    fn apply(text: &str, edits: &[PlistEdit]) -> String {
        let mut out = text.as_bytes().to_vec();
        for edit in edits.iter().rev() {
            out.splice(edit.range.clone(), edit.replacement.iter().copied());
        }
        String::from_utf8(out).unwrap()
    }

    fn text(value: &str) -> Value {
        Value::Text(value.to_owned())
    }

    #[test]
    fn replacing_scalars_keeps_quoting_and_typed_forms() {
        let source =
            "{ a = \"x\"; b = 'y'; c = z; i = <*I1>; r = <*R1.5>; f = <*BN>; d = <00ff 0102 03>; }";
        assert_eq!(
            set(source, "a", &text("p q"), false),
            source.replace("\"x\"", "\"p q\"")
        );
        assert_eq!(
            set(source, "b", &text("it's"), false),
            source.replace("'y'", "'it\\'s'")
        );
        assert_eq!(
            set(source, "c", &text("w"), false),
            source.replace("c = z", "c = w")
        );
        assert_eq!(
            set(source, "c", &text("two words"), false),
            source.replace("c = z", "c = \"two words\"")
        );
        assert_eq!(
            set(source, "i", &Value::Int(7), false),
            source.replace("<*I1>", "<*I7>")
        );
        assert_eq!(
            set(source, "r", &Value::Int(2), false),
            source.replace("<*R1.5>", "<*R2>")
        );
        assert_eq!(
            set(source, "f", &Value::Bool(true), false),
            source.replace("<*BN>", "<*BY>")
        );
        let bytes = Value::Bytes(Bytes::new((1..=6).collect()));
        assert_eq!(
            set(source, "d", &bytes, false),
            source.replace("<00ff 0102 03>", "<01020304 0506>")
        );
        assert_eq!(set(source, "a", &text("x"), false), source);
    }

    #[test]
    fn numbers_and_truth_values_become_strings() {
        let source = "{ n = old; }";
        assert_eq!(set(source, "n", &Value::Int(12), false), "{ n = 12; }");
        assert_eq!(set(source, "n", &Value::Bool(false), false), "{ n = NO; }");
        assert_eq!(set(source, "n", &Value::Number(0.5), false), "{ n = 0.5; }");
        let error = plan_set(
            &build(source, 0).unwrap().0,
            &KeyPath::parse("n").unwrap(),
            &Value::Null,
            false,
        )
        .and_then(|ops| {
            let (tree, syntax) = build(source, 0)?;
            text_set(&syntax, source, &tree, &KeyPath::parse("n")?, &ops)
        })
        .unwrap_err();
        assert_eq!(error.code().as_str(), "E_TYPE");
    }

    #[test]
    fn new_entries_follow_sibling_layout() {
        let lines = "{\n    a = 1;\n    b = (x);\n}\n";
        assert_eq!(
            set(lines, "c", &text("v"), true),
            "{\n    a = 1;\n    b = (x);\n    c = v;\n}\n"
        );
        let compact = "{a=1;}";
        assert_eq!(set(compact, "b", &text("2"), true), "{a=1; b=2;}");
        assert_eq!(set("{}", "k", &text("v"), true), "{ k = v; }");
        assert_eq!(set("{\n}", "k", &text("v"), true), "{\n\tk = v;\n}");
        assert_eq!(set("a = 1;\n", "b", &text("2"), true), "a = 1;\nb = 2;\n");
        assert_eq!(set("", "b", &text("2"), true), "b = 2;\n");
    }

    #[test]
    fn appending_follows_comma_style() {
        assert_eq!(set("(a, b)", "[+]", &text("c"), false), "(a, b, c)");
        assert_eq!(set("(a, b,)", "[2]", &text("c"), false), "(a, b, c,)");
        assert_eq!(
            set("(\n  a,\n  b,\n)", "[+]", &text("c"), false),
            "(\n  a,\n  b,\n  c,\n)"
        );
        assert_eq!(
            set("(\n  a\n)", "[+]", &text("c"), false),
            "(\n  a,\n  c\n)"
        );
        assert_eq!(set("()", "[+]", &text("c"), false), "(c)");
    }

    #[test]
    fn nested_values_and_created_paths() {
        let mut fields = Fields::new();
        fields.insert(
            "k".into(),
            Value::List(vec![text("a"), Value::Bytes(Bytes::new(vec![1]))]),
        );
        assert_eq!(
            set("{ }", "m", &Value::Map(fields), true),
            "{ m = { k = (a, <01>); }; }"
        );
        assert_eq!(
            set("{ a = 1; }", "x.y[+]", &text("z"), true),
            "{ a = 1; x = { y = (z); }; }"
        );
        let (tree, _) = build("{ a = 1; }", 0).unwrap();
        let error =
            plan_set(&tree, &KeyPath::parse("x.y").unwrap(), &text("z"), false).unwrap_err();
        assert_eq!(error.code().as_str(), "E_KEY_NOT_FOUND");
        let error = plan_set(&tree, &KeyPath::parse("a.b").unwrap(), &text("z"), true).unwrap_err();
        assert_eq!(error.code().as_str(), "E_KEY_CONFLICT");
        assert!(matches!(
            plan_set(&tree, &KeyPath::parse("a").unwrap(), &text("2"), false).unwrap()[0],
            Op::Replace { .. }
        ));
    }

    #[test]
    fn removal_takes_separators_and_lines() {
        assert_eq!(remove("{ a = 1; b = 2; }", "a"), "{ b = 2; }");
        assert_eq!(remove("{ a = 1; b = 2; }", "b"), "{ a = 1; }");
        assert_eq!(remove("{\n\ta = 1;\n\tb = 2;\n}", "a"), "{\n\tb = 2;\n}");
        assert_eq!(remove("(a, b, c)", "[1]"), "(a, c)");
        assert_eq!(remove("(a, b, c)", "[2]"), "(a, b)");
        assert_eq!(remove("(\n  a,\n  b\n)", "[1]"), "(\n  a,\n)");
        assert_eq!(remove("{ a = (x, y); }", "a[*]"), "{ a = (); }");
        let (tree, _) = build("{ a = 1; }", 0).unwrap();
        assert_eq!(
            plan_remove(&tree, &KeyPath::root())
                .unwrap_err()
                .code()
                .as_str(),
            "E_KEY_CONFLICT"
        );
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(quoted("a\"b\\c\nd\u{1}", '"'), "\"a\\\"b\\\\c\\nd\\U0001\"");
        assert!(!unquoted_allowed("//x"));
        assert!(unquoted_allowed("com.example.app"));
        assert!(!unquoted_allowed(""));
    }

    #[test]
    fn multiline_containers_are_indented() {
        let source = "{\n\tlist = (\n\t\ta\n\t);\n}";
        let value = Value::List(vec![text("b"), text("c d")]);
        assert_eq!(
            set(source, "list", &value, false),
            "{\n\tlist = (\n\t\tb,\n\t\t\"c d\",\n\t);\n}"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
