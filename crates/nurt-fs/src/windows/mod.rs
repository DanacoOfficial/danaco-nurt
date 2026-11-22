// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Windows back end.
//!
//! `WindowsFs` (available on Windows) implements the file system interface
//! with Win32 calls on verbatim paths: replacement with a backup name through
//! `ReplaceFileW`, renames with `MoveFileExW(MOVEFILE_WRITE_THROUGH)` or with
//! POSIX semantics, NTFS alternate data streams, security descriptors, the
//! Restart Manager for processes holding files, block cloning on ReFS and
//! bounded retries after sharing violations.
//!
//! The submodules other than the system layer contain the platform-independent
//! rules of the back end (error classification, path forms, metadata
//! conversion, volume classification, retry schedule, security descriptor and
//! reparse data layouts). They compile on every system, so the rules are
//! tested on any build host.

pub mod error;
pub mod meta;
pub mod path;
pub mod reparse;
pub mod replace;
pub mod retry;
pub mod security;
pub mod volume;

#[cfg(windows)]
mod sys;

#[cfg(windows)]
pub use sys::WindowsFs;
