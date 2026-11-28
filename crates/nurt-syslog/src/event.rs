// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::event
// Responsibility:   Log events.
// Dependencies:     indexmap, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Log events.
//!
//! [`LogEvent`] is the event that every reader produces. It keeps the
//! numeric severity (`priority`, 0–7 as in RFC 5424) and the store of
//! origin next to the fields of the record `log_event`, and converts to that
//! record with [`LogEvent::to_record`].

use indexmap::IndexMap;
use nurt_contracts::record::{LogEvent as LogEventRecord, LogLevel};
use nurt_contracts::value::{Instant, Record};

use crate::cursor::Cursor;

/// Highest (least important) severity: debugging.
pub const PRIORITY_DEBUG: u8 = 7;
/// Severity of informational messages, used when a store gives none.
pub const PRIORITY_INFO: u8 = 6;
/// Severity of notices.
pub const PRIORITY_NOTICE: u8 = 5;
/// Severity of warnings.
pub const PRIORITY_WARNING: u8 = 4;
/// Severity of errors.
pub const PRIORITY_ERROR: u8 = 3;
/// Severity of critical conditions; 0 (emergency) and 1 (alert) are more
/// important still and map to the same level.
pub const PRIORITY_CRITICAL: u8 = 2;

/// Store that produced an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreKind {
    /// systemd journal files.
    Journald,
    /// macOS unified log.
    UnifiedLog,
    /// Windows event log.
    EventLog,
}

impl StoreKind {
    /// Name of the store as written in reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Journald => "journald",
            Self::UnifiedLog => "unified_log",
            Self::EventLog => "event_log",
        }
    }
}

/// Conditions of an event that changed its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventFlags {
    /// A value was cut or fields were dropped because of a limit.
    pub truncated: bool,
    /// The message could not be formatted and holds the raw values.
    pub unformatted: bool,
    /// Bytes that are not UTF-8 were replaced by U+FFFD.
    pub lossy: bool,
}

/// Event of a system log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent {
    /// Time of the event.
    pub time: Instant,
    /// Program, provider or process that logged the event.
    pub source: String,
    /// Severity 0 (emergency) to 7 (debugging).
    pub priority: u8,
    /// Message text.
    pub message: String,
    /// Identifier of the kind of message.
    pub id: Option<String>,
    /// Unit, channel or subsystem.
    pub unit: Option<String>,
    /// Host name.
    pub host: Option<String>,
    /// Process identifier.
    pub pid: Option<u64>,
    /// Thread identifier.
    pub tid: Option<u64>,
    /// Account as a decimal user identifier or a security identifier.
    pub user: Option<String>,
    /// Boot identifier as 32 hexadecimal digits.
    pub boot_id: Option<String>,
    /// Category of the event.
    pub category: Option<String>,
    /// Other fields in the order of the store; a name may repeat.
    pub fields: Vec<(String, String)>,
    /// Conditions that changed the content.
    pub flags: EventFlags,
    /// Position of the event in its store.
    pub cursor: Cursor,
    /// Store that produced the event.
    pub store: StoreKind,
}

impl LogEvent {
    /// Event with the required values and no optional ones.
    ///
    /// ```
    /// use nurt_contracts::record::LogLevel;
    /// use nurt_contracts::value::Instant;
    /// use nurt_syslog::cursor::Cursor;
    /// use nurt_syslog::event::{LogEvent, StoreKind};
    ///
    /// let cursor = Cursor::EventLog { channel: "System".to_owned(), record_id: 1 };
    /// let event = LogEvent::new(Instant::from_unix_nanos(0), "app", 3, "failed", StoreKind::EventLog, cursor);
    /// assert_eq!(event.level(), LogLevel::Error);
    /// ```
    #[must_use]
    pub fn new(
        time: Instant,
        source: &str,
        priority: u8,
        message: &str,
        store: StoreKind,
        cursor: Cursor,
    ) -> Self {
        Self {
            time,
            source: source.to_owned(),
            priority,
            message: message.to_owned(),
            id: None,
            unit: None,
            host: None,
            pid: None,
            tid: None,
            user: None,
            boot_id: None,
            category: None,
            fields: Vec::new(),
            flags: EventFlags::default(),
            cursor,
            store,
        }
    }

    /// Level of the event derived from its priority.
    #[must_use]
    pub fn level(&self) -> LogLevel {
        level_of_priority(self.priority)
    }

    /// Converts the event to the record `log_event`.
    ///
    /// Repeated names in [`LogEvent::fields`] keep the first value under the
    /// name and later values under `NAME#2`, `NAME#3` and so on.
    #[must_use]
    pub fn to_record(&self) -> LogEventRecord {
        LogEventRecord {
            time: self.time,
            source: self.source.clone(),
            level: self.level(),
            message: self.message.clone(),
            id: self.id.clone(),
            host: self.host.clone(),
            unit: self.unit.clone(),
            pid: self.pid,
            boot_id: self.boot_id.clone(),
            cursor: Some(self.cursor.to_string()),
            fields: (!self.fields.is_empty()).then(|| field_map(&self.fields)),
        }
    }

    /// Converts the event to a dynamic record of kind `log_event`.
    #[must_use]
    pub fn into_record(self) -> Record {
        Record::from(self.to_record())
    }
}

