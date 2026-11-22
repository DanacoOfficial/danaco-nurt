// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Levels of Windows events.
//!
//! The `Level` of an event is a number defined by its provider. The values 1
//! to 5 are the standard levels of the Windows Event Log. The value 0
//! (`LogAlways`) is used by events written regardless of the level, among
//! them the audit events of the `Security` channel. Values from 16 on are
//! levels of providers.

use nurt_contracts::record::LogLevel;

use crate::event::{
    PRIORITY_CRITICAL, PRIORITY_DEBUG, PRIORITY_ERROR, PRIORITY_INFO, PRIORITY_WARNING,
};

/// `Level` of critical events.
pub const CRITICAL: u8 = 1;
/// `Level` of errors.
pub const ERROR: u8 = 2;
/// `Level` of warnings.
pub const WARNING: u8 = 3;
/// `Level` of verbose events.
pub const VERBOSE: u8 = 5;

/// Severity of an event for its `Level`.
///
/// `LogAlways`, `Information` and every level outside the standard range
/// have the severity of informational messages. The raw value is kept in the
/// fields of the event.
///
/// # Examples
///
/// ```
/// use nurt_syslog::eventlog::level::priority;
/// assert_eq!(priority(1), 2);
/// assert_eq!(priority(0), 6);
/// assert_eq!(priority(16), 6);
/// ```
#[must_use]
pub const fn priority(level: u8) -> u8 {
    match level {
        CRITICAL => PRIORITY_CRITICAL,
        ERROR => PRIORITY_ERROR,
        WARNING => PRIORITY_WARNING,
        VERBOSE => PRIORITY_DEBUG,
        _ => PRIORITY_INFO,
    }
}

/// XPath condition on `Level` that selects the events at the threshold or
/// above it.
///
/// The result is `None` when every event qualifies. The condition selects
/// exactly the events whose [`priority`] the threshold admits, so the
/// threshold `info` excludes only `Verbose`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::record::LogLevel;
/// use nurt_syslog::eventlog::level::xpath_condition;
/// assert_eq!(xpath_condition(LogLevel::Error).as_deref(), Some("(Level=1 or Level=2)"));
/// assert_eq!(xpath_condition(LogLevel::Debug), None);
/// ```
#[must_use]
pub fn xpath_condition(threshold: LogLevel) -> Option<String> {
    let levels: &[u8] = match threshold {
        LogLevel::Critical => &[CRITICAL],
        LogLevel::Error => &[CRITICAL, ERROR],
        LogLevel::Warning => &[CRITICAL, ERROR, WARNING],
        LogLevel::Debug => return None,
        _ => return Some(format!("(Level<{VERBOSE} or Level>{VERBOSE})")),
    };
    let terms: Vec<String> = levels.iter().map(|level| format!("Level={level}")).collect();
    Some(format!("({})", terms.join(" or ")))
}
