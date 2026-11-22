// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Stable codes of the report enumerations and the rules checked by
//! `Report::validate`.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fmt::Display;
use std::str::FromStr;

use nurt_contracts::report::{
    Change, ChangeAction, Correction, CorrectionClass, Hunk, Irreversible, IrreversibleEntry,
    Notable, NotableEntry, REPORT_VERSION, Report, ReportStatus, ReportViolation, Severity,
    Targets, UnknownCode, is_ast_hash, is_supported_version,
};
use serde::Serialize;

fn check_codes<T>(all: &[T], as_str: fn(T) -> &'static str)
where
    T: Copy + PartialEq + std::fmt::Debug + Display + Serialize + FromStr<Err = UnknownCode>,
{
    let mut seen = BTreeSet::new();
    for value in all {
        let code = as_str(*value);
        assert!(seen.insert(code), "duplicate code {code}");
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(code)
        );
        assert_eq!(value.to_string(), code);
        assert_eq!(code.parse::<T>().unwrap(), *value);
        let decoded: T = code.parse().unwrap();
        assert_eq!(decoded, *value);
    }
    let error = "not-a-code".parse::<T>().unwrap_err();
    assert_eq!(error.code(), "not-a-code");
    assert!(error.to_string().contains("not-a-code"));
}

#[test]
fn codes_are_stable_and_round_trip() {
    check_codes(ReportStatus::ALL, ReportStatus::as_str);
    check_codes(Severity::ALL, Severity::as_str);
    check_codes(Notable::ALL, Notable::as_str);
    check_codes(Irreversible::ALL, Irreversible::as_str);
    check_codes(CorrectionClass::ALL, CorrectionClass::as_str);
    for action in ChangeAction::ALL {
        assert_eq!(action.as_str().parse::<ChangeAction>().unwrap(), *action);
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            serde_json::json!(action.as_str())
        );
    }
    assert!("not-a-code".parse::<ChangeAction>().is_err());
}

#[test]
fn codes_deserialize_from_their_text() {
    for code in Notable::ALL {
        let value: Notable = serde_json::from_value(serde_json::json!(code.as_str())).unwrap();
        assert_eq!(value, *code);
    }
    for reason in Irreversible::ALL {
        let value: Irreversible =
            serde_json::from_value(serde_json::json!(reason.as_str())).unwrap();
        assert_eq!(value, *reason);
    }
    assert!(serde_json::from_value::<Notable>(serde_json::json!("undo_possible")).is_err());
}

#[test]
fn notable_and_irreversible_codes_are_snake_case() {
    let snake = |code: &str| {
        !code.is_empty()
            && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            && !code.starts_with('_')
            && !code.ends_with('_')
    };
    for code in Notable::ALL {
        assert!(snake(code.as_str()), "{code}");
    }
    for reason in Irreversible::ALL {
        assert!(snake(reason.as_str()), "{reason}");
    }
}

#[test]
fn reports_enumerations_contain_the_documented_values() {
    let statuses: BTreeSet<&str> = ReportStatus::ALL.iter().map(|s| s.as_str()).collect();
    for status in [
        "ok",
        "planned",
        "running",
        "partial",
        "rolled_back",
        "failed",
    ] {
        assert!(statuses.contains(status), "{status}");
    }
    let reasons: BTreeSet<&str> = Irreversible::ALL.iter().map(|r| r.as_str()).collect();
    for reason in [
        "external_effect",
        "remote_ref_updated",
        "network",
        "database",
        "process_stopped",
        "service_changed",
        "package_changed",
        "previous_content_not_kept",
        "no_space_for_copy",
        "journal_inactive",
        "network_volume",
        "removable_volume",
        "purged",
        "special_file",
        "too_large_for_journal",
        "released_for_space",
        "job_output_removed",
        "process_state_changed",
        "journal_entries_expired",
        "journal_error",
    ] {
        assert!(reasons.contains(reason), "{reason}");
    }
    assert_eq!(Notable::ALL.len(), 86);
    assert_eq!(
        Notable::ProjectConfigIgnored.as_str(),
        "project_config_ignored"
    );
    assert_eq!(
        Notable::TrustConfigIgnored.default_severity(),
        Severity::Warning
    );
}

