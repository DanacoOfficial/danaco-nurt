// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Metadata of entries: `statx`, extended attributes, inode flags and the
//! transfer of complete metadata to another entry.
//!
//! POSIX ACL are extended attributes on Linux (`system.posix_acl_access`,
//! `system.posix_acl_default`) and travel with the other attributes. The
//! birth time cannot be set on Linux; a transfer leaves it unchanged and
//! does not list it as an element that was not transferred.

use std::ffi::{OsStr, OsString};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use nurt_contracts::fs::{
    ExtendedMeta, FileId, FileKind, FileMeta, Follow, FsError, FsErrorKind, FsResult,
    MetaTransferReport, Xattr,
};
use rustix::fs::{
    AtFlags, CWD, FileType, Gid, IFlags, Mode, OFlags, Statx, StatxFlags, Timestamps, UTIME_OMIT,
    Uid, XattrFlags,
};
use rustix::io::Errno;

use super::mountinfo::device_number;
use super::sys::{nanos, privileged_error, retry, sys_error, timespec};

/// Permission bits of `st_mode`, including setuid, setgid and sticky.
pub(super) const PERMISSION_BITS: u32 = 0o7777;
/// Fields requested from `statx`.
const STATX_MASK: StatxFlags = StatxFlags::BASIC_STATS.union(StatxFlags::BTIME);
/// Initial buffer for an attribute value or name list.
const XATTR_INITIAL_BUFFER: usize = 1024;
/// Largest attribute value and name list accepted by the kernel
/// (`XATTR_SIZE_MAX`, `XATTR_LIST_MAX`).
const XATTR_MAX_BUFFER: usize = 65_536;
/// Prefix of the POSIX ACL attributes.
const ACL_PREFIX: &[u8] = b"system.posix_acl_";
/// Attribute names whose change requires a capability.
const PRIVILEGED_XATTR_PREFIXES: &[&[u8]] = &[b"security.capability", b"trusted."];
/// Inode flags that refuse every later change of the entry; they are lifted
/// before a transfer and set again at its end.
const BLOCKING_FLAGS: IFlags = IFlags::IMMUTABLE.union(IFlags::APPEND);

/// Basic metadata of `path` relative to `dir`.
pub(super) fn stat_at(
    dir: BorrowedFd<'_>,
    path: &Path,
    follow: Follow,
    op: &str,
    report: &Path,
) -> FsResult<FileMeta> {
    let flags = match follow {
        Follow::Follow => AtFlags::empty(),
        Follow::NoFollow => AtFlags::SYMLINK_NOFOLLOW,
    };
    match retry(|| rustix::fs::statx(dir, path, flags, STATX_MASK)) {
        Ok(stat) => Ok(meta_from_statx(&stat)),
        Err(Errno::NOSYS) => retry(|| rustix::fs::statat(dir, path, flags))
            .map(|stat| meta_from_stat(&stat))
            .map_err(|errno| sys_error(errno, op, report)),
        Err(errno) => Err(sys_error(errno, op, report)),
    }
}

/// Basic metadata of an open file.
pub(super) fn stat_fd(fd: BorrowedFd<'_>, op: &str, report: &Path) -> FsResult<FileMeta> {
    match retry(|| rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, STATX_MASK)) {
        Ok(stat) => Ok(meta_from_statx(&stat)),
        Err(Errno::NOSYS) => retry(|| rustix::fs::fstat(fd))
            .map(|stat| meta_from_stat(&stat))
            .map_err(|errno| sys_error(errno, op, report)),
        Err(errno) => Err(sys_error(errno, op, report)),
    }
}

fn meta_from_statx(stat: &Statx) -> FileMeta {
    let mode = u32::from(stat.stx_mode);
    let birth = StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::BTIME);
    FileMeta {
        kind: kind_from_mode(mode),
        size: stat.stx_size,
        mode: mode & PERMISSION_BITS,
        uid: Some(stat.stx_uid),
        gid: Some(stat.stx_gid),
        links: u64::from(stat.stx_nlink),
        modified_ns: nanos(stat.stx_mtime.tv_sec, stat.stx_mtime.tv_nsec),
        accessed_ns: Some(nanos(stat.stx_atime.tv_sec, stat.stx_atime.tv_nsec)),
        created_ns: birth.then(|| nanos(stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec)),
        changed_ns: Some(nanos(stat.stx_ctime.tv_sec, stat.stx_ctime.tv_nsec)),
        file_id: FileId {
            volume: device_number(stat.stx_dev_major, stat.stx_dev_minor),
            low: stat.stx_ino,
            high: 0,
        },
        windows_attributes: 0,
    }
}

