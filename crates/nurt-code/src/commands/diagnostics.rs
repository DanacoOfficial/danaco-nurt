// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.diagnostics` stage.
//!
//! Diagnostics of each file come from its language server
//! ([`SemanticProvider::diagnostics`]): servers with pull diagnostics are
//! asked, the others publish them. Under `source: auto` a file without a
//! usable server is checked by syntax analysis: the grammar of its language
//! or, for structured documents (JSON, YAML, TOML and the other document
//! formats), the parser of the format. Each distinct cause of a fallback is
//! reported once as a `semantic_fallback` note.
//!
//! Directories are expanded to the source files and documents below them.
//! With `changed: true` only files changed in the repository (working tree,
//! index and untracked files, from the repository query service) are
//! checked. Records are free of repetitions and ordered by severity, path
//! and line.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Diagnostic, DiagnosticSeverity, RelatedLocation};
use nurt_contracts::services::{GitQueryService, IssueSeverity, LanguageId, SyntaxIssue};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use crate::commands::definition::shared::{self, Files, Source};
use crate::semantic::{SemanticDiagnostic, SemanticErrorKind, SemanticProvider, SemanticScope};

const COMMAND: &str = "code.diagnostics";

/// Source of diagnostics found by syntax analysis.
const SYNTAX_SOURCE: &str = "syntax";

/// Source of diagnostics of a server that names no tool.
const SERVER_SOURCE: &str = "lsp";

/// Number of files above which progress is reported.
const PROGRESS_FILES_MIN: usize = 5;

/// Implementation of `code.diagnostics`.
///
/// ```
/// use nurt_code::commands::diagnostics::CodeDiagnostics;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeDiagnostics::new(None, None);
/// assert_eq!(stage.descriptor().command, "code.diagnostics");
/// ```
pub struct CodeDiagnostics {
    semantic: Option<Arc<dyn SemanticProvider>>,
    git: Option<Arc<dyn GitQueryService>>,
}

struct Request {
    source: Source,
    minimum: DiagnosticSeverity,
    requested: Option<LanguageId>,
    root: Option<PathBuf>,
}

impl CodeDiagnostics {
    /// Stage that asks `semantic` first and lists changed files through
    /// `git`; without a provider every answer comes from syntax analysis,
    /// without a repository service `changed: true` fails with
    /// `E_EXTERNAL_MISSING`.
    #[must_use]
    pub fn new(
        semantic: Option<Arc<dyn SemanticProvider>>,
        git: Option<Arc<dyn GitQueryService>>,
    ) -> Self {
        Self { semantic, git }
    }

    fn records(&self, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
        let files = Files::new(ctx);
        let root = match shared::path_arg(args, "root")? {
            Some(text) => Some(shared::resolve_path(ctx, &text)?),
            None => None,
        };
        let request = Request {
            source: Source::from_args(args)?,
            minimum: minimum_severity(args)?,
            requested: shared::requested_language(args)?,
            root,
        };
        let paths = self.paths(args, ctx, &files)?;
        let mut reasons = BTreeSet::new();
        let mut items = Vec::new();
        for (done, path) in paths.iter().enumerate() {
            ctx.checkpoint()?;
            items.extend(self.file_diagnostics(ctx, &files, &request, path, &mut reasons)?);
            if paths.len() > PROGRESS_FILES_MIN {
                ctx.progress(
                    "check",
                    u64::try_from(done + 1).unwrap_or(u64::MAX),
                    u64::try_from(paths.len()).ok(),
                );
            }
        }
        for reason in reasons {
            shared::note_fallback(ctx, &reason);
        }
        items.retain(|item| rank(item.severity) <= rank(request.minimum));
        items.sort_by(|left, right| {
            (
                rank(left.severity),
                left.path.as_str(),
                left.line,
                left.column,
            )
                .cmp(&(
                    rank(right.severity),
                    right.path.as_str(),
                    right.line,
                    right.column,
                ))
        });
        items.dedup_by(|left, right| {
            left.path == right.path
                && left.line == right.line
                && left.column == right.column
                && left.message == right.message
        });
        Ok(items.into_iter().map(Record::from).collect())
    }

