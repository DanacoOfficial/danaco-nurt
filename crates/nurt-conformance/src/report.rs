// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance::report
// Responsibility:   Conformance report `nurt-conformance-report/1`.
// Dependencies:     serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conformance report `nurt-conformance-report/1`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Name and version of the format.
pub const FORMAT: &str = "nurt-conformance-report/1";

/// Result of a run of the suite against one implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformanceReport {
    /// Format name, [`FORMAT`].
    pub format: String,
    /// Version and digest of the suite.
    pub suite: SuiteInfo,
    /// Tested implementation.
    pub implementation: Implementation,
    /// System the suite ran on.
    pub system: System,
    /// Start of the run, RFC 3339 in UTC.
    pub started: String,
    /// Duration of the run in milliseconds.
    pub duration_ms: u64,
    /// Result of every selected case.
    pub results: Vec<CaseResult>,
    /// Counts and the conformance verdict.
    pub summary: Summary,
}

/// Version and digest of the suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteInfo {
    /// Version of the suite.
    pub version: String,
    /// SHA-256 digest of the case files.
    pub digest: String,
    /// Number of cases of the suite.
    pub cases: usize,
    /// Filters of the run; empty for a complete run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<String>,
}

/// Tested implementation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Implementation {
    /// Command line that started the server.
    pub command: String,
    /// SHA-256 digest of the executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_digest: Option<String>,
    /// Result of `initialize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialize: Option<Value>,
    /// Result of `info`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<Value>,
}

/// System the suite ran on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct System {
    /// Operating system, as named by the Rust standard library.
    pub os: String,
    /// Processor architecture, as named by the Rust standard library.
    pub arch: String,
}

impl System {
    /// Returns the current system.
    #[must_use]
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }
}

/// Status of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Every check held.
    Pass,
    /// At least one check failed.
    Fail,
    /// The case did not apply or needs a feature the implementation lacks.
    Skip,
}

/// Result of one case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    /// Identifier of the case.
    pub id: String,
    /// Kind of the case.
    pub kind: String,
    /// Class of the case.
    pub class: String,
    /// Status of the case.
    pub status: Status,
    /// Why the case was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Failed checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<Failure>,
}

/// Failed check of a case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    /// Number of the step, counted from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<usize>,
    /// Name of the check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// JSON pointer or file path of the difference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Description of the failure.
    pub message: String,
    /// Expected value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    /// Actual value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

impl Failure {
    /// Returns a failure with a message only.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            step: None,
            check: None,
            path: None,
            message: message.into(),
            expected: None,
            actual: None,
        }
    }
}

/// Counts of results.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Number of cases.
    pub total: usize,
    /// Passed cases.
    pub passed: usize,
    /// Failed cases.
    pub failed: usize,
    /// Skipped cases.
    pub skipped: usize,
}

impl Counts {
    fn add(&mut self, status: Status) {
        self.total += 1;
        match status {
            Status::Pass => self.passed += 1,
            Status::Fail => self.failed += 1,
            Status::Skip => self.skipped += 1,
        }
    }
}

/// Counts per class and per kind and the conformance verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// Mandatory cases.
    pub mandatory: Counts,
    /// Optional cases.
    pub optional: Counts,
    /// Cases per kind.
    pub kinds: BTreeMap<String, Counts>,
    /// Whether the run was complete and every applicable mandatory case
    /// passed.
    pub conformant: bool,
}

impl Summary {
    /// Counts the results; `complete` is false when the run was filtered.
    #[must_use]
    pub fn from_results(results: &[CaseResult], complete: bool) -> Self {
        let mut mandatory = Counts::default();
        let mut optional = Counts::default();
        let mut kinds: BTreeMap<String, Counts> = BTreeMap::new();
        for result in results {
            let class = if result.class == "mandatory" {
                &mut mandatory
            } else {
                &mut optional
            };
            class.add(result.status);
            kinds
                .entry(result.kind.clone())
                .or_default()
                .add(result.status);
        }
        Self {
            conformant: complete && mandatory.failed == 0 && mandatory.passed > 0,
            mandatory,
            optional,
            kinds,
        }
    }
}

impl ConformanceReport {
    /// Whether every applicable mandatory case of the run passed.
    #[must_use]
    pub fn mandatory_passed(&self) -> bool {
        self.summary.mandatory.failed == 0
    }

    /// Writes the report as readable text.
    ///
    /// Every case gets one line unless `quiet` is set; failed cases are
    /// followed by their failures. The last lines hold the counts and the
    /// verdict.
    #[must_use]
    pub fn to_text(&self, quiet: bool) -> String {
        let mut out = String::new();
        for result in &self.results {
            if quiet && result.status != Status::Fail {
                continue;
            }
            let status = match result.status {
                Status::Pass => "pass",
                Status::Fail => "FAIL",
                Status::Skip => "skip",
            };
            let _ = write!(out, "{status} {} [{}]", result.id, result.class);
            if let Some(reason) = &result.reason {
                let _ = write!(out, " ({reason})");
            }
            out.push('\n');
            for failure in &result.failures {
                let _ = writeln!(out, "    {}", describe(failure));
            }
        }
        let summary = &self.summary;
        let _ = writeln!(
            out,
            "suite {} ({} cases), {}",
            self.suite.version, self.suite.cases, self.implementation.command
        );
        for (name, counts) in [
            ("mandatory", summary.mandatory),
            ("optional", summary.optional),
        ] {
            let _ = writeln!(
                out,
                "{name}: {} passed, {} failed, {} skipped",
                counts.passed, counts.failed, counts.skipped
            );
        }
        let verdict = match (summary.conformant, self.mandatory_passed()) {
            (true, _) => "conformant",
            (false, true) => "mandatory cases passed; the run was filtered or empty",
            (false, false) => "not conformant",
        };
        // Skipped mandatory cases do not break conformance, but they are shown.
        let skipped = summary.mandatory.skipped;
        if summary.conformant && skipped > 0 {
            let _ = writeln!(
                out,
                "result: {verdict} ({skipped} mandatory case(s) skipped)"
            );
        } else {
            let _ = writeln!(out, "result: {verdict}");
        }
        out
    }
}

