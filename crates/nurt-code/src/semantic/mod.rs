// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface to semantic analysis provided by language servers.
//!
//! The commands of the code group receive an optional
//! [`SemanticProvider`] when they are created. With a provider and a server
//! for the language, a command answers from the server (`source: lsp`);
//! otherwise, or when the provider reports [`SemanticErrorKind::Unavailable`],
//! [`SemanticErrorKind::Failed`] or [`SemanticErrorKind::TimedOut`] under
//! `source: auto`, the command answers from syntax analysis and reports the
//! reason as `semantic_fallback`.
//!
//! Positions are one-based lines and one-based columns counted in Unicode
//! scalar values ([`TextPosition`]); a provider converts them to the
//! encoding negotiated with the server. Paths are absolute.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::record::{DiagnosticSeverity, SymbolKind};
use nurt_contracts::services::{LanguageId, TextPosition};
use nurt_contracts::stage::ExecCtx;

/// Project and limits of one semantic request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticScope {
    /// Project root the server is started for.
    pub root: PathBuf,
    /// Language of the request; `None` lets the provider choose by file
    /// name.
    pub language: Option<LanguageId>,
    /// Time limit of the request; `None` uses `code.timeout`.
    pub timeout: Option<Duration>,
}

impl SemanticScope {
    /// Scope of `root` with the language chosen by file name and the
    /// configured time limit.
    ///
    /// ```
    /// use nurt_code::semantic::SemanticScope;
    ///
    /// let scope = SemanticScope::new("/work/project");
    /// assert!(scope.language.is_none());
    /// assert!(scope.timeout.is_none());
    /// ```
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            language: None,
            timeout: None,
        }
    }
}

/// Position in a file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocumentPosition {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Position in the file.
    pub position: TextPosition,
}

/// Region of a file; `end` is the position after the last character.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    /// Absolute path of the file.
    pub path: PathBuf,
    /// Position of the first character.
    pub start: TextPosition,
    /// Position after the last character.
    pub end: TextPosition,
}

/// Symbol reported by a language server.
///
/// Hierarchical document symbols are flattened in source order; `container`
/// and `depth` keep the hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSymbol {
    /// Name as reported by the server.
    pub name: String,
    /// Kind, mapped from the protocol symbol kind.
    pub kind: SymbolKind,
    /// Qualified name of the enclosing symbol (`Outer.Inner`), if any.
    pub container: Option<String>,
    /// Nesting level, 0 for top-level symbols.
    pub depth: u32,
    /// Whole definition, including its body.
    pub location: Location,
    /// Position of the name.
    pub name_position: TextPosition,
    /// Detail reported by the server, usually the signature.
    pub detail: Option<String>,
}

/// Hover information at a position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    /// Content as plain text or Markdown, as sent by the server.
    pub contents: String,
    /// Region the information applies to.
    pub range: Option<Location>,
}

/// Direction of a call hierarchy request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallDirection {
    /// Functions that call the symbol.
    Incoming,
    /// Functions called by the symbol.
    Outgoing,
    /// Both directions.
    Both,
}

/// One edge of a call hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEdge {
    /// [`CallDirection::Incoming`] for a caller, [`CallDirection::Outgoing`]
    /// for a callee.
    pub direction: CallDirection,
    /// The caller or the callee.
    pub symbol: SemanticSymbol,
    /// Call sites; their number is the multiplicity of the edge.
    pub calls: Vec<Location>,
}

/// Replacement of a region of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChange {
    /// Region replaced.
    pub range: Location,
    /// Replacement text.
    pub new_text: String,
}

/// Change of a file system resource requested by a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceChange {
    /// Creates a file.
    Create {
        /// File created.
        path: PathBuf,
        /// Replace an existing file.
        overwrite: bool,
        /// Do nothing when the file exists.
        ignore_if_exists: bool,
    },
    /// Renames a file or a directory.
    Rename {
        /// Current path.
        from: PathBuf,
        /// New path.
        to: PathBuf,
        /// Replace an existing target.
        overwrite: bool,
        /// Do nothing when the target exists.
        ignore_if_exists: bool,
    },
    /// Deletes a file or a directory.
    Delete {
        /// Path deleted.
        path: PathBuf,
        /// Delete a directory with its content.
        recursive: bool,
        /// Do nothing when the path does not exist.
        ignore_if_not_exists: bool,
    },
}

