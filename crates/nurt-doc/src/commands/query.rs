// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `query` stage.
//!
//! `query` applies a `jq` filter to documents and produces one `item` record
//! per result. In the source form the document is the file given by `path`;
//! in the stage form the input records supply the values:
//!
//! | Input | Value given to the filter |
//! |---|---|
//! | `content` | the text read as a document of the format of the file; JSON when no format is recognised |
//! | `entry` | the file read as a document |
//! | `item` | the value of the item, with the key of the item as the base of result keys |
//! | `response` | the body read as a document; JSON when no format is recognised |
//!
//! A result produced by a path expression gets the canonical key of its
//! position in the document; other results get the key `"."`.

use std::collections::VecDeque;
use std::io::Read;
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Location, Result, Stage};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::keypath::{DocSelector, KeyPath, KeySegment};
use nurt_contracts::record::Item;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Path, Record, RecordKind, Value};

use crate::jq::{PathStep, Query, QueryOutput, value_type};
use crate::registry::{DocOptions, Registry};

/// Name of the command in the catalog.
pub const COMMAND: &str = "query";

/// Key of results that are not values at a key path of the input.
pub const COMPUTED_KEY: &str = ".";

const EXPRESSION: &str = "expression";
const PATH: &str = "path";
const JSON_MEDIA_TYPE: &str = "json";

/// Implementation of the `query` command.
///
/// A stage built with [`QueryStage::new`] detects and opens documents through
/// the format registry, so that the path of the file takes part in the choice
/// of the format. A stage without a registry reads documents through the
/// document service of the context, and as JSON when the context has none.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_doc::commands::query::QueryStage;
///
/// let descriptor = QueryStage::default().descriptor();
/// assert_eq!(descriptor.command, "query");
/// assert!(descriptor.source);
/// ```
#[derive(Debug, Clone, Default)]
pub struct QueryStage {
    registry: Option<Arc<Registry>>,
}

impl QueryStage {
    /// Stage that opens documents with the formats of `registry`.
    #[must_use]
    pub const fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry: Some(registry),
        }
    }
}

impl StageImpl for QueryStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![
                RecordKind::Content,
                RecordKind::Entry,
                RecordKind::Item,
                RecordKind::Response,
            ]),
        }
    }

    /// Compiles a constant filter, so that a filter that does not parse is
    /// reported at binding, before any document is read.
    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        if let Some(expression) = args.text(EXPRESSION)? {
            Query::compile(expression).map_err(|error| error.with_stage(Stage::Bind))?;
        }
        let mut info = StaticInfo::default();
        if let Some(path) = args.value(PATH).and_then(Value::as_str) {
            info.paths.push(path.to_owned());
        }
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for QueryStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let subjects: Subjects<'a> = if input.is_none() {
            let path = args.value(PATH).and_then(Value::as_str).ok_or_else(|| {
                Error::new(
                    ErrorCode::MissingParam,
                    "`query` without input records needs `path`; add path: \"<file>\"",
                )
                .with_stage(Stage::Bind)
            })?;
            Box::new(std::iter::once(Ok(Subject::File(path.to_owned()))))
        } else {
            Box::new(
                input
                    .into_records()?
                    .map(|record| record.map(Subject::Record)),
            )
        };
        let outputs = Outputs {
            args,
            ctx,
            registry: self.registry.as_deref(),
            subjects,
            pending: VecDeque::new(),
            query: None,
            done: false,
        };
        Ok(StageOutput::records(
            RecordStream::new(outputs).checked(ctx),
        ))
    }
}

enum Subject {
    Record(Record),
    File(String),
}

type Subjects<'a> = Box<dyn Iterator<Item = Result<Subject>> + Send + 'a>;

/// One value given to the filter and what the results inherit from it.
pub(crate) struct Root {
    /// Path of the file, or the address of a response.
    pub(crate) path: String,
    /// Format identifier.
    pub(crate) format: String,
    /// Entity tag of the content.
    pub(crate) etag: String,
    /// Key path of the value in its document; `None` when the value has no
    /// key path.
    pub(crate) key: Option<KeyPath>,
    /// The value.
    pub(crate) value: Value,
}

