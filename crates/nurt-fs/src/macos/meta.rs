// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::macos::meta
// Responsibility:   Entry kinds and BSD file flags.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Entry kinds and BSD file flags.

use nurt_contracts::fs::FileKind;

/// `S_IFMT`: mask of the entry type in `st_mode`.
const S_IFMT: u32 = 0o170_000;
/// `S_IFREG`.
const S_IFREG: u32 = 0o100_000;
/// `S_IFDIR`.
const S_IFDIR: u32 = 0o040_000;
/// `S_IFLNK`.
const S_IFLNK: u32 = 0o120_000;
/// Permission bits including set-user-id, set-group-id and sticky bits.
pub const MODE_PERMISSION_BITS: u32 = 0o7777;

/// `UF_IMMUTABLE`.
pub const UF_IMMUTABLE: u32 = 0x0000_0002;
/// `UF_APPEND`.
pub const UF_APPEND: u32 = 0x0000_0004;
/// `UF_COMPRESSED`: the content is stored compressed (`decmpfs`).
pub const UF_COMPRESSED: u32 = 0x0000_0020;
/// `UF_HIDDEN`: hidden from graphical file browsers.
pub const UF_HIDDEN: u32 = 0x0000_8000;
/// `UF_SETTABLE`: flags the account of the file may change.
pub const UF_SETTABLE: u32 = 0x0000_ffff;
/// `SF_IMMUTABLE`.
pub const SF_IMMUTABLE: u32 = 0x0002_0000;
/// `SF_APPEND`.
pub const SF_APPEND: u32 = 0x0004_0000;
/// `SF_SETTABLE`: flags only the superuser may change.
pub const SF_SETTABLE: u32 = 0x3fff_0000;
/// Flags that refuse changes of content or metadata while set.
pub const WRITE_BLOCKING_FLAGS: u32 = UF_IMMUTABLE | UF_APPEND | SF_IMMUTABLE | SF_APPEND;

/// Flags restored from stored metadata. `UF_COMPRESSED` describes how the
/// file system stores the current content and is never copied.
const RESTORED_FLAGS: u32 = (UF_SETTABLE | SF_SETTABLE) & !UF_COMPRESSED;

/// Kind of an entry from `st_mode`.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::FileKind;
/// use nurt_fs::macos::file_kind;
///
/// assert_eq!(file_kind(0o100_644), FileKind::File);
/// assert_eq!(file_kind(0o120_755), FileKind::Symlink);
/// ```
#[must_use]
pub fn file_kind(mode: u32) -> FileKind {
    match mode & S_IFMT {
        S_IFREG => FileKind::File,
        S_IFDIR => FileKind::Directory,
        S_IFLNK => FileKind::Symlink,
        _ => FileKind::Special,
    }
}

/// Flags to set when restoring stored flags onto an entry whose flags are
/// `current`.
///
/// Settable flags come from `stored`; `UF_COMPRESSED` and flags outside the
/// settable masks keep their current value.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{UF_COMPRESSED, UF_HIDDEN, flags_to_restore};
///
/// let restored = flags_to_restore(u64::from(UF_HIDDEN), UF_COMPRESSED);
/// assert_eq!(restored, UF_HIDDEN | UF_COMPRESSED);
/// ```
#[must_use]
pub fn flags_to_restore(stored: u64, current: u32) -> u32 {
    let stored = u32::try_from(stored & u64::from(u32::MAX)).unwrap_or_default();
    (stored & RESTORED_FLAGS) | (current & !RESTORED_FLAGS)
}

/// Flags of an entry with every write-blocking flag cleared; `None` when no
/// write-blocking flag is set.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{UF_HIDDEN, UF_IMMUTABLE, unblocked_flags};
///
/// assert_eq!(unblocked_flags(UF_IMMUTABLE | UF_HIDDEN), Some(UF_HIDDEN));
/// assert_eq!(unblocked_flags(UF_HIDDEN), None);
/// ```
#[must_use]
pub fn unblocked_flags(current: u32) -> Option<u32> {
    (current & WRITE_BLOCKING_FLAGS != 0).then_some(current & !WRITE_BLOCKING_FLAGS)
}

/// Flags of an entry with `UF_HIDDEN` added; `None` when it is already set.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{UF_HIDDEN, hidden_flags};
///
/// assert_eq!(hidden_flags(0), Some(UF_HIDDEN));
/// assert_eq!(hidden_flags(UF_HIDDEN), None);
/// ```
#[must_use]
pub fn hidden_flags(current: u32) -> Option<u32> {
    (current & UF_HIDDEN == 0).then_some(current | UF_HIDDEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `UF_NODUMP`.
    const UF_NODUMP: u32 = 0x1;
    /// `SF_ARCHIVED`.
    const SF_ARCHIVED: u32 = 0x1_0000;
    /// `SF_DATALESS`: outside both settable masks.
    const SF_DATALESS: u32 = 0x4000_0000;

    #[test]
    fn classifies_entry_types_from_mode() {
        let table = [
            (0o100_600, FileKind::File),
            (0o040_755, FileKind::Directory),
            (0o120_777, FileKind::Symlink),
            (0o020_666, FileKind::Special),
            (0o060_660, FileKind::Special),
            (0o010_644, FileKind::Special),
            (0o140_755, FileKind::Special),
        ];
        for (mode, expected) in table {
            assert_eq!(file_kind(mode), expected, "{mode:o}");
        }
    }

    #[test]
    fn restores_settable_user_and_system_flags() {
        let stored = u64::from(UF_NODUMP | UF_HIDDEN | SF_ARCHIVED | UF_IMMUTABLE);
        assert_eq!(
            flags_to_restore(stored, 0),
            UF_NODUMP | UF_HIDDEN | SF_ARCHIVED | UF_IMMUTABLE
        );
    }

    #[test]
    fn keeps_compression_and_unsettable_flags_of_target() {
        let stored = u64::from(UF_COMPRESSED | SF_DATALESS | UF_HIDDEN);
        assert_eq!(flags_to_restore(stored, 0), UF_HIDDEN);
        assert_eq!(
            flags_to_restore(0, UF_COMPRESSED | SF_DATALESS | UF_NODUMP),
            UF_COMPRESSED | SF_DATALESS
        );
    }

    #[test]
    fn ignores_stored_bits_above_32() {
        let stored = (1_u64 << 40) | u64::from(UF_HIDDEN);
        assert_eq!(flags_to_restore(stored, 0), UF_HIDDEN);
    }

    #[test]
    fn clears_only_write_blocking_flags() {
        let all_blocking = UF_IMMUTABLE | UF_APPEND | SF_IMMUTABLE | SF_APPEND;
        assert_eq!(unblocked_flags(all_blocking | UF_NODUMP), Some(UF_NODUMP));
        assert_eq!(unblocked_flags(UF_NODUMP | UF_HIDDEN), None);
    }

    #[test]
    fn adds_hidden_flag_once() {
        assert_eq!(hidden_flags(UF_NODUMP), Some(UF_NODUMP | UF_HIDDEN));
        assert_eq!(hidden_flags(UF_NODUMP | UF_HIDDEN), None);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
