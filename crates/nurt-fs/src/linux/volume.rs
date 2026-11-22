// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Volume identity, class, capabilities and space figures.
//!
//! The mount of a path is found through `stx_mnt_id` of `statx` in the
//! mount table of the process, or by device number and the longest mount
//! point when the kernel does not report mount identifiers. Block device
//! attributes (`removable`, `queue/rotational`) are read from
//! `/sys/dev/block/<major>:<minor>`, falling back to the parent device for
//! partitions. Without block device data a volume is treated as fixed and
//! non-rotational.

use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fs;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nurt_contracts::fs::{
    AbsPath, FileKind, FsError, FsErrorKind, FsResult, SpaceInfo, VolumeId, VolumeInfo, VolumeKind,
};
use rustix::fs::{AtFlags, CWD, Mode, OFlags, StatVfsMountFlags, Statx, StatxFlags};
use rustix::io::Errno;

use super::meta::kind_from_mode;
use super::mountinfo::{MountEntry, MountTable, device_number};
use super::quota::{QuotaFigures, quota_figures};
use super::sys::{io_error, retry, sys_error};
use super::table::{features_for, fs_type_from_magic, volume_kind};

/// Mount table of the calling process.
const MOUNTINFO: &str = "/proc/self/mountinfo";
/// Directory of block devices by number.
const SYS_DEV_BLOCK: &str = "/sys/dev/block";
/// Links from file system UUIDs to block devices.
const DISK_BY_UUID: &str = "/dev/disk/by-uuid";
/// `STATX_SUBVOL` (Linux 6.10): subvolume identifier in `stx_subvol`.
const STATX_SUBVOL: StatxFlags = StatxFlags::from_bits_retain(0x8000);
/// Fields requested from `statx` to locate a volume.
const VOLUME_MASK: StatxFlags = StatxFlags::TYPE
    .union(StatxFlags::INO)
    .union(StatxFlags::MNT_ID)
    .union(STATX_SUBVOL);
/// Lowest object identifier of a btrfs subvolume (`BTRFS_FIRST_FREE_OBJECTID`).
const BTRFS_FIRST_FREE_OBJECTID: u64 = 256;
/// `XFS_FSOP_GEOM_FLAGS_REFLINK`: the XFS volume shares blocks.
const XFS_GEOM_FLAGS_REFLINK: u32 = 1 << 20;
/// Quota type of user quotas (`USRQUOTA`).
const USRQUOTA: i32 = 0;
/// Magic numbers of `statfs` are 32-bit values.
const MAGIC_MASK: u64 = 0xFFFF_FFFF;

/// Mount point and source of a volume returned by `volume`, used to answer
/// `space` for its identifier.
#[derive(Debug, Clone)]
pub(super) struct KnownVolume {
    mount: PathBuf,
    source: Vec<u8>,
    case_sensitive: bool,
}

/// Volumes described so far, by device number.
#[derive(Debug, Default)]
pub(super) struct VolumeCache {
    volumes: Mutex<HashMap<u64, KnownVolume>>,
}

impl VolumeCache {
    fn remember(&self, dev: u64, volume: KnownVolume) {
        if let Ok(mut volumes) = self.volumes.lock() {
            volumes.insert(dev, volume);
        }
    }

    fn known(&self, dev: u64) -> Option<KnownVolume> {
        self.volumes.lock().ok()?.get(&dev).cloned()
    }
}

/// Nearest existing entry of `path` and its `statx` data. The last
/// component is not followed; ancestors are, because entries created below
/// a symbolic link to a directory land on the volume of its target.
fn nearest_existing(path: &Path) -> FsResult<(PathBuf, Statx)> {
    let mut current = path.to_path_buf();
    let mut flags = AtFlags::SYMLINK_NOFOLLOW;
    loop {
        match retry(|| rustix::fs::statx(CWD, &current, flags, VOLUME_MASK)) {
            Ok(stat) => return Ok((current, stat)),
            Err(Errno::NOENT | Errno::NOTDIR) => {
                if !current.pop() {
                    return Err(FsError::new(FsErrorKind::NotFound, "volume", Some(path)));
                }
                flags = AtFlags::empty();
            }
            Err(errno) => return Err(sys_error(errno, "volume", path)),
        }
    }
}

fn read_mount_table() -> Option<MountTable> {
    let text = fs::read(MOUNTINFO).ok()?;
    MountTable::parse(&text).ok()
}

