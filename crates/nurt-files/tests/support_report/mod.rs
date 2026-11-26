// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// A failed assertion is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Every test crate that includes this module uses a part of it.
#![allow(dead_code)]

//! Checks of the report rules on the changes of a stage: the changes of a
//! plan or of an effect record are put into a report that the engine would
//! return, and the report is validated with the rules of `nurt-contracts`.

use nurt_contracts::record::{Change, ChangeAction, Effect};
use nurt_contracts::report::{Report, ReportStatus};

/// Validates a report of the given status that carries `changes`, and checks
/// that the report survives a round trip through JSON.
fn assert_report(status: ReportStatus, kind: &str, changes: &[Change]) {
    let mut report = Report::new(status, kind);
    if status == ReportStatus::Ok {
        report.op = Some(1);
    }
    report.changes = changes.to_vec();
    if let Err(violation) = report.validate() {
        panic!("the {status} report of `{kind}` breaks a report rule: {violation:?}");
    }
    let json = serde_json::to_value(&report).expect("a report serializes");
    let decoded: Report = serde_json::from_value(json).expect("a report deserializes");
    assert_eq!(decoded, report, "the report of `{kind}` changes in JSON");
}

/// The changes of a plan form a valid preview report; call it before
/// applying the plan.
pub(crate) fn assert_plan_valid(kind: &str, changes: &[Change]) {
    assert_report(ReportStatus::Planned, kind, changes);
    assert_actions_complete(kind, changes);
}

/// The changes of an applied stage form a valid report of a finished run.
pub(crate) fn assert_changes_valid(kind: &str, changes: &[Change]) {
    assert_report(ReportStatus::Ok, kind, changes);
    assert_actions_complete(kind, changes);
}

/// The effect record of an applied stage forms a valid report.
pub(crate) fn assert_effect_valid(effect: &Effect) {
    let changes = effect.changes.clone().unwrap_or_default();
    assert_changes_valid(&effect.kind, &changes);
}

/// A rename names the previous path; the report rules also demand it, and
/// the message here names the command.
fn assert_actions_complete(kind: &str, changes: &[Change]) {
    for change in changes
        .iter()
        .filter(|change| change.action == ChangeAction::Rename)
    {
        assert!(
            change.from.is_some(),
            "`{kind}`: a rename of {:?} has no `from`",
            change.path
        );
    }
}