/// Metadata from `struct stat`, used on kernels without `statx`.
#[allow(
    clippy::useless_conversion,
    clippy::unnecessary_fallible_conversions,
    reason = "field types of `struct stat` differ between architectures"
)]
fn meta_from_stat(stat: &rustix::fs::Stat) -> FileMeta {
    let mode = u32::from(stat.st_mode);
    let time = |seconds, nanoseconds| {
        nanos(
            i64::try_from(seconds).unwrap_or(i64::MAX),
            u32::try_from(nanoseconds).unwrap_or(0),
        )
    };
    FileMeta {
        kind: kind_from_mode(mode),
        size: u64::try_from(stat.st_size).unwrap_or(0),
        mode: mode & PERMISSION_BITS,
        uid: Some(stat.st_uid),
        gid: Some(stat.st_gid),
        links: u64::try_from(stat.st_nlink).unwrap_or(u64::MAX),
        modified_ns: time(stat.st_mtime, stat.st_mtime_nsec),
        accessed_ns: Some(time(stat.st_atime, stat.st_atime_nsec)),
        created_ns: None,
        changed_ns: Some(time(stat.st_ctime, stat.st_ctime_nsec)),
        file_id: FileId {
            volume: u64::try_from(stat.st_dev).unwrap_or(0),
            low: u64::try_from(stat.st_ino).unwrap_or(0),
            high: 0,
        },
        windows_attributes: 0,
    }
}

pub(super) fn kind_from_mode(mode: u32) -> FileKind {
    match FileType::from_raw_mode(mode) {
        FileType::RegularFile => FileKind::File,
        FileType::Directory => FileKind::Directory,
        FileType::Symlink => FileKind::Symlink,
        _ => FileKind::Special,
    }
}

