// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::windows::meta
// Responsibility:   Conversion of Windows file information to the portable metadata model.
// Dependencies:     attributes, nurt_contracts, windows_sys
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conversion of Windows file information to the portable metadata model.
//!
//! Times are `FILETIME` values: 100-nanosecond intervals since
//! 1601-01-01 UTC. Permission bits do not exist on Windows; they are
//! synthesised from the read-only attribute by a fixed rule
//! ([`synthesize_mode`]).

use nurt_contracts::fs::{FileId, FileKind, FileMeta};

/// `FILE_ATTRIBUTE_*` bits used by the back end.
pub mod attributes {
    /// Read-only file.
    pub const READONLY: u32 = 0x0000_0001;
    /// Hidden entry.
    pub const HIDDEN: u32 = 0x0000_0002;
    /// System entry.
    pub const SYSTEM: u32 = 0x0000_0004;
    /// Directory.
    pub const DIRECTORY: u32 = 0x0000_0010;
    /// Archive bit.
    pub const ARCHIVE: u32 = 0x0000_0020;
    /// Device.
    pub const DEVICE: u32 = 0x0000_0040;
    /// No other attribute; used to clear every settable attribute.
    pub const NORMAL: u32 = 0x0000_0080;
    /// Temporary file.
    pub const TEMPORARY: u32 = 0x0000_0100;
    /// Sparse file.
    pub const SPARSE_FILE: u32 = 0x0000_0200;
    /// Reparse point (symbolic link, junction or other).
    pub const REPARSE_POINT: u32 = 0x0000_0400;
    /// NTFS compression.
    pub const COMPRESSED: u32 = 0x0000_0800;
    /// Content not immediately available.
    pub const OFFLINE: u32 = 0x0000_1000;
    /// Excluded from content indexing.
    pub const NOT_CONTENT_INDEXED: u32 = 0x0000_2000;
    /// Encrypting File System.
    pub const ENCRYPTED: u32 = 0x0000_4000;
}

/// Reparse tags recognised by the back end; the tags of the Windows Subsystem
/// for Linux are defined in `ntifs.h` of the Windows Driver Kit.
pub mod reparse_tags {
    /// Junction (mount point).
    pub const MOUNT_POINT: u32 = 0xA000_0003;
    /// Symbolic link.
    pub const SYMLINK: u32 = 0xA000_000C;
    /// Unix domain socket.
    pub const AF_UNIX: u32 = 0x8000_0023;
    /// FIFO created by the Windows Subsystem for Linux.
    pub const LX_FIFO: u32 = 0x8000_0024;
    /// Character device created by the Windows Subsystem for Linux.
    pub const LX_CHR: u32 = 0x8000_0025;
    /// Block device created by the Windows Subsystem for Linux.
    pub const LX_BLK: u32 = 0x8000_0026;
}

#[allow(
    clippy::wildcard_imports,
    reason = "the attribute bits keep their Win32 names without the module prefix"
)]
use attributes::*;

/// Attributes that [`FileMeta::windows_attributes`] restores through
/// `SetFileInformationByHandle(FileBasicInfo)`.
pub const RESTORABLE_ATTRIBUTES: u32 =
    READONLY | HIDDEN | SYSTEM | ARCHIVE | TEMPORARY | OFFLINE | NOT_CONTENT_INDEXED;

/// Attributes set on the journal directory.
pub const JOURNAL_DIRECTORY_ATTRIBUTES: u32 = HIDDEN | SYSTEM | NOT_CONTENT_INDEXED;

/// `FILETIME` ticks between 1601-01-01 and 1970-01-01.
pub const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// Nanoseconds per `FILETIME` tick.
pub const NANOS_PER_TICK: i64 = 100;

/// Permission bits of a writable file.
pub const MODE_FILE: u32 = 0o666;
/// Permission bits of a writable directory.
pub const MODE_DIRECTORY: u32 = 0o777;
/// Write bits removed for read-only entries.
pub const MODE_WRITE_BITS: u32 = 0o222;

/// Converts `FILETIME` ticks to nanoseconds since the Unix epoch.
///
/// Values outside the range of `i64` nanoseconds (before 1677 or after 2262)
/// saturate at the range bounds.
///
/// # Examples
///
/// ```
/// use nurt_fs::windows::meta::{filetime_to_unix_ns, UNIX_EPOCH_TICKS};
///
/// assert_eq!(filetime_to_unix_ns(UNIX_EPOCH_TICKS), 0);
/// assert_eq!(filetime_to_unix_ns(UNIX_EPOCH_TICKS + 1), 100);
/// ```
#[must_use]
pub fn filetime_to_unix_ns(ticks: i64) -> i64 {
    let nanos = (i128::from(ticks) - i128::from(UNIX_EPOCH_TICKS)) * i128::from(NANOS_PER_TICK);
    i64::try_from(nanos).unwrap_or(if nanos < 0 { i64::MIN } else { i64::MAX })
}

