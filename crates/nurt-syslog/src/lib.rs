// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! System log readers for Danaco Nurt.
//!
//! The crate reads systemd journal files on Linux directly from their file
//! format, the unified log on macOS and the event log on Windows, and produces
//! typed log records for the `logs` command. No external program is run.
//! Unsafe code is limited to operating system interfaces of the Windows and
//! macOS readers.

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod cursor;
pub mod event;
pub mod eventlog;
pub mod filter;
pub mod journald;
pub mod limits;
pub mod merge;
pub mod query;
pub mod unified;
