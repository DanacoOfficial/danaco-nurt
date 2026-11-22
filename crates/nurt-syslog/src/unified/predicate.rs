// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Predicates of the unified log store.
//!
//! The `OSLog` framework filters entries with an `NSPredicate` before they
//! reach the reader. Names are passed as arguments of the format (`%@`) and
//! are never written into the format text, so no quoting is needed. Glob
//! patterns are not passed to the store; the final filter of the crate
//! decides them. Backends without the framework evaluate the same conditions
//! with [`Predicate::matches`].

use nurt_contracts::record::LogLevel;

use crate::event::max_priority;

use super::entry::UnifiedEntry;
use super::level;

/// Conditions of a query that the log store evaluates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    /// Exact name of the subsystem.
    pub subsystem: Option<String>,
    /// Exact name of the process.
    pub process: Option<String>,
    /// Lowest level of the entries; `None` admits every level.
    pub level: Option<LogLevel>,
}

impl Predicate {
    /// Predicate for the exact names of a subsystem and a process and a
    /// level threshold.
    ///
    /// The result is `None` when no condition applies.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::record::LogLevel;
    /// use nurt_syslog::unified::predicate::Predicate;
    /// let built = Predicate::new(Some("com.example.agent"), None, LogLevel::Error).unwrap();
    /// assert_eq!(built.format(), "subsystem == %@ AND messageType IN {16, 17}");
    /// assert_eq!(built.arguments(), ["com.example.agent"]);
    /// assert_eq!(Predicate::new(None, None, LogLevel::Debug), None);
    /// ```
    #[must_use]
    pub fn new(subsystem: Option<&str>, process: Option<&str>, level: LogLevel) -> Option<Self> {
        let predicate = Self {
            subsystem: subsystem.map(str::to_owned),
            process: process.map(str::to_owned),
            level: level::message_types(level).map(|_| level),
        };
        predicate.has_conditions().then_some(predicate)
    }

    /// Format text of the `NSPredicate`, with one `%@` per argument.
    #[must_use]
    pub fn format(&self) -> String {
        let mut terms = Vec::new();
        if self.subsystem.is_some() {
            terms.push("subsystem == %@".to_owned());
        }
        if self.process.is_some() {
            terms.push("process == %@".to_owned());
        }
        if let Some(types) = self.level.and_then(level::message_types) {
            let values: Vec<String> = types.iter().map(u8::to_string).collect();
            terms.push(format!("messageType IN {{{}}}", values.join(", ")));
        }
        terms.join(" AND ")
    }

    /// Arguments of the format, in order.
    #[must_use]
    pub fn arguments(&self) -> Vec<&str> {
        [self.subsystem.as_deref(), self.process.as_deref()]
            .into_iter()
            .flatten()
            .collect()
    }

    /// Whether the predicate has a condition on the level.
    #[must_use]
    pub const fn has_level(&self) -> bool {
        self.level.is_some()
    }

    /// The predicate without its condition on the level, or `None` when no
    /// other condition remains.
    ///
    /// A log store that does not accept the key `messageType` is queried
    /// with this predicate; the final filter then decides the level.
    #[must_use]
    pub fn without_level(&self) -> Option<Self> {
        let reduced = Self {
            level: None,
            ..self.clone()
        };
        reduced.has_conditions().then_some(reduced)
    }

    /// Whether an entry satisfies every condition.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::record::LogLevel;
    /// use nurt_syslog::unified::entry::UnifiedEntry;
    /// use nurt_syslog::unified::level::EntryLevel;
    /// use nurt_syslog::unified::predicate::Predicate;
    /// let predicate = Predicate::new(None, Some("agent"), LogLevel::Error).unwrap();
    /// let mut entry = UnifiedEntry {
    ///     level: EntryLevel::Fault,
    ///     process: "agent".to_owned(),
    ///     ..UnifiedEntry::default()
    /// };
    /// assert!(predicate.matches(&entry));
    /// entry.level = EntryLevel::Info;
    /// assert!(!predicate.matches(&entry));
    /// ```
    #[must_use]
    pub fn matches(&self, entry: &UnifiedEntry) -> bool {
        let subsystem = self
            .subsystem
            .as_deref()
            .is_none_or(|name| entry.subsystem.as_deref() == Some(name));
        let process = self
            .process
            .as_deref()
            .is_none_or(|name| entry.process == name);
        let level = self
            .level
            .is_none_or(|level| entry.level.priority() <= max_priority(level));
        subsystem && process && level
    }

    const fn has_conditions(&self) -> bool {
        self.subsystem.is_some() || self.process.is_some() || self.level.is_some()
    }
}
