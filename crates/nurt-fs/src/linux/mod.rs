// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::linux
// Responsibility:   Linux back end.
// Dependencies:     errno, fs, mountinfo, nurt_contracts, quota, table
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Linux back end.
//!
//! `LinuxFs` implements [`Fs`](nurt_contracts::fs::Fs) and
//! [`FsWrite`](nurt_contracts::fs::FsWrite) with Linux system calls:
//! `renameat2` (`RENAME_EXCHANGE`, `RENAME_NOREPLACE`), `FICLONE`,
//! `copy_file_range`, `statx`, extended attributes (POSIX ACL included),
//! inode flags, `statvfs`, `quotactl`, `flock` and `fsync` of files and
//! directories. It is available only when the crate is built for Linux.
//!
//! The parts that do not call the system compile on every target and are
//! tested there: classification of `errno` values ([`error_kind`]), the
//! mount table parser ([`MountTable`]), the file system type table
//! ([`volume_kind`], [`features_for`]) and quota arithmetic
//! ([`quota_figures`]).

mod errno;
mod mountinfo;
mod quota;
mod table;

#[cfg(target_os = "linux")]
mod copy;
#[cfg(target_os = "linux")]
mod fs;
#[cfg(target_os = "linux")]
mod handle;
#[cfg(target_os = "linux")]
mod holders;
#[cfg(target_os = "linux")]
mod meta;
#[cfg(target_os = "linux")]
mod remove;
#[cfg(target_os = "linux")]
mod sys;
#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests;
#[cfg(target_os = "linux")]
mod volume;

pub use errno::{ErrnoContext, errno_error, error_kind};
#[cfg(target_os = "linux")]
pub use fs::LinuxFs;
pub use mountinfo::{MountEntry, MountTable, MountinfoError, OverlayLayers, device_number};
pub use quota::{QIF_BLIMITS, QIF_SPACE, QUOTA_BLOCK_SIZE, QuotaFigures, quota_figures};
pub use table::{features_for, fs_type_from_magic, volume_kind};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
