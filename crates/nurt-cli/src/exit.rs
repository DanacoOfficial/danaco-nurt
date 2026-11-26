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

/// Exit code of a report: success for `ok`, `planned` and `running` without an
/// error, the code of the error otherwise. A report with status `ok` that
/// carries an error (a check that failed after the effect was applied) exits
/// with the code of the error, so that a script does not go on after it.
#[must_use]
pub fn for_report(report: &Report) -> u8 {
    let error = |report: &Report| {
        report
            .error
            .as_ref()
            .map_or(FAILURE, |error| for_error_code(error.code))
    };
    match report.status {
        ReportStatus::Ok | ReportStatus::Planned | ReportStatus::Running => {
            if report.error.is_some() {
                error(report)
            } else {
                SUCCESS
            }
        }
        _ => error(report),
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
    fn a_check_that_failed_after_the_effect_exits_with_its_error_code() {
        let mut report = Report::new(ReportStatus::Ok, "remove");
        report.error = Some(Error::new(ErrorCode::Assert, "check failed"));
        assert_eq!(for_report(&report), FAILURE);
        report.error = Some(Error::new(ErrorCode::Timeout, "too slow"));
        assert_eq!(for_report(&report), TIMEOUT);
    }

    #[test]
    fn failures_follow_the_interface_status_of_the_error() {
        assert_eq!(for_report(&failed(ErrorCode::Timeout)), TIMEOUT);
        assert_eq!(for_report(&failed(ErrorCode::Cancelled)), CANCELLED);
        assert_eq!(for_report(&failed(ErrorCode::Internal)), INTERNAL);
        assert_eq!(for_report(&failed(ErrorCode::UnknownCommand)), FAILURE);
    }

    #[test]
    fn the_codes_are_those_of_the_reference() {
        assert_eq!(
            [
                SUCCESS,
                FAILURE,
                USAGE,
                UNAVAILABLE,
                TIMEOUT,
                INTERNAL,
                CANCELLED
            ],
            [0, 1, 2, 3, 4, 70, 130]
        );
    }

    #[test]
    fn errors_of_the_text_and_of_the_storage_exit_with_one() {
        for code in [
            ErrorCode::Syntax,
            ErrorCode::NotFound,
            ErrorCode::Unsupported,
            ErrorCode::UnknownCommand,
        ] {
            assert_eq!(for_error_code(code), FAILURE, "{code:?}");
        }
    }

    #[test]
    fn a_failed_status_with_each_special_error_takes_its_code() {
        for (code, expected) in [
            (ErrorCode::Timeout, TIMEOUT),
            (ErrorCode::Cancelled, CANCELLED),
            (ErrorCode::Internal, INTERNAL),
            (ErrorCode::Syntax, FAILURE),
        ] {
            assert_eq!(for_report(&failed(code)), expected, "{code:?}");
            let mut rolled_back = Report::new(ReportStatus::RolledBack, "remove");
            rolled_back.error = Some(Error::new(code, "failed"));
            assert_eq!(for_report(&rolled_back), expected, "{code:?}");
        }
    }

    #[test]
    fn failure_without_an_error_exits_with_one() {
        let report = Report::new(ReportStatus::RolledBack, "remove");
        assert_eq!(for_report(&report), FAILURE);
    }
}
