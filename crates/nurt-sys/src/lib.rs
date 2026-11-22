// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! System inspection and administration for Danaco Nurt.
//!
//! The crate reports processes, ports, disks, host information, environment,
//! accounts, open files and system logs, and manages services, the Windows
//! registry and CIM classes on Linux, macOS and Windows. Unsafe code is limited
//! to operating system calls and every block documents its safety conditions.

#![deny(missing_docs)]
#![allow(unsafe_code, reason = "calls of the operating system")]
#![deny(clippy::undocumented_unsafe_blocks)]
