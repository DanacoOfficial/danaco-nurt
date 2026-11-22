// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reflog entry format.

use bstr::{BStr, BString};
use gix_hash::oid;

const SECONDS_PER_HOUR: i64 = 3600;
const SECONDS_PER_MINUTE: i64 = 60;

/// Message as Git stores it in a reflog: leading whitespace removed, every run
/// of whitespace (including line breaks) replaced by one space, trailing
/// whitespace removed.
///
/// # Examples
///
/// ```
/// use nurt_git::refs_tx::normalize_reflog_message;
///
/// assert_eq!(normalize_reflog_message("  merge  topic:\n Fast-forward \n".into()), "merge topic: Fast-forward");
/// ```
#[must_use]
pub fn normalize_reflog_message(message: &BStr) -> BString {
    let mut out = BString::default();
    let mut pending_space = false;
    for &byte in message.iter() {
        if byte.is_ascii_whitespace() || byte == 0x0b {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(b' ');
            pending_space = false;
        }
        out.push(byte);
    }
    out
}

/// One reflog line, terminated by a line feed.
///
/// The message is normalized with [`normalize_reflog_message`]; an empty
/// message is written without the separating tab, as Git does.
///
/// # Examples
///
/// ```
/// use gix_hash::ObjectId;
/// use nurt_git::refs_tx::format_reflog_entry;
///
/// let null = ObjectId::null(gix_hash::Kind::Sha1);
/// let id = ObjectId::from_hex(b"0123456789012345678901234567890123456789").unwrap();
/// let signature = gix_actor::Signature {
///     name: "A U Thor".into(),
///     email: "author@example.com".into(),
///     time: gix_date::Time { seconds: 1_700_000_000, offset: -5400 },
/// };
/// let line = format_reflog_entry(&null, &id, &signature, "commit (initial): first".into());
/// assert_eq!(
///     line,
///     "0000000000000000000000000000000000000000 0123456789012345678901234567890123456789 \
///      A U Thor <author@example.com> 1700000000 -0130\tcommit (initial): first\n"
/// );
/// ```
#[must_use]
pub fn format_reflog_entry(
    old: &oid,
    new: &oid,
    committer: &gix_actor::Signature,
    message: &BStr,
) -> BString {
    let mut line = BString::from(format!("{} {} ", old.to_hex(), new.to_hex()));
    line.extend_from_slice(&committer.name);
    line.extend_from_slice(b" <");
    line.extend_from_slice(&committer.email);
    line.extend_from_slice(b"> ");
    line.extend_from_slice(format_time(&committer.time).as_bytes());
    let message = normalize_reflog_message(message);
    if !message.is_empty() {
        line.push(b'\t');
        line.extend_from_slice(&message);
    }
    line.push(b'\n');
    line
}

/// `<seconds> <+hhmm>` as written in identities.
fn format_time(time: &gix_date::Time) -> String {
    let offset = i64::from(time.offset);
    let sign = if offset < 0 { '-' } else { '+' };
    let offset = offset.abs();
    let hours = offset / SECONDS_PER_HOUR;
    let minutes = (offset % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    format!("{} {sign}{hours:02}{minutes:02}", time.seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_message_has_no_tab() {
        let id = gix_hash::Kind::Sha1.null();
        let signature = gix_actor::Signature {
            name: "N".into(),
            email: "e@x".into(),
            time: gix_date::Time {
                seconds: 5,
                offset: 3600,
            },
        };
        let line = format_reflog_entry(&id, &id, &signature, " \n ".into());
        assert!(line.ends_with(b"N <e@x> 5 +0100\n"));
        assert!(!line.contains(&b'\t'));
    }

    #[test]
    fn whitespace_runs_collapse_to_one_space() {
        assert_eq!(normalize_reflog_message("a\t\tb\r\nc".into()), "a b c");
        assert_eq!(normalize_reflog_message("   ".into()), "");
    }
}
