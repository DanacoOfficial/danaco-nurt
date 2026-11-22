// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registered formats `json` and `jsonc` and their document handle.
//!
//! Every change goes through [`guard::commit`]: the edits must lie in the
//! changed nodes and their separators, and `serde_json` must read the new
//! text as the expected value.

use nurt_contracts::error::{Error, Result};
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::syntax::{self, Entry, Tree};
use super::{Dialect, JsonDocument, Outcome, Target, editor, lexeme, reference};
use crate::constructs::{self, DOC_SEMANTIC_MISMATCH};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{
    DocHandle, DocTree, FormatSemantics, Layout, MapSemantics, NodeKind, NodeSpec, ScalarStyle,
    SetOptions, Span, Style, TreeBuilder, doc_parse, key_not_found,
};
use crate::registry::{Confidence, DocFormat, DocOptions};

/// Rules of key paths in JSON: a repeated member name means its last
/// occurrence.
const SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// Flag of a change that created intermediate members.
const KEYS_CREATED_FLAG: &str = "keys_created";

/// Byte order mark of UTF-8, skipped when sniffing.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// The format `json` or `jsonc`.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::json::{Dialect, JsonFormat};
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let format = JsonFormat::new(Dialect::Jsonc);
/// let handle = format.open(b"{\"a\": 1, // note\n}".to_vec(), &DocOptions::default())?;
/// assert_eq!(handle.text(), b"{\"a\": 1, // note\n}");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JsonFormat {
    dialect: Dialect,
}

impl JsonFormat {
    /// Format of the given dialect.
    #[must_use]
    pub const fn new(dialect: Dialect) -> Self {
        Self { dialect }
    }
}

impl DocFormat for JsonFormat {
    fn id(&self) -> DocFormatId {
        self.dialect.format_id()
    }

    fn extensions(&self) -> &[&'static str] {
        match self.dialect {
            Dialect::Json => &["json"],
            Dialect::Jsonc => &["jsonc"],
        }
    }

    fn file_names(&self) -> &[&'static str] {
        match self.dialect {
            Dialect::Json => &[],
            Dialect::Jsonc => &super::JSONC_FILE_NAMES,
        }
    }

    /// A text that parses in the dialect is probably a document of it; for
    /// JSONC the text must also use a comment or a trailing comma, so that
    /// plain JSON goes to `json`. A text that starts with a bracket (or, for
    /// JSONC, a comment) but does not parse, possibly because it is only the
    /// start of a file, could be one.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let content = content.strip_prefix(UTF8_BOM).unwrap_or(content);
        let start = content
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(content.len());
        let rest = &content[start..];
        let bracket = matches!(rest.first(), Some(b'{' | b'['));
        let comment = rest.starts_with(b"//") || rest.starts_with(b"/*");
        if !bracket && !(comment && self.dialect == Dialect::Jsonc) {
            return Confidence::None;
        }
        let parsed = std::str::from_utf8(content)
            .ok()
            .and_then(|text| syntax::parse(text, self.dialect).ok());
        match (self.dialect, parsed) {
            (Dialect::Json, Some(_)) => Confidence::Medium,
            (Dialect::Jsonc, Some(tree)) if uses_jsonc_syntax(&tree) => Confidence::Medium,
            _ => Confidence::Low,
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let id = self.id();
        let text = String::from_utf8(content).map_err(|error| {
            let offset = error.utf8_error().valid_up_to();
            doc_parse(
                &id,
                error.as_bytes(),
                Some(offset),
                "the text is not valid UTF-8; save the file as UTF-8",
            )
        })?;
        Ok(Box::new(JsonHandle::new(JsonDocument::parse(
            text,
            self.dialect,
        )?)))
    }
}

/// The tree has a comment or a trailing comma.
fn uses_jsonc_syntax(tree: &Tree) -> bool {
    !tree.comments.is_empty()
        || tree.nodes.iter().any(|node| {
            node.entries()
                .and_then(<[Entry]>::last)
                .is_some_and(|last| last.comma.is_some())
        })
}

