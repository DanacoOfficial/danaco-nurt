// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs.account_secure
// Responsibility:   Creates, reads, moves and removes journal entries of an account through checked directory descriptors and hands new entries to the account before use.
// Dependencies:     libc, rustix, xattr, std
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl
//! Access to journal entries of an account without a file system identity
//! (Unix systems other than Linux, and the portable path on Linux).
//!
//! The process has the rights of the administrator, so it must neither follow
//! a link that the account placed in its journal nor leave a file owned by the
//! administrator there. Every directory from the state directory of the
//! account (or from the first journal directory) down is opened with
//! `openat(O_NOFOLLOW | O_DIRECTORY)` relative to the directory descriptor
//! that was opened before it and must be owned by the account. A file is then
//! created with `openat(O_CREAT | O_EXCL | O_NOFOLLOW)` relative to the
//! descriptor of its directory and given to the account with `fchown` and
//! `fchmod` through the open descriptor, before anything is written; a
//! directory is created with `mkdirat`, opened without following links and
//! given to the account the same way. A name that already exists, whatever it
//! is, makes the creation fail.
//!
//! Reading, renaming, linking and removing follow the same rule: the entry is
//! named relative to the descriptor of its checked directory and is opened,
//! examined (`fstatat` with `AT_SYMLINK_NOFOLLOW`), renamed (`renameat`),
//! linked (`linkat`) or removed (`unlinkat`) through that descriptor, with no
//! second walk of the path. A directory of the journal that the account
//! replaces by a link after the check cannot redirect the call, because the
//! call no longer resolves the directory by name.

use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};

use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, RenameFlags};
use rustix::io::Errno;
use xattr::FileExt;

/// Name that marks a journal directory.
const JOURNAL_DIR: &str = ".nurt-journal";

/// The account that owns the journal.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Owner<'a> {
    pub uid: u32,
    pub gid: u32,
    pub state_dir: &'a Path,
}

fn cstring(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn check(code: libc::c_int) -> io::Result<libc::c_int> {
    if code < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(code)
    }
}

fn owned(fd: libc::c_int) -> OwnedFd {
    // SAFETY: `fd` is a descriptor that the caller has obtained from a
    // successful system call and that nothing else owns.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn open_at(dir: libc::c_int, name: &OsStr, flags: libc::c_int) -> io::Result<OwnedFd> {
    let name = cstring(name)?;
    // SAFETY: `name` is a valid C string; `dir` is an open descriptor.
    let fd = check(unsafe { libc::openat(dir, name.as_ptr(), flags | libc::O_CLOEXEC, 0o600) })?;
    Ok(owned(fd))
}

fn verify_directory(fd: &OwnedFd, owner: &Owner<'_>) -> io::Result<()> {
    // SAFETY: an all-zero `stat` is a valid buffer for `fstat` to fill.
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is open and `info` is a valid buffer.
    check(unsafe { libc::fstat(fd.as_raw_fd(), &mut info) })?;
    if info.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(io::Error::from_raw_os_error(libc::ENOTDIR));
    }
    if info.st_uid != owner.uid {
        return Err(io::Error::from_raw_os_error(libc::EPERM));
    }
    Ok(())
}

