// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-ffi::error
// Responsibility:   Status codes and the failure record of the calling thread.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Status codes and the failure record of the calling thread.

use std::cell::RefCell;

use nurt_contracts::error::AbiStatus;
use serde_json::{Map, Value, json};

/// Result of a call of the library (`nurt_status`).
///
/// Only failures of the call itself are reported here; errors of the
/// language are carried by the JSON result of a call that returns
/// [`NurtStatus::Ok`]. A later minor version of the interface may add
/// codes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NurtStatus {
    /// `NURT_OK`: the call succeeded.
    Ok = 0,
    /// `NURT_E_TIMEOUT`: the execution is still running after the timeout.
    Timeout = 1,
    /// `NURT_E_CANCELLED`: the execution ended because it was cancelled.
    Cancelled = 2,
    /// `NURT_E_ARG`: a null pointer, invalid UTF-8 or invalid JSON.
    Arg = 3,
    /// `NURT_E_ABI`: incompatible interface version.
    Abi = 4,
    /// `NURT_E_IO`: a file of the instance cannot be opened or read.
    Io = 5,
    /// `NURT_E_INTERNAL`: internal failure, including a caught panic.
    Internal = 6,
}

impl NurtStatus {
    /// Every status in numeric order.
    pub const ALL: &'static [Self] = &[
        Self::Ok,
        Self::Timeout,
        Self::Cancelled,
        Self::Arg,
        Self::Abi,
        Self::Io,
        Self::Internal,
    ];

    /// Numeric value of the status.
    #[must_use]
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// Status of the contract with the same value.
    #[must_use]
    pub const fn abi(self) -> AbiStatus {
        match self {
            Self::Ok => AbiStatus::Ok,
            Self::Timeout => AbiStatus::Timeout,
            Self::Cancelled => AbiStatus::Cancelled,
            Self::Arg => AbiStatus::Arg,
            Self::Abi => AbiStatus::Abi,
            Self::Io => AbiStatus::Io,
            Self::Internal => AbiStatus::Internal,
        }
    }

    /// Name of the constant in `nurt.h`, for example `NURT_E_ARG`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.abi().name()
    }
}

impl From<AbiStatus> for NurtStatus {
    fn from(status: AbiStatus) -> Self {
        match status {
            AbiStatus::Ok => Self::Ok,
            AbiStatus::Timeout => Self::Timeout,
            AbiStatus::Cancelled => Self::Cancelled,
            AbiStatus::Arg => Self::Arg,
            AbiStatus::Abi => Self::Abi,
            AbiStatus::Io => Self::Io,
            _ => Self::Internal,
        }
    }
}

/// Failure of a call: the status and the record returned by
/// `nurt_last_error`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CallError {
    status: NurtStatus,
    message: String,
    detail: Map<String, Value>,
}

impl CallError {
    pub(crate) fn new(status: NurtStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            detail: Map::new(),
        }
    }

    pub(crate) fn arg(message: impl Into<String>) -> Self {
        Self::new(NurtStatus::Arg, message)
    }

    pub(crate) fn abi(message: impl Into<String>) -> Self {
        Self::new(NurtStatus::Abi, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(NurtStatus::Internal, message)
    }

    pub(crate) fn with_detail(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.detail.insert(key.to_owned(), value.into());
        self
    }

    pub(crate) const fn status(&self) -> NurtStatus {
        self.status
    }

    fn to_json(&self, function: &str) -> Vec<u8> {
        let mut detail = Map::new();
        detail.insert("function".to_owned(), Value::from(function));
        detail.extend(self.detail.clone());
        let record = json!({
            "status": self.status.code(),
            "code": self.status.name(),
            "message": self.message,
            "detail": detail,
        });
        serde_json::to_vec(&record).unwrap_or_default()
    }
}

thread_local! {
    static LAST_ERROR: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// Replaces the failure record of the calling thread.
pub(crate) fn record(function: &str, error: &CallError) {
    let json = error.to_json(function);
    LAST_ERROR.with(|last| *last.borrow_mut() = Some(json));
}

/// Failure record of the calling thread as JSON, when there is one.
pub(crate) fn last() -> Option<Vec<u8>> {
    LAST_ERROR.with(|last| last.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::{CallError, NurtStatus, last, record};
    use nurt_contracts::error::AbiStatus;

    #[test]
    fn statuses_match_the_contract() {
        for (status, abi) in NurtStatus::ALL.iter().zip(AbiStatus::ALL) {
            assert_eq!(status.abi(), *abi);
            assert_eq!(status.code(), abi.code());
            assert_eq!(NurtStatus::from(*abi), *status);
        }
    }

    #[test]
    fn record_holds_status_code_message_and_function() {
        let error = CallError::arg("`program` must not be null").with_detail("argument", "program");
        record("nurt_parse", &error);
        let json: serde_json::Value = serde_json::from_slice(&last().unwrap()).unwrap();
        assert_eq!(json["status"], 3);
        assert_eq!(json["code"], "NURT_E_ARG");
        assert_eq!(json["message"], "`program` must not be null");
        assert_eq!(json["detail"]["function"], "nurt_parse");
        assert_eq!(json["detail"]["argument"], "program");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
