// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::linux::table
// Responsibility:   Volume classes and capabilities by file system type.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Volume classes and capabilities by file system type.
//!
//! Linux reports the file system type of a mount in the mount table
//! (`fuse.<subtype>` for FUSE). Capabilities that the kernel does not report
//! are taken from the table below, which follows the `rename` and `ioctl`
//! implementations of the kernel file systems; flags that depend on the
//! volume (ACL support, case sensitivity of ZFS, XFS reflink) are read from
//! the superblock options or decided by the caller.

use nurt_contracts::fs::{FsFeatures, VolumeKind};

/// Network file systems; the journal is disabled on them by default.
const NETWORK_TYPES: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "sshfs",
    "9p",
    "afs",
    "ceph",
    "glusterfs",
    "lustre",
    "gpfs",
    "beegfs",
    "orangefs",
    "coda",
    "davfs",
    "ncpfs",
    "virtiofs",
];

/// FUSE subtypes of file systems that store data on another machine.
const REMOTE_FUSE_SUBTYPES: &[&str] = &[
    "sshfs",
    "rclone",
    "s3fs",
    "gcsfuse",
    "goofys",
    "glusterfs",
    "ceph-fuse",
    "juicefs",
    "curlftpfs",
    "smbnetfs",
    "onedriver",
    "mountpoint-s3",
    "blobfuse2",
];

/// Memory-backed file systems.
const MEMORY_TYPES: &[&str] = &["tmpfs", "ramfs", "devtmpfs"];

/// Overlay file systems: the kernel driver and its FUSE implementation.
const OVERLAY_TYPES: &[&str] = &["overlay", "fuse.fuse-overlayfs"];

/// Kernel interfaces and other file systems that hold no user data.
const PSEUDO_TYPES: &[&str] = &[
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "devpts",
    "mqueue",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "efivarfs",
    "configfs",
    "fusectl",
    "bpf",
    "autofs",
    "binfmt_misc",
    "hugetlbfs",
    "rpc_pipefs",
    "nsfs",
    "selinuxfs",
    "nfsd",
    "fuse.lxcfs",
    "fuse.portal",
];

/// FAT family: no hard links, no extended attributes, case-insensitive
/// names, metadata not journaled.
const FAT_TYPES: &[&str] = &["vfat", "exfat", "msdos", "fat"];

/// File systems whose `rename` accepts `RENAME_EXCHANGE`.
const EXCHANGE_TYPES: &[&str] = &[
    "ext3", "ext4", "xfs", "btrfs", "tmpfs", "f2fs", "overlay", "bcachefs", "ubifs", "shmem", "zfs",
];

/// File systems whose `rename` accepts `RENAME_NOREPLACE`.
const NOREPLACE_TYPES: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "tmpfs", "f2fs", "overlay", "bcachefs", "ubifs",
    "shmem", "vfat", "msdos", "exfat", "ntfs3", "jfs", "reiserfs", "minix", "hfsplus", "udf",
    "zfs",
];

/// File systems that share blocks through `FICLONE`; ZFS shares blocks
/// from `OpenZFS` 2.2 with the `block_cloning` pool feature. XFS shares
/// blocks only when formatted with `reflink=1` and is decided by the caller.
const REFLINK_TYPES: &[&str] = &["btrfs", "bcachefs", "ocfs2", "zfs"];

/// File systems with POSIX ACL unless mounted with `noacl`.
const ACL_TYPES: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "tmpfs", "f2fs", "overlay", "bcachefs", "jfs",
    "reiserfs", "ocfs2",
];

/// File systems with POSIX ACL only when the superblock option `acl` or
/// `posixacl` is present.
const OPTIONAL_ACL_TYPES: &[&str] = &["zfs", "ntfs3"];

