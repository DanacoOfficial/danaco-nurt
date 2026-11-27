// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Commands of the group `notebook`: cells of Jupyter notebooks in
//! nbformat 4.
//!
//! `notebook.cells` lists cells as `cell` records. The editing commands read
//! the notebook, change the parsed document and replace the content of the
//! file in one write; cells and members the change does not touch keep their
//! bytes, so a difference of the file shows only the changed cells. The
//! journal checks that the file still has the entity tag it had when it was
//! read. `notebook.export` writes the notebook to a new file.
//!
//! Errors of the notebook library keep their codes, with two refinements: an
//! index outside the notebook is `E_OUT_OF_RANGE`, and every error names the
//! notebook in its location. A file that is not an nbformat 4 notebook is
//! `E_FORMAT`.

use std::collections::HashMap;
use std::io::Read;
use std::ops::{ControlFlow, Range as Span};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Location, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Etag, FileMeta, Follow, Fs};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::record::{Cell, CellType, Change, Effect, EffectStatus};
use nurt_contracts::stage::{
    BoundArgs, EffectOutcome, ExecCtx, RecordStream, StageImpl, StageOutput, StaticInfo,
};
use nurt_contracts::value::{self, Fields, Range, Value};
use nurt_notebook::{CellData, Literal, Notebook, NotebookError, Pattern};
use nurt_text::search::{Hit, Match, Pattern as SearchPattern, SearchOptions, Searcher};
use nurt_text::template::{CaptureSource, Groups, Template, TemplateError};
use serde_json::json;

use crate::common::path::{path_arg, path_field};

pub(crate) mod cells;
pub(crate) mod edit;
pub(crate) mod export;

/// Stages of the group `notebook`.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    [cells::stages(), edit::stages(), export::stages()]
        .into_iter()
        .flatten()
        .collect()
}

/// Name of the `path` parameter.
pub(crate) const PATH: &str = "path";
/// Name of the `index` parameter.
pub(crate) const INDEX: &str = "index";
/// Name of the `id` parameter.
pub(crate) const ID: &str = "id";
/// Name of the `cell_type` parameter.
pub(crate) const CELL_TYPE: &str = "cell_type";
/// Name of the `find` parameter.
pub(crate) const FIND: &str = "find";

/// Values of `cell_type`.
const CELL_TYPES: [&str; 3] = ["code", "markdown", "raw"];

/// Text value of a `range` parameter that selects every cell.
const ALL: &str = "all";

/// Counter of the `effect` record for the rewritten notebook.
const FILES_MODIFIED: &str = "files_modified";

/// Notebook read from a file, with the state the journal checks before the
/// file is replaced.
#[derive(Debug, Clone)]
pub(crate) struct LoadedNotebook {
    /// Notebook file.
    pub(crate) path: AbsPath,
    /// Metadata of the file as read, a link in the last component not
    /// followed.
    pub(crate) meta: FileMeta,
    /// Content of the file as read.
    pub(crate) bytes: Vec<u8>,
    /// Parsed notebook.
    pub(crate) notebook: Notebook,
}

impl LoadedNotebook {
    /// Entity tag of the file as read.
    pub(crate) fn etag(&self) -> Etag {
        Etag {
            hash: Some(self.hash()),
            ..self.meta.etag()
        }
    }

    /// SHA-256 hash of the file as read.
    pub(crate) fn hash(&self) -> ContentHash {
        nurt_text::hash::content_hash(&self.bytes)
    }

    /// Size of the file as read, in bytes.
    pub(crate) fn size(&self) -> u64 {
        count(self.bytes.len())
    }
}