/// Entry whose metadata is read or changed: an open file or a path whose
/// last component is followed or not.
#[derive(Clone, Copy)]
pub(super) enum Target<'a> {
    Fd(BorrowedFd<'a>),
    Path { path: &'a Path, follow: bool },
}

impl Target<'_> {
    fn list_xattr_names(self) -> rustix::io::Result<Vec<u8>> {
        read_growing(|buffer| match self {
            Self::Fd(fd) => rustix::fs::flistxattr(fd, buffer),
            Self::Path { path, follow: true } => rustix::fs::listxattr(path, buffer),
            Self::Path {
                path,
                follow: false,
            } => rustix::fs::llistxattr(path, buffer),
        })
    }

    fn get_xattr(self, name: &OsStr) -> rustix::io::Result<Vec<u8>> {
        read_growing(|buffer| match self {
            Self::Fd(fd) => rustix::fs::fgetxattr(fd, name, buffer),
            Self::Path { path, follow: true } => rustix::fs::getxattr(path, name, buffer),
            Self::Path {
                path,
                follow: false,
            } => rustix::fs::lgetxattr(path, name, buffer),
        })
    }

    fn set_xattr(self, name: &OsStr, value: &[u8]) -> rustix::io::Result<()> {
        let flags = XattrFlags::empty();
        retry(|| match self {
            Self::Fd(fd) => rustix::fs::fsetxattr(fd, name, value, flags),
            Self::Path { path, follow: true } => rustix::fs::setxattr(path, name, value, flags),
            Self::Path {
                path,
                follow: false,
            } => rustix::fs::lsetxattr(path, name, value, flags),
        })
    }

    fn remove_xattr(self, name: &OsStr) -> rustix::io::Result<()> {
        retry(|| match self {
            Self::Fd(fd) => rustix::fs::fremovexattr(fd, name),
            Self::Path { path, follow: true } => rustix::fs::removexattr(path, name),
            Self::Path {
                path,
                follow: false,
            } => rustix::fs::lremovexattr(path, name),
        })
    }

    fn chown(self, uid: Option<u32>, gid: Option<u32>) -> rustix::io::Result<()> {
        let uid = uid.map(Uid::from_raw);
        let gid = gid.map(Gid::from_raw);
        retry(|| match self {
            Self::Fd(fd) => rustix::fs::fchown(fd, uid, gid),
            Self::Path { path, follow } => {
                rustix::fs::chownat(CWD, path, uid, gid, nofollow_flag(follow))
            }
        })
    }

    fn chmod(self, mode: u32) -> rustix::io::Result<()> {
        let mode = Mode::from_raw_mode(mode & PERMISSION_BITS);
        retry(|| match self {
            Self::Fd(fd) => rustix::fs::fchmod(fd, mode),
            Self::Path { path, .. } => rustix::fs::chmodat(CWD, path, mode, AtFlags::empty()),
        })
    }

    fn set_times(
        self,
        accessed_ns: Option<i64>,
        modified_ns: Option<i64>,
    ) -> rustix::io::Result<()> {
        let stamp = |value: Option<i64>| {
            value.map_or(
                rustix::fs::Timespec {
                    tv_sec: 0,
                    tv_nsec: UTIME_OMIT,
                },
                timespec,
            )
        };
        let times = Timestamps {
            last_access: stamp(accessed_ns),
            last_modification: stamp(modified_ns),
        };
        retry(|| match self {
            Self::Fd(fd) => rustix::fs::futimens(fd, &times),
            Self::Path { path, follow } => {
                rustix::fs::utimensat(CWD, path, &times, nofollow_flag(follow))
            }
        })
    }

    fn statx(self) -> rustix::io::Result<Statx> {
        let mask = StatxFlags::TYPE | StatxFlags::MODE | StatxFlags::UID | StatxFlags::GID;
        retry(|| match self {
            Self::Fd(fd) => rustix::fs::statx(fd, "", AtFlags::EMPTY_PATH, mask),
            Self::Path { path, follow } => {
                rustix::fs::statx(CWD, path, nofollow_flag(follow), mask)
            }
        })
    }
}

fn nofollow_flag(follow: bool) -> AtFlags {
    if follow {
        AtFlags::empty()
    } else {
        AtFlags::SYMLINK_NOFOLLOW
    }
}

/// Calls `read` with a buffer that doubles on `ERANGE` up to the kernel
/// limit of attribute values and name lists.
fn read_growing(
    mut read: impl FnMut(&mut [u8]) -> rustix::io::Result<usize>,
) -> rustix::io::Result<Vec<u8>> {
    let mut size = XATTR_INITIAL_BUFFER;
    loop {
        let mut buffer = vec![0; size];
        match retry(|| read(&mut buffer)) {
            Ok(length) => {
                buffer.truncate(length);
                return Ok(buffer);
            }
            Err(Errno::RANGE) if size < XATTR_MAX_BUFFER => size *= 2,
            Err(errno) => return Err(errno),
        }
    }
}

/// Every readable extended attribute, sorted by name.
///
/// A volume without extended attributes yields an empty list. Attributes
/// that disappear while they are read, and attributes the account may not
/// read (`trusted.*` without `CAP_SYS_ADMIN`), are omitted.
pub(super) fn read_xattrs(target: Target<'_>, op: &str, report: &Path) -> FsResult<Vec<Xattr>> {
    let names = match target.list_xattr_names() {
        Ok(names) => names,
        Err(Errno::NOTSUP | Errno::NOSYS) => return Ok(Vec::new()),
        Err(errno) => return Err(sys_error(errno, op, report)),
    };
    let mut xattrs = Vec::new();
    for name in names
        .split(|&byte| byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = OsStr::from_bytes(name);
        match target.get_xattr(name) {
            Ok(value) => xattrs.push(Xattr {
                name: name.to_os_string(),
                value,
            }),
            Err(Errno::NODATA | Errno::ACCESS | Errno::PERM | Errno::NOTSUP) => {}
            Err(errno) => return Err(sys_error(errno, op, report)),
        }
    }
    xattrs.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(xattrs)
}