/// Reads JSON and JSONC with `serde_json`, for the meaning guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JsonReference(Dialect);

impl ReferenceParser for JsonReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let id = self.0.format_id();
        let text = std::str::from_utf8(text).map_err(|error| {
            doc_parse(
                &id,
                text,
                Some(error.valid_up_to()),
                "the text is not valid UTF-8",
            )
        })?;
        let value = reference::read(text, self.0)
            .map_err(|message| doc_parse(&id, text.as_bytes(), None, message))?;
        Ok(vec![value])
    }
}

/// Open JSON or JSONC document.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::json::{Dialect, JsonDocument, JsonHandle};
/// use nurt_doc::model::{DocHandle, SetOptions};
///
/// let document = JsonDocument::parse("{\n  \"name\": \"a\"\n}\n", Dialect::Json)?;
/// let mut handle = JsonHandle::new(document);
/// let options = SetOptions { create: true };
/// handle.set(&KeyPath::parse("version")?, &Value::Text("1.0".to_owned()), &options)?;
/// assert_eq!(handle.text(), b"{\n  \"name\": \"a\",\n  \"version\": \"1.0\"\n}\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct JsonHandle {
    document: JsonDocument,
    tree: DocTree,
}

impl JsonHandle {
    /// Handle of a parsed document.
    #[must_use]
    pub fn new(document: JsonDocument) -> Self {
        let tree = build_tree(&document);
        Self { document, tree }
    }

    /// The parsed document.
    #[must_use]
    pub const fn document(&self) -> &JsonDocument {
        &self.document
    }

    /// Concrete keys selected by `key`: the key itself when it has only
    /// names, indexes and a final `[+]`, otherwise the keys of the nodes it
    /// matches, without repetitions.
    fn concrete_keys(&self, key: &KeyPath) -> Result<Vec<KeyPath>> {
        let direct = matches!(key.document(), None | Some(DocSelector::Index(_)))
            && key.segments().iter().all(|segment| {
                matches!(
                    segment,
                    KeySegment::Key(_) | KeySegment::Index(_) | KeySegment::Append
                )
            });
        if direct {
            return Ok(vec![key.clone()]);
        }
        let mut keys: Vec<KeyPath> = Vec::new();
        for resolved in self.tree.resolve(key, &SEMANTICS)? {
            if !keys.contains(&resolved.key) {
                keys.push(resolved.key);
            }
        }
        if keys.is_empty() {
            return Err(key_not_found(key, &[]));
        }
        Ok(keys)
    }

    fn reference(&self) -> JsonReference {
        JsonReference(self.document.dialect())
    }

    fn expectation(&self) -> Expectation<'static> {
        Expectation::new(vec![self.document.to_value()], &SEMANTICS)
    }

    /// Checks the plan with the guards, then takes the new text.
    fn commit(&mut self, plan: EditPlan, expectation: &Expectation<'_>) -> Result<EditReport> {
        let committed = guard::commit(
            self.document.text().as_bytes(),
            plan,
            expectation,
            &self.reference(),
        )?;
        if committed.report.is_unchanged() {
            return Ok(committed.report);
        }
        let text = String::from_utf8(committed.bytes)
            .map_err(|_| mismatch("the change produced invalid UTF-8"))?;
        let document = JsonDocument::parse(text, self.document.dialect())
            .map_err(|error| mismatch("the changed document does not parse").with_cause(error))?;
        *self = Self::new(document);
        Ok(committed.report)
    }

    /// Keys to remove with the span of their object or array in the
    /// current text: nodes inside another selected node are dropped, and
    /// the rest come in reverse document order, so that removing one keeps
    /// the offsets and indexes of the others.
    fn removal_order(&self, keys: Vec<KeyPath>) -> Result<Vec<(KeyPath, Span)>> {
        let tree = &self.document.tree;
        let mut located = Vec::with_capacity(keys.len());
        for key in keys {
            let Target::Node { id, .. } = self.document.locate(&key)? else {
                return Err(key_not_found(&key, &[]));
            };
            let container = tree.nodes[id].parent.unwrap_or(id);
            located.push((tree.nodes[id].span, span(tree.nodes[container].span), key));
        }
        let spans: Vec<syntax::Span> = located.iter().map(|(node, _, _)| *node).collect();
        located.retain(|(node, _, _)| {
            !spans
                .iter()
                .any(|outer| outer != node && outer.start <= node.start && node.end <= outer.end)
        });
        located.sort_by_key(|(node, _, _)| std::cmp::Reverse(node.start));
        Ok(located
            .into_iter()
            .map(|(_, container, key)| (key, container))
            .collect())
    }
}