    /// Files to check: the given files, the checkable files below the given
    /// directories, limited to changed files with `changed: true`.
    fn paths(&self, args: &BoundArgs, ctx: &ExecCtx, files: &Files<'_>) -> Result<Vec<PathBuf>> {
        let given: Vec<PathBuf> = shared::paths_arg(args, "path")?
            .iter()
            .map(|text| shared::resolve_path(ctx, text))
            .collect::<Result<_>>()?;
        let changed = args.bool("changed")?.unwrap_or(false);
        if !changed {
            if given.is_empty() {
                return Err(Error::new(
                    ErrorCode::MissingParam,
                    format!("`{COMMAND}` needs a path or changed: true"),
                ));
            }
            return Ok(expand(ctx, files, &given));
        }
        let changed = self.changed_files(ctx, files, &given)?;
        Ok(changed
            .into_iter()
            .filter(|path| given.is_empty() || given.iter().any(|base| path.starts_with(base)))
            .filter(|path| checkable(ctx, path))
            .collect())
    }

    fn changed_files(
        &self,
        ctx: &ExecCtx,
        files: &Files<'_>,
        given: &[PathBuf],
    ) -> Result<Vec<PathBuf>> {
        let git = self.git.as_ref().ok_or_else(|| {
            Error::new(
                ErrorCode::ExternalMissing,
                "changed: true needs repository queries, which are not available in this engine; give the paths instead",
            )
        })?;
        let start = match (given.first(), ctx.options().base.as_deref()) {
            (Some(path), _) => path.clone(),
            (None, Some(base)) => PathBuf::from(base),
            (None, None) => {
                return Err(Error::new(
                    ErrorCode::Arg,
                    "changed: true needs a path or a base directory inside the repository",
                ));
            }
        };
        let root = git.repo_root(&start)?.ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "{} is not inside a repository; give the paths instead of changed: true",
                    start.display()
                ),
            )
        })?;
        let mut paths: Vec<PathBuf> = git
            .changed_files(&root, None)?
            .iter()
            .map(|relative| root.join(relative))
            .filter(|path| files.exists(path) && !files.is_dir(path))
            .collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn file_diagnostics(
        &self,
        ctx: &ExecCtx,
        files: &Files<'_>,
        request: &Request,
        path: &Path,
        reasons: &mut BTreeSet<String>,
    ) -> Result<Vec<Diagnostic>> {
        let file = files.read(path)?;
        let language = request.requested.clone().or_else(|| {
            ctx.code()
                .and_then(|code| code.language_of(Some(path), file.text.as_bytes()))
        });
        let language_name = language
            .as_ref()
            .map(|language| language.as_str().to_owned());
        if request.source != Source::Syntax {
            let scope = SemanticScope {
                root: request
                    .root
                    .clone()
                    .unwrap_or_else(|| shared::project_root(files, path)),
                language: request.requested.clone(),
                timeout: None,
            };
            match self.served(ctx, &scope, path, request.source)? {
                Ok(items) => {
                    return Ok(items
                        .into_iter()
                        .map(|item| served_record(path, item, language_name.clone()))
                        .collect());
                }
                Err(reason) => {
                    reasons.insert(reason);
                }
            }
        }
        Ok(
            syntax_issues(ctx, path, file.text.as_bytes(), language.as_ref())
                .into_iter()
                .map(|issue| syntax_record(path, &issue, language_name.clone()))
                .collect(),
        )
    }

    /// Diagnostics of the server, or the cause of a fallback.
    fn served(
        &self,
        ctx: &ExecCtx,
        scope: &SemanticScope,
        path: &Path,
        source: Source,
    ) -> Result<std::result::Result<Vec<SemanticDiagnostic>, String>> {
        let Some(provider) = self.semantic.as_deref() else {
            if source == Source::Lsp {
                return Err(Error::new(
                    ErrorCode::ExternalMissing,
                    "language servers are not available in this engine; use source: syntax",
                ));
            }
            return Ok(Err("language servers are not available".to_owned()));
        };
        match provider.diagnostics(ctx, scope, path, None) {
            Ok(items) => Ok(Ok(items)),
            Err(error) if source == Source::Auto && error.kind != SemanticErrorKind::Cancelled => {
                Ok(Err(error.to_string()))
            }
            Err(error) => Err(error.to_error()),
        }
    }
}

