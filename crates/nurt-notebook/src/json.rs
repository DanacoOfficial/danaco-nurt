// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! JSON objects that keep the exact text of every part not changed.
//!
//! A parsed object remembers the byte span of each member in the original
//! document. Rendering copies unchanged members verbatim and serializes only
//! changed values, in the layout detected from the document: indentation,
//! line ending, key separator and escaping of non-ASCII characters. The
//! layout matches the output of Python `json.dumps`, which Jupyter uses.

use std::ops::Range;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::error::NotebookError;

/// Byte order mark of UTF-8, kept verbatim before the root object.
const BYTE_ORDER_MARK: &str = "\u{feff}";

/// Whitespace used by Python `json.dumps` after `,` in compact output.
const COMPACT_ITEM_SPACE: &str = " ";

/// Key separator of Python `json.dumps` and of Jupyter.
const DEFAULT_COLON: &str = ": ";

/// First code point escaped by `json.dumps(ensure_ascii=True)`.
const FIRST_NON_ASCII: u32 = 0x80;

/// Layout of a JSON document, detected from its root object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Indentation unit; `None` for single-line documents.
    indent: Option<String>,
    /// Line ending of indented documents.
    newline: String,
    /// Text between a key and its value, including `:`.
    colon: String,
    /// Whitespace after `,` in single-line documents.
    item_space: String,
    /// Whether characters outside ASCII are written as `\u` escapes.
    ascii_only: bool,
}

impl Default for Layout {
    /// Layout written by Jupyter: one space of indentation, `\n`, `": "`.
    fn default() -> Self {
        Self {
            indent: Some(" ".to_owned()),
            newline: "\n".to_owned(),
            colon: DEFAULT_COLON.to_owned(),
            item_space: COMPACT_ITEM_SPACE.to_owned(),
            ascii_only: false,
        }
    }
}

impl Layout {
    /// Line break followed by the indentation of `depth` levels; empty in
    /// single-line documents.
    fn break_at(&self, depth: usize) -> String {
        match &self.indent {
            Some(unit) => format!("{}{}", self.newline, unit.repeat(depth)),
            None => String::new(),
        }
    }

    /// Separator between items whose indentation is `depth` levels.
    fn separator(&self, depth: usize) -> String {
        match &self.indent {
            Some(_) => format!(",{}", self.break_at(depth)),
            None => format!(",{}", self.item_space),
        }
    }

    /// Serializes a value nested `depth` levels deep.
    pub(crate) fn write_value(&self, value: &Value, depth: usize) -> String {
        let mut out = String::new();
        self.push_value(&mut out, value, depth);
        out
    }

