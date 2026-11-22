// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Single-instance lock of the service.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

use fs4::{FileExt, TryLockError};

/// Exclusive lock on `service.lock`, held for the lifetime of the service.
///
/// The lock is an advisory whole-file lock (`flock` on Unix, `LockFileEx` on
/// Windows). The operating system releases it when the process ends, so a
/// crashed service never leaves a stale lock behind.
#[derive(Debug)]
pub(crate) struct InstanceLock {
    file: File,
}

/// Outcome of an attempt to take the instance lock.
#[derive(Debug)]
pub(crate) enum LockOutcome {
    /// The lock is held by this process.
    Acquired(InstanceLock),
    /// Another process holds the lock.
    Busy,
}

impl InstanceLock {
    /// Takes the lock without waiting, creating the state directory and the
    /// lock file when they do not exist.
    pub(crate) fn try_acquire(path: &Path) -> io::Result<LockOutcome> {
        if let Some(dir) = path.parent() {
            create_private_dir(dir)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match FileExt::try_lock(&file) {
            Ok(()) => Ok(LockOutcome::Acquired(Self { file })),
            Err(TryLockError::WouldBlock) => Ok(LockOutcome::Busy),
            Err(TryLockError::Error(error)) => Err(error),
        }
    }

    /// Releases the lock.
    pub(crate) fn release(self) {
        let _released_on_close = FileExt::unlock(&self.file);
    }
}

/// Creates a directory readable only by the account (mode `0700` on Unix).
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    restrict_dir(dir)
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    const OWNER_ONLY_DIR: u32 = 0o700;
    fs::set_permissions(dir, fs::Permissions::from_mode(OWNER_ONLY_DIR))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_exclusive_until_released() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("service.lock");
        let LockOutcome::Acquired(first) = InstanceLock::try_acquire(&path).unwrap() else {
            panic!("first attempt must acquire the lock");
        };
        assert!(matches!(
            InstanceLock::try_acquire(&path).unwrap(),
            LockOutcome::Busy
        ));
        first.release();
        assert!(matches!(
            InstanceLock::try_acquire(&path).unwrap(),
            LockOutcome::Acquired(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn state_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        create_private_dir(&state).unwrap();
        let mode = fs::metadata(&state).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}