/// Content of a file and its entity tag.
pub(crate) struct FileContent {
    /// The bytes.
    pub(crate) bytes: Vec<u8>,
    /// Entity tag: the content hash in the form `sha256:<hex>`.
    pub(crate) etag: String,
}

struct Outputs<'a> {
    args: &'a BoundArgs,
    ctx: &'a ExecCtx,
    registry: Option<&'a Registry>,
    subjects: Subjects<'a>,
    pending: VecDeque<Result<Record>>,
    query: Option<Query>,
    done: bool,
}

impl Iterator for Outputs<'_> {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                if item.is_err() {
                    self.done = true;
                    self.pending.clear();
                }
                return Some(item);
            }
            if self.done {
                return None;
            }
            match self.subjects.next()? {
                Ok(subject) => {
                    if let Err(error) = self.process(subject) {
                        self.pending.push_back(Err(error));
                    }
                }
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
    }
}

impl Outputs<'_> {
    fn process(&mut self, subject: Subject) -> Result<()> {
        let record = match &subject {
            Subject::Record(record) => Some(record),
            Subject::File(_) => None,
        };
        self.prepare_query(record)?;
        let roots = match subject {
            Subject::Record(record) => record_roots(self.ctx, self.registry, &record)?,
            Subject::File(path) => file_roots(self.ctx, self.registry, &path)?,
        };
        let Some(query) = self.query.as_ref() else {
            return Ok(());
        };
        for root in roots {
            let run = query.run(&root.value, self.ctx);
            for output in run.outputs {
                self.pending.push_back(Ok(item_record(&root, output)));
            }
            if let Some(error) = run.error {
                return Err(locate(error.with_stage(Stage::Read), &root));
            }
        }
        Ok(())
    }

    /// Compiles the filter, again only when a per-record expression yields
    /// another text.
    fn prepare_query(&mut self, record: Option<&Record>) -> Result<()> {
        let value = self.args.eval(EXPRESSION, record, self.ctx)?;
        let Some(Value::Text(expression)) = value else {
            return Err(Error::new(
                ErrorCode::Type,
                "`query` needs the filter as text, for example query \".name\"",
            )
            .with_stage(Stage::Read));
        };
        if self
            .query
            .as_ref()
            .is_none_or(|query| query.expression() != expression)
        {
            let query =
                Query::compile(&expression).map_err(|error| error.with_stage(Stage::Read))?;
            self.query = Some(query);
        }
        Ok(())
    }
}

fn locate(error: Error, root: &Root) -> Error {
    let error = error.with_location(Location {
        path: root.path.clone(),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        byte: None,
    });
    match &root.key {
        Some(key) => error.with_detail("key", serde_json::Value::from(key.to_string())),
        None => error,
    }
}

fn item_record(root: &Root, output: QueryOutput) -> Record {
    let key = output
        .path
        .as_deref()
        .zip(root.key.as_ref())
        .and_then(|(steps, base)| concrete_key(base, steps))
        .map_or_else(|| COMPUTED_KEY.to_owned(), |key| key.to_string());
    Record::from(Item {
        path: Path::new(root.path.clone()),
        format: root.format.clone(),
        key,
        value_type: value_type(&output.value),
        value: output.value,
        etag: root.etag.clone(),
        ..Item::default()
    })
}

/// Key path of a result: the key of the input value followed by the steps
/// of the result; `None` when a step has no key path counterpart.
fn concrete_key(base: &KeyPath, steps: &[PathStep]) -> Option<KeyPath> {
    let mut key = base.clone();
    for step in steps {
        let segment = match step {
            PathStep::Key(name) => KeySegment::Key(name.clone()),
            PathStep::Index(index) => KeySegment::Index(*index),
            PathStep::Other(_) => return None,
        };
        key.push(segment).ok()?;
    }
    Some(key)
}

fn text_field<'r>(record: &'r Record, name: &str) -> Option<&'r str> {
    record.get(name).and_then(Value::as_str)
}