/// Reads and parses a notebook.
///
/// # Errors
///
/// Fails with the errors of the file system (`E_NOT_FOUND`, `E_PERMISSION`,
/// `E_IO`), with `E_LIMIT` when the file is larger than the text limit, and
/// with `E_FORMAT` when the file is not an nbformat 4 notebook.
pub(crate) fn load(ctx: &ExecCtx, path: &AbsPath) -> Result<LoadedNotebook> {
    let fs = ctx.require_fs()?;
    let meta = fs.metadata(path, Follow::NoFollow)?;
    let bytes = read_limited(fs, path, ctx.limits().max_file_bytes_text)?;
    let notebook = Notebook::parse(&bytes).map_err(|error| notebook_error(&error, path))?;
    Ok(LoadedNotebook {
        path: path.clone(),
        meta,
        bytes,
        notebook,
    })
}

fn read_limited(fs: &dyn Fs, path: &AbsPath, limit: u64) -> Result<Vec<u8>> {
    let mut handle = fs.open_read(path)?;
    let size = handle.meta()?.size;
    if size > limit {
        return Err(limit_error(path, size, limit));
    }
    let mut bytes = Vec::new();
    if let Some(mapped) = handle.as_bytes() {
        bytes.extend_from_slice(mapped);
    } else {
        handle
            .by_ref()
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
            })?;
    }
    let read = count(bytes.len());
    if read > limit {
        return Err(limit_error(path, read, limit));
    }
    Ok(bytes)
}

fn limit_error(path: &AbsPath, size: u64, limit: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "`{path}` has {size} bytes, more than the {limit} bytes read as text: raise the text limit or split the notebook"
        ),
    )
    .with_expected(json!(limit))
    .with_actual(json!(size))
}

/// Number as the value of a record field.
pub(crate) fn count(number: usize) -> u64 {
    u64::try_from(number).unwrap_or(u64::MAX)
}

/// Error of the language for an error of the notebook library: the code of
/// the library, `E_OUT_OF_RANGE` for an index outside the notebook, the
/// notebook as the location and a program that shows valid cells or repairs
/// identifiers where one helps.
pub(crate) fn notebook_error(error: &NotebookError, path: &AbsPath) -> Error {
    let cause = innermost(error);
    let mut converted = error.to_error();
    if matches!(cause, NotebookError::IndexOutOfRange { .. }) {
        converted.code = ErrorCode::OutOfRange;
    }
    let mut location = Location {
        path: path.to_language_string(),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        byte: None,
    };
    match cause {
        NotebookError::InvalidJson { line, column, .. } => {
            location.line = Some(count(*line));
            location.column = Some(count(*column));
        }
        NotebookError::NotUtf8 { offset } => location.byte = Some(count(*offset)),
        _ => {}
    }
    let converted = converted.with_location(location);
    let fix = match cause {
        NotebookError::DuplicateId { .. } => Some("notebook.normalize"),
        NotebookError::IndexOutOfRange { .. }
        | NotebookError::UnknownId { .. }
        | NotebookError::AddressMismatch { .. } => Some("notebook.cells"),
        _ => None,
    };
    match fix {
        Some(command) => converted.with_fix(format!("{command} {}", quoted(path))),
        None => converted,
    }
}

fn innermost(error: &NotebookError) -> &NotebookError {
    match error {
        NotebookError::Change { source, .. } => innermost(source),
        other => other,
    }
}

/// Path as a string literal of the language.
pub(crate) fn quoted(path: &AbsPath) -> String {
    let text = path.to_language_string();
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        if matches!(character, '"' | '\\') {
            out.push('\\');
        }
        out.push(character);
    }
    out.push('"');
    out
}

/// The notebook named by `path`.
///
/// # Errors
///
/// Fails with `E_MISSING_PARAM` without a path and with the errors of
/// [`path_arg`].
pub(crate) fn notebook_path(args: &BoundArgs, ctx: &ExecCtx) -> Result<AbsPath> {
    path_arg(args, PATH, ctx)?.ok_or_else(|| missing_param(args, PATH))
}

/// Static facts of a notebook command: the paths it names.
pub(crate) fn static_info(args: &BoundArgs, names: &[&str]) -> StaticInfo {
    StaticInfo {
        paths: names
            .iter()
            .filter_map(|name| args.value(name))
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        ..StaticInfo::default()
    }
}

