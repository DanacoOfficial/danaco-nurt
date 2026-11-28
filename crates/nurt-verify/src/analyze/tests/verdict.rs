// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-verify::analyze::tests::verdict
// Responsibility:   Verdict, ordering and selection of findings.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Verdict, ordering and selection of findings.

use nurt_contracts::record::{Finding, FindingKind, FindingSeverity};
use nurt_contracts::value::Path;

use crate::analyze::verdict::{Counts, FailOn, Filter, order_findings, select};

fn item(severity: FindingSeverity, kind: FindingKind, file: &str, line: u64) -> Finding {
    Finding {
        path: Path::new(file),
        kind,
        severity,
        message: format!("{file}:{line}"),
        line: Some(line),
        ..Finding::default()
    }
}

fn sample() -> Vec<Finding> {
    vec![
        item(FindingSeverity::Info, FindingKind::Lint, "a.ts", 9),
        item(FindingSeverity::Warning, FindingKind::Format, "b.ts", 0),
        item(FindingSeverity::Error, FindingKind::Types, "b.ts", 4),
        item(FindingSeverity::Warning, FindingKind::Lint, "a.ts", 2),
        item(FindingSeverity::Error, FindingKind::Lint, "a.ts", 1),
    ]
}

fn filter(min: FindingSeverity, limit: usize) -> Filter {
    Filter {
        min_severity: min,
        check: None,
        file: None,
        offset: 0,
        limit,
    }
}

#[test]
fn orders_by_severity_check_file_and_line() {
    let ordered = order_findings(sample());
    let keys: Vec<String> = ordered.iter().map(|f| f.message.clone()).collect();
    assert_eq!(keys, vec!["b.ts:4", "a.ts:1", "a.ts:2", "b.ts:0", "a.ts:9"]);
}

#[test]
fn removes_repeated_findings_keeping_most_severe() {
    let mut weaker = item(FindingSeverity::Warning, FindingKind::Lint, "a.ts", 1);
    let stronger = item(FindingSeverity::Error, FindingKind::Lint, "a.ts", 1);
    weaker.message = stronger.message.clone();
    let ordered = order_findings(vec![weaker, stronger]);
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0].severity, FindingSeverity::Error);
}

#[test]
fn fails_on_error_by_default_and_on_warning_when_requested() {
    let warnings_only = Counts {
        errors: 0,
        warnings: 2,
        infos: 5,
    };
    assert!(warnings_only.passed(FailOn::Error));
    assert!(!warnings_only.passed(FailOn::Warning));
    let one_error = Counts {
        errors: 1,
        ..Counts::default()
    };
    assert!(!one_error.passed(FailOn::Error));
}

#[test]
fn info_findings_never_fail_the_analysis() {
    let counts = Counts::of(&[item(FindingSeverity::Info, FindingKind::Lint, "a.ts", 1)]);
    assert!(counts.passed(FailOn::Warning));
    assert_eq!(counts.infos, 1);
}

#[test]
fn min_severity_filters_the_response_and_not_the_counts() {
    let all = order_findings(sample());
    let counts = Counts::of(&all);
    let (page, total) = select(&all, &filter(FindingSeverity::Error, 100));
    assert_eq!(page.len(), 2);
    assert_eq!(total, 2);
    assert_eq!(counts.errors, 2);
    assert_eq!(counts.warnings, 2);
    assert_eq!(counts.infos, 1);
    assert!(!counts.passed(FailOn::Warning));
}

#[test]
fn max_findings_limits_the_page_and_reports_the_total() {
    let all = order_findings(sample());
    let (page, total) = select(&all, &filter(FindingSeverity::Info, 2));
    assert_eq!(page.len(), 2);
    assert_eq!(total, 5);
    assert_eq!(
        Counts::of(&all).errors + Counts::of(&all).warnings + Counts::of(&all).infos,
        5
    );
}

#[test]
fn selects_by_check_file_and_offset() {
    let all = order_findings(sample());
    let mut selection = filter(FindingSeverity::Info, 100);
    selection.check = Some("lint".to_owned());
    selection.file = Some("a.ts".to_owned());
    selection.offset = 1;
    let (page, total) = select(&all, &selection);
    assert_eq!(total, 3);
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].message, "a.ts:2");
}

#[test]
fn parses_fail_on_values() {
    assert_eq!(FailOn::parse("warning"), Some(FailOn::Warning));
    assert_eq!(FailOn::parse("error").map(FailOn::as_str), Some("error"));
    assert_eq!(FailOn::parse("info"), None);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
