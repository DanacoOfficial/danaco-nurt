// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::{Fields, Value};

use crate::edit::guard::{EditPlan, Expectation, ReferenceParser, commit};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{
    DocHandle, DocTree, FlatSemantics, FormatSemantics, NodeKind, NodeSpec, SetOptions, Span,
    TreeBuilder, doc_parse, key_conflict, key_not_found,
};
use crate::registry::{Confidence, DocFormat, DocOptions};
use crate::style::{Spacing, StyleProbe};
use crate::value::{Syntax, parse_number, render_scalar, to_document_value};

/// Minimal format of `name = value` lines with `#` comments, for tests of
/// the shared layers.
///
/// Values are typed like JSON scalars (`8080` is an integer, `true` a
/// boolean, anything else text) and cannot hold line breaks; names are flat
/// (`server.port` is one name). A comment line directly above an entry is
/// attached to it. Changes go through the guards of [`crate::edit::guard`]
/// with [`KeyValueReference`] as the reference parser.
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::services::DocFormatId;
/// use nurt_contracts::value::Value;
/// use nurt_doc::model::SetOptions;
/// use nurt_doc::registry::{DocFormat, DocOptions};
/// use nurt_doc::testing::KeyValueFormat;
///
/// let format = KeyValueFormat::new(DocFormatId::new("kv")?);
/// let mut document = format.open(b"a = 1\n".to_vec(), &DocOptions::default())?;
/// document.set(&KeyPath::parse("a")?, &Value::Int(2), &SetOptions::default())?;
/// document.set(&KeyPath::parse("b")?, &Value::Text("x".to_owned()), &SetOptions { create: true })?;
/// assert_eq!(document.text(), b"a = 2\nb = x\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValueFormat {
    id: DocFormatId,
}

impl KeyValueFormat {
    /// Format registered under `id`.
    #[must_use]
    pub const fn new(id: DocFormatId) -> Self {
        Self { id }
    }
}

impl DocFormat for KeyValueFormat {
    fn id(&self) -> DocFormatId {
        self.id.clone()
    }

    fn extensions(&self) -> &[&'static str] {
        &["kv"]
    }

    fn sniff(&self, content: &[u8]) -> Confidence {
        let text = String::from_utf8_lossy(content);
        let mut entries = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .peekable();
        if entries.peek().is_some() && entries.all(|line| line.contains('=')) {
            Confidence::Low
        } else {
            Confidence::None
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let tree = parse(&self.id, &content)?;
        Ok(Box::new(KeyValueDocument {
            id: self.id.clone(),
            text: content,
            tree,
        }))
    }
}

/// Typed value of a scalar text.
fn typed(text: &str) -> Value {
    match text {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        other => parse_number(other, Syntax::Json).unwrap_or_else(|| Value::Text(other.to_owned())),
    }
}

/// Lines of `text` as `(start, end, next)`: the line without its line break
/// and the start of the next line.
fn lines(text: &[u8]) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= text.len() {
            return None;
        }
        let next = text[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(text.len(), |at| start + at + 1);
        let mut end = next;
        while end > start && matches!(text[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        let line = (start, end, next);
        start = next;
        Some(line)
    })
}

fn trimmed(text: &[u8], span: Span) -> Span {
    let mut start = span.start;
    let mut end = span.end;
    while start < end && matches!(text[start], b' ' | b'\t') {
        start += 1;
    }
    while end > start && matches!(text[end - 1], b' ' | b'\t') {
        end -= 1;
    }
    Span::new(start, end)
}

fn parse(id: &DocFormatId, text: &[u8]) -> Result<DocTree> {
    let source = std::str::from_utf8(text)
        .map_err(|error| doc_parse(id, text, Some(error.valid_up_to()), "invalid UTF-8"))?;
    let mut builder = TreeBuilder::new(text);
    let root = builder.node(None, NodeSpec::new(NodeKind::Map, Span::new(0, text.len())));
    let mut comment: Option<String> = None;
    for (start, end, _) in lines(text) {
        let line = trimmed(text, Span::new(start, end));
        let content = &source[line.start..line.end];
        if content.is_empty() {
            comment = None;
            continue;
        }
        if let Some(note) = content.strip_prefix('#') {
            comment = Some(note.trim().to_owned());
            continue;
        }
        let Some(equals) = content.find('=') else {
            return Err(doc_parse(
                id,
                text,
                Some(line.start),
                "expected `name = value`",
            ));
        };
        let key_span = trimmed(text, Span::new(line.start, line.start + equals));
        let value_span = trimmed(text, Span::new(line.start + equals + 1, line.end));
        let name = &source[key_span.start..key_span.end];
        if name.is_empty() {
            return Err(doc_parse(
                id,
                text,
                Some(line.start),
                "expected a name before `=`",
            ));
        }
        let mut spec = NodeSpec::new(NodeKind::Scalar, Span::new(line.start, line.end))
            .with_name(name)
            .with_key_span(key_span)
            .with_value_span(value_span)
            .with_value(typed(&source[value_span.start..value_span.end]));
        if let Some(note) = comment.take() {
            spec = spec.with_comment(note);
        }
        builder.node(Some(root), spec);
    }
    Ok(builder.finish())
}

/// Independent reader of [`KeyValueFormat`] documents: splits lines at the
/// first `=` without building a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyValueReference;

impl ReferenceParser for KeyValueReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let text = std::str::from_utf8(text)
            .map_err(|_| Error::new(ErrorCode::DocParse, "the document is not valid UTF-8"))?;
        let mut fields = Fields::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line.split_once('=').ok_or_else(|| {
                Error::new(ErrorCode::DocParse, format!("line `{line}` has no `=`"))
            })?;
            fields.insert(name.trim().to_owned(), typed(value.trim()));
        }
        Ok(vec![Value::Map(fields)])
    }
}

