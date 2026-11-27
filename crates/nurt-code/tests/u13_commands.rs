// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the navigation and diagnostics stages of the code group:
//! `code.definition`, `code.references`, `code.hover`,
//! `code.implementations`, `code.type_definition`, `code.call_hierarchy`,
//! `code.actions` and `code.diagnostics`.
//!
//! A scripted semantic provider stands for the language server, and a small
//! code service for indented Python text stands for syntax analysis. Every
//! test works on a project of three Python files in a temporary directory.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
// The fixtures and the shared test doubles fail the test on a setup error
// by unwrapping or panicking; clippy exempts only `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod u14_support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nurt_code::commands::actions::CodeActions;
use nurt_code::commands::call_hierarchy::CodeCallHierarchy;
use nurt_code::commands::definition::CodeDefinition;
use nurt_code::commands::diagnostics::CodeDiagnostics;
use nurt_code::commands::hover::CodeHover;
use nurt_code::commands::implementations::CodeImplementations;
use nurt_code::commands::references::CodeReferences;
use nurt_code::commands::type_definition::CodeTypeDefinition;
use nurt_code::semantic::{
    CallDirection, CallEdge, CodeActionInfo, DocumentPosition, HoverInfo, Location,
    SemanticDiagnostic, SemanticError, SemanticErrorKind, SemanticProvider, SemanticResult,
    SemanticScope, SemanticSymbol, ServerStatus, WorkspaceChange,
};
use nurt_contracts::config::Config;
use nurt_contracts::envelope::{RequestOptions, Verbosity};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{
    CodeAction, Diagnostic, DiagnosticSeverity, Hover, Reference, ReferenceRole, Symbol,
    SymbolKind as RecordKind, SymbolSource,
};
use nurt_contracts::report::Notable;
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, CodeService, FileHistoryStat, FunctionMetrics, GitPathStatus,
    GitQueryService, LanguageId, LanguageInfo, Services, SymbolInfo, SymbolKind, SymbolReference,
    SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken, TextPosition, TextSpan,
};
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CancelReason, ExecCtx, RecordStream, StageImpl, StageInput,
};
use nurt_contracts::value::{Record, Value};

use u14_support::{StdFs, TestClock, TestHost, ToyCode};

const SHAPES: &str = "class Shape:
    def area(self):
        return 0


class Circle(Shape):
    def area(self):
        return scale(3)


def scale(value):
    \"\"\"Double the value. Used by total and run.\"\"\"
    return value * 2


def total(items):
    return scale(len(items)) + scale(1)
";

const MAIN: &str = "from shapes import scale


def run():
    return scale(5)
";

const TEST_SHAPES: &str = "from shapes import scale


def test_scale():
    assert scale(2) == 4
";

/// Project with `main.py`, `shapes.py` and `tests/test_shapes.py` under a
/// directory with a `.git` marker.
struct Project {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("shapes.py"), SHAPES).unwrap();
        std::fs::write(root.join("main.py"), MAIN).unwrap();
        std::fs::write(root.join("tests/test_shapes.py"), TEST_SHAPES).unwrap();
        Self { _dir: dir, root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Path of a record relative to the project root.
    fn relative(&self, path: &str) -> String {
        let root = format!("{}/", self.root.display());
        path.strip_prefix(&root).unwrap_or(path).to_owned()
    }
}

fn context(options: RequestOptions) -> ExecCtx {
    ExecCtx::builder(Arc::new(TestClock), Arc::new(TestHost::new()))
        .fs(Arc::new(StdFs))
        .services(Arc::new(Services::new().with_code(Arc::new(PyCode))))
        .config(Arc::new(Config::default()))
        .options(Arc::new(options))
        .build()
}

fn plain_context() -> ExecCtx {
    context(RequestOptions::default())
}

fn args(command: &str, pairs: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(command);
    for (name, value) in pairs {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

fn path_value(path: &Path) -> Value {
    Value::Path(path.to_string_lossy().into_owned())
}

fn word(text: &str) -> Value {
    Value::Text(text.to_owned())
}

fn run_with(
    stage: &dyn StageImpl,
    args: &BoundArgs,
    input: StageInput<'_>,
    ctx: &ExecCtx,
) -> Result<Vec<Record>> {
    let output = stage.as_read().unwrap().execute(args, input, ctx)?;
    StageInput::from(output)
        .into_records()?
        .collect::<Result<Vec<_>>>()
}

fn run(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    run_with(stage, args, StageInput::none(), ctx)
}

fn failure(result: Result<Vec<Record>>) -> Error {
    match result {
        Ok(records) => panic!("expected an error, got {} records", records.len()),
        Err(error) => error,
    }
}

fn fallback_details(ctx: &ExecCtx) -> Vec<String> {
    ctx.take_notes()
        .into_iter()
        .filter(|entry| entry.code == Notable::SemanticFallback)
        .map(|entry| entry.detail.unwrap_or_default())
        .collect()
}

fn position(line: u32, column: u32) -> TextPosition {
    TextPosition { line, column }
}

fn location(path: &Path, line: u32, column: u32, length: u32) -> Location {
    Location {
        path: path.to_path_buf(),
        start: position(line, column),
        end: position(line, column + length),
    }
}

fn symbols(records: Vec<Record>) -> Vec<Symbol> {
    records
        .into_iter()
        .map(|record| Symbol::try_from(record).unwrap())
        .collect()
}

fn references(records: Vec<Record>) -> Vec<Reference> {
    records
        .into_iter()
        .map(|record| Reference::try_from(record).unwrap())
        .collect()
}

fn unavailable() -> SemanticError {
    SemanticError::new(SemanticErrorKind::Unavailable, "no server for the test")
}

// Code service for indented Python text. Definitions start with `def` or
// `class`; a definition ends before the next line indented no deeper than
// its own line. The other operations come from the code service of the
// rename tests.

struct PyCode;

struct Definition {
    start: usize,
    name_start: usize,
    name_end: usize,
    end: usize,
    indent: usize,
    class: bool,
    signature: String,
}

fn text_position(text: &str, offset: usize) -> TextPosition {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |found| found + 1);
    position(
        u32::try_from(before.matches('\n').count() + 1).unwrap(),
        u32::try_from(text[line_start..offset].chars().count() + 1).unwrap(),
    )
}

fn text_span(text: &str, start: usize, end: usize) -> TextSpan {
    TextSpan {
        bytes: ByteRange::new(u64::try_from(start).unwrap(), u64::try_from(end).unwrap()),
        start: text_position(text, start),
        end: text_position(text, end),
    }
}

fn scan_definitions(text: &str) -> Vec<Definition> {
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        lines.push((offset, line));
        offset += line.len() + 1;
    }
    let mut found = Vec::new();
    for (index, (offset, line)) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let Some(keyword) = ["def ", "class "]
            .into_iter()
            .find(|keyword| trimmed.starts_with(keyword))
        else {
            continue;
        };
        let rest = &trimmed[keyword.len()..];
        let name_length = rest
            .find(|character: char| !(character.is_alphanumeric() || character == '_'))
            .unwrap_or(rest.len());
        let mut end = offset + line.len();
        for (next_offset, next) in &lines[index + 1..] {
            if next.trim().is_empty() {
                continue;
            }
            if next.len() - next.trim_start().len() <= indent {
                break;
            }
            end = next_offset + next.len();
        }
        let name_start = offset + indent + keyword.len();
        found.push(Definition {
            start: offset + indent,
            name_start,
            name_end: name_start + name_length,
            end,
            indent,
            class: keyword == "class ",
            signature: trimmed.trim_end().trim_end_matches(':').to_owned(),
        });
    }
    found
}

