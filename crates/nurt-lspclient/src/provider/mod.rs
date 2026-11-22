// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Semantic analysis of the code group backed by the server pool.
//!
//! [`LspProvider`] implements [`SemanticProvider`]. Each request acquires the
//! server of the language and project root from the pool, brings the
//! documents opened earlier up to date with the disk, opens the requested
//! file and converts positions between the one-based character columns of
//! the code group and the position encoding of the server.
//!
//! A server that is loading the project often answers with an empty or
//! partial result. A request answered while the server reported work in
//! progress is therefore repeated once the work ends: an empty or failed
//! answer within the whole time limit, a non-empty answer once and only when
//! the work ends within [`WARM_WAIT`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lsp_types::{
    Diagnostic, DiagnosticSeverity as LspSeverity, NumberOrString, PositionEncodingKind,
};
use nurt_code::semantic::{
    CallDirection, CallEdge, CodeActionInfo, DocumentPosition, HoverInfo, Location, RelatedInfo,
    SemanticDiagnostic, SemanticError, SemanticErrorKind, SemanticProvider, SemanticResult,
    SemanticScope, SemanticSymbol, ServerStatus, WorkspaceChange,
};
use nurt_contracts::config::CodeConfig;
use nurt_contracts::env::Clock;
use nurt_contracts::record::{DiagnosticSeverity, SymbolKind};
use nurt_contracts::services::{LanguageId, ProcessHost, TextPosition};
use nurt_contracts::stage::{CancelToken, Event, ExecCtx, LogEvent, LogLevel};
use serde_json::{Value, json};

use crate::config::{self, ClientSettings};
use crate::diagnostics::QuietWait;
use crate::pool::{AcquireRequest, ServerPool};
use crate::rename::{PositionUnit, workspace_change};
use crate::server::{LanguageServer, LspError};
use crate::sync::{self, CandidateLimits, path_to_uri, uri_to_path};

/// Longest wait for the end of project loading after a non-empty answer.
pub const WARM_WAIT: Duration = Duration::from_secs(5);

/// Largest number of repetitions of a request answered during loading.
const WARM_ROUNDS: usize = 5;

/// Largest number of project files opened before `references` and
/// `rename`, so that files the server has not loaded yet are included.
pub const PREOPEN_MAX: usize = 60;

/// Longest wait for published diagnostics used as the context of code
/// actions.
const ACTION_DIAGNOSTICS_WAIT: Duration = Duration::from_secs(2);

/// Shortest time limit given to a single request.
const REQUEST_MIN: Duration = Duration::from_millis(1);

/// Language server pool behind the [`SemanticProvider`] of the code group.
///
/// The provider stops its servers when it is dropped or with
/// [`LspProvider::shutdown`].
#[derive(Debug)]
pub struct LspProvider {
    pool: ServerPool,
}

impl LspProvider {
    /// Provider with the settings of the `[code]` configuration; servers are
    /// started through `host`, idle and retry periods measured on `clock`.
    #[must_use]
    pub fn new(host: Arc<dyn ProcessHost>, clock: Arc<dyn Clock>, config: &CodeConfig) -> Self {
        Self::with_settings(host, clock, ClientSettings::from_config(config))
    }

    /// Provider with explicit settings.
    #[must_use]
    pub fn with_settings(
        host: Arc<dyn ProcessHost>,
        clock: Arc<dyn Clock>,
        settings: ClientSettings,
    ) -> Self {
        Self {
            pool: ServerPool::new(host, clock, settings),
        }
    }

    /// Server pool of the provider.
    #[must_use]
    pub fn pool(&self) -> &ServerPool {
        &self.pool
    }

    /// Stops every server; returns how many stopped.
    pub fn shutdown(&self) -> usize {
        self.pool.stop(None)
    }