/// `E_MISSING_PARAM` for a parameter without a value.
pub(crate) fn missing_param(args: &BoundArgs, name: &str) -> Error {
    Error::new(
        ErrorCode::MissingParam,
        format!("`{}` needs a value for `{name}`", args.command()),
    )
}

/// `E_TYPE` for a value of another type.
pub(crate) fn type_error(args: &BoundArgs, name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`{}` expects {expected} for `{name}`", args.command()),
    )
}

/// `E_PARAM_CONFLICT` for two parameters that exclude each other.
pub(crate) fn conflict(args: &BoundArgs, first: &str, second: &str) -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!(
            "`{}` takes `{first}` or `{second}`, not both: remove one of them",
            args.command()
        ),
    )
}

/// Value of an enumeration parameter, one of `values`.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value that is not text and with
/// `E_UNKNOWN_VALUE` for text that is not one of `values`.
pub(crate) fn choice<'v>(
    args: &BoundArgs,
    name: &str,
    values: &[&'v str],
) -> Result<Option<&'v str>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| type_error(args, name, "text"))?;
            values
                .iter()
                .copied()
                .find(|known| *known == text)
                .map(Some)
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::UnknownValue,
                        format!(
                            "`{text}` is not a value of `{name}`: use {}",
                            values.join(", ")
                        ),
                    )
                    .with_found(text)
                })
        }
    }
}

/// Value of a `cell_type` parameter.
///
/// # Errors
///
/// Fails as [`choice`].
pub(crate) fn cell_type_param(args: &BoundArgs, name: &str) -> Result<Option<CellType>> {
    Ok(choice(args, name, &CELL_TYPES)?.map(|name| match name {
        "markdown" => CellType::Markdown,
        "raw" => CellType::Raw,
        _ => CellType::Code,
    }))
}

/// Value of a `range` parameter: a range, a single index or `all`.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value of another type.
pub(crate) fn range_param(args: &BoundArgs, name: &str) -> Result<Option<Range>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Range(range)) => Ok(Some(*range)),
        Some(Value::Int(index)) => Ok(Some(Range::single(*index))),
        Some(value) if value.as_str() == Some(ALL) => Ok(Some(Range::full())),
        Some(_) => Err(type_error(args, name, "a range such as 2, 1..4 or all")),
    }
}

/// Value of an identifier parameter: one identifier or a list of them.
///
/// # Errors
///
/// Fails with `E_TYPE` for a value of another type.
pub(crate) fn ids_param(args: &BoundArgs, name: &str) -> Result<Vec<String>> {
    let expected = "text or a list of text";
    match args.value(name) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Text(id)) => Ok(vec![id.clone()]),
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Text(id) => Ok(id.clone()),
                _ => Err(type_error(args, name, expected)),
            })
            .collect(),
        Some(_) => Err(type_error(args, name, expected)),
    }
}

/// Pattern of a `find` argument.
#[derive(Debug, Clone)]
pub(crate) enum FindPattern {
    /// Literal text.
    Literal(Literal),
    /// Regular expression.
    Regex(RegexPattern),
}

impl FindPattern {
    /// Pattern of a value: literal text or a regular expression.
    ///
    /// # Errors
    ///
    /// Fails with `E_TYPE` for a value of another type and with `E_REGEX`
    /// for an invalid regular expression.
    pub(crate) fn from_value(
        args: &BoundArgs,
        name: &str,
        value: &Value,
        whole_word: bool,
    ) -> Result<Self> {
        match value {
            Value::Text(text) => Ok(Self::Literal(Literal::new(text.clone(), whole_word))),
            Value::Regex(regex) => RegexPattern::new(regex, whole_word).map(Self::Regex),
            _ => Err(type_error(args, name, "text or a regular expression")),
        }
    }

    /// The pattern for the notebook library.
    pub(crate) fn as_pattern(&self) -> &dyn Pattern {
        match self {
            Self::Literal(literal) => literal,
            Self::Regex(regex) => regex,
        }
    }