impl DocHandle for JsonHandle {
    fn format(&self) -> DocFormatId {
        self.document.dialect().format_id()
    }

    fn text(&self) -> &[u8] {
        self.document.text().as_bytes()
    }

    fn tree(&self) -> &DocTree {
        &self.tree
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        &SEMANTICS
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        let keys = self.concrete_keys(key)?;
        let mut plan = EditPlan::new();
        let mut expectation = self.expectation();
        for concrete in keys {
            let outcome = self.document.plan_set(&concrete, value, options.create)?;
            for target in &outcome.targets {
                plan.target(span(*target));
            }
            record(&mut plan, &outcome, &concrete);
            for change in outcome.changes {
                plan.edit(ByteEdit::replace(
                    span(change.span),
                    change.text.into_bytes(),
                ));
            }
            expectation = expectation.set(concrete, value.clone());
        }
        self.commit(plan, &expectation)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let keys = self.removal_order(self.concrete_keys(key)?)?;
        let original = self.document.text().to_owned();
        let mut current = self.document.clone();
        let mut plan = EditPlan::new();
        let mut expectation = self.expectation();
        for (concrete, container) in keys {
            let outcome = current.plan_remove(&concrete)?;
            plan.target(container);
            record(&mut plan, &outcome, &concrete);
            let text = editor::apply(current.text(), &outcome.changes)?;
            current = JsonDocument::parse(text, current.dialect()).map_err(|error| {
                mismatch("the document does not parse after the removal").with_cause(error)
            })?;
            expectation = expectation.remove(concrete);
        }
        let mut rewrite =
            EditPlan::from_rewrite(original.as_bytes(), current.text().as_bytes(), plan.targets);
        rewrite.report = plan.report;
        self.commit(rewrite, &expectation)
    }
}

/// Adds the effects of one operation to the report of the plan.
fn record(plan: &mut EditPlan, outcome: &Outcome, key: &KeyPath) {
    if !outcome.changes.is_empty() {
        plan.report.replacements += 1;
    }
    if outcome.duplicate_key {
        plan.report
            .note(Notable::DuplicateKey, Some(key.to_string()));
    }
    if outcome.keys_created {
        plan.report
            .note(Notable::KeysCreated, Some(key.to_string()));
        plan.report.flag(KEYS_CREATED_FLAG);
    }
}

const fn span(span: syntax::Span) -> Span {
    Span::new(span.start, span.end)
}

fn mismatch(message: &str) -> Error {
    constructs::unsupported(DOC_SEMANTIC_MISMATCH, message)
}

/// Node tree of the document. Members carry their name and key span; the
/// span of a member reaches from the comment lines directly above it, which
/// a removal takes with it, or else from its key, to the end of its value.
fn build_tree(document: &JsonDocument) -> DocTree {
    let tree = &document.tree;
    let mut builder = TreeBuilder::new(document.text().as_bytes());
    let mut pending: Vec<(syntax::NodeId, Option<crate::model::NodeId>, Option<&Entry>)> =
        vec![(tree.root, None, None)];
    while let Some((id, parent, entry)) = pending.pop() {
        let added = builder.node(parent, node_spec(document, id, entry));
        if let Some(entries) = tree.nodes[id].entries() {
            pending.extend(
                entries
                    .iter()
                    .rev()
                    .map(|child| (child.value, Some(added), Some(child))),
            );
        }
    }
    builder.finish()
}

