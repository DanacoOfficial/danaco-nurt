// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Records of log archives parsed without the `OSLog` framework.
//!
//! On macOS a `.logarchive` directory can be parsed by the
//! `macos-unifiedlogs` library. Its records are mapped here to entries of
//! the reader. The library returns the records of a file in the order of
//! the file, so [`ArchiveBuffer`] keeps the entries that match a request and
//! returns them newest first; its memory grows with the number of matching
//! entries of the archive.
//!
//! | Field of the entry | Field of the record |
//! |---|---|
//! | `time` | `time`, nanoseconds since the Unix epoch |
//! | `level` | `log_type` of a record of type `Log` |
//! | `message` | `message`; `raw_message` and the flag `unformatted` when it is empty |
//! | `process` | file name of `process` |
//! | `sender` | file name of `library` |
//! | `user` | `euid` |
//! | `boot_id` | `boot_uuid` |

use std::cmp::Reverse;

use nurt_contracts::value::Instant;

use super::backend::{BackendError, BackendItem, EntryRequest};
use super::entry::{UnifiedEntry, digest, instant_from_unix_nanos_f64};
use super::level::EntryLevel;
use super::predicate::Predicate;

/// Number of hexadecimal digits of a boot identifier.
const BOOT_ID_DIGITS: usize = 32;

/// Kind of a record of a log archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// A log message of the given level.
    Log(EntryLevel),
    /// A record of messages lost by the system.
    Loss,
    /// An activity, a signpost, a state dump or another record.
    Other,
}

/// Record of a log archive, with the fields of the parser.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArchiveRecord {
    /// Kind of the record.
    pub kind: Option<ArchiveKind>,
    /// Time in nanoseconds since the Unix epoch.
    pub time: f64,
    /// Subsystem; empty when the message has none.
    pub subsystem: String,
    /// Category; empty when the message has none.
    pub category: String,
    /// Path of the main image of the process.
    pub process: String,
    /// Process identifier.
    pub pid: u64,
    /// Thread identifier.
    pub thread_id: u64,
    /// Effective user identifier of the process.
    pub euid: u32,
    /// Path of the image that logged the message.
    pub library: String,
    /// UUID of the image that logged the message.
    pub library_uuid: String,
    /// UUID of the main image of the process.
    pub process_uuid: String,
    /// UUID of the boot.
    pub boot_uuid: String,
    /// Identifier of the activity; 0 when the message has none.
    pub activity_id: u64,
    /// Message composed from the format string and its arguments.
    pub message: String,
    /// Text of the entry when the format string was not found.
    pub raw_message: String,
}

impl ArchiveRecord {
    /// Item of the log store for the record.
    ///
    /// A record without a kind or with a time outside the range of
    /// [`Instant`] is not a log message.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_syslog::unified::archive::{ArchiveKind, ArchiveRecord};
    /// use nurt_syslog::unified::backend::BackendItem;
    /// use nurt_syslog::unified::level::EntryLevel;
    /// let record = ArchiveRecord {
    ///     kind: Some(ArchiveKind::Log(EntryLevel::Error)),
    ///     time: 1.0e18,
    ///     process: "/usr/libexec/exampled".to_owned(),
    ///     message: "failed".to_owned(),
    ///     ..ArchiveRecord::default()
    /// };
    /// let BackendItem::Log(entry) = record.into_item() else { panic!() };
    /// assert_eq!(entry.process, "exampled");
    /// ```
    #[must_use]
    pub fn into_item(self) -> BackendItem {
        match self.kind {
            Some(ArchiveKind::Log(level)) => instant_from_unix_nanos_f64(self.time)
                .map_or(BackendItem::Other, |time| {
                    BackendItem::Log(Box::new(self.into_entry(time, level)))
                }),
            Some(ArchiveKind::Loss) => BackendItem::Lost(lost_count(&self.message)),
            Some(ArchiveKind::Other) | None => BackendItem::Other,
        }
    }