/// Converts `FILETIME` ticks to nanoseconds since the Unix epoch, treating
/// zero as a time the file system does not record.
#[must_use]
pub fn optional_filetime(ticks: i64) -> Option<i64> {
    (ticks != 0).then(|| filetime_to_unix_ns(ticks))
}

/// Converts nanoseconds since the Unix epoch to `FILETIME` ticks for
/// `FILE_BASIC_INFO`, rounding down to the 100-nanosecond resolution.
///
/// The range of `i64` nanoseconds (years 1677 to 2262) maps to positive tick
/// values, so the result never takes the special meanings of zero (leave
/// unchanged) and minus one (stop updating) in `FILE_BASIC_INFO`.
#[must_use]
pub fn unix_ns_to_filetime(nanos: i64) -> i64 {
    nanos.div_euclid(NANOS_PER_TICK) + UNIX_EPOCH_TICKS
}

/// Permission bits synthesised from Windows attributes.
///
/// Files are `0o666` and directories `0o777`; the read-only attribute removes
/// the write bits (`0o444`, `0o555`). Execute bits of files are not
/// synthesised.
#[must_use]
pub fn synthesize_mode(attributes: u32) -> u32 {
    let base = if attributes & DIRECTORY != 0 {
        MODE_DIRECTORY
    } else {
        MODE_FILE
    };
    if attributes & READONLY != 0 {
        base & !MODE_WRITE_BITS
    } else {
        base
    }
}

/// Attributes after applying permission bits: the read-only attribute is set
/// when `mode` has no write bit and cleared otherwise; other bits are kept.
#[must_use]
pub fn attributes_for_mode(current: u32, mode: u32) -> u32 {
    if mode & MODE_WRITE_BITS == 0 {
        current | READONLY
    } else {
        current & !READONLY
    }
}

/// Value for the `FileAttributes` field of `FILE_BASIC_INFO` that leaves
/// exactly the restorable bits of `desired` set.
///
/// Zero means "unchanged" in that structure, so an empty set is expressed as
/// `FILE_ATTRIBUTE_NORMAL`.
#[must_use]
pub fn basic_info_attributes(desired: u32) -> u32 {
    let settable = desired & RESTORABLE_ATTRIBUTES;
    if settable == 0 { NORMAL } else { settable }
}

/// Requested attribute bits that cannot be set through `FILE_BASIC_INFO`
/// (directory, compression, encryption, sparse and reparse state).
#[must_use]
pub fn unsettable_attributes(bits: u32) -> u32 {
    bits & !RESTORABLE_ATTRIBUTES
}

/// Names of attribute states that differ between a source entry and its
/// restored copy and that metadata restoration cannot change.
///
/// Compression and encryption are reported (`attribute:compressed`,
/// `attribute:encrypted`); an unencrypted copy of an encrypted file must not
/// pass unnoticed.
#[must_use]
pub fn untransferable_attributes(source: u32, target: u32) -> Vec<String> {
    [(COMPRESSED, "compressed"), (ENCRYPTED, "encrypted")]
        .into_iter()
        .filter(|(bit, _)| (source ^ target) & bit != 0)
        .map(|(_, name)| format!("attribute:{name}"))
        .collect()
}

/// Kind of an entry from its attributes and reparse tag.
///
/// Symbolic links and junctions are reported as such; socket and device
/// reparse points as [`FileKind::Special`]. Other reparse points (cloud
/// placeholders, deduplicated files) are not links and are reported by their
/// directory bit.
#[must_use]
pub fn classify_kind(attributes: u32, reparse_tag: u32) -> FileKind {
    if attributes & REPARSE_POINT != 0 {
        match reparse_tag {
            reparse_tags::SYMLINK => return FileKind::Symlink,
            reparse_tags::MOUNT_POINT => return FileKind::Junction,
            reparse_tags::AF_UNIX
            | reparse_tags::LX_FIFO
            | reparse_tags::LX_CHR
            | reparse_tags::LX_BLK => return FileKind::Special,
            _ => {}
        }
    }
    if attributes & DEVICE != 0 {
        FileKind::Special
    } else if attributes & DIRECTORY != 0 {
        FileKind::Directory
    } else {
        FileKind::File
    }
}