/// Index of the first component below which links are not followed and
/// directories must belong to the account.
fn strict_from(owner: &Owner<'_>, path: &Path) -> Option<usize> {
    let names: Vec<_> = path.components().collect();
    let by_state = path
        .starts_with(owner.state_dir)
        .then(|| owner.state_dir.components().count().saturating_sub(1));
    let by_journal = names.iter().position(|part| {
        matches!(part, Component::Normal(name) if name.to_string_lossy().starts_with(JOURNAL_DIR))
    });
    match (by_state, by_journal) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Opens `dir`, an absolute path, and returns the descriptor of the directory.
///
/// # Errors
///
/// Fails when a component below the state directory or a journal directory is
/// a link, is not a directory or does not belong to the account.
pub(crate) fn open_dir(owner: &Owner<'_>, dir: &Path) -> io::Result<OwnedFd> {
    if !dir.is_absolute() {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    let strict = strict_from(owner, dir);
    let mut current = open_at(
        libc::AT_FDCWD,
        OsStr::new("/"),
        libc::O_RDONLY | libc::O_DIRECTORY,
    )?;
    for (index, part) in dir.components().enumerate() {
        let name = match part {
            Component::Normal(name) => name,
            Component::RootDir => continue,
            _ => return Err(io::Error::from_raw_os_error(libc::EINVAL)),
        };
        let guarded = strict.is_some_and(|from| index >= from);
        let mut flags = libc::O_RDONLY | libc::O_DIRECTORY;
        if guarded {
            flags |= libc::O_NOFOLLOW;
        }
        let next = open_at(current.as_raw_fd(), name, flags)?;
        if guarded {
            verify_directory(&next, owner)?;
        }
        current = next;
    }
    Ok(current)
}

fn give_to_account(fd: libc::c_int, owner: &Owner<'_>, mode: u32) -> io::Result<()> {
    // SAFETY: `fd` is an open descriptor; the arguments are plain numbers.
    check(unsafe { libc::fchown(fd, owner.uid, owner.gid) })?;
    // SAFETY: as above.
    check(unsafe { libc::fchmod(fd, mode as libc::mode_t) })?;
    Ok(())
}

fn unlink_at(dir: libc::c_int, name: &OsStr, flags: libc::c_int) {
    if let Ok(name) = cstring(name) {
        // SAFETY: `name` is a valid C string; `dir` is an open descriptor.
        unsafe { libc::unlinkat(dir, name.as_ptr(), flags) };
    }
}

/// Creates the file `name` in the directory `dir`, owned by the account and
/// with `mode`, failing when the name exists.
///
/// # Errors
///
/// Fails with the error of the system call; a file that could not be given to
/// the account is removed again.
pub(crate) fn create_file_in(
    owner: &Owner<'_>,
    dir: &OwnedFd,
    name: &OsStr,
    mode: u32,
) -> io::Result<File> {
    let fd = open_at(
        dir.as_raw_fd(),
        name,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
    )?;
    if let Err(error) = give_to_account(fd.as_raw_fd(), owner, mode) {
        unlink_at(dir.as_raw_fd(), name, 0);
        return Err(error);
    }
    Ok(File::from(fd))
}

/// Creates the directory `name` in the directory `dir`, owned by the account
/// and with `mode`, failing when the name exists.
///
/// # Errors
///
/// Fails with the error of the system call; a directory that could not be
/// given to the account is removed again.
pub(crate) fn create_dir_in(
    owner: &Owner<'_>,
    dir: &OwnedFd,
    name: &OsStr,
    mode: u32,
) -> io::Result<()> {
    let text = cstring(name)?;
    // SAFETY: `text` is a valid C string; `dir` is an open descriptor.
    check(unsafe { libc::mkdirat(dir.as_raw_fd(), text.as_ptr(), 0o700) })?;
    let made = open_at(
        dir.as_raw_fd(),
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
    )
    .and_then(|fd| give_to_account(fd.as_raw_fd(), owner, mode));
    if let Err(error) = made {
        unlink_at(dir.as_raw_fd(), name, libc::AT_REMOVEDIR);
        return Err(error);
    }
    Ok(())
}

/// Creates the symbolic link `name` to `target` in the directory `dir`, owned
/// by the account (`fchownat` with `AT_SYMLINK_NOFOLLOW`), failing when the
/// name exists.
///
/// # Errors
///
/// Fails with the error of the system call; a link that could not be given to
/// the account is removed again.
pub(crate) fn symlink_in(
    owner: &Owner<'_>,
    dir: &OwnedFd,
    name: &OsStr,
    target: &Path,
) -> io::Result<()> {
    let text = cstring(name)?;
    let to = cstring(target.as_os_str())?;
    // SAFETY: both are valid C strings; `dir` is an open descriptor.
    check(unsafe { libc::symlinkat(to.as_ptr(), dir.as_raw_fd(), text.as_ptr()) })?;
    // SAFETY: as above; the link itself is changed, not what it points to.
    let given = check(unsafe {
        libc::fchownat(
            dir.as_raw_fd(),
            text.as_ptr(),
            owner.uid,
            owner.gid,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    });
    if let Err(error) = given {
        unlink_at(dir.as_raw_fd(), name, 0);
        return Err(error);
    }
    Ok(())
}

/// Splits `path` into its directory and its last component.
///
/// # Errors
///
/// Fails when the path has no parent or no file name.
pub(crate) fn split(path: &Path) -> io::Result<(&Path, &OsStr)> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => Ok((parent, name)),
        _ => Err(io::Error::from_raw_os_error(libc::EINVAL)),
    }
}

fn from_errno(errno: Errno) -> io::Error {
    io::Error::from_raw_os_error(errno.raw_os_error())
}

/// Repeats a system call that a signal interrupted, at most eight times.
fn retried<T>(mut call: impl FnMut() -> rustix::io::Result<T>) -> io::Result<T> {
    let mut attempts = 1;
    loop {
        match call() {
            Err(Errno::INTR) if attempts < 8 => attempts += 1,
            result => return result.map_err(from_errno),
        }
    }
}

/// `fstatat` of the entry `name` of `dir`; a link in the last component is
/// reported, not followed.
fn stat_entry(dir: &OwnedFd, name: &OsStr) -> io::Result<libc::stat> {
    let text = cstring(name)?;
    // SAFETY: an all-zero `stat` is a valid buffer for `fstatat` to fill.
    let mut info: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `text` is a valid C string, `dir` is an open descriptor and
    // `info` is a valid buffer.
    check(unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            text.as_ptr(),
            &raw mut info,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    })?;
    Ok(info)
}

