// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::sync::Arc;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::fs::VolumeKind;
use nurt_contracts::record::{Disk, DiskType};
use nurt_contracts::report::Notable;
use nurt_contracts::stage::{ArgValue, BoundArgs, CancelToken, Demand, ExecCtx};
use nurt_contracts::value::Value;

use super::classify::{MEMORY, NETWORK, PSEUDO, case_sensitive, classify, is_pseudo};
use super::mountinfo::{parse, unescape};
use super::{COMMAND, Disks, Volume, VolumeList, VolumeSource, disk_type};
use crate::processes::testing::{SystemClock, VirtualClock, context, run};

/// Volume list held in memory.
struct FakeVolumes(Vec<Volume>, u64);

impl VolumeSource for FakeVolumes {
    fn volumes(&self) -> Result<VolumeList> {
        Ok(VolumeList {
            volumes: self.0.clone(),
            unreadable: self.1,
        })
    }
}

fn volume(mount: &str, filesystem: &str, kind: VolumeKind) -> Volume {
    Volume {
        mount: mount.to_owned(),
        device: format!("/dev/{}", mount.trim_start_matches('/')),
        filesystem: filesystem.to_owned(),
        kind: Some(kind),
        total: 1000,
        free: 400,
        used: 500,
        case_sensitive: true,
        ..Volume::default()
    }
}

fn ctx() -> ExecCtx {
    context(VirtualClock::at(0), Demand::default(), CancelToken::new())
}

fn disks(source: FakeVolumes, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Disk>> {
    let stage = Disks::with_source(Arc::new(source));
    Ok(run(&stage, args, ctx)?
        .into_iter()
        .map(|record| Disk::try_from(record).unwrap())
        .collect())
}

fn of_type(kind: &str) -> BoundArgs {
    BoundArgs::new(COMMAND).with("type", ArgValue::Value(Value::Enum(kind.to_owned())), false)
}

#[test]
fn disks_mountinfo_octal_escapes() {
    assert_eq!(unescape(r"/media/My\040Disk"), "/media/My Disk");
    assert_eq!(unescape(r"a\011b\134c\012"), "a\tb\\c\n");
    assert_eq!(unescape(r"keep\09x\"), r"keep\09x\", "incomplete escapes are kept");
    let text = "36 25 8:17 / /media/My\\040Disk rw,nosuid,relatime shared:5 - vfat /dev/sdb1 \
                rw,fmask=0022\n\
                22 1 0:21 / /proc rw,nosuid - proc proc rw\n\
                broken line\n";
    let mounts = parse(text);
    assert_eq!(mounts.len(), 2);
    assert_eq!(mounts[0].mount_point, "/media/My Disk");
    assert_eq!(mounts[0].device_number, "8:17");
    assert_eq!(mounts[0].options, ["rw", "nosuid", "relatime"]);
    assert_eq!(mounts[0].fs_type, "vfat");
    assert_eq!(mounts[0].source, "/dev/sdb1");
    assert_eq!(mounts[0].super_options, ["rw", "fmask=0022"]);
}

#[test]
fn disks_pseudo_filesystems_skipped() {
    for fs_type in ["proc", "sysfs", "cgroup2", "devpts", "bpf", "autofs", "binfmt_misc"] {
        assert!(is_pseudo(fs_type), "{fs_type}");
    }
    for fs_type in ["ext4", "tmpfs", "overlay", "nfs4"] {
        assert!(!is_pseudo(fs_type), "{fs_type}");
    }
    assert!(PSEUDO.iter().all(|fs| !NETWORK.contains(fs) && !MEMORY.contains(fs)));
}

#[test]
fn disks_overlay_classified_fixed() {
    assert_eq!(classify("overlay", false), VolumeKind::Overlay);
    assert_eq!(disk_type(VolumeKind::Overlay), DiskType::Fixed);
    let records = disks(
        FakeVolumes(vec![volume("/", "overlay", VolumeKind::Overlay)], 0),
        &BoundArgs::new(COMMAND),
        &ctx(),
    )
    .unwrap();
    assert_eq!(records[0].kind, DiskType::Fixed);
    assert_eq!(records[0].filesystem, "overlay");
}

#[test]
fn disks_tmpfs_is_ram() {
    for fs_type in ["tmpfs", "ramfs", "devtmpfs"] {
        assert_eq!(disk_type(classify(fs_type, false)), DiskType::Ram, "{fs_type}");
    }
}

#[test]
fn disks_network_types_from_f76_list() {
    for fs_type in [
        "nfs", "nfs4", "cifs", "smb3", "sshfs", "fuse.sshfs", "9p", "afs", "smbfs", "afpfs",
        "webdav",
    ] {
        assert_eq!(classify(fs_type, true), VolumeKind::Network, "{fs_type}");
    }
    assert_eq!(classify("fuse.portal", false), VolumeKind::Other);
    assert_eq!(classify("ext4", true), VolumeKind::Removable);
    assert_eq!(classify("btrfs", false), VolumeKind::Fixed);
    assert!(!case_sensitive("vfat"));
    assert!(case_sensitive("ext4"));
}

#[test]
fn disks_type_filter_removable() {
    let source = || {
        FakeVolumes(
            vec![
                volume("/", "ext4", VolumeKind::Fixed),
                volume("/media/usb", "vfat", VolumeKind::Removable),
                volume("/mnt/share", "nfs4", VolumeKind::Network),
                volume("/run", "tmpfs", VolumeKind::Memory),
            ],
            0,
        )
    };
    let mounts = |kind: &str| -> Vec<String> {
        disks(source(), &of_type(kind), &ctx())
            .unwrap()
            .into_iter()
            .map(|disk| disk.mount.into_string())
            .collect()
    };
    assert_eq!(mounts("removable"), ["/media/usb"]);
    assert_eq!(mounts("network"), ["/mnt/share"]);
    assert_eq!(mounts("fixed"), ["/"]);
    assert_eq!(mounts("all"), ["/", "/media/usb", "/mnt/share", "/run"]);
    let error = disks(source(), &of_type("cdrom"), &ctx()).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn disks_maps_volume_info_from_contract() {
    let cases = [
        (VolumeKind::Fixed, DiskType::Fixed),
        (VolumeKind::Removable, DiskType::Removable),
        (VolumeKind::Network, DiskType::Network),
        (VolumeKind::Memory, DiskType::Ram),
        (VolumeKind::Overlay, DiskType::Fixed),
        (VolumeKind::Other, DiskType::Other),
    ];
    for (kind, expected) in cases {
        assert_eq!(disk_type(kind), expected, "{kind:?}");
    }
}

#[test]
fn disks_unreadable_volumes_noted() {
    let ctx = ctx();
    let records = disks(
        FakeVolumes(vec![volume("/", "ext4", VolumeKind::Fixed)], 2),
        &BoundArgs::new(COMMAND),
        &ctx,
    )
    .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].used.bytes(), 500);
    let notes = ctx.take_notes();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].code, Notable::SourceUnavailable);
    assert_eq!(notes[0].count, Some(2));
}