/// Inode flags (`FS_IOC_GETFLAGS`) of an open file; `None` when the file
/// system has none.
pub(super) fn read_flags(fd: BorrowedFd<'_>, op: &str, report: &Path) -> FsResult<Option<u64>> {
    match retry(|| rustix::fs::ioctl_getflags(fd)) {
        Ok(flags) => Ok(Some(u64::from(flags.bits()))),
        Err(Errno::NOTTY | Errno::NOTSUP | Errno::NOSYS | Errno::INVAL) => Ok(None),
        Err(errno) => Err(sys_error(errno, op, report)),
    }
}

/// Opens a regular file or directory to query or change inode flags,
/// without following a symbolic link in the last component when `follow`
/// is false. Special files are never opened; opening a device or FIFO can
/// have side effects.
pub(super) fn open_for_flags(path: &Path, kind: FileKind, follow: bool) -> Option<OwnedFd> {
    let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY;
    match kind {
        FileKind::File => {}
        FileKind::Directory => flags |= OFlags::DIRECTORY,
        _ => return None,
    }
    if !follow {
        flags |= OFlags::NOFOLLOW;
    }
    retry(|| rustix::fs::openat(CWD, path, flags, Mode::empty())).ok()
}

/// Complete metadata of `path`.
pub(super) fn extended_metadata(path: &Path, follow: Follow) -> FsResult<ExtendedMeta> {
    const OP: &str = "extended_metadata";
    let basic = stat_at(CWD, path, follow, OP, path)?;
    let follow_last = follow == Follow::Follow;
    let xattrs = read_xattrs(
        Target::Path {
            path,
            follow: follow_last,
        },
        OP,
        path,
    )?;
    let flags = match open_for_flags(path, basic.kind, follow_last) {
        Some(fd) => read_flags(fd.as_fd(), OP, path)?,
        None => None,
    };
    let link_target = if basic.kind == FileKind::Symlink {
        let target = retry(|| rustix::fs::readlinkat(CWD, path, Vec::new()))
            .map_err(|errno| sys_error(errno, OP, path))?;
        Some(OsString::from_vec(target.into_bytes()).into())
    } else {
        None
    };
    Ok(ExtendedMeta {
        basic,
        xattrs,
        acl: None,
        account_sid: None,
        flags,
        streams: Vec::new(),
        link_target,
    })
}

/// Transfers complete metadata to an entry and lists the elements that
/// could not be transferred.
///
/// The order follows the dependencies between the elements: write-blocking
/// flags are lifted first; the account and group of the file are changed
/// before the permission bits, because the change clears setuid and setgid;
/// extended attributes follow, because the change of account also removes
/// `security.capability`; the remaining flags, the times and finally the
/// write-blocking flags close the transfer.
pub(super) struct Transfer<'a> {
    /// Entry that receives the metadata.
    pub(super) target: Target<'a>,
    /// Open file used for inode flags; `None` when the entry cannot be
    /// opened, in which case requested flags are reported as not
    /// transferred.
    pub(super) flags_fd: Option<BorrowedFd<'a>>,
    /// Whether the entry is a symbolic link (no permission bits, no flags).
    pub(super) symlink: bool,
    /// Path used in errors.
    pub(super) report: &'a Path,
}

