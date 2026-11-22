// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Environment files (`.env`).
//!
//! An environment file is a flat map of variables: `KEY=value`, optionally
//! preceded by `export`. Values are unquoted, in single quotes (literal) or
//! in double quotes (escapes `\\`, `\"`, `\n`, `\r`, `\t`; line breaks
//! allowed). Comments are full lines starting with `#` and, after an
//! unquoted or quoted value, text from a `#` preceded by whitespace.
//! References such as `${HOME}` are part of the value and are neither
//! expanded nor changed. A variable written more than once has the value of
//! its last occurrence.

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocItem;
use nurt_contracts::value::{Fields, Value};

use crate::formats::ini::support::errors::{key_conflict, parse_error, unsupported};
use crate::formats::ini::support::flat::{
    block_range, flat_key, flat_tree, removals, scalar_text, single_name,
};
use crate::formats::ini::support::text::{
    Line, LineIndex, Splice, apply_splices, dominant_newline, line_break_before_append, lines,
};
use crate::formats::ini::support::tree::{Node, resolve};

/// File names of environment files; names starting with `.env.` (such as
/// `.env.local`) are environment files as well.
pub const FILE_NAMES: [&str; 1] = [".env"];

/// Name of the format in error messages.
const FORMAT: &str = "env";

/// Keyword that exports a variable.
const EXPORT: &str = "export";

/// Construct name: a list or map assigned to a variable.
const NESTED: &str = "env.nested_value";
/// Construct name: a variable name that cannot be written.
const KEY_NAME: &str = "env.key_name";
/// Construct name: the changed file does not read back as expected.
const VERIFY: &str = "env.verify";

/// Returns `true` for file names of environment files: `.env` and
/// `.env.<suffix>`.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::env::is_env_file_name;
///
/// assert!(is_env_file_name("project/.env"));
/// assert!(is_env_file_name(".env.production"));
/// assert!(!is_env_file_name("env.txt"));
/// ```
pub fn is_env_file_name(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name == ".env" || name.starts_with(".env.")
}

/// Quoting of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quote {
    None,
    Single,
    Double,
    Backtick,
}

/// One variable definition.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    key: String,
    key_range: Range<usize>,
    /// Raw value with its quotes; empty at the end of the key when there is
    /// no `=`.
    value_range: Range<usize>,
    /// Decoded value; `None` for a name without `=`.
    value: Option<String>,
    quote: Quote,
    first_line: usize,
    last_line: usize,
    comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct Parsed {
    lines: Vec<Line>,
    entries: Vec<Entry>,
    comment_lines: Vec<bool>,
}

/// Parsed environment file that keeps its original text.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::env::EnvDocument;
///
/// let mut doc = EnvDocument::parse("export PATH=\"${HOME}/bin\" # local\n")?;
/// let key = KeyPath::parse("PATH")?;
/// assert_eq!(doc.get(&key)?[0].value, Value::Text("${HOME}/bin".into()));
/// doc.set(&key, &Value::Text("a b".into()), false)?;
/// assert_eq!(doc.text(), "export PATH=\"a b\" # local\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct EnvDocument {
    text: String,
    parsed: Parsed,
    tree: Node,
}

struct Plan {
    splices: Vec<Splice>,
    count: usize,
    expected: Vec<(String, Option<String>)>,
}