/// Whether recursive operations must treat the entry as a leaf: every reparse
/// point is removed itself and never entered.
#[must_use]
pub fn is_leaf_for_recursion(attributes: u32) -> bool {
    attributes & DIRECTORY == 0 || attributes & REPARSE_POINT != 0
}

/// File identity from the volume serial number and the 128-bit file id of
/// `FILE_ID_INFO` (little-endian halves).
#[must_use]
pub fn file_id(volume_serial: u64, identifier: [u8; 16]) -> FileId {
    let (low, high) = identifier.split_at(8);
    let half = |bytes: &[u8]| {
        let mut out = [0_u8; 8];
        out.copy_from_slice(bytes);
        u64::from_le_bytes(out)
    };
    FileId {
        volume: volume_serial,
        low: half(low),
        high: half(high),
    }
}

/// Raw information of an open file collected from `FILE_BASIC_INFO`,
/// `FILE_STANDARD_INFO`, `FILE_ID_INFO` and `FILE_ATTRIBUTE_TAG_INFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawFileInfo {
    /// `FILE_ATTRIBUTE_*` bits.
    pub attributes: u32,
    /// Reparse tag; meaningful when the reparse-point attribute is set.
    pub reparse_tag: u32,
    /// End of file of the default stream.
    pub size: u64,
    /// Number of hard links.
    pub links: u32,
    /// Creation time in `FILETIME` ticks.
    pub created: i64,
    /// Last access time in `FILETIME` ticks.
    pub accessed: i64,
    /// Last write time in `FILETIME` ticks.
    pub modified: i64,
    /// Last change time in `FILETIME` ticks.
    pub changed: i64,
    /// Volume serial number.
    pub volume_serial: u64,
    /// 128-bit file id.
    pub file_id: [u8; 16],
}

impl RawFileInfo {
    /// Portable metadata of the file. `uid` and `gid` are `None`; the account
    /// of the file is reported by extended metadata as a security identifier.
    #[must_use]
    pub fn to_meta(&self) -> FileMeta {
        let kind = classify_kind(self.attributes, self.reparse_tag);
        let size = if kind == FileKind::Directory {
            0
        } else {
            self.size
        };
        FileMeta {
            kind,
            size,
            mode: synthesize_mode(self.attributes),
            uid: None,
            gid: None,
            links: u64::from(self.links),
            modified_ns: filetime_to_unix_ns(self.modified),
            accessed_ns: optional_filetime(self.accessed),
            created_ns: optional_filetime(self.created),
            changed_ns: optional_filetime(self.changed),
            file_id: file_id(self.volume_serial, self.file_id),
            windows_attributes: self.attributes,
        }
    }
}

#[cfg(windows)]
mod verify {
    use super::{attributes, reparse_tags};
    use windows_sys::Win32::Storage::FileSystem as fs;
    use windows_sys::Win32::System::SystemServices as ss;

