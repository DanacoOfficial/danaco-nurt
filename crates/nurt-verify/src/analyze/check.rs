// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The checks of `analyze.check`: external tools and built-in analyses.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration as StdDuration;

use nurt_contracts::error::Result;
use nurt_contracts::record::{Analysis, AnalysisCheck, CheckStatus, Finding, FindingKind};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;
use nurt_contracts::value::{Duration, Instant, Path as RecordPath};

use crate::analyze::execute::run_captured;
use crate::analyze::scope::Scope;
use crate::analyze::tools::{PATH_ARGUMENTS_MAX, Tool, candidates, resolve_program};
use crate::analyze::verdict::{Counts, FINDINGS_STORED_MAX, FailOn, order_findings};
use crate::complexity;
use crate::duplication::{self, DUPLICATION_MIN_LINES, SourceTokens};

/// Largest source file read by the built-in checks, in bytes.
const SOURCE_MAX_BYTES: u64 = 512_000;

#[allow(
    clippy::expect_used,
    reason = "the pattern is a constant checked by the tests"
)]
static TEST_FILE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(^|/)(tests?|__tests__|fixtures)/|\.(test|spec)\.[a-z]+$|(^|/)test_[^/]*\.py$|_test\.(go|py)$",
    )
    .expect("constant pattern")
});

/// Parameters of a check run.
#[derive(Debug, Clone)]
pub(crate) struct CheckRequest {
    /// Project directory.
    pub(crate) dir: PathBuf,
    /// Paths as given, for the record.
    pub(crate) paths: Vec<String>,
    /// Only changed files.
    pub(crate) changed_only: bool,
    /// Checks requested; `None` runs every available check.
    pub(crate) checks: Option<Vec<FindingKind>>,
    /// Test files take part in the duplication check.
    pub(crate) include_tests: bool,
    /// Highest complexity accepted.
    pub(crate) complexity_max: u32,
    /// Lowest severity that fails the analysis.
    pub(crate) fail_on: FailOn,
}

/// Result of a check run, before the identifier is assigned.
#[derive(Debug, Clone)]
pub(crate) struct CheckOutcome {
    /// Header record.
    pub(crate) analysis: Analysis,
    /// All findings, ordered.
    pub(crate) findings: Vec<Finding>,
    /// Remarks for the report.
    pub(crate) notes: Vec<NotableEntry>,
}

struct ToolReport {
    check: AnalysisCheck,
    findings: Vec<Finding>,
    note: Option<NotableEntry>,
}