fn is_directory(info: &libc::stat) -> bool {
    info.st_mode & libc::S_IFMT == libc::S_IFDIR
}

/// Opens the entry `name` of `dir` with `flags`, never following a link in the
/// last component.
///
/// # Errors
///
/// Fails with the error of `openat`; a link in the place of the entry gives
/// `ELOOP` (`ENOTDIR` for a directory open).
pub(crate) fn open_entry(dir: &OwnedFd, name: &OsStr, flags: libc::c_int) -> io::Result<OwnedFd> {
    open_at(
        dir.as_raw_fd(),
        name,
        flags | libc::O_NOFOLLOW | libc::O_NOCTTY,
    )
}

/// Opens the file `name` of `dir` for reading without waiting for a writer of
/// a pipe and without following a link.
///
/// # Errors
///
/// Fails with the error of `openat`.
pub(crate) fn open_file_for_read(dir: &OwnedFd, name: &OsStr) -> io::Result<File> {
    open_entry(dir, name, libc::O_RDONLY | libc::O_NONBLOCK).map(File::from)
}

/// Opens the file `name` of `dir` for writing in place without following a
/// link.
///
/// # Errors
///
/// Fails with the error of `openat`.
pub(crate) fn open_file_for_write(dir: &OwnedFd, name: &OsStr) -> io::Result<File> {
    open_entry(dir, name, libc::O_WRONLY).map(File::from)
}

/// Target of the link `name` of `dir`, as stored in the link.
///
/// # Errors
///
/// Fails with the error of `readlinkat`; `EINVAL` when the entry is not a link.
pub(crate) fn read_link_in(dir: &OwnedFd, name: &OsStr) -> io::Result<PathBuf> {
    let target = retried(|| rustix::fs::readlinkat(dir, name, Vec::new()))?;
    Ok(PathBuf::from(OsString::from_vec(target.into_bytes())))
}

/// How [`rename_in`] treats an existing destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameHow {
    /// Replace the destination.
    Replace,
    /// Fail with `EEXIST` when the destination exists.
    NoReplace,
    /// Swap the two names.
    Exchange,
}