    const _: () = assert!(attributes::READONLY == fs::FILE_ATTRIBUTE_READONLY);
    const _: () = assert!(attributes::HIDDEN == fs::FILE_ATTRIBUTE_HIDDEN);
    const _: () = assert!(attributes::SYSTEM == fs::FILE_ATTRIBUTE_SYSTEM);
    const _: () = assert!(attributes::DIRECTORY == fs::FILE_ATTRIBUTE_DIRECTORY);
    const _: () = assert!(attributes::ARCHIVE == fs::FILE_ATTRIBUTE_ARCHIVE);
    const _: () = assert!(attributes::DEVICE == fs::FILE_ATTRIBUTE_DEVICE);
    const _: () = assert!(attributes::NORMAL == fs::FILE_ATTRIBUTE_NORMAL);
    const _: () = assert!(attributes::TEMPORARY == fs::FILE_ATTRIBUTE_TEMPORARY);
    const _: () = assert!(attributes::SPARSE_FILE == fs::FILE_ATTRIBUTE_SPARSE_FILE);
    const _: () = assert!(attributes::REPARSE_POINT == fs::FILE_ATTRIBUTE_REPARSE_POINT);
    const _: () = assert!(attributes::COMPRESSED == fs::FILE_ATTRIBUTE_COMPRESSED);
    const _: () = assert!(attributes::OFFLINE == fs::FILE_ATTRIBUTE_OFFLINE);
    const _: () =
        assert!(attributes::NOT_CONTENT_INDEXED == fs::FILE_ATTRIBUTE_NOT_CONTENT_INDEXED);
    const _: () = assert!(attributes::ENCRYPTED == fs::FILE_ATTRIBUTE_ENCRYPTED);
    const _: () = assert!(reparse_tags::MOUNT_POINT == ss::IO_REPARSE_TAG_MOUNT_POINT);
    const _: () = assert!(reparse_tags::SYMLINK == ss::IO_REPARSE_TAG_SYMLINK);
    const _: () = assert!(reparse_tags::AF_UNIX == ss::IO_REPARSE_TAG_AF_UNIX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_filetime_at_epoch_boundaries() {
        assert_eq!(filetime_to_unix_ns(UNIX_EPOCH_TICKS), 0);
        assert_eq!(filetime_to_unix_ns(UNIX_EPOCH_TICKS - 1), -100);
        assert_eq!(filetime_to_unix_ns(0), i64::MIN);
        assert_eq!(filetime_to_unix_ns(i64::MAX), i64::MAX);
        assert_eq!(filetime_to_unix_ns(i64::MIN), i64::MIN);
    }

    #[test]
    fn converts_unix_time_to_filetime_rounding_down() {
        assert_eq!(unix_ns_to_filetime(0), UNIX_EPOCH_TICKS);
        assert_eq!(unix_ns_to_filetime(199), UNIX_EPOCH_TICKS + 1);
        assert_eq!(unix_ns_to_filetime(-1), UNIX_EPOCH_TICKS - 1);
        assert_eq!(unix_ns_to_filetime(-150), UNIX_EPOCH_TICKS - 2);
    }

    #[test]
    fn maps_every_unix_time_to_a_positive_filetime() {
        assert!(unix_ns_to_filetime(i64::MIN) > 0);
        assert!(unix_ns_to_filetime(i64::MAX) > 0);
    }

    #[test]
    fn round_trips_times_at_tick_resolution() {
        for nanos in [
            0,
            1_700_000_000_000_000_000,
            -1_000_000_000_000_000_000,
            100,
        ] {
            assert_eq!(filetime_to_unix_ns(unix_ns_to_filetime(nanos)), nanos);
        }
    }

    #[test]
    fn treats_zero_filetime_as_unrecorded() {
        assert_eq!(optional_filetime(0), None);
        assert_eq!(optional_filetime(UNIX_EPOCH_TICKS), Some(0));
    }

    #[test]
    fn synthesises_mode_from_read_only_attribute() {
        assert_eq!(synthesize_mode(0), 0o666);
        assert_eq!(synthesize_mode(ARCHIVE), 0o666);
        assert_eq!(synthesize_mode(READONLY), 0o444);
        assert_eq!(synthesize_mode(DIRECTORY), 0o777);
        assert_eq!(synthesize_mode(DIRECTORY | READONLY), 0o555);
    }

    #[test]
    fn maps_mode_back_to_read_only_attribute() {
        assert_eq!(attributes_for_mode(ARCHIVE, 0o444), ARCHIVE | READONLY);
        assert_eq!(attributes_for_mode(ARCHIVE | READONLY, 0o644), ARCHIVE);
        assert_eq!(attributes_for_mode(READONLY, 0o200), 0);
        for attributes in [0, READONLY, DIRECTORY, DIRECTORY | READONLY] {
            let mode = synthesize_mode(attributes);
            assert_eq!(attributes_for_mode(attributes, mode), attributes);
        }
    }

    #[test]
    fn expresses_empty_attribute_set_as_normal() {
        assert_eq!(basic_info_attributes(0), NORMAL);
        assert_eq!(basic_info_attributes(DIRECTORY | COMPRESSED), NORMAL);
        assert_eq!(basic_info_attributes(DIRECTORY | HIDDEN), HIDDEN);
        assert_eq!(
            basic_info_attributes(JOURNAL_DIRECTORY_ATTRIBUTES | ARCHIVE),
            HIDDEN | SYSTEM | NOT_CONTENT_INDEXED | ARCHIVE
        );
    }

    #[test]
    fn reports_unsettable_attribute_bits() {
        assert_eq!(unsettable_attributes(HIDDEN | READONLY), 0);
        assert_eq!(unsettable_attributes(HIDDEN | COMPRESSED), COMPRESSED);
        assert_eq!(
            unsettable_attributes(ENCRYPTED | DIRECTORY),
            ENCRYPTED | DIRECTORY
        );
    }

    #[test]
    fn reports_compression_and_encryption_differences() {
        assert!(untransferable_attributes(ARCHIVE, 0).is_empty());
        assert_eq!(
            untransferable_attributes(ENCRYPTED | COMPRESSED, 0),
            ["attribute:compressed", "attribute:encrypted"]
        );
        assert_eq!(
            untransferable_attributes(0, COMPRESSED),
            ["attribute:compressed"]
        );
    }

    #[test]
    fn classifies_entry_kinds() {
        assert_eq!(classify_kind(0, 0), FileKind::File);
        assert_eq!(classify_kind(DIRECTORY, 0), FileKind::Directory);
        assert_eq!(
            classify_kind(REPARSE_POINT, reparse_tags::SYMLINK),
            FileKind::Symlink
        );
        assert_eq!(
            classify_kind(REPARSE_POINT | DIRECTORY, reparse_tags::SYMLINK),
            FileKind::Symlink
        );
        assert_eq!(
            classify_kind(REPARSE_POINT | DIRECTORY, reparse_tags::MOUNT_POINT),
            FileKind::Junction
        );
        assert_eq!(
            classify_kind(REPARSE_POINT, reparse_tags::AF_UNIX),
            FileKind::Special
        );
        assert_eq!(
            classify_kind(REPARSE_POINT, reparse_tags::LX_FIFO),
            FileKind::Special
        );
        assert_eq!(
            classify_kind(REPARSE_POINT, reparse_tags::LX_CHR),
            FileKind::Special
        );
        assert_eq!(
            classify_kind(REPARSE_POINT, reparse_tags::LX_BLK),
            FileKind::Special
        );
        assert_eq!(classify_kind(REPARSE_POINT, 0x9000_001A), FileKind::File);
        assert_eq!(
            classify_kind(REPARSE_POINT | DIRECTORY, 0x9000_001A),
            FileKind::Directory
        );
        assert_eq!(classify_kind(DEVICE, 0), FileKind::Special);
        assert_eq!(
            classify_kind(DIRECTORY, reparse_tags::SYMLINK),
            FileKind::Directory
        );
    }

    #[test]
    fn never_enters_reparse_points() {
        assert!(!is_leaf_for_recursion(DIRECTORY));
        assert!(is_leaf_for_recursion(DIRECTORY | REPARSE_POINT));
        assert!(is_leaf_for_recursion(0));
        assert!(is_leaf_for_recursion(REPARSE_POINT));
    }

    #[test]
    fn splits_128_bit_file_id_into_halves() {
        let mut identifier = [0_u8; 16];
        identifier[0] = 0x01;
        identifier[7] = 0x02;
        identifier[8] = 0x03;
        identifier[15] = 0x04;
        let id = file_id(7, identifier);
        assert_eq!(id.volume, 7);
        assert_eq!(id.low, 0x0200_0000_0000_0001);
        assert_eq!(id.high, 0x0400_0000_0000_0003);
    }

    #[test]
    fn builds_portable_metadata_from_raw_information() {
        let raw = RawFileInfo {
            attributes: READONLY | ARCHIVE,
            reparse_tag: 0,
            size: 42,
            links: 2,
            created: UNIX_EPOCH_TICKS + 10,
            accessed: 0,
            modified: UNIX_EPOCH_TICKS + 20,
            changed: UNIX_EPOCH_TICKS + 30,
            volume_serial: 9,
            file_id: [1; 16],
        };
        let meta = raw.to_meta();
        assert_eq!(meta.kind, FileKind::File);
        assert_eq!(meta.size, 42);
        assert_eq!(meta.mode, 0o444);
        assert_eq!((meta.uid, meta.gid), (None, None));
        assert_eq!(meta.links, 2);
        assert_eq!(meta.created_ns, Some(1_000));
        assert_eq!(meta.accessed_ns, None);
        assert_eq!(meta.modified_ns, 2_000);
        assert_eq!(meta.changed_ns, Some(3_000));
        assert_eq!(meta.file_id.volume, 9);
        assert_eq!(meta.windows_attributes, READONLY | ARCHIVE);
    }

    #[test]
    fn reports_zero_size_for_directories() {
        let raw = RawFileInfo {
            attributes: DIRECTORY,
            reparse_tag: 0,
            size: 4096,
            links: 1,
            created: 0,
            accessed: 0,
            modified: 0,
            changed: 0,
            volume_serial: 0,
            file_id: [0; 16],
        };
        let meta = raw.to_meta();
        assert_eq!(meta.kind, FileKind::Directory);
        assert_eq!(meta.size, 0);
        assert_eq!(meta.mode, 0o777);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
