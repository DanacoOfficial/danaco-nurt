// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test doubles and helpers of the syntax analysis tests: a workspace on
//! the virtual file system of `nurt-testkit`, a file listing service over
//! that file system, a scripted semantic provider, stage runners and the
//! text forms of symbols used by the corpus snapshots.

// Each test target uses a subset of these helpers.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use nurt_code::semantic::{
    CallDirection, CallEdge, CodeActionInfo, DocumentPosition, HoverInfo, Location,
    SemanticDiagnostic, SemanticError, SemanticErrorKind, SemanticProvider, SemanticResult,
    SemanticScope, SemanticSymbol, ServerStatus, WorkspaceChange,
};
use nurt_code::symbols::kind_name;
use nurt_contracts::config::Config;
use nurt_contracts::envelope::RequestOptions;
use nurt_contracts::error::Result;
use nurt_contracts::fs::{AbsPath, FileKind, Fs};
use nurt_contracts::record::{Symbol, SymbolKind as RecordKind};
use nurt_contracts::services::{
    ExcludePreset, FileListService, LanguageId, Services, SymbolInfo, TextPosition, WalkEntry,
    WalkIter, WalkSpec,
};
use nurt_contracts::stage::{ArgValue, BoundArgs, ExecCtx, StageImpl, StageInput};
use nurt_contracts::value::{Record, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::vfs::{VirtualFs, test_path};

/// Root of the project in the virtual file system.
pub(crate) const ROOT: &str = "/repo";

/// Directory of the corpus of one kind (`symbols`, `outline`, `local`).
pub(crate) fn corpus_dir(kind: &str) -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5/code")
    .join(kind)
}

/// Text form of a symbol in the corpus snapshots: indentation by depth,
/// kind, name, container, name position, whole range, visibility and
/// signature.
pub(crate) fn symbol_line(info: &SymbolInfo) -> String {
    let depth = usize::try_from(info.depth).unwrap_or(0);
    let mut line = format!(
        "{}{} {}",
        "  ".repeat(depth),
        kind_name(info.kind),
        info.name
    );
    if let Some(container) = &info.container {
        line.push_str(&format!(" in {container}"));
    }
    line.push_str(&format!(
        " @{} [{}-{}]",
        position(info.name_span.start),
        position(info.span.start),
        position(info.span.end)
    ));
    match info.exported {
        Some(true) => line.push_str(" exported"),
        Some(false) => line.push_str(" private"),
        None => {}
    }
    if let Some(signature) = &info.signature {
        line.push_str(&format!(" | {signature}"));
    }
    line
}

/// `line:column` of a position.
pub(crate) fn position(at: TextPosition) -> String {
    format!("{}:{}", at.line, at.column)
}

/// Language identifier of a known language.
pub(crate) fn language(id: &str) -> LanguageId {
    LanguageId::new(id).unwrap()
}

/// Directory names skipped by the listing double for each exclusion group.
fn preset_directories(preset: ExcludePreset) -> &'static [&'static str] {
    match preset {
        ExcludePreset::Vcs => &[".git"],
        ExcludePreset::Deps => &["node_modules", "venv", ".venv"],
        ExcludePreset::Cache => &["__pycache__"],
        ExcludePreset::Build => &["target", "build"],
        _ => &[],
    }
}

/// File listing service over a virtual file system: lists the files under
/// the root in name order, skips the directories of the exclusion groups
/// and the path components named by exclusions, and keeps every request.
pub(crate) struct VfsFileList {
    fs: Arc<VirtualFs>,
    requests: Mutex<Vec<WalkSpec>>,
}

impl VfsFileList {
    /// Listing of `fs`.
    pub(crate) fn new(fs: Arc<VirtualFs>) -> Self {
        Self {
            fs,
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Requests received so far.
    pub(crate) fn requests(&self) -> Vec<WalkSpec> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn skipped(spec: &WalkSpec, name: &str) -> bool {
        spec.exclude_presets
            .iter()
            .any(|preset| preset_directories(*preset).contains(&name))
            || spec.exclude.iter().any(|pattern| pattern == name)
    }

    fn collect(&self, spec: &WalkSpec, directory: &Path, depth: u32, out: &mut Vec<WalkEntry>) {
        let Ok(absolute) = AbsPath::new(directory) else {
            return;
        };
        let Ok(entries) = self.fs.read_dir(&absolute) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(std::result::Result::ok).collect();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries {
            let name = entry.name.to_string_lossy().into_owned();
            if Self::skipped(spec, &name) {
                continue;
            }
            let path = directory.join(&name);
            if entry.kind == Some(FileKind::Directory) {
                out.push(WalkEntry {
                    path: path.clone(),
                    depth,
                    is_dir: true,
                });
                self.collect(spec, &path, depth + 1, out);
            } else {
                out.push(WalkEntry {
                    path,
                    depth,
                    is_dir: false,
                });
            }
        }
    }
}

impl FileListService for VfsFileList {
    fn walk<'a>(&'a self, spec: &WalkSpec) -> Result<WalkIter<'a>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(spec.clone());
        let mut entries = Vec::new();
        self.collect(spec, &spec.root, 1, &mut entries);
        Ok(Box::new(entries.into_iter().map(Ok)))
    }
}

