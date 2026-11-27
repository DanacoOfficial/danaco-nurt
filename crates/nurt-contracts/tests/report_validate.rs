// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/report_validate.rs
// Responsibility:   Stable codes of the report enumerations and the rules checked by.
// Dependencies:     serde
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

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
use nurt_contracts::value::{Record, RecordKind};
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
    assert_eq!(Notable::ALL.len(), 98);
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
        "nothing_matched",
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

#[test]
fn rename_gives_the_previous_path_in_from() {
    // The shape of the golden `change` record of a rename: `path` is the
    // target after the change, `from` the previous path and `to` the new path.
    let mut report = ok_report();
    let mut change = Change::new("/srv/app/new/name.txt", ChangeAction::Rename);
    change.from = Some("/srv/app/old/name.txt".into());
    change.to = Some("/srv/app/new/name.txt".into());
    report.changes.push(change);
    assert!(report.validate().is_ok());
}

#[test]
fn rename_that_names_only_the_destination_is_rejected() {
    let mut report = ok_report();
    let mut change = Change::new("/d/a.txt", ChangeAction::Rename);
    change.to = Some("/d/b.txt".into());
    report.changes.push(change);
    let violation = report.validate().unwrap_err();
    assert_eq!(
        violation,
        ReportViolation::RenameWithoutFrom {
            path: "/d/a.txt".to_owned()
        }
    );
    assert_eq!(
        violation.to_string(),
        "the rename of `/d/a.txt` does not give the previous path"
    );
}

#[test]
fn the_first_rename_without_from_is_named() {
    let mut report = ok_report();
    let mut complete = Change::new("/d/b.txt", ChangeAction::Rename);
    complete.from = Some("/d/a.txt".into());
    report.changes.push(complete);
    report
        .changes
        .push(Change::new("/d/c.txt", ChangeAction::Modify));
    report
        .changes
        .push(Change::new("/d/e.txt", ChangeAction::Rename));
    report
        .changes
        .push(Change::new("/d/f.txt", ChangeAction::Rename));
    assert_eq!(
        report.validate(),
        Err(ReportViolation::RenameWithoutFrom {
            path: "/d/e.txt".to_owned()
        })
    );
}

#[test]
fn rename_violation_keeps_multi_byte_paths() {
    let path = "/srv/\u{105}\u{15b}\u{107}/\u{65e5}\u{672c}.txt";
    let mut report = ok_report();
    report.changes.push(Change::new(path, ChangeAction::Rename));
    let violation = report.validate().unwrap_err();
    assert!(violation.to_string().contains(path), "{violation}");
}

#[test]
fn from_is_required_only_for_renames() {
    let mut report = ok_report();
    for action in ChangeAction::ALL {
        if *action != ChangeAction::Rename {
            report.changes.push(Change::new("/srv/app/a.go", *action));
        }
    }
    assert!(!report.changes.is_empty());
    assert!(report.validate().is_ok());
}

#[test]
fn rolled_back_partial_running_and_planned_reports_with_their_members_are_valid() {
    let mut report = Report::new(ReportStatus::RolledBack, "replace");
    report.error = Some(rolled_back_error());
    assert!(report.validate().is_ok());

    let mut report = Report::new(ReportStatus::Partial, "undo");
    report.op = Some(25);
    report.error = Some(sample_error("E_JOURNAL_PARTIAL"));
    assert!(report.validate().is_ok());

    let mut report = Report::new(ReportStatus::Running, "run");
    report.job = Some(Record::new(RecordKind::Job));
    assert!(report.validate().is_ok());

    let report = Report::new(ReportStatus::Planned, "remove");
    assert!(report.validate().is_ok());
}

fn rolled_back_error() -> nurt_contracts::error::Error {
    serde_json::from_value(serde_json::json!({
        "code": "E_ROLLED_BACK",
        "message": "the verification failed and the effect was rolled back",
        "cause": { "code": "E_ASSERT", "message": "exit code 1" }
    }))
    .unwrap()
}

#[test]
fn rolled_back_carries_the_rolled_back_error_with_its_cause() {
    let mut report = Report::new(ReportStatus::RolledBack, "replace");
    report.error = Some(sample_error("E_STALE"));
    assert_eq!(
        report.validate(),
        Err(ReportViolation::RolledBackWithoutCause)
    );

    report.error = Some(sample_error("E_ROLLED_BACK"));
    let violation = report.validate().unwrap_err();
    assert_eq!(violation, ReportViolation::RolledBackWithoutCause);
    assert_eq!(
        violation.to_string(),
        "a report with status `rolled_back` must carry the error `E_ROLLED_BACK` with a cause"
    );

    let mut cause_only = rolled_back_error();
    cause_only.code = nurt_contracts::error::ErrorCode::Stale;
    report.error = Some(cause_only);
    assert_eq!(
        report.validate(),
        Err(ReportViolation::RolledBackWithoutCause)
    );

    report.error = Some(rolled_back_error());
    assert!(report.validate().is_ok());
}

#[test]
fn rename_with_from_equal_to_path_is_valid_when_to_gives_the_destination() {
    // Producers write the source in `path`, repeat it in `from` and give the
    // destination in `to`; the format does not forbid the repetition.
    let mut report = ok_report();
    let mut change = Change::new("/d/a.txt", ChangeAction::Rename);
    change.from = Some("/d/a.txt".into());
    change.to = Some("/d/b.txt".into());
    report.changes.push(change);
    assert!(report.validate().is_ok());
}

