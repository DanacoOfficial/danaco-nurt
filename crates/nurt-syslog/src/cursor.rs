// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Opaque positions in log stores.
//!
//! Every event carries a cursor that identifies its position in the store.
//! The text form starts with the version `v1:` and a letter of the store:
//!
//! | Store | Form |
//! |---|---|
//! | systemd journal | `v1:j:<file_id>:<seqnum_id>:<seqnum>:<realtime>` |
//! | Windows event log | `v1:w:<channel>:<record_id>` |
//! | macOS unified log | `v1:m:<nanos>:<pid>:<tid>:<digest>` |
//!
//! Identifiers are 32 lower-case hexadecimal digits, the digest 16; numbers
//! are decimal; `realtime` is in microseconds since the UNIX epoch. A query
//! with a cursor returns the events older than the event of the cursor.

use std::fmt;
use std::str::FromStr;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::value::Instant;

const VERSION: &str = "v1";
const NANOS_PER_MICRO: i64 = 1_000;
const ID_BYTES: usize = 16;
const ID_HEX_DIGITS: usize = 2 * ID_BYTES;
const DIGEST_HEX_DIGITS: usize = 16;

/// 128-bit identifier stored as 16 raw bytes.
///
/// ```
/// use nurt_syslog::cursor::Id128;
///
/// let id: Id128 = "000102030405060708090a0b0c0d0e0f".parse().unwrap();
/// assert_eq!(id.as_bytes()[15], 15);
/// assert_eq!(id.to_string(), "000102030405060708090a0b0c0d0e0f");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Id128([u8; ID_BYTES]);

impl Id128 {
    /// Identifier with the given bytes.
    #[must_use]
    pub const fn new(bytes: [u8; ID_BYTES]) -> Self {
        Self(bytes)
    }

    /// Raw bytes of the identifier.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; ID_BYTES] {
        &self.0
    }

    /// Whether all bytes are zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|byte| *byte == 0)
    }
}

impl fmt::Display for Id128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Id128 {
    type Err = CursorError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != ID_HEX_DIGITS || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CursorError::new(text));
        }
        let mut bytes = [0_u8; ID_BYTES];
        let (pairs, _) = text.as_bytes().as_chunks::<2>();
        for (slot, [high, low]) in bytes.iter_mut().zip(pairs) {
            *slot = (hex_value(*high) << 4) | hex_value(*low);
        }
        Ok(Self(bytes))
    }
}

/// Value of an ASCII hexadecimal digit; the caller has checked the digit.
const fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        b'A'..=b'F' => digit - b'A' + 10,
        _ => 0,
    }
}

/// Position of an event in a log store.
///
/// The ordering compares the store first and then the fields in their
/// order; it only serves to break ties between events of equal time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cursor {
    /// Entry of a systemd journal file.
    Journal {
        /// Identifier of the file that holds the entry.
        file_id: Id128,
        /// Identifier of the sequence number space.
        seqnum_id: Id128,
        /// Sequence number of the entry.
        seqnum: u64,
        /// Reception time in microseconds since the UNIX epoch.
        realtime: u64,
    },
    /// Record of a Windows event log channel.
    EventLog {
        /// Channel name.
        channel: String,
        /// Record number within the channel.
        record_id: u64,
    },
    /// Entry of the macOS unified log.
    Unified {
        /// Time in nanoseconds since the UNIX epoch.
        nanos: i64,
        /// Process identifier.
        pid: u64,
        /// Thread identifier.
        tid: u64,
        /// Digest of the message that separates entries of equal time.
        digest: u64,
    },
}

impl Cursor {
    /// Time of the event of the cursor, when the cursor records one.
    ///
    /// ```
    /// use nurt_syslog::cursor::{Cursor, Id128};
    ///
    /// let cursor = Cursor::Journal {
    ///     file_id: Id128::default(),
    ///     seqnum_id: Id128::default(),
    ///     seqnum: 7,
    ///     realtime: 1_500,
    /// };
    /// assert_eq!(cursor.time().unwrap().as_unix_nanos(), 1_500_000);
    /// ```
    #[must_use]
    pub fn time(&self) -> Option<Instant> {
        match self {
            Self::Journal { realtime, .. } => {
                let micros = i64::try_from(*realtime).ok()?;
                micros
                    .checked_mul(NANOS_PER_MICRO)
                    .map(Instant::from_unix_nanos)
            }
            Self::Unified { nanos, .. } => Some(Instant::from_unix_nanos(*nanos)),
            Self::EventLog { .. } => None,
        }
    }