/// Project on a virtual file system with a state directory on disk.
pub(crate) struct Workspace {
    pub(crate) test: TestContext,
    pub(crate) listing: Arc<VfsFileList>,
    pub(crate) state: tempfile::TempDir,
}

impl Workspace {
    /// Empty project with the listing service.
    pub(crate) fn new() -> Self {
        let test = TestContext::new();
        let listing = Arc::new(VfsFileList::new(test.fs().clone()));
        let services = Services::new().with_file_list(listing.clone());
        Self {
            test: test.with_services(services),
            listing,
            state: tempfile::tempdir().unwrap(),
        }
    }

    /// Empty project without a listing service: listings read the file
    /// system of the context.
    pub(crate) fn without_listing() -> Self {
        let test = TestContext::new();
        let listing = Arc::new(VfsFileList::new(test.fs().clone()));
        Self {
            test: test.with_services(Services::new()),
            listing,
            state: tempfile::tempdir().unwrap(),
        }
    }

    /// Absolute path of a project file.
    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        nurt_testkit::paths::abs_path(&format!("{ROOT}/{relative}"))
    }

    /// Writes a project file.
    pub(crate) fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.path(relative);
        self.test
            .fs()
            .write_file(
                &test_path(&format!("{ROOT}/{relative}")).unwrap(),
                content.as_bytes(),
            )
            .unwrap();
        path
    }

    /// Writes several project files.
    pub(crate) fn write_all(&self, files: &[(&str, &str)]) {
        for (relative, content) in files {
            self.write(relative, content);
        }
    }

    /// Context with the default configuration, the state directory and
    /// the project root as base directory.
    pub(crate) fn ctx(&self) -> ExecCtx {
        self.ctx_with(Config::default())
    }

    /// Context with `config`.
    pub(crate) fn ctx_with(&self, config: Config) -> ExecCtx {
        let options = RequestOptions {
            base: Some(nurt_testkit::paths::abs(ROOT)),
            ..RequestOptions::default()
        };
        self.test
            .builder()
            .config(Arc::new(config))
            .options(Arc::new(options))
            .state_dir(self.state.path().to_path_buf())
            .build()
    }

    /// Context without a state directory.
    pub(crate) fn ctx_without_state(&self) -> ExecCtx {
        self.test.builder().build()
    }
}

/// Configuration with a limit of listed files.
pub(crate) fn config_with_max_files(max: u32) -> Config {
    let mut config = Config::default();
    config.code.index_max_files = max;
    config
}

/// Arguments of a stage.
pub(crate) fn args(command: &str, pairs: &[(&str, Value)]) -> BoundArgs {
    let mut args = BoundArgs::new(command);
    for (name, value) in pairs {
        args.insert(*name, ArgValue::Value(value.clone()), false);
    }
    args
}

/// Text argument.
pub(crate) fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// Enumeration argument.
pub(crate) fn word(value: &str) -> Value {
    Value::Enum(value.to_owned())
}

/// Path argument.
pub(crate) fn path_value(path: &Path) -> Value {
    // A project path in POSIX form names the same entry on every system with
    // the root of `nurt_testkit::paths`.
    let text = path.to_string_lossy().replace('\\', "/");
    if text.starts_with('/') {
        Value::Path(nurt_testkit::paths::abs(&text))
    } else {
        Value::Path(text)
    }
}

/// Records of a read stage.
pub(crate) fn run(stage: &dyn StageImpl, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let output = stage
        .as_read()
        .unwrap()
        .execute(args, StageInput::none(), ctx)?;
    StageInput::from(output)
        .into_records()?
        .collect::<Result<Vec<_>>>()
}

