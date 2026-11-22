// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Volume eligibility rules.
//!
//! The journal is kept by default on local fixed volumes and on overlay
//! volumes. Network, removable and memory-backed volumes and file systems
//! without durable metadata are excluded unless configuration enables them.

use std::path::PathBuf;

use nurt_contracts::config::JournalConfig;
use nurt_contracts::fs::{AbsPath, VolumeInfo, VolumeKind};
use nurt_contracts::journal::{Eligibility, InactiveReason};

/// File system types of network volumes (Linux and macOS names).
const NETWORK_TYPES: &[&str] = &[
    "nfs", "nfs4", "cifs", "smb3", "smbfs", "sshfs", "9p", "afs", "afpfs", "webdav",
];

/// FUSE file systems that serve remote storage.
const REMOTE_FUSE_TYPES: &[&str] = &["fuse.sshfs", "fuse.rclone", "fuse.s3fs", "fuse.gcsfuse"];

/// Memory-backed file system types.
const MEMORY_TYPES: &[&str] = &["tmpfs", "ramfs", "devtmpfs"];

/// File system types without durable renames and metadata.
const NOT_DURABLE_TYPES: &[&str] = &["vfat", "msdos", "fat", "exfat"];

/// Overlay file system types.
const OVERLAY_TYPES: &[&str] = &["overlay", "overlayfs"];

/// Class of a volume for the eligibility rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VolumeClass {
    Network,
    Removable,
    Memory,
    Overlay,
    Local,
}

/// Class of a volume: the kind reported by the back end decides first, the
/// file system type name second.
fn classify(volume: &VolumeInfo) -> VolumeClass {
    match volume.kind {
        VolumeKind::Network => return VolumeClass::Network,
        VolumeKind::Removable => return VolumeClass::Removable,
        VolumeKind::Memory => return VolumeClass::Memory,
        VolumeKind::Overlay => return VolumeClass::Overlay,
        _ => {}
    }
    let fs_type = volume.fs_type.to_ascii_lowercase();
    let fs_type = fs_type.as_str();
    if NETWORK_TYPES.contains(&fs_type) || REMOTE_FUSE_TYPES.contains(&fs_type) {
        VolumeClass::Network
    } else if MEMORY_TYPES.contains(&fs_type) {
        VolumeClass::Memory
    } else if OVERLAY_TYPES.contains(&fs_type) {
        VolumeClass::Overlay
    } else {
        VolumeClass::Local
    }
}

/// Whether the journal is kept on a volume, and why not.
///
/// Rules, in order:
///
/// 1. `journal.volume."<mount point>".enabled` decides when set, in both
///    directions; an explicitly enabled volume is eligible whatever its class.
/// 2. `journal.enabled = false` disables the journal ([`InactiveReason::Disabled`]).
/// 3. A read-only volume has no writable anchor ([`InactiveReason::NoAnchor`]).
/// 4. Network volumes need `journal.volumes.network`, removable volumes need
///    `journal.volumes.removable`; memory-backed volumes are excluded.
/// 5. A file system without durable metadata (FAT, exFAT, some network
///    shares) is excluded ([`InactiveReason::NotDurable`]).
///
/// Overlay volumes are eligible: the journal directory lives in the upper
/// layer and entries of the lower layer are preserved by copy.
///
/// # Examples
///
/// ```
/// use nurt_contracts::config::JournalConfig;
/// use nurt_contracts::fs::{AbsPath, FsFeatures, VolumeId, VolumeInfo, VolumeKind};
/// use nurt_contracts::journal::{Eligibility, InactiveReason};
/// use nurt_journal::volumes::evaluate_eligibility;
///
/// let volume = VolumeInfo {
///     id: VolumeId { dev: 41, uuid: None, subvolume: 0 },
///     mount: AbsPath::new("/tmp").unwrap(),
///     fs_type: "tmpfs".to_owned(),
///     kind: VolumeKind::Memory,
///     features: FsFeatures { durable_metadata: true, ..FsFeatures::default() },
///     rotational: false,
///     read_only: false,
/// };
/// let config = JournalConfig::default();
/// assert_eq!(
///     evaluate_eligibility(&volume, &config),
///     Eligibility::Inactive(InactiveReason::MemoryVolume)
/// );
/// ```
pub fn evaluate(volume: &VolumeInfo, config: &JournalConfig) -> Eligibility {
    match volume_override(volume, config) {
        Some(false) => return Eligibility::Inactive(InactiveReason::Disabled),
        Some(true) => return writability(volume),
        None => {}
    }
    if !config.enabled {
        return Eligibility::Inactive(InactiveReason::Disabled);
    }
    if volume.read_only {
        return Eligibility::Inactive(InactiveReason::NoAnchor);
    }
    match classify(volume) {
        VolumeClass::Network if !config.volumes.network => {
            Eligibility::Inactive(InactiveReason::NetworkVolume)
        }
        VolumeClass::Removable if !config.volumes.removable => {
            Eligibility::Inactive(InactiveReason::RemovableVolume)
        }
        VolumeClass::Memory => Eligibility::Inactive(InactiveReason::MemoryVolume),
        VolumeClass::Network
        | VolumeClass::Removable
        | VolumeClass::Overlay
        | VolumeClass::Local => durability(volume),
    }
}

