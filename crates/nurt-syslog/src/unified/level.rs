// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Levels of entries of the unified log.
//!
//! An entry has the level of `OSLogEntryLog.Level`. Predicates of the log
//! store select entries by `messageType`, the `os_log_type_t` value of the
//! message.

use nurt_contracts::record::LogLevel;

use crate::event::{
    PRIORITY_CRITICAL, PRIORITY_DEBUG, PRIORITY_ERROR, PRIORITY_INFO, PRIORITY_NOTICE,
};

/// `messageType` of messages of type `default`.
pub const TYPE_DEFAULT: u8 = 0x00;
/// `messageType` of messages of type `info`.
pub const TYPE_INFO: u8 = 0x01;
/// `messageType` of messages of type `error`.
pub const TYPE_ERROR: u8 = 0x10;
/// `messageType` of messages of type `fault`.
pub const TYPE_FAULT: u8 = 0x11;

/// Level of an entry, as `OSLogEntryLog.Level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EntryLevel {
    /// The level is not known.
    #[default]
    Undefined,
    /// Debugging message.
    Debug,
    /// Informational message.
    Info,
    /// Message of type `default`.
    Notice,
    /// Error.
    Error,
    /// Fault.
    Fault,
}

impl EntryLevel {
    /// Level for the raw value of `OSLogEntryLog.Level`.
    ///
    /// The values are 0 for undefined, 1 for debug, 2 for info, 3 for notice,
    /// 4 for error and 5 for fault. Other values are undefined.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_syslog::unified::level::EntryLevel;
    /// assert_eq!(EntryLevel::from_raw(5), EntryLevel::Fault);
    /// assert_eq!(EntryLevel::from_raw(9), EntryLevel::Undefined);
    /// ```
    #[must_use]
    pub const fn from_raw(raw: i64) -> Self {
        match raw {
            1 => Self::Debug,
            2 => Self::Info,
            3 => Self::Notice,
            4 => Self::Error,
            5 => Self::Fault,
            _ => Self::Undefined,
        }
    }

    /// Severity of the entry, 0 to 7 as in RFC 5424.
    ///
    /// Fault is critical (2), error is error (3), the type `default` and an
    /// undefined level are notices (5), info is informational (6) and debug
    /// is debugging (7).
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_syslog::unified::level::EntryLevel;
    /// assert_eq!(EntryLevel::Fault.priority(), 2);
    /// assert_eq!(EntryLevel::Notice.priority(), 5);
    /// ```
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::Fault => PRIORITY_CRITICAL,
            Self::Error => PRIORITY_ERROR,
            Self::Undefined | Self::Notice => PRIORITY_NOTICE,
            Self::Info => PRIORITY_INFO,
            Self::Debug => PRIORITY_DEBUG,
        }
    }
}

/// Values of `messageType` of the entries at the threshold or above it.
///
/// The result is `None` when every entry qualifies. The unified log has no
/// warnings, so the threshold `warning` selects errors and faults like
/// `error`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::record::LogLevel;
/// use nurt_syslog::unified::level::{message_types, TYPE_ERROR, TYPE_FAULT};
/// assert_eq!(message_types(LogLevel::Warning), Some(&[TYPE_ERROR, TYPE_FAULT][..]));
/// assert_eq!(message_types(LogLevel::Debug), None);
/// ```
#[must_use]
pub fn message_types(threshold: LogLevel) -> Option<&'static [u8]> {
    match threshold {
        LogLevel::Critical => Some(&[TYPE_FAULT]),
        LogLevel::Error | LogLevel::Warning => Some(&[TYPE_ERROR, TYPE_FAULT]),
        LogLevel::Debug => None,
        _ => Some(&[TYPE_DEFAULT, TYPE_INFO, TYPE_ERROR, TYPE_FAULT]),
    }
}
