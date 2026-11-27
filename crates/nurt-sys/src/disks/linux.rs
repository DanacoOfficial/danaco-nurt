// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::disks::linux
// Responsibility:   Volumes of Linux, read from `/proc/self/mountinfo`, `statvfs`, `/sys`.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Volumes of Linux, read from `/proc/self/mountinfo`, `statvfs`, `/sys`
//! and `/dev/disk/by-label`.

use std::collections::HashMap;
use std::fs;

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::classify::{case_sensitive, classify, is_pseudo};
use super::mountinfo::{self, Mount};
use super::{Volume, VolumeList, VolumeSource};
use crate::common::SysRoot;

/// Mount option of read-only mounts.
const READ_ONLY: &str = "ro";

/// Content of the `removable` attribute of a removable block device.
const REMOVABLE: &str = "1";

/// Digits of a byte escape in udev names (`\x20`).
const HEX_ESCAPE_DIGITS: usize = 2;

/// Space figures of one mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Space {
    pub(crate) total: u64,
    pub(crate) available: u64,
    pub(crate) free: u64,
    pub(crate) inodes_total: u64,
    pub(crate) inodes_free: u64,
}

/// Volume source reading the mount table under a [`SysRoot`].
#[derive(Debug, Clone)]
pub(crate) struct MountTable {
    root: SysRoot,
}

impl MountTable {
    #[cfg(target_os = "linux")]
    pub(crate) fn live() -> Self {
        Self {
            root: SysRoot::live(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(root: SysRoot) -> Self {
        Self { root }
    }

    fn space(&self, mount_point: &str) -> Option<Space> {
        let stat = rustix::fs::statvfs(self.root.path(mount_point)).ok()?;
        let block = stat.f_frsize.max(1);
        Some(Space {
            total: stat.f_blocks.saturating_mul(block),
            available: stat.f_bavail.saturating_mul(block),
            free: stat.f_bfree.saturating_mul(block),
            inodes_total: stat.f_files,
            inodes_free: stat.f_ffree,
        })
    }

    fn removable(&self, device_number: &str) -> bool {
        let base = self.root.path(format!("/sys/dev/block/{device_number}"));
        [base.join("removable"), base.join("../removable")]
            .iter()
            .find_map(|path| fs::read_to_string(path).ok())
            .is_some_and(|value| value.trim() == REMOVABLE)
    }

    /// Labels of block devices by device path.
    fn labels(&self) -> HashMap<String, String> {
        let Ok(entries) = fs::read_dir(self.root.path("/dev/disk/by-label")) else {
            return HashMap::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let target = fs::read_link(entry.path()).ok()?;
                let device = target.file_name()?.to_str()?.to_owned();
                let label = decode_label(entry.file_name().to_str()?);
                Some((format!("/dev/{device}"), label))
            })
            .collect()
    }

    fn volume(&self, mount: Mount, labels: &HashMap<String, String>) -> Option<Volume> {
        let space = self.space(&mount.mount_point)?;
        let kind = classify(&mount.fs_type, self.removable(&mount.device_number));
        let readonly = mount.options.iter().any(|option| option == READ_ONLY);
        let mut options = mount.options;
        for option in mount.super_options {
            if !options.contains(&option) {
                options.push(option);
            }
        }
        Some(Volume {
            label: labels.get(&mount.source).cloned(),
            case_sensitive: case_sensitive(&mount.fs_type),
            mount: mount.mount_point,
            device: mount.source,
            filesystem: mount.fs_type,
            kind: Some(kind),
            total: space.total,
            free: space.available,
            used: space.total.saturating_sub(space.free),
            readonly,
            inodes_total: (space.inodes_total > 0).then_some(space.inodes_total),
            inodes_free: (space.inodes_total > 0).then_some(space.inodes_free),
            options: Some(options),
        })
    }
}

/// Decodes the `\xHH` escapes of a udev link name.
pub(crate) fn decode_label(name: &str) -> String {
    let mut decoded = Vec::with_capacity(name.len());
    let bytes = name.as_bytes();
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escape = (byte == b'\\' && bytes.get(index + 1) == Some(&b'x'))
            .then(|| bytes.get(index + 2..index + 2 + HEX_ESCAPE_DIGITS))
            .flatten()
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok());
        match escape {
            Some(value) => {
                decoded.push(value);
                index += 2 + HEX_ESCAPE_DIGITS;
            }
            None => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

impl VolumeSource for MountTable {
    fn volumes(&self) -> Result<VolumeList> {
        let text = self
            .root
            .read_to_string("/proc/self/mountinfo")
            .map_err(|error| {
                Error::new(
                    ErrorCode::Io,
                    format!("cannot read the mount table: {error}"),
                )
            })?;
        let mut mounts: Vec<Mount> = mountinfo::parse(&text)
            .into_iter()
            .filter(|mount| !is_pseudo(&mount.fs_type))
            .collect();
        // A later mount on the same point hides the earlier one.
        let mut seen = std::collections::HashSet::new();
        mounts.reverse();
        mounts.retain(|mount| seen.insert(mount.mount_point.clone()));
        let labels = self.labels();
        let mut list = VolumeList::default();
        for mount in mounts {
            match self.volume(mount, &labels) {
                Some(volume) => list.volumes.push(volume),
                None => list.unreadable += 1,
            }
        }
        Ok(list)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