#[test]
fn severities_are_ordered() {
    assert!(Severity::ALL.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(Notable::OutsideProject.default_severity(), Severity::Alert);
    assert_eq!(Notable::ForcePush.default_severity(), Severity::Alert);
    assert_eq!(Notable::MassDelete.default_severity(), Severity::Warning);
    assert_eq!(Notable::DeleteTree.default_severity(), Severity::Notice);
    assert_eq!(
        Notable::EncodingPreserved.default_severity(),
        Severity::Info
    );
    assert_eq!(
        Notable::HardlinkSetPreserved.default_severity(),
        Severity::Info
    );
}

#[test]
fn report_builder_computes_the_change_derived_codes_only() {
    let computed: BTreeSet<&str> = Notable::ALL
        .iter()
        .filter(|code| code.computed_by_report())
        .map(|code| code.as_str())
        .collect();
    let expected: BTreeSet<&str> = [
        "outside_project",
        "system_path",
        "home_config",
        "mass_delete",
        "delete_tree",
        "overwrote_existing",
        "file_emptied",
        "size_shrunk",
        "many_per_file",
        "target_count_high",
    ]
    .into_iter()
    .collect();
    assert_eq!(computed, expected);
}

fn sample_error(code: &str) -> nurt_contracts::error::Error {
    serde_json::from_value(serde_json::json!({ "code": code, "message": "failed" })).unwrap()
}

fn ok_report() -> Report {
    let mut report = Report::new(ReportStatus::Ok, "replace");
    report.op = Some(18);
    report
}

#[test]
fn a_minimal_report_is_valid() {
    let report = Report::new(ReportStatus::Ok, "find");
    assert_eq!(report.nurt, REPORT_VERSION);
    assert!(report.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::json!({ "nurt": "1.0", "status": "ok", "kind": "find" })
    );
}

#[test]
fn header_rules() {
    let mut report = ok_report();
    report.nurt = "2.0".to_owned();
    assert!(matches!(
        report.validate(),
        Err(ReportViolation::UnsupportedVersion { .. })
    ));

    let mut report = ok_report();
    report.kind.clear();
    assert_eq!(report.validate(), Err(ReportViolation::EmptyKind));

    let mut report = ok_report();
    report.op = Some(0);
    assert_eq!(report.validate(), Err(ReportViolation::ZeroOp));

    let mut report = ok_report();
    report.ast_hash = Some("sha256:abc".to_owned());
    assert!(matches!(
        report.validate(),
        Err(ReportViolation::InvalidAstHash { .. })
    ));

    let mut report = ok_report();
    report.result_file = Some(String::new());
    assert_eq!(report.validate(), Err(ReportViolation::EmptyResultFile));

    let mut report = ok_report();
    report.next_cursor = Some(String::new());
    assert_eq!(report.validate(), Err(ReportViolation::EmptyCursor));
}

#[test]
fn status_rules() {
    let report = Report::new(ReportStatus::Failed, "remove");
    assert_eq!(
        report.validate(),
        Err(ReportViolation::MissingError {
            status: ReportStatus::Failed
        })
    );

    let report = Report::new(ReportStatus::RolledBack, "remove");
    assert_eq!(
        report.validate(),
        Err(ReportViolation::MissingError {
            status: ReportStatus::RolledBack
        })
    );

    let mut report = Report::new(ReportStatus::Failed, "remove");
    report.error = Some(sample_error("E_STALE"));
    assert!(report.validate().is_ok());

    let mut report = Report::new(ReportStatus::Planned, "remove");
    report.op = Some(4);
    assert_eq!(report.validate(), Err(ReportViolation::OpInPreview));

    let report = Report::new(ReportStatus::Running, "run");
    assert_eq!(report.validate(), Err(ReportViolation::MissingJob));

    let report = Report::new(ReportStatus::Partial, "undo");
    assert_eq!(
        report.validate(),
        Err(ReportViolation::MissingOp {
            status: ReportStatus::Partial
        })
    );

    let mut report = Report::new(ReportStatus::Partial, "undo");
    report.op = Some(25);
    assert_eq!(
        report.validate(),
        Err(ReportViolation::PartialWithoutRemainder)
    );
    report
        .irreversible
        .push(IrreversibleEntry::new(Irreversible::ReleasedForSpace));
    assert!(report.validate().is_ok());
}

#[test]
fn ok_report_may_carry_an_error_of_a_check_after_the_effect() {
    let mut report = ok_report();
    report.error = Some(sample_error("E_ASSERT"));
    assert!(report.validate().is_ok());
}

#[test]
fn correction_rules() {
    let correction = Correction {
        id: "C1".to_owned(),
        rule: "L01".to_owned(),
        class: CorrectionClass::Fuzzy,
        at: None,
        from: "remvoe".to_owned(),
        to: "remove".to_owned(),
        kind: None,
        alternatives: Vec::new(),
    };
    let mut report = ok_report();
    report.ran = Some("remove \"/srv/app/old.log\"".to_owned());
    assert_eq!(
        report.validate(),
        Err(ReportViolation::RanWithoutCorrections)
    );
    report.corrections.push(correction.clone());
    assert!(report.validate().is_ok());
    report.corrections[0].rule = "X1".to_owned();
    assert!(matches!(
        report.validate(),
        Err(ReportViolation::InvalidCorrectionRule { .. })
    ));
    assert!(Correction::is_rule_id("B12"));
    assert!(!Correction::is_rule_id("L1"));
    assert!(!Correction::is_rule_id("l01"));
}

#[test]
fn change_rules() {
    let mut report = ok_report();
    report.targets = Some(Targets {
        count: 2,
        listed: Some(2),
        ..Targets::default()
    });
    report
        .changes
        .push(Change::new("/srv/app/a.go", ChangeAction::Modify));
    assert_eq!(
        report.validate(),
        Err(ReportViolation::ListedMismatch {
            listed: 2,
            changes: 1
        })
    );
    report
        .changes
        .push(Change::new("/srv/app/b.go", ChangeAction::Rename));
    assert!(matches!(
        report.validate(),
        Err(ReportViolation::RenameWithoutFrom { .. })
    ));
    report.changes[1].from = Some("/srv/app/c.go".into());
    assert!(report.validate().is_ok());

    report.diff.push(Hunk {
        path: "/srv/app/a.go".into(),
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        lines: vec!["-a".to_owned(), "b".to_owned()],
        ..Hunk::default()
    });
    assert_eq!(
        report.validate(),
        Err(ReportViolation::InvalidHunkLine {
            path: "/srv/app/a.go".to_owned(),
            line: 1
        })
    );
    report.diff[0].lines[1] = "+b".to_owned();
    assert!(report.validate().is_ok());
}

#[test]
fn notable_entries_are_kept_in_descending_severity() {
    let mut report = ok_report();
    report
        .notable
        .push(NotableEntry::new(Notable::EncodingPreserved));
    report.notable.push(NotableEntry::new(Notable::ManyPerFile));
    report.notable.push(NotableEntry::new(Notable::SystemPath));
    report
        .notable
        .push(NotableEntry::new(Notable::LineEndingPreserved));
    assert_eq!(
        report.validate(),
        Err(ReportViolation::NotableOrder { index: 1 })
    );
    report.sort_notable();
    assert!(report.validate().is_ok());
    let order: Vec<Notable> = report.notable.iter().map(|entry| entry.code).collect();
    assert_eq!(
        order,
        [
            Notable::SystemPath,
            Notable::ManyPerFile,
            Notable::EncodingPreserved,
            Notable::LineEndingPreserved
        ]
    );
}

#[test]
fn absent_lists_are_omitted_from_json() {
    let mut report = ok_report();
    report
        .irreversible
        .push(IrreversibleEntry::new(Irreversible::ExternalEffect));
    let value = serde_json::to_value(&report).unwrap();
    assert!(value.get("notable").is_none());
    assert!(value.get("changes").is_none());
    assert_eq!(value["irreversible"][0]["reason"], "external_effect");
    report.irreversible.clear();
    assert!(
        serde_json::to_value(&report)
            .unwrap()
            .get("irreversible")
            .is_none()
    );
}

#[test]
fn unknown_fields_of_a_newer_minor_version_are_ignored() {
    let value = serde_json::json!({
        "nurt": "1.7",
        "status": "ok",
        "kind": "find",
        "future_field": { "anything": true }
    });
    let report: Report = serde_json::from_value(value).unwrap();
    assert!(report.validate().is_ok());
}

#[test]
fn helpers() {
    assert!(is_supported_version("1.0"));
    assert!(!is_supported_version("1"));
    assert!(!is_supported_version("1.x"));
    assert!(is_ast_hash(&format!("sha256:{}", "ab".repeat(32))));
    assert!(!is_ast_hash(&format!("sha256:{}", "AB".repeat(32))));
    assert!(!is_ast_hash(&format!("sha1:{}", "ab".repeat(32))));
    let violation = ReportViolation::ListedMismatch {
        listed: 3,
        changes: 1,
    };
    assert!(violation.to_string().contains("3"));
}
