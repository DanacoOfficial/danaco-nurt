// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text form of macOS ACLs stored in `ExtendedMeta::acl`.
//!
//! The stored bytes are the output of `acl_to_text` without the terminating
//! NUL: a header line `!#acl 1` followed by one line per entry,
//! `tag:uuid:name:id:allow|deny:permissions`, where `tag` is `user` or
//! `group`. `acl_from_text` accepts the same form. An ACL without entries is
//! stored as absent, because a file without an extended ACL and a file with
//! an empty one are indistinguishable for access checks.

use std::ffi::CString;
use std::fmt;

/// First line of the text form.
pub const ACL_TEXT_HEADER: &str = "!#acl 1";

/// Minimum number of `:`-separated fields of an entry line.
const MIN_ENTRY_FIELDS: usize = 5;
/// Index of the first field that may hold `allow` or `deny`.
const FIRST_DECISION_FIELD: usize = 3;

/// Error of [`acl_text_for_system`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AclTextError {
    /// The bytes are not valid UTF-8.
    NotUtf8,
    /// The bytes contain a NUL byte.
    InteriorNul,
    /// The first line is not [`ACL_TEXT_HEADER`].
    MissingHeader,
    /// An entry line does not have the form
    /// `tag:uuid:name:id:allow|deny:permissions`; the value is the 1-based
    /// line number.
    MalformedEntry(usize),
}

impl fmt::Display for AclTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 => f.write_str("acl text is not valid utf-8"),
            Self::InteriorNul => f.write_str("acl text contains a nul byte"),
            Self::MissingHeader => write!(f, "acl text must start with `{ACL_TEXT_HEADER}`"),
            Self::MalformedEntry(line) => write!(
                f,
                "acl entry on line {line} must have the form `user|group:uuid:name:id:allow|deny:permissions`"
            ),
        }
    }
}

impl std::error::Error for AclTextError {}

/// Stored form of the text returned by `acl_to_text`; `None` when the ACL has
/// no entries.
///
/// Trailing NUL bytes are removed; the text is kept otherwise unchanged.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::acl_from_system_text;
///
/// assert_eq!(acl_from_system_text(b"!#acl 1\n\0"), None);
/// let text = b"!#acl 1\ngroup:ABCDEFAB-CDEF-ABCD-EFAB-CDEF0000000C:everyone:12:deny:delete\n";
/// assert_eq!(acl_from_system_text(text).as_deref(), Some(&text[..]));
/// ```
#[must_use]
pub fn acl_from_system_text(text: &[u8]) -> Option<Vec<u8>> {
    let end = text
        .iter()
        .rposition(|&byte| byte != 0)
        .map_or(0, |last| last + 1);
    let text = text.get(..end)?;
    let has_entries = text
        .split(|&byte| byte == b'\n')
        .skip(1)
        .any(|line| !line.is_empty());
    has_entries.then(|| text.to_vec())
}

/// Validates stored ACL bytes and returns the NUL-terminated text accepted by
/// `acl_from_text`.
///
/// # Errors
///
/// Returns [`AclTextError`] when the bytes are not UTF-8, contain a NUL byte,
/// lack the header line or contain a malformed entry line.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{AclTextError, acl_text_for_system};
///
/// let text = b"!#acl 1\nuser:FFFFEEEE-DDDD-CCCC-BBBB-AAAA000001F5:user:501:allow:read,write\n";
/// assert!(acl_text_for_system(text).is_ok());
/// assert_eq!(acl_text_for_system(b"user::"), Err(AclTextError::MissingHeader));
/// ```
pub fn acl_text_for_system(stored: &[u8]) -> Result<CString, AclTextError> {
    let text = std::str::from_utf8(stored).map_err(|_| AclTextError::NotUtf8)?;
    if stored.contains(&0) {
        return Err(AclTextError::InteriorNul);
    }
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some(ACL_TEXT_HEADER) {
        return Err(AclTextError::MissingHeader);
    }
    for (index, line) in lines.enumerate() {
        if !line.is_empty() && !is_entry(line) {
            return Err(AclTextError::MalformedEntry(index + 2));
        }
    }
    CString::new(stored).map_err(|_| AclTextError::InteriorNul)
}