#[test]
fn hunks_admit_the_no_newline_marker() {
    let mut report = ok_report();
    report.diff.push(nurt_contracts::report::Hunk {
        path: "/d/a.txt".into(),
        lines: vec![
            "-a".to_owned(),
            "\\ No newline at end of file".to_owned(),
            "+a".to_owned(),
        ],
        ..Default::default()
    });
    assert!(report.validate().is_ok());
}

#[test]
fn listed_is_compared_with_the_changes_of_the_page() {
    let mut report = ok_report();
    report.targets = Some(Targets {
        count: 1,
        listed: Some(0),
        ..Targets::default()
    });
    assert!(report.validate().is_ok());

    report.targets = Some(Targets {
        count: 1,
        listed: Some(u64::MAX),
        ..Targets::default()
    });
    assert_eq!(
        report.validate(),
        Err(ReportViolation::ListedMismatch {
            listed: u64::MAX,
            changes: 0
        })
    );
}

#[test]
fn hunk_lines_need_a_prefix_even_when_empty() {
    let mut report = ok_report();
    report.diff.push(Hunk {
        path: "/srv/app/a.go".into(),
        lines: vec![" context".to_owned(), String::new()],
        ..Hunk::default()
    });
    assert_eq!(
        report.validate(),
        Err(ReportViolation::InvalidHunkLine {
            path: "/srv/app/a.go".to_owned(),
            line: 1
        })
    );
}

#[test]
fn violations_describe_the_broken_rule() {
    let cases = [
        (
            ReportViolation::UnsupportedVersion {
                found: "2.0".to_owned(),
            },
            "unsupported report version `2.0`",
        ),
        (ReportViolation::EmptyKind, "the operation kind is empty"),
        (ReportViolation::ZeroOp, "operation numbers start at 1"),
        (
            ReportViolation::InvalidAstHash {
                found: "sha256:x".to_owned(),
            },
            "invalid syntax tree hash `sha256:x`",
        ),
        (
            ReportViolation::EmptyResultFile,
            "the result file path is empty",
        ),
        (
            ReportViolation::EmptyCursor,
            "the cursor of the next page is empty",
        ),
        (
            ReportViolation::MissingError {
                status: ReportStatus::RolledBack,
            },
            "status `rolled_back` must carry an error",
        ),
        (
            ReportViolation::OpInPreview,
            "a preview report must not carry an operation number",
        ),
        (
            ReportViolation::MissingJob,
            "status `running` must carry a job",
        ),
        (
            ReportViolation::MissingOp {
                status: ReportStatus::Partial,
            },
            "status `partial` must carry an operation number",
        ),
        (
            ReportViolation::PartialWithoutRemainder,
            "must list irreversible effects or an error",
        ),
        (
            ReportViolation::RanWithoutCorrections,
            "present without corrections",
        ),
        (
            ReportViolation::InvalidCorrectionRule {
                rule: "X1".to_owned(),
            },
            "invalid correction rule identifier `X1`",
        ),
        (
            ReportViolation::InvalidHunkLine {
                path: "/a".to_owned(),
                line: 2,
            },
            "line 2 of a hunk of `/a`",
        ),
        (
            ReportViolation::NotableOrder { index: 3 },
            "notable entry 3",
        ),
    ];
    for (violation, fragment) in cases {
        assert!(
            violation.to_string().contains(fragment),
            "{violation:?}: {violation}"
        );
    }
}

#[test]
fn version_and_hash_checks_reject_near_misses() {
    for version in [
        "",
        "1.",
        "1",
        "1.x",
        "1.0 ",
        " 1.0",
        "10.0",
        "1.0.0",
        "1.-1",
        "1.\u{663}",
    ] {
        assert!(!is_supported_version(version), "{version:?}");
    }
    for version in ["1.0", "1.00", "1.12345678901234567890"] {
        assert!(is_supported_version(version), "{version:?}");
    }

    let digits = "0123456789abcdef".repeat(4);
    assert!(is_ast_hash(&format!("sha256:{digits}")));
    for hash in [
        String::new(),
        "sha256:".to_owned(),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha256:{}", "a".repeat(65)),
        format!("sha256:{}", "\u{e9}".repeat(32)),
        format!("SHA256:{digits}"),
        format!(" sha256:{digits}"),
        format!("sha256:{}", digits.to_uppercase()),
    ] {
        assert!(!is_ast_hash(&hash), "{hash:?}");
    }
}

#[test]
fn rule_identifiers_have_one_letter_and_two_digits() {
    for rule in ["L00", "T99", "V10", "B01", "P42"] {
        assert!(Correction::is_rule_id(rule), "{rule}");
    }
    for rule in [
        "", "L", "L0", "L001", "L0a", "l01", "Q01", "L\u{e9}1", "01L",
    ] {
        assert!(!Correction::is_rule_id(rule), "{rule:?}");
    }
}

#[test]
fn sorting_notable_entries_keeps_the_order_of_equal_severity() {
    let mut report = ok_report();
    for code in [
        Notable::ManyPerFile,
        Notable::DeleteTree,
        Notable::SystemPath,
        Notable::TargetCountHigh,
    ] {
        report.notable.push(NotableEntry::new(code));
    }
    report.sort_notable();
    let order: Vec<Notable> = report.notable.iter().map(|entry| entry.code).collect();
    assert_eq!(
        order,
        [
            Notable::SystemPath,
            Notable::ManyPerFile,
            Notable::DeleteTree,
            Notable::TargetCountHigh
        ]
    );
    assert!(report.validate().is_ok());
}

#[test]
fn a_report_has_more_pages_while_it_carries_a_cursor() {
    let mut report = ok_report();
    assert!(!report.has_more());
    report.next_cursor = Some("c_91fe".to_owned());
    assert!(report.has_more());
    assert!(report.validate().is_ok());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
