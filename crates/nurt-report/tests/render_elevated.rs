// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/render_elevated.rs
// Responsibility:   The text form of the report of an elevated operation names the backend.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The text form of the report of an elevated operation names the backend.

// Test failures report through panics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_contracts::report::{Report, ReportStatus};
use nurt_report::{RenderOptions, respond};

fn first_line(report: &Report) -> String {
    let response = respond(report, &RenderOptions::default());
    response
        .rendered
        .unwrap()
        .text
        .lines()
        .next()
        .unwrap()
        .to_owned()
}

#[test]
fn the_header_names_the_backend_of_an_elevated_operation() {
    let mut report = Report::new(ReportStatus::Ok, "write");
    report.op = Some(4);
    assert_eq!(first_line(&report), "op 4  write  ok");
    report.elevated = true;
    report.backend = Some("sudo".to_owned());
    assert_eq!(first_line(&report), "op 4  write  ok  elevated sudo");
    report.as_user = Some("deploy".to_owned());
    assert_eq!(
        first_line(&report),
        "op 4  write  ok  elevated sudo as deploy"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
