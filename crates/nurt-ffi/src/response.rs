// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ffi::response
// Responsibility:   Responses produced by the test double without a recorded case.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Responses produced by the test double without a recorded case.

use nurt_contracts::envelope::{EnvelopeError, Response};
use nurt_contracts::error::{Error, ErrorCode, Stage};
use nurt_contracts::report::{Report, ReportStatus};

/// Operation kind of the report of a request that was rejected before it
/// named a command.
pub(crate) const REQUEST_KIND: &str = "request";

/// Response with status `failed` and `error`.
pub(crate) fn failure(kind: &str, error: Error) -> Response {
    let mut report = Report::new(ReportStatus::Failed, kind);
    report.error = Some(error);
    Response::new(report)
}

/// Response for a request envelope that matches the schema but breaks a rule
/// of the envelope: the language error `E_ARG`.
pub(crate) fn envelope_rule(violation: &EnvelopeError) -> Response {
    let error = Error::new(ErrorCode::Arg, violation.to_string()).with_stage(Stage::Bind);
    failure(REQUEST_KIND, error)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::envelope::EnvelopeError;
    use nurt_contracts::error::ErrorCode;

    use super::envelope_rule;

    #[test]
    fn envelope_rule_response_is_valid() {
        let response = envelope_rule(&EnvelopeError::MissingInput);
        response.validate().unwrap();
        let error = response.report.error.unwrap();
        assert_eq!(error.code, ErrorCode::Arg);
        assert!(error.message.contains("`program`"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
