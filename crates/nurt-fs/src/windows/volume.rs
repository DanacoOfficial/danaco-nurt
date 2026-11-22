// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of Windows volumes and their space figures.

use nurt_contracts::fs::{FsFeatures, SpaceInfo, VolumeKind};

/// Drive types returned by `GetDriveTypeW`.
pub mod drive_types {
    /// The type cannot be determined.
    pub const UNKNOWN: u32 = 0;
    /// The root path is invalid.
    pub const NO_ROOT_DIR: u32 = 1;
    /// Removable medium.
    pub const REMOVABLE: u32 = 2;
    /// Fixed disk.
    pub const FIXED: u32 = 3;
    /// Network share.
    pub const REMOTE: u32 = 4;
    /// Optical drive.
    pub const CDROM: u32 = 5;
    /// RAM disk.
    pub const RAMDISK: u32 = 6;
}

/// File system flags returned by `GetVolumeInformationByHandleW`.
pub mod volume_flags {
    /// Case-sensitive lookups are possible (set by NTFS; names remain
    /// case-insensitive by default).
    pub const CASE_SENSITIVE_SEARCH: u32 = 0x0000_0001;
    /// Access control lists are kept.
    pub const PERSISTENT_ACLS: u32 = 0x0000_0008;
    /// Rename and delete with POSIX semantics are supported.
    pub const SUPPORTS_POSIX_UNLINK_RENAME: u32 = 0x0000_0400;
    /// Named data streams are supported.
    pub const NAMED_STREAMS: u32 = 0x0004_0000;
    /// The volume is read-only.
    pub const READ_ONLY_VOLUME: u32 = 0x0008_0000;
    /// Hard links are supported.
    pub const SUPPORTS_HARD_LINKS: u32 = 0x0040_0000;
    /// Block reference counting (block cloning) is supported.
    pub const SUPPORTS_BLOCK_REFCOUNTING: u32 = 0x0800_0000;
}

#[allow(
    clippy::wildcard_imports,
    reason = "the volume flags keep their Win32 names without the module prefix"
)]
use volume_flags::*;

/// File system type names, as reported in [`nurt_contracts::fs::VolumeInfo`].
pub mod fs_types {
    /// NTFS.
    pub const NTFS: &str = "ntfs";
    /// Resilient File System.
    pub const REFS: &str = "refs";
}

/// File systems whose metadata is not journaled.
const UNJOURNALED_FS_TYPES: [&str; 5] = ["fat", "fat12", "fat16", "fat32", "exfat"];

/// Number of characters of a volume GUID in text form.
const GUID_TEXT_LEN: usize = 36;
/// Positions of the dashes in the text form of a GUID.
const GUID_DASHES: [usize; 4] = [8, 13, 18, 23];
/// Prefix of a volume GUID path.
const VOLUME_GUID_PREFIX: &str = r"\\?\Volume{";
/// Suffix of a volume GUID path.
const VOLUME_GUID_SUFFIX: &str = r"}\";

/// Volume class from the drive type.
///
/// Optical drives are removable media; unknown types and invalid roots are
/// [`VolumeKind::Other`].
#[must_use]
pub fn volume_kind(drive_type: u32) -> VolumeKind {
    match drive_type {
        drive_types::FIXED => VolumeKind::Fixed,
        drive_types::REMOVABLE | drive_types::CDROM => VolumeKind::Removable,
        drive_types::REMOTE => VolumeKind::Network,
        drive_types::RAMDISK => VolumeKind::Memory,
        _ => VolumeKind::Other,
    }
}