    fn session<'a>(
        &'a self,
        ctx: &'a ExecCtx,
        scope: &SemanticScope,
        path: &Path,
    ) -> SemanticResult<Session<'a>> {
        let language = match &scope.language {
            Some(language) => config::normalize_language(language.as_str()),
            None => config::language_for_path(path)
                .map(str::to_owned)
                .ok_or_else(|| {
                    SemanticError::new(
                        SemanticErrorKind::Unavailable,
                        format!(
                            "no language server is known for {}; set language",
                            path.display()
                        ),
                    )
                })?,
        };
        let settings = self.pool.settings();
        let mut timeout = settings.request_timeout(scope.timeout);
        if let Some(remaining) = ctx.remaining() {
            timeout = timeout.min(remaining);
        }
        let started = Instant::now();
        let request = AcquireRequest {
            language: &language,
            root: &scope.root,
            timeout,
            cancel: ctx.cancel(),
        };
        let server = self.pool.acquire(&request).map_err(semantic_error)?;
        server.refresh_documents(settings.file_bytes_max);
        let unit = server
            .capabilities()
            .get("positionEncoding")
            .and_then(Value::as_str)
            .map_or(PositionUnit::Utf16, |kind| {
                PositionUnit::from_kind(&PositionEncodingKind::from(kind.to_owned()))
            });
        Ok(Session {
            provider: self,
            ctx,
            server,
            unit,
            language,
            root: scope.root.clone(),
            deadline: started + timeout,
            texts: RefCell::new(HashMap::new()),
        })
    }

    fn positional(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        capability: (&str, &str),
        extra: Option<(&str, Value)>,
    ) -> SemanticResult<Vec<Location>> {
        let session = self.session(ctx, scope, &at.path)?;
        session.require(capability.0, capability.1)?;
        session.open(&at.path, None)?;
        let mut params = session.position_params(at);
        if let (Some((key, value)), Some(object)) = (extra, params.as_object_mut()) {
            object.insert(key.to_owned(), value);
        }
        let answer = session.ask(capability.1, &params)?;
        Ok(session.locations(&answer))
    }
}

/// Converts a pool or server failure to the error of the code group.
#[must_use]
pub fn semantic_error(error: LspError) -> SemanticError {
    let message = error.to_string();
    match error {
        LspError::Missing { server, .. } => {
            let hint = if server.is_empty() {
                "set code.servers.<language>.command to a language server".to_owned()
            } else {
                config::install_hint(&server).map_or_else(
                    || format!("install {server} or set code.servers.<language>.command"),
                    str::to_owned,
                )
            };
            let error = SemanticError::new(SemanticErrorKind::Unavailable, message).with_fix(hint);
            if server.is_empty() {
                error
            } else {
                error.with_server(server)
            }
        }
        LspError::Unavailable { server, .. }
        | LspError::Closed { server, .. }
        | LspError::Rpc { server, .. } => {
            SemanticError::new(SemanticErrorKind::Failed, message).with_server(server)
        }
        LspError::Timeout { server } => {
            SemanticError::new(SemanticErrorKind::TimedOut, message).with_server(server)
        }
        LspError::Cancelled(_) => SemanticError::new(SemanticErrorKind::Cancelled, message),
    }
}

struct Answer {
    value: Value,
    empty: bool,
    error: Option<SemanticError>,
    loading: bool,
}

/// One request of a command to one server.
struct Session<'a> {
    provider: &'a LspProvider,
    ctx: &'a ExecCtx,
    server: LanguageServer,
    unit: PositionUnit,
    language: String,
    root: PathBuf,
    deadline: Instant,
    texts: RefCell<HashMap<PathBuf, Option<String>>>,
}

