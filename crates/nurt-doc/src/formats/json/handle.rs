// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registered formats `json` and `jsonc` and their document handle.
//!
//! Every change goes through [`guard::commit`]: the edits must lie in the
//! changed nodes and their separators, and `serde_json` must read the new
//! text as the expected value.

use nurt_contracts::error::{Error, Result};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::syntax::{self, Entry};
use super::{Dialect, JsonDocument, Outcome, Target, editor, lexeme, reference};
use crate::constructs::{self, DOC_SEMANTIC_MISMATCH};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{
    DocHandle, DocTree, FormatSemantics, Layout, MapSemantics, NodeKind, NodeSpec, ScalarStyle, SetOptions, Span,
    Style, TreeBuilder, doc_parse,
};
use crate::registry::{Confidence, DocFormat, DocOptions};

/// Rules of key paths in JSON: a repeated member name means its last
/// occurrence.
const SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// Flag of a change that created intermediate members.
const KEYS_CREATED_FLAG: &str = "keys_created";

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

    /// A text starting with `{` or `[` is probably JSON; a text starting
    /// with a comment is probably JSONC.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let content = content.strip_prefix(syntax::UTF8_BOM).unwrap_or(content);
        let start = content.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(content.len());
        let rest = &content[start..];
        let bracket = matches!(rest.first(), Some(b'{' | b'['));
        let comment = rest.starts_with(b"//") || rest.starts_with(b"/*");
        match (self.dialect, bracket, comment) {
            (Dialect::Json, true, _) | (Dialect::Jsonc, _, true) => Confidence::Medium,
            (Dialect::Jsonc, true, _) => Confidence::Low,
            _ => Confidence::None,
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let id = self.id();
        let text = String::from_utf8(content).map_err(|error| {
            let offset = error.utf8_error().valid_up_to();
            doc_parse(&id, error.as_bytes(), Some(offset), "the text is not valid UTF-8; save the file as UTF-8")
        })?;
        Ok(Box::new(JsonHandle::new(JsonDocument::parse(text, self.dialect)?)))
    }
}

/// Reads JSON and JSONC with `serde_json`, for the meaning guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JsonReference(Dialect);

impl ReferenceParser for JsonReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let id = self.0.format_id();
        let text = std::str::from_utf8(text)
            .map_err(|error| doc_parse(&id, text, Some(error.valid_up_to()), "the text is not valid UTF-8"))?;
        let value = reference::read(text, self.0).map_err(|message| doc_parse(&id, text.as_bytes(), None, message))?;
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
        let direct = key.document().is_none()
            && key
                .segments()
                .iter()
                .all(|segment| matches!(segment, KeySegment::Key(_) | KeySegment::Index(_) | KeySegment::Append));
        if direct {
            return Ok(vec![key.clone()]);
        }
        let mut keys: Vec<KeyPath> = Vec::new();
        for resolved in self.tree.resolve(key, &SEMANTICS)? {
            if !keys.contains(&resolved.key) {
                keys.push(resolved.key);
            }
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
        let committed = guard::commit(self.document.text().as_bytes(), plan, expectation, &self.reference())?;
        if committed.report.is_unchanged() {
            return Ok(committed.report);
        }
        let text = String::from_utf8(committed.bytes).map_err(|_| mismatch("the change produced invalid UTF-8"))?;
        let document = JsonDocument::parse(text, self.document.dialect())
            .map_err(|error| mismatch("the changed document does not parse").with_cause(error))?;
        *self = Self::new(document);
        Ok(committed.report)
    }

    /// Keys of nodes that are not inside another selected node, in reverse
    /// document order, so that removing one keeps the offsets and indexes of
    /// the others.
    fn removal_order(&self, keys: Vec<KeyPath>) -> Result<Vec<KeyPath>> {
        let mut located = Vec::with_capacity(keys.len());
        for key in keys {
            match self.document.locate(&key)? {
                Target::Node { id, .. } => located.push((self.document.tree.nodes[id].span, key)),
                _ => return Err(crate::model::key_not_found(&key, &[])),
            }
        }
        let spans: Vec<syntax::Span> = located.iter().map(|(span, _)| *span).collect();
        located.retain(|(span, _)| {
            !spans
                .iter()
                .any(|outer| outer != span && outer.start <= span.start && span.end <= outer.end)
        });
        located.sort_by(|(a, _), (b, _)| b.start.cmp(&a.start));
        Ok(located.into_iter().map(|(_, key)| key).collect())
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
            record(&mut plan, &outcome, &concrete);
            for change in outcome.changes {
                plan.edit(ByteEdit::replace(span(change.span), change.text.into_bytes()));
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
        for concrete in keys {
            let outcome = current.plan_remove(&concrete)?;
            record(&mut plan, &outcome, &concrete);
            let text = editor::apply(current.text(), &outcome.changes)?;
            current = JsonDocument::parse(text, current.dialect())
                .map_err(|error| mismatch("the document does not parse after the removal").with_cause(error))?;
            expectation = expectation.remove(concrete);
        }
        let mut rewrite = EditPlan::from_rewrite(original.as_bytes(), current.text().as_bytes(), plan.targets);
        rewrite.report = plan.report;
        self.commit(rewrite, &expectation)
    }
}

/// Adds the targets and the effects of one operation to the plan.
fn record(plan: &mut EditPlan, outcome: &Outcome, key: &KeyPath) {
    for target in &outcome.targets {
        plan.target(span(*target));
    }
    if !outcome.changes.is_empty() {
        plan.report.replacements += 1;
    }
    if outcome.duplicate_key {
        plan.report.note(Notable::DuplicateKey, Some(key.to_string()));
    }
    if outcome.keys_created {
        plan.report.note(Notable::KeysCreated, Some(key.to_string()));
        plan.report.flag(KEYS_CREATED_FLAG);
    }
}

