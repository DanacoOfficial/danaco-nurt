// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Size limits of log queries and records.
//!
//! Every limit has a configuration key under `logs.` and a default value.
//! Exceeding a limit on a record never fails the query: the value is cut and
//! the record carries the flag `truncated`. Only a requested `limit` above
//! [`LogLimits::max_limit`] is an error.

use nurt_contracts::error::{Error, ErrorCode};

use crate::event::LogEvent;

/// Configuration key of [`LogLimits::max_limit`].
pub const KEY_MAX_LIMIT: &str = "logs.max_limit";
/// Configuration key of [`LogLimits::max_message_bytes`].
pub const KEY_MAX_MESSAGE_BYTES: &str = "logs.max_message_bytes";
/// Configuration key of [`LogLimits::max_field_bytes`].
pub const KEY_MAX_FIELD_BYTES: &str = "logs.max_field_bytes";
/// Configuration key of [`LogLimits::max_fields`].
pub const KEY_MAX_FIELDS: &str = "logs.max_fields";
/// Configuration key of [`LogLimits::max_data_bytes`].
pub const KEY_MAX_DATA_BYTES: &str = "logs.max_data_bytes";
/// Configuration key of [`LogLimits::read_cache_bytes`].
pub const KEY_READ_CACHE_BYTES: &str = "logs.read_cache_bytes";
/// Configuration key of [`LogLimits::max_open_files`].
pub const KEY_MAX_OPEN_FILES: &str = "logs.max_open_files";

const MEBIBYTE: u64 = 1024 * 1024;

/// Default of [`LogLimits::max_limit`].
pub const DEFAULT_MAX_LIMIT: u32 = 100_000;
/// Default of [`LogLimits::max_message_bytes`].
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 65_536;
/// Default of [`LogLimits::max_field_bytes`].
pub const DEFAULT_MAX_FIELD_BYTES: usize = 16_384;
/// Default of [`LogLimits::max_fields`].
pub const DEFAULT_MAX_FIELDS: usize = 128;
/// Default of [`LogLimits::max_data_bytes`].
pub const DEFAULT_MAX_DATA_BYTES: u64 = 16 * MEBIBYTE;
/// Default of [`LogLimits::read_cache_bytes`].
pub const DEFAULT_READ_CACHE_BYTES: u64 = 32 * MEBIBYTE;
/// Default of [`LogLimits::max_open_files`].
pub const DEFAULT_MAX_OPEN_FILES: usize = 64;

/// Default number of events returned by `logs`.
pub const DEFAULT_LIMIT: u32 = 100;

/// Limits that bound the memory and the size of the records of one query.
///
/// ```
/// use nurt_syslog::limits::LogLimits;
///
/// let limits = LogLimits::default();
/// assert_eq!(limits.max_limit, 100_000);
/// assert!(limits.check_limit(100).is_ok());
/// assert!(limits.check_limit(0).is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLimits {
    /// Largest accepted value of the parameter `limit`.
    pub max_limit: u32,
    /// Largest message in bytes; a longer message is cut at a character
    /// boundary.
    pub max_message_bytes: usize,
    /// Largest value of one entry of `fields` in bytes.
    pub max_field_bytes: usize,
    /// Largest number of entries of `fields`; further entries are dropped.
    pub max_fields: usize,
    /// Largest decompressed size of one stored field; a larger field is not
    /// decompressed.
    pub max_data_bytes: u64,
    /// Memory used for cached blocks of log files during one query.
    pub read_cache_bytes: u64,
    /// Largest number of log files open at the same time.
    pub max_open_files: usize,
}

impl Default for LogLimits {
    fn default() -> Self {
        Self {
            max_limit: DEFAULT_MAX_LIMIT,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            max_field_bytes: DEFAULT_MAX_FIELD_BYTES,
            max_fields: DEFAULT_MAX_FIELDS,
            max_data_bytes: DEFAULT_MAX_DATA_BYTES,
            read_cache_bytes: DEFAULT_READ_CACHE_BYTES,
            max_open_files: DEFAULT_MAX_OPEN_FILES,
        }
    }
}

impl LogLimits {
    /// Checks the requested number of events and converts it.
    ///
    /// # Errors
    ///
    /// Returns `E_LIMIT` when `limit` is below 1 or above
    /// [`LogLimits::max_limit`].
    pub fn check_limit(&self, limit: i64) -> Result<u32, Error> {
        match u32::try_from(limit) {
            Ok(value) if (1..=self.max_limit).contains(&value) => Ok(value),
            _ => Err(Error::new(
                ErrorCode::Limit,
                format!(
                    "limit {limit} is outside 1..{}; use a smaller limit or page with the cursor of the last event",
                    self.max_limit
                ),
            )
            .with_detail("key", KEY_MAX_LIMIT.into())
            .with_detail("max", self.max_limit.into())),
        }
    }

