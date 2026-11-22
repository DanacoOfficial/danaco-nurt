// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Predicates of the unified log store.
//!
//! The log store filters entries with an `NSPredicate` before they reach the
//! reader. Names are passed as arguments of the format (`%@`) and are never
//! written into the format text, so no quoting is needed. Glob patterns are
//! not passed to the store; the final filter of the crate decides them.

use nurt_contracts::record::LogLevel;

use super::level;

/// Format and arguments of an `NSPredicate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    /// Format text with one `%@` per argument.
    pub format: String,
    /// Arguments of the format, in order.
    pub arguments: Vec<String>,
}

/// Predicate for the exact names of a subsystem and a process and a level
/// threshold.
///
/// The result is `None` when no condition applies.
///
/// # Examples
///
/// ```
/// use nurt_contracts::record::LogLevel;
/// use nurt_syslog::unified::predicate::predicate;
/// let built = predicate(Some("com.example.agent"), None, LogLevel::Error).unwrap();
/// assert_eq!(built.format, "subsystem == %@ AND messageType IN {16, 17}");
/// assert_eq!(built.arguments, ["com.example.agent"]);
/// assert_eq!(predicate(None, None, LogLevel::Debug), None);
/// ```
#[must_use]
pub fn predicate(subsystem: Option<&str>, process: Option<&str>, level: LogLevel) -> Option<Predicate> {
    let mut terms = Vec::new();
    let mut arguments = Vec::new();
    for (key, name) in [("subsystem", subsystem), ("process", process)] {
        if let Some(name) = name {
            terms.push(format!("{key} == %@"));
            arguments.push(name.to_owned());
        }
    }
    if let Some(types) = level::message_types(level) {
        let values: Vec<String> = types.iter().map(u8::to_string).collect();
        terms.push(format!("messageType IN {{{}}}", values.join(", ")));
    }
    (!terms.is_empty()).then(|| Predicate {
        format: terms.join(" AND "),
        arguments,
    })
}