    fn into_entry(self, time: Instant, level: EntryLevel) -> UnifiedEntry {
        let unformatted = self.message.is_empty() && !self.raw_message.is_empty();
        let message = if unformatted {
            self.raw_message
        } else {
            self.message
        };
        UnifiedEntry {
            time,
            level,
            message,
            process: image_name(&self.process).to_owned(),
            pid: Some(self.pid),
            tid: Some(self.thread_id),
            subsystem: Some(self.subsystem).filter(|text| !text.is_empty()),
            category: Some(self.category).filter(|text| !text.is_empty()),
            sender: Some(image_name(&self.library).to_owned()).filter(|text| !text.is_empty()),
            activity_id: (self.activity_id != 0).then_some(self.activity_id),
            format_string: None,
            user: Some(self.euid.to_string()),
            boot_id: boot_id(&self.boot_uuid),
            process_uuid: Some(self.process_uuid).filter(|text| !text.is_empty()),
            sender_uuid: Some(self.library_uuid).filter(|text| !text.is_empty()),
            unformatted,
        }
    }
}

/// Number of lost messages of a loss record: the first decimal number of its
/// message, at least 1.
///
/// # Examples
///
/// ```
/// use nurt_syslog::unified::archive::lost_count;
/// assert_eq!(lost_count("lost 12 unreliable messages"), 12);
/// assert_eq!(lost_count("messages lost"), 1);
/// ```
#[must_use]
pub fn lost_count(message: &str) -> u64 {
    message
        .split(|character: char| !character.is_ascii_digit())
        .find(|digits| !digits.is_empty())
        .and_then(|digits| digits.parse::<u64>().ok())
        .map_or(1, |count| count.max(1))
}

/// File name of an image path; the path itself when it has no `/`.
///
/// # Examples
///
/// ```
/// use nurt_syslog::unified::archive::image_name;
/// assert_eq!(image_name("/usr/sbin/exampled"), "exampled");
/// assert_eq!(image_name("kernel"), "kernel");
/// ```
#[must_use]
pub fn image_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Boot identifier of a UUID: its 32 hexadecimal digits in lowercase, or
/// `None` when the text is not a UUID.
///
/// # Examples
///
/// ```
/// use nurt_syslog::unified::archive::boot_id;
/// assert_eq!(
///     boot_id("8E4C2A3B-1F2D-4C5E-9A8B-7C6D5E4F3A2B").as_deref(),
///     Some("8e4c2a3b1f2d4c5e9a8b7c6d5e4f3a2b"),
/// );
/// assert_eq!(boot_id(""), None);
/// ```
#[must_use]
pub fn boot_id(uuid: &str) -> Option<String> {
    let digits: String = uuid
        .chars()
        .filter(|character| *character != '-')
        .map(|character| character.to_ascii_lowercase())
        .collect();
    (digits.len() == BOOT_ID_DIGITS && digits.chars().all(|digit| digit.is_ascii_hexdigit()))
        .then_some(digits)
}

/// Entries of a log archive that match a request, collected in any order and
/// returned newest first.
#[derive(Debug, Clone, Default)]
pub struct ArchiveBuffer {
    predicate: Option<Predicate>,
    until: Option<Instant>,
    entries: Vec<UnifiedEntry>,
    lost: u64,
}

impl ArchiveBuffer {
    /// Empty buffer for a request.
    #[must_use]
    pub fn new(request: &EntryRequest) -> Self {
        Self {
            predicate: request.predicate.clone(),
            until: request.until,
            entries: Vec::new(),
            lost: 0,
        }
    }

    /// Adds a record. Log messages that match the request are kept, lost
    /// messages are counted and other records are dropped.
    pub fn push(&mut self, record: ArchiveRecord) {
        match record.into_item() {
            BackendItem::Log(entry) => {
                let before = self.until.is_none_or(|until| entry.time < until);
                let matches = self
                    .predicate
                    .as_ref()
                    .is_none_or(|predicate| predicate.matches(&entry));
                if before && matches {
                    self.entries.push(*entry);
                }
            }
            BackendItem::Lost(count) => self.lost = self.lost.saturating_add(count),
            BackendItem::Other => {}
        }
    }

    /// Items of the buffer: the count of lost messages, when there are any,
    /// then the entries newest first.
    ///
    /// Entries of the same time are ordered by process and thread identifier
    /// and digest of the message, so that a cursor finds its entry again.
    #[must_use]
    pub fn into_items(mut self) -> Vec<Result<BackendItem, BackendError>> {
        self.entries.sort_by_cached_key(|entry| {
            Reverse((entry.time, entry.pid, entry.tid, digest(&entry.message)))
        });
        let lost = (self.lost > 0).then_some(BackendItem::Lost(self.lost));
        lost.into_iter()
            .chain(
                self.entries
                    .into_iter()
                    .map(|entry| BackendItem::Log(Box::new(entry))),
            )
            .map(Ok)
            .collect()
    }
}