    /// Whether a journal entry lies before the position of this cursor.
    ///
    /// Entries of the same sequence number space are ordered by sequence
    /// number. Other entries are ordered by reception time, which is an
    /// approximation; the result tells which rule was used. A cursor of
    /// another store admits nothing.
    ///
    /// ```
    /// use nurt_syslog::cursor::{Cursor, Id128, Resume};
    ///
    /// let space = Id128::new([1; 16]);
    /// let cursor = Cursor::Journal { file_id: space, seqnum_id: space, seqnum: 10, realtime: 500 };
    /// assert_eq!(cursor.admits_journal(&space, 9, 900), Some(Resume::Exact));
    /// assert_eq!(cursor.admits_journal(&space, 10, 100), None);
    /// assert_eq!(cursor.admits_journal(&Id128::default(), 1, 499), Some(Resume::Approximate));
    /// ```
    #[must_use]
    pub fn admits_journal(&self, seqnum_id: &Id128, seqnum: u64, realtime: u64) -> Option<Resume> {
        let Self::Journal {
            seqnum_id: cursor_space,
            seqnum: cursor_seqnum,
            realtime: cursor_realtime,
            ..
        } = self
        else {
            return None;
        };
        if cursor_space == seqnum_id {
            (seqnum < *cursor_seqnum).then_some(Resume::Exact)
        } else {
            (realtime < *cursor_realtime).then_some(Resume::Approximate)
        }
    }

    /// Parses the value of a cursor parameter.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALID_VALUE` when `text` is not a cursor of version 1.
    pub fn parse_param(text: &str) -> Result<Self, Error> {
        text.parse().map_err(|error: CursorError| {
            Error::new(ErrorCode::InvalidValue, error.to_string())
                .with_found(text)
                .with_problem("pass the cursor of an event returned by `logs` unchanged")
        })
    }
}

/// Rule by which a position was compared with a cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Same sequence space or record numbering: the position is exact.
    Exact,
    /// Different sequence space: the comparison used the time only.
    Approximate,
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal {
                file_id,
                seqnum_id,
                seqnum,
                realtime,
            } => write!(f, "{VERSION}:j:{file_id}:{seqnum_id}:{seqnum}:{realtime}"),
            Self::EventLog { channel, record_id } => {
                write!(f, "{VERSION}:w:{channel}:{record_id}")
            }
            Self::Unified {
                nanos,
                pid,
                tid,
                digest,
            } => write!(f, "{VERSION}:m:{nanos}:{pid}:{tid}:{digest:016x}"),
        }
    }
}

impl FromStr for Cursor {
    type Err = CursorError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || CursorError::new(text);
        let rest = text
            .strip_prefix(VERSION)
            .and_then(|rest| rest.strip_prefix(':'))
            .ok_or_else(invalid)?;
        let (store, body) = rest.split_once(':').ok_or_else(invalid)?;
        match store {
            "j" => parse_journal(body).ok_or_else(invalid),
            "w" => parse_event_log(body).ok_or_else(invalid),
            "m" => parse_unified(body).ok_or_else(invalid),
            _ => Err(invalid()),
        }
    }
}