fn node_spec(document: &JsonDocument, id: syntax::NodeId, entry: Option<&Entry>) -> NodeSpec {
    let node = &document.tree.nodes[id];
    let value_span = span(node.span);
    let (kind, value, scalar) = match &node.kind {
        syntax::NodeKind::Null => (NodeKind::Scalar, Some(Value::Null), ScalarStyle::Plain),
        syntax::NodeKind::Bool(flag) => (
            NodeKind::Scalar,
            Some(Value::Bool(*flag)),
            ScalarStyle::Plain,
        ),
        syntax::NodeKind::Number => {
            let text = &document.text()[node.span.start..node.span.end];
            (
                NodeKind::Scalar,
                Some(lexeme::number_value(text)),
                ScalarStyle::Plain,
            )
        }
        syntax::NodeKind::String(text) => (
            NodeKind::Scalar,
            Some(Value::Text(text.clone())),
            ScalarStyle::DoubleQuoted,
        ),
        syntax::NodeKind::Object(_) => (NodeKind::Map, None, ScalarStyle::Plain),
        syntax::NodeKind::Array(_) => (NodeKind::List, None, ScalarStyle::Plain),
    };
    let layout = if value.is_none() {
        Layout::Flow
    } else {
        Layout::default()
    };
    let key = entry.and_then(|entry| entry.key.as_ref());
    let start = key.map_or(node.span.start, |key| key.span.start);
    let start = document
        .leading_comment_start(id)
        .map_or(start, |comments| comments.min(start));
    let mut spec = NodeSpec::new(kind, Span::new(start, node.span.end))
        .with_value_span(value_span)
        .with_style(Style {
            scalar,
            layout,
            indent: None,
        });
    if let Some(key) = key {
        spec = spec
            .with_name(key.text.clone())
            .with_key_span(span(key.span));
    }
    if let Some(value) = value {
        spec = spec.with_value(value);
    }
    if let Some(comment) = document.comment_of(id) {
        spec = spec.with_comment(comment);
    }
    spec
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;

    fn handle(text: &str, dialect: Dialect) -> JsonHandle {
        JsonHandle::new(JsonDocument::parse(text, dialect).unwrap())
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    const CREATE: SetOptions = SetOptions { create: true };

    #[test]
    fn builds_a_tree_with_names_spans_values_and_comments() {
        let doc = handle(
            "{\n  // the name\n  \"name\": \"x\",\n  \"list\": [1.10, true]\n}",
            Dialect::Jsonc,
        );
        let found = doc.get(&key("name")).unwrap();
        let node = &doc.tree()[found[0].node];
        assert_eq!(node.comment.as_deref(), Some("the name"));
        assert_eq!(node.value, Some(Value::Text("x".to_owned())));
        assert_eq!(node.start.line, 3);
        assert_eq!(
            &doc.text()[node.span.start..node.span.end],
            b"// the name\n  \"name\": \"x\""
        );
        assert_eq!(
            &doc.text()[node.key_span.unwrap().start..node.value_span.end],
            b"\"name\": \"x\""
        );
        let list = doc.get(&key("list[0]")).unwrap();
        assert_eq!(
            doc.tree()[list[0].node].value,
            Some(Value::Decimal("1.10".to_owned()))
        );
        assert_eq!(doc.to_value().unwrap(), doc.document().to_value());
    }

    #[test]
    fn sets_every_value_matched_by_a_pattern() {
        let mut doc = handle("[{\"a\": 1}, {\"a\": 2}, {\"b\": 3}]", Dialect::Json);
        let report = doc.set(&key("[*].a"), &Value::Int(0), &CREATE).unwrap();
        assert_eq!(report.replacements, 2);
        assert_eq!(doc.text(), b"[{\"a\": 0}, {\"a\": 0}, {\"b\": 3}]");
    }

    #[test]
    fn removes_every_value_matched_by_a_pattern() {
        let mut doc = handle(
            "{\n  \"l\": [\n    1,\n    2,\n    3\n  ],\n  \"k\": 0\n}",
            Dialect::Json,
        );
        let report = doc.remove(&key("l[*]")).unwrap();
        assert_eq!(report.replacements, 3);
        assert_eq!(doc.text(), b"{\n  \"l\": [],\n  \"k\": 0\n}");
    }

    #[test]
    fn a_pattern_that_selects_nothing_fails_with_key_not_found() {
        let mut doc = handle("{\"l\": []}", Dialect::Json);
        let error = doc
            .set(&key("l[*].a"), &Value::Int(0), &CREATE)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::KeyNotFound);
    }

    #[test]
    fn reports_repeated_member_names_and_created_keys() {
        let mut doc = handle("{\"a\": 1, \"a\": 2}", Dialect::Json);
        let report = doc.set(&key("a"), &Value::Int(3), &CREATE).unwrap();
        assert_eq!(report.notable[0].code, Notable::DuplicateKey);
        let report = doc.remove(&key("a")).unwrap();
        assert_eq!(report.notable[0].code, Notable::DuplicateKey);
        assert_eq!(doc.text(), b"{}");
        let report = doc.set(&key("x.y"), &Value::Null, &CREATE).unwrap();
        assert_eq!(report.flags, vec![KEYS_CREATED_FLAG.to_owned()]);
        assert_eq!(doc.text(), b"{\"x\": {\"y\": null}}");
    }

    #[test]
    fn removes_a_member_below_a_repeated_name_once() {
        let mut doc = handle(
            "{\"a\": {\"x\": 1}, \"a\": {\"x\": 2, \"y\": 3}}",
            Dialect::Json,
        );
        let report = doc.remove(&key("a.x")).unwrap();
        assert_eq!(report.notable[0].code, Notable::DuplicateKey);
        assert_eq!(doc.text(), b"{\"a\": {\"x\": 1}, \"a\": {\"y\": 3}}");
    }

    #[test]
    fn setting_the_current_value_reports_no_change() {
        let mut doc = handle("{\"n\": 1e400}", Dialect::Json);
        let report = doc
            .set(&key("n"), &Value::Decimal("1e400".to_owned()), &CREATE)
            .unwrap();
        assert!(report.is_unchanged());
        assert_eq!(report.replacements, 0);
    }

    #[test]
    fn rejects_invalid_text_when_opening() {
        let format = JsonFormat::new(Dialect::Json);
        for content in [b"{\"a\": 1,}".to_vec(), vec![b'"', 0xFF, b'"']] {
            let Err(error) = format.open(content, &DocOptions::default()) else {
                panic!("invalid text opened");
            };
            assert_eq!(error.code, ErrorCode::DocParse);
        }
    }

    #[test]
    fn sniffs_documents_of_each_dialect() {
        let json = JsonFormat::new(Dialect::Json);
        let jsonc = JsonFormat::new(Dialect::Jsonc);
        assert_eq!(json.sniff(b"\xEF\xBB\xBF  {}"), Confidence::Medium);
        assert_eq!(jsonc.sniff(b"[1]"), Confidence::Low);
        assert_eq!(jsonc.sniff(b"// settings\n{}"), Confidence::Medium);
        assert_eq!(jsonc.sniff(b"[1,]"), Confidence::Medium);
        assert_eq!(json.sniff(b"// settings\n{}"), Confidence::None);
        assert_eq!(json.sniff(b"{\"truncated\": "), Confidence::Low);
        assert_eq!(json.sniff(b"{unquoted: 1}"), Confidence::Low);
        assert_eq!(json.sniff(b"a: 1"), Confidence::None);
        assert_eq!(json.file_names(), &[] as &[&str]);
        assert!(jsonc.file_names().contains(&"tsconfig.*"));
        assert_eq!(jsonc.extensions(), &["jsonc"]);
    }

    #[test]
    fn reference_parser_reads_one_document() {
        let values = JsonReference(Dialect::Jsonc)
            .parse(b"[1, // one\n]")
            .unwrap();
        assert_eq!(values, vec![Value::List(vec![Value::Int(1)])]);
        assert!(JsonReference(Dialect::Json).parse(b"[1,]").is_err());
        assert!(JsonReference(Dialect::Json).parse(&[0xFF]).is_err());
    }
}