fn minimum_severity(args: &BoundArgs) -> Result<DiagnosticSeverity> {
    match shared::word_arg(args, "severity")?.as_deref() {
        None | Some("hint") => Ok(DiagnosticSeverity::Hint),
        Some("error") => Ok(DiagnosticSeverity::Error),
        Some("warning") => Ok(DiagnosticSeverity::Warning),
        Some("info") => Ok(DiagnosticSeverity::Info),
        Some(other) => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("unknown severity `{other}`; use error, warning, info or hint"),
        )),
    }
}

fn rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 2,
        _ => 3,
    }
}

/// Returns `true` for files with a grammar or a document format.
fn checkable(ctx: &ExecCtx, path: &Path) -> bool {
    let code = ctx
        .code()
        .is_some_and(|code| code.language_of(Some(path), &[]).is_some());
    code || ctx
        .doc()
        .is_some_and(|doc| doc.detect_format(Some(path), &[]).is_some())
}

fn expand(ctx: &ExecCtx, files: &Files<'_>, given: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for path in given {
        if files.is_dir(path) {
            let (listed, _) = files.walk(path, &|candidate: &Path| checkable(ctx, candidate));
            paths.extend(listed);
        } else {
            paths.push(path.clone());
        }
    }
    paths.dedup();
    paths
}

fn syntax_issues(
    ctx: &ExecCtx,
    path: &Path,
    bytes: &[u8],
    language: Option<&LanguageId>,
) -> Vec<SyntaxIssue> {
    if let (Some(code), Some(language)) = (ctx.code(), language)
        && let Ok(issues) = code.check_syntax(bytes, language)
    {
        return issues;
    }
    ctx.doc()
        .and_then(|doc| {
            let format = doc.detect_format(Some(path), bytes)?;
            doc.check_syntax(bytes, &format).ok()
        })
        .unwrap_or_default()
}

fn served_record(path: &Path, item: SemanticDiagnostic, language: Option<String>) -> Diagnostic {
    let related: Vec<RelatedLocation> = item
        .related
        .iter()
        .map(|related| RelatedLocation {
            path: shared::record_path(&related.location.path),
            line: u64::from(related.location.start.line),
            end_line: Some(u64::from(related.location.end.line)),
            message: Some(related.message.clone()),
        })
        .collect();
    Diagnostic {
        path: shared::record_path(path),
        line: u64::from(item.range.start.line),
        column: u64::from(item.range.start.column),
        severity: item.severity,
        message: item.message,
        source: item.source.unwrap_or_else(|| SERVER_SOURCE.to_owned()),
        code: item.code,
        end_line: Some(u64::from(item.range.end.line)),
        end_column: Some(u64::from(item.range.end.column)),
        language,
        related: (!related.is_empty()).then_some(related),
    }
}

fn syntax_record(path: &Path, issue: &SyntaxIssue, language: Option<String>) -> Diagnostic {
    Diagnostic {
        path: shared::record_path(path),
        line: u64::from(issue.span.start.line),
        column: u64::from(issue.span.start.column),
        severity: if issue.severity == IssueSeverity::Warning {
            DiagnosticSeverity::Warning
        } else {
            DiagnosticSeverity::Error
        },
        message: issue.message.clone(),
        source: SYNTAX_SOURCE.to_owned(),
        code: issue.kind.clone(),
        end_line: Some(u64::from(issue.span.end.line)),
        end_column: Some(u64::from(issue.span.end.column)),
        language,
        related: None,
    }
}

impl StageImpl for CodeDiagnostics {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CodeDiagnostics {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let records = self.records(args, ctx)?;
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}