/// File system type name in the portable lower-case form (`NTFS` becomes
/// `ntfs`, `ReFS` becomes `refs`).
#[must_use]
pub fn fs_type_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Capabilities of a volume from its file system type (as returned by
/// [`fs_type_name`]), its flags and its class.
///
/// Names are reported case-insensitive: NTFS sets the case-sensitive search
/// flag without making names case-sensitive, and per-directory case
/// sensitivity is not reflected in volume capabilities. Exchange of names is
/// unavailable on Windows; replacement keeps the old file under a backup name
/// instead.
#[must_use]
pub fn features(fs_type: &str, flags: u32, kind: VolumeKind) -> FsFeatures {
    let replace_capable = fs_type == fs_types::NTFS || fs_type == fs_types::REFS;
    FsFeatures {
        exchange: false,
        rename_noreplace: true,
        replace_with_backup: replace_capable,
        hardlink: flags & SUPPORTS_HARD_LINKS != 0,
        reflink: fs_type == fs_types::REFS && flags & SUPPORTS_BLOCK_REFCOUNTING != 0,
        xattr: false,
        acl: flags & PERSISTENT_ACLS != 0,
        alternate_streams: flags & NAMED_STREAMS != 0,
        durable_metadata: kind != VolumeKind::Network && !UNJOURNALED_FS_TYPES.contains(&fs_type),
        case_sensitive: false,
        decomposes_names: false,
    }
}

/// Whether the flags mark a read-only volume.
#[must_use]
pub fn is_read_only(flags: u32) -> bool {
    flags & READ_ONLY_VOLUME != 0
}

/// Whether rename and delete with POSIX semantics are available.
#[must_use]
pub fn supports_posix_semantics(flags: u32) -> bool {
    flags & SUPPORTS_POSIX_UNLINK_RENAME != 0
}

/// Lower-case GUID of a volume GUID path (`\\?\Volume{GUID}\`), the form
/// kept in [`nurt_contracts::fs::VolumeId::uuid`].
///
/// Returns `None` when the text is not a volume GUID path.
///
/// # Examples
///
/// ```
/// use nurt_fs::windows::volume::{parse_volume_guid_path, volume_guid_path};
///
/// let guid = parse_volume_guid_path(r"\\?\Volume{3F2504E0-4F89-11D3-9A0C-0305E82C3301}\");
/// assert_eq!(guid.as_deref(), Some("3f2504e0-4f89-11d3-9a0c-0305e82c3301"));
/// assert_eq!(
///     volume_guid_path("3f2504e0-4f89-11d3-9a0c-0305e82c3301"),
///     r"\\?\Volume{3f2504e0-4f89-11d3-9a0c-0305e82c3301}\"
/// );
/// ```
#[must_use]
pub fn parse_volume_guid_path(path: &str) -> Option<String> {
    let guid = path
        .strip_prefix(VOLUME_GUID_PREFIX)?
        .strip_suffix(VOLUME_GUID_SUFFIX)?;
    let valid = guid.len() == GUID_TEXT_LEN
        && guid.char_indices().all(|(index, character)| {
            if GUID_DASHES.contains(&index) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        });
    valid.then(|| guid.to_ascii_lowercase())
}

/// Volume GUID path (`\\?\Volume{GUID}\`) for a GUID kept in
/// [`nurt_contracts::fs::VolumeId::uuid`].
#[must_use]
pub fn volume_guid_path(guid: &str) -> String {
    format!("{VOLUME_GUID_PREFIX}{guid}{VOLUME_GUID_SUFFIX}")
}

/// Space figures from the three values of `GetDiskFreeSpaceExW`.
///
/// `free_to_caller` and `total_to_caller` already reflect the quota of the
/// account. A remaining quota is reported when the bytes available to the
/// caller are fewer than the free bytes of the volume, which happens only when
/// a per-account limit applies; a limit whose remainder equals the free space
/// has no effect and is not distinguishable. The quota limit is reported with
/// the remaining quota as `total_to_caller`, which `GetDiskFreeSpaceExW` sets
/// to the quota.
#[must_use]
pub fn space_info(free_to_caller: u64, total_to_caller: u64, total_free: u64) -> SpaceInfo {
    let quota_remaining = (free_to_caller < total_free).then_some(free_to_caller);
    SpaceInfo {
        total: total_to_caller,
        available: free_to_caller,
        quota_remaining,
        quota_limit: quota_remaining.map(|_| total_to_caller),
        purgeable_system: None,
    }
}

#[cfg(windows)]
mod verify {
    use super::volume_flags;
    use windows_sys::Win32::System::SystemServices as ss;

    const _: () = assert!(volume_flags::CASE_SENSITIVE_SEARCH == ss::FILE_CASE_SENSITIVE_SEARCH);
    const _: () = assert!(volume_flags::PERSISTENT_ACLS == ss::FILE_PERSISTENT_ACLS);
    const _: () = assert!(
        volume_flags::SUPPORTS_POSIX_UNLINK_RENAME == ss::FILE_SUPPORTS_POSIX_UNLINK_RENAME
    );
    const _: () = assert!(volume_flags::NAMED_STREAMS == ss::FILE_NAMED_STREAMS);
    const _: () = assert!(volume_flags::READ_ONLY_VOLUME == ss::FILE_READ_ONLY_VOLUME);
    const _: () = assert!(volume_flags::SUPPORTS_HARD_LINKS == ss::FILE_SUPPORTS_HARD_LINKS);
    const _: () =
        assert!(volume_flags::SUPPORTS_BLOCK_REFCOUNTING == ss::FILE_SUPPORTS_BLOCK_REFCOUNTING);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NTFS_FLAGS: u32 =
        CASE_SENSITIVE_SEARCH | PERSISTENT_ACLS | NAMED_STREAMS | SUPPORTS_HARD_LINKS;

    #[test]
    fn classifies_drive_types() {
        assert_eq!(volume_kind(drive_types::FIXED), VolumeKind::Fixed);
        assert_eq!(volume_kind(drive_types::REMOVABLE), VolumeKind::Removable);
        assert_eq!(volume_kind(drive_types::CDROM), VolumeKind::Removable);
        assert_eq!(volume_kind(drive_types::REMOTE), VolumeKind::Network);
        assert_eq!(volume_kind(drive_types::RAMDISK), VolumeKind::Memory);
        assert_eq!(volume_kind(drive_types::UNKNOWN), VolumeKind::Other);
        assert_eq!(volume_kind(drive_types::NO_ROOT_DIR), VolumeKind::Other);
    }

    #[test]
    fn normalises_file_system_names() {
        assert_eq!(fs_type_name("NTFS"), "ntfs");
        assert_eq!(fs_type_name("ReFS"), "refs");
        assert_eq!(fs_type_name("exFAT "), "exfat");
    }

    #[test]
    fn describes_ntfs_on_a_fixed_disk() {
        let features = features("ntfs", NTFS_FLAGS, VolumeKind::Fixed);
        assert!(features.replace_with_backup);
        assert!(features.hardlink);
        assert!(!features.reflink);
        assert!(features.acl);
        assert!(features.alternate_streams);
        assert!(features.durable_metadata);
        assert!(features.rename_noreplace);
        assert!(!features.exchange);
        assert!(!features.xattr);
        assert!(!features.case_sensitive);
        assert!(!features.decomposes_names);
    }

    #[test]
    fn enables_clones_only_on_refs_with_block_refcounting() {
        let refs = features(
            "refs",
            PERSISTENT_ACLS | SUPPORTS_BLOCK_REFCOUNTING,
            VolumeKind::Fixed,
        );
        assert!(refs.reflink);
        assert!(refs.replace_with_backup);
        assert!(!refs.alternate_streams);
        let old_refs = features("refs", PERSISTENT_ACLS, VolumeKind::Fixed);
        assert!(!old_refs.reflink);
        let ntfs = features("ntfs", SUPPORTS_BLOCK_REFCOUNTING, VolumeKind::Fixed);
        assert!(!ntfs.reflink);
    }

    #[test]
    fn marks_fat_and_network_volumes_without_durable_metadata() {
        for name in ["fat", "fat32", "exfat"] {
            let fat = features(name, 0, VolumeKind::Removable);
            assert!(!fat.durable_metadata, "{name}");
            assert!(!fat.replace_with_backup, "{name}");
            assert!(!fat.acl, "{name}");
        }
        let share = features("ntfs", NTFS_FLAGS, VolumeKind::Network);
        assert!(!share.durable_metadata);
        assert!(share.replace_with_backup);
    }

    #[test]
    fn reads_volume_flags() {
        assert!(is_read_only(READ_ONLY_VOLUME));
        assert!(!is_read_only(NTFS_FLAGS));
        assert!(supports_posix_semantics(SUPPORTS_POSIX_UNLINK_RENAME));
        assert!(!supports_posix_semantics(NTFS_FLAGS));
    }

    #[test]
    fn parses_volume_guid_paths() {
        let path = r"\\?\Volume{3f2504e0-4f89-11d3-9a0c-0305e82c3301}\";
        let guid = parse_volume_guid_path(path).unwrap();
        assert_eq!(volume_guid_path(&guid), path);
        for invalid in [
            r"C:\",
            r"\\?\Volume{3f2504e0-4f89-11d3-9a0c-0305e82c3301}",
            r"\\?\Volume{3f2504e0x4f89-11d3-9a0c-0305e82c3301}\",
            r"\\?\Volume{3f2504e0-4f89-11d3-9a0c-0305e82c330g}\",
            r"\\?\Volume{3f2504e0-4f89-11d3-9a0c}\",
        ] {
            assert_eq!(parse_volume_guid_path(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn reports_quota_only_when_it_limits_the_caller() {
        let unlimited = space_info(500, 1_000, 500);
        assert_eq!(unlimited.available, 500);
        assert_eq!(unlimited.total, 1_000);
        assert_eq!(unlimited.quota_remaining, None);
        assert_eq!(unlimited.quota_limit, None);
        assert_eq!(unlimited.purgeable_system, None);
        let limited = space_info(100, 400, 500);
        assert_eq!(limited.available, 100);
        assert_eq!(limited.total, 400);
        assert_eq!(limited.quota_remaining, Some(100));
        assert_eq!(limited.quota_limit, Some(400));
    }
}