    fn push_value(&self, out: &mut String, value: &Value, depth: usize) {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
            Value::Number(number) => match number.as_f64().filter(|_| number.is_f64()) {
                Some(float) => out.push_str(&python_float(float)),
                None => out.push_str(&number.to_string()),
            },
            Value::String(text) => self.push_string(out, text),
            Value::Array(items) if items.is_empty() => out.push_str("[]"),
            Value::Array(items) => {
                out.push('[');
                out.push_str(&self.break_at(depth + 1));
                for (position, item) in items.iter().enumerate() {
                    if position > 0 {
                        out.push_str(&self.separator(depth + 1));
                    }
                    self.push_value(out, item, depth + 1);
                }
                out.push_str(&self.break_at(depth));
                out.push(']');
            }
            Value::Object(map) if map.is_empty() => out.push_str("{}"),
            Value::Object(map) => {
                out.push('{');
                out.push_str(&self.break_at(depth + 1));
                for (position, (key, item)) in map.iter().enumerate() {
                    if position > 0 {
                        out.push_str(&self.separator(depth + 1));
                    }
                    self.push_string(out, key);
                    out.push_str(&self.colon);
                    self.push_value(out, item, depth + 1);
                }
                out.push_str(&self.break_at(depth));
                out.push('}');
            }
        }
    }

    /// Writes a JSON string with the escapes of Python `json.dumps`: `"`,
    /// `\`, the short escapes `\b \f \n \r \t`, other control characters as
    /// lowercase `\u00XX`, and characters outside ASCII as `\u` escapes with
    /// surrogate pairs when the document is ASCII only.
    fn push_string(&self, out: &mut String, text: &str) {
        out.push('"');
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if c < ' ' => push_unicode_escape(out, u32::from(c)),
                c if self.ascii_only && u32::from(c) >= FIRST_NON_ASCII => {
                    let mut units = [0_u16; 2];
                    for unit in c.encode_utf16(&mut units) {
                        push_unicode_escape(out, u32::from(*unit));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }
}

fn push_unicode_escape(out: &mut String, unit: u32) {
    out.push_str(&format!("\\u{unit:04x}"));
}

/// Largest decimal point position written without an exponent by Python
/// `repr(float)`; `1e16` is the first value written as `1e+16`.
const PYTHON_FIXED_MAX_POINT: i32 = 16;

/// Smallest decimal point position written without an exponent; values below
/// `1e-4` are written with an exponent, as in `1e-05`.
const PYTHON_FIXED_MIN_POINT: i32 = -3;

/// Writes a finite float as Python `repr(float)` does: the shortest digits
/// that read back, an exponent with a sign and at least two digits outside the
/// fixed range, and a `.0` after a whole number.
fn python_float(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_owned();
    }
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .unwrap_or((scientific.as_str(), "0"));
    let exponent: i32 = exponent.parse().unwrap_or_default();
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exponent + 1;
    let sign = if value < 0.0 { "-" } else { "" };
    let width = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    if !(PYTHON_FIXED_MIN_POINT..=PYTHON_FIXED_MAX_POINT).contains(&point) {
        let (head, tail) = digits.split_at(1);
        let fraction = if tail.is_empty() {
            String::new()
        } else {
            format!(".{tail}")
        };
        let exponent_sign = if exponent < 0 { '-' } else { '+' };
        return format!(
            "{sign}{head}{fraction}e{exponent_sign}{:02}",
            exponent.abs()
        );
    }
    let zeros = |count: i32| "0".repeat(usize::try_from(count).unwrap_or_default());
    if point <= 0 {
        format!("{sign}0.{}{digits}", zeros(-point))
    } else if point >= width {
        format!("{sign}{digits}{}.0", zeros(point - width))
    } else {
        let (whole, fraction) = digits.split_at(usize::try_from(point).unwrap_or_default());
        format!("{sign}{whole}.{fraction}")
    }
}

/// Whitespace around the items of a container, taken from the original text
/// or derived from the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Gaps {
    open: String,
    separator: String,
    close: String,
}

impl Gaps {
    /// Gaps of a container nested `depth` levels deep, written anew.
    pub(crate) fn from_layout(layout: &Layout, depth: usize) -> Self {
        Self {
            open: layout.break_at(depth + 1),
            separator: layout.separator(depth + 1),
            close: layout.break_at(depth),
        }
    }

    /// Gaps of an original container with the given item spans; the
    /// container text starts with its bracket at `start` and ends with its
    /// bracket before `end`.
    fn from_text(
        text: &str,
        container: Range<usize>,
        items: &[Range<usize>],
        layout: &Layout,
        depth: usize,
    ) -> Self {
        let derived = Self::from_layout(layout, depth);
        let (Some(first), Some(last)) = (items.first(), items.last()) else {
            return derived;
        };
        let open = text[container.start + 1..first.start].to_owned();
        let close = text[last.end..container.end - 1].to_owned();
        let separator = match items.get(1) {
            Some(second) => text[first.end..second.start].to_owned(),
            None if open.is_empty() => derived.separator,
            None => format!(",{open}"),
        };
        Self {
            open,
            separator,
            close,
        }
    }

