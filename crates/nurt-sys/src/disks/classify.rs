// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of mounted file systems by type name.
//!
//! The lists follow the journal space rules: network file systems and file
//! systems in memory are classified by type; the overlay file system of
//! containers is a class of its own that counts as a fixed disk.

use nurt_contracts::fs::VolumeKind;

/// Pseudo file systems of the kernel, which are not volumes.
pub(crate) const PSEUDO: &[&str] = &[
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
    "bpf",
    "configfs",
    "fusectl",
    "binfmt_misc",
    "autofs",
    "devfs",
];

/// Network file systems of Linux and macOS.
pub(crate) const NETWORK: &[&str] = &[
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "sshfs",
    "fuse.sshfs",
    "9p",
    "afs",
    "afpfs",
    "webdav",
];

/// File systems in memory.
pub(crate) const MEMORY: &[&str] = &["tmpfs", "ramfs", "devtmpfs"];

/// Overlay file system.
const OVERLAY: &str = "overlay";

/// Prefix of file systems in user space; those not listed as network file
/// systems are of another class.
const FUSE_PREFIX: &str = "fuse.";

/// File systems whose names do not differ by case.
const CASE_INSENSITIVE: &[&str] = &[
    "vfat", "msdos", "exfat", "ntfs", "ntfs3", "cifs", "smb3", "smbfs", "hfsplus",
];

/// Returns `true` for a pseudo file system of the kernel.
pub(crate) fn is_pseudo(fs_type: &str) -> bool {
    PSEUDO.contains(&fs_type)
}

/// Class of a volume with file system `fs_type` on a medium that is
/// removable or not.
pub(crate) fn classify(fs_type: &str, removable: bool) -> VolumeKind {
    if NETWORK.contains(&fs_type) {
        VolumeKind::Network
    } else if MEMORY.contains(&fs_type) {
        VolumeKind::Memory
    } else if fs_type == OVERLAY {
        VolumeKind::Overlay
    } else if fs_type.starts_with(FUSE_PREFIX) {
        VolumeKind::Other
    } else if removable {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
    }
}

/// Returns `true` when names on a file system of type `fs_type` differ by
/// case.
pub(crate) fn case_sensitive(fs_type: &str) -> bool {
    !CASE_INSENSITIVE.contains(&fs_type)
}