    /// An owned pattern sharing the state of this one.
    pub(crate) fn boxed(&self) -> Box<dyn Pattern> {
        match self {
            Self::Literal(literal) => Box::new(literal.clone()),
            Self::Regex(regex) => Box::new(regex.clone()),
        }
    }

    /// `E_REGEX` when the replacement of a regular expression was not a
    /// valid template.
    pub(crate) fn template_error(&self) -> Option<Error> {
        match self {
            Self::Literal(_) => None,
            Self::Regex(regex) => regex.template_error(),
        }
    }
}

/// Value of a `find` parameter.
///
/// # Errors
///
/// Fails as [`FindPattern::from_value`].
pub(crate) fn find_param(
    args: &BoundArgs,
    name: &str,
    whole_word: bool,
) -> Result<Option<FindPattern>> {
    match args.value(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => FindPattern::from_value(args, name, value, whole_word).map(Some),
    }
}

/// Regular expression of `find`, in the dialect of `search`.
///
/// Used for replacement, the text of `with` is a template: `$1` or `${1}`
/// inserts a capture group by number, `$0` the whole match and `$$` a `$`.
/// The template is checked against the groups at the first match; an invalid
/// template is reported by [`RegexPattern::template_error`] after the
/// replacement, which the caller then discards.
#[derive(Debug, Clone)]
pub(crate) struct RegexPattern {
    inner: Arc<RegexInner>,
}

#[derive(Debug)]
struct RegexInner {
    written: String,
    searcher: Searcher,
    template: Mutex<TemplateState>,
}

#[derive(Debug)]
enum TemplateState {
    Unparsed,
    Parsed(Template),
    Failed { with: String, error: TemplateError },
}

/// Text of the capture groups of one match; group 0 is the whole match.
struct GroupTexts(Vec<Option<String>>);

impl CaptureSource for GroupTexts {
    fn group(&self, index: usize) -> Option<&str> {
        self.0.get(index)?.as_deref()
    }
}

