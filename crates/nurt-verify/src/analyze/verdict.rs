// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::verdict
// Responsibility:   Ordering of findings, counts, verdict and selection of the findings returned to the caller.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Ordering of findings, counts, verdict and selection of the findings
//! returned to the caller.

use std::collections::HashMap;

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};

/// Most findings kept in a stored analysis.
pub(crate) const FINDINGS_STORED_MAX: usize = 5000;

/// Lowest severity that fails an analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailOn {
    /// Errors fail the analysis.
    Error,
    /// Errors and warnings fail the analysis.
    Warning,
}

impl FailOn {
    /// Parses the value of the `fail_on` parameter.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            _ => None,
        }
    }

    /// Value as written in the `analysis` record.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// Numbers of findings per severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Errors.
    pub(crate) errors: u64,
    /// Warnings.
    pub(crate) warnings: u64,
    /// Information.
    pub(crate) infos: u64,
}

impl Counts {
    /// Counts of all findings, whatever the filter of the response.
    pub(crate) fn of(findings: &[Finding]) -> Self {
        let mut counts = Self::default();
        for finding in findings {
            match finding.severity {
                FindingSeverity::Error => counts.errors += 1,
                FindingSeverity::Warning => counts.warnings += 1,
                _ => counts.infos += 1,
            }
        }
        counts
    }

    /// Whether the analysis passes: no finding of the severity `fail_on` or
    /// higher.
    pub(crate) fn passed(self, fail_on: FailOn) -> bool {
        match fail_on {
            FailOn::Error => self.errors == 0,
            FailOn::Warning => self.errors == 0 && self.warnings == 0,
        }
    }
}

/// Rank of a severity; the most severe is the lowest.
pub(crate) fn severity_rank(severity: FindingSeverity) -> u8 {
    match severity {
        FindingSeverity::Error => 0,
        FindingSeverity::Warning => 1,
        _ => 2,
    }
}

/// Rank of a check in the order of the output.
pub(crate) fn kind_rank(kind: FindingKind) -> u8 {
    match kind {
        FindingKind::Types => 0,
        FindingKind::Lint => 1,
        FindingKind::Format => 2,
        FindingKind::Complexity => 3,
        FindingKind::Duplication => 4,
        _ => 5,
    }
}

fn identity(finding: &Finding) -> (String, u64, String, String) {
    (
        finding.path.as_str().to_owned(),
        finding.line.unwrap_or(0),
        finding.rule.clone().unwrap_or_default(),
        finding.message.clone(),
    )
}

/// Removes repeated findings, keeping the most severe copy, and orders the
/// rest by severity, check, file, line and column.
pub(crate) fn order_findings(findings: Vec<Finding>) -> Vec<Finding> {
    let mut slots: HashMap<(String, u64, String, String), usize> = HashMap::new();
    let mut unique: Vec<Finding> = Vec::new();
    for finding in findings {
        match slots.get(&identity(&finding)) {
            Some(&slot) => {
                if severity_rank(finding.severity) < severity_rank(unique[slot].severity) {
                    unique[slot] = finding;
                }
            }
            None => {
                slots.insert(identity(&finding), unique.len());
                unique.push(finding);
            }
        }
    }
    unique.sort_by(|left, right| {
        severity_rank(left.severity)
            .cmp(&severity_rank(right.severity))
            .then(kind_rank(left.kind).cmp(&kind_rank(right.kind)))
            .then_with(|| left.path.as_str().cmp(right.path.as_str()))
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });
    unique
}

/// Selection of the findings returned in a response.
#[derive(Debug, Clone)]
pub(crate) struct Filter {
    /// Lowest severity returned.
    pub(crate) min_severity: FindingSeverity,
    /// Only this check, as written by the caller.
    pub(crate) check: Option<String>,
    /// Only files whose path contains this text.
    pub(crate) file: Option<String>,
    /// Matching findings skipped.
    pub(crate) offset: usize,
    /// Most findings returned.
    pub(crate) limit: usize,
}

/// Findings matching the filter before the offset and the limit apply.
pub(crate) fn matching<'a>(findings: &'a [Finding], filter: &Filter) -> Vec<&'a Finding> {
    let rank = severity_rank(filter.min_severity);
    findings
        .iter()
        .filter(|item| severity_rank(item.severity) <= rank)
        .filter(|item| {
            filter
                .check
                .as_deref()
                .is_none_or(|check| item.kind.as_str() == check)
        })
        .filter(|item| {
            filter
                .file
                .as_deref()
                .is_none_or(|file| item.path.as_str().contains(file))
        })
        .collect()
}

/// Page of the matching findings and the number of matching findings.
pub(crate) fn select<'a>(findings: &'a [Finding], filter: &Filter) -> (Vec<&'a Finding>, usize) {
    let all = matching(findings, filter);
    let total = all.len();
    let page = all
        .into_iter()
        .skip(filter.offset)
        .take(filter.limit)
        .collect();
    (page, total)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
