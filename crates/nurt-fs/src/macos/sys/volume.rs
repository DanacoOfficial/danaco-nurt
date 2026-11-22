// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Volume identity, class, capabilities and space.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use nurt_contracts::fs::{
    AbsPath, Follow, FsError, FsErrorKind, FsResult, SpaceInfo, VolumeId, VolumeInfo,
};
use rustix::fs::StatFs;
use rustix::io::Errno;

use super::ffi::{self, VolumeKey, VolumeUrl};
use super::metadata::stat_path;
use super::{c_path, retry, sys_error};
use crate::macos::{
    ErrnoContext, MNT_LOCAL, MNT_QUOTA, MNT_RDONLY, SpaceFigures, device_number, space_info,
    statfs_name, uuid_text, volume_features, volume_kind,
};

/// `statfs` of the nearest existing ancestor of `path`, with that ancestor.
pub(super) fn nearest_statfs(path: &AbsPath, op: &str) -> FsResult<(AbsPath, StatFs)> {
    let mut current = Some(path.clone());
    while let Some(candidate) = current {
        match retry(|| rustix::fs::statfs(candidate.as_path())) {
            Ok(stat) => return Ok((candidate, stat)),
            Err(Errno::NOENT | Errno::NOTDIR) => current = candidate.parent(),
            Err(errno) => return Err(sys_error(errno, ErrnoContext::Plain, op, path.as_path())),
        }
    }
    Err(FsError::new(
        FsErrorKind::NotFound,
        op,
        Some(path.as_path()),
    ))
}

/// Case sensitivity of the volume that contains `path`.
pub(super) fn case_sensitive_at(path: &AbsPath, op: &str) -> FsResult<bool> {
    let (existing, stat) = nearest_statfs(path, op)?;
    let probe = c_path(existing.as_path(), op)?;
    Ok(ffi::case_sensitive(&probe).unwrap_or_else(|| {
        volume_features(&fs_type(&stat), None, capabilities(&stat)).case_sensitive
    }))
}

/// Description of the volume that contains `path`.
///
/// `rotational` is always `false`: macOS exposes the medium type only
/// through `IOKit`, which the back end does not query, so rotational disks
/// are treated like solid-state storage.
pub(super) fn describe(path: &AbsPath) -> FsResult<VolumeInfo> {
    const OP: &str = "volume";
    let (existing, stat) = nearest_statfs(path, OP)?;
    let device = stat_path(existing.as_path(), Follow::Follow, OP)?.st_dev;
    let mount = mount_point(&stat, OP)?;
    let fs_type = fs_type(&stat);
    let mount_c = c_path(mount.as_path(), OP)?;
    let attributes = ffi::volume_attributes(&mount_c).ok();
    let url = VolumeUrl::new(mount.as_path());
    let removable = url.as_ref().is_some_and(|url| {
        url.flag(VolumeKey::IsRemovable).unwrap_or(false)
            || url.flag(VolumeKey::IsEjectable).unwrap_or(false)
    });
    let probe = c_path(existing.as_path(), OP)?;
    let features = volume_features(
        &fs_type,
        ffi::case_sensitive(&probe),
        attributes.map(|reply| reply.capabilities),
    );
    Ok(VolumeInfo {
        id: VolumeId {
            dev: device_number(device),
            uuid: attributes
                .and_then(|reply| reply.uuid)
                .map(|uuid| uuid_text(&uuid)),
            subvolume: 0,
        },
        mount,
        kind: volume_kind(&fs_type, stat.f_flags, removable),
        features,
        rotational: false,
        read_only: stat.f_flags & MNT_RDONLY != 0,
        fs_type,
    })
}

/// Space figures of the mounted volume with the identity `id`.
pub(super) fn space(id: &VolumeId) -> FsResult<SpaceInfo> {
    const OP: &str = "space";
    let mount = find_mount(id)?;
    let stat = retry(|| rustix::fs::statfs(mount.as_path()))
        .map_err(|errno| sys_error(errno, ErrnoContext::Plain, OP, mount.as_path()))?;
    let url = VolumeUrl::new(mount.as_path());
    let number = |key| url.as_ref().and_then(|url| url.number(key));
    let quota = if stat.f_flags & MNT_QUOTA == 0 {
        None
    } else {
        let mount_c = c_path(mount.as_path(), OP)?;
        ffi::user_quota(&mount_c, rustix::process::geteuid().as_raw())
    };
    Ok(space_info(&SpaceFigures {
        block_size: u64::from(stat.f_bsize),
        blocks: stat.f_blocks,
        blocks_available: stat.f_bavail,
        total_capacity: number(VolumeKey::TotalCapacity),
        available_capacity: number(VolumeKey::AvailableCapacity),
        important_capacity: number(VolumeKey::ImportantCapacity),
        quota,
    }))
}

/// Mount point of the volume with the identity `id`.
///
/// `statfs.f_fsid.val[0]` holds the device number of the volume; local
/// volumes whose identifier differs are matched by `stat` of their mount
/// point. Network volumes are not probed with `stat`, which can block on an
/// unreachable server.
fn find_mount(id: &VolumeId) -> FsResult<AbsPath> {
    const OP: &str = "space";
    let mounts = ffi::mounted_volumes()
        .map_err(|error| super::io_error(&error, ErrnoContext::Plain, OP, Path::new("/")))?;
    let by_fsid = mounts
        .iter()
        .find(|stat| device_number(ffi::fsid_device(stat.f_fsid)) == id.dev);
    let by_stat = || {
        mounts.iter().find(|stat| {
            stat.f_flags & MNT_LOCAL != 0
                && mount_point(stat, OP)
                    .and_then(|mount| stat_path(mount.as_path(), Follow::Follow, OP))
                    .is_ok_and(|mount_stat| device_number(mount_stat.st_dev) == id.dev)
        })
    };
    by_fsid
        .or_else(by_stat)
        .ok_or_else(|| FsError::new(FsErrorKind::NotFound, OP, None))
        .and_then(|stat| mount_point(stat, OP))
}

fn capabilities(stat: &StatFs) -> Option<crate::macos::VolumeCapabilities> {
    let mount = mount_point(stat, "volume").ok()?;
    let mount_c = c_path(mount.as_path(), "volume").ok()?;
    ffi::volume_attributes(&mount_c)
        .ok()
        .map(|reply| reply.capabilities)
}

fn fs_type(stat: &StatFs) -> String {
    String::from_utf8_lossy(statfs_name(&c_bytes(&stat.f_fstypename))).into_owned()
}

fn mount_point(stat: &StatFs, op: &str) -> FsResult<AbsPath> {
    let bytes = c_bytes(&stat.f_mntonname);
    let name = statfs_name(&bytes);
    AbsPath::new(Path::new(OsStr::from_bytes(name)))
        .map_err(|_| FsError::new(FsErrorKind::Io, op, None))
}

fn c_bytes(field: &[std::ffi::c_char]) -> Vec<u8> {
    field.iter().map(|&byte| byte.cast_unsigned()).collect()
}

/// Whether `stat` describes a read-only mount.
pub(super) fn read_only(stat: &StatFs) -> bool {
    stat.f_flags & MNT_RDONLY != 0
}