fn record_roots(ctx: &ExecCtx, registry: Option<&Registry>, record: &Record) -> Result<Vec<Root>> {
    match record.kind {
        RecordKind::Content => content_roots(ctx, registry, record),
        RecordKind::Entry => {
            let kind = text_field(record, "type").unwrap_or("file");
            let path = text_field(record, PATH).unwrap_or_default();
            if kind != "file" {
                return Err(Error::new(
                    ErrorCode::Type,
                    format!(
                        "`query` reads files, not the {kind} {path}; select files with type: file"
                    ),
                )
                .with_stage(Stage::Read));
            }
            file_roots(ctx, registry, path)
        }
        RecordKind::Item => Ok(vec![item_root(record)]),
        RecordKind::Response => response_roots(ctx, registry, record),
        other => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`query` reads content, entry, item or response records, not {}",
                other.as_str()
            ),
        )
        .with_stage(Stage::Read)),
    }
}

fn item_root(record: &Record) -> Root {
    Root {
        path: text_field(record, PATH).unwrap_or_default().to_owned(),
        format: text_field(record, "format").unwrap_or_default().to_owned(),
        etag: text_field(record, "etag").unwrap_or_default().to_owned(),
        key: text_field(record, "key").and_then(|key| KeyPath::parse(key).ok()),
        value: record.get("value").cloned().unwrap_or_default(),
    }
}

fn content_roots(ctx: &ExecCtx, registry: Option<&Registry>, record: &Record) -> Result<Vec<Root>> {
    let path = text_field(record, PATH).unwrap_or_default();
    if record.get("binary") == Some(&Value::Bool(true)) {
        return Err(Error::new(
            ErrorCode::Binary,
            format!("`query` reads text documents; {path} is binary"),
        )
        .with_stage(Stage::Read));
    }
    let text = text_field(record, "text").unwrap_or_default();
    let etag = text_field(record, "etag").unwrap_or_default();
    document_roots(ctx, registry, path, text.as_bytes(), None, etag)
}

fn response_roots(
    ctx: &ExecCtx,
    registry: Option<&Registry>,
    record: &Record,
) -> Result<Vec<Root>> {
    let url = text_field(record, "url").unwrap_or_default();
    let body = text_field(record, "body").unwrap_or_default();
    let etag = text_field(record, "etag").unwrap_or_default();
    let hint = text_field(record, "content_type")
        .filter(|media| media.contains(JSON_MEDIA_TYPE))
        .map(|_| DocFormatId::JSON);
    document_roots(ctx, registry, url, body.as_bytes(), hint, etag)
}

fn file_roots(ctx: &ExecCtx, registry: Option<&Registry>, path: &str) -> Result<Vec<Root>> {
    let content = read_file(ctx, path)?;
    document_roots(ctx, registry, path, &content.bytes, None, &content.etag)
}

/// Reads a file through the file system of the context.
///
/// # Errors
///
/// Fails with the error of the file system, with `E_LIMIT` when the file is
/// larger than `max_file_bytes_text`, and with `E_STALE` when the file
/// changed while it was read.
pub(crate) fn read_file(ctx: &ExecCtx, path: &str) -> Result<FileContent> {
    let fs = ctx.require_fs()?;
    let abs = AbsPath::new(path)?;
    let before = fs.content_hash(&abs)?;
    let mut handle = fs.open_read(&abs)?;
    let limit = ctx.limits().max_file_bytes_text;
    let size = handle.meta()?.size;
    if size > limit {
        return Err(Error::new(
            ErrorCode::Limit,
            format!("{path} has {size} bytes, more than the limit of {limit} bytes for documents"),
        ));
    }
    let mut bytes = Vec::new();
    handle
        .by_ref()
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| Error::new(ErrorCode::Io, format!("reading {path} failed: {error}")))?;
    let after = fs.content_hash(&abs)?;
    if before != after {
        return Err(Error::new(
            ErrorCode::Stale,
            format!("{path} changed while it was read; run the command again"),
        ));
    }
    Ok(FileContent {
        bytes,
        etag: after.to_string(),
    })
}

