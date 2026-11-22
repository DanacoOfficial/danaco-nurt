// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

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