    /// Joins item texts into a container between `brackets`.
    pub(crate) fn join(&self, brackets: [char; 2], items: &[String]) -> String {
        let mut out = String::new();
        out.push(brackets[0]);
        if !items.is_empty() {
            out.push_str(&self.open);
            for (position, item) in items.iter().enumerate() {
                if position > 0 {
                    out.push_str(&self.separator);
                }
                out.push_str(item);
            }
            out.push_str(&self.close);
        }
        out.push(brackets[1]);
        out
    }
}

/// Spans of one member of an original object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemberSpan {
    /// Start of the key string.
    key_start: usize,
    /// Start of the value.
    value_start: usize,
    /// End of the value.
    value_end: usize,
}

#[derive(Debug, Clone)]
struct Member {
    key: String,
    value: Value,
    span: Option<MemberSpan>,
    /// The value differs from the original text and is serialized anew.
    changed: bool,
}

/// Text of the document an original object was parsed from.
#[derive(Debug, Clone)]
struct Origin {
    text: Arc<str>,
    span: Range<usize>,
    gaps: Gaps,
}

/// JSON object that renders unchanged members exactly as in the original.
#[derive(Debug, Clone)]
pub(crate) struct JsonObject {
    members: Vec<Member>,
    origin: Option<Origin>,
    /// Members were added or removed since parsing.
    reshaped: bool,
}

impl JsonObject {
    /// New object with the members of `map`, without original text.
    pub(crate) fn from_map(map: Map<String, Value>) -> Self {
        Self {
            members: map
                .into_iter()
                .map(|(key, value)| Member {
                    key,
                    value,
                    span: None,
                    changed: true,
                })
                .collect(),
            origin: None,
            reshaped: true,
        }
    }

    /// Parses the object starting at byte `start` of `text`; `depth` is the
    /// nesting level of the object. Values of the keys in `unparsed` are
    /// kept only as text and read as `null`. Returns the object and the end
    /// of its text.
    pub(crate) fn parse(
        text: &Arc<str>,
        start: usize,
        layout: &Layout,
        depth: usize,
        unparsed: &[&str],
    ) -> Result<(Self, usize), NotebookError> {
        let scanned = scan_object(text, start)?;
        let mut members = Vec::with_capacity(scanned.members.len());
        let mut item_spans = Vec::with_capacity(scanned.members.len());
        for raw in scanned.members {
            let value = if unparsed.contains(&raw.key.as_str()) {
                Value::Null
            } else {
                serde_json::from_str(&text[raw.value_start..raw.value_end])
                    .map_err(|error| malformed(&error.to_string()))?
            };
            item_spans.push(raw.key_start..raw.value_end);
            members.push(Member {
                key: raw.key,
                value,
                span: Some(MemberSpan {
                    key_start: raw.key_start,
                    value_start: raw.value_start,
                    value_end: raw.value_end,
                }),
                changed: false,
            });
        }
        let span = start..scanned.end;
        let gaps = Gaps::from_text(text, span.clone(), &item_spans, layout, depth);
        Ok((
            Self {
                members,
                origin: Some(Origin {
                    text: Arc::clone(text),
                    span,
                    gaps,
                }),
                reshaped: false,
            },
            scanned.end,
        ))
    }

