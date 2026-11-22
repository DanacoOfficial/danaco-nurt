// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Entries of the unified log and their mapping to log events.
//!
//! | Field | Source |
//! |---|---|
//! | `time` | `date` |
//! | `source` | `process` |
//! | `priority` | `level`, see [`EntryLevel::priority`] |
//! | `message` | `composedMessage` |
//! | `unit` | `subsystem`; an empty subsystem gives no unit |
//! | `category` | `category` |
//! | `pid`, `tid` | `processIdentifier`, `threadIdentifier` |
//! | `user` | effective user identifier, from log archives only |
//! | `boot_id` | boot identifier, from log archives only |
//! | `fields` | `sender`, `activityIdentifier`, `formatString`, `processUUID`, `senderUUID` |
//! | `cursor` | `v1:m:<nanos>:<pid>:<tid>:<digest of the message>` |

use nurt_contracts::value::Instant;

use crate::cursor::Cursor;
use crate::event::{LogEvent, StoreKind};

use super::level::EntryLevel;

const NANOS_PER_MICRO: i64 = 1_000;
const MICROS_PER_SECOND: f64 = 1_000_000.0;
const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// Offset basis of the 64-bit FNV-1a hash.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// Prime of the 64-bit FNV-1a hash.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Log message of the unified log.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UnifiedEntry {
    /// Time of the entry.
    pub time: Instant,
    /// Level of the entry.
    pub level: EntryLevel,
    /// Message composed from the format string and its arguments.
    pub message: String,
    /// Name of the process.
    pub process: String,
    /// Process identifier.
    pub pid: Option<u64>,
    /// Thread identifier.
    pub tid: Option<u64>,
    /// Subsystem, in reverse DNS notation.
    pub subsystem: Option<String>,
    /// Category within the subsystem.
    pub category: Option<String>,
    /// Name of the image that logged the message.
    pub sender: Option<String>,
    /// Identifier of the activity of the message.
    pub activity_id: Option<u64>,
    /// Format string of the message.
    pub format_string: Option<String>,
    /// Effective user identifier of the process, in decimal.
    pub user: Option<String>,
    /// Boot identifier as 32 lowercase hexadecimal digits.
    pub boot_id: Option<String>,
    /// UUID of the main image of the process.
    pub process_uuid: Option<String>,
    /// UUID of the image that logged the message.
    pub sender_uuid: Option<String>,
    /// The format string of the message was not found and the message holds
    /// the raw text of the entry.
    pub unformatted: bool,
}

impl UnifiedEntry {
    /// Cursor of the entry.
    ///
    /// The log store has no identifier of an entry. The cursor combines the
    /// time, the process and thread identifiers and a digest of the message;
    /// a missing identifier is written as 0.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        Cursor::Unified {
            nanos: self.time.as_unix_nanos(),
            pid: self.pid.unwrap_or_default(),
            tid: self.tid.unwrap_or_default(),
            digest: digest(&self.message),
        }
    }

    /// Maps the entry to a log event.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::record::LogLevel;
    /// use nurt_syslog::unified::entry::UnifiedEntry;
    /// use nurt_syslog::unified::level::EntryLevel;
    /// let entry = UnifiedEntry {
    ///     level: EntryLevel::Error,
    ///     message: "connection refused".to_owned(),
    ///     process: "agent".to_owned(),
    ///     subsystem: Some("com.example.agent".to_owned()),
    ///     ..UnifiedEntry::default()
    /// };
    /// let event = entry.to_event();
    /// assert_eq!(event.level(), LogLevel::Error);
    /// assert_eq!(event.unit.as_deref(), Some("com.example.agent"));
    /// ```
    #[must_use]
    pub fn to_event(&self) -> LogEvent {
        let mut event = LogEvent::new(
            self.time,
            &self.process,
            self.level.priority(),
            &self.message,
            StoreKind::UnifiedLog,
            self.cursor(),
        );
        event.unit = non_empty(self.subsystem.as_deref());
        event.category = non_empty(self.category.as_deref());
        event.pid = self.pid;
        event.tid = self.tid;
        event.user.clone_from(&self.user);
        event.boot_id.clone_from(&self.boot_id);
        let extra = [
            ("sender", non_empty(self.sender.as_deref())),
            (
                "activityIdentifier",
                self.activity_id.map(|id| id.to_string()),
            ),
            ("formatString", self.format_string.clone()),
            ("processUUID", non_empty(self.process_uuid.as_deref())),
            ("senderUUID", non_empty(self.sender_uuid.as_deref())),
        ];
        event.fields = extra
            .into_iter()
            .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value)))
            .collect();
        event.flags.unformatted = self.unformatted;
        event
    }

    /// Whether the entry is the entry of a cursor.
    #[must_use]
    pub fn is_at(&self, cursor: &Cursor) -> bool {
        self.cursor() == *cursor
    }
}

fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|text| !text.is_empty()).map(str::to_owned)
}

/// Digest of a message: the 64-bit FNV-1a hash of its UTF-8 bytes.
///
/// # Examples
///
/// ```
/// use nurt_syslog::unified::entry::digest;
/// assert_eq!(digest(""), 0xcbf2_9ce4_8422_2325);
/// assert_eq!(digest("a"), 0xaf63_dc4c_8601_ec8c);
/// ```
#[must_use]
pub fn digest(message: &str) -> u64 {
    message.bytes().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

/// Instant of a time in seconds since the Unix epoch, as `NSDate` gives it,
/// rounded to microseconds.
///
/// The result is `None` for a value that is not finite or outside the range
/// of [`Instant`]. A double has about 16 significant digits, so finer parts
/// of a current time are not significant.
///
/// # Examples
///
/// ```
/// use nurt_syslog::unified::entry::instant_from_unix_seconds;
/// let instant = instant_from_unix_seconds(1_700_000_000.25).unwrap();
/// assert_eq!(instant.as_unix_nanos(), 1_700_000_000_250_000_000);
/// assert_eq!(instant_from_unix_seconds(f64::NAN), None);
/// ```
#[must_use]
pub fn instant_from_unix_seconds(seconds: f64) -> Option<Instant> {
    let micros = (seconds * MICROS_PER_SECOND).round();
    let in_range = micros.is_finite() && micros >= i64::MIN as f64 && micros < i64::MAX as f64;
    in_range
        .then_some(micros as i64)?
        .checked_mul(NANOS_PER_MICRO)
        .map(Instant::from_unix_nanos)
}

/// Instant of a time in nanoseconds since the Unix epoch held in a double,
/// rounded to whole nanoseconds.
///
/// The result is `None` for a value that is not finite or outside the range
/// of [`Instant`].
///
/// # Examples
///
/// ```
/// use nurt_syslog::unified::entry::instant_from_unix_nanos_f64;
/// let instant = instant_from_unix_nanos_f64(1_500.4).unwrap();
/// assert_eq!(instant.as_unix_nanos(), 1_500);
/// assert_eq!(instant_from_unix_nanos_f64(f64::INFINITY), None);
/// ```
#[must_use]
pub fn instant_from_unix_nanos_f64(nanos: f64) -> Option<Instant> {
    let nanos = nanos.round();
    (nanos.is_finite() && nanos >= i64::MIN as f64 && nanos < i64::MAX as f64)
        .then(|| Instant::from_unix_nanos(nanos as i64))
}

/// Seconds since the Unix epoch of an instant, as `NSDate` takes them.
///
/// # Examples
///
/// ```
/// use nurt_contracts::value::Instant;
/// use nurt_syslog::unified::entry::unix_seconds;
/// assert_eq!(unix_seconds(Instant::from_unix_nanos(1_500_000_000)), 1.5);
/// ```
#[must_use]
pub fn unix_seconds(instant: Instant) -> f64 {
    let nanos = instant.as_unix_nanos();
    let whole = nanos.div_euclid(NANOS_PER_SECOND);
    let fraction = nanos.rem_euclid(NANOS_PER_SECOND);
    whole as f64 + fraction as f64 / NANOS_PER_SECOND as f64
}