/// Mount that contains `path` (the nearest existing ancestor decides).
pub(super) fn mount_entry(path: &Path) -> FsResult<MountEntry> {
    let (existing, stat) = nearest_existing(path)?;
    let table = read_mount_table()
        .ok_or_else(|| FsError::new(FsErrorKind::Unsupported, "mount_entry", Some(path)))?;
    find_mount(&table, &existing, &stat)
        .cloned()
        .ok_or_else(|| FsError::new(FsErrorKind::NotFound, "mount_entry", Some(path)))
}

fn find_mount<'a>(table: &'a MountTable, existing: &Path, stat: &Statx) -> Option<&'a MountEntry> {
    let mask = StatxFlags::from_bits_retain(stat.stx_mask);
    if mask.contains(StatxFlags::MNT_ID)
        && let Some(entry) = table.by_mount_id(stat.stx_mnt_id)
    {
        return Some(entry);
    }
    let real = fs::canonicalize(existing).ok()?;
    table.containing(
        real.as_os_str().as_bytes(),
        Some((stat.stx_dev_major, stat.stx_dev_minor)),
    )
}

/// Description of the volume that contains `path`.
pub(super) fn volume(cache: &VolumeCache, path: &Path) -> FsResult<VolumeInfo> {
    let (existing, stat) = nearest_existing(path)?;
    let dev = device_number(stat.stx_dev_major, stat.stx_dev_minor);
    let table = read_mount_table();
    let entry = table
        .as_ref()
        .and_then(|table| find_mount(table, &existing, &stat));
    let described = match entry {
        Some(entry) => Described::from_entry(entry),
        None => Described::from_statfs(&existing, dev)?,
    };
    let statvfs = retry(|| rustix::fs::statvfs(&existing))
        .map_err(|errno| sys_error(errno, "volume", path))?;
    let block = block_device(stat.stx_dev_major, stat.stx_dev_minor, &described.source);
    let removable = block.is_some_and(|device| sysfs_flag(device, "removable"));
    let kind = volume_kind(&described.fs_type, &described.source, removable);
    let opened = open_any(&existing, kind_from_mode(u32::from(stat.stx_mode)));
    let mut features = features_for(&described.fs_type, kind, &described.super_options);
    if described.fs_type == "xfs" {
        features.reflink = opened.as_ref().is_some_and(|fd| xfs_reflink(fd.as_fd()));
    }
    let subvolume = if described.fs_type == "btrfs" {
        btrfs_subvolume(&stat, opened.as_ref().map(AsFd::as_fd))
    } else {
        0
    };
    let rotational = match kind {
        VolumeKind::Fixed | VolumeKind::Removable => {
            block.is_some_and(|device| sysfs_flag(device, "queue/rotational"))
        }
        _ => false,
    };
    let mount = AbsPath::new(&described.mount)?;
    cache.remember(
        dev,
        KnownVolume {
            mount: described.mount.clone(),
            source: described.source.clone(),
            case_sensitive: features.case_sensitive,
        },
    );
    Ok(VolumeInfo {
        id: VolumeId {
            dev,
            uuid: block.and_then(filesystem_uuid),
            subvolume,
        },
        mount,
        fs_type: described.fs_type,
        kind,
        features,
        rotational,
        read_only: described.read_only || statvfs.f_flag.contains(StatVfsMountFlags::RDONLY),
    })
}

/// Mount data of a volume, from the mount table or, without one, from
/// `statfs`.
struct Described {
    mount: PathBuf,
    fs_type: String,
    source: Vec<u8>,
    super_options: Vec<Vec<u8>>,
    read_only: bool,
}

impl Described {
    fn from_entry(entry: &MountEntry) -> Self {
        Self {
            mount: PathBuf::from(OsStr::from_bytes(&entry.mount_point)),
            fs_type: entry.fs_type.clone(),
            source: entry.source.clone(),
            super_options: entry.super_options.clone(),
            read_only: entry.is_read_only(),
        }
    }