/// One change of a [`WorkspaceChange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentChange {
    /// Change of file content.
    Text(TextChange),
    /// Change of a file system resource.
    Resource(ResourceChange),
}

/// Changes of several files, in the order the server requires them to be
/// applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceChange {
    /// Changes in application order.
    pub changes: Vec<DocumentChange>,
}

/// Code action offered by a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeActionInfo {
    /// Title shown to the user.
    pub title: String,
    /// Protocol kind, for example `quickfix` or `refactor.extract`.
    pub kind: Option<String>,
    /// The server marks the action as preferred.
    pub preferred: bool,
    /// Changes of the action, when the server sends them with the action.
    pub edit: Option<WorkspaceChange>,
    /// Server command run by the action, when it has one.
    pub command: Option<String>,
}

/// Related position of a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedInfo {
    /// Region.
    pub location: Location,
    /// Message.
    pub message: String,
}

/// Diagnostic reported by a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticDiagnostic {
    /// Region of the problem.
    pub range: Location,
    /// Severity.
    pub severity: DiagnosticSeverity,
    /// Message.
    pub message: String,
    /// Code of the diagnostic.
    pub code: Option<String>,
    /// Tool that reported it inside the server (for example `pyright`).
    pub source: Option<String>,
    /// Related positions.
    pub related: Vec<RelatedInfo>,
}

/// State of the language server of one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    /// Language.
    pub language: LanguageId,
    /// Server program configured for the language, if any.
    pub server: Option<String>,
    /// The program is found on this computer.
    pub found: bool,
    /// The server may be started under `code.servers_allow`.
    pub allowed: bool,
    /// A server process is running.
    pub running: bool,
    /// Process identifier of the running server.
    pub pid: Option<u32>,
    /// Project root of the running server.
    pub root: Option<PathBuf>,
    /// The running server is still loading the project.
    pub loading: bool,
}

/// Category of a [`SemanticError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SemanticErrorKind {
    /// No server for the language: none configured, not installed, not
    /// allowed, the pool is full, or a recent start failed.
    Unavailable,
    /// The server failed, ended or broke the protocol.
    Failed,
    /// The server did not answer within the time limit.
    TimedOut,
    /// The request was cancelled.
    Cancelled,
    /// The server does not support the request.
    Unsupported,
}

/// Failure of a semantic request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    /// Category.
    pub kind: SemanticErrorKind,
    /// Description in English, used as the fallback reason.
    pub message: String,
    /// Server program concerned.
    pub server: Option<String>,
    /// How to install or enable the server, for [`SemanticErrorKind::Unavailable`].
    pub fix: Option<String>,
}

