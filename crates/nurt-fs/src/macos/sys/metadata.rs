// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading and applying metadata.

use std::ffi::OsStr;
use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use nurt_contracts::fs::{
    ExtendedMeta, FileId, FileKind, FileMeta, Follow, FsError, FsErrorKind, FsResult, MetaChange,
    MetaTransferReport, Xattr,
};
use rustix::fs::{AtFlags, CWD, Gid, Mode, OFlags, RawMode, Stat, Timespec, Timestamps, Uid};
use xattr::FileExt;

use super::{c_path, ffi, io_error, retry, sys_error};
use crate::macos::{
    ErrnoContext, MODE_PERMISSION_BITS, acl_text_for_system, device_number, file_kind,
    flags_to_restore, split_nanos, timespec_nanos, unblocked_flags,
};

/// Label of the account and group in [`MetaTransferReport`].
const ACCOUNT: &str = "account";
/// Label of the permission bits.
const MODE: &str = "mode";
/// Label of the ACL.
const ACL: &str = "acl";
/// Label of the access and modification times.
const TIMES: &str = "times";
/// Label of the creation time.
const CREATED: &str = "created";
/// Label of the BSD file flags.
const FLAGS: &str = "flags";
/// Prefix of the label of an extended attribute.
const XATTR_PREFIX: &str = "xattr:";
/// Label used when the extended attributes of the target cannot be listed.
const XATTR_LIST: &str = "xattr:*";

/// `stat` of an entry.
pub(super) fn stat_path(path: &Path, follow: Follow, op: &str) -> FsResult<Stat> {
    let flags = match follow {
        Follow::NoFollow => AtFlags::SYMLINK_NOFOLLOW,
        Follow::Follow => AtFlags::empty(),
    };
    retry(|| rustix::fs::statat(CWD, path, flags))
        .map_err(|errno| sys_error(errno, ErrnoContext::Plain, op, path))
}

/// `fstat` of an open entry.
pub(super) fn stat_fd(fd: BorrowedFd<'_>, op: &str, path: &Path) -> FsResult<Stat> {
    retry(|| rustix::fs::fstat(fd)).map_err(|errno| sys_error(errno, ErrnoContext::Plain, op, path))
}

/// Basic metadata from `struct stat`.
pub(super) fn file_meta(stat: &Stat) -> FileMeta {
    let mode = u32::from(stat.st_mode);
    FileMeta {
        kind: file_kind(mode),
        size: u64::try_from(stat.st_size).unwrap_or_default(),
        mode: mode & MODE_PERMISSION_BITS,
        uid: Some(stat.st_uid),
        gid: Some(stat.st_gid),
        links: u64::from(stat.st_nlink),
        modified_ns: timespec_nanos(stat.st_mtime, stat.st_mtime_nsec),
        accessed_ns: Some(timespec_nanos(stat.st_atime, stat.st_atime_nsec)),
        created_ns: Some(timespec_nanos(stat.st_birthtime, stat.st_birthtime_nsec)),
        changed_ns: Some(timespec_nanos(stat.st_ctime, stat.st_ctime_nsec)),
        file_id: FileId {
            volume: device_number(stat.st_dev),
            low: stat.st_ino,
            high: 0,
        },
        windows_attributes: 0,
    }
}

/// Complete metadata of an entry.
pub(super) fn read_extended(path: &Path, follow: Follow) -> FsResult<ExtendedMeta> {
    const OP: &str = "extended_metadata";
    let stat = stat_path(path, follow, OP)?;
    let basic = file_meta(&stat);
    let xattrs = read_xattrs(path, follow)?;
    let c_path = c_path(path, OP)?;
    let acl = ffi::read_path_acl(&c_path, follow == Follow::Follow)
        .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, path))?;
    let link_target = if basic.kind == FileKind::Symlink {
        Some(read_link(path)?)
    } else {
        None
    };
    Ok(ExtendedMeta {
        basic,
        xattrs,
        acl,
        account_sid: None,
        flags: Some(u64::from(stat.st_flags)),
        streams: Vec::new(),
        link_target,
    })
}

/// Target of a symbolic link as stored in the link.
pub(super) fn read_link(path: &Path) -> FsResult<PathBuf> {
    const OP: &str = "read_link";
    match retry(|| rustix::fs::readlinkat(CWD, path, Vec::new())) {
        Ok(target) => Ok(PathBuf::from(OsStr::from_bytes(target.as_bytes()))),
        Err(rustix::io::Errno::INVAL) => {
            Err(FsError::new(FsErrorKind::InvalidPath, OP, Some(path)))
        }
        Err(errno) => Err(sys_error(errno, ErrnoContext::Plain, OP, path)),
    }
}