impl RegexPattern {
    /// Compiles a regular expression of the language.
    ///
    /// # Errors
    ///
    /// Fails with `E_REGEX` for an invalid expression and with `E_LIMIT` for
    /// an expression above the size limit.
    pub(crate) fn new(regex: &value::Regex, whole_word: bool) -> Result<Self> {
        let written = regex.to_string();
        let pattern = SearchPattern::regex_with_flags(regex.pattern.clone(), regex.flags);
        let options = SearchOptions::new().whole_word(whole_word).max_line(0);
        let searcher = Searcher::new(&[pattern], options).map_err(|error| {
            Error::new(
                error.code(),
                format!("pattern {written} is invalid: {error}"),
            )
            .with_found(written.clone())
        })?;
        Ok(Self {
            inner: Arc::new(RegexInner {
                written,
                searcher,
                template: Mutex::new(TemplateState::Unparsed),
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, TemplateState> {
        self.inner
            .template
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Capture groups of the match of `haystack` that starts at `start`.
    fn groups_at(&self, haystack: &str, start: usize) -> Option<GroupTexts> {
        let searcher = &self.inner.searcher;
        let mut groups = None;
        searcher.search_slice(haystack.as_bytes(), |hit| {
            for found in hit.matches() {
                if offset(found.start) == start {
                    groups = self.group_texts(hit, found);
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        });
        groups
    }

    /// Capture groups of every match of `haystack` by the offset where the
    /// match starts, found in one search.
    fn groups_by_start(&self, haystack: &str) -> HashMap<usize, GroupTexts> {
        let mut all = HashMap::new();
        self.inner
            .searcher
            .search_slice(haystack.as_bytes(), |hit| {
                for found in hit.matches() {
                    if let Some(groups) = self.group_texts(hit, found) {
                        all.entry(offset(found.start)).or_insert(groups);
                    }
                }
                ControlFlow::Continue(())
            });
        all
    }

    fn group_texts(&self, hit: &Hit<'_>, found: &Match) -> Option<GroupTexts> {
        self.inner.searcher.captures(hit, found).map(|captures| {
            GroupTexts(
                (0..captures.len())
                    .map(|index| captures.get(index).map(|group| group.text().into_owned()))
                    .collect(),
            )
        })
    }

    /// Replacement for a match with the groups `groups`; the template is
    /// parsed at the first match.
    fn expand_groups(&self, groups: &GroupTexts, with: &str) -> String {
        let mut state = self.state();
        if matches!(*state, TemplateState::Unparsed) {
            let names = Groups::from_names(std::iter::repeat_n(None, groups.0.len()));
            *state = match Template::parse(with, &names) {
                Ok(template) => TemplateState::Parsed(template),
                Err(error) => TemplateState::Failed {
                    with: with.to_owned(),
                    error,
                },
            };
        }
        match &*state {
            TemplateState::Parsed(template) => {
                let mut out = String::new();
                template.expand_into(groups, &mut out);
                out
            }
            TemplateState::Unparsed | TemplateState::Failed { .. } => with.to_owned(),
        }
    }

    /// `E_REGEX` when the replacement used with this pattern is not a valid
    /// template.
    pub(crate) fn template_error(&self) -> Option<Error> {
        match &*self.state() {
            TemplateState::Failed { with, error } => Some(
                Error::new(
                    ErrorCode::Regex,
                    format!(
                        "replacement `{with}` is invalid at column {}: {error}; write `$$` for a literal `$`",
                        error.column()
                    ),
                )
                .with_found(with.clone())
                .with_detail("column", json!(error.column())),
            ),
            TemplateState::Unparsed | TemplateState::Parsed(_) => None,
        }
    }
}

fn offset(position: u64) -> usize {
    usize::try_from(position).unwrap_or(usize::MAX)
}

impl Pattern for RegexPattern {
    fn find_all(&self, haystack: &str) -> Vec<Span<usize>> {
        let mut found = Vec::new();
        self.inner
            .searcher
            .search_slice(haystack.as_bytes(), |hit| {
                found.extend(hit.matches().iter().map(|m| offset(m.start)..offset(m.end)));
                ControlFlow::Continue(())
            });
        found
    }

    fn expand(&self, haystack: &str, found: Span<usize>, with: &str) -> String {
        match self.groups_at(haystack, found.start) {
            Some(groups) => self.expand_groups(&groups, with),
            None => with.to_owned(),
        }
    }

    fn expand_all(&self, haystack: &str, found: &[Span<usize>], with: &str) -> Vec<String> {
        let groups = self.groups_by_start(haystack);
        found
            .iter()
            .map(|span| match groups.get(&span.start) {
                Some(groups) => self.expand_groups(groups, with),
                None => with.to_owned(),
            })
            .collect()
    }

    fn display(&self) -> String {
        self.inner.written.clone()
    }
}

/// `cell` record of a cell of the notebook `path` with the entity tag
/// `etag` of the file.
pub(crate) fn cell_record(path: &AbsPath, etag: &str, data: CellData) -> Cell {
    Cell {
        path: path_field(path),
        index: count(data.index),
        cell_type: data.cell_type,
        source: data.source,
        etag: etag.to_owned(),
        id: data.id,
        outputs: data
            .outputs
            .map(|outputs| outputs.into_iter().map(Value::from_plain_json).collect()),
        outputs_summary: data.outputs_summary,
        execution_count: data.execution_count,
        metadata: data.metadata.map(|metadata| {
            metadata
                .into_iter()
                .map(|(key, item)| (key, Value::from_plain_json(item)))
                .collect()
        }),
    }
}

/// Map value of a new cell: its index and identifier.
pub(crate) fn new_cell_value(index: usize, id: Option<&str>) -> Value {
    let mut fields = Fields::new();
    fields.insert(
        "index".to_owned(),
        Value::Int(i64::try_from(index).unwrap_or(i64::MAX)),
    );
    fields.insert(
        "id".to_owned(),
        id.map_or(Value::Null, |id| Value::Text(id.to_owned())),
    );
    Value::Map(fields)
}

/// Outcome of an applied notebook command: the `effect` record with the
/// changes and counters, and the changes for the report.
pub(crate) fn effect_outcome<'a>(
    command: &str,
    tx: &dyn MutationTx,
    changes: Vec<Change>,
    counters: &[(&str, u64)],
    after: Option<Value>,
) -> EffectOutcome<'a> {
    let mut summary: Vec<(String, u64)> = Vec::with_capacity(counters.len() + 1);
    if !changes.is_empty() {
        summary.push((FILES_MODIFIED.to_owned(), count(changes.len())));
    }
    summary.extend(
        counters
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value)),
    );
    let effect = Effect {
        status: EffectStatus::Ok,
        kind: command.to_owned(),
        op: tx.op().map(|op| op.0),
        targets: Some(count(changes.len())),
        summary: Some(summary.into_iter().collect()),
        changes: Some(changes.clone()),
        after,
        ..Effect::default()
    };
    let mut outcome = EffectOutcome::new(StageOutput::records(RecordStream::once(effect.into())));
    outcome.changes = changes;
    outcome
}

/// `E_STAGE_FAILED` for an apply step without the state of its plan.
pub(crate) fn missing_state(command: &str) -> Error {
    Error::new(
        ErrorCode::StageFailed,
        format!("the plan of `{command}` carries no state: plan the command again"),
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Regex;

    use super::*;
    use crate::commands::notebook::fixture::abs;

    fn args(name: &str, value: Value) -> BoundArgs {
        BoundArgs::new("notebook.cells").with(name, ArgValue::Value(value), false)
    }

    #[test]
    fn out_of_range_indexes_have_their_own_code() {
        let path = abs("/data/a.ipynb");
        let error = notebook_error(&NotebookError::IndexOutOfRange { index: 7, len: 2 }, &path);
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        assert_eq!(error.actual, Some(json!(7)));
        assert_eq!(error.fix, [format!("notebook.cells {}", quoted(&path))]);
        let nested = NotebookError::Change {
            position: 1,
            source: Box::new(NotebookError::IndexOutOfRange { index: 7, len: 2 }),
        };
        let error = notebook_error(&nested, &path);
        assert_eq!(error.code(), ErrorCode::OutOfRange);
        assert_eq!(error.details.get("change"), Some(&json!(1)));
    }

    #[test]
    fn syntax_errors_point_into_the_file() {
        let path = abs("/data/a.ipynb");
        let error = notebook_error(
            &NotebookError::InvalidJson {
                line: 3,
                column: 9,
                detail: "expected value".to_owned(),
            },
            &path,
        );
        assert_eq!(error.code(), ErrorCode::Format);
        let location = error.location.clone().unwrap();
        assert_eq!(location.path, path.to_language_string());
        assert_eq!((location.line, location.column), (Some(3), Some(9)));
        assert!(error.fix.is_empty());
    }

    #[test]
    fn duplicate_ids_suggest_normalize() {
        let path = abs("/data/a.ipynb");
        let error = notebook_error(&NotebookError::DuplicateId { id: "a".to_owned() }, &path);
        assert_eq!(error.code(), ErrorCode::Format);
        assert!(error.fix[0].starts_with("notebook.normalize "));
    }

    #[test]
    fn paths_are_quoted_as_string_literals() {
        let path = abs("/data/say \"hi\".ipynb");
        assert!(quoted(&path).ends_with("say \\\"hi\\\".ipynb\""));
    }

    #[test]
    fn ranges_accept_indexes_ranges_and_all() {
        assert_eq!(
            range_param(&args("index", Value::Int(2)), "index").unwrap(),
            Some(Range::single(2))
        );
        assert_eq!(
            range_param(&args("index", Value::Enum("all".to_owned())), "index").unwrap(),
            Some(Range::full())
        );
        let error = range_param(&args("index", Value::Bool(true)), "index").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }

    #[test]
    fn identifiers_accept_one_or_many() {
        assert_eq!(
            ids_param(&args("id", Value::Text("a".to_owned())), "id").unwrap(),
            ["a"]
        );
        let list = Value::List(vec![
            Value::Text("a".to_owned()),
            Value::Text("b".to_owned()),
        ]);
        assert_eq!(ids_param(&args("id", list), "id").unwrap(), ["a", "b"]);
        let mixed = Value::List(vec![Value::Int(1)]);
        assert!(ids_param(&args("id", mixed), "id").is_err());
    }

    #[test]
    fn unknown_choices_list_the_values() {
        let error = cell_type_param(
            &args("cell_type", Value::Enum("heading".to_owned())),
            "cell_type",
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UnknownValue);
        assert!(
            error.message.contains("code, markdown, raw"),
            "{}",
            error.message
        );
    }

    #[test]
    fn regular_expressions_find_and_expand_groups() {
        let regex = RegexPattern::new(&Regex::new(r"read_(\w+)", "").unwrap(), false).unwrap();
        let haystack = "pd.read_csv(a)\npd.read_json(b)";
        let found = regex.find_all(haystack);
        assert_eq!(found, [3..11, 18..27]);
        assert_eq!(
            regex.expand(haystack, found[1].clone(), "load_$1"),
            "load_json"
        );
        assert!(regex.template_error().is_none());
        assert_eq!(regex.display(), "/read_(\\w+)/");
    }

    #[test]
    fn the_groups_of_all_matches_are_evaluated_in_one_pass() {
        let regex = RegexPattern::new(&Regex::new(r"read_(\w+)", "").unwrap(), false).unwrap();
        let haystack = "read_a(); read_b(); read_c();";
        let found = regex.find_all(haystack);
        let all = regex.expand_all(haystack, &found, "load_$1");
        assert_eq!(all, ["load_a", "load_b", "load_c"]);
        let one_by_one: Vec<String> = found
            .iter()
            .map(|span| regex.expand(haystack, span.clone(), "load_$1"))
            .collect();
        assert_eq!(all, one_by_one);
    }

    #[test]
    fn many_matches_expand_without_a_search_per_match() {
        let regex = RegexPattern::new(&Regex::new(r"(\d)x", "").unwrap(), false).unwrap();
        let haystack = "1x ".repeat(20_000);
        let found = regex.find_all(&haystack);
        let expanded = regex.expand_all(&haystack, &found, "[$1]");
        assert_eq!(expanded.len(), 20_000);
        assert!(expanded.iter().all(|text| text == "[1]"));
    }

    #[test]
    fn invalid_templates_are_reported() {
        let regex = RegexPattern::new(&Regex::new(r"(a)", "").unwrap(), false).unwrap();
        let _ = regex.expand("a", 0..1, "$2");
        let error = regex.template_error().unwrap();
        assert_eq!(error.code(), ErrorCode::Regex);
        assert_eq!(error.found.as_deref(), Some("$2"));
    }

    #[test]
    fn invalid_regular_expressions_fail_with_regex() {
        let error = RegexPattern::new(&Regex::new("a(b", "").unwrap(), false).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Regex);
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! Values shared by the unit tests of the notebook commands.

    use nurt_contracts::fs::AbsPath;
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;

    /// Absolute path of the virtual file system for a path written with `/`.
    pub(crate) fn abs(text: &str) -> AbsPath {
        test_path(text).expect("test paths are absolute")
    }

    /// Test context whose file system holds one file.
    pub(crate) fn with_file(path: &str, content: &[u8]) -> (TestContext, AbsPath) {
        let test = TestContext::new();
        let path = abs(path);
        test.fs()
            .write_file(&path, content)
            .expect("the virtual file system accepts the file");
        (test, path)
    }
}