/// Magic numbers of `statfs(2)` (`f_type`) and the type names used in the
/// mount table, for systems without a readable mount table.
const MAGIC_TYPES: &[(u64, &str)] = &[
    (0xEF53, "ext4"),
    (0x5846_5342, "xfs"),
    (0x9123_683E, "btrfs"),
    (0x0102_1994, "tmpfs"),
    (0x8584_58F6, "ramfs"),
    (0x794C_7630, "overlay"),
    (0xF2F5_2010, "f2fs"),
    (0xCA45_1A4E, "bcachefs"),
    (0x2FC1_2FC1, "zfs"),
    (0x6969, "nfs"),
    (0xFF53_4D42, "cifs"),
    (0xFE53_4D42, "smb3"),
    (0x0102_1997, "9p"),
    (0x6573_5546, "fuse"),
    (0x4D44, "vfat"),
    (0x2011_BAB0, "exfat"),
    (0x7366_746E, "ntfs3"),
    (0x7371_7368, "squashfs"),
    (0x9660, "iso9660"),
    (0x9FA0, "proc"),
    (0x6265_6572, "sysfs"),
    (0x6367_7270, "cgroup2"),
];

/// Class of a volume from its file system type, mount source and the
/// removable flag of its block device.
///
/// FUSE file systems (`fuse.<subtype>`) are network volumes when the
/// subtype is a known remote file system (`sshfs`, `rclone`, `s3fs` and
/// others) or when the mount source names another machine (`host:/path`,
/// `user@host:path`, `//server/share`); other FUSE file systems are local.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::VolumeKind;
/// use nurt_fs::linux::volume_kind;
///
/// assert_eq!(volume_kind("overlay", b"overlay", false), VolumeKind::Overlay);
/// assert_eq!(volume_kind("fuse.sshfs", b"user@host:/home/user", false), VolumeKind::Network);
/// assert_eq!(volume_kind("ext4", b"/dev/sdb1", true), VolumeKind::Removable);
/// ```
#[must_use]
pub fn volume_kind(fs_type: &str, source: &[u8], removable: bool) -> VolumeKind {
    if is_network(fs_type, source) {
        VolumeKind::Network
    } else if MEMORY_TYPES.contains(&fs_type) {
        VolumeKind::Memory
    } else if OVERLAY_TYPES.contains(&fs_type) {
        VolumeKind::Overlay
    } else if PSEUDO_TYPES.contains(&fs_type) {
        VolumeKind::Other
    } else if removable {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
    }
}

/// Capabilities of a volume from its file system type, class and
/// superblock options.
///
/// `reflink` is `false` for XFS: the caller sets it from the file system
/// geometry, because only volumes formatted with `reflink=1` share blocks.
/// Linux names are case-sensitive except on the FAT family and on ZFS
/// datasets created with `casesensitivity=insensitive` or `mixed`;
/// directories with the ext4 `casefold` attribute are not reflected.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::VolumeKind;
/// use nurt_fs::linux::features_for;
///
/// let ext4 = features_for("ext4", VolumeKind::Fixed, &[b"rw".to_vec()]);
/// assert!(ext4.exchange && ext4.hardlink && ext4.acl && !ext4.reflink);
/// let fat = features_for("vfat", VolumeKind::Removable, &[]);
/// assert!(!fat.hardlink && !fat.durable_metadata && !fat.case_sensitive);
/// ```
#[must_use]
pub fn features_for(fs_type: &str, kind: VolumeKind, super_options: &[Vec<u8>]) -> FsFeatures {
    let has_option = |name: &str| super_options.iter().any(|option| option == name.as_bytes());
    let fat = FAT_TYPES.contains(&fs_type);
    let network = kind == VolumeKind::Network;
    let pseudo = kind == VolumeKind::Other && PSEUDO_TYPES.contains(&fs_type);
    let acl = if ACL_TYPES.contains(&fs_type) {
        !has_option("noacl")
    } else if OPTIONAL_ACL_TYPES.contains(&fs_type) {
        has_option("acl") || has_option("posixacl")
    } else {
        false
    };
    let case_insensitive_zfs =
        fs_type == "zfs" && (has_option("caseinsensitive") || has_option("casemixed"));
    FsFeatures {
        exchange: EXCHANGE_TYPES.contains(&fs_type),
        rename_noreplace: NOREPLACE_TYPES.contains(&fs_type),
        replace_with_backup: false,
        hardlink: !fat && !pseudo && fs_type != "iso9660",
        reflink: REFLINK_TYPES.contains(&fs_type),
        xattr: !fat && !network && !pseudo && fs_type != "iso9660" && !has_option("noxattr"),
        acl,
        alternate_streams: false,
        durable_metadata: !fat && !network,
        case_sensitive: !fat && !case_insensitive_zfs,
        decomposes_names: false,
    }
}