    /// Value of the member `key`; with repeated keys, the last one.
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.members
            .iter()
            .rev()
            .find(|member| member.key == key)
            .map(|member| &member.value)
    }

    /// Keys in document order.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|member| member.key.as_str())
    }

    /// Whether rendering differs from the original text.
    pub(crate) fn is_changed(&self) -> bool {
        self.origin.is_none() || self.reshaped || self.members.iter().any(|m| m.changed)
    }

    /// Sets the member `key`. An equal value leaves the object untouched; a
    /// new key is inserted in sorted position when the existing keys are
    /// sorted, otherwise appended. Returns whether the object changed.
    pub(crate) fn set(&mut self, key: &str, value: Value) -> bool {
        if let Some(member) = self.members.iter_mut().rev().find(|m| m.key == key) {
            if member.value == value {
                return false;
            }
            member.value = value;
            member.changed = true;
            return true;
        }
        let member = Member {
            key: key.to_owned(),
            value,
            span: None,
            changed: true,
        };
        let sorted = self
            .members
            .windows(2)
            .all(|pair| pair[0].key <= pair[1].key);
        let position = if sorted {
            self.members.partition_point(|m| m.key.as_str() < key)
        } else {
            self.members.len()
        };
        self.members.insert(position, member);
        self.reshaped = true;
        true
    }

    /// Removes every member `key`. Returns whether the object changed.
    pub(crate) fn remove(&mut self, key: &str) -> bool {
        let before = self.members.len();
        self.members.retain(|member| member.key != key);
        let removed = self.members.len() != before;
        self.reshaped |= removed;
        removed
    }

    /// Renders the object nested `depth` levels deep. Members listed in
    /// `replaced` are written with the given text instead of their value.
    pub(crate) fn render(
        &self,
        layout: &Layout,
        depth: usize,
        replaced: &[(&str, &str)],
    ) -> String {
        let Some(origin) = &self.origin else {
            return self.rebuild(layout, depth, None, replaced);
        };
        if self.reshaped {
            return self.rebuild(layout, depth, Some(origin), replaced);
        }
        let text = &origin.text;
        let mut out = String::with_capacity(origin.span.len());
        let mut cursor = origin.span.start;
        for member in &self.members {
            let new_text = match replacement(replaced, &member.key) {
                Some(raw) => Some(raw.to_owned()),
                None if member.changed => Some(layout.write_value(&member.value, depth + 1)),
                None => None,
            };
            if let (Some(new_text), Some(span)) = (new_text, member.span) {
                out.push_str(&text[cursor..span.value_start]);
                out.push_str(&new_text);
                cursor = span.value_end;
            }
        }
        out.push_str(&text[cursor..origin.span.end]);
        out
    }

    fn rebuild(
        &self,
        layout: &Layout,
        depth: usize,
        origin: Option<&Origin>,
        replaced: &[(&str, &str)],
    ) -> String {
        let gaps = origin.map_or_else(|| Gaps::from_layout(layout, depth), |o| o.gaps.clone());
        let items: Vec<String> = self
            .members
            .iter()
            .map(|member| {
                let value = match replacement(replaced, &member.key) {
                    Some(raw) => raw.to_owned(),
                    None => match (origin, member.span, member.changed) {
                        (Some(o), Some(span), false) => {
                            o.text[span.value_start..span.value_end].to_owned()
                        }
                        _ => layout.write_value(&member.value, depth + 1),
                    },
                };
                let key = match (origin, member.span) {
                    (Some(o), Some(span)) => o.text[span.key_start..span.value_start].to_owned(),
                    _ => {
                        let mut key = String::new();
                        layout.push_string(&mut key, &member.key);
                        key.push_str(&layout.colon);
                        key
                    }
                };
                key + &value
            })
            .collect();
        gaps.join(['{', '}'], &items)
    }
}

/// Raw text given for the member `key` in `replaced`.
fn replacement<'a>(replaced: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    replaced
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, text)| *text)
}

/// JSON array whose items are objects, as the `cells` array of a notebook.
#[derive(Debug, Clone)]
pub(crate) struct ObjectArray {
    /// Span of the original array and its gaps.
    origin: Option<(Range<usize>, Gaps)>,
}