/// Renames `from` of `from_dir` to `to` of `to_dir` (`renameat`,
/// `renameat2` or `renamex_np` through `rustix`).
///
/// Where the file system refuses `RENAME_NOREPLACE`, a non-directory is moved
/// with `linkat` followed by `unlinkat`, both relative to the same
/// descriptors.
///
/// # Errors
///
/// Fails with the error of the system call; `EOPNOTSUPP` when an exchange or a
/// no-replace move of a directory is not available.
pub(crate) fn rename_in(
    from_dir: &OwnedFd,
    from: &OsStr,
    to_dir: &OwnedFd,
    to: &OsStr,
    how: RenameHow,
) -> io::Result<()> {
    let flags = match how {
        RenameHow::Replace => RenameFlags::empty(),
        RenameHow::NoReplace => RenameFlags::NOREPLACE,
        RenameHow::Exchange => RenameFlags::EXCHANGE,
    };
    let result = retried(|| {
        if flags.is_empty() {
            rustix::fs::renameat(from_dir, from, to_dir, to)
        } else {
            rustix::fs::renameat_with(from_dir, from, to_dir, to, flags)
        }
    });
    match (result, how) {
        (Err(error), RenameHow::NoReplace)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)
            ) =>
        {
            move_without_replacing(from_dir, from, to_dir, to)
        }
        (Err(error), RenameHow::Exchange)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)
            ) =>
        {
            Err(io::Error::from_raw_os_error(libc::EOPNOTSUPP))
        }
        (result, _) => result,
    }
}

fn move_without_replacing(
    from_dir: &OwnedFd,
    from: &OsStr,
    to_dir: &OwnedFd,
    to: &OsStr,
) -> io::Result<()> {
    if is_directory(&stat_entry(from_dir, from)?) {
        return Err(io::Error::from_raw_os_error(libc::EOPNOTSUPP));
    }
    retried(|| rustix::fs::linkat(from_dir, from, to_dir, to, AtFlags::empty()))?;
    if let Err(error) = retried(|| rustix::fs::unlinkat(from_dir, from, AtFlags::empty())) {
        let _ = rustix::fs::unlinkat(to_dir, to, AtFlags::empty());
        return Err(error);
    }
    Ok(())
}

/// Gives the existing entry `existing` of `existing_dir` the additional name
/// `new` in `new_dir` (`linkat`, a link in the last component is not
/// followed).
///
/// # Errors
///
/// Fails with the error of `linkat`.
pub(crate) fn link_in(
    existing_dir: &OwnedFd,
    existing: &OsStr,
    new_dir: &OwnedFd,
    new: &OsStr,
) -> io::Result<()> {
    retried(|| rustix::fs::linkat(existing_dir, existing, new_dir, new, AtFlags::empty()))
}

/// Removes the entry `name` of `dir`: a directory when `directory` is set, any
/// other entry otherwise (`unlinkat`).
///
/// # Errors
///
/// Fails with the error of `unlinkat`.
pub(crate) fn unlink_entry(dir: &OwnedFd, name: &OsStr, directory: bool) -> io::Result<()> {
    let flags = if directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    retried(|| rustix::fs::unlinkat(dir, name, flags))
}

/// What a recursive removal took away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Removed {
    pub files: u64,
    pub directories: u64,
    pub bytes: u64,
}

/// Deepest tree that a recursive removal enters.
const TREE_DEPTH_MAX: usize = 256;

/// Removes the entry `name` of `dir` with everything below it. A link is
/// removed, never followed; every subdirectory is opened relative to its
/// parent with `O_NOFOLLOW`; a mount point inside the tree ends the removal
/// with `EBUSY`.
///
/// # Errors
///
/// Fails with the error of the first system call that failed; the entries
/// removed before it stay removed.
pub(crate) fn remove_tree_in(dir: &OwnedFd, name: &OsStr) -> io::Result<Removed> {
    remove_entry(dir, name, 0)
}