/// Extended attributes of every namespace, sorted by name. Extended
/// attributes of symbolic links are read with `XATTR_NOFOLLOW`.
fn read_xattrs(path: &Path, follow: Follow) -> FsResult<Vec<Xattr>> {
    const OP: &str = "list_xattr";
    let names = match follow {
        Follow::NoFollow => xattr::list(path),
        Follow::Follow => xattr::list_deref(path),
    };
    let names = match names {
        Ok(names) => names,
        Err(error) if xattrs_unsupported(&error) => return Ok(Vec::new()),
        Err(error) => return Err(io_error(&error, ErrnoContext::Plain, OP, path)),
    };
    let mut xattrs = Vec::new();
    for name in names {
        let value = match follow {
            Follow::NoFollow => xattr::get(path, &name),
            Follow::Follow => xattr::get_deref(path, &name),
        }
        .map_err(|error| io_error(&error, ErrnoContext::Plain, "get_xattr", path))?;
        if let Some(value) = value {
            xattrs.push(Xattr { name, value });
        }
    }
    xattrs.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(xattrs)
}

fn xattrs_unsupported(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOTSUP | libc::EOPNOTSUPP | libc::EPERM)
    )
}

/// Opens an entry for metadata changes without following a symbolic link in
/// the last component: `O_SYMLINK` opens the link itself. `O_NONBLOCK` keeps
/// the open of a FIFO from waiting for a writer.
pub(super) fn open_for_meta(path: &Path, op: &str) -> FsResult<File> {
    let stat = stat_path(path, Follow::NoFollow, op)?;
    let link = file_kind(u32::from(stat.st_mode)) == FileKind::Symlink;
    let mut flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
    flags |= if link {
        OFlags::SYMLINK
    } else {
        OFlags::NOFOLLOW
    };
    retry(|| rustix::fs::openat(CWD, path, flags, Mode::empty()))
        .map(File::from)
        .map_err(|errno| sys_error(errno, ErrnoContext::Plain, op, path))
}

/// Applies complete metadata to an open entry and reports every element that
/// could not be applied.
///
/// Write-blocking flags of the target are cleared first and the stored flags
/// are set last, because `UF_IMMUTABLE` and `UF_APPEND` refuse every other
/// change. The account and group are changed before the permission bits,
/// because changing them clears the set-user-id and set-group-id bits. The
/// creation time is set after the modification time, because HFS+ moves a
/// creation time later than the modification time back to it.
pub(super) fn apply_extended(
    file: &File,
    meta: &ExtendedMeta,
    path: &Path,
) -> FsResult<MetaTransferReport> {
    let current = stat_fd(file.as_fd(), "apply_meta", path)?;
    let mut report = Report::default();
    if let Some(flags) = unblocked_flags(current.st_flags) {
        report.check(FLAGS, ffi::set_fd_flags(file.as_fd(), flags).is_ok());
    }
    apply_account(file.as_fd(), &meta.basic, &current, &mut report);
    report.check(MODE, set_mode(file.as_fd(), meta.basic.mode).is_ok());
    apply_xattrs(file, &meta.xattrs, &mut report);
    apply_acl(file.as_fd(), meta.acl.as_deref(), &mut report);
    let times = timestamps(
        meta.basic.accessed_ns,
        Some(meta.basic.modified_ns),
        &current,
    );
    report.check(
        TIMES,
        retry(|| rustix::fs::futimens(file.as_fd(), &times)).is_ok(),
    );
    if let Some(created) = meta.basic.created_ns {
        report.check(
            CREATED,
            ffi::set_creation_time(file.as_fd(), created).is_ok(),
        );
    }
    let stored = meta.flags.unwrap_or(u64::from(current.st_flags));
    let flags = flags_to_restore(stored, current.st_flags);
    report.check(FLAGS, ffi::set_fd_flags(file.as_fd(), flags).is_ok());
    Ok(report.finish())
}

fn apply_account(fd: BorrowedFd<'_>, wanted: &FileMeta, current: &Stat, report: &mut Report) {
    let uid = wanted.uid.filter(|&uid| uid != current.st_uid);
    let gid = wanted.gid.filter(|&gid| gid != current.st_gid);
    if uid.is_some() || gid.is_some() {
        let changed =
            retry(|| rustix::fs::fchown(fd, uid.map(Uid::from_raw), gid.map(Gid::from_raw)));
        report.check(ACCOUNT, changed.is_ok());
    }
}

fn set_mode(fd: BorrowedFd<'_>, mode: u32) -> rustix::io::Result<()> {
    let raw =
        RawMode::try_from(mode & MODE_PERMISSION_BITS).map_err(|_| rustix::io::Errno::INVAL)?;
    retry(|| rustix::fs::fchmod(fd, Mode::from_raw_mode(raw)))
}