impl Session<'_> {
    fn cancel(&self) -> &CancelToken {
        self.ctx.cancel()
    }

    fn remaining(&self) -> Duration {
        self.deadline
            .saturating_duration_since(Instant::now())
            .max(REQUEST_MIN)
    }

    fn require(&self, capability: &str, method: &str) -> SemanticResult<()> {
        if self.server.supports(capability, method) {
            return Ok(());
        }
        Err(SemanticError::new(
            SemanticErrorKind::Unsupported,
            format!(
                "language server {} does not support {method}",
                self.server.name()
            ),
        )
        .with_server(self.server.name()))
    }

    fn text(&self, path: &Path) -> Option<String> {
        let max = self.provider.pool.settings().file_bytes_max;
        self.texts
            .borrow_mut()
            .entry(path.to_path_buf())
            .or_insert_with(|| sync::read_text(path, max))
            .clone()
    }

    /// Opens `path` in the server with `text`, or with the content on disk.
    fn open(&self, path: &Path, text: Option<&str>) -> SemanticResult<()> {
        let language_id = config::language_id(path, &self.language);
        if let Some(text) = text {
            self.texts
                .borrow_mut()
                .insert(path.to_path_buf(), Some(text.to_owned()));
            self.server.sync_document(path, &language_id, text, None);
            return Ok(());
        }
        let stamp = sync::file_stamp(path);
        let content = self.text(path).ok_or_else(|| {
            SemanticError::new(
                SemanticErrorKind::Failed,
                format!(
                    "cannot read {}: the file is missing, binary, not UTF-8 or larger than code.file_bytes_max",
                    path.display()
                ),
            )
        })?;
        self.server
            .sync_document(path, &language_id, &content, stamp);
        Ok(())
    }

    fn uri(&self, path: &Path) -> SemanticResult<String> {
        path_to_uri(path).ok_or_else(|| {
            SemanticError::new(
                SemanticErrorKind::Failed,
                format!("{} is not an absolute path", path.display()),
            )
        })
    }

    fn position_params(&self, at: &DocumentPosition) -> Value {
        let text = self.text(&at.path);
        json!({
            "textDocument": {"uri": path_to_uri(&at.path)},
            "position": to_lsp(text.as_deref(), at.position, self.unit),
        })
    }

    fn range_value(&self, location: &Location) -> Value {
        let text = self.text(&location.path);
        json!({
            "start": to_lsp(text.as_deref(), location.start, self.unit),
            "end": to_lsp(text.as_deref(), location.end, self.unit),
        })
    }

    fn location(&self, path: PathBuf, range: &Value) -> Option<Location> {
        let text = self.text(&path);
        let start = from_lsp(text.as_deref(), range.get("start")?, self.unit)?;
        let end = range
            .get("end")
            .and_then(|end| from_lsp(text.as_deref(), end, self.unit))
            .unwrap_or(start);
        Some(Location { path, start, end })
    }

    fn location_of(&self, item: &Value) -> Option<Location> {
        let uri = item
            .get("uri")
            .or_else(|| item.get("targetUri"))?
            .as_str()?;
        let path = uri_to_path(uri)?;
        let range = item
            .get("range")
            .or_else(|| item.get("targetSelectionRange"))
            .or_else(|| item.get("targetRange"));
        match range {
            Some(range) => self.location(path, range),
            None => Some(Location {
                path,
                start: TextPosition { line: 1, column: 1 },
                end: TextPosition { line: 1, column: 1 },
            }),
        }
    }

    /// Locations of a `Location`, `Location[]` or `LocationLink[]` answer,
    /// without repetitions.
    fn locations(&self, answer: &Value) -> Vec<Location> {
        let items: Vec<&Value> = match answer {
            Value::Array(items) => items.iter().collect(),
            Value::Null => Vec::new(),
            other => vec![other],
        };
        let mut found: Vec<Location> = Vec::new();
        for item in items {
            if let Some(location) = self.location_of(item)
                && !found.contains(&location)
            {
                found.push(location);
            }
        }
        found
    }

    fn ask(&self, method: &str, params: &Value) -> SemanticResult<Value> {
        let patience = Instant::now() + WARM_WAIT;
        let mut answer = self.attempt(method, params)?;
        let mut round = 0;
        while round < WARM_ROUNDS && answer.loading && (answer.empty || round == 0) {
            self.note_loading();
            let wait = if answer.empty {
                self.remaining()
            } else {
                self.remaining()
                    .min(patience.saturating_duration_since(Instant::now()))
            };
            let ready = !wait.is_zero() && self.server.progress().wait_settled(wait, self.cancel());
            if !ready && answer.empty {
                return Err(self.loading_error());
            }
            if !ready {
                break;
            }
            answer = self.attempt(method, params)?;
            round += 1;
        }
        match answer.error {
            Some(error) => Err(error),
            None => Ok(answer.value),
        }
    }

    fn loading_error(&self) -> SemanticError {
        if self.cancel().is_cancelled() {
            return SemanticError::new(SemanticErrorKind::Cancelled, "the request was cancelled");
        }
        SemanticError::new(
            SemanticErrorKind::TimedOut,
            format!(
                "language server {} did not finish loading the project within the time limit",
                self.server.name()
            ),
        )
        .with_server(self.server.name())
    }

    fn attempt(&self, method: &str, params: &Value) -> SemanticResult<Answer> {
        let progress = self.server.progress();
        let loads = progress.loads();
        let busy_before = progress.busy();
        let result = self
            .server
            .request(method, params.clone(), self.remaining(), self.cancel());
        let loading = busy_before || progress.busy() || progress.loads() != loads;
        match result {
            Ok(value) => Ok(Answer {
                empty: is_empty(&value),
                value,
                error: None,
                loading,
            }),
            Err(error @ LspError::Rpc { .. }) => Ok(Answer {
                value: Value::Null,
                empty: true,
                error: Some(semantic_error(error)),
                loading,
            }),
            Err(error) => Err(semantic_error(error)),
        }
    }

    fn note_loading(&self) {
        let note = self.server.progress().last_note().unwrap_or_default();
        let message = format!("{} is loading the project {note}", self.server.name());
        self.ctx.emit(Event::Log(LogEvent {
            level: LogLevel::Info,
            message: message.trim_end().to_owned(),
            source: Some(self.server.name().to_owned()),
        }));
    }

    /// Opens project files that contain the identifier at `at` and are
    /// handled by the same server.
    fn open_candidates(&self, at: &DocumentPosition) {
        let Some(word) = self
            .text(&at.path)
            .and_then(|text| word_at(&text, at.position))
        else {
            return;
        };
        let program = self.server.name().to_owned();
        let same_server = RefCell::new(HashMap::<&'static str, bool>::new());
        let accept = |path: &Path| {
            path != at.path
                && config::language_for_path(path).is_some_and(|language| {
                    *same_server.borrow_mut().entry(language).or_insert_with(|| {
                        self.provider
                            .pool
                            .command_for(language, &self.root)
                            .is_ok_and(|command| command.name == program)
                    })
                })
        };
        let settings = self.provider.pool.settings();
        let limits = CandidateLimits {
            max_files: PREOPEN_MAX,
            max_scanned: settings.index_max_files,
            max_bytes: settings.file_bytes_max,
        };
        for path in sync::candidate_files(&self.root, &word, accept, limits) {
            let _ = self.open(&path, None);
        }
    }

    fn symbol(
        &self,
        item: &Value,
        path: Option<&Path>,
        container: Option<&str>,
        depth: u32,
    ) -> Option<SemanticSymbol> {
        let name = item.get("name")?.as_str()?.to_owned();
        let (path, whole, selection) = match item.get("location") {
            Some(location) => {
                let path = uri_to_path(location.get("uri")?.as_str()?)?;
                let range = location.get("range");
                (path, range, range)
            }
            None => {
                let path = match item.get("uri").and_then(Value::as_str) {
                    Some(uri) => uri_to_path(uri)?,
                    None => path?.to_path_buf(),
                };
                (
                    path,
                    item.get("range"),
                    item.get("selectionRange").or_else(|| item.get("range")),
                )
            }
        };
        let fallback = Location {
            path: path.clone(),
            start: TextPosition { line: 1, column: 1 },
            end: TextPosition { line: 1, column: 1 },
        };
        let location = whole
            .and_then(|range| self.location(path.clone(), range))
            .unwrap_or(fallback);
        let name_position = selection
            .and_then(|range| self.location(path, range))
            .map_or(location.start, |selection| selection.start);
        let container = item
            .get("containerName")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .or(container)
            .map(str::to_owned);
        Some(SemanticSymbol {
            name,
            kind: symbol_kind(item.get("kind").and_then(Value::as_u64).unwrap_or(0)),
            container,
            depth,
            location,
            name_position,
            detail: item
                .get("detail")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
        })
    }

    fn flatten(
        &self,
        items: &[Value],
        path: &Path,
        container: Option<&str>,
        depth: u32,
        into: &mut Vec<SemanticSymbol>,
    ) {
        for item in items {
            let Some(symbol) = self.symbol(item, Some(path), container, depth) else {
                continue;
            };
            let qualified = match &symbol.container {
                Some(outer) => format!("{outer}.{}", symbol.name),
                None => symbol.name.clone(),
            };
            into.push(symbol);
            if let Some(children) = item.get("children").and_then(Value::as_array) {
                self.flatten(children, path, Some(&qualified), depth + 1, into);
            }
        }
    }

    fn diagnostic(&self, path: &Path, item: &Diagnostic) -> SemanticDiagnostic {
        let range = serde_json::to_value(item.range).unwrap_or(Value::Null);
        let fallback = Location {
            path: path.to_path_buf(),
            start: TextPosition { line: 1, column: 1 },
            end: TextPosition { line: 1, column: 1 },
        };
        let related = item
            .related_information
            .iter()
            .flatten()
            .filter_map(|info| {
                let value = serde_json::to_value(&info.location).ok()?;
                Some(RelatedInfo {
                    location: self.location_of(&value)?,
                    message: info.message.clone(),
                })
            })
            .collect();
        SemanticDiagnostic {
            range: self
                .location(path.to_path_buf(), &range)
                .unwrap_or(fallback),
            severity: severity(item.severity),
            message: collapse_whitespace(&item.message),
            code: item.code.as_ref().map(|code| match code {
                NumberOrString::Number(number) => number.to_string(),
                NumberOrString::String(text) => text.clone(),
            }),
            source: item.source.clone(),
            related,
        }
    }

    fn published(&self, path: &Path, timeout: Duration) -> SemanticResult<Option<Vec<Diagnostic>>> {
        let uri = self.uri(path)?;
        let version = self.server.document_version(path).unwrap_or(0);
        let wait = QuietWait {
            timeout,
            quiet: self.provider.pool.settings().diagnostics_quiet,
            cancel: self.cancel(),
        };
        let progress = self.server.progress();
        Ok(self
            .server
            .published_diagnostics()
            .wait(&uri, version, &wait, || progress.busy()))
    }

    fn call_edge(
        &self,
        call: &Value,
        direction: CallDirection,
        origin: Option<&Path>,
    ) -> Option<CallEdge> {
        let key = if direction == CallDirection::Incoming {
            "from"
        } else {
            "to"
        };
        let item = call.get(key)?;
        let symbol = self.symbol(item, None, None, 0)?;
        let sites_path = match direction {
            CallDirection::Incoming => symbol.location.path.clone(),
            CallDirection::Outgoing | CallDirection::Both => origin?.to_path_buf(),
        };
        let calls = call
            .get("fromRanges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|range| self.location(sites_path.clone(), range))
            .collect();
        Some(CallEdge {
            direction,
            symbol,
            calls,
        })
    }
}

impl SemanticProvider for LspProvider {
    fn workspace_symbols(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        query: &str,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        let session = self.session(ctx, scope, &scope.root)?;
        session.require("workspaceSymbolProvider", "workspace/symbol")?;
        let answer = session.ask("workspace/symbol", &json!({"query": query}))?;
        Ok(answer
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| session.symbol(item, None, None, 0))
            .collect())
    }

    fn document_symbols(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        path: &Path,
    ) -> SemanticResult<Vec<SemanticSymbol>> {
        let session = self.session(ctx, scope, path)?;
        session.require("documentSymbolProvider", "textDocument/documentSymbol")?;
        session.open(path, None)?;
        let params = json!({"textDocument": {"uri": session.uri(path)?}});
        let answer = session.ask("textDocument/documentSymbol", &params)?;
        let mut symbols = Vec::new();
        if let Some(items) = answer.as_array() {
            session.flatten(items, path, None, 0, &mut symbols);
        }
        Ok(symbols)
    }

    fn definition(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        self.positional(
            ctx,
            scope,
            at,
            ("definitionProvider", "textDocument/definition"),
            None,
        )
    }

    fn references(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        include_declaration: bool,
    ) -> SemanticResult<Vec<Location>> {
        let session = self.session(ctx, scope, &at.path)?;
        session.require("referencesProvider", "textDocument/references")?;
        session.open(&at.path, None)?;
        session.open_candidates(at);
        let mut params = session.position_params(at);
        if let Some(object) = params.as_object_mut() {
            object.insert(
                "context".to_owned(),
                json!({"includeDeclaration": include_declaration}),
            );
        }
        let answer = session.ask("textDocument/references", &params)?;
        Ok(session.locations(&answer))
    }

    fn hover(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Option<HoverInfo>> {
        let session = self.session(ctx, scope, &at.path)?;
        session.require("hoverProvider", "textDocument/hover")?;
        session.open(&at.path, None)?;
        let answer = session.ask("textDocument/hover", &session.position_params(at))?;
        let contents = answer.get("contents").map(hover_text).unwrap_or_default();
        let contents = contents.trim();
        if contents.is_empty() {
            return Ok(None);
        }
        let range = answer
            .get("range")
            .and_then(|range| session.location(at.path.clone(), range));
        Ok(Some(HoverInfo {
            contents: contents.to_owned(),
            range,
        }))
    }

    fn implementations(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        self.positional(
            ctx,
            scope,
            at,
            ("implementationProvider", "textDocument/implementation"),
            None,
        )
    }

    fn type_definition(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>> {
        self.positional(
            ctx,
            scope,
            at,
            ("typeDefinitionProvider", "textDocument/typeDefinition"),
            None,
        )
    }

    fn call_hierarchy(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        direction: CallDirection,
    ) -> SemanticResult<Vec<CallEdge>> {
        let session = self.session(ctx, scope, &at.path)?;
        session.require("callHierarchyProvider", "textDocument/prepareCallHierarchy")?;
        session.open(&at.path, None)?;
        let prepared = session.ask(
            "textDocument/prepareCallHierarchy",
            &session.position_params(at),
        )?;
        let Some(item) = prepared.as_array().and_then(|items| items.first()).cloned() else {
            return Ok(Vec::new());
        };
        let origin = item
            .get("uri")
            .and_then(Value::as_str)
            .and_then(uri_to_path);
        let mut edges = Vec::new();
        for side in [CallDirection::Incoming, CallDirection::Outgoing] {
            if direction != CallDirection::Both && direction != side {
                continue;
            }
            let method = if side == CallDirection::Incoming {
                "callHierarchy/incomingCalls"
            } else {
                "callHierarchy/outgoingCalls"
            };
            let answer = session.ask(method, &json!({"item": item}))?;
            edges.extend(
                answer
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|call| session.call_edge(call, side, origin.as_deref())),
            );
        }
        Ok(edges)
    }

    fn code_actions(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        range: &Location,
    ) -> SemanticResult<Vec<CodeActionInfo>> {
        let session = self.session(ctx, scope, &range.path)?;
        session.require("codeActionProvider", "textDocument/codeAction")?;
        session.open(&range.path, None)?;
        let wait = session.remaining().min(ACTION_DIAGNOSTICS_WAIT);
        let first = range.start.line.saturating_sub(1);
        let last = range.end.line.saturating_sub(1).max(first);
        let known: Vec<Value> = session
            .published(&range.path, wait)?
            .unwrap_or_default()
            .iter()
            .filter(|item| item.range.start.line <= last && item.range.end.line >= first)
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();
        let params = json!({
            "textDocument": {"uri": session.uri(&range.path)?},
            "range": session.range_value(range),
            "context": {"diagnostics": known},
        });
        let answer = session.ask("textDocument/codeAction", &params)?;
        let text_of = |path: &Path| session.text(path);
        Ok(answer
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| code_action(item, session.unit, &text_of))
            .collect())
    }

    fn diagnostics(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        path: &Path,
        text: Option<&str>,
    ) -> SemanticResult<Vec<SemanticDiagnostic>> {
        let session = self.session(ctx, scope, path)?;
        session.open(path, text)?;
        let items = if session
            .server
            .supports("diagnosticProvider", "textDocument/diagnostic")
        {
            let params = json!({"textDocument": {"uri": session.uri(path)?}});
            let report = session.ask("textDocument/diagnostic", &params)?;
            report
                .get("items")
                .and_then(Value::as_array)
                .map(|items| crate::diagnostics::parse_items(items))
                .unwrap_or_default()
        } else {
            session
                .published(path, session.remaining())?
                .ok_or_else(|| session.loading_error_for_diagnostics())?
        };
        Ok(items
            .iter()
            .map(|item| session.diagnostic(path, item))
            .collect())
    }

    fn rename(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        new_name: &str,
    ) -> SemanticResult<WorkspaceChange> {
        let session = self.session(ctx, scope, &at.path)?;
        session.require("renameProvider", "textDocument/rename")?;
        session.open(&at.path, None)?;
        session.open_candidates(at);
        let mut params = session.position_params(at);
        if let Some(object) = params.as_object_mut() {
            object.insert("newName".to_owned(), Value::String(new_name.to_owned()));
        }
        let answer = session.ask("textDocument/rename", &params)?;
        let text_of = |path: &Path| session.text(path);
        workspace_change(&answer, session.unit, &text_of)
    }

    fn server_status(&self, ctx: &ExecCtx, language: Option<&LanguageId>) -> Vec<ServerStatus> {
        let settings = self.pool.settings();
        let languages: Vec<String> = match language {
            Some(language) => vec![config::normalize_language(language.as_str())],
            None => settings
                .languages()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        let cwd = PathBuf::from(ctx.options().base.clone().unwrap_or_else(|| ".".to_owned()));
        let running = self.pool.status();
        languages
            .iter()
            .filter_map(|name| {
                let id = LanguageId::new(name.clone()).ok()?;
                let program = settings
                    .command(name)
                    .and_then(|command| command.first())
                    .cloned();
                let server = program.as_deref().map(config::program_name);
                let allowed = program
                    .as_deref()
                    .is_some_and(|program| settings.allows(name, program));
                let found = self.pool.command_for(name, &cwd).is_ok();
                let active = running.iter().find(|status| {
                    Some(&status.server) == server.as_ref()
                        && (status.languages.is_empty()
                            || status.languages.contains(name)
                            || shares_server(settings, name, &status.languages))
                });
                Some(ServerStatus {
                    language: id,
                    server,
                    found,
                    allowed,
                    running: active.is_some(),
                    pid: active.and_then(|status| status.pid),
                    root: active.map(|status| status.root.clone()),
                    loading: active.is_some_and(|status| status.loading),
                })
            })
            .collect()
    }
}

impl Session<'_> {
    fn loading_error_for_diagnostics(&self) -> SemanticError {
        if self.cancel().is_cancelled() {
            return SemanticError::new(SemanticErrorKind::Cancelled, "the request was cancelled");
        }
        SemanticError::new(
            SemanticErrorKind::TimedOut,
            format!(
                "language server {} published no diagnostics within the time limit",
                self.server.name()
            ),
        )
        .with_server(self.server.name())
    }
}

