// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of macOS volumes by file system type, mount flags and
//! volume capabilities.

use nurt_contracts::fs::{FsFeatures, VolumeKind};

/// `MNT_RDONLY` of `sys/mount.h`.
pub const MNT_RDONLY: u32 = 0x0000_0001;
/// `MNT_LOCAL` of `sys/mount.h`: the volume is stored on a local device.
pub const MNT_LOCAL: u32 = 0x0000_1000;
/// `MNT_QUOTA` of `sys/mount.h`: quotas are enforced on the volume.
pub const MNT_QUOTA: u32 = 0x0000_2000;

/// File system types served over a network (`f_fstypename`).
const NETWORK_TYPES: &[&str] = &["smbfs", "nfs", "afpfs", "webdav", "ftp", "cifs"];
/// File system types kept in memory.
const MEMORY_TYPES: &[&str] = &["tmpfs"];
/// File system types of local disks and disk images.
const DISK_TYPES: &[&str] = &["apfs", "hfs", "msdos", "exfat", "ntfs", "udf", "cd9660"];
/// File system types without durable metadata after a flush.
const FAT_TYPES: &[&str] = &["msdos", "exfat"];

/// Volume capabilities reported by `getattrlist(ATTR_VOL_CAPABILITIES)`.
///
/// A capability is known only when its bit is set in the `valid` mask; an
/// unknown capability leaves the default of the file system type in force.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VolumeCapabilities {
    /// `capabilities[VOL_CAPABILITIES_FORMAT]`.
    pub format: u32,
    /// `valid[VOL_CAPABILITIES_FORMAT]`.
    pub format_valid: u32,
    /// `capabilities[VOL_CAPABILITIES_INTERFACES]`.
    pub interfaces: u32,
    /// `valid[VOL_CAPABILITIES_INTERFACES]`.
    pub interfaces_valid: u32,
}

/// `VOL_CAP_FMT_CASE_SENSITIVE`.
pub const VOL_CAP_FMT_CASE_SENSITIVE: u32 = 0x0000_0100;
/// `VOL_CAP_INT_CLONE`.
pub const VOL_CAP_INT_CLONE: u32 = 0x0001_0000;
/// `VOL_CAP_INT_RENAME_SWAP`.
pub const VOL_CAP_INT_RENAME_SWAP: u32 = 0x0004_0000;
/// `VOL_CAP_INT_RENAME_EXCL`.
pub const VOL_CAP_INT_RENAME_EXCL: u32 = 0x0008_0000;
/// `VOL_CAP_INT_EXTENDED_ATTR`.
pub const VOL_CAP_INT_EXTENDED_ATTR: u32 = 0x0000_4000;
/// `VOL_CAP_INT_EXTENDED_SECURITY`.
pub const VOL_CAP_INT_EXTENDED_SECURITY: u32 = 0x0000_0400;

impl VolumeCapabilities {
    /// Whether the volume reports a format capability; `None` when the bit
    /// is not valid.
    #[must_use]
    pub fn format_bit(&self, bit: u32) -> Option<bool> {
        (self.format_valid & bit != 0).then_some(self.format & bit != 0)
    }

    /// Whether the volume reports an interface capability; `None` when the
    /// bit is not valid.
    #[must_use]
    pub fn interface_bit(&self, bit: u32) -> Option<bool> {
        (self.interfaces_valid & bit != 0).then_some(self.interfaces & bit != 0)
    }
}

/// Class of a volume from its file system type, mount flags and removability.
///
/// Network types (`smbfs`, `nfs`, `afpfs`, `webdav`, `ftp`, `cifs`) and every
/// volume without `MNT_LOCAL` are [`VolumeKind::Network`]; `tmpfs` is
/// [`VolumeKind::Memory`]; a local disk type is [`VolumeKind::Removable`] when
/// the medium is removable or ejectable and [`VolumeKind::Fixed`] otherwise.
/// Pseudo file systems (`devfs`, `autofs`, `nullfs`) and unknown types are
/// [`VolumeKind::Other`].
///
/// RAM disks created with `hdiutil attach ram://` carry a disk type (`hfs`
/// or `apfs`) on a device that reports neither removability nor a memory
/// medium through these inputs; they are classified like other disk images.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::VolumeKind;
/// use nurt_fs::macos::{MNT_LOCAL, volume_kind};
///
/// assert_eq!(volume_kind("apfs", MNT_LOCAL, false), VolumeKind::Fixed);
/// assert_eq!(volume_kind("smbfs", 0, false), VolumeKind::Network);
/// assert_eq!(volume_kind("exfat", MNT_LOCAL, true), VolumeKind::Removable);
/// ```
#[must_use]
pub fn volume_kind(fs_type: &str, mount_flags: u32, removable: bool) -> VolumeKind {
    if NETWORK_TYPES.contains(&fs_type) || mount_flags & MNT_LOCAL == 0 {
        VolumeKind::Network
    } else if MEMORY_TYPES.contains(&fs_type) {
        VolumeKind::Memory
    } else if !DISK_TYPES.contains(&fs_type) {
        VolumeKind::Other
    } else if removable {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
    }
}