fn is_entry(line: &str) -> bool {
    let fields: Vec<&str> = line.split(':').collect();
    let tagged = matches!(fields.first(), Some(&("user" | "group")));
    let decided = fields
        .iter()
        .skip(FIRST_DECISION_FIELD)
        .any(|field| matches!(*field, "allow" | "deny"));
    tagged && decided && fields.len() >= MIN_ENTRY_FIELDS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Output of `acl_to_text` for a file with a deny entry for the
    /// `everyone` group (gid 12, fixed UUID) and an allow entry with
    /// inheritance flags for a user.
    const GOLDEN: &[u8] = b"!#acl 1\n\
group:ABCDEFAB-CDEF-ABCD-EFAB-CDEF0000000C:everyone:12:deny:delete\n\
user:FFFFEEEE-DDDD-CCCC-BBBB-AAAA000001F5:user:501:allow:read,write,file_inherit,directory_inherit\n";

    #[test]
    fn stores_system_text_without_terminating_nul() {
        let mut system = GOLDEN.to_vec();
        system.push(0);
        assert_eq!(acl_from_system_text(&system).as_deref(), Some(GOLDEN));
    }

    #[test]
    fn stores_acl_without_entries_as_absent() {
        assert_eq!(acl_from_system_text(b"!#acl 1\n"), None);
        assert_eq!(acl_from_system_text(b"!#acl 1"), None);
        assert_eq!(acl_from_system_text(b""), None);
        assert_eq!(acl_from_system_text(b"\0\0"), None);
    }

    #[test]
    fn decodes_golden_text_into_nul_terminated_text() {
        let text = acl_text_for_system(GOLDEN).unwrap();
        assert_eq!(text.as_bytes(), GOLDEN);
        assert_eq!(text.as_bytes_with_nul().last(), Some(&0));
    }

    #[test]
    fn round_trips_golden_text_through_storage() {
        let stored = acl_from_system_text(GOLDEN).unwrap();
        let system = acl_text_for_system(&stored).unwrap();
        assert_eq!(
            acl_from_system_text(system.as_bytes_with_nul()),
            Some(stored)
        );
    }

    #[test]
    fn accepts_entry_without_resolvable_name() {
        let text = b"!#acl 1\nuser:FFFFEEEE-DDDD-CCCC-BBBB-AAAA00000BB8::3000:allow:read\n";
        assert!(acl_text_for_system(text).is_ok());
    }

    #[test]
    fn rejects_text_without_header() {
        assert_eq!(
            acl_text_for_system(b"group:X:everyone:12:deny:delete\n"),
            Err(AclTextError::MissingHeader)
        );
    }

    #[test]
    fn rejects_malformed_entry_with_its_line_number() {
        let text = b"!#acl 1\ngroup:A:everyone:12:deny:delete\nother:A:b:1:allow:read\n";
        assert_eq!(
            acl_text_for_system(text),
            Err(AclTextError::MalformedEntry(3))
        );
        let undecided = b"!#acl 1\nuser:A:b:1:maybe:read\n";
        assert_eq!(
            acl_text_for_system(undecided),
            Err(AclTextError::MalformedEntry(2))
        );
        let short = b"!#acl 1\nuser:allow\n";
        assert_eq!(
            acl_text_for_system(short),
            Err(AclTextError::MalformedEntry(2))
        );
    }

    #[test]
    fn rejects_invalid_utf8_and_nul_bytes() {
        assert_eq!(
            acl_text_for_system(b"!#acl 1\n\xff"),
            Err(AclTextError::NotUtf8)
        );
        assert_eq!(
            acl_text_for_system(b"!#acl 1\n\0"),
            Err(AclTextError::InteriorNul)
        );
    }

    #[test]
    fn describes_errors_in_lower_case_without_period() {
        let errors = [
            AclTextError::NotUtf8,
            AclTextError::InteriorNul,
            AclTextError::MissingHeader,
            AclTextError::MalformedEntry(2),
        ];
        for error in errors {
            let text = error.to_string();
            assert!(text.starts_with("acl "), "{text}");
            assert!(!text.ends_with('.'), "{text}");
        }
    }
}