struct KeyValueDocument {
    id: DocFormatId,
    text: Vec<u8>,
    tree: DocTree,
}

impl KeyValueDocument {
    fn render(key: &KeyPath, value: &Value) -> Result<String> {
        match to_document_value(value.clone()) {
            Value::Text(text) if !text.contains(['\n', '\r']) && text.trim() == text => Ok(text),
            Value::Text(_) => Err(Error::new(
                ErrorCode::InvalidValue,
                "values are single-line text without surrounding blanks; remove line breaks",
            )),
            Value::Map(_) | Value::List(_) => Err(key_conflict(
                key,
                "map",
                "sets a map or list; the format holds scalars only",
            )),
            other => render_scalar(&other, Syntax::Json).ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "a {} value cannot be written; set text or a number",
                        other.type_name()
                    ),
                )
            }),
        }
    }

    fn name_of(key: &KeyPath) -> Result<String> {
        let names: Option<Vec<&str>> = key
            .segments()
            .iter()
            .map(|segment| match segment {
                KeySegment::Key(name) => Some(name.as_str()),
                _ => None,
            })
            .collect();
        match names {
            Some(names) if !names.is_empty() => Ok(names.join(".")),
            _ => Err(key_conflict(
                key,
                "map",
                "is not a name; the format has flat names only",
            )),
        }
    }

    fn expectation(&self) -> Expectation<'static> {
        let value = self
            .tree
            .roots()
            .first()
            .map(|root| self.tree.value_of(*root, &FlatSemantics, None))
            .unwrap_or_default();
        Expectation::new(vec![value], &FlatSemantics)
    }

    /// Commits the plan and rebuilds the tree from the new text.
    ///
    /// The expectation and the old tree are released before the new tree
    /// is built, so that a change of a large document does not hold two
    /// trees at once; when the new text does not parse, the tree of the old
    /// text is rebuilt.
    fn accept(&mut self, plan: EditPlan, expectation: Expectation<'_>) -> Result<EditReport> {
        let committed = commit(&self.text, plan, &expectation, &KeyValueReference)?;
        drop(expectation);
        self.tree = DocTree::default();
        match parse(&self.id, &committed.bytes) {
            Ok(tree) => {
                self.tree = tree;
                self.text = committed.bytes;
                Ok(committed.report)
            }
            Err(error) => {
                self.tree = parse(&self.id, &self.text)?;
                Err(error)
            }
        }
    }

    fn line_with_break(&self, span: Span) -> Span {
        let probe = StyleProbe::new(&self.text);
        let line = probe.line_at(span.start);
        let end = self.text[line.end..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(self.text.len(), |at| line.end + at + 1);
        Span::new(line.start, end)
    }
}

impl DocHandle for KeyValueDocument {
    fn format(&self) -> DocFormatId {
        self.id.clone()
    }

    fn text(&self) -> &[u8] {
        &self.text
    }

    fn tree(&self) -> &DocTree {
        &self.tree
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        &FlatSemantics
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        let found = self.get(key)?;
        let rendered = Self::render(key, value)?;
        let mut plan = EditPlan::new();
        let mut expectation = self.expectation();
        if found.is_empty() {
            if !options.create {
                return Err(key_not_found(
                    key,
                    &self.tree.similar_keys(key, &FlatSemantics),
                ));
            }
            let name = Self::name_of(key)?;
            let probe = StyleProbe::new(&self.text);
            let spacing = if self.text.contains(&b'=') {
                probe.separator(b'=')
            } else {
                Spacing {
                    before: " ".to_owned(),
                    after: " ".to_owned(),
                }
            };
            let eol = probe.line_ending().as_str();
            let lead = if self.text.is_empty() || self.text.ends_with(b"\n") {
                ""
            } else {
                eol
            };
            let line = format!(
                "{lead}{name}{}={}{rendered}{eol}",
                spacing.before, spacing.after
            );
            plan.target(Span::at(self.text.len()));
            plan.edit(ByteEdit::insert(self.text.len(), line));
            plan.report.replacements = 1;
            expectation = expectation.set(key.clone(), value.clone());
        }
        for resolved in &found {
            let node = &self.tree[resolved.node];
            if node.kind.is_container() {
                return Err(key_conflict(
                    key,
                    node.kind.as_str(),
                    "selects the document; set a name",
                ));
            }
            plan.target(node.value_span);
            plan.edit(ByteEdit::replace(node.value_span, rendered.clone()));
            plan.report.replacements += 1;
            expectation = expectation.set(resolved.key.clone(), value.clone());
        }
        self.accept(plan, expectation)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let found = self.get(key)?;
        if found.is_empty() {
            return Err(key_not_found(
                key,
                &self.tree.similar_keys(key, &FlatSemantics),
            ));
        }
        let mut plan = EditPlan::new();
        let mut expectation = self.expectation();
        for resolved in &found {
            let node = &self.tree[resolved.node];
            if node.parent.is_none() {
                return Err(key_conflict(
                    key,
                    "map",
                    "selects the document; remove a name",
                ));
            }
            let line = self.line_with_break(node.span);
            plan.target(line);
            plan.edit(ByteEdit::delete(line));
            plan.report.replacements += 1;
            expectation = expectation.remove(resolved.key.clone());
        }
        self.accept(plan, expectation)
    }
}
