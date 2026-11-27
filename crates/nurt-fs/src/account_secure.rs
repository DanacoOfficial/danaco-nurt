// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs.account_secure
// Responsibility:   Creates journal files, directories and links for an account through checked directory descriptors and hands them to the account before use.
// Dependencies:     libc, std
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl
//! Creation of journal files for an account without a file system identity
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

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

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
    // SAFETY: `fd` is a descriptor that the caller has just obtained from a
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
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