/// Symbol records of a read stage.
pub(crate) fn run_symbols(
    stage: &dyn StageImpl,
    args: &BoundArgs,
    ctx: &ExecCtx,
) -> Result<Vec<Symbol>> {
    Ok(run(stage, args, ctx)?
        .into_iter()
        .map(|record| Symbol::try_from(record).unwrap())
        .collect())
}

/// Names of symbol records in order.
pub(crate) fn names(symbols: &[Symbol]) -> Vec<String> {
    symbols.iter().map(|symbol| symbol.name.clone()).collect()
}

/// Path of a record relative to the project root.
pub(crate) fn relative(symbol: &Symbol) -> String {
    let path = symbol.path.as_str();
    path.strip_prefix(&format!("{}/", nurt_testkit::paths::abs(ROOT)))
        .unwrap_or(path)
        .to_owned()
}

/// Answers of the scripted semantic provider.
#[derive(Clone)]
pub(crate) struct Script {
    pub(crate) workspace: SemanticResult<Vec<SemanticSymbol>>,
    pub(crate) document: SemanticResult<Vec<SemanticSymbol>>,
    pub(crate) references: SemanticResult<Vec<Location>>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            workspace: Ok(Vec::new()),
            document: Ok(Vec::new()),
            references: Ok(Vec::new()),
        }
    }
}

/// Semantic provider that answers from a [`Script`] and keeps the
/// workspace symbol queries and document paths it receives.
pub(crate) struct ScriptedSemantic {
    script: Script,
    queries: Mutex<Vec<String>>,
    documents: Mutex<Vec<PathBuf>>,
}

impl ScriptedSemantic {
    /// Provider answering `script`.
    pub(crate) fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            script,
            queries: Mutex::new(Vec::new()),
            documents: Mutex::new(Vec::new()),
        })
    }

    /// Provider whose every request fails with `kind`.
    pub(crate) fn failing(kind: SemanticErrorKind) -> Arc<Self> {
        let error = SemanticError::new(kind, "no server for the language").with_server("toy-ls");
        Self::new(Script {
            workspace: Err(error.clone()),
            document: Err(error.clone()),
            references: Err(error),
        })
    }

    /// Workspace symbol queries received.
    pub(crate) fn queries(&self) -> Vec<String> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Paths of the document symbol requests received.
    pub(crate) fn documents(&self) -> Vec<PathBuf> {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn unsupported<T>() -> SemanticResult<T> {
    Err(SemanticError::new(
        SemanticErrorKind::Unsupported,
        "not scripted",
    ))
}

impl SemanticProvider for ScriptedSemantic {
    fn workspace_symbols(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        query: &str,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(query.to_owned());
        self.script.workspace.clone()
    }

    fn document_symbols(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        path: &Path,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path.to_path_buf());
        self.script.document.clone()
    }

    fn definition(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        unsupported()
    }

    fn references(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _include_declaration: bool,
    ) -> SemanticResult<Vec<Location>> {
        self.script.references.clone()
    }

    fn hover(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Option<HoverInfo>> {
        unsupported()
    }

    fn implementations(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        unsupported()
    }

    fn type_definition(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        unsupported()
    }

    fn call_hierarchy(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _direction: CallDirection,
    ) -> SemanticResult<Vec<CallEdge>> {
        unsupported()
    }

    fn code_actions(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _range: &Location,
    ) -> SemanticResult<Vec<CodeActionInfo>> {
        unsupported()
    }

    fn diagnostics(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _path: &Path,
        _text: Option<&str>,
    ) -> SemanticResult<Vec<SemanticDiagnostic>> {
        unsupported()
    }

    fn rename(
        &self,
        _ctx: &ExecCtx,
        _scope: &SemanticScope,
        _at: &DocumentPosition,
        _new_name: &str,
    ) -> SemanticResult<WorkspaceChange> {
        unsupported()
    }

    fn server_status(&self, _ctx: &ExecCtx, _language: Option<&LanguageId>) -> Vec<ServerStatus> {
        Vec::new()
    }
}

/// Symbol reported by the scripted server.
pub(crate) fn server_symbol(
    name: &str,
    kind: RecordKind,
    path: &Path,
    line: u32,
) -> SemanticSymbol {
    let start = TextPosition { line, column: 1 };
    let end = TextPosition {
        line: line + 1,
        column: 2,
    };
    SemanticSymbol {
        name: name.to_owned(),
        kind,
        container: None,
        depth: 0,
        location: Location {
            path: path.to_path_buf(),
            start,
            end,
        },
        name_position: TextPosition { line, column: 5 },
        detail: Some(format!("{name}()")),
    }
}