impl EnvDocument {
    /// Parses an environment file.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` for an invalid variable name, a line without `=` that
    /// is not a name, an unterminated quote or text after a closing quote.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let parsed = parse(text)?;
        let tree = flat_tree(parsed.entries.iter().map(|e| e.key.as_str()));
        Ok(Self {
            text: text.to_owned(),
            parsed,
            tree,
        })
    }

    /// Text of the file, including all changes made so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Values selected by `key`; a name without `=` is `null`.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing variable with the closest names,
    /// `E_KEY_CONFLICT` for indices.
    pub fn get(&self, key: &KeyPath) -> Result<Vec<DocItem>, Error> {
        let key = flat_key(key)?;
        let value_of = |node: &Node| self.node_value(node);
        let hits = resolve(&self.tree, &key, &value_of)?;
        let index = LineIndex::new(&self.text);
        hits.iter()
            .map(|hit| {
                let path = KeyPath::new(None, hit.path.clone())
                    .map_err(|error| key_conflict(&error.to_string(), "key path", &[]))?;
                let (range, comment) = match hit.node {
                    Node::Leaf(id) => {
                        let entry = &self.parsed.entries[*id];
                        (entry.value_range.clone(), entry.comment.clone())
                    }
                    _ => (0..self.text.len(), None),
                };
                Ok(DocItem {
                    key: path,
                    value: self.node_value(hit.node),
                    span: Some(index.span(&self.text, range)),
                    comment,
                    document: None,
                    anchor: None,
                    alias_of: None,
                    tag: None,
                })
            })
            .collect()
    }

    /// Content of the file as a map in the order of first occurrence.
    pub fn to_value(&self) -> Value {
        self.node_value(&self.tree)
    }

    /// Sets the variables selected by `key`; `create` appends a missing
    /// variable. Returns the number of variables set.
    ///
    /// The quoting of the current value is kept when it can hold the new
    /// value; a value that needs quoting is written in double quotes.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_NOT_FOUND` for a missing variable without `create`,
    /// `E_UNSUPPORTED_CONSTRUCT` `env.nested_value` for a list or map and
    /// `env.key_name` for a name that is not a variable name.
    pub fn set(&mut self, key: &KeyPath, value: &Value, create: bool) -> Result<usize, Error> {
        let text = scalar_text(value).ok_or_else(|| {
            unsupported(NESTED, "environment variables hold text; write a scalar value")
        })?;
        let flat = flat_key(key)?;
        let mut plan = self.plan();
        let value_of = |node: &Node| self.node_value(node);
        match resolve(&self.tree, &flat, &value_of) {
            Ok(hits) => {
                for hit in hits {
                    let Node::Leaf(id) = hit.node else {
                        return Err(unsupported(
                            NESTED,
                            "an environment file is a flat map; address a single variable",
                        ));
                    };
                    self.plan_value(&mut plan, *id, &text);
                }
            }
            Err(error) => match (create, single_name(&flat)) {
                (true, Some(name)) if error.code() == ErrorCode::KeyNotFound => {
                    self.plan_new(&mut plan, &name, &text)?;
                }
                _ => return Err(error),
            },
        }
        self.commit(plan)
    }

    /// Removes every definition of the variables selected by `key` with the
    /// comment lines directly above them. Returns the number of variables
    /// removed.
    ///
    /// # Errors
    ///
    /// The errors of [`EnvDocument::get`].
    pub fn remove(&mut self, key: &KeyPath) -> Result<usize, Error> {
        let flat = flat_key(key)?;
        let value_of = |node: &Node| self.node_value(node);
        let hits = resolve(&self.tree, &flat, &value_of)?;
        let names: Vec<&str> = hits
            .iter()
            .filter_map(|hit| match hit.node {
                Node::Leaf(id) => Some(self.parsed.entries[*id].key.as_str()),
                _ => None,
            })
            .collect();
        let mut plan = self.plan();
        let ranges = self
            .parsed
            .entries
            .iter()
            .filter(|entry| names.contains(&entry.key.as_str()))
            .map(|entry| {
                block_range(
                    &self.parsed.lines,
                    &self.parsed.comment_lines,
                    entry.first_line,
                    entry.last_line,
                    0,
                )
            })
            .collect();
        plan.splices = removals(ranges);
        plan.expected.retain(|(name, _)| !names.contains(&name.as_str()));
        plan.count = names.len();
        self.commit(plan)
    }

    fn node_value(&self, node: &Node) -> Value {
        match node {
            Node::Leaf(id) => self.parsed.entries[*id]
                .value
                .clone()
                .map_or(Value::Null, Value::Text),
            Node::Map(map) => Value::Map(
                map.members
                    .iter()
                    .map(|(name, node)| (name.clone(), self.node_value(node)))
                    .collect::<Fields>(),
            ),
            Node::List(_) | Node::Repeated(_) => Value::Null,
        }
    }

    fn plan(&self) -> Plan {
        Plan {
            splices: Vec::new(),
            count: 0,
            expected: self
                .parsed
                .entries
                .iter()
                .map(|e| (e.key.clone(), e.value.clone()))
                .collect(),
        }
    }

    fn plan_value(&self, plan: &mut Plan, id: usize, value: &str) {
        plan.count += 1;
        let entry = &self.parsed.entries[id];
        if entry.value.as_deref() == Some(value) {
            return;
        }
        plan.expected[id].1 = Some(value.to_owned());
        let raw = &self.text[entry.value_range.clone()];
        let encoded = encode(value, entry.quote, raw.contains('\n'));
        if entry.value.is_some() {
            plan.splices.push(Splice::new(entry.value_range.clone(), encoded));
        } else {
            plan.splices.push(Splice::insert(entry.key_range.end, format!("={encoded}")));
        }
    }

    fn plan_new(&self, plan: &mut Plan, name: &str, value: &str) -> Result<(), Error> {
        if !is_key(name) {
            return Err(unsupported(
                KEY_NAME,
                "variable names start with a letter or `_` and hold letters, digits, `_`, `.` and `-`",
            ));
        }
        plan.count += 1;
        plan.expected.push((name.to_owned(), Some(value.to_owned())));
        let last = self.parsed.entries.iter().rev().find(|e| e.value.is_some());
        let (prefix, separator) = match last {
            Some(entry) => {
                let line = self.parsed.lines[entry.first_line];
                let head = &self.text[line.start..entry.key_range.start];
                let separator = &self.text[entry.key_range.end..entry.value_range.start];
                (head.trim_start_matches([' ', '\t']), separator)
            }
            None => ("", "="),
        };
        let newline = dominant_newline(&self.text);
        let line = format!(
            "{}{prefix}{name}{separator}{}{newline}",
            line_break_before_append(&self.text, newline),
            encode(value, Quote::None, false),
        );
        plan.splices.push(Splice::insert(self.text.len(), line));
        Ok(())
    }

    fn commit(&mut self, plan: Plan) -> Result<usize, Error> {
        let splices: Vec<Splice> = plan
            .splices
            .into_iter()
            .filter(|s| self.text.get(s.range.clone()) != Some(s.text.as_str()))
            .collect();
        if splices.is_empty() {
            return Ok(plan.count);
        }
        let failed = || {
            unsupported(
                VERIFY,
                "the changed environment file does not read back with the expected variables; nothing was changed",
            )
        };
        let text = apply_splices(&self.text, &splices).ok_or_else(failed)?;
        let changed = Self::parse(&text).map_err(|_| failed())?;
        let found: Vec<(String, Option<String>)> = changed
            .parsed
            .entries
            .iter()
            .map(|e| (e.key.clone(), e.value.clone()))
            .collect();
        if found != plan.expected {
            return Err(failed());
        }
        *self = changed;
        Ok(plan.count)
    }
}