fn writability(volume: &VolumeInfo) -> Eligibility {
    if volume.read_only {
        Eligibility::Inactive(InactiveReason::NoAnchor)
    } else {
        Eligibility::Eligible
    }
}

fn durability(volume: &VolumeInfo) -> Eligibility {
    let fs_type = volume.fs_type.to_ascii_lowercase();
    if !volume.features.durable_metadata || NOT_DURABLE_TYPES.contains(&fs_type.as_str()) {
        Eligibility::Inactive(InactiveReason::NotDurable)
    } else {
        Eligibility::Eligible
    }
}

/// Per-volume `enabled` setting whose key names the mount point of the volume.
fn volume_override(volume: &VolumeInfo, config: &JournalConfig) -> Option<bool> {
    let mount = volume.mount.to_language_string();
    config
        .volume
        .iter()
        .find(|(key, _)| names_mount(key, &mount))
        .and_then(|(_, settings)| settings.enabled)
}

/// Whether a configuration key names a mount point; keys are compared in
/// language form, so trailing separators and `\` versus `/` do not matter.
fn names_mount(key: &str, mount: &str) -> bool {
    AbsPath::new(PathBuf::from(key)).is_ok_and(|path| path.to_language_string() == mount)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::{JournalConfig, VolumeOverride};
    use nurt_contracts::fs::{VolumeInfo, VolumeKind};
    use nurt_contracts::journal::{Eligibility, InactiveReason};

    use super::evaluate;
    use crate::volumes::testfs::volume_info;

    fn inactive(reason: InactiveReason) -> Eligibility {
        Eligibility::Inactive(reason)
    }

    fn with_override(mount: &str, enabled: bool) -> JournalConfig {
        let mut config = JournalConfig::default();
        let mut settings = VolumeOverride::default();
        settings.enabled = Some(enabled);
        config.volume.insert(mount.to_owned(), settings);
        config
    }

    fn typed(fs_type: &str) -> VolumeInfo {
        volume_info(7, "/mnt/data", fs_type, VolumeKind::Other)
    }

    #[test]
    fn applies_default_rules_to_every_volume_class() {
        let config = JournalConfig::default();
        let cases = [
            (VolumeKind::Fixed, "ext4", Eligibility::Eligible),
            (VolumeKind::Overlay, "overlay", Eligibility::Eligible),
            (
                VolumeKind::Network,
                "nfs4",
                inactive(InactiveReason::NetworkVolume),
            ),
            (
                VolumeKind::Removable,
                "exfat",
                inactive(InactiveReason::RemovableVolume),
            ),
            (
                VolumeKind::Memory,
                "tmpfs",
                inactive(InactiveReason::MemoryVolume),
            ),
        ];
        for (kind, fs_type, expected) in cases {
            let volume = volume_info(1, "/mnt/data", fs_type, kind);
            assert_eq!(evaluate(&volume, &config), expected, "{fs_type}");
        }
    }

    #[test]
    fn recognises_classes_by_file_system_type_name() {
        let config = JournalConfig::default();
        for fs_type in ["nfs", "cifs", "smb3", "sshfs", "fuse.sshfs", "9p", "afs"] {
            assert_eq!(
                evaluate(&typed(fs_type), &config),
                inactive(InactiveReason::NetworkVolume),
                "{fs_type}"
            );
        }
        for fs_type in ["tmpfs", "ramfs", "devtmpfs"] {
            assert_eq!(
                evaluate(&typed(fs_type), &config),
                inactive(InactiveReason::MemoryVolume),
                "{fs_type}"
            );
        }
        for fs_type in ["vfat", "exFAT"] {
            assert_eq!(
                evaluate(&typed(fs_type), &config),
                inactive(InactiveReason::NotDurable),
                "{fs_type}"
            );
        }
        assert_eq!(evaluate(&typed("overlay"), &config), Eligibility::Eligible);
        assert_eq!(evaluate(&typed("btrfs"), &config), Eligibility::Eligible);
    }

    #[test]
    fn excludes_volume_without_durable_metadata() {
        let mut volume = volume_info(1, "/mnt/share", "ntfs3", VolumeKind::Fixed);
        volume.features.durable_metadata = false;
        assert_eq!(
            evaluate(&volume, &JournalConfig::default()),
            inactive(InactiveReason::NotDurable)
        );
    }

    #[test]
    fn excludes_read_only_volume_as_without_anchor() {
        let mut volume = volume_info(1, "/mnt/ro", "ext4", VolumeKind::Fixed);
        volume.read_only = true;
        assert_eq!(
            evaluate(&volume, &JournalConfig::default()),
            inactive(InactiveReason::NoAnchor)
        );
        assert_eq!(
            evaluate(&volume, &with_override("/mnt/ro", true)),
            inactive(InactiveReason::NoAnchor)
        );
    }

    #[test]
    fn enables_network_and_removable_volumes_by_class_setting() {
        let mut config = JournalConfig::default();
        config.volumes.network = true;
        config.volumes.removable = true;
        let network = volume_info(1, "/mnt/nfs", "nfs", VolumeKind::Network);
        let removable = volume_info(2, "/media/usb", "ext4", VolumeKind::Removable);
        assert_eq!(evaluate(&network, &config), Eligibility::Eligible);
        assert_eq!(evaluate(&removable, &config), Eligibility::Eligible);

        let mut share = network;
        share.features.durable_metadata = false;
        assert_eq!(
            evaluate(&share, &config),
            inactive(InactiveReason::NotDurable)
        );
    }

    #[test]
    fn disables_journal_by_global_setting() {
        let mut config = JournalConfig::default();
        config.enabled = false;
        let volume = volume_info(1, "/home", "ext4", VolumeKind::Fixed);
        assert_eq!(
            evaluate(&volume, &config),
            inactive(InactiveReason::Disabled)
        );
    }

    #[test]
    fn per_volume_setting_overrides_class_and_global_setting() {
        let memory = volume_info(1, "/mnt/scratch", "tmpfs", VolumeKind::Memory);
        assert_eq!(
            evaluate(&memory, &with_override("/mnt/scratch/", true)),
            Eligibility::Eligible
        );

        let mut config = with_override("/home", true);
        config.enabled = false;
        let home = volume_info(2, "/home", "ext4", VolumeKind::Fixed);
        assert_eq!(evaluate(&home, &config), Eligibility::Eligible);

        let data = volume_info(3, "/mnt/data", "ext4", VolumeKind::Fixed);
        assert_eq!(
            evaluate(&data, &with_override("/mnt/data", false)),
            inactive(InactiveReason::Disabled)
        );
        assert_eq!(
            evaluate(&data, &with_override("relative/path", false)),
            Eligibility::Eligible
        );
    }
}