#[cfg(unix)]
mod mount_table {
    use super::*;
    use crate::disks::linux::{MountTable, decode_label};
    use crate::processes::sysroot::SysRoot;
    use crate::processes::testing::write;

    #[test]
    fn disks_reads_synthetic_mount_table() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "/proc/self/mountinfo",
            "21 1 8:1 / / rw,relatime - ext4 /dev/sda1 rw,errors=remount-ro\n\
             22 21 0:5 / /proc rw - proc proc rw\n\
             23 21 0:25 / /run rw,nosuid - tmpfs tmpfs rw,size=100k\n\
             24 21 8:17 / /media/USB\\040KEY ro,nosuid - vfat /dev/sdb1 rw\n\
             25 21 0:40 / /mnt/gone rw - nfs4 server:/export rw\n",
        );
        for dir in ["run", "media/USB KEY"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        write(root, "/sys/devices/block/sdb/removable", "1\n");
        std::fs::create_dir_all(root.join("sys/devices/block/sdb/sdb1")).unwrap();
        std::fs::create_dir_all(root.join("sys/dev/block")).unwrap();
        std::os::unix::fs::symlink(
            root.join("sys/devices/block/sdb/sdb1"),
            root.join("sys/dev/block/8:17"),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("dev/disk/by-label")).unwrap();
        std::os::unix::fs::symlink("../../sdb1", root.join("dev/disk/by-label/USB\\x20KEY"))
            .unwrap();
        let stage = Disks::with_source(Arc::new(MountTable::new(SysRoot::new(root))));
        let ctx = ctx();
        let records: Vec<Disk> = run(&stage, &BoundArgs::new(COMMAND), &ctx)
            .unwrap()
            .into_iter()
            .map(|record| Disk::try_from(record).unwrap())
            .collect();
        let mounts: Vec<&str> = records.iter().map(|disk| disk.mount.as_str()).collect();
        assert_eq!(mounts, ["/", "/media/USB KEY", "/run"]);
        let usb = &records[1];
        assert_eq!(usb.kind, DiskType::Removable);
        assert!(usb.readonly);
        assert!(!usb.case_sensitive);
        assert_eq!(usb.label.as_deref(), Some("USB KEY"));
        assert_eq!(records[0].options.as_deref().unwrap(), ["rw", "relatime", "errors=remount-ro"]);
        assert!(records[0].total.bytes() > 0);
        assert_eq!(records[2].kind, DiskType::Ram);
        let notes = ctx.take_notes();
        assert_eq!(notes[0].count, Some(1), "the missing network mount point is counted");
    }

    #[test]
    fn disks_udev_label_escapes() {
        assert_eq!(decode_label(r"My\x20Disk\x2fA"), "My Disk/A");
        assert_eq!(decode_label(r"plain\x2"), r"plain\x2");
    }
}

#[test]
#[ignore = "requires the mounted volumes of the running system"]
fn it_disks_contains_root_volume() {
    let ctx = context(SystemClock::new(), Demand::default(), CancelToken::new());
    let records: Vec<Disk> = run(&Disks::new(), &BoundArgs::new(COMMAND), &ctx)
        .unwrap()
        .into_iter()
        .map(|record| Disk::try_from(record).unwrap())
        .collect();
    let root = if cfg!(windows) {
        std::env::var("SystemDrive").map_or_else(|_| "C:/".to_owned(), |drive| format!("{drive}/"))
    } else {
        "/".to_owned()
    };
    let volume = records
        .iter()
        .find(|disk| disk.mount.as_str() == root)
        .unwrap_or_else(|| panic!("{root} is not listed"));
    assert!(volume.total.bytes() > 0);
}