fn remove_entry(dir: &OwnedFd, name: &OsStr, depth: usize) -> io::Result<Removed> {
    let info = stat_entry(dir, name)?;
    let mut total = Removed::default();
    if !is_directory(&info) {
        unlink_entry(dir, name, false)?;
        total.files = 1;
        total.bytes = u64::try_from(info.st_size).unwrap_or(0);
        return Ok(total);
    }
    if depth >= TREE_DEPTH_MAX {
        return Err(io::Error::from_raw_os_error(libc::ELOOP));
    }
    let child = open_entry(dir, name, libc::O_RDONLY | libc::O_DIRECTORY)?;
    let mut opened: libc::stat = {
        // SAFETY: an all-zero `stat` is a valid buffer for `fstat` to fill.
        unsafe { std::mem::zeroed() }
    };
    // SAFETY: `child` is open and `opened` is a valid buffer.
    check(unsafe { libc::fstat(child.as_raw_fd(), &raw mut opened) })?;
    if opened.st_dev != info.st_dev {
        return Err(io::Error::from_raw_os_error(libc::EBUSY));
    }
    for (entry, _) in list_entries(&child)? {
        match remove_entry(&child, &entry, depth + 1) {
            Ok(removed) => {
                total.files += removed.files;
                total.directories += removed.directories;
                total.bytes += removed.bytes;
            }
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {}
            Err(error) => return Err(error),
        }
    }
    drop(child);
    unlink_entry(dir, name, true)?;
    total.directories += 1;
    Ok(total)
}

/// Names and types of the entries of the open directory `dir`, without `.` and
/// `..`.
///
/// # Errors
///
/// Fails with the error of reading the directory.
pub(crate) fn list_entries(dir: &OwnedFd) -> io::Result<Vec<(OsString, FileType)>> {
    let reader = Dir::read_from(dir).map_err(from_errno)?;
    let mut entries = Vec::new();
    for entry in reader {
        let entry = entry.map_err(from_errno)?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        entries.push((OsString::from_vec(name.to_vec()), entry.file_type()));
    }
    Ok(entries)
}

/// Opens the lock file `name` of `dir`, creating it for the account when it
/// does not exist, and refuses an entry that is not a regular file.
///
/// # Errors
///
/// Fails with the error of the system call.
pub(crate) fn open_lock_file(owner: &Owner<'_>, dir: &OwnedFd, name: &OsStr) -> io::Result<File> {
    match create_file_in(owner, dir, name, 0o600) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = open_file_for_read(dir, name)?;
            if !existing.metadata()?.is_file() {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            }
            Ok(existing)
        }
        other => other,
    }
}