impl ObjectArray {
    /// Parses the array of objects at byte `start` of `text`, nested
    /// `depth` levels deep; `what` names an item in error messages.
    pub(crate) fn parse(
        text: &Arc<str>,
        start: usize,
        layout: &Layout,
        depth: usize,
        what: &str,
    ) -> Result<(Self, Vec<JsonObject>), NotebookError> {
        let mut items = Vec::new();
        let mut spans = Vec::new();
        let mut cursor = skip_ws(text, start + 1);
        if byte_at(text, cursor) == Some(b']') {
            return Ok((
                Self {
                    origin: Some((
                        start..cursor + 1,
                        Gaps::from_text(text, start..cursor + 1, &[], layout, depth),
                    )),
                },
                items,
            ));
        }
        loop {
            if byte_at(text, cursor) != Some(b'{') {
                return Err(NotebookError::structure(format!(
                    "{what} {} is not an object",
                    items.len()
                )));
            }
            let (object, end) = JsonObject::parse(text, cursor, layout, depth + 1, &[])?;
            spans.push(cursor..end);
            items.push(object);
            cursor = skip_ws(text, end);
            match byte_at(text, cursor) {
                Some(b',') => cursor = skip_ws(text, cursor + 1),
                Some(b']') => break,
                _ => return Err(malformed("expected `,` or `]`")),
            }
        }
        let span = start..cursor + 1;
        let gaps = Gaps::from_text(text, span.clone(), &spans, layout, depth);
        Ok((
            Self {
                origin: Some((span, gaps)),
            },
            items,
        ))
    }

    /// Renders `items`, nested `depth` levels deep, with the gaps of the
    /// original array.
    pub(crate) fn render(&self, items: &[JsonObject], layout: &Layout, depth: usize) -> String {
        let gaps = self.origin.as_ref().map_or_else(
            || Gaps::from_layout(layout, depth),
            |(_, gaps)| gaps.clone(),
        );
        let texts: Vec<String> = items
            .iter()
            .map(|item| item.render(layout, depth + 1, &[]))
            .collect();
        gaps.join(['[', ']'], &texts)
    }
}

/// Root object of a document with the text around it.
#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub(crate) text: Arc<str>,
    pub(crate) layout: Layout,
    pub(crate) root: JsonObject,
    /// Span of the root object.
    root_span: Range<usize>,
}

impl Document {
    /// Parses a document whose root is an object; values of the root keys
    /// in `unparsed` are kept only as text.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::NotUtf8`] or [`NotebookError::InvalidJson`]
    /// for undecodable or malformed text, and [`NotebookError::Structure`]
    /// when the root is not an object.
    pub(crate) fn parse(bytes: &[u8], unparsed: &[&str]) -> Result<Self, NotebookError> {
        let text = std::str::from_utf8(bytes).map_err(|error| NotebookError::NotUtf8 {
            offset: error.valid_up_to(),
        })?;
        let body_start = if text.starts_with(BYTE_ORDER_MARK) {
            BYTE_ORDER_MARK.len()
        } else {
            0
        };
        serde_json::from_str::<serde::de::IgnoredAny>(&text[body_start..]).map_err(|error| {
            NotebookError::InvalidJson {
                line: error.line(),
                column: error.column(),
                detail: syntax_detail(&error),
            }
        })?;
        let text: Arc<str> = Arc::from(text);
        let root_start = skip_ws(&text, body_start);
        if byte_at(&text, root_start) != Some(b'{') {
            return Err(NotebookError::structure("the root is not a JSON object"));
        }
        let layout = detect_layout(&text, root_start)?;
        let (root, root_end) = JsonObject::parse(&text, root_start, &layout, 0, unparsed)?;
        Ok(Self {
            text,
            layout,
            root,
            root_span: root_start..root_end,
        })
    }

    /// Start of the original value of the root member `key`.
    pub(crate) fn member_value_start(&self, key: &str) -> Option<usize> {
        self.root
            .members
            .iter()
            .rev()
            .find(|member| member.key == key)
            .and_then(|member| member.span)
            .map(|span| span.value_start)
    }

    /// Renders the document; `replaced` gives raw text for root members.
    pub(crate) fn render(&self, replaced: &[(&str, &str)]) -> String {
        let mut out = String::with_capacity(self.text.len());
        out.push_str(&self.text[..self.root_span.start]);
        out.push_str(&self.root.render(&self.layout, 0, replaced));
        out.push_str(&self.text[self.root_span.end..]);
        out
    }
}