    /// Mount point found by walking up while the device stays the same.
    fn from_statfs(existing: &Path, dev: u64) -> FsResult<Self> {
        let statfs = retry(|| rustix::fs::statfs(existing))
            .map_err(|errno| sys_error(errno, "volume", existing))?;
        let magic = u64::try_from(i128::from(statfs.f_type) & i128::from(MAGIC_MASK)).unwrap_or(0);
        let real =
            fs::canonicalize(existing).map_err(|error| io_error(&error, "volume", existing))?;
        let mut mount = real.clone();
        for ancestor in real.ancestors().skip(1) {
            match fs::metadata(ancestor) {
                Ok(meta) if meta.dev() == dev => mount = ancestor.to_path_buf(),
                _ => break,
            }
        }
        Ok(Self {
            mount,
            fs_type: fs_type_from_magic(magic).unwrap_or("unknown").to_owned(),
            source: Vec::new(),
            super_options: Vec::new(),
            read_only: false,
        })
    }
}

/// Block device that holds the volume: the device of the file system, or,
/// for file systems with an anonymous device number (btrfs), the device
/// named by the mount source.
fn block_device(major: u32, minor: u32, source: &[u8]) -> Option<(u32, u32)> {
    if major != 0 {
        return Some((major, minor));
    }
    if !source.starts_with(b"/dev/") {
        return None;
    }
    let meta = fs::metadata(OsStr::from_bytes(source)).ok()?;
    meta.file_type().is_block_device().then(|| {
        let rdev = meta.rdev();
        (rustix::fs::major(rdev), rustix::fs::minor(rdev))
    })
}

/// Boolean attribute of a block device; partitions inherit the attribute
/// of their disk.
fn sysfs_flag((major, minor): (u32, u32), attribute: &str) -> bool {
    let device = PathBuf::from(format!("{SYS_DEV_BLOCK}/{major}:{minor}"));
    let value = fs::read_to_string(device.join(attribute))
        .or_else(|_| fs::read_to_string(device.join("..").join(attribute)));
    value.is_ok_and(|text| text.trim() == "1")
}

/// File system UUID from the links in `/dev/disk/by-uuid`.
fn filesystem_uuid((major, minor): (u32, u32)) -> Option<String> {
    let wanted = device_number(major, minor);
    fs::read_dir(DISK_BY_UUID).ok()?.flatten().find_map(|link| {
        let meta = fs::metadata(link.path()).ok()?;
        (meta.file_type().is_block_device() && meta.rdev() == wanted)
            .then(|| link.file_name().to_string_lossy().into_owned())
    })
}

/// Opens a regular file or directory for capability queries; special files
/// are never opened.
fn open_any(path: &Path, kind: FileKind) -> Option<OwnedFd> {
    let flags = match kind {
        FileKind::Directory => OFlags::DIRECTORY,
        FileKind::File => OFlags::empty(),
        _ => return None,
    };
    let flags = flags
        | OFlags::RDONLY
        | OFlags::CLOEXEC
        | OFlags::NONBLOCK
        | OFlags::NOCTTY
        | OFlags::NOFOLLOW;
    retry(|| rustix::fs::openat(CWD, path, flags, Mode::empty())).ok()
}

/// `struct xfs_fsop_geom_v1` of `xfs_fs.h`; only `flags` is read.
#[repr(C)]
struct XfsGeometryV1 {
    _sizes: [u32; 8],
    _blocks: [u64; 4],
    _uuid: [u8; 16],
    _stripe: [u32; 3],
    flags: u32,
    _sectors: [u32; 3],
}

const _: () = assert!(size_of::<XfsGeometryV1>() == 112);

/// `XFS_IOC_FSGEOMETRY_V1`.
const XFS_IOC_FSGEOMETRY_V1: rustix::ioctl::Opcode =
    rustix::ioctl::opcode::read::<XfsGeometryV1>(b'X', 100);

/// Whether an XFS volume was formatted with `reflink=1`, read from its
/// geometry without writing to the volume.
fn xfs_reflink(fd: BorrowedFd<'_>) -> bool {
    // SAFETY: `XFS_IOC_FSGEOMETRY_V1` writes exactly one
    // `struct xfs_fsop_geom_v1`, whose layout `XfsGeometryV1` reproduces
    // (112 bytes, checked at compile time); the descriptor is open.
    let geometry = unsafe {
        rustix::ioctl::ioctl(
            fd,
            rustix::ioctl::Getter::<XFS_IOC_FSGEOMETRY_V1, XfsGeometryV1>::new(),
        )
    };
    geometry.is_ok_and(|geometry| geometry.flags & XFS_GEOM_FLAGS_REFLINK != 0)
}

/// `struct btrfs_ioctl_ino_lookup_args` of `btrfs.h`.
#[repr(C)]
struct BtrfsInoLookup {
    tree_id: u64,
    object_id: u64,
    _name: [u8; 4080],
}