fn millis(duration: StdDuration) -> Duration {
    Duration::from_millis(i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}

fn missing(detail: String) -> NotableEntry {
    let mut entry = NotableEntry::new(Notable::ToolMissing);
    entry.detail = Some(detail);
    entry
}

fn wanted(request: &CheckRequest, kind: FindingKind) -> bool {
    request
        .checks
        .as_ref()
        .is_none_or(|checks| checks.contains(&kind))
}

/// Paths passed to a tool: `None` skips it, an empty list checks the project.
fn tool_paths(tool: Tool, scope: &Scope) -> Option<Vec<String>> {
    if !scope.scoped {
        return Some(Vec::new());
    }
    let own = scope.with_extensions(tool.extensions());
    if own.is_empty() {
        return None;
    }
    Some(if tool.accepts_paths() && own.len() <= PATH_ARGUMENTS_MAX {
        own
    } else {
        Vec::new()
    })
}

fn run_one(
    ctx: &ExecCtx,
    request: &CheckRequest,
    tool: Tool,
    program: PathBuf,
    paths: &[String],
) -> ToolReport {
    let started = ctx.clock().monotonic();
    let mut command = vec![program.to_string_lossy().into_owned()];
    command.extend(tool.check_args(paths));
    let mut check = AnalysisCheck {
        check: tool.check().to_string(),
        tool: Some(tool.name().to_owned()),
        ..AnalysisCheck::default()
    };
    let mut findings = Vec::new();
    let mut note = None;
    match run_captured(ctx, &request.dir, command, None) {
        Err(error) => {
            check.status = CheckStatus::Failed;
            note = Some(missing(format!(
                "{}: {}",
                tool.name(),
                error.data().message
            )));
        }
        Ok(captured) => {
            findings = tool.parse(&captured.output, &request.dir);
            let silent = findings.is_empty()
                && captured.output.exit_code != Some(0)
                && !(tool.formats() && captured.output.exit_code == Some(1));
            let fails = Counts::of(&findings).passed(request.fail_on);
            check.status = if captured.timed_out || silent || !fails {
                CheckStatus::Failed
            } else {
                CheckStatus::Ok
            };
        }
    }
    check.findings = Some(findings.len() as u64);
    check.duration = Some(millis(ctx.clock().monotonic().saturating_sub(started)));
    ToolReport {
        check,
        findings,
        note,
    }
}

fn external_checks(
    ctx: &ExecCtx,
    request: &CheckRequest,
    scope: &Scope,
) -> (Vec<ToolReport>, Vec<AnalysisCheck>, Vec<NotableEntry>) {
    let search = ctx.host().env_var("PATH");
    let mut jobs: Vec<(Tool, PathBuf, Vec<String>)> = Vec::new();
    let mut absent = Vec::new();
    let mut notes = Vec::new();
    let found = candidates(&request.dir, &scope.extensions());
    for candidate in found.iter().filter(|c| wanted(request, c.tool.check())) {
        let tool = candidate.tool;
        let Some(paths) = tool_paths(tool, scope) else {
            continue;
        };
        match resolve_program(tool, &request.dir, search.as_deref()) {
            Some(program) => jobs.push((tool, program, paths)),
            None => {
                absent.push(status_only(
                    tool.check(),
                    Some(tool.name()),
                    CheckStatus::Missing,
                ));
                notes.push(missing(format!(
                    "{}: program {} not found",
                    tool.name(),
                    tool.program()
                )));
            }
        }
    }
    for kind in [FindingKind::Types, FindingKind::Lint, FindingKind::Format] {
        let named = request.checks.as_ref().is_some_and(|c| c.contains(&kind));
        if named && !found.iter().any(|c| c.tool.check() == kind) {
            absent.push(status_only(kind, None, CheckStatus::Missing));
            notes.push(missing(format!(
                "{kind}: no tool is configured in the project"
            )));
        }
    }
    let reports = std::thread::scope(|threads| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(tool, program, paths)| {
                let handle =
                    threads.spawn(move || run_one(ctx, request, *tool, program.clone(), paths));
                (*tool, handle)
            })
            .collect();
        handles
            .into_iter()
            .map(|(tool, handle)| handle.join().unwrap_or_else(|_| crashed(tool)))
            .collect()
    });
    (reports, absent, notes)
}

/// Report of a tool whose check ended in a panic: the check failed and the
/// note names the tool, so that the analysis does not pass without it.
fn crashed(tool: Tool) -> ToolReport {
    ToolReport {
        check: AnalysisCheck {
            check: tool.check().to_string(),
            tool: Some(tool.name().to_owned()),
            status: CheckStatus::Failed,
            findings: Some(0),
            ..AnalysisCheck::default()
        },
        findings: Vec::new(),
        note: Some(missing(format!(
            "{}: the check ended in a panic",
            tool.name()
        ))),
    }
}

fn status_only(kind: FindingKind, tool: Option<&str>, status: CheckStatus) -> AnalysisCheck {
    AnalysisCheck {
        check: kind.to_string(),
        tool: tool.map(str::to_owned),
        status,
        ..AnalysisCheck::default()
    }
}

struct BuiltinResult {
    findings: Vec<Finding>,
    checks: Vec<AnalysisCheck>,
    notes: Vec<NotableEntry>,
    /// Share of duplicated lines; `None` when the duplication check did not
    /// run.
    duplication_percent: Option<f64>,
}

