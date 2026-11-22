// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `analyze.check`, `analyze.show`, `analyze.format` and `analyze.tools`
//! commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::{Analysis, AnalysisCheck, CheckStatus, FindingSeverity};
use nurt_contracts::stage::{BoundArgs, ExecCtx, StageImpl};
use nurt_contracts::value::{Record, Value};
use nurt_proc::{JobFinisher, ToolResult};
use serde::{Deserialize, Serialize};

use crate::analyze::execute::captured_of;
use crate::analyze::tools::Tool;
use crate::analyze::verdict::{Counts, FINDINGS_STORED_MAX, FailOn, order_findings};
use crate::iface::{EntryKind, StoredEntry, verify_store};
use crate::stage::{analyze_check, analyze_format, analyze_show, analyze_tools};

/// Parameters of a background analysis run, kept with its job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FinishData {
    /// Identifier reserved for the analysis.
    pub(crate) id: String,
    /// Key of the tool that runs.
    pub(crate) tool: String,
    /// Project directory.
    pub(crate) dir: String,
    /// Value of `fail_on`.
    pub(crate) fail_on: String,
}

/// Finisher of analysis runs that continue in the background.
pub(crate) fn job_finisher() -> Arc<dyn JobFinisher> {
    Arc::new(AnalysisFinisher)
}

struct AnalysisFinisher;

impl JobFinisher for AnalysisFinisher {
    fn finish(
        &self,
        ctx: &ExecCtx,
        result: &ToolResult,
        data: Option<&serde_json::Value>,
    ) -> Result<Record> {
        let data: FinishData = data
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::Internal,
                    "the analysis job has no readable run data",
                )
            })?;
        let (tool, fail_on) = Tool::from_key(&data.tool)
            .zip(FailOn::parse(&data.fail_on))
            .ok_or_else(|| Error::new(ErrorCode::Internal, "the analysis job data is invalid"))?;
        let captured = captured_of(result);
        let mut findings = order_findings(tool.parse(&captured.output, Path::new(&data.dir)));
        let counts = Counts::of(&findings);
        findings.truncate(FINDINGS_STORED_MAX);
        for (index, item) in findings.iter_mut().enumerate() {
            item.id = Some(format!("{}#{}", data.id, index + 1));
            item.analysis = Some(data.id.clone());
        }
        let passed = counts.passed(fail_on);
        let analysis = Analysis {
            id: data.id.clone(),
            passed,
            fail_on: fail_on.as_str().to_owned(),
            errors: counts.errors,
            warnings: counts.warnings,
            infos: counts.infos,
            checks: vec![AnalysisCheck {
                check: tool.check().to_string(),
                tool: Some(tool.name().to_owned()),
                status: if passed && !captured.timed_out {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Failed
                },
                findings: Some(findings.len() as u64),
                duration: Some(result.exec.elapsed),
            }],
            paths: Vec::new(),
            changed_only: false,
            started: result.exec.started,
            duration: result.exec.elapsed,
            fingerprint: None,
            op: result.op,
        };
        let header = Record::from(analysis);
        verify_store().save(
            ctx,
            EntryKind::Analysis,
            &StoredEntry {
                id: data.id,
                header: header.clone(),
                items: findings.into_iter().map(Record::from).collect(),
            },
        )?;
        Ok(header)
    }
}

/// Stages of the analysis commands.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(analyze_check::Check),
        Arc::new(analyze_show::Show),
        Arc::new(analyze_format::Format),
        Arc::new(analyze_tools::Tools),
    ]
}

/// Project directory: the base of the request or the working directory.
pub(crate) fn base_dir(ctx: &ExecCtx) -> Result<PathBuf> {
    match &ctx.options().base {
        Some(base) => Ok(PathBuf::from(base)),
        None => std::env::current_dir().map_err(|error| {
            Error::new(
                ErrorCode::Arg,
                format!("cannot read the working directory: {error}"),
            )
        }),
    }
}

/// Text of an argument given as text, enum or path.
pub(crate) fn text_arg(args: &BoundArgs, name: &str) -> Option<String> {
    match args.value(name)? {
        Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.clone()),
        _ => None,
    }
}

/// Elements of a list argument, or the single value of the argument.
pub(crate) fn list_arg(args: &BoundArgs, name: &str) -> Vec<String> {
    match args.value(name) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::Text(text) | Value::Enum(text) | Value::Path(text) => Some(text.clone()),
                _ => None,
            })
            .collect(),
        Some(Value::Text(text) | Value::Enum(text) | Value::Path(text)) => vec![text.clone()],
        _ => Vec::new(),
    }
}

/// Boolean argument with a default.
pub(crate) fn bool_arg(args: &BoundArgs, name: &str, default: bool) -> bool {
    args.value(name).and_then(Value::as_bool).unwrap_or(default)
}

/// Non-negative integer argument with a default.
pub(crate) fn count_arg(args: &BoundArgs, name: &str, default: usize) -> usize {
    args.value(name)
        .and_then(Value::as_int)
        .and_then(|number| usize::try_from(number).ok())
        .unwrap_or(default)
}

/// Severity argument with a default.
pub(crate) fn severity_arg(
    args: &BoundArgs,
    name: &str,
    default: FindingSeverity,
) -> FindingSeverity {
    text_arg(args, name)
        .and_then(|text| text.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::Exec;
    use nurt_contracts::value::Duration;

    use super::*;
    use crate::stage::test::support::{ctx, write};

    fn data(dir: &Path, tool: &str, fail_on: &str) -> serde_json::Value {
        serde_json::to_value(FinishData {
            id: "an-7".to_owned(),
            tool: tool.to_owned(),
            dir: dir.to_string_lossy().into_owned(),
            fail_on: fail_on.to_owned(),
        })
        .unwrap()
    }

    fn result(dir: &Path, stdout: &str) -> ToolResult {
        ToolResult {
            exec: Exec {
                exit_code: 1,
                elapsed: Duration::from_millis(40),
                ..Exec::default()
            },
            stdout_file: Some(write(dir, "tool-output.txt", stdout)),
            stderr_file: None,
            op: None,
        }
    }

    #[test]
    fn finishes_a_background_run_into_a_stored_analysis() {
        let state = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write(project.path(), "a.ts", "let a = 1;  \n");
        let output = result(state.path(), "a.ts\n");
        let header = job_finisher()
            .finish(
                &ctx(state.path()),
                &output,
                Some(&data(project.path(), "prettier", "warning")),
            )
            .unwrap();
        assert_eq!(
            header.get("id").and_then(|value| value.as_str()),
            Some("an-7")
        );
        assert_eq!(header.get("passed").and_then(Value::as_bool), Some(false));
        let stored = verify_store()
            .load(&ctx(state.path()), EntryKind::Analysis, "an-7")
            .unwrap()
            .unwrap();
        assert_eq!(stored.items.len(), 1);
    }

    #[test]
    fn rejects_a_job_without_run_data() {
        let state = tempfile::tempdir().unwrap();
        let output = result(state.path(), "");
        let error = job_finisher()
            .finish(&ctx(state.path()), &output, None)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
    }
}