/// Returns `true` when `language` uses the same server command as one of
/// `served`.
fn shares_server(settings: &ClientSettings, language: &str, served: &[String]) -> bool {
    let command = settings.command(language);
    served
        .iter()
        .any(|other| settings.command(other) == command)
}

fn code_action(
    item: &Value,
    unit: PositionUnit,
    text_of: &dyn Fn(&Path) -> Option<String>,
) -> Option<CodeActionInfo> {
    let title = item.get("title")?.as_str()?.to_owned();
    if let Some(command) = item.get("command").and_then(Value::as_str) {
        return Some(CodeActionInfo {
            title,
            kind: None,
            preferred: false,
            edit: None,
            command: Some(command.to_owned()),
        });
    }
    let edit = item
        .get("edit")
        .and_then(|edit| workspace_change(edit, unit, text_of).ok());
    Some(CodeActionInfo {
        title,
        kind: item.get("kind").and_then(Value::as_str).map(str::to_owned),
        preferred: item.get("isPreferred").and_then(Value::as_bool) == Some(true),
        edit,
        command: item
            .pointer("/command/command")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(members) => members.is_empty(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text of the `contents` of a hover answer: Markdown or plain text, a
/// marked string, or a list of them separated by blank lines.
#[must_use]
pub fn hover_text(contents: &Value) -> String {
    match contents {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(hover_text)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(_) => {
            let value = contents
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match contents.get("language").and_then(Value::as_str) {
                Some(language) => format!("```{language}\n{value}\n```"),
                None => value.to_owned(),
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => String::new(),
    }
}

fn severity(severity: Option<LspSeverity>) -> DiagnosticSeverity {
    match severity {
        Some(LspSeverity::WARNING) => DiagnosticSeverity::Warning,
        Some(LspSeverity::INFORMATION) => DiagnosticSeverity::Info,
        Some(LspSeverity::HINT) => DiagnosticSeverity::Hint,
        _ => DiagnosticSeverity::Error,
    }
}

/// Kind of the code group for a protocol `SymbolKind` number.
///
/// The numbers are the values of the `SymbolKind` enumeration of the
/// Language Server Protocol 3.17 specification.
#[must_use]
pub fn symbol_kind(kind: u64) -> SymbolKind {
    match kind {
        1..=4 => SymbolKind::Module,
        5 => SymbolKind::Class,
        6 | 9 => SymbolKind::Method,
        7 | 8 | 20 | 22 => SymbolKind::Field,
        10 => SymbolKind::Enum,
        11 => SymbolKind::Interface,
        12 | 24 | 25 => SymbolKind::Function,
        13 | 15..=19 | 21 => SymbolKind::Variable,
        14 => SymbolKind::Constant,
        23 => SymbolKind::Struct,
        26 => SymbolKind::Type,
        _ => SymbolKind::Other,
    }
}

fn line_text(text: &str, line: u32) -> Option<&str> {
    let index = usize::try_from(line.checked_sub(1)?).ok()?;
    text.split('\n')
        .nth(index)
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

fn width(character: char, unit: PositionUnit) -> u32 {
    let units = match unit {
        PositionUnit::Utf8 => character.len_utf8(),
        PositionUnit::Utf16 => character.len_utf16(),
        PositionUnit::Utf32 => 1,
    };
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// Protocol position of a one-based line and character column.
#[must_use]
pub fn to_lsp(text: Option<&str>, position: TextPosition, unit: PositionUnit) -> Value {
    let line = position.line.saturating_sub(1);
    let columns = usize::try_from(position.column.saturating_sub(1)).unwrap_or(usize::MAX);
    let character = match text.and_then(|text| line_text(text, position.line)) {
        Some(line_text) => {
            let counted: u32 = line_text
                .chars()
                .take(columns)
                .map(|character| width(character, unit))
                .sum();
            let beyond = columns.saturating_sub(line_text.chars().count());
            counted.saturating_add(u32::try_from(beyond).unwrap_or(u32::MAX))
        }
        None => u32::try_from(columns).unwrap_or(u32::MAX),
    };
    json!({"line": line, "character": character})
}

/// One-based line and character column of a protocol position.
#[must_use]
pub fn from_lsp(text: Option<&str>, position: &Value, unit: PositionUnit) -> Option<TextPosition> {
    let line = u32::try_from(position.get("line")?.as_u64()?)
        .ok()?
        .saturating_add(1);
    let character = u32::try_from(position.get("character")?.as_u64()?).ok()?;
    let column = match text.and_then(|text| line_text(text, line)) {
        Some(line_text) => {
            let mut units = 0u32;
            let mut columns = 0u32;
            for current in line_text.chars() {
                if units >= character {
                    break;
                }
                units = units.saturating_add(width(current, unit));
                columns += 1;
            }
            columns.saturating_add(character.saturating_sub(units))
        }
        None => character,
    };
    Some(TextPosition {
        line,
        column: column.saturating_add(1),
    })
}

/// Identifier that contains or starts at a position.
#[must_use]
pub fn word_at(text: &str, position: TextPosition) -> Option<String> {
    let line = line_text(text, position.line)?;
    let characters: Vec<char> = line.chars().collect();
    let is_ident =
        |character: &char| character.is_alphanumeric() || *character == '_' || *character == '$';
    let mut index = usize::try_from(position.column.checked_sub(1)?).ok()?;
    if index >= characters.len() || !is_ident(&characters[index]) {
        index = index
            .checked_sub(1)
            .filter(|before| characters.get(*before).is_some_and(is_ident))?;
    }
    let start = characters[..index]
        .iter()
        .rposition(|character| !is_ident(character))
        .map_or(0, |found| found + 1);
    let end = characters[index..]
        .iter()
        .position(|character| !is_ident(character))
        .map_or(characters.len(), |found| index + found);
    let word: String = characters[start..end].iter().collect();
    (!word.is_empty()).then_some(word)
}