fn python_definitions(text: &str) -> Vec<SymbolInfo> {
    let definitions = scan_definitions(text);
    definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| {
            let ancestors: Vec<&Definition> = definitions[..index]
                .iter()
                .filter(|outer| outer.indent < definition.indent && outer.end >= definition.start)
                .collect();
            let names: Vec<&str> = ancestors
                .iter()
                .map(|outer| &text[outer.name_start..outer.name_end])
                .collect();
            let kind = match (definition.class, ancestors.last()) {
                (true, _) => SymbolKind::Class,
                (false, Some(outer)) if outer.class => SymbolKind::Method,
                (false, _) => SymbolKind::Function,
            };
            SymbolInfo {
                name: text[definition.name_start..definition.name_end].to_owned(),
                kind,
                container: (!names.is_empty()).then(|| names.join(".")),
                depth: u32::try_from(ancestors.len()).unwrap(),
                span: text_span(text, definition.start, definition.end),
                name_span: text_span(text, definition.name_start, definition.name_end),
                signature: Some(definition.signature.clone()),
                exported: None,
            }
        })
        .collect()
}

fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|_| Error::new(ErrorCode::Encoding, "not UTF-8"))
}

impl CodeService for PyCode {
    fn languages(&self) -> Vec<LanguageInfo> {
        ToyCode.languages()
    }

    fn language_of(&self, path: Option<&Path>, bytes: &[u8]) -> Option<LanguageId> {
        ToyCode.language_of(path, bytes)
    }

    fn outline(
        &self,
        bytes: &[u8],
        _language: &LanguageId,
        _depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        Ok(python_definitions(utf8(bytes)?))
    }

    fn find_symbol(
        &self,
        bytes: &[u8],
        _language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        let (container, last) = match name.rsplit_once('.') {
            Some((container, last)) => (Some(container), last),
            None => (None, name),
        };
        Ok(python_definitions(utf8(bytes)?)
            .into_iter()
            .filter(|info| info.name == last)
            .filter(|info| container.is_none_or(|wanted| info.container.as_deref() == Some(wanted)))
            .collect())
    }

    fn enclosing(
        &self,
        bytes: &[u8],
        _language: &LanguageId,
        offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        Ok(python_definitions(utf8(bytes)?)
            .into_iter()
            .filter(|info| info.span.bytes.start <= offset && offset < info.span.bytes.end)
            .max_by_key(|info| info.depth))
    }

    fn classify_spans(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        ToyCode.classify_spans(bytes, language)
    }

    fn check_syntax(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        ToyCode.check_syntax(bytes, language)
    }

    fn compile_pattern(
        &self,
        pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        ToyCode.compile_pattern(pattern, language)
    }

    fn syntax_search(&self, bytes: &[u8], pattern: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>> {
        ToyCode.syntax_search(bytes, pattern)
    }

    fn render_template(&self, found: &SyntaxMatch, template: &str) -> Result<String> {
        ToyCode.render_template(found, template)
    }

    fn references(
        &self,
        bytes: &[u8],
        language: &LanguageId,
        name: &str,
    ) -> Result<Vec<SymbolReference>> {
        ToyCode.references(bytes, language, name)
    }

    fn function_metrics(
        &self,
        bytes: &[u8],
        language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        ToyCode.function_metrics(bytes, language)
    }

    fn tokens(&self, bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        ToyCode.tokens(bytes, language)
    }
}

/// Semantic provider with one scripted answer per request; it keeps the
/// positions, ranges and directions it was asked for.
struct Script {
    definition: SemanticResult<Vec<Location>>,
    references: SemanticResult<Vec<Location>>,
    hover: SemanticResult<Option<HoverInfo>>,
    implementations: SemanticResult<Vec<Location>>,
    type_definition: SemanticResult<Vec<Location>>,
    calls: SemanticResult<Vec<CallEdge>>,
    actions: SemanticResult<Vec<CodeActionInfo>>,
    diagnostics: SemanticResult<Vec<SemanticDiagnostic>>,
    asked_at: Mutex<Vec<DocumentPosition>>,
    asked_ranges: Mutex<Vec<Location>>,
    asked_directions: Mutex<Vec<CallDirection>>,
    asked_paths: Mutex<Vec<PathBuf>>,
}

impl Script {
    fn unavailable() -> Self {
        Self {
            definition: Err(unavailable()),
            references: Err(unavailable()),
            hover: Err(unavailable()),
            implementations: Err(unavailable()),
            type_definition: Err(unavailable()),
            calls: Err(unavailable()),
            actions: Err(unavailable()),
            diagnostics: Err(unavailable()),
            asked_at: Mutex::new(Vec::new()),
            asked_ranges: Mutex::new(Vec::new()),
            asked_directions: Mutex::new(Vec::new()),
            asked_paths: Mutex::new(Vec::new()),
        }
    }

    fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    fn remember(&self, at: &DocumentPosition) {
        self.asked_at.lock().unwrap().push(at.clone());
    }
}

impl SemanticProvider for Script {
    fn workspace_symbols(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _query: &str,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        Err(unavailable())
    }

    fn document_symbols(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _path: &Path,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        Err(unavailable())
    }

    fn definition(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        self.remember(at);
        self.definition.clone()
    }

    fn references(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        at: &DocumentPosition,
        _include_definition: bool,
    ) -> SemanticResult<Vec<Location>> {
        self.remember(at);
        self.references.clone()
    }