/// `BTRFS_IOC_INO_LOOKUP`.
const BTRFS_IOC_INO_LOOKUP: rustix::ioctl::Opcode =
    rustix::ioctl::opcode::read_write::<BtrfsInoLookup>(0x94, 18);

/// Identifier of the btrfs subvolume that contains an entry: `stx_subvol`
/// on kernels that report it, otherwise `BTRFS_IOC_INO_LOOKUP`, which
/// answers the subvolume of the descriptor without privileges.
fn btrfs_subvolume(stat: &Statx, fd: Option<BorrowedFd<'_>>) -> u64 {
    if StatxFlags::from_bits_retain(stat.stx_mask).contains(STATX_SUBVOL) {
        return stat.stx_subvol;
    }
    let Some(fd) = fd else { return 0 };
    let mut args = BtrfsInoLookup {
        tree_id: 0,
        object_id: BTRFS_FIRST_FREE_OBJECTID,
        _name: [0; 4080],
    };
    // SAFETY: `BTRFS_IOC_INO_LOOKUP` reads and writes one
    // `struct btrfs_ioctl_ino_lookup_args` (4096 bytes), whose layout
    // `BtrfsInoLookup` reproduces; `args` outlives the call.
    let result = unsafe {
        rustix::ioctl::ioctl(
            fd,
            rustix::ioctl::Updater::<BTRFS_IOC_INO_LOOKUP, BtrfsInoLookup>::new(&mut args),
        )
    };
    result.map_or(0, |()| args.tree_id)
}

/// Whether the volume that contains `path` distinguishes letter case; one
/// `statx` call when the volume was described before.
pub(super) fn case_sensitive(cache: &VolumeCache, path: &Path) -> FsResult<bool> {
    let (_, stat) = nearest_existing(path)?;
    let dev = device_number(stat.stx_dev_major, stat.stx_dev_minor);
    match cache.known(dev) {
        Some(known) => Ok(known.case_sensitive),
        None => volume(cache, path).map(|info| info.features.case_sensitive),
    }
}

/// Space figures of the volume with identifier `id`.
pub(super) fn space(cache: &VolumeCache, id: &VolumeId) -> FsResult<SpaceInfo> {
    let known = cache.known(id.dev).or_else(|| {
        let table = read_mount_table()?;
        table
            .entries()
            .iter()
            .rev()
            .find(|entry| entry.device() == id.dev)
            .map(|entry| KnownVolume {
                mount: PathBuf::from(OsStr::from_bytes(&entry.mount_point)),
                source: entry.source.clone(),
                case_sensitive: true,
            })
    });
    let Some(known) = known else {
        return Err(FsError::new(FsErrorKind::NotFound, "space", None));
    };
    let statvfs = retry(|| rustix::fs::statvfs(&known.mount))
        .map_err(|errno| sys_error(errno, "space", &known.mount))?;
    let total = statvfs.f_blocks.saturating_mul(statvfs.f_frsize);
    let available = statvfs.f_bavail.saturating_mul(statvfs.f_frsize);
    let quota = account_quota(&known.source);
    Ok(SpaceInfo {
        total: quota.map_or(total, |quota| total.min(quota.limit)),
        available: quota.map_or(available, |quota| available.min(quota.remaining)),
        quota_remaining: quota.map(|quota| quota.remaining),
        quota_limit: quota.map(|quota| quota.limit),
        purgeable_system: None,
    })
}