impl Transfer<'_> {
    pub(super) fn apply(&self, meta: &ExtendedMeta) -> FsResult<MetaTransferReport> {
        const OP: &str = "restore_meta";
        let mut missing = Vec::new();
        let current = self
            .target
            .statx()
            .map_err(|errno| sys_error(errno, OP, self.report))?;
        let wanted_flags = if self.symlink { None } else { meta.flags };
        let mut flags = self.current_flags(OP)?;
        if let (Some(fd), Some(current_flags)) = (self.flags_fd, flags)
            && current_flags.intersects(BLOCKING_FLAGS)
        {
            let lifted = current_flags.difference(BLOCKING_FLAGS);
            if retry(|| rustix::fs::ioctl_setflags(fd, lifted)).is_ok() {
                flags = Some(lifted);
            } else {
                push_once(&mut missing, "flags");
            }
        }

        let uid = meta.basic.uid.filter(|&uid| uid != current.stx_uid);
        let gid = meta.basic.gid.filter(|&gid| gid != current.stx_gid);
        let owner_changed = uid.is_some() || gid.is_some();
        if owner_changed && self.target.chown(uid, gid).is_err() {
            missing.push("owner".to_owned());
        }
        let current_mode = u32::from(current.stx_mode) & PERMISSION_BITS;
        let wanted_mode = meta.basic.mode & PERMISSION_BITS;
        if !self.symlink
            && (owner_changed || current_mode != wanted_mode)
            && self.target.chmod(wanted_mode).is_err()
        {
            missing.push("mode".to_owned());
        }

        self.transfer_xattrs(&meta.xattrs, &mut missing, OP)?;

        if let Some(wanted) = wanted_flags {
            let wanted = flags_from_u64(wanted);
            let first = wanted.difference(BLOCKING_FLAGS);
            match (self.flags_fd, flags) {
                (Some(fd), Some(current_flags)) if current_flags != first => {
                    if retry(|| rustix::fs::ioctl_setflags(fd, first)).is_err() {
                        push_once(&mut missing, "flags");
                    }
                }
                (Some(_), Some(_)) => {}
                _ => push_once(&mut missing, "flags"),
            }
        }

        if self
            .target
            .set_times(meta.basic.accessed_ns, Some(meta.basic.modified_ns))
            .is_err()
        {
            missing.push("times".to_owned());
        }

        if let (Some(wanted), Some(fd)) = (wanted_flags.map(flags_from_u64), self.flags_fd)
            && wanted.intersects(BLOCKING_FLAGS)
            && retry(|| rustix::fs::ioctl_setflags(fd, wanted)).is_err()
        {
            push_once(&mut missing, "flags");
        }
        Ok(MetaTransferReport {
            not_transferred: missing,
        })
    }

    fn current_flags(&self, op: &str) -> FsResult<Option<IFlags>> {
        match self.flags_fd {
            Some(fd) => Ok(read_flags(fd, op, self.report)?.map(flags_from_u64)),
            None => Ok(None),
        }
    }

    fn transfer_xattrs(
        &self,
        wanted: &[Xattr],
        missing: &mut Vec<String>,
        op: &str,
    ) -> FsResult<()> {
        let current = read_xattrs(self.target, op, self.report)?;
        for xattr in wanted {
            let unchanged = current
                .iter()
                .any(|present| present.name == xattr.name && present.value == xattr.value);
            if !unchanged && self.target.set_xattr(&xattr.name, &xattr.value).is_err() {
                push_once(missing, &xattr_element(&xattr.name));
            }
        }
        for present in &current {
            let kept = wanted.iter().any(|xattr| xattr.name == present.name);
            if !kept && self.target.remove_xattr(&present.name).is_err() {
                push_once(missing, &xattr_element(&present.name));
            }
        }
        Ok(())
    }
}

/// Applies a metadata change requested by a command; the first refused
/// element fails the call.
pub(super) fn set_meta(
    target: Target<'_>,
    change: &nurt_contracts::fs::MetaChange,
    report: &Path,
) -> FsResult<()> {
    const OP: &str = "set_meta";
    if change.created_ns.is_some()
        || change.acl.is_some()
        || change.set_windows_attributes != 0
        || change.clear_windows_attributes != 0
    {
        return Err(FsError::new(FsErrorKind::Unsupported, OP, Some(report)));
    }
    if change.uid.is_some() || change.gid.is_some() {
        target
            .chown(change.uid, change.gid)
            .map_err(|errno| privileged_error(errno, OP, report))?;
    }
    if let Some(mode) = change.mode {
        target
            .chmod(mode)
            .map_err(|errno| sys_error(errno, OP, report))?;
    }
    for xattr in &change.set_xattrs {
        target
            .set_xattr(&xattr.name, &xattr.value)
            .map_err(|errno| xattr_error(errno, &xattr.name, OP, report))?;
    }
    for name in &change.remove_xattrs {
        target
            .remove_xattr(name)
            .map_err(|errno| xattr_error(errno, name, OP, report))?;
    }
    if change.accessed_ns.is_some() || change.modified_ns.is_some() {
        target
            .set_times(change.accessed_ns, change.modified_ns)
            .map_err(|errno| sys_error(errno, OP, report))?;
    }
    Ok(())
}