/// Values of a document: one root per document of a multi-document stream.
fn document_roots(
    ctx: &ExecCtx,
    registry: Option<&Registry>,
    path: &str,
    bytes: &[u8],
    hint: Option<DocFormatId>,
    etag: &str,
) -> Result<Vec<Root>> {
    let service = ctx.doc();
    let file = (!path.is_empty()).then(|| std::path::Path::new(path));
    let format = hint.or_else(|| match registry {
        Some(registry) => registry.detect(file, bytes),
        None => service.and_then(|service| service.detect_format(file, bytes)),
    });
    let root = |key: Option<KeyPath>, format: &str, value: Value| Root {
        path: path.to_owned(),
        format: format.to_owned(),
        etag: etag.to_owned(),
        key,
        value,
    };
    let Some(format) = format.filter(|_| registry.is_some() || service.is_some()) else {
        let value = parse_json(path, bytes)?;
        return Ok(vec![root(
            Some(KeyPath::root()),
            DocFormatId::JSON.as_str(),
            value,
        )]);
    };
    let selector = if format == DocFormatId::YAML {
        KeyPath::new(Some(DocSelector::All), Vec::new())?
    } else {
        KeyPath::root()
    };
    let items = match (registry, service) {
        (Some(registry), _) => read_with_registry(ctx, registry, &format, file, bytes, &selector),
        (None, Some(service)) => service
            .read_keys(bytes, &format, &selector, None)
            .map(|items| {
                items
                    .into_iter()
                    .map(|item| (item.key, item.value))
                    .collect()
            }),
        (None, None) => Ok(Vec::new()),
    }
    .map_err(|error| error.with_stage(Stage::Read))?;
    Ok(items
        .into_iter()
        .map(|(key, value)| root(Some(key), format.as_str(), value))
        .collect())
}

/// Opens the document with the path of its file as context and returns the
/// values at `selector` with their keys.
fn read_with_registry(
    ctx: &ExecCtx,
    registry: &Registry,
    format: &DocFormatId,
    file: Option<&std::path::Path>,
    bytes: &[u8],
    selector: &KeyPath,
) -> Result<Vec<(KeyPath, Value)>> {
    let options = DocOptions::from_config(&ctx.config().documents);
    let document = registry.open_at(format, file, bytes, &options)?;
    let handle = &document.handle;
    let tree = handle.tree();
    Ok(handle
        .get(selector)?
        .into_iter()
        .map(|resolved| {
            let value = tree.value_of(resolved.node, handle.semantics(), None);
            (resolved.key, value)
        })
        .collect())
}

/// Value of a document read as JSON. Only a document without a recognised
/// format or without a document service gets here: a recognised format is
/// read by the service.
fn parse_json(path: &str, bytes: &[u8]) -> Result<Value> {
    let parsed = jaq_json::read::parse_single(bytes).map_err(|error| {
        Error::new(
            ErrorCode::DocParse,
            format!("{path} is not a document of a known format nor JSON: {error}"),
        )
        .with_stage(Stage::Read)
        .with_location(Location {
            path: path.to_owned(),
            line: None,
            column: None,
            end_line: None,
            end_column: None,
            byte: None,
        })
    })?;
    Ok(crate::jq::value_of(&parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn result_keys_extend_the_base_key() {
        let steps = [PathStep::Key("b.c".to_owned()), PathStep::Index(-1)];
        assert_eq!(
            concrete_key(&key("a"), &steps).unwrap().to_string(),
            r#"a["b.c"][-1]"#
        );
        assert_eq!(
            concrete_key(&KeyPath::root(), &[]).unwrap().to_string(),
            "$"
        );
        assert!(concrete_key(&key("a"), &[PathStep::Other(Value::Null)]).is_none());
    }

    #[test]
    fn computed_results_get_the_dot_key() {
        let root = Root {
            path: "/data/a.json".to_owned(),
            format: "json".to_owned(),
            etag: "sha256:00".to_owned(),
            key: Some(KeyPath::root()),
            value: Value::Null,
        };
        let record = item_record(
            &root,
            QueryOutput {
                value: Value::Int(2),
                path: None,
            },
        );
        let item = Item::try_from(record).unwrap();
        assert_eq!(item.key, COMPUTED_KEY);
        assert_eq!(item.format, "json");
        assert_eq!(item.value, Value::Int(2));
    }

    #[test]
    fn item_roots_parse_their_key() {
        let record = Record::from(Item {
            path: Path::new("/data/a.yaml"),
            format: "yaml".to_owned(),
            key: "spec.replicas".to_owned(),
            value: Value::Int(3),
            ..Item::default()
        });
        let root = item_root(&record);
        assert_eq!(root.key, Some(key("spec.replicas")));
        assert_eq!(root.value, Value::Int(3));
        assert_eq!(root.format, "yaml");
    }
}