/// User quota of the calling account on the block device `source`; `None`
/// when quotas are not enabled, not supported, or not readable.
fn account_quota(source: &[u8]) -> Option<QuotaFigures> {
    if !source.starts_with(b"/") {
        return None;
    }
    let special = CString::new(source).ok()?;
    let uid = rustix::process::getuid().as_raw();
    let mut block = libc::dqblk {
        dqb_bhardlimit: 0,
        dqb_bsoftlimit: 0,
        dqb_curspace: 0,
        dqb_ihardlimit: 0,
        dqb_isoftlimit: 0,
        dqb_curinodes: 0,
        dqb_btime: 0,
        dqb_itime: 0,
        dqb_valid: 0,
    };
    // SAFETY: `special` is a NUL-terminated string that outlives the call,
    // and `Q_GETQUOTA` writes one `struct if_dqblk`, whose layout
    // `libc::dqblk` reproduces, into `block`, which outlives the call. The
    // quota identifier is the user id reinterpreted as `int`, as in the C
    // interface.
    let result = unsafe {
        libc::quotactl(
            libc::QCMD(libc::Q_GETQUOTA, USRQUOTA),
            special.as_ptr(),
            uid.cast_signed(),
            (&raw mut block).cast(),
        )
    };
    if result != 0 {
        return None;
    }
    quota_figures(
        block.dqb_valid,
        block.dqb_bsoftlimit,
        block.dqb_bhardlimit,
        block.dqb_curspace,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_no_quota_for_sources_without_block_device() {
        assert_eq!(account_quota(b"tmpfs"), None);
        assert_eq!(account_quota(b"/dev/nurt-missing-device"), None);
        assert_eq!(account_quota(b"/dev/null\0x"), None);
    }

    #[test]
    fn finds_no_block_device_for_virtual_file_systems() {
        assert_eq!(block_device(8, 1, b"/dev/sda1"), Some((8, 1)));
        assert_eq!(block_device(0, 40, b"tmpfs"), None);
        assert_eq!(block_device(0, 40, b"/dev/null"), None);
        assert_eq!(block_device(0, 40, b"/dev/nurt-missing-device"), None);
        assert!(!sysfs_flag((u32::MAX, u32::MAX), "removable"));
        assert_eq!(filesystem_uuid((u32::MAX, u32::MAX)), None);
    }

    #[test]
    fn falls_back_to_statfs_without_mount_table() {
        let dir = tempfile::tempdir().expect("directory");
        let (existing, stat) = nearest_existing(&dir.path().join("a/b")).expect("ancestor");
        assert_eq!(existing, dir.path());
        let dev = device_number(stat.stx_dev_major, stat.stx_dev_minor);
        let described = Described::from_statfs(&existing, dev).expect("statfs");
        assert!(dir.path().starts_with(&described.mount));
        assert!(!described.fs_type.is_empty());
    }

    #[test]
    fn reads_no_subvolume_without_descriptor_or_btrfs() {
        let dir = tempfile::tempdir().expect("directory");
        let (_, mut stat) = nearest_existing(dir.path()).expect("directory");
        stat.stx_mask &= !STATX_SUBVOL.bits();
        let fd = open_any(dir.path(), FileKind::Directory).expect("open");
        let fs_type = mount_entry(dir.path())
            .map(|entry| entry.fs_type)
            .unwrap_or_default();
        assert!(fs_type == "xfs" || !xfs_reflink(fd.as_fd()), "{fs_type}");
        assert_eq!(btrfs_subvolume(&stat, None), 0);
        let subvolume = btrfs_subvolume(&stat, Some(fd.as_fd()));
        assert!(fs_type == "btrfs" || subvolume == 0, "{fs_type}");
        assert!(open_any(Path::new("/dev/null"), FileKind::Special).is_none());
    }

    #[test]
    fn nearest_existing_reports_paths_without_ancestor_and_overlong_names() {
        let error = nearest_existing(Path::new("nurt-missing-relative/child"))
            .expect_err("no existing ancestor");
        assert_eq!(error.kind, FsErrorKind::NotFound);
        let dir = tempfile::tempdir().expect("directory");
        let error = nearest_existing(&dir.path().join("n".repeat(300))).expect_err("overlong name");
        assert_eq!(error.kind, FsErrorKind::NameTooLong);
    }

    #[test]
    fn finds_mount_by_device_without_mount_identifier() {
        let dir = tempfile::tempdir().expect("directory");
        let (existing, mut stat) = nearest_existing(dir.path()).expect("directory");
        let table = read_mount_table().expect("mount table");
        stat.stx_mask &= !StatxFlags::MNT_ID.bits();
        let entry = find_mount(&table, &existing, &stat).expect("mount by device");
        let mount = Path::new(OsStr::from_bytes(&entry.mount_point));
        let real = fs::canonicalize(dir.path()).expect("real path");
        assert!(real.starts_with(mount), "{}", real.display());
    }

    #[test]
    fn reports_subvolume_from_statx_when_the_kernel_provides_it() {
        let dir = tempfile::tempdir().expect("directory");
        let (_, mut stat) = nearest_existing(dir.path()).expect("directory");
        stat.stx_mask |= STATX_SUBVOL.bits();
        stat.stx_subvol = 42;
        assert_eq!(btrfs_subvolume(&stat, None), 42);
    }
}