    /// Cuts the message, the identifiers and the fields of an event to the
    /// limits and marks the event as truncated when anything was cut.
    pub fn apply(&self, event: &mut LogEvent) {
        let mut cut = truncate_text(&mut event.message, self.max_message_bytes);
        if event.fields.len() > self.max_fields {
            event.fields.truncate(self.max_fields);
            cut = true;
        }
        for (_, value) in &mut event.fields {
            cut |= truncate_text(value, self.max_field_bytes);
        }
        for value in [
            &mut event.id,
            &mut event.unit,
            &mut event.host,
            &mut event.user,
            &mut event.category,
        ]
        .into_iter()
        .flatten()
        {
            cut |= truncate_text(value, self.max_field_bytes);
        }
        cut |= truncate_text(&mut event.source, self.max_field_bytes);
        if cut {
            event.flags.truncated = true;
        }
    }
}

/// Cuts `text` to at most `max` bytes at a character boundary.
///
/// Returns whether anything was removed.
///
/// ```
/// use nurt_syslog::limits::truncate_text;
///
/// let mut text = String::from("a\u{20ac}b");
/// assert!(truncate_text(&mut text, 3));
/// assert_eq!(text, "a");
/// ```
pub fn truncate_text(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::Cursor;
    use crate::event::StoreKind;
    use nurt_contracts::value::Instant;

    fn event_with(message: &str, fields: usize, value: &str) -> LogEvent {
        let mut event = LogEvent::new(
            Instant::from_unix_nanos(0),
            "app",
            6,
            message,
            StoreKind::Journald,
            Cursor::EventLog {
                channel: "System".to_owned(),
                record_id: 1,
            },
        );
        for index in 0..fields {
            event.fields.push((format!("F{index}"), value.to_owned()));
        }
        event
    }

    #[test]
    fn accepts_limit_within_range() {
        let limits = LogLimits::default();
        assert_eq!(limits.check_limit(1).unwrap(), 1);
        assert_eq!(limits.check_limit(100_000).unwrap(), 100_000);
    }

    #[test]
    fn rejects_limit_outside_range_with_e_limit() {
        let limits = LogLimits::default();
        for value in [0, -1, 100_001, i64::MAX] {
            let error = limits.check_limit(value).unwrap_err();
            assert_eq!(error.code(), ErrorCode::Limit);
        }
    }

    #[test]
    fn truncates_at_character_boundary() {
        let mut text = String::from("\u{e9}\u{e9}\u{e9}");
        assert!(truncate_text(&mut text, 3));
        assert_eq!(text, "\u{e9}");
        let mut short = String::from("abc");
        assert!(!truncate_text(&mut short, 3));
        assert_eq!(short, "abc");
    }

    #[test]
    fn marks_event_truncated_when_message_is_cut() {
        let limits = LogLimits {
            max_message_bytes: 4,
            ..LogLimits::default()
        };
        let mut event = event_with("abcdefgh", 0, "");
        limits.apply(&mut event);
        assert_eq!(event.message, "abcd");
        assert!(event.flags.truncated);
    }

    #[test]
    fn drops_fields_above_count_and_cuts_values() {
        let limits = LogLimits {
            max_fields: 2,
            max_field_bytes: 3,
            ..LogLimits::default()
        };
        let mut event = event_with("m", 5, "value");
        limits.apply(&mut event);
        assert_eq!(event.fields.len(), 2);
        assert!(event.fields.iter().all(|(_, value)| value == "val"));
        assert!(event.flags.truncated);
    }

    #[test]
    fn cuts_identifiers_to_field_limit() {
        let limits = LogLimits {
            max_field_bytes: 2,
            ..LogLimits::default()
        };
        let mut event = event_with("m", 0, "");
        event.unit = Some("long.service".to_owned());
        limits.apply(&mut event);
        assert_eq!(event.unit.as_deref(), Some("lo"));
        assert_eq!(event.source, "ap");
        assert!(event.flags.truncated);
    }

    #[test]
    fn leaves_event_within_limits_unchanged() {
        let limits = LogLimits::default();
        let mut event = event_with("message", 3, "v");
        let before = event.clone();
        limits.apply(&mut event);
        assert_eq!(event, before);
    }
}
