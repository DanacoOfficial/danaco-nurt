// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt::error
// Responsibility:   Call errors and status codes.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Call errors and status codes.
//!
//! A call error means that the call itself failed: the executable was not
//! found, the process ended, the protocol was broken or an argument was
//! rejected. Errors of the language (syntax, binding, assertions, stale
//! targets) are not call errors; they are part of a successful
//! [`Response`](crate::Response) and are read with
//! [`Response::error`](crate::Response::error).

use std::fmt;

/// Status code of a failed call, as defined by the C interface (`nurt.h`).
///
/// The numeric codes are stable within a major version of the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Status {
    /// The operation did not finish within the time limit (`NURT_E_TIMEOUT`).
    Timeout,
    /// The operation ended because it was cancelled (`NURT_E_CANCELLED`).
    Cancelled,
    /// An argument is not valid (`NURT_E_ARG`).
    InvalidArgument,
    /// The interface versions are not compatible (`NURT_E_ABI`).
    Abi,
    /// A file, the state directory or the executable cannot be used
    /// (`NURT_E_IO`).
    Io,
    /// Internal failure (`NURT_E_INTERNAL`).
    Internal,
}

impl Status {
    /// Every status, in the order of the numeric codes.
    pub const ALL: &'static [Self] = &[
        Self::Timeout,
        Self::Cancelled,
        Self::InvalidArgument,
        Self::Abi,
        Self::Io,
        Self::Internal,
    ];

    /// Numeric code of the status in `nurt.h`.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(nurt::Status::InvalidArgument.code(), 3);
    /// ```
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::Timeout => 1,
            Self::Cancelled => 2,
            Self::InvalidArgument => 3,
            Self::Abi => 4,
            Self::Io => 5,
            Self::Internal => 6,
        }
    }

    /// Name of the status in `nurt.h`, for example `NURT_E_ARG`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Timeout => "NURT_E_TIMEOUT",
            Self::Cancelled => "NURT_E_CANCELLED",
            Self::InvalidArgument => "NURT_E_ARG",
            Self::Abi => "NURT_E_ABI",
            Self::Io => "NURT_E_IO",
            Self::Internal => "NURT_E_INTERNAL",
        }
    }

    /// Status with the given numeric code; `None` for `0` (`NURT_OK`) and
    /// for codes unknown to this version.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt::Status;
    ///
    /// assert_eq!(Status::from_code(6), Some(Status::Internal));
    /// assert_eq!(Status::from_code(0), None);
    /// ```
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|status| status.code() == code)
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Kind of a call error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// No response arrived within the time limit.
    Timeout,
    /// The operation was cancelled.
    Cancelled,
    /// An argument or a request was rejected.
    InvalidArgument,
    /// The server implements another major version of the protocol.
    Incompatible,
    /// A file or the state directory of the server cannot be used.
    Io,
    /// The server reported an internal failure.
    Internal,
    /// The `nurt` executable was not found.
    NotFound,
    /// The connection to the server is closed or the process has ended.
    Disconnected,
    /// The server sent a message that breaks the protocol.
    Protocol,
    /// A call that returns no report failed with a language error; the
    /// stable `E_*` code is available from [`Error::code`].
    Language,
}

impl ErrorKind {
    /// Status code of the C interface that corresponds to the kind.
    ///
    /// Kinds without their own status map to the nearest one: a missing
    /// executable and a closed connection to [`Status::Io`], a broken
    /// protocol to [`Status::Internal`] and a language error to
    /// [`Status::InvalidArgument`].
    #[must_use]
    pub const fn status(self) -> Status {
        match self {
            Self::Timeout => Status::Timeout,
            Self::Cancelled => Status::Cancelled,
            Self::InvalidArgument => Status::InvalidArgument,
            Self::Incompatible => Status::Abi,
            Self::Io | Self::NotFound | Self::Disconnected => Status::Io,
            Self::Internal | Self::Protocol => Status::Internal,
            Self::Language => Status::InvalidArgument,
        }
    }

    /// Kind that corresponds to a status code of the C interface.
    #[must_use]
    pub const fn from_status(status: Status) -> Self {
        match status {
            Status::Timeout => Self::Timeout,
            Status::Cancelled => Self::Cancelled,
            Status::InvalidArgument => Self::InvalidArgument,
            Status::Abi => Self::Incompatible,
            Status::Io => Self::Io,
            Status::Internal => Self::Internal,
        }
    }
}

/// Error of a call.
///
/// # Examples
///
/// ```
/// use nurt::{Error, ErrorKind, Status};
///
/// let error = Error::new(ErrorKind::Timeout, "no response to `run` within 30s");
/// assert_eq!(error.status(), Status::Timeout);
/// assert_eq!(error.to_string(), "no response to `run` within 30s");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    code: Option<String>,
}

impl Error {
    /// Error of the given kind with a message.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: None,
        }
    }

    /// Error of kind [`ErrorKind::Language`] with the stable code of the
    /// language error, for example `E_SYNTAX`.
    #[must_use]
    pub fn language(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Language,
            message: message.into(),
            code: Some(code.into()),
        }
    }

    /// Kind of the error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Status code of the C interface that corresponds to the error.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.kind.status()
    }

    /// Message of the error.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Stable `E_*` code of the language error; `None` for other kinds.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_match_the_c_interface() {
        let codes: Vec<(u32, &str)> = Status::ALL
            .iter()
            .map(|status| (status.code(), status.name()))
            .collect();
        assert_eq!(
            codes,
            [
                (1, "NURT_E_TIMEOUT"),
                (2, "NURT_E_CANCELLED"),
                (3, "NURT_E_ARG"),
                (4, "NURT_E_ABI"),
                (5, "NURT_E_IO"),
                (6, "NURT_E_INTERNAL"),
            ]
        );
    }

    #[test]
    fn unknown_status_codes_have_no_status() {
        assert_eq!(Status::from_code(0), None);
        assert_eq!(Status::from_code(7), None);
        assert_eq!(Status::from_code(u32::MAX), None);
    }

    #[test]
    fn every_status_round_trips_through_its_kind() {
        for status in Status::ALL {
            assert_eq!(ErrorKind::from_status(*status).status(), *status);
            assert_eq!(Status::from_code(status.code()), Some(*status));
            assert_eq!(status.to_string(), status.name());
        }
    }

    #[test]
    fn kinds_without_own_status_map_to_the_nearest_status() {
        assert_eq!(ErrorKind::NotFound.status(), Status::Io);
        assert_eq!(ErrorKind::Disconnected.status(), Status::Io);
        assert_eq!(ErrorKind::Protocol.status(), Status::Internal);
        assert_eq!(ErrorKind::Language.status(), Status::InvalidArgument);
    }

    #[test]
    fn language_error_keeps_its_code() {
        let error = Error::language("E_SYNTAX", "unclosed list");
        assert_eq!(error.kind(), ErrorKind::Language);
        assert_eq!(error.code(), Some("E_SYNTAX"));
        assert_eq!(error.to_string(), "unclosed list");
        assert_eq!(Error::new(ErrorKind::Io, "m").code(), None);
    }

    #[test]
    fn error_exposes_kind_and_message() {
        let error = Error::new(ErrorKind::Protocol, "invalid JSON from nurt serve");
        assert_eq!(error.kind(), ErrorKind::Protocol);
        assert_eq!(error.message(), "invalid JSON from nurt serve");
        assert_eq!(error.status(), Status::Internal);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