fn field_map(fields: &[(String, String)]) -> IndexMap<String, String> {
    let mut map = IndexMap::with_capacity(fields.len());
    let mut seen: IndexMap<&str, usize> = IndexMap::new();
    for (name, value) in fields {
        let count = seen.entry(name.as_str()).or_insert(0);
        *count += 1;
        let key = if *count == 1 {
            name.clone()
        } else {
            format!("{name}#{count}")
        };
        map.insert(key, value.clone());
    }
    map
}

/// Level of a severity: 0–2 `critical`, 3 `error`, 4 `warning`, 5–6 `info`,
/// 7 and above `debug`.
///
/// ```
/// use nurt_contracts::record::LogLevel;
/// use nurt_syslog::event::level_of_priority;
///
/// assert_eq!(level_of_priority(0), LogLevel::Critical);
/// assert_eq!(level_of_priority(5), LogLevel::Info);
/// ```
#[must_use]
pub const fn level_of_priority(priority: u8) -> LogLevel {
    match priority {
        0..=PRIORITY_CRITICAL => LogLevel::Critical,
        PRIORITY_ERROR => LogLevel::Error,
        PRIORITY_WARNING => LogLevel::Warning,
        PRIORITY_NOTICE | PRIORITY_INFO => LogLevel::Info,
        _ => LogLevel::Debug,
    }
}

/// Highest priority value that a level threshold admits.
///
/// The parameter `level` is a threshold: `error` returns events of level
/// `critical` and `error`, that is priorities 0–3.
///
/// ```
/// use nurt_contracts::record::LogLevel;
/// use nurt_syslog::event::max_priority;
///
/// assert_eq!(max_priority(LogLevel::Error), 3);
/// assert_eq!(max_priority(LogLevel::Info), 6);
/// ```
#[must_use]
pub const fn max_priority(level: LogLevel) -> u8 {
    match level {
        LogLevel::Critical => PRIORITY_CRITICAL,
        LogLevel::Error => PRIORITY_ERROR,
        LogLevel::Warning => PRIORITY_WARNING,
        LogLevel::Debug => PRIORITY_DEBUG,
        _ => PRIORITY_INFO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Id128;
    use nurt_contracts::value::{RecordKind, Value};

    fn sample() -> LogEvent {
        let mut event = LogEvent::new(
            Instant::from_unix_nanos(1_000),
            "sshd",
            4,
            "disconnected",
            StoreKind::Journald,
            Cursor::Journal {
                file_id: Id128::new([1; 16]),
                seqnum_id: Id128::new([2; 16]),
                seqnum: 3,
                realtime: 1,
            },
        );
        event.unit = Some("ssh.service".to_owned());
        event.pid = Some(42);
        event
    }

    #[test]
    fn every_priority_maps_to_a_level() {
        let levels: Vec<_> = (0..=8).map(level_of_priority).collect();
        assert_eq!(
            levels,
            [
                LogLevel::Critical,
                LogLevel::Critical,
                LogLevel::Critical,
                LogLevel::Error,
                LogLevel::Warning,
                LogLevel::Info,
                LogLevel::Info,
                LogLevel::Debug,
                LogLevel::Debug,
            ]
        );
    }

    #[test]
    fn threshold_admits_exactly_levels_at_or_above() {
        for level in [
            LogLevel::Critical,
            LogLevel::Error,
            LogLevel::Warning,
            LogLevel::Info,
            LogLevel::Debug,
        ] {
            let max = max_priority(level);
            for priority in 0..=PRIORITY_DEBUG {
                let rank = |l: LogLevel| max_priority(l);
                let admitted = rank(level_of_priority(priority)) <= rank(level);
                assert_eq!(priority <= max, admitted, "{level:?} {priority}");
            }
        }
    }

    #[test]
    fn store_names_match_report_values() {
        assert_eq!(StoreKind::Journald.as_str(), "journald");
        assert_eq!(StoreKind::UnifiedLog.as_str(), "unified_log");
        assert_eq!(StoreKind::EventLog.as_str(), "event_log");
    }

    #[test]
    fn converts_to_record_with_cursor_text() {
        let record = sample().to_record();
        assert_eq!(record.level, LogLevel::Warning);
        assert_eq!(record.unit.as_deref(), Some("ssh.service"));
        assert_eq!(record.pid, Some(42));
        assert!(record.cursor.as_deref().unwrap().starts_with("v1:j:"));
        assert!(record.fields.is_none());
    }

    #[test]
    fn repeated_field_names_get_numbered_keys() {
        let mut event = sample();
        event.fields = vec![
            ("TAG".to_owned(), "a".to_owned()),
            ("OTHER".to_owned(), "x".to_owned()),
            ("TAG".to_owned(), "b".to_owned()),
            ("TAG".to_owned(), "c".to_owned()),
        ];
        let fields = event.to_record().fields.unwrap();
        let keys: Vec<_> = fields.keys().map(String::as_str).collect();
        assert_eq!(keys, ["TAG", "OTHER", "TAG#2", "TAG#3"]);
        assert_eq!(fields["TAG#3"], "c");
    }

    #[test]
    fn dynamic_record_has_kind_log_event() {
        let record = sample().into_record();
        assert_eq!(record.kind, RecordKind::LogEvent);
        assert_eq!(record.get("source"), Some(&Value::Text("sshd".to_owned())));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
