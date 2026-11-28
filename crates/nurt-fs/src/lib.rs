// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs
// Responsibility:   Platform-independent file system layer of Danaco Nurt.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Platform-independent file system layer of Danaco Nurt.
//!
//! The crate implements the file system interface defined in `nurt_contracts`
//! on Linux, macOS and Windows: canonical paths, atomic replacement, name
//! exchange, clones, links, locks, extended attributes, access control lists,
//! volume information and path providers. It is the only crate that issues
//! file system calls of the operating system directly; unsafe code is limited
//! to those calls and every block documents its safety conditions.

#![deny(missing_docs)]
#![allow(unsafe_code, reason = "file system calls of the operating system")]
#![deny(clippy::undocumented_unsafe_blocks)]

#[cfg(unix)]
pub mod account;
#[cfg(unix)]
mod account_secure;
#[cfg(feature = "archive")]
pub mod archive;
#[cfg(feature = "mutation-audit")]
pub mod audit;
pub mod clock;
#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
pub mod failpoint;
pub mod key;
pub mod linux;
pub mod macos;
pub mod mem;
pub mod path;
pub mod transfer;
pub mod windows;

/// File system back end of the running operating system.
///
/// The journal, the space service and the commands share one back end, so it
/// is returned as a shared trait object; commands that only read use it
/// through [`Fs`](nurt_contracts::fs::Fs) by trait upcasting.
///
/// # Examples
///
/// ```
/// use nurt_contracts::fs::{AbsPath, Fs, FsError};
///
/// let fs = nurt_fs::system_fs();
/// let dir = AbsPath::new(std::env::temp_dir())?;
/// assert!(fs.can_write_dir(&dir)?);
/// assert_eq!(fs.volume(&dir)?.id, fs.volume(&dir)?.id);
/// # Ok::<(), FsError>(())
/// ```
#[must_use]
pub fn system_fs() -> std::sync::Arc<dyn nurt_contracts::fs::FsWrite> {
    #[cfg(target_os = "linux")]
    let backend = std::sync::Arc::new(linux::LinuxFs::new());
    #[cfg(target_os = "macos")]
    let backend = std::sync::Arc::new(macos::MacosFs::new());
    #[cfg(windows)]
    let backend = std::sync::Arc::new(windows::WindowsFs::new());
    backend
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::AbsPath;

    #[test]
    fn system_fs_describes_the_volume_of_a_writable_directory() {
        let fs = super::system_fs();
        let dir = tempfile::tempdir().expect("temporary directory");
        let root = AbsPath::new(dir.path()).expect("absolute path");
        assert!(fs.can_write_dir(&root).expect("own directory"));
        let volume = fs.volume(&root).expect("volume");
        assert!(root.starts_with(&volume.mount));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