    fn hover(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Option<HoverInfo>> {
        self.remember(at);
        self.hover.clone()
    }

    fn implementations(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        self.remember(at);
        self.implementations.clone()
    }

    fn type_definition(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        self.remember(at);
        self.type_definition.clone()
    }

    fn call_hierarchy(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        at: &DocumentPosition,
        direction: CallDirection,
    ) -> SemanticResult<Vec<CallEdge>> {
        self.remember(at);
        self.asked_directions.lock().unwrap().push(direction);
        self.calls.clone()
    }

    fn code_actions(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        range: &Location,
    ) -> SemanticResult<Vec<CodeActionInfo>> {
        self.asked_ranges.lock().unwrap().push(range.clone());
        self.actions.clone()
    }

    fn diagnostics(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        path: &Path,
        _text: Option<&str>,
    ) -> SemanticResult<Vec<SemanticDiagnostic>> {
        self.asked_paths.lock().unwrap().push(path.to_path_buf());
        self.diagnostics.clone().map(|items| {
            items
                .into_iter()
                .map(|mut item| {
                    item.range.path = path.to_path_buf();
                    item
                })
                .collect()
        })
    }

    fn rename(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _new_name: &str,
    ) -> SemanticResult<WorkspaceChange> {
        Err(unavailable())
    }

    fn server_status(&self, _ctx: &ExecCtx, _language: Option<&LanguageId>) -> Vec<ServerStatus> {
        Vec::new()
    }
}

/// Repository queries that report a fixed list of changed files.
struct FixedGit {
    root: PathBuf,
    changed: Vec<String>,
}

impl GitQueryService for FixedGit {
    fn repo_root(&self, path: &Path) -> Result<Option<PathBuf>> {
        Ok(path.starts_with(&self.root).then(|| self.root.clone()))
    }

    fn status(&self, _root: &Path, _paths: &[String]) -> Result<Vec<GitPathStatus>> {
        Ok(Vec::new())
    }

    fn changed_files(&self, _root: &Path, _against: Option<&str>) -> Result<Vec<String>> {
        Ok(self.changed.clone())
    }

    fn tracked_files(&self, _root: &Path) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn is_ignored(&self, _root: &Path, _path: &str) -> Result<bool> {
        Ok(false)
    }

    fn file_history(
        &self,
        _root: &Path,
        _since_days: u32,
        _limit: u32,
    ) -> Result<Vec<FileHistoryStat>> {
        Ok(Vec::new())
    }
}

fn definition_args(project: &Project, file: &str, extra: &[(&str, Value)]) -> BoundArgs {
    let mut pairs = vec![("path", path_value(&project.path(file)))];
    pairs.extend(extra.iter().cloned());
    args("code.definition", &pairs)
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn definition_answers_from_the_server() {
    let project = Project::new();
    let shapes = project.path("shapes.py");
    let script = Script {
        definition: Ok(vec![location(&shapes, 11, 5, 5)]),
        ..Script::unavailable()
    }
    .shared();
    let ctx = plain_context();
    let stage = CodeDefinition::new(Some(script.clone()));
    let records = run(
        &stage,
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("column", Value::Int(12))],
        ),
        &ctx,
    )
    .unwrap();
    let found = symbols(records);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "scale");
    assert_eq!(found[0].kind, RecordKind::Function);
    assert_eq!(found[0].source, SymbolSource::Lsp);
    assert_eq!(
        (found[0].line, found[0].column, found[0].end_line),
        (11, 5, 13)
    );
    assert_eq!(found[0].signature.as_deref(), Some("def scale(value)"));
    assert_eq!(project.relative(found[0].path.as_str()), "shapes.py");
    assert!(found[0].etag.starts_with("sha256:"), "{}", found[0].etag);
    assert_eq!(script.asked_at.lock().unwrap()[0].position, position(5, 12));
    assert!(fallback_details(&ctx).is_empty());
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn definition_falls_back_to_syntax_with_the_cause_when_the_server_is_unavailable() {
    let project = Project::new();
    let ctx = plain_context();
    let stage = CodeDefinition::new(Some(Script::unavailable().shared()));
    let records = run(
        &stage,
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("column", Value::Int(12))],
        ),
        &ctx,
    )
    .unwrap();
    let found = symbols(records);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].source, SymbolSource::Syntax);
    assert_eq!(project.relative(found[0].path.as_str()), "shapes.py");
    assert_eq!(found[0].line, 11);
    let details = fallback_details(&ctx);
    assert_eq!(details.len(), 1);
    assert!(details[0].contains("no server for the test"), "{details:?}");
}

#[test]
fn definition_without_a_provider_uses_syntax_and_notes_the_cause() {
    let project = Project::new();
    let ctx = plain_context();
    let records = run(
        &CodeDefinition::new(None),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("column", Value::Int(12))],
        ),
        &ctx,
    )
    .unwrap();
    assert_eq!(symbols(records)[0].source, SymbolSource::Syntax);
    assert_eq!(
        fallback_details(&ctx),
        vec!["language servers are not available".to_owned()]
    );
}

#[test]
fn definition_replaces_an_empty_server_answer_by_syntax_analysis() {
    let project = Project::new();
    let script = Script {
        definition: Ok(Vec::new()),
        ..Script::unavailable()
    }
    .shared();
    let ctx = plain_context();
    let records = run(
        &CodeDefinition::new(Some(script)),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("column", Value::Int(12))],
        ),
        &ctx,
    )
    .unwrap();
    assert_eq!(symbols(records)[0].source, SymbolSource::Syntax);
    assert_eq!(
        fallback_details(&ctx),
        vec!["the language server found no definition".to_owned()]
    );
}

#[test]
fn definition_keeps_an_empty_server_answer_under_source_lsp() {
    let project = Project::new();
    let script = Script {
        definition: Ok(Vec::new()),
        ..Script::unavailable()
    }
    .shared();
    let records = run(
        &CodeDefinition::new(Some(script)),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("source", word("lsp"))],
        ),
        &plain_context(),
    )
    .unwrap();
    assert!(records.is_empty());
}