fn parse_journal(body: &str) -> Option<Cursor> {
    let mut parts = body.split(':');
    let file_id = parts.next()?.parse().ok()?;
    let seqnum_id = parts.next()?.parse().ok()?;
    let seqnum = parse_decimal(parts.next()?)?;
    let realtime = parse_decimal(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(Cursor::Journal {
        file_id,
        seqnum_id,
        seqnum,
        realtime,
    })
}

fn parse_event_log(body: &str) -> Option<Cursor> {
    let (channel, record) = body.rsplit_once(':')?;
    if channel.is_empty() {
        return None;
    }
    Some(Cursor::EventLog {
        channel: channel.to_owned(),
        record_id: parse_decimal(record)?,
    })
}

fn parse_unified(body: &str) -> Option<Cursor> {
    let mut parts = body.split(':');
    let nanos_text = parts.next()?;
    // The time is written with a minus sign when negative and never with a
    // plus sign, as the other numbers of a cursor have digits only.
    if nanos_text.starts_with('+') {
        return None;
    }
    let nanos = nanos_text.parse::<i64>().ok()?;
    let pid = parse_decimal(parts.next()?)?;
    let tid = parse_decimal(parts.next()?)?;
    let digest_text = parts.next()?;
    if parts.next().is_some()
        || digest_text.len() != DIGEST_HEX_DIGITS
        || !digest_text.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let digest = u64::from_str_radix(digest_text, 16).ok()?;
    Some(Cursor::Unified {
        nanos,
        pid,
        tid,
        digest,
    })
}

fn parse_decimal(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Text that is not a cursor of version 1.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` is not a log cursor; expected the form `v1:<store>:…`")]
pub struct CursorError {
    text: String,
}

impl CursorError {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal_cursor() -> Cursor {
        Cursor::Journal {
            file_id: Id128::new([0xab; 16]),
            seqnum_id: Id128::new([0x01; 16]),
            seqnum: 42,
            realtime: 1_700_000_000_000_000,
        }
    }

    #[test]
    fn journal_cursor_round_trips_through_text() {
        let cursor = journal_cursor();
        let text = cursor.to_string();
        assert_eq!(
            text,
            "v1:j:abababababababababababababababab:01010101010101010101010101010101:42:1700000000000000"
        );
        assert_eq!(text.parse::<Cursor>().unwrap(), cursor);
    }

    #[test]
    fn event_log_cursor_keeps_channel_with_separators() {
        let cursor = Cursor::EventLog {
            channel: "Microsoft-Windows-Test/Operational".to_owned(),
            record_id: 7,
        };
        let text = cursor.to_string();
        assert_eq!(text, "v1:w:Microsoft-Windows-Test/Operational:7");
        assert_eq!(text.parse::<Cursor>().unwrap(), cursor);
        let colon = "v1:w:a:b:9".parse::<Cursor>().unwrap();
        assert_eq!(
            colon,
            Cursor::EventLog {
                channel: "a:b".to_owned(),
                record_id: 9
            }
        );
    }

    #[test]
    fn unified_cursor_round_trips_with_negative_time() {
        let cursor = Cursor::Unified {
            nanos: -5,
            pid: 1,
            tid: 2,
            digest: 0xdead_beef,
        };
        let text = cursor.to_string();
        assert_eq!(text, "v1:m:-5:1:2:00000000deadbeef");
        assert_eq!(text.parse::<Cursor>().unwrap(), cursor);
        assert_eq!(cursor.time(), Some(Instant::from_unix_nanos(-5)));
    }

    #[test]
    fn unified_cursor_with_a_plus_sign_in_the_time_is_rejected() {
        assert!("v1:m:+1:1:2:00000000deadbeef".parse::<Cursor>().is_err());
        assert!("v1:m:1:1:2:00000000deadbeef".parse::<Cursor>().is_ok());
        assert!("v1:m:-1:1:2:00000000deadbeef".parse::<Cursor>().is_ok());
    }

    #[test]
    fn rejects_malformed_cursors() {
        for text in [
            "",
            "v2:j:x",
            "v1",
            "v1:x:1",
            "v1:j:00:00:1:1",
            "v1:j:abababababababababababababababab:01010101010101010101010101010101:42",
            "v1:j:abababababababababababababababab:01010101010101010101010101010101:42:1:9",
            "v1:j:abababababababababababababababab:01010101010101010101010101010101:-1:1",
            "v1:j:zzababababababababababababababab:01010101010101010101010101010101:1:1",
            "v1:w::5",
            "v1:w:System:",
            "v1:w:System",
            "v1:m:1:2:3",
            "v1:m:1:2:3:xyz",
            "v1:m:x:2:3:00000000deadbeef",
            "v1:m:1:2:3:00000000deadbeef:1",
        ] {
            assert!(text.parse::<Cursor>().is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn parse_param_reports_invalid_value() {
        let error = Cursor::parse_param("nonsense").unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert!(error.to_string().contains("not a log cursor"));
    }

    #[test]
    fn journal_admission_uses_sequence_numbers_in_same_space() {
        let cursor = journal_cursor();
        let space = Id128::new([0x01; 16]);
        assert_eq!(
            cursor.admits_journal(&space, 41, u64::MAX),
            Some(Resume::Exact)
        );
        assert_eq!(cursor.admits_journal(&space, 42, 0), None);
        assert_eq!(cursor.admits_journal(&space, 43, 0), None);
    }

    #[test]
    fn journal_admission_falls_back_to_time_in_other_space() {
        let cursor = journal_cursor();
        let other = Id128::new([0x02; 16]);
        assert_eq!(
            cursor.admits_journal(&other, 1, 1_699_999_999_999_999),
            Some(Resume::Approximate)
        );
        assert_eq!(
            cursor.admits_journal(&other, 1, 1_700_000_000_000_000),
            None
        );
    }

    #[test]
    fn foreign_cursor_admits_no_journal_entry() {
        let cursor = Cursor::EventLog {
            channel: "System".to_owned(),
            record_id: 1,
        };
        assert_eq!(cursor.admits_journal(&Id128::default(), 0, 0), None);
        assert_eq!(cursor.time(), None);
    }

    #[test]
    fn journal_time_overflow_gives_no_time() {
        let cursor = Cursor::Journal {
            file_id: Id128::default(),
            seqnum_id: Id128::default(),
            seqnum: 1,
            realtime: u64::MAX,
        };
        assert_eq!(cursor.time(), None);
    }

    #[test]
    fn id_rejects_wrong_length_and_digits() {
        assert!("abc".parse::<Id128>().is_err());
        assert!("g0000000000000000000000000000000".parse::<Id128>().is_err());
        assert!(Id128::default().is_zero());
        assert!(!Id128::new([1; 16]).is_zero());
    }

    #[test]
    fn id_accepts_upper_case_digits_and_prints_lower_case() {
        let id: Id128 = "ABCDEF00112233445566778899aabbcc".parse().unwrap();
        assert_eq!(id.to_string(), "abcdef00112233445566778899aabbcc");
        assert!(
            "\u{e9}0000000000000000000000000000000"
                .parse::<Id128>()
                .is_err()
        );
        assert!(" 0000000000000000000000000000000".parse::<Id128>().is_err());
    }

    #[test]
    fn journal_cursor_keeps_the_largest_numbers() {
        let text = format!("v1:j:{0}:{0}:{1}:{1}", "00".repeat(16), u64::MAX);
        let cursor = text.parse::<Cursor>().unwrap();
        assert_eq!(cursor.to_string(), text);
        assert_eq!(cursor.time(), None);
        let over = format!("v1:j:{0}:{0}:{1}0:1", "00".repeat(16), u64::MAX);
        assert!(over.parse::<Cursor>().is_err());
    }

    #[test]
    fn unified_cursor_keeps_the_extreme_times() {
        for nanos in [i64::MIN, i64::MAX] {
            let text = format!("v1:m:{nanos}:0:0:0000000000000000");
            let cursor = text.parse::<Cursor>().unwrap();
            assert_eq!(cursor.to_string(), text);
            assert_eq!(cursor.time(), Some(Instant::from_unix_nanos(nanos)));
        }
        assert!(
            "v1:m:9223372036854775808:0:0:0000000000000000"
                .parse::<Cursor>()
                .is_err()
        );
    }

    #[test]
    fn event_log_cursor_rejects_signs_and_spaces_in_the_record_number() {
        for text in [
            "v1:w:System:+1",
            "v1:w:System:-1",
            "v1:w:System: 1",
            "v1:w:System:1 ",
        ] {
            assert!(text.parse::<Cursor>().is_err(), "accepted {text:?}");
        }
        let channel = "v1:w:Microsoft-Windows-X/Operational:18446744073709551615";
        assert!(channel.parse::<Cursor>().is_ok());
        assert!(
            "v1:w:System:18446744073709551616"
                .parse::<Cursor>()
                .is_err()
        );
    }

    #[test]
    fn parse_param_keeps_the_text_it_was_given() {
        let error = Cursor::parse_param("v1:j:zz").unwrap_err();
        assert_eq!(error.found.as_deref(), Some("v1:j:zz"));
    }

    #[test]
    fn journal_entries_of_another_store_cursor_are_not_admitted() {
        let cursor = Cursor::Unified {
            nanos: 1,
            pid: 1,
            tid: 1,
            digest: 1,
        };
        assert_eq!(cursor.admits_journal(&Id128::default(), 0, 0), None);
    }
}