const fn span(span: syntax::Span) -> Span {
    Span::new(span.start, span.end)
}

fn mismatch(message: &str) -> Error {
    constructs::unsupported(DOC_SEMANTIC_MISMATCH, message)
}

/// Node tree of the document: members carry their name, key span and the
/// span from the key to the end of the value.
fn build_tree(document: &JsonDocument) -> DocTree {
    let tree = &document.tree;
    let mut builder = TreeBuilder::new(document.text().as_bytes());
    let mut pending: Vec<(syntax::NodeId, Option<crate::model::NodeId>, Option<&Entry>)> = vec![(tree.root, None, None)];
    while let Some((id, parent, entry)) = pending.pop() {
        let added = builder.node(parent, node_spec(document, id, entry));
        if let Some(entries) = tree.nodes[id].entries() {
            pending.extend(entries.iter().rev().map(|child| (child.value, Some(added), Some(child))));
        }
    }
    builder.finish()
}

fn node_spec(document: &JsonDocument, id: syntax::NodeId, entry: Option<&Entry>) -> NodeSpec {
    let node = &document.tree.nodes[id];
    let value_span = span(node.span);
    let (kind, value, scalar) = match &node.kind {
        syntax::NodeKind::Null => (NodeKind::Scalar, Some(Value::Null), ScalarStyle::Plain),
        syntax::NodeKind::Bool(flag) => (NodeKind::Scalar, Some(Value::Bool(*flag)), ScalarStyle::Plain),
        syntax::NodeKind::Number => {
            let text = &document.text()[node.span.start..node.span.end];
            (NodeKind::Scalar, Some(lexeme::number_value(text)), ScalarStyle::Plain)
        }
        syntax::NodeKind::String(text) => {
            (NodeKind::Scalar, Some(Value::Text(text.clone())), ScalarStyle::DoubleQuoted)
        }
        syntax::NodeKind::Object(_) => (NodeKind::Map, None, ScalarStyle::Plain),
        syntax::NodeKind::Array(_) => (NodeKind::List, None, ScalarStyle::Plain),
    };
    let layout = if value.is_none() { Layout::Flow } else { Layout::Block };
    let key = entry.and_then(|entry| entry.key.as_ref());
    let whole = key.map_or(value_span, |key| Span::new(key.span.start, node.span.end));
    let mut spec = NodeSpec::new(kind, whole).with_value_span(value_span).with_style(Style {
        scalar,
        layout,
        indent: None,
    });
    if let Some(key) = key {
        spec = spec.with_name(key.text.clone()).with_key_span(span(key.span));
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
        let doc = handle("{\n  // the name\n  \"name\": \"x\",\n  \"list\": [1.10, true]\n}", Dialect::Jsonc);
        let found = doc.get(&key("name")).unwrap();
        let node = &doc.tree()[found[0].node];
        assert_eq!(node.comment.as_deref(), Some("the name"));
        assert_eq!(node.value, Some(Value::Text("x".to_owned())));
        assert_eq!(node.start.line, 3);
        assert_eq!(&doc.text()[node.key_span.unwrap().start..node.value_span.end], b"\"name\": \"x\"");
        let list = doc.get(&key("list[0]")).unwrap();
        assert_eq!(doc.tree()[list[0].node].value, Some(Value::Decimal("1.10".to_owned())));
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
        let mut doc = handle("{\n  \"l\": [\n    1,\n    2,\n    3\n  ],\n  \"k\": 0\n}", Dialect::Json);
        let report = doc.remove(&key("l[*]")).unwrap();
        assert_eq!(report.replacements, 3);
        assert_eq!(doc.text(), b"{\n  \"l\": [],\n  \"k\": 0\n}");
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
    fn setting_the_current_value_reports_no_change() {
        let mut doc = handle("{\"n\": 1e400}", Dialect::Json);
        let report = doc.set(&key("n"), &Value::Decimal("1e400".to_owned()), &CREATE).unwrap();
        assert!(report.is_unchanged());
        assert_eq!(report.replacements, 0);
    }

    #[test]
    fn rejects_invalid_text_when_opening() {
        let format = JsonFormat::new(Dialect::Json);
        let error = format.open(b"{\"a\": 1,}".to_vec(), &DocOptions::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::DocParse);
        let error = format.open(vec![b'"', 0xFF, b'"'], &DocOptions::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::DocParse);
    }

    #[test]
    fn sniffs_brackets_and_comments() {
        let json = JsonFormat::new(Dialect::Json);
        let jsonc = JsonFormat::new(Dialect::Jsonc);
        assert_eq!(json.sniff(b"\xEF\xBB\xBF  {}"), Confidence::Medium);
        assert_eq!(jsonc.sniff(b"[1]"), Confidence::Low);
        assert_eq!(jsonc.sniff(b"// settings\n{}"), Confidence::Medium);
        assert_eq!(json.sniff(b"// settings\n{}"), Confidence::None);
        assert_eq!(json.sniff(b"a: 1"), Confidence::None);
        assert_eq!(json.file_names(), &[] as &[&str]);
        assert!(jsonc.file_names().contains(&"tsconfig.*"));
        assert_eq!(jsonc.extensions(), &["jsonc"]);
    }

    #[test]
    fn reference_parser_reads_one_document() {
        let values = JsonReference(Dialect::Jsonc).parse(b"[1, // one\n]").unwrap();
        assert_eq!(values, vec![Value::List(vec![Value::Int(1)])]);
        assert!(JsonReference(Dialect::Json).parse(b"[1,]").is_err());
        assert!(JsonReference(Dialect::Json).parse(&[0xFF]).is_err());
    }
}