impl SemanticError {
    /// Error of the given kind and message.
    ///
    /// ```
    /// use nurt_code::semantic::{SemanticError, SemanticErrorKind};
    ///
    /// let error = SemanticError::new(SemanticErrorKind::TimedOut, "no answer in 20s");
    /// assert!(error.allows_fallback());
    /// ```
    pub fn new(kind: SemanticErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            server: None,
            fix: None,
        }
    }

    /// Sets the server program.
    #[must_use]
    pub fn with_server(mut self, server: impl Into<String>) -> Self {
        self.server = Some(server.into());
        self
    }

    /// Sets the installation hint.
    #[must_use]
    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    /// Returns `true` when `source: auto` answers from syntax analysis
    /// instead of failing: the server is unavailable, failed, timed out or
    /// does not support the request. A cancellation is never replaced.
    #[must_use]
    pub fn allows_fallback(&self) -> bool {
        !matches!(self.kind, SemanticErrorKind::Cancelled)
    }

    /// Error of the language for a command that requires the server
    /// (`source: lsp`): `E_EXTERNAL_MISSING` with the installation hint,
    /// `E_EXTERNAL_FAILED`, `E_TIMEOUT` or `E_CANCELLED`.
    ///
    /// ```
    /// use nurt_code::semantic::{SemanticError, SemanticErrorKind};
    /// use nurt_contracts::error::ErrorCode;
    ///
    /// let error = SemanticError::new(SemanticErrorKind::Unavailable, "gopls is not installed")
    ///     .with_fix("go install golang.org/x/tools/gopls@latest");
    /// assert_eq!(error.to_error().code(), ErrorCode::ExternalMissing);
    /// ```
    #[must_use]
    pub fn to_error(&self) -> Error {
        let code = match self.kind {
            SemanticErrorKind::Unavailable | SemanticErrorKind::Unsupported => {
                ErrorCode::ExternalMissing
            }
            SemanticErrorKind::Failed => ErrorCode::ExternalFailed,
            SemanticErrorKind::TimedOut => ErrorCode::Timeout,
            SemanticErrorKind::Cancelled => ErrorCode::Cancelled,
        };
        let mut error = Error::new(code, self.message.clone());
        if let Some(server) = &self.server {
            error = error.with_found(server.clone());
        }
        if let Some(fix) = &self.fix {
            error = error.with_fix(fix.clone());
        }
        error
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.server {
            Some(server) => write!(f, "{server}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for SemanticError {}

/// Result of a semantic request.
pub type SemanticResult<T> = Result<T, SemanticError>;

/// Semantic analysis backed by language servers.
///
/// Every method blocks until the server answers, the time limit of
/// [`SemanticScope::timeout`] (or `code.timeout`) passes, or the context is
/// cancelled. Methods do not change files: [`SemanticProvider::rename`] and
/// [`SemanticProvider::code_actions`] return changes for the caller to
/// apply through a mutation transaction.
pub trait SemanticProvider: Send + Sync {
    /// Symbols of the project whose names match `query` as the server
    /// matches them.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn workspace_symbols(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        query: &str,
    ) -> SemanticResult<Vec<SemanticSymbol>>;

    /// Symbols of one file in source order.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn document_symbols(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        path: &Path,
    ) -> SemanticResult<Vec<SemanticSymbol>>;

    /// Definitions of the symbol at a position.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn definition(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>>;

    /// References to the symbol at a position.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn references(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        include_declaration: bool,
    ) -> SemanticResult<Vec<Location>>;

    /// Hover information at a position; `None` when the server has none.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn hover(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Option<HoverInfo>>;

    /// Implementations of the interface or method at a position.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn implementations(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>>;

    /// Definitions of the type of the symbol at a position.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn type_definition(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
    ) -> SemanticResult<Vec<Location>>;

    /// Callers, callees or both of the function at a position.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn call_hierarchy(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        direction: CallDirection,
    ) -> SemanticResult<Vec<CallEdge>>;

    /// Code actions available for a region.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn code_actions(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        range: &Location,
    ) -> SemanticResult<Vec<CodeActionInfo>>;

    /// Diagnostics of a file; with `text`, of that content instead of the
    /// content on disk, so a change can be checked before it is written.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers.
    fn diagnostics(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        path: &Path,
        text: Option<&str>,
    ) -> SemanticResult<Vec<SemanticDiagnostic>>;

    /// Changes that rename the symbol at a position to `new_name`.
    ///
    /// # Errors
    ///
    /// Fails with a [`SemanticError`] when no server answers or the server
    /// refuses the rename.
    fn rename(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        at: &DocumentPosition,
        new_name: &str,
    ) -> SemanticResult<WorkspaceChange>;

    /// State of the servers: of `language`, or of every configured language
    /// when `None`. Does not start servers and does not wait for requests in
    /// progress.
    fn server_status(&self, ctx: &ExecCtx, language: Option<&LanguageId>) -> Vec<ServerStatus>;
}