/// Capabilities of a volume from its file system type, the answer of
/// `pathconf(_PC_CASE_SENSITIVE)` and the capabilities reported by the
/// volume.
///
/// The defaults per type are: APFS — exchange, rename without replacement,
/// hard links, clones, extended attributes, ACLs and durable metadata; HFS+
/// — hard links, extended attributes, ACLs, durable metadata and decomposed
/// names; FAT and exFAT — none of these. Reported capabilities
/// (`VOL_CAP_INT_RENAME_SWAP`, `VOL_CAP_INT_RENAME_EXCL`,
/// `VOL_CAP_INT_CLONE`, `VOL_CAP_INT_EXTENDED_ATTR`,
/// `VOL_CAP_INT_EXTENDED_SECURITY`) override the defaults when valid. Case
/// sensitivity comes from `pathconf` when it answers, then from
/// `VOL_CAP_FMT_CASE_SENSITIVE`, and is `false` otherwise, which is the
/// default of APFS and HFS+.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{VolumeCapabilities, volume_features};
///
/// let apfs = volume_features("apfs", None, None);
/// assert!(apfs.exchange && apfs.reflink && !apfs.case_sensitive);
/// let fat = volume_features("msdos", Some(false), None);
/// assert!(!fat.durable_metadata && !fat.hardlink);
/// ```
#[must_use]
pub fn volume_features(
    fs_type: &str,
    pathconf_case_sensitive: Option<bool>,
    capabilities: Option<VolumeCapabilities>,
) -> FsFeatures {
    let apfs = fs_type == "apfs";
    let hfs = fs_type == "hfs";
    let caps = capabilities.unwrap_or_default();
    let interface = |bit: u32, default: bool| caps.interface_bit(bit).unwrap_or(default);
    let local_disk = DISK_TYPES.contains(&fs_type) && !FAT_TYPES.contains(&fs_type);
    FsFeatures {
        exchange: interface(VOL_CAP_INT_RENAME_SWAP, apfs),
        rename_noreplace: interface(VOL_CAP_INT_RENAME_EXCL, apfs),
        replace_with_backup: false,
        hardlink: apfs || hfs || fs_type == "nfs",
        reflink: interface(VOL_CAP_INT_CLONE, apfs),
        xattr: interface(VOL_CAP_INT_EXTENDED_ATTR, apfs || hfs),
        acl: interface(VOL_CAP_INT_EXTENDED_SECURITY, apfs || hfs),
        alternate_streams: false,
        durable_metadata: local_disk,
        case_sensitive: pathconf_case_sensitive
            .or_else(|| caps.format_bit(VOL_CAP_FMT_CASE_SENSITIVE))
            .unwrap_or(false),
        decomposes_names: hfs,
    }
}

/// Text of a fixed-size, NUL-terminated name field of `struct statfs`
/// (`f_fstypename`, `f_mntonname`); bytes after the first NUL are ignored.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::statfs_name;
///
/// assert_eq!(statfs_name(b"apfs\0\0\0\0"), b"apfs");
/// ```
#[must_use]
pub fn statfs_name(field: &[u8]) -> &[u8] {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    field.get(..end).unwrap_or_default()
}