fn xattr_error(errno: Errno, name: &OsStr, op: &str, report: &Path) -> FsError {
    let privileged = PRIVILEGED_XATTR_PREFIXES
        .iter()
        .any(|prefix| name.as_bytes().starts_with(prefix));
    if privileged {
        privileged_error(errno, op, report)
    } else {
        sys_error(errno, op, report)
    }
}

/// Element name of an attribute in [`MetaTransferReport`]: `acl` for POSIX
/// ACL, `xattr:<name>` otherwise.
fn xattr_element(name: &OsStr) -> String {
    if name.as_bytes().starts_with(ACL_PREFIX) {
        "acl".to_owned()
    } else {
        format!("xattr:{}", name.to_string_lossy())
    }
}

fn push_once(missing: &mut Vec<String>, element: &str) {
    if !missing.iter().any(|present| present == element) {
        missing.push(element.to_owned());
    }
}

/// Inode flags occupy the low 32 bits; `FS_IOC_GETFLAGS` transfers an
/// `int`.
fn flags_from_u64(flags: u64) -> IFlags {
    IFlags::from_bits_retain(u32::try_from(flags & u64::from(u32::MAX)).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_transfer_elements_of_attributes() {
        assert_eq!(xattr_element(OsStr::new("system.posix_acl_access")), "acl");
        assert_eq!(xattr_element(OsStr::new("system.posix_acl_default")), "acl");
        assert_eq!(
            xattr_element(OsStr::new("security.selinux")),
            "xattr:security.selinux"
        );
    }

    #[test]
    fn lists_each_element_once() {
        let mut missing = Vec::new();
        push_once(&mut missing, "acl");
        push_once(&mut missing, "acl");
        push_once(&mut missing, "flags");
        assert_eq!(missing, ["acl", "flags"]);
    }

    #[test]
    fn classifies_refused_privileged_attributes_as_privilege() {
        let path = Path::new("/a");
        let error = xattr_error(
            Errno::PERM,
            OsStr::new("security.capability"),
            "set_meta",
            path,
        );
        assert_eq!(error.kind, FsErrorKind::Privilege);
        let error = xattr_error(Errno::PERM, OsStr::new("trusted.overlay"), "set_meta", path);
        assert_eq!(error.kind, FsErrorKind::Privilege);
        let error = xattr_error(Errno::PERM, OsStr::new("user.a"), "set_meta", path);
        assert_eq!(error.kind, FsErrorKind::PermissionDenied);
    }

    #[test]
    fn keeps_low_32_bits_of_flags() {
        assert_eq!(flags_from_u64(0x10).bits(), 0x10);
        assert_eq!(flags_from_u64(0x1_0000_0010).bits(), 0x10);
    }

    #[test]
    fn grows_buffer_until_value_fits() {
        let mut sizes = Vec::new();
        let value = read_growing(|buffer| {
            sizes.push(buffer.len());
            if buffer.len() < 3000 {
                Err(Errno::RANGE)
            } else {
                buffer[0] = 7;
                Ok(1)
            }
        });
        assert_eq!(value, Ok(vec![7]));
        assert_eq!(sizes, [1024, 2048, 4096]);
        let value = read_growing(|_| Err(Errno::RANGE));
        assert_eq!(value, Err(Errno::RANGE));
    }

    #[test]
    fn classifies_file_types() {
        assert_eq!(kind_from_mode(0o100_644), FileKind::File);
        assert_eq!(kind_from_mode(0o040_755), FileKind::Directory);
        assert_eq!(kind_from_mode(0o120_777), FileKind::Symlink);
        assert_eq!(kind_from_mode(0o010_644), FileKind::Special);
        assert_eq!(kind_from_mode(0o020_644), FileKind::Special);
    }
}