#[test]
fn source_lsp_without_a_provider_is_external_missing() {
    let project = Project::new();
    let error = failure(run(
        &CodeDefinition::new(None),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("source", word("lsp"))],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn source_lsp_with_a_missing_server_keeps_the_installation_hint() {
    let project = Project::new();
    let script = Script {
        definition: Err(SemanticError::new(
            SemanticErrorKind::Unavailable,
            "pyright-langserver is not installed",
        )
        .with_server("pyright-langserver")
        .with_fix("npm install --global pyright")),
        ..Script::unavailable()
    }
    .shared();
    let error = failure(run(
        &CodeDefinition::new(Some(script)),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("source", word("lsp"))],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    assert_eq!(
        error.data().fix,
        vec!["npm install --global pyright".to_owned()]
    );
    assert_eq!(error.data().found.as_deref(), Some("pyright-langserver"));
}

#[test]
fn source_lsp_with_a_failing_server_is_external_failed() {
    let project = Project::new();
    let script = Script {
        definition: Err(SemanticError::new(
            SemanticErrorKind::Failed,
            "the server ended: Traceback: ImportError",
        )),
        ..Script::unavailable()
    }
    .shared();
    let error = failure(run(
        &CodeDefinition::new(Some(script)),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("source", word("lsp"))],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalFailed);
    assert!(error.data().message.contains("ImportError"));
}

#[test]
fn a_cancelled_request_is_not_replaced_by_syntax_analysis() {
    let project = Project::new();
    let script = Script {
        definition: Err(SemanticError::new(
            SemanticErrorKind::Cancelled,
            "cancelled",
        )),
        ..Script::unavailable()
    }
    .shared();
    let error = failure(run(
        &CodeDefinition::new(Some(script)),
        &definition_args(&project, "main.py", &[("line", Value::Int(5))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

#[test]
fn source_syntax_does_not_ask_the_server() {
    let project = Project::new();
    let script = Script::unavailable().shared();
    let ctx = plain_context();
    let records = run(
        &CodeDefinition::new(Some(script.clone())),
        &definition_args(
            &project,
            "main.py",
            &[
                ("line", Value::Int(5)),
                ("column", Value::Int(12)),
                ("source", word("syntax")),
            ],
        ),
        &ctx,
    )
    .unwrap();
    assert_eq!(symbols(records).len(), 1);
    assert!(script.asked_at.lock().unwrap().is_empty());
    assert!(fallback_details(&ctx).is_empty());
}

#[test]
fn an_unknown_source_is_rejected() {
    let project = Project::new();
    let error = failure(run(
        &CodeDefinition::new(None),
        &definition_args(
            &project,
            "main.py",
            &[("line", Value::Int(5)), ("source", word("guess"))],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn a_symbol_qualified_by_its_container_selects_the_method() {
    let project = Project::new();
    let shapes = project.path("shapes.py");
    let script = Script {
        definition: Ok(vec![location(&shapes, 7, 9, 4)]),
        ..Script::unavailable()
    }
    .shared();
    let records = run(
        &CodeDefinition::new(Some(script.clone())),
        &definition_args(&project, "shapes.py", &[("symbol", word("Circle.area"))]),
        &plain_context(),
    )
    .unwrap();
    assert_eq!(script.asked_at.lock().unwrap()[0].position, position(7, 9));
    let found = symbols(records);
    assert_eq!(found[0].kind, RecordKind::Method);
    assert_eq!(found[0].container.as_deref(), Some("Circle"));
}

#[test]
fn an_ambiguous_symbol_lists_the_candidates() {
    let project = Project::new();
    let error = failure(run(
        &CodeDefinition::new(None),
        &definition_args(&project, "shapes.py", &[("symbol", word("area"))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::SymbolAmbiguous);
    assert_eq!(
        error.data().sample,
        vec![
            "method Shape.area line 2".to_owned(),
            "method Circle.area line 7".to_owned()
        ]
    );
}

#[test]
fn an_unknown_symbol_is_symbol_not_found() {
    let project = Project::new();
    let error = failure(run(
        &CodeDefinition::new(None),
        &definition_args(&project, "shapes.py", &[("symbol", word("volume"))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::SymbolNotFound);
}

#[test]
fn a_position_needs_a_line_or_a_symbol() {
    let project = Project::new();
    let error = failure(run(
        &CodeDefinition::new(None),
        &definition_args(&project, "shapes.py", &[]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::MissingParam);
    let error = failure(run(
        &CodeDefinition::new(None),
        &definition_args(&project, "shapes.py", &[("line", Value::Int(400))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn a_relative_path_without_a_base_directory_is_rejected() {
    let error = failure(run(
        &CodeDefinition::new(None),
        &args(
            "code.definition",
            &[
                ("path", Value::Path("main.py".to_owned())),
                ("line", Value::Int(1)),
            ],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::Arg);
}

#[test]
fn definition_record_ends_at_the_end_of_the_whole_definition() {
    let project = Project::new();
    let shapes = project.path("shapes.py");
    let script = Script {
        definition: Ok(vec![location(&shapes, 11, 5, 5)]),
        ..Script::unavailable()
    }
    .shared();
    let position_args = definition_args(
        &project,
        "main.py",
        &[("line", Value::Int(5)), ("column", Value::Int(12))],
    );
    let served = symbols(
        run(
            &CodeDefinition::new(Some(script)),
            &position_args,
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!((served[0].end_line, served[0].end_column), (13, Some(21)));
    let local = symbols(run(&CodeDefinition::new(None), &position_args, &plain_context()).unwrap());
    assert_eq!((local[0].end_line, local[0].end_column), (13, Some(21)));
    assert_eq!((local[0].line, local[0].column), (11, 5));
}

fn references_args(project: &Project, extra: &[(&str, Value)]) -> BoundArgs {
    let mut pairs = vec![
        ("path", path_value(&project.path("shapes.py"))),
        ("symbol", word("scale")),
    ];
    pairs.extend(extra.iter().cloned());
    args("code.references", &pairs)
}

fn role_list(project: &Project, found: &[Reference]) -> Vec<(String, u64, u64, ReferenceRole)> {
    found
        .iter()
        .map(|item| {
            (
                project.relative(item.path.as_str()),
                item.line,
                item.column,
                item.role,
            )
        })
        .collect()
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn references_from_syntax_analysis_carry_roles_in_path_order() {
    let project = Project::new();
    let ctx = plain_context();
    let found = references(
        run(
            &CodeReferences::new(None),
            &references_args(&project, &[]),
            &ctx,
        )
        .unwrap(),
    );
    let expected = vec![
        ("main.py".to_owned(), 1, 20, ReferenceRole::Import),
        ("main.py".to_owned(), 5, 12, ReferenceRole::Reference),
        ("shapes.py".to_owned(), 8, 16, ReferenceRole::Reference),
        ("shapes.py".to_owned(), 11, 5, ReferenceRole::Definition),
        ("shapes.py".to_owned(), 17, 12, ReferenceRole::Reference),
        ("shapes.py".to_owned(), 17, 32, ReferenceRole::Reference),
        (
            "tests/test_shapes.py".to_owned(),
            1,
            20,
            ReferenceRole::Import,
        ),
        (
            "tests/test_shapes.py".to_owned(),
            5,
            12,
            ReferenceRole::Test,
        ),
    ];
    assert_eq!(role_list(&project, &found), expected);
    assert!(
        found
            .iter()
            .all(|item| item.source == SymbolSource::Syntax && item.name == "scale")
    );
    let total = found.iter().find(|item| item.line == 17).unwrap();
    assert_eq!(total.container.as_deref(), None);
    assert_eq!(
        total.text.as_deref(),
        Some("return scale(len(items)) + scale(1)")
    );
    assert_eq!(fallback_details(&ctx).len(), 1);
}

fn context_with_config(config: Config) -> ExecCtx {
    ExecCtx::builder(Arc::new(TestClock), Arc::new(TestHost::new()))
        .fs(Arc::new(StdFs))
        .services(Arc::new(Services::new().with_code(Arc::new(PyCode))))
        .config(Arc::new(config))
        .options(Arc::new(RequestOptions::default()))
        .build()
}

fn unavailable_source_notes(ctx: &ExecCtx) -> Vec<String> {
    ctx.take_notes()
        .into_iter()
        .filter(|entry| entry.code == Notable::SourceUnavailable)
        .map(|entry| entry.detail.unwrap_or_default())
        .collect()
}

#[test]
fn project_listing_cut_by_the_file_limit_is_noted() {
    let project = Project::new();
    let mut config = Config::default();
    config.code.index_max_files = 2;
    let ctx = context_with_config(config);
    let found = references(
        run(
            &CodeReferences::new(None),
            &references_args(&project, &[]),
            &ctx,
        )
        .unwrap(),
    );
    assert!(!found.is_empty());
    let notes = unavailable_source_notes(&ctx);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("code.index_max_files (2)"), "{notes:?}");
    let complete = plain_context();
    run(
        &CodeReferences::new(None),
        &references_args(&project, &[]),
        &complete,
    )
    .unwrap();
    assert!(unavailable_source_notes(&complete).is_empty());
}

#[test]
fn references_leave_out_the_definition_on_request() {
    let project = Project::new();
    let found = references(
        run(
            &CodeReferences::new(None),
            &references_args(&project, &[("include_definition", Value::Bool(false))]),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 7);
    assert!(
        found
            .iter()
            .all(|item| item.role != ReferenceRole::Definition)
    );
}

#[test]
fn references_with_file_scope_stay_in_the_target_file() {
    let project = Project::new();
    let found = references(
        run(
            &CodeReferences::new(None),
            &references_args(&project, &[("scope", word("file"))]),
            &plain_context(),
        )
        .unwrap(),
    );
    let lines: Vec<(u64, u64)> = found.iter().map(|item| (item.line, item.column)).collect();
    assert_eq!(lines, vec![(8, 16), (11, 5), (17, 12), (17, 32)]);
    let error = failure(run(
        &CodeReferences::new(None),
        &references_args(&project, &[("scope", word("everywhere"))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn references_from_the_server_are_classified_and_deduplicated() {
    let project = Project::new();
    let main = project.path("main.py");
    let shapes = project.path("shapes.py");
    let test = project.path("tests/test_shapes.py");
    let script = Script {
        references: Ok(vec![
            location(&test, 5, 12, 5),
            location(&shapes, 11, 5, 5),
            location(&main, 1, 20, 5),
            location(&shapes, 8, 16, 5),
            location(&shapes, 8, 16, 5),
        ]),
        ..Script::unavailable()
    }
    .shared();
    let found = references(
        run(
            &CodeReferences::new(Some(script)),
            &references_args(&project, &[]),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(
        role_list(&project, &found),
        vec![
            ("main.py".to_owned(), 1, 20, ReferenceRole::Import),
            ("shapes.py".to_owned(), 8, 16, ReferenceRole::Reference),
            ("shapes.py".to_owned(), 11, 5, ReferenceRole::Definition),
            (
                "tests/test_shapes.py".to_owned(),
                5,
                12,
                ReferenceRole::Test
            ),
        ]
    );
    assert!(found.iter().all(|item| item.source == SymbolSource::Lsp));
    assert_eq!(found[1].container.as_deref(), Some("Circle.area"));
    assert_eq!(found[3].container.as_deref(), Some("test_scale"));
}

#[test]
fn references_in_stage_form_take_symbol_records_as_targets() {
    let project = Project::new();
    let ctx = plain_context();
    let definitions = run(
        &CodeDefinition::new(None),
        &definition_args(
            &project,
            "shapes.py",
            &[("symbol", word("total")), ("source", word("syntax"))],
        ),
        &ctx,
    )
    .unwrap();
    assert_eq!(definitions.len(), 1);
    let input = StageInput::records(RecordStream::from_records(definitions));
    let found = references(
        run_with(
            &CodeReferences::new(None),
            &args("code.references", &[("source", word("syntax"))]),
            input,
            &ctx,
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].role, ReferenceRole::Definition);
    assert_eq!(found[0].name, "total");
}

fn hover_args(project: &Project, extra: &[(&str, Value)]) -> BoundArgs {
    let mut pairs = vec![
        ("path", path_value(&project.path("main.py"))),
        ("line", Value::Int(5)),
        ("column", Value::Int(12)),
    ];
    pairs.extend(extra.iter().cloned());
    args("code.hover", &pairs)
}

const HOVER_TEXT: &str = "```python\ndef scale(\n    value,\n) -> int\n```\n---\nDouble the value. Used by total and run.";

#[test]
fn hover_shortens_the_server_answer_to_signature_and_first_sentence() {
    let project = Project::new();
    let script = Script {
        hover: Ok(Some(HoverInfo {
            contents: HOVER_TEXT.to_owned(),
            range: None,
        })),
        ..Script::unavailable()
    }
    .shared();
    let records = run(
        &CodeHover::new(Some(script)),
        &hover_args(&project, &[]),
        &plain_context(),
    )
    .unwrap();
    let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
    assert_eq!(hover.source, SymbolSource::Lsp);
    assert_eq!(hover.signature.as_deref(), Some("def scale(value) -> int"));
    assert_eq!(hover.documentation.as_deref(), Some("Double the value."));
    assert_eq!((hover.line, hover.column), (5, 12));
}

#[test]
fn hover_with_detailed_verbosity_keeps_the_whole_answer() {
    let project = Project::new();
    let script = Script {
        hover: Ok(Some(HoverInfo {
            contents: HOVER_TEXT.to_owned(),
            range: None,
        })),
        ..Script::unavailable()
    }
    .shared();
    let ctx = context(RequestOptions {
        verbosity: Some(Verbosity::Detailed),
        ..RequestOptions::default()
    });
    let records = run(
        &CodeHover::new(Some(script)),
        &hover_args(&project, &[]),
        &ctx,
    )
    .unwrap();
    let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
    assert_eq!(hover.signature.as_deref(), Some("def scale(value) -> int"));
    assert_eq!(hover.documentation.as_deref(), Some(HOVER_TEXT));
}

#[test]
fn hover_from_syntax_reads_the_docstring_of_the_definition() {
    let project = Project::new();
    let ctx = plain_context();
    let records = run(
        &CodeHover::new(Some(Script::unavailable().shared())),
        &hover_args(&project, &[]),
        &ctx,
    )
    .unwrap();
    let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
    assert_eq!(hover.source, SymbolSource::Syntax);
    assert_eq!(hover.signature.as_deref(), Some("def scale(value)"));
    assert_eq!(hover.documentation.as_deref(), Some("Double the value."));
    assert_eq!(fallback_details(&ctx).len(), 1);
}

#[test]
fn hover_without_server_information_falls_back_or_stays_empty_under_lsp() {
    let project = Project::new();
    let script = Script {
        hover: Ok(None),
        ..Script::unavailable()
    }
    .shared();
    let ctx = plain_context();
    let records = run(
        &CodeHover::new(Some(script.clone())),
        &hover_args(&project, &[]),
        &ctx,
    )
    .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        fallback_details(&ctx),
        vec!["the language server has no information".to_owned()]
    );
    let records = run(
        &CodeHover::new(Some(script)),
        &hover_args(&project, &[("source", word("lsp"))]),
        &plain_context(),
    )
    .unwrap();
    assert!(records.is_empty());
}

#[test]
fn hover_without_a_code_block_takes_the_first_line_as_the_signature() {
    let project = Project::new();
    let script = Script {
        hover: Ok(Some(HoverInfo {
            contents: "\n  def scale(value)  \n\nDouble the value.\nUsed by total.".to_owned(),
            range: None,
        })),
        ..Script::unavailable()
    }
    .shared();
    let records = run(
        &CodeHover::new(Some(script)),
        &hover_args(&project, &[]),
        &plain_context(),
    )
    .unwrap();
    let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
    assert_eq!(hover.signature.as_deref(), Some("def scale(value)"));
    assert_eq!(hover.documentation.as_deref(), Some("Double the value."));
}

/// Module whose function is documented by the comment lines above it.
const GEOMETRY: &str = "LIMIT = 3
# Triple the value. Kept for older callers.
# Prefer scale.
def triple(value):
    return value * 3


def use():
    return triple(2)
";

#[test]
fn hover_from_syntax_reads_the_comment_above_the_definition() {
    let project = Project::new();
    let geometry = project.path("geometry.py");
    std::fs::write(&geometry, GEOMETRY).unwrap();
    let ctx = plain_context();
    let records = run(
        &CodeHover::new(None),
        &args(
            "code.hover",
            &[
                ("path", path_value(&geometry)),
                ("line", Value::Int(9)),
                ("column", Value::Int(13)),
            ],
        ),
        &ctx,
    )
    .unwrap();
    let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
    assert_eq!(hover.source, SymbolSource::Syntax);
    assert_eq!(hover.signature.as_deref(), Some("def triple(value)"));
    assert_eq!(hover.documentation.as_deref(), Some("Triple the value."));
}

#[test]
fn hover_from_syntax_does_not_take_attributes_or_directives_as_documentation() {
    let project = Project::new();
    let geometry = project.path("geometry.py");
    for above in ["#[derive(Debug)]", "#include <stdio.h>"] {
        std::fs::write(
            &geometry,
            format!("# Real note.\n{above}\ndef triple(value):\n    return value * 3\n"),
        )
        .unwrap();
        let records = run(
            &CodeHover::new(None),
            &args(
                "code.hover",
                &[("path", path_value(&geometry)), ("symbol", word("triple"))],
            ),
            &plain_context(),
        )
        .unwrap();
        let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
        assert_eq!(hover.documentation, None, "{above}");
    }
}

#[test]
fn hover_from_syntax_without_documentation_has_only_the_signature() {
    let project = Project::new();
    let records = run(
        &CodeHover::new(None),
        &args(
            "code.hover",
            &[
                ("path", path_value(&project.path("shapes.py"))),
                ("symbol", word("Circle")),
            ],
        ),
        &plain_context(),
    )
    .unwrap();
    let hover = Hover::try_from(records.into_iter().next().unwrap()).unwrap();
    assert_eq!(hover.signature.as_deref(), Some("class Circle(Shape)"));
    assert_eq!(hover.documentation, None);
}

#[test]
fn hover_text_helpers_keep_decimal_points_and_drop_trailing_commas() {
    use nurt_code::commands::hover::{first_sentence, flat_signature, shorten_hover};

    assert_eq!(
        first_sentence("Version 1.5 is used.\nNext sentence."),
        "Version 1.5 is used."
    );
    assert_eq!(first_sentence("Is it done? Yes."), "Is it done?");
    assert_eq!(
        flat_signature("fn area(\n    shape: &Shape,\n    scale: f64,\n) -> f64"),
        "fn area(shape: &Shape, scale: f64) -> f64"
    );
    assert_eq!(flat_signature("\n\n"), "");
    assert_eq!(shorten_hover(""), (String::new(), String::new()));
    let (signature, documentation) = shorten_hover("```rust\nfn area()\n```\n***\nArea.");
    assert_eq!(signature, "fn area()");
    assert_eq!(documentation, "Area.");
}

fn shape_args(command: &str, project: &Project, extra: &[(&str, Value)]) -> BoundArgs {
    let mut pairs = vec![
        ("path", path_value(&project.path("shapes.py"))),
        ("symbol", word("Shape")),
    ];
    pairs.extend(extra.iter().cloned());
    args(command, &pairs)
}

#[test]
fn implementations_from_syntax_list_derived_classes() {
    let project = Project::new();
    let ctx = plain_context();
    let found = symbols(
        run(
            &CodeImplementations::new(Some(Script::unavailable().shared())),
            &shape_args("code.implementations", &project, &[]),
            &ctx,
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Circle");
    assert_eq!(found[0].kind, RecordKind::Class);
    assert_eq!(found[0].source, SymbolSource::Syntax);
    assert_eq!((found[0].line, found[0].column), (6, 7));
    assert_eq!(fallback_details(&ctx).len(), 1);
}

#[test]
fn implementations_answer_from_the_server() {
    let project = Project::new();
    let shapes = project.path("shapes.py");
    let script = Script {
        implementations: Ok(vec![location(&shapes, 6, 7, 6)]),
        ..Script::unavailable()
    }
    .shared();
    let found = symbols(
        run(
            &CodeImplementations::new(Some(script)),
            &shape_args("code.implementations", &project, &[]),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(found[0].name, "Circle");
    assert_eq!(found[0].source, SymbolSource::Lsp);
}

#[test]
fn type_definition_needs_a_language_server() {
    let project = Project::new();
    let error = failure(run(
        &CodeTypeDefinition::new(None),
        &shape_args("code.type_definition", &project, &[]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    let script = Script {
        type_definition: Ok(Vec::new()),
        ..Script::unavailable()
    }
    .shared();
    let error = failure(run(
        &CodeTypeDefinition::new(Some(script)),
        &shape_args(
            "code.type_definition",
            &project,
            &[("source", word("syntax"))],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    let error = failure(run(
        &CodeTypeDefinition::new(Some(Script::unavailable().shared())),
        &shape_args("code.type_definition", &project, &[]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn type_definition_answers_from_the_server() {
    let project = Project::new();
    let shapes = project.path("shapes.py");
    let script = Script {
        type_definition: Ok(vec![location(&shapes, 1, 7, 5)]),
        ..Script::unavailable()
    }
    .shared();
    let found = symbols(
        run(
            &CodeTypeDefinition::new(Some(script)),
            &args(
                "code.type_definition",
                &[
                    ("path", path_value(&project.path("main.py"))),
                    ("line", Value::Int(5)),
                    ("column", Value::Int(12)),
                ],
            ),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "Shape");
    assert_eq!(found[0].kind, RecordKind::Class);
    assert_eq!(found[0].source, SymbolSource::Lsp);
}

fn call_texts(project: &Project, found: &[Reference]) -> Vec<(String, u64, String)> {
    found
        .iter()
        .map(|item| {
            (
                project.relative(item.path.as_str()),
                item.line,
                item.text.clone().unwrap_or_default(),
            )
        })
        .collect()
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn call_hierarchy_from_syntax_lists_callers_with_multiplicity() {
    let project = Project::new();
    let found = references(
        run(
            &CodeCallHierarchy::new(None),
            &args(
                "code.call_hierarchy",
                &[
                    ("path", path_value(&project.path("shapes.py"))),
                    ("symbol", word("scale")),
                    ("direction", word("incoming")),
                ],
            ),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(
        call_texts(&project, &found),
        vec![
            ("main.py".to_owned(), 4, "<- function run".to_owned()),
            (
                "shapes.py".to_owned(),
                7,
                "<- method Circle.area".to_owned()
            ),
            (
                "shapes.py".to_owned(),
                16,
                "<- function total x2".to_owned()
            ),
            (
                "tests/test_shapes.py".to_owned(),
                4,
                "<- function test_scale".to_owned()
            ),
        ]
    );
    assert!(found.iter().all(|item| item.source == SymbolSource::Syntax));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn call_hierarchy_from_syntax_lists_callees_of_the_body() {
    let project = Project::new();
    let found = references(
        run(
            &CodeCallHierarchy::new(None),
            &args(
                "code.call_hierarchy",
                &[
                    ("path", path_value(&project.path("shapes.py"))),
                    ("symbol", word("total")),
                    ("direction", word("outgoing")),
                ],
            ),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(
        call_texts(&project, &found),
        vec![(
            "shapes.py".to_owned(),
            11,
            "-> function scale x2".to_owned()
        )]
    );
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn call_hierarchy_from_the_server_keeps_direction_and_call_count() {
    let project = Project::new();
    let main = project.path("main.py");
    let shapes = project.path("shapes.py");
    let edge = |direction, name: &str, path: &Path, line, calls| CallEdge {
        direction,
        symbol: SemanticSymbol {
            name: name.to_owned(),
            kind: RecordKind::Function,
            container: None,
            depth: 0,
            location: location(path, line, 1, 10),
            name_position: position(line, 5),
            detail: None,
        },
        calls,
    };
    let script = Script {
        calls: Ok(vec![
            edge(
                CallDirection::Incoming,
                "run",
                &main,
                4,
                vec![location(&main, 5, 12, 5), location(&main, 5, 20, 5)],
            ),
            edge(CallDirection::Outgoing, "helper", &shapes, 11, Vec::new()),
        ]),
        ..Script::unavailable()
    }
    .shared();
    let found = references(
        run(
            &CodeCallHierarchy::new(Some(script.clone())),
            &args(
                "code.call_hierarchy",
                &[
                    ("path", path_value(&project.path("shapes.py"))),
                    ("symbol", word("scale")),
                ],
            ),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(
        call_texts(&project, &found),
        vec![
            ("main.py".to_owned(), 4, "<- function run x2".to_owned()),
            ("shapes.py".to_owned(), 11, "-> function helper".to_owned()),
        ]
    );
    assert!(found.iter().all(|item| item.source == SymbolSource::Lsp));
    assert_eq!(
        *script.asked_directions.lock().unwrap(),
        vec![CallDirection::Both]
    );
}

#[test]
fn call_hierarchy_rejects_an_unknown_direction() {
    let project = Project::new();
    let error = failure(run(
        &CodeCallHierarchy::new(None),
        &args(
            "code.call_hierarchy",
            &[
                ("path", path_value(&project.path("shapes.py"))),
                ("symbol", word("scale")),
                ("direction", word("sideways")),
            ],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

fn actions_args(project: &Project, extra: &[(&str, Value)]) -> BoundArgs {
    let mut pairs = vec![
        ("path", path_value(&project.path("shapes.py"))),
        ("line", Value::Int(16)),
    ];
    pairs.extend(extra.iter().cloned());
    args("code.actions", &pairs)
}

#[test]
fn actions_list_the_server_actions_for_a_line_range() {
    let project = Project::new();
    let script = Script {
        actions: Ok(vec![
            CodeActionInfo {
                title: "Extract function".to_owned(),
                kind: Some("refactor.extract".to_owned()),
                preferred: false,
                edit: None,
                command: None,
            },
            CodeActionInfo {
                title: "Organize imports".to_owned(),
                kind: None,
                preferred: true,
                edit: None,
                command: Some("organize".to_owned()),
            },
        ]),
        ..Script::unavailable()
    }
    .shared();
    let records = run(
        &CodeActions::new(Some(script.clone())),
        &actions_args(&project, &[("end_line", Value::Int(17))]),
        &plain_context(),
    )
    .unwrap();
    let found: Vec<CodeAction> = records
        .into_iter()
        .map(|record| CodeAction::try_from(record).unwrap())
        .collect();
    let listed: Vec<(&str, Option<&str>, u64, Option<u64>)> = found
        .iter()
        .map(|action| {
            (
                action.title.as_str(),
                action.kind.as_deref(),
                action.line,
                action.end_line,
            )
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            ("Extract function", Some("refactor.extract"), 16, Some(17)),
            ("Organize imports", Some("command"), 16, Some(17)),
        ]
    );
    let range = script.asked_ranges.lock().unwrap()[0].clone();
    assert_eq!(
        (range.start, range.end),
        (position(16, 1), position(17, 40))
    );
}

#[test]
fn actions_need_a_language_server_and_an_ordered_range() {
    let project = Project::new();
    let error = failure(run(
        &CodeActions::new(None),
        &actions_args(&project, &[]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
    let script = Script {
        actions: Ok(Vec::new()),
        ..Script::unavailable()
    }
    .shared();
    let error = failure(run(
        &CodeActions::new(Some(script.clone())),
        &actions_args(&project, &[("end_line", Value::Int(3))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    let error = failure(run(
        &CodeActions::new(Some(script)),
        &actions_args(&project, &[("end_line", Value::Int(90))]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

fn diagnostic(line: u32, severity: DiagnosticSeverity, message: &str) -> SemanticDiagnostic {
    SemanticDiagnostic {
        range: Location {
            path: PathBuf::new(),
            start: position(line, 1),
            end: position(line, 4),
        },
        severity,
        message: message.to_owned(),
        code: None,
        source: Some("pyright".to_owned()),
        related: Vec::new(),
    }
}

fn diagnostics(records: Vec<Record>) -> Vec<Diagnostic> {
    records
        .into_iter()
        .map(|record| Diagnostic::try_from(record).unwrap())
        .collect()
}

#[test]
fn diagnostics_from_the_server_are_filtered_ordered_and_deduplicated() {
    let project = Project::new();
    let script = Script {
        diagnostics: Ok(vec![
            diagnostic(2, DiagnosticSeverity::Hint, "unused parameter"),
            diagnostic(3, DiagnosticSeverity::Warning, "constant return"),
            diagnostic(8, DiagnosticSeverity::Error, "scale is not defined"),
            diagnostic(8, DiagnosticSeverity::Error, "scale is not defined"),
        ]),
        ..Script::unavailable()
    }
    .shared();
    let found = diagnostics(
        run(
            &CodeDiagnostics::new(Some(script), None),
            &args(
                "code.diagnostics",
                &[
                    ("path", path_value(&project.path("shapes.py"))),
                    ("severity", word("warning")),
                ],
            ),
            &plain_context(),
        )
        .unwrap(),
    );
    let listed: Vec<(u64, DiagnosticSeverity, &str)> = found
        .iter()
        .map(|item| (item.line, item.severity, item.message.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (8, DiagnosticSeverity::Error, "scale is not defined"),
            (3, DiagnosticSeverity::Warning, "constant return"),
        ]
    );
    assert!(found.iter().all(|item| item.source == "pyright"));
    assert_eq!(found[0].language.as_deref(), Some("python"));
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn diagnostics_of_a_directory_fall_back_to_syntax_with_one_note_per_cause() {
    let project = Project::new();
    std::fs::write(project.path("broken.py"), "def broken(:\n    return 1\n").unwrap();
    let script = Script::unavailable().shared();
    let ctx = plain_context();
    let found = diagnostics(
        run(
            &CodeDiagnostics::new(Some(script.clone()), None),
            &args("code.diagnostics", &[("path", path_value(&project.root))]),
            &ctx,
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(project.relative(found[0].path.as_str()), "broken.py");
    assert_eq!(found[0].source, "syntax");
    assert_eq!(found[0].severity, DiagnosticSeverity::Error);
    assert_eq!(script.asked_paths.lock().unwrap().len(), 4);
    assert_eq!(fallback_details(&ctx).len(), 1);
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn diagnostics_with_changed_check_only_changed_files() {
    let project = Project::new();
    let git = Arc::new(FixedGit {
        root: project.root.clone(),
        changed: vec![
            "main.py".to_owned(),
            "removed.py".to_owned(),
            "notes.txt".to_owned(),
        ],
    });
    std::fs::write(project.path("notes.txt"), "plain text\n").unwrap();
    let script = Script {
        diagnostics: Ok(vec![diagnostic(
            5,
            DiagnosticSeverity::Info,
            "call result unused",
        )]),
        ..Script::unavailable()
    }
    .shared();
    let found = diagnostics(
        run(
            &CodeDiagnostics::new(Some(script.clone()), Some(git)),
            &args(
                "code.diagnostics",
                &[
                    ("path", Value::List(vec![path_value(&project.root)])),
                    ("changed", Value::Bool(true)),
                ],
            ),
            &plain_context(),
        )
        .unwrap(),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(project.relative(found[0].path.as_str()), "main.py");
    assert_eq!(
        *script.asked_paths.lock().unwrap(),
        vec![project.path("main.py")]
    );
}

#[test]
fn listing_a_directory_stops_when_the_execution_is_cancelled() {
    let project = Project::new();
    let empty = project.path("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let ctx = plain_context();
    ctx.cancel().cancel(CancelReason::Timeout);
    let error = failure(run(
        &CodeDiagnostics::new(None, None),
        &args("code.diagnostics", &[("path", path_value(&empty))]),
        &ctx,
    ));
    assert!(
        matches!(error.code(), ErrorCode::Cancelled | ErrorCode::Timeout),
        "{error:?}"
    );
}

#[test]
fn diagnostics_with_changed_need_the_repository_service() {
    let project = Project::new();
    let error = failure(run(
        &CodeDiagnostics::new(None, None),
        &args(
            "code.diagnostics",
            &[
                ("path", path_value(&project.root)),
                ("changed", Value::Bool(true)),
            ],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalMissing);
}

#[test]
fn diagnostics_need_a_path_or_changed_and_a_known_severity() {
    let project = Project::new();
    let error = failure(run(
        &CodeDiagnostics::new(None, None),
        &args("code.diagnostics", &[]),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::MissingParam);
    let error = failure(run(
        &CodeDiagnostics::new(None, None),
        &args(
            "code.diagnostics",
            &[
                ("path", path_value(&project.root)),
                ("severity", word("fatal")),
            ],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn diagnostics_under_source_lsp_report_the_server_failure() {
    let project = Project::new();
    let script = Script {
        diagnostics: Err(SemanticError::new(
            SemanticErrorKind::Failed,
            "the server ended",
        )),
        ..Script::unavailable()
    }
    .shared();
    let error = failure(run(
        &CodeDiagnostics::new(Some(script), None),
        &args(
            "code.diagnostics",
            &[
                ("path", path_value(&project.path("main.py"))),
                ("source", word("lsp")),
            ],
        ),
        &plain_context(),
    ));
    assert_eq!(error.code(), ErrorCode::ExternalFailed);
}