fn syntax_detail(error: &serde_json::Error) -> String {
    let text = error.to_string();
    match text.find(" at line ") {
        Some(position) => text[..position].to_owned(),
        None => text,
    }
}

/// Detects the layout from the first member of the root object at `start`.
fn detect_layout(text: &str, start: usize) -> Result<Layout, NotebookError> {
    let mut layout = Layout {
        ascii_only: detect_ascii_only(text),
        ..Layout::default()
    };
    let key_start = skip_ws(text, start + 1);
    if byte_at(text, key_start) != Some(b'"') {
        return Ok(layout);
    }
    let open = &text[start + 1..key_start];
    let key_end = skip_string(text, key_start)?;
    let colon_end = skip_ws(text, skip_ws(text, key_end) + 1);
    layout.colon = text[key_end..colon_end].to_owned();
    match open.rfind('\n') {
        Some(position) => {
            layout.newline = if open[..position].ends_with('\r') {
                "\r\n"
            } else {
                "\n"
            }
            .to_owned();
            layout.indent = Some(open[position + 1..].to_owned());
        }
        None => {
            layout.indent = None;
            let value_end = skip_value(text, colon_end)?;
            let comma = skip_ws(text, value_end);
            if byte_at(text, comma) == Some(b',') {
                let next = skip_ws(text, comma + 1);
                layout.item_space = text[comma + 1..next].to_owned();
            }
        }
    }
    Ok(layout)
}

/// Whether the document writes characters outside ASCII only as escapes:
/// it has no raw non-ASCII byte and at least one such escape.
fn detect_ascii_only(text: &str) -> bool {
    if !text.is_ascii() {
        return false;
    }
    let bytes = text.as_bytes();
    let mut position = 0;
    while let Some(offset) = text[position..].find("\\u") {
        let escape = position + offset;
        let preceding = bytes[..escape]
            .iter()
            .rev()
            .take_while(|b| **b == b'\\')
            .count();
        let digits = text.get(escape + 2..escape + 6);
        if preceding % 2 == 0
            && let Some(code) = digits.and_then(|d| u32::from_str_radix(d, 16).ok())
            && code >= FIRST_NON_ASCII
        {
            return true;
        }
        position = escape + 2;
    }
    false
}

fn malformed(detail: &str) -> NotebookError {
    NotebookError::InvalidJson {
        line: 0,
        column: 0,
        detail: detail.to_owned(),
    }
}

fn byte_at(text: &str, position: usize) -> Option<u8> {
    text.as_bytes().get(position).copied()
}

fn skip_ws(text: &str, mut position: usize) -> usize {
    while matches!(byte_at(text, position), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        position += 1;
    }
    position
}

/// End of the string starting with `"` at `start`.
fn skip_string(text: &str, start: usize) -> Result<usize, NotebookError> {
    let bytes = text.as_bytes();
    let mut position = start + 1;
    while let Some(byte) = bytes.get(position) {
        match byte {
            b'\\' => position += 2,
            b'"' => return Ok(position + 1),
            _ => position += 1,
        }
    }
    Err(malformed("unterminated string"))
}

/// End of the value starting at `start`.
fn skip_value(text: &str, start: usize) -> Result<usize, NotebookError> {
    match byte_at(text, start) {
        Some(b'"') => skip_string(text, start),
        Some(b'{' | b'[') => {
            let mut depth = 0_usize;
            let mut position = start;
            while let Some(byte) = byte_at(text, position) {
                match byte {
                    b'"' => {
                        position = skip_string(text, position)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(position + 1);
                        }
                    }
                    _ => {}
                }
                position += 1;
            }
            Err(malformed("unterminated container"))
        }
        Some(_) => {
            let mut position = start;
            while let Some(byte) = byte_at(text, position) {
                if matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                    break;
                }
                position += 1;
            }
            Ok(position)
        }
        None => Err(malformed("unexpected end of document")),
    }
}