/// Type name of a file system from the magic number of `statfs(2)`.
///
/// Used only when the mount table cannot be read; the magic number does
/// not distinguish `ext2`, `ext3` and `ext4`, which are reported as
/// `ext4`.
///
/// # Examples
///
/// ```
/// use nurt_fs::linux::fs_type_from_magic;
///
/// assert_eq!(fs_type_from_magic(0x0102_1994), Some("tmpfs"));
/// assert_eq!(fs_type_from_magic(1), None);
/// ```
#[must_use]
pub fn fs_type_from_magic(magic: u64) -> Option<&'static str> {
    MAGIC_TYPES
        .iter()
        .find(|(value, _)| *value == magic)
        .map(|(_, name)| *name)
}

fn is_network(fs_type: &str, source: &[u8]) -> bool {
    if NETWORK_TYPES.contains(&fs_type) {
        return true;
    }
    match fs_type.strip_prefix("fuse.") {
        Some(subtype) => REMOTE_FUSE_SUBTYPES.contains(&subtype) || is_remote_source(source),
        None => false,
    }
}

/// Whether a mount source names another machine: `//server/share`,
/// `host:/path` or `user@host:path`. Local sources start with `/` or carry
/// no colon.
fn is_remote_source(source: &[u8]) -> bool {
    if source.starts_with(b"//") {
        return true;
    }
    if source.first() == Some(&b'/') {
        return false;
    }
    let host = source
        .iter()
        .position(|&byte| byte == b':')
        .map(|colon| &source[..colon]);
    host.is_some_and(|host| !host.is_empty() && !host.contains(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_network_file_systems() {
        for fs_type in ["nfs", "nfs4", "cifs", "smb3", "smbfs", "sshfs", "9p", "afs"] {
            assert_eq!(
                volume_kind(fs_type, b"server:/x", false),
                VolumeKind::Network,
                "{fs_type}"
            );
        }
    }

    #[test]
    fn classifies_fuse_mounts_by_subtype_and_source() {
        assert_eq!(volume_kind("fuse.sshfs", b"x", false), VolumeKind::Network);
        assert_eq!(
            volume_kind("fuse.rclone", b"drive:", false),
            VolumeKind::Network
        );
        assert_eq!(
            volume_kind("fuse.unknownfs", b"user@host:dir", false),
            VolumeKind::Network
        );
        assert_eq!(
            volume_kind("fuse.unknownfs", b"//server/share", false),
            VolumeKind::Network
        );
        assert_eq!(
            volume_kind("fuse.unknownfs", b"/dev/sdc1", false),
            VolumeKind::Fixed
        );
        assert_eq!(
            volume_kind("fuse.unknownfs", b"local", false),
            VolumeKind::Fixed
        );
        assert_eq!(
            volume_kind("fuse.unknownfs", b":x", false),
            VolumeKind::Fixed
        );
        assert_eq!(
            volume_kind("fuseblk", b"/dev/sdc1", true),
            VolumeKind::Removable
        );
        assert_eq!(
            volume_kind("fuse.lxcfs", b"lxcfs", false),
            VolumeKind::Other
        );
    }

    #[test]
    fn classifies_memory_overlay_and_pseudo_file_systems() {
        for fs_type in ["tmpfs", "ramfs", "devtmpfs"] {
            assert_eq!(volume_kind(fs_type, b"none", false), VolumeKind::Memory);
        }
        assert_eq!(
            volume_kind("overlay", b"overlay", false),
            VolumeKind::Overlay
        );
        assert_eq!(
            volume_kind("fuse.fuse-overlayfs", b"fuse-overlayfs", false),
            VolumeKind::Overlay
        );
        for fs_type in ["proc", "sysfs", "cgroup2", "devpts"] {
            assert_eq!(volume_kind(fs_type, b"none", false), VolumeKind::Other);
        }
    }

    #[test]
    fn classifies_disks_by_removable_flag() {
        assert_eq!(volume_kind("ext4", b"/dev/sda1", false), VolumeKind::Fixed);
        assert_eq!(volume_kind("xfs", b"/dev/sda2", false), VolumeKind::Fixed);
        assert_eq!(
            volume_kind("btrfs", b"/dev/nvme0n1p2", false),
            VolumeKind::Fixed
        );
        assert_eq!(
            volume_kind("vfat", b"/dev/sdb1", true),
            VolumeKind::Removable
        );
        assert_eq!(volume_kind("tmpfs", b"tmpfs", true), VolumeKind::Memory);
    }

    #[test]
    fn describes_capabilities_of_local_disk_file_systems() {
        let ext4 = features_for("ext4", VolumeKind::Fixed, &[]);
        assert_eq!(
            ext4,
            FsFeatures {
                exchange: true,
                rename_noreplace: true,
                replace_with_backup: false,
                hardlink: true,
                reflink: false,
                xattr: true,
                acl: true,
                alternate_streams: false,
                durable_metadata: true,
                case_sensitive: true,
                decomposes_names: false,
            }
        );
        let btrfs = features_for("btrfs", VolumeKind::Fixed, &[]);
        assert!(btrfs.reflink && btrfs.exchange);
        let xfs = features_for("xfs", VolumeKind::Fixed, &[]);
        assert!(!xfs.reflink && xfs.exchange && xfs.acl);
        let ext2 = features_for("ext2", VolumeKind::Fixed, &[]);
        assert!(!ext2.exchange && ext2.rename_noreplace);
    }

    #[test]
    fn reads_acl_and_case_options_from_superblock() {
        let options = |items: &[&str]| -> Vec<Vec<u8>> {
            items.iter().map(|item| item.as_bytes().to_vec()).collect()
        };
        assert!(!features_for("ext4", VolumeKind::Fixed, &options(&["noacl"])).acl);
        assert!(!features_for("zfs", VolumeKind::Fixed, &options(&["xattr"])).acl);
        let zfs = features_for(
            "zfs",
            VolumeKind::Fixed,
            &options(&["xattr", "posixacl", "casesensitive"]),
        );
        assert!(zfs.acl && zfs.xattr && zfs.case_sensitive);
        let mixed = features_for(
            "zfs",
            VolumeKind::Fixed,
            &options(&["noxattr", "casemixed"]),
        );
        assert!(!mixed.xattr && !mixed.case_sensitive);
        assert!(
            !features_for("zfs", VolumeKind::Fixed, &options(&["caseinsensitive"])).case_sensitive
        );
        assert!(features_for("ntfs3", VolumeKind::Fixed, &options(&["acl"])).acl);
    }

    #[test]
    fn describes_fat_network_memory_and_overlay_volumes() {
        let fat = features_for("exfat", VolumeKind::Removable, &[]);
        assert!(!fat.hardlink && !fat.xattr && !fat.acl && !fat.durable_metadata);
        assert!(!fat.case_sensitive && !fat.exchange && fat.rename_noreplace);
        let nfs = features_for("nfs4", VolumeKind::Network, &[]);
        assert!(!nfs.durable_metadata && !nfs.xattr && !nfs.exchange && nfs.hardlink);
        let tmpfs = features_for("tmpfs", VolumeKind::Memory, &[]);
        assert!(tmpfs.exchange && tmpfs.xattr && tmpfs.acl && tmpfs.durable_metadata);
        let overlay = features_for("overlay", VolumeKind::Overlay, &[]);
        assert!(overlay.exchange && overlay.rename_noreplace && !overlay.reflink);
        let proc = features_for("proc", VolumeKind::Other, &[]);
        assert!(!proc.hardlink && !proc.xattr);
        let iso = features_for("iso9660", VolumeKind::Removable, &[]);
        assert!(!iso.hardlink && !iso.xattr);
    }

    #[test]
    fn never_reports_windows_or_hfs_capabilities() {
        for fs_type in ["ext4", "btrfs", "vfat", "nfs4", "overlay"] {
            let features = features_for(fs_type, VolumeKind::Fixed, &[]);
            assert!(!features.replace_with_backup && !features.alternate_streams);
            assert!(!features.decomposes_names);
        }
    }

    #[test]
    fn names_file_systems_by_magic_number() {
        assert_eq!(fs_type_from_magic(0xEF53), Some("ext4"));
        assert_eq!(fs_type_from_magic(0x794C_7630), Some("overlay"));
        assert_eq!(fs_type_from_magic(0x9123_683E), Some("btrfs"));
        assert_eq!(fs_type_from_magic(0), None);
        for (magic, name) in MAGIC_TYPES {
            assert_eq!(fs_type_from_magic(*magic), Some(*name));
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
