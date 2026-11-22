// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Exit codes of the `nurt` executable.
//!
//! The codes of a program follow the status of its report and the interface
//! status of its error ([`ErrorCode::abi`]), so that the command line and the
//! C interface describe the same outcomes.

use nurt_contracts::error::{AbiStatus, ErrorCode};
use nurt_contracts::report::{Report, ReportStatus};

/// The program finished (`ok`, `planned` or `running`).
pub const SUCCESS: u8 = 0;
/// The program failed with a language or execution error.
pub const FAILURE: u8 = 1;
/// The command line is invalid.
pub const USAGE: u8 = 2;
/// The subcommand is not available in this build.
pub const UNAVAILABLE: u8 = 3;
/// The program exceeded its time limit.
pub const TIMEOUT: u8 = 4;
/// Internal error.
pub const INTERNAL: u8 = 70;
/// The program was cancelled (interrupted).
pub const CANCELLED: u8 = 130;

/// Exit code of an error code.
#[must_use]
pub fn for_error_code(code: ErrorCode) -> u8 {
    match code.abi() {
        AbiStatus::Timeout => TIMEOUT,
        AbiStatus::Cancelled => CANCELLED,
        AbiStatus::Internal => INTERNAL,
        _ => FAILURE,
    }
}

/// Exit code of a report: success for `ok`, `planned` and `running`, the code
/// of the error otherwise.
#[must_use]
pub fn for_report(report: &Report) -> u8 {
    match report.status {
        ReportStatus::Ok | ReportStatus::Planned | ReportStatus::Running => SUCCESS,
        _ => report
            .error
            .as_ref()
            .map_or(FAILURE, |error| for_error_code(error.code)),
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::Error;

    use super::*;

    fn failed(code: ErrorCode) -> Report {
        let mut report = Report::new(ReportStatus::Failed, "remove");
        report.error = Some(Error::new(code, "failed"));
        report
    }

    #[test]
    fn successful_statuses_exit_with_zero() {
        for status in [
            ReportStatus::Ok,
            ReportStatus::Planned,
            ReportStatus::Running,
        ] {
            assert_eq!(for_report(&Report::new(status, "find")), SUCCESS);
        }
    }

    #[test]
    fn failures_follow_the_interface_status_of_the_error() {
        assert_eq!(for_report(&failed(ErrorCode::Timeout)), TIMEOUT);
        assert_eq!(for_report(&failed(ErrorCode::Cancelled)), CANCELLED);
        assert_eq!(for_report(&failed(ErrorCode::Internal)), INTERNAL);
        assert_eq!(for_report(&failed(ErrorCode::UnknownCommand)), FAILURE);
    }

    #[test]
    fn failure_without_an_error_exits_with_one() {
        let report = Report::new(ReportStatus::RolledBack, "remove");
        assert_eq!(for_report(&report), FAILURE);
    }
}