struct RawMember {
    key: String,
    key_start: usize,
    value_start: usize,
    value_end: usize,
}

struct ScannedObject {
    members: Vec<RawMember>,
    end: usize,
}

/// Member spans of the object starting with `{` at `start`.
fn scan_object(text: &str, start: usize) -> Result<ScannedObject, NotebookError> {
    let mut members = Vec::new();
    let mut cursor = skip_ws(text, start + 1);
    if byte_at(text, cursor) == Some(b'}') {
        return Ok(ScannedObject {
            members,
            end: cursor + 1,
        });
    }
    loop {
        if byte_at(text, cursor) != Some(b'"') {
            return Err(malformed("expected a key"));
        }
        let key_start = cursor;
        let key_end = skip_string(text, key_start)?;
        let key: String = serde_json::from_str(&text[key_start..key_end])
            .map_err(|error| malformed(&error.to_string()))?;
        let colon = skip_ws(text, key_end);
        if byte_at(text, colon) != Some(b':') {
            return Err(malformed("expected `:`"));
        }
        let value_start = skip_ws(text, colon + 1);
        let value_end = skip_value(text, value_start)?;
        members.push(RawMember {
            key,
            key_start,
            value_start,
            value_end,
        });
        cursor = skip_ws(text, value_end);
        match byte_at(text, cursor) {
            Some(b',') => cursor = skip_ws(text, cursor + 1),
            Some(b'}') => {
                return Ok(ScannedObject {
                    members,
                    end: cursor + 1,
                });
            }
            _ => return Err(malformed("expected `,` or `}`")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(text: &str) -> Document {
        Document::parse(text.as_bytes(), &[]).unwrap()
    }

    #[test]
    fn renders_unchanged_document_verbatim() {
        let text = "\u{feff} {\n    \"b\" :  1.50,\n    \"a\": [ 1,2 ],\"c\": \"\\u00e9\"\n}\n\n";
        assert_eq!(parse(text).render(&[]), text);
    }

    #[test]
    fn detects_jupyter_layout() {
        let document = parse("{\n \"a\": 1,\n \"b\": 2\n}\n");
        assert_eq!(document.layout, Layout::default());
    }

    #[test]
    fn detects_crlf_and_four_spaces() {
        let document = parse("{\r\n    \"a\": 1\r\n}\r\n");
        assert_eq!(document.layout.newline, "\r\n");
        assert_eq!(document.layout.indent.as_deref(), Some("    "));
    }

    #[test]
    fn detects_compact_layout() {
        let document = parse("{\"a\":1,\"b\":2}");
        assert_eq!(document.layout.indent, None);
        assert_eq!(document.layout.item_space, "");
        assert_eq!(document.layout.colon, ":");
    }

    #[test]
    fn changes_only_the_value_of_a_set_member() {
        let mut document = parse("{\n  \"a\":   1,\n  \"b\": [1,  2]\n}");
        assert!(document.root.set("b", json!([3])));
        assert_eq!(
            document.render(&[]),
            "{\n  \"a\":   1,\n  \"b\": [\n    3\n  ]\n}"
        );
    }

    #[test]
    fn setting_an_equal_value_keeps_the_text() {
        let mut document = parse("{\"a\": 1.0}");
        assert!(!document.root.set("a", json!(1.0)));
        assert!(!document.root.is_changed());
    }

    #[test]
    fn inserts_new_keys_in_sorted_position() {
        let mut document = parse("{\n \"a\": 1,\n \"c\": 3\n}");
        document.root.set("b", json!(2));
        assert_eq!(
            document.render(&[]),
            "{\n \"a\": 1,\n \"b\": 2,\n \"c\": 3\n}"
        );
    }

    #[test]
    fn appends_new_keys_to_unsorted_objects() {
        let mut document = parse("{\n \"c\": 1,\n \"a\": 3\n}");
        document.root.set("b", json!(2));
        assert_eq!(
            document.render(&[]),
            "{\n \"c\": 1,\n \"a\": 3,\n \"b\": 2\n}"
        );
    }

    #[test]
    fn removes_members_keeping_other_text() {
        let mut document = parse("{\n \"a\": 1,\n \"b\":  2,\n \"c\": 3\n}");
        assert!(document.root.remove("b"));
        assert_eq!(document.render(&[]), "{\n \"a\": 1,\n \"c\": 3\n}");
    }

    #[test]
    fn writes_python_escapes() {
        let layout = Layout::default();
        let text = layout.write_value(&json!("a\"\\\n\t\u{1b}é"), 0);
        assert_eq!(text, "\"a\\\"\\\\\\n\\t\\u001bé\"");
    }

    #[test]
    fn writes_floats_like_python() {
        let cases = [
            (1e-7, "1e-07"),
            (1e16, "1e+16"),
            (1.5e16, "1.5e+16"),
            (1e15, "1000000000000000.0"),
            (123_456_789_012_345_680.0, "1.2345678901234568e+17"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-10, "1.5e-10"),
            (1e-100, "1e-100"),
            (1e300, "1e+300"),
            (0.5, "0.5"),
            (1.0, "1.0"),
            (-2.5, "-2.5"),
            (-1e-7, "-1e-07"),
            (123.456, "123.456"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
        ];
        let layout = Layout::default();
        for (value, expected) in cases {
            assert_eq!(layout.write_value(&json!(value), 0), expected, "{value:e}");
        }
        assert_eq!(layout.write_value(&json!(42), 0), "42");
        assert_eq!(
            layout.write_value(&json!(u64::MAX), 0),
            "18446744073709551615"
        );
    }

    #[test]
    fn escapes_non_ascii_in_ascii_documents() {
        let mut document = parse("{\"a\": \"\\u00e9\"}");
        assert!(document.layout.ascii_only);
        document.root.set("a", json!("\u{17c}\u{1f600}"));
        assert_eq!(document.render(&[]), "{\"a\": \"\\u017c\\ud83d\\ude00\"}");
    }

    #[test]
    fn writes_empty_containers_on_one_line() {
        let layout = Layout::default();
        assert_eq!(
            layout.write_value(&json!({"a": [], "b": {}}), 0),
            "{\n \"a\": [],\n \"b\": {}\n}"
        );
    }

    #[test]
    fn reports_syntax_errors_with_position() {
        let error = Document::parse(b"{\n \"a\": 1,\n}", &[]).unwrap_err();
        match error {
            NotebookError::InvalidJson { line, .. } => assert_eq!(line, 3),
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn rejects_invalid_utf8() {
        let error = Document::parse(b"{\"a\": \"\xff\"}", &[]).unwrap_err();
        assert_eq!(error, NotebookError::NotUtf8 { offset: 7 });
    }

    #[test]
    fn rejects_non_object_root() {
        let error = Document::parse(b"[1]", &[]).unwrap_err();
        assert!(matches!(error, NotebookError::Structure { .. }));
    }

    #[test]
    fn rebuilds_array_with_original_gaps() {
        let text: Arc<str> = Arc::from("[\n  {\"a\": 1},\n  {\"b\": 2}\n]");
        let layout = Layout::default();
        let (array, mut items) = ObjectArray::parse(&text, 0, &layout, 0, "item").unwrap();
        items.swap(0, 1);
        assert_eq!(
            array.render(&items, &layout, 0),
            "[\n  {\"b\": 2},\n  {\"a\": 1}\n]"
        );
    }

    #[test]
    fn derives_separator_from_single_item_array() {
        let text: Arc<str> = Arc::from("[\n  {\"a\": 1}\n]");
        let layout = Layout::default();
        let (array, mut items) = ObjectArray::parse(&text, 0, &layout, 0, "item").unwrap();
        items.push(items[0].clone());
        assert_eq!(
            array.render(&items, &layout, 0),
            "[\n  {\"a\": 1},\n  {\"a\": 1}\n]"
        );
    }
}
