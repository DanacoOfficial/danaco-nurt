// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Single-instance lock of the service.

use std::fs::{File, OpenOptions};
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
///
/// On Unix an existing directory is tightened to `0700`. A directory of
/// another account is refused, unless the caller is `root`, which leaves it
/// as it is.
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    imp::create_private_dir(dir)
}

#[cfg(unix)]
mod imp {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::path::Path;

    const OWNER_ONLY_DIR: u32 = 0o700;
    const GROUP_AND_OTHERS: u32 = 0o077;

    pub(super) fn create_private_dir(dir: &Path) -> io::Result<()> {
        match fs::metadata(dir) {
            Ok(metadata) if metadata.is_dir() => {
                let own = nix::unistd::geteuid().as_raw();
                if metadata.uid() != own {
                    return if own == 0 {
                        Ok(())
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("{} belongs to another account", dir.display()),
                        ))
                    };
                }
                if metadata.mode() & GROUP_AND_OTHERS != 0 {
                    fs::set_permissions(dir, fs::Permissions::from_mode(OWNER_ONLY_DIR))?;
                }
                Ok(())
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a directory", dir.display()),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
                .recursive(true)
                .mode(OWNER_ONLY_DIR)
                .create(dir),
            Err(error) => Err(error),
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::fs;
    use std::io;
    use std::path::Path;

    pub(super) fn create_private_dir(dir: &Path) -> io::Result<()> {
        if dir.is_dir() {
            return Ok(());
        }
        fs::create_dir_all(dir)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;

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
    fn existing_directory_with_open_permissions_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&state).unwrap();
        let mode = fs::metadata(&state).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        create_private_dir(&state).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn nested_directories_are_created_private_and_a_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        create_private_dir(&nested).unwrap();
        for path in [dir.path().join("a"), nested] {
            let mode = fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(create_private_dir(&file).is_err());
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
