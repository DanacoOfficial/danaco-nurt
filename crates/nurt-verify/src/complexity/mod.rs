// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::complexity
// Responsibility:   Cyclomatic complexity findings from the function metrics of the code service.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Cyclomatic complexity findings from the function metrics of the code
//! service.

use nurt_contracts::error::Result;
use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use nurt_contracts::services::{CodeService, FunctionMetrics, LanguageId};
use nurt_contracts::value::Path as RecordPath;

/// Tool name of the findings.
pub(crate) const TOOL: &str = "builtin";

/// Rule of the findings.
pub(crate) const RULE: &str = "complexity";

/// Default highest complexity accepted.
pub(crate) const COMPLEXITY_MAX_DEFAULT: u32 = 15;

/// Complexity measured in one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FileComplexity {
    /// Functions above the threshold.
    pub(crate) findings: Vec<Finding>,
    /// Highest complexity of a function in the file.
    pub(crate) highest: u32,
    /// Functions measured.
    pub(crate) functions: usize,
}

/// Measures the functions of one file with the code service.
///
/// # Errors
///
/// Fails with the error of the code service, for example when the language
/// has no grammar.
pub(crate) fn measure_file(
    code: &dyn CodeService,
    path: &str,
    bytes: &[u8],
    language: &LanguageId,
    threshold: u32,
) -> Result<FileComplexity> {
    let metrics = code.function_metrics(bytes, language)?;
    Ok(findings_for(path, &metrics, threshold))
}

/// Findings for the functions of `path` whose complexity exceeds
/// `threshold`; every function counts separately.
pub(crate) fn findings_for(
    path: &str,
    metrics: &[FunctionMetrics],
    threshold: u32,
) -> FileComplexity {
    let mut result = FileComplexity {
        functions: metrics.len(),
        ..FileComplexity::default()
    };
    for function in metrics {
        result.highest = result.highest.max(function.cyclomatic);
        if function.cyclomatic > threshold {
            result.findings.push(finding(path, function, threshold));
        }
    }
    result
}

fn finding(path: &str, function: &FunctionMetrics, threshold: u32) -> Finding {
    let name = match &function.container {
        Some(container) => format!("{container}.{}", function.name),
        None => function.name.clone(),
    };
    Finding {
        path: RecordPath::new(path),
        kind: FindingKind::Complexity,
        severity: FindingSeverity::Error,
        message: format!(
            "{name}: cyclomatic complexity {} exceeds {threshold}",
            function.cyclomatic
        ),
        line: Some(u64::from(function.span.start.line)),
        column: Some(1),
        end_line: Some(u64::from(function.span.end.line)),
        tool: Some(TOOL.to_owned()),
        rule: Some(RULE.to_owned()),
        value: Some(f64::from(function.cyclomatic)),
        threshold: Some(f64::from(threshold)),
        ..Finding::default()
    }
}

#[cfg(test)]
mod tests;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