/// Takes the exclusive lock of `file` without waiting; `false` when another
/// holder has it.
///
/// # Errors
///
/// Fails with the error of `flock` other than the lock being held.
pub(crate) fn try_lock_exclusive(file: &File) -> io::Result<bool> {
    match retried(|| rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive)) {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == Some(libc::EWOULDBLOCK) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Extended attributes of the open file, sorted by name; empty when the file
/// system has none.
///
/// # Errors
///
/// Fails with the error of listing or reading an attribute.
pub(crate) fn read_xattrs_of(file: &File) -> io::Result<Vec<(OsString, Vec<u8>)>> {
    let names = match file.list_xattr() {
        Ok(names) => names,
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::ENOTSUP
                    || code == libc::EOPNOTSUPP
                    || code == libc::EPERM
            ) =>
        {
            return Ok(Vec::new());
        }
        Err(error) => return Err(error),
    };
    let mut found = Vec::new();
    for name in names {
        if let Some(value) = file.get_xattr(&name)? {
            found.push((name, value));
        }
    }
    found.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    fn me(state: &Path) -> Owner<'_> {
        // SAFETY: `geteuid` and `getegid` take no arguments and cannot fail.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        Owner {
            uid,
            gid,
            state_dir: state,
        }
    }

    #[test]
    fn a_file_and_a_directory_are_given_to_the_account_with_their_modes() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        create_dir_in(&owner, &dir, OsStr::new(".nurt-journal"), 0o750).unwrap();
        let journal = open_dir(&owner, &state.join(".nurt-journal")).unwrap();
        let file = create_file_in(&owner, &journal, OsStr::new("op.json"), 0o640).unwrap();
        drop(file);
        let dir_meta = std::fs::metadata(state.join(".nurt-journal")).unwrap();
        let file_meta = std::fs::metadata(state.join(".nurt-journal/op.json")).unwrap();
        assert_eq!(dir_meta.permissions().mode() & 0o777, 0o750);
        assert_eq!(file_meta.permissions().mode() & 0o777, 0o640);
        assert_eq!((file_meta.uid(), file_meta.gid()), (owner.uid, owner.gid));
        assert_eq!(dir_meta.uid(), owner.uid);
    }

    #[test]
    fn an_existing_name_or_a_link_in_its_place_is_not_used() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"target").unwrap();
        std::os::unix::fs::symlink(&outside, state.join("link")).unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        assert!(create_file_in(&owner, &dir, OsStr::new("link"), 0o600).is_err());
        assert!(create_dir_in(&owner, &dir, OsStr::new("link"), 0o700).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"target");
        create_file_in(&owner, &dir, OsStr::new("once"), 0o600).unwrap();
        assert!(create_file_in(&owner, &dir, OsStr::new("once"), 0o600).is_err());
    }

    #[test]
    fn a_link_in_the_chain_below_the_state_directory_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let elsewhere = root.path().join("elsewhere");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, state.join(".nurt-journal")).unwrap();
        let owner = me(&state);
        let error = open_dir(&owner, &state.join(".nurt-journal")).unwrap_err();
        assert_ne!(error.raw_os_error(), None);
        assert!(open_dir(&owner, &state.join(".nurt-journal/ops")).is_err());
        // Above the state directory a link is followed: it is not the account's.
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&state, &alias).unwrap();
        let other = Owner {
            state_dir: &state,
            ..owner
        };
        assert!(open_dir(&other, &alias).is_ok());
    }

    #[test]
    fn a_directory_of_another_owner_below_the_state_directory_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let mut owner = me(&state);
        owner.uid = owner.uid.wrapping_add(1);
        assert!(open_dir(&owner, &state).is_err());
    }

    #[test]
    fn a_link_is_created_owned_by_the_account_and_not_over_a_name() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        symlink_in(&owner, &dir, OsStr::new("link"), Path::new("/nowhere")).unwrap();
        let meta = std::fs::symlink_metadata(state.join("link")).unwrap();
        assert!(meta.file_type().is_symlink());
        assert_eq!(meta.uid(), owner.uid);
        assert!(symlink_in(&owner, &dir, OsStr::new("link"), Path::new("/else")).is_err());
    }

    /// Journal directory `ops` that the account swaps for a link to `outside`
    /// after its descriptor was opened: the descriptor, the directory that
    /// moved away and the directory that the link points to.
    fn swapped_directory(root: &Path, owner: &Owner<'_>) -> (OwnedFd, PathBuf, PathBuf) {
        let state = root.join("state");
        let journal = state.join(".nurt-journal");
        let ops = journal.join("ops");
        let outside = root.join("outside");
        std::fs::create_dir_all(&ops).unwrap();
        std::fs::create_dir(&outside).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(ops.join(name), b"inside").unwrap();
            std::fs::write(outside.join(name), b"outside").unwrap();
        }
        let checked = open_dir(owner, &ops).unwrap();
        let moved = journal.join("ops-moved");
        std::fs::rename(&ops, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &ops).unwrap();
        (checked, moved, outside)
    }

    #[test]
    fn a_read_after_the_swap_of_the_directory_for_a_link_stays_in_the_checked_directory() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let owner = me(&state);
        let (checked, _moved, _outside) = swapped_directory(root.path(), &owner);
        let mut content = String::new();
        std::io::Read::read_to_string(
            &mut open_file_for_read(&checked, OsStr::new("a")).unwrap(),
            &mut content,
        )
        .unwrap();
        assert_eq!(content, "inside");
        let info = stat_entry(&checked, OsStr::new("b")).unwrap();
        assert_eq!(info.st_size, 6);
        let names: Vec<_> = list_entries(&checked)
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names.len(), 3);
        assert!(open_dir(&owner, &state.join(".nurt-journal/ops")).is_err());
    }

    #[test]
    fn a_move_after_the_swap_of_the_directory_for_a_link_leaves_the_outside_alone() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let owner = me(&state);
        let (checked, moved, outside) = swapped_directory(root.path(), &owner);
        rename_in(
            &checked,
            OsStr::new("a"),
            &checked,
            OsStr::new("moved"),
            RenameHow::Replace,
        )
        .unwrap();
        rename_in(
            &checked,
            OsStr::new("b"),
            &checked,
            OsStr::new("c"),
            RenameHow::Exchange,
        )
        .unwrap();
        link_in(
            &checked,
            OsStr::new("moved"),
            &checked,
            OsStr::new("linked"),
        )
        .unwrap();
        assert!(!moved.join("a").exists());
        assert_eq!(std::fs::read(moved.join("moved")).unwrap(), b"inside");
        assert_eq!(std::fs::read(moved.join("linked")).unwrap(), b"inside");
        for name in ["a", "b", "c"] {
            assert_eq!(std::fs::read(outside.join(name)).unwrap(), b"outside");
        }
        assert!(!outside.join("moved").exists());
        assert!(!outside.join("linked").exists());
    }

    #[test]
    fn a_removal_after_the_swap_of_the_directory_for_a_link_leaves_the_outside_alone() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let owner = me(&state);
        let (checked, moved, outside) = swapped_directory(root.path(), &owner);
        std::fs::create_dir(moved.join("tree")).unwrap();
        std::fs::write(moved.join("tree/leaf"), b"leaf").unwrap();
        std::fs::create_dir(outside.join("tree")).unwrap();
        std::fs::write(outside.join("tree/leaf"), b"outside").unwrap();
        unlink_entry(&checked, OsStr::new("a"), false).unwrap();
        let removed = remove_tree_in(&checked, OsStr::new("tree")).unwrap();
        assert_eq!(
            (removed.files, removed.directories, removed.bytes),
            (1, 1, 4)
        );
        assert!(!moved.join("a").exists());
        assert!(!moved.join("tree").exists());
        assert!(outside.join("a").exists());
        assert_eq!(
            std::fs::read(outside.join("tree/leaf")).unwrap(),
            b"outside"
        );
    }

    #[test]
    fn a_removal_of_a_tree_removes_a_link_inside_it_and_not_what_it_points_to() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let tree = state.join("tree");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(tree.join("deeper")).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("deeper/link")).unwrap();
        std::fs::write(tree.join("file"), b"12345").unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        let removed = remove_tree_in(&dir, OsStr::new("tree")).unwrap();
        assert_eq!(removed.directories, 2);
        assert_eq!(removed.files, 2);
        assert!(!tree.exists());
        assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn a_link_in_the_place_of_the_entry_is_not_opened_for_reading_or_writing() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, state.join("link")).unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        assert!(open_file_for_read(&dir, OsStr::new("link")).is_err());
        assert!(open_file_for_write(&dir, OsStr::new("link")).is_err());
        assert!(open_lock_file(&owner, &dir, OsStr::new("link")).is_err());
        assert_eq!(read_link_in(&dir, OsStr::new("link")).unwrap(), outside);
        unlink_entry(&dir, OsStr::new("link"), false).unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }

    #[test]
    fn a_move_without_replacing_keeps_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("from"), b"from").unwrap();
        std::fs::write(state.join("to"), b"to").unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        let refused = rename_in(
            &dir,
            OsStr::new("from"),
            &dir,
            OsStr::new("to"),
            RenameHow::NoReplace,
        )
        .unwrap_err();
        assert_eq!(refused.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(std::fs::read(state.join("to")).unwrap(), b"to");
        rename_in(
            &dir,
            OsStr::new("from"),
            &dir,
            OsStr::new("fresh"),
            RenameHow::NoReplace,
        )
        .unwrap();
        assert!(!state.join("from").exists());
        assert_eq!(std::fs::read(state.join("fresh")).unwrap(), b"from");
    }

    #[test]
    fn the_lock_is_taken_once_and_the_attributes_of_a_file_are_read() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let owner = me(&state);
        let dir = open_dir(&owner, &state).unwrap();
        let first = open_lock_file(&owner, &dir, OsStr::new("lock")).unwrap();
        let second = open_lock_file(&owner, &dir, OsStr::new("lock")).unwrap();
        assert!(try_lock_exclusive(&first).unwrap());
        assert!(!try_lock_exclusive(&second).unwrap());
        drop(first);
        // A process started by another test between the open and the close of
        // the descriptor can hold the lock until it executes its program.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !try_lock_exclusive(&second).unwrap() {
            assert!(std::time::Instant::now() < deadline, "the lock stayed held");
            std::thread::yield_now();
        }
        assert!(read_xattrs_of(&second).is_ok());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