fn apply_xattrs(file: &File, wanted: &[Xattr], report: &mut Report) {
    let Ok(present) = file.list_xattr() else {
        if !wanted.is_empty() {
            report.fail(XATTR_LIST);
        }
        return;
    };
    for name in present.filter(|name| wanted.iter().all(|xattr| &xattr.name != name)) {
        report.check(&xattr_label(&name), file.remove_xattr(&name).is_ok());
    }
    for xattr in wanted {
        let unchanged = file
            .get_xattr(&xattr.name)
            .is_ok_and(|value| value.as_deref() == Some(xattr.value.as_slice()));
        if !unchanged {
            report.check(
                &xattr_label(&xattr.name),
                file.set_xattr(&xattr.name, &xattr.value).is_ok(),
            );
        }
    }
}

fn xattr_label(name: &OsStr) -> String {
    format!("{XATTR_PREFIX}{}", name.to_string_lossy())
}

fn apply_acl(fd: BorrowedFd<'_>, wanted: Option<&[u8]>, report: &mut Report) {
    let current = ffi::read_fd_acl(fd);
    if current.as_ref().is_ok_and(|acl| acl.as_deref() == wanted) {
        return;
    }
    let applied = match wanted {
        Some(text) => {
            acl_text_for_system(text).is_ok_and(|text| ffi::write_fd_acl(fd, Some(&text)).is_ok())
        }
        None => ffi::write_fd_acl(fd, None).is_ok(),
    };
    report.check(ACL, applied);
}

/// Times for `futimens`; an absent time keeps its current value.
fn timestamps(accessed_ns: Option<i64>, modified_ns: Option<i64>, current: &Stat) -> Timestamps {
    let accessed =
        accessed_ns.unwrap_or_else(|| timespec_nanos(current.st_atime, current.st_atime_nsec));
    let modified =
        modified_ns.unwrap_or_else(|| timespec_nanos(current.st_mtime, current.st_mtime_nsec));
    Timestamps {
        last_access: timespec(accessed),
        last_modification: timespec(modified),
    }
}

fn timespec(nanos: i64) -> Timespec {
    let (seconds, fraction) = split_nanos(nanos);
    Timespec {
        tv_sec: seconds,
        tv_nsec: fraction,
    }
}

/// Applies a metadata change requested by a command; the first refused
/// element fails the call.
pub(super) fn apply_change(file: &File, change: &MetaChange, path: &Path) -> FsResult<()> {
    const OP: &str = "set_meta";
    if change.set_windows_attributes != 0 || change.clear_windows_attributes != 0 {
        return Err(FsError::new(FsErrorKind::Unsupported, OP, Some(path)));
    }
    let fd = file.as_fd();
    let current = stat_fd(fd, OP, path)?;
    let plain = |errno| sys_error(errno, ErrnoContext::Plain, OP, path);
    if change.uid.is_some() || change.gid.is_some() {
        retry(|| {
            rustix::fs::fchown(
                fd,
                change.uid.map(Uid::from_raw),
                change.gid.map(Gid::from_raw),
            )
        })
        .map_err(|errno| sys_error(errno, ErrnoContext::Privileged, OP, path))?;
    }
    if let Some(mode) = change.mode {
        set_mode(fd, mode).map_err(plain)?;
    }
    change_xattrs(file, change, path)?;
    if let Some(acl) = &change.acl {
        let text = acl_text_for_system(acl)
            .map_err(|_| FsError::new(FsErrorKind::InvalidPath, OP, Some(path)))?;
        ffi::write_fd_acl(fd, Some(&text))
            .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, path))?;
    }
    if change.modified_ns.is_some() || change.accessed_ns.is_some() {
        let times = timestamps(change.accessed_ns, change.modified_ns, &current);
        retry(|| rustix::fs::futimens(fd, &times)).map_err(plain)?;
    }
    if let Some(created) = change.created_ns {
        ffi::set_creation_time(fd, created)
            .map_err(|error| io_error(&error, ErrnoContext::Plain, OP, path))?;
    }
    Ok(())
}

fn change_xattrs(file: &File, change: &MetaChange, path: &Path) -> FsResult<()> {
    let failed = |error: std::io::Error| io_error(&error, ErrnoContext::Plain, "set_meta", path);
    for name in &change.remove_xattrs {
        file.remove_xattr(name).map_err(failed)?;
    }
    for xattr in &change.set_xattrs {
        file.set_xattr(&xattr.name, &xattr.value).map_err(failed)?;
    }
    Ok(())
}

/// Elements that could not be transferred, without duplicates.
#[derive(Default)]
struct Report(Vec<String>);

impl Report {
    fn check(&mut self, label: &str, transferred: bool) {
        if !transferred {
            self.fail(label);
        }
    }

    fn fail(&mut self, label: &str) {
        if !self.0.iter().any(|known| known == label) {
            self.0.push(label.to_owned());
        }
    }

    fn finish(self) -> MetaTransferReport {
        MetaTransferReport {
            not_transferred: self.0,
        }
    }
}