/// Device number of a volume or file as stored in contract types.
///
/// `dev_t` is a signed 32-bit value on macOS; it is widened without sign
/// extension so that `FileId::volume` and `VolumeId::dev` agree with
/// `statfs.f_fsid.val[0]`.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::device_number;
///
/// assert_eq!(device_number(-1), 0xffff_ffff);
/// ```
#[must_use]
pub fn device_number(raw: i32) -> u64 {
    u64::from(raw.cast_unsigned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_VALID: u32 = u32::MAX;

    fn caps(interfaces: u32) -> VolumeCapabilities {
        VolumeCapabilities {
            format: 0,
            format_valid: FULL_VALID,
            interfaces,
            interfaces_valid: FULL_VALID,
        }
    }

    #[test]
    fn classifies_file_system_types_into_volume_kinds() {
        let table: &[(&str, u32, bool, VolumeKind)] = &[
            ("apfs", MNT_LOCAL, false, VolumeKind::Fixed),
            ("hfs", MNT_LOCAL, false, VolumeKind::Fixed),
            ("apfs", MNT_LOCAL | MNT_RDONLY, false, VolumeKind::Fixed),
            ("apfs", MNT_LOCAL, true, VolumeKind::Removable),
            ("msdos", MNT_LOCAL, true, VolumeKind::Removable),
            ("exfat", MNT_LOCAL, false, VolumeKind::Fixed),
            ("smbfs", 0, false, VolumeKind::Network),
            ("nfs", 0, false, VolumeKind::Network),
            ("afpfs", 0, false, VolumeKind::Network),
            ("webdav", 0, false, VolumeKind::Network),
            ("smbfs", MNT_LOCAL, false, VolumeKind::Network),
            ("apfs", 0, false, VolumeKind::Network),
            ("tmpfs", MNT_LOCAL, false, VolumeKind::Memory),
            ("devfs", MNT_LOCAL, false, VolumeKind::Other),
            ("autofs", MNT_LOCAL, false, VolumeKind::Other),
            ("unknownfs", MNT_LOCAL, true, VolumeKind::Other),
        ];
        for &(fs_type, flags, removable, expected) in table {
            assert_eq!(
                volume_kind(fs_type, flags, removable),
                expected,
                "{fs_type} {flags:#x} {removable}"
            );
        }
    }

    #[test]
    fn gives_apfs_every_preservation_feature() {
        let features = volume_features("apfs", Some(false), None);
        assert!(features.exchange);
        assert!(features.rename_noreplace);
        assert!(features.hardlink);
        assert!(features.reflink);
        assert!(features.xattr);
        assert!(features.acl);
        assert!(features.durable_metadata);
        assert!(!features.case_sensitive);
        assert!(!features.decomposes_names);
        assert!(!features.replace_with_backup);
        assert!(!features.alternate_streams);
    }

    #[test]
    fn gives_hfs_decomposed_names_without_clones() {
        let features = volume_features("hfs", None, None);
        assert!(!features.reflink);
        assert!(!features.exchange);
        assert!(features.hardlink);
        assert!(features.decomposes_names);
        assert!(features.durable_metadata);
    }

    #[test]
    fn denies_durable_metadata_and_links_on_fat() {
        for fs_type in ["msdos", "exfat"] {
            let features = volume_features(fs_type, Some(false), None);
            assert!(!features.durable_metadata, "{fs_type}");
            assert!(!features.hardlink, "{fs_type}");
            assert!(!features.xattr, "{fs_type}");
            assert!(!features.exchange, "{fs_type}");
        }
    }

    #[test]
    fn lets_reported_capabilities_override_type_defaults() {
        let hfs_with_swap = volume_features(
            "hfs",
            None,
            Some(caps(VOL_CAP_INT_RENAME_SWAP | VOL_CAP_INT_RENAME_EXCL)),
        );
        assert!(hfs_with_swap.exchange);
        assert!(hfs_with_swap.rename_noreplace);
        assert!(!hfs_with_swap.reflink);

        let apfs_without_clone = volume_features("apfs", None, Some(caps(0)));
        assert!(!apfs_without_clone.reflink);
        assert!(!apfs_without_clone.exchange);
    }

    #[test]
    fn ignores_capabilities_outside_valid_mask() {
        let reported = VolumeCapabilities {
            format: VOL_CAP_FMT_CASE_SENSITIVE,
            format_valid: 0,
            interfaces: 0,
            interfaces_valid: 0,
        };
        let features = volume_features("apfs", None, Some(reported));
        assert!(features.reflink);
        assert!(!features.case_sensitive);
    }

    #[test]
    fn takes_case_sensitivity_from_pathconf_then_capabilities() {
        let reported = VolumeCapabilities {
            format: VOL_CAP_FMT_CASE_SENSITIVE,
            format_valid: VOL_CAP_FMT_CASE_SENSITIVE,
            interfaces: 0,
            interfaces_valid: 0,
        };
        assert!(volume_features("apfs", None, Some(reported)).case_sensitive);
        assert!(!volume_features("apfs", Some(false), Some(reported)).case_sensitive);
        assert!(volume_features("apfs", Some(true), None).case_sensitive);
    }

    #[test]
    fn reads_nul_terminated_statfs_names() {
        assert_eq!(statfs_name(b"apfs\0\0\0\0"), b"apfs");
        assert_eq!(statfs_name(b"hfs"), b"hfs");
        assert_eq!(statfs_name(b"\0apfs"), b"");
    }

    #[test]
    fn widens_device_numbers_without_sign_extension() {
        assert_eq!(device_number(16_777_220), 16_777_220);
        assert_eq!(device_number(i32::MIN), 0x8000_0000);
    }
}