/// Returns `true` for a valid variable name.
fn is_key(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Raw text of `value`, keeping `quote` when it can hold the value.
fn encode(value: &str, quote: Quote, literal_newlines: bool) -> String {
    let fits_single = !value.contains(['\'', '\n', '\r']);
    match quote {
        Quote::Single if fits_single => return format!("'{value}'"),
        Quote::Backtick if !value.contains(['`', '\n', '\r']) => return format!("`{value}`"),
        Quote::None if is_plain(value) => return value.to_owned(),
        _ => {}
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' if !literal_newlines => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Returns `true` when a value reads back unchanged without quotes.
fn is_plain(value: &str) -> bool {
    !value.starts_with(['"', '\'', '`', '#'])
        && value.trim_matches([' ', '\t']) == value
        && !value.contains(['\n', '\r'])
        && !value.contains(" #")
        && !value.contains("\t#")
}

fn parse(text: &str) -> Result<Parsed, Error> {
    let lines = lines(text);
    let mut parsed = Parsed {
        comment_lines: vec![false; lines.len()],
        lines,
        entries: Vec::new(),
    };
    let mut comment: Vec<String> = Vec::new();
    let mut index = 0;
    while index < parsed.lines.len() {
        let line = parsed.lines[index];
        let content = line.content(text);
        let body = content.trim_start_matches([' ', '\t', '\u{feff}']);
        if body.is_empty() {
            comment.clear();
            index += 1;
        } else if let Some(note) = body.strip_prefix('#') {
            parsed.comment_lines[index] = true;
            comment.push(note.trim().to_owned());
            index += 1;
        } else {
            let start = line.content_end - body.len();
            let mut entry = parse_entry(text, &parsed.lines, index, start)?;
            entry.comment = merge_comment(&mut comment, entry.comment.take());
            index = entry.last_line + 1;
            parsed.entries.push(entry);
        }
    }
    Ok(parsed)
}

fn merge_comment(above: &mut Vec<String>, trailing: Option<String>) -> Option<String> {
    let mut parts = std::mem::take(above);
    parts.extend(trailing);
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn parse_entry(text: &str, lines: &[Line], index: usize, start: usize) -> Result<Entry, Error> {
    let line_end = lines[index].content_end;
    let mut body = &text[start..line_end];
    let mut key_start = start;
    if let Some(rest) = body.strip_prefix(EXPORT)
        && rest.starts_with([' ', '\t'])
    {
        let trimmed = rest.trim_start_matches([' ', '\t']);
        key_start = line_end - trimmed.len();
        body = trimmed;
    }
    let key_len = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .unwrap_or(body.len());
    let key = &body[..key_len];
    if !is_key(key) {
        return Err(parse_error(
            FORMAT,
            "expected a variable name of letters, digits, `_`, `.` and `-`",
            text,
            key_start,
        ));
    }
    let key_range = key_start..key_start + key_len;
    let after_key = body[key_len..].trim_start_matches([' ', '\t']);
    let Some(after_eq) = after_key.strip_prefix('=') else {
        return name_only(text, key, key_range, index, after_key, line_end);
    };
    let value_start = line_end - after_eq.trim_start_matches([' ', '\t']).len();
    let (value, quote, value_end, last_line) = parse_value(text, lines, index, value_start)?;
    let rest = &text[value_end..lines[last_line].content_end];
    let trailing = trailing_comment(rest, quote != Quote::None).ok_or_else(|| {
        parse_error(FORMAT, "unexpected text after the closing quote", text, value_end)
    })?;
    Ok(Entry {
        key: key.to_owned(),
        key_range,
        value_range: value_start..value_end,
        value: Some(value),
        quote,
        first_line: index,
        last_line,
        comment: trailing,
    })
}

fn name_only(
    text: &str,
    key: &str,
    key_range: Range<usize>,
    index: usize,
    rest: &str,
    line_end: usize,
) -> Result<Entry, Error> {
    let comment = trailing_comment(rest, true)
        .ok_or_else(|| parse_error(FORMAT, "expected `=` after the variable name", text, line_end - rest.len()))?;
    Ok(Entry {
        key: key.to_owned(),
        value_range: key_range.end..key_range.end,
        key_range,
        value: None,
        quote: Quote::None,
        first_line: index,
        last_line: index,
        comment,
    })
}

/// Comment in the text after a value: `Some(None)` when there is none,
/// `None` when the text is not a comment. After a quoted value the comment
/// marker may follow directly; after an unquoted one the caller has already
/// cut the value at the marker.
fn trailing_comment(rest: &str, allow_direct: bool) -> Option<Option<String>> {
    let trimmed = rest.trim_start_matches([' ', '\t']);
    if trimmed.is_empty() {
        return Some(None);
    }
    let direct = trimmed.len() == rest.len();
    let note = trimmed.strip_prefix('#')?;
    (allow_direct || !direct).then(|| Some(note.trim().to_owned()))
}

/// Parses a value from `start`; returns the decoded value, its quoting, the
/// end of the raw value and the last line it occupies.
fn parse_value(
    text: &str,
    lines: &[Line],
    index: usize,
    start: usize,
) -> Result<(String, Quote, usize, usize), Error> {
    let quote = match text[start..].chars().next() {
        Some('\'') => Quote::Single,
        Some('"') => Quote::Double,
        Some('`') => Quote::Backtick,
        _ => {
            let content = &text[start..lines[index].content_end];
            let cut = comment_start(content).unwrap_or(content.len());
            let raw = content[..cut].trim_end_matches([' ', '\t']);
            return Ok((raw.to_owned(), Quote::None, start + raw.len(), index));
        }
    };
    let close = closing_quote(text, start, quote)
        .ok_or_else(|| parse_error(FORMAT, "quoted value is not terminated", text, start))?;
    let inner = &text[start + 1..close];
    let value = if quote == Quote::Double {
        unescape_double(inner)
    } else {
        inner.to_owned()
    };
    let last_line = index + text[start..close].matches('\n').count();
    debug_assert!(last_line < lines.len());
    Ok((value, quote, close + 1, last_line))
}

/// Offset of a `#` preceded by whitespace in an unquoted value.
fn comment_start(content: &str) -> Option<usize> {
    let bytes = content.as_bytes();
    (1..bytes.len()).find(|&i| bytes[i] == b'#' && matches!(bytes[i - 1], b' ' | b'\t'))
}

fn closing_quote(text: &str, start: usize, quote: Quote) -> Option<usize> {
    let mark = match quote {
        Quote::Single => '\'',
        Quote::Backtick => '`',
        _ => '"',
    };
    let mut escaped = false;
    for (i, c) in text[start + 1..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quote == Quote::Double => escaped = true,
            _ if c == mark => return Some(start + 1 + i),
            _ => {}
        }
    }
    None
}

fn unescape_double(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(escaped @ ('\\' | '"')) => out.push(escaped),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    fn value(doc: &EnvDocument, name: &str) -> Value {
        doc.get(&key(name)).unwrap().remove(0).value
    }

    #[test]
    fn reads_quoting_styles_comments_and_interpolation() {
        let text = "# db\nexport DB_URL=postgres://h/${DB} # main\nA='x # y'\nB=\"l1\nl2\\t\\$X\"\nC=a#b\nD=\nE\nF = spaced\n";
        let doc = EnvDocument::parse(text).unwrap();
        assert_eq!(value(&doc, "DB_URL"), Value::Text("postgres://h/${DB}".into()));
        assert_eq!(value(&doc, "A"), Value::Text("x # y".into()));
        assert_eq!(value(&doc, "B"), Value::Text("l1\nl2\t\\$X".into()));
        assert_eq!(value(&doc, "C"), Value::Text("a#b".into()));
        assert_eq!(value(&doc, "D"), Value::Text(String::new()));
        assert_eq!(value(&doc, "E"), Value::Null);
        assert_eq!(value(&doc, "F"), Value::Text("spaced".into()));
        let item = doc.get(&key("DB_URL")).unwrap().remove(0);
        assert_eq!(item.comment.as_deref(), Some("db\nmain"));
    }

    #[test]
    fn rejects_malformed_lines() {
        for text in ["1A=x\n", "A='open\n", "A=\"x\" tail\n", "A b\n", "=x\n"] {
            assert!(EnvDocument::parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn keeps_the_quote_style_when_it_holds_the_value() {
        let mut doc = EnvDocument::parse("A='x'\nB=plain\nC=\"q\"\n").unwrap();
        doc.set(&key("A"), &Value::Text("y z".into()), false).unwrap();
        doc.set(&key("B"), &Value::Text("needs # quoting".into()), false).unwrap();
        doc.set(&key("C"), &Value::Text("say \"hi\"\n".into()), false).unwrap();
        assert_eq!(doc.text(), "A='y z'\nB=\"needs # quoting\"\nC=\"say \\\"hi\\\"\\n\"\n");
        doc.set(&key("A"), &Value::Text("it's".into()), false).unwrap();
        assert_eq!(value(&doc, "A"), Value::Text("it's".into()));
    }

    #[test]
    fn keeps_literal_line_breaks_of_multiline_values() {
        let mut doc = EnvDocument::parse("CERT=\"a\nb\"\nX=1\n").unwrap();
        doc.set(&key("CERT"), &Value::Text("c\nd\ne".into()), false).unwrap();
        assert_eq!(doc.text(), "CERT=\"c\nd\ne\"\nX=1\n");
    }

    #[test]
    fn creates_variables_in_the_style_of_the_file() {
        let mut doc = EnvDocument::parse("export A = 1").unwrap();
        doc.set(&key("NEW_VAR"), &Value::Bool(true), true).unwrap();
        assert_eq!(doc.text(), "export A = 1\nexport NEW_VAR = true\n");
        let error = doc.set(&key("bad name"), &Value::Int(1), true).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(KEY_NAME));
        let mut bare = EnvDocument::parse("FLAG\n").unwrap();
        bare.set(&key("FLAG"), &Value::Int(1), false).unwrap();
        assert_eq!(bare.text(), "FLAG=1\n");
    }

    #[test]
    fn removes_variables_with_their_comments() {
        let mut doc = EnvDocument::parse("# a\nA=1\n\nB=2\nA=3").unwrap();
        assert_eq!(value(&doc, "A"), Value::Text("3".into()));
        doc.remove(&key("A")).unwrap();
        assert_eq!(doc.text(), "\nB=2");
        assert_eq!(doc.to_value().get("B"), Some(&Value::Text("2".into())));
    }

    #[test]
    fn setting_the_same_value_changes_no_byte() {
        let text = "A=\"x\"  # c\r\nB='y'\r\n";
        let mut doc = EnvDocument::parse(text).unwrap();
        assert_eq!(doc.set(&key("*"), &Value::Text("x".into()), false).unwrap(), 2);
        assert_eq!(value(&doc, "B"), Value::Text("x".into()));
        let mut same = EnvDocument::parse(text).unwrap();
        same.set(&key("A"), &Value::Text("x".into()), false).unwrap();
        assert_eq!(same.text(), text);
    }

    #[test]
    fn plain_values_round_trip() {
        for sample in ["", "a b", "x#y", "${A}:${B}", "C:\\dir"] {
            assert!(is_plain(sample) || sample.is_empty());
            assert_eq!(encode(sample, Quote::None, false), sample);
        }
        assert_eq!(encode(" lead", Quote::None, false), "\" lead\"");
        assert!(is_env_file_name("a\\.env.local"));
    }
}