fn builtin_checks(ctx: &ExecCtx, request: &CheckRequest, scope: &Scope) -> BuiltinResult {
    let mut result = BuiltinResult {
        findings: Vec::new(),
        checks: Vec::new(),
        notes: Vec::new(),
        duplication_percent: None,
    };
    let kinds: Vec<FindingKind> = [FindingKind::Complexity, FindingKind::Duplication]
        .into_iter()
        .filter(|kind| request.checks.as_ref().is_none_or(|c| c.contains(kind)))
        .collect();
    if kinds.is_empty() {
        return result;
    }
    let Some(code) = ctx.code() else {
        for kind in kinds {
            result
                .checks
                .push(status_only(kind, None, CheckStatus::Missing));
            result.notes.push(missing(format!(
                "{kind}: the code service is not available"
            )));
        }
        return result;
    };
    let mut tokens: Vec<SourceTokens> = Vec::new();
    for file in &scope.files {
        let path = request.dir.join(file);
        let Ok(bytes) = read_source(&path) else {
            continue;
        };
        let Some(language) = code.language_of(Some(&path), &bytes) else {
            continue;
        };
        if kinds.contains(&FindingKind::Complexity)
            && let Ok(measured) =
                complexity::measure_file(code, file, &bytes, &language, request.complexity_max)
        {
            result.findings.extend(measured.findings);
        }
        let skip_tests = !request.include_tests && TEST_FILE.is_match(file);
        if kinds.contains(&FindingKind::Duplication)
            && !skip_tests
            && let Ok(source) = duplication::tokenize_file(code, file, &bytes, &language)
        {
            tokens.push(source);
        }
    }
    for kind in &kinds {
        let count = result.findings.iter().filter(|f| f.kind == *kind).count();
        let mut check = status_only(*kind, Some(complexity::TOOL), CheckStatus::Ok);
        if *kind == FindingKind::Complexity {
            check.findings = Some(count as u64);
            if count > 0 {
                check.status = CheckStatus::Failed;
            }
        }
        result.checks.push(check);
    }
    if kinds.contains(&FindingKind::Duplication) {
        let report = duplication::detect(&tokens, DUPLICATION_MIN_LINES);
        let own = result
            .checks
            .iter_mut()
            .find(|c| c.check == FindingKind::Duplication.as_str());
        if let Some(own) = own {
            own.findings = Some(report.findings.len() as u64);
        }
        result.duplication_percent = Some(report.percent());
        result.findings.extend(report.findings);
    }
    result
}

fn read_source(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    if std::fs::metadata(path)?.len() > SOURCE_MAX_BYTES {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    std::fs::read(path)
}

/// Runs the requested checks and builds the analysis record.
///
/// # Errors
///
/// Fails when the clock value cannot be represented.
pub(crate) fn perform(
    ctx: &ExecCtx,
    request: &CheckRequest,
    scope: &Scope,
    id: &str,
) -> Result<CheckOutcome> {
    let started = ctx.clock().monotonic();
    let began = Instant::from_unix_nanos(i64::try_from(ctx.clock().now().0).unwrap_or(0));
    let (reports, mut checks, mut notes) = external_checks(ctx, request, scope);
    let builtin = builtin_checks(ctx, request, scope);
    checks.extend(builtin.checks);
    notes.extend(builtin.notes);
    let mut findings = builtin.findings;
    for report in reports {
        checks.push(report.check);
        notes.extend(report.note);
        findings.extend(report.findings);
    }
    if scope.scoped {
        let known: std::collections::HashSet<&str> =
            scope.files.iter().map(String::as_str).collect();
        findings.retain(|f| f.path.as_str().is_empty() || known.contains(f.path.as_str()));
    }
    let mut ordered = order_findings(findings);
    let counts = Counts::of(&ordered);
    ordered.truncate(FINDINGS_STORED_MAX);
    for (index, item) in ordered.iter_mut().enumerate() {
        item.id = Some(format!("{id}#{}", index + 1));
        item.analysis = Some(id.to_owned());
    }
    checks.sort_by_key(|c| c.check.clone());
    let analysis = Analysis {
        id: id.to_owned(),
        passed: counts.passed(request.fail_on),
        fail_on: request.fail_on.as_str().to_owned(),
        errors: counts.errors,
        warnings: counts.warnings,
        infos: counts.infos,
        checks,
        paths: request
            .paths
            .iter()
            .map(|p| RecordPath::new(p.as_str()))
            .collect(),
        changed_only: request.changed_only,
        started: began,
        duration: millis(ctx.clock().monotonic().saturating_sub(started)),
        fingerprint: None,
        duplication_percent: builtin.duplication_percent,
        op: None,
    };
    Ok(CheckOutcome {
        analysis,
        findings: ordered,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicked_check_is_a_failed_check_with_a_note() {
        let report = crashed(Tool::Eslint);
        assert!(matches!(report.check.status, CheckStatus::Failed));
        assert_eq!(report.check.tool.as_deref(), Some("eslint"));
        assert_eq!(report.check.check, FindingKind::Lint.to_string());
        assert!(report.findings.is_empty());
        let note = report.note.unwrap();
        assert!(
            note.detail
                .unwrap()
                .contains("eslint: the check ended in a panic")
        );
    }
}