fn describe(failure: &Failure) -> String {
    let mut text = String::new();
    if let Some(step) = failure.step {
        let _ = write!(text, "step {step}: ");
    }
    if let Some(check) = &failure.check {
        let _ = write!(text, "{check}: ");
    }
    if let Some(path) = &failure.path {
        let _ = write!(text, "{path}: ");
    }
    text.push_str(&failure.message);
    if let Some(expected) = &failure.expected {
        let _ = write!(text, "; expected {expected}");
    }
    if let Some(actual) = &failure.actual {
        let _ = write!(text, "; actual {actual}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(id: &str, class: &str, status: Status) -> CaseResult {
        CaseResult {
            id: id.to_owned(),
            kind: "golden".to_owned(),
            class: class.to_owned(),
            status,
            reason: (status == Status::Skip).then(|| "feature testkit missing".to_owned()),
            duration_ms: 1,
            failures: if status == Status::Fail {
                vec![Failure {
                    step: Some(2),
                    check: Some("expect".to_owned()),
                    path: Some("/report/status".to_owned()),
                    message: "values differ".to_owned(),
                    expected: Some(json!("ok")),
                    actual: Some(json!("failed")),
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn report(results: Vec<CaseResult>, complete: bool) -> ConformanceReport {
        let summary = Summary::from_results(&results, complete);
        ConformanceReport {
            format: FORMAT.to_owned(),
            suite: SuiteInfo {
                version: "1.0.0".to_owned(),
                digest: format!("sha256:{}", "0".repeat(64)),
                cases: results.len(),
                filter: Vec::new(),
            },
            implementation: Implementation {
                command: "nurt serve --stdio".to_owned(),
                executable_digest: None,
                initialize: Some(json!({})),
                info: None,
            },
            system: System::current(),
            started: "2026-01-01T00:00:00Z".to_owned(),
            duration_ms: 5,
            results,
            summary,
        }
    }

    #[test]
    fn summary_counts_classes_and_kinds() {
        let results = vec![
            result("a", "mandatory", Status::Pass),
            result("b", "mandatory", Status::Skip),
            result("c", "optional:testkit", Status::Fail),
            result("d", "optional:testkit", Status::Skip),
        ];
        let summary = Summary::from_results(&results, true);
        assert_eq!(
            summary.mandatory,
            Counts {
                total: 2,
                passed: 1,
                failed: 0,
                skipped: 1
            }
        );
        assert_eq!(
            summary.optional,
            Counts {
                total: 2,
                passed: 0,
                failed: 1,
                skipped: 1
            }
        );
        assert_eq!(summary.kinds["golden"].total, 4);
        assert!(summary.conformant);
        assert!(!Summary::from_results(&results, false).conformant);
        assert!(!Summary::from_results(&[], true).conformant);
    }

    #[test]
    fn report_is_valid_under_its_schema_and_round_trips() {
        let report = report(vec![result("a", "mandatory", Status::Fail)], true);
        let value = serde_json::to_value(&report).unwrap();
        let schemas = crate::schemas::SchemaSet::new().unwrap();
        assert_eq!(schemas.validate("conformance-report", &value), Ok(()));
        let back: ConformanceReport = serde_json::from_value(value).unwrap();
        assert_eq!(back, report);
        assert!(!report.mandatory_passed());
    }

    #[test]
    fn text_lists_cases_failures_and_the_verdict() {
        let failing = report(
            vec![
                result("a", "mandatory", Status::Pass),
                result("b", "mandatory", Status::Fail),
                result("c", "optional:x", Status::Skip),
            ],
            true,
        );
        let text = failing.to_text(false);
        assert!(text.contains("pass a [mandatory]"));
        assert!(text.contains("FAIL b [mandatory]"));
        assert!(text.contains("skip c [optional:x] (feature testkit missing)"));
        assert!(text.contains(
            "step 2: expect: /report/status: values differ; expected \"ok\"; actual \"failed\""
        ));
        assert!(text.ends_with("result: not conformant\n"));
        let quiet = failing.to_text(true);
        assert!(!quiet.contains("pass a"));
        assert!(quiet.contains("FAIL b"));

        let passing = report(vec![result("a", "mandatory", Status::Pass)], true);
        assert!(passing.to_text(true).ends_with("result: conformant\n"));
        let skipping = report(
            vec![
                result("a", "mandatory", Status::Pass),
                result("b", "mandatory", Status::Skip),
                result("c", "mandatory", Status::Skip),
            ],
            true,
        );
        assert!(
            skipping
                .to_text(true)
                .ends_with("result: conformant (2 mandatory case(s) skipped)\n")
        );
        let filtered = report(vec![result("a", "mandatory", Status::Pass)], false);
        assert!(filtered.to_text(true).contains("the run was filtered"));
        assert_eq!(describe(&Failure::new("m")), "m");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
