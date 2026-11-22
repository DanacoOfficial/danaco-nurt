// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! System log readers for Danaco Nurt.
//!
//! The crate reads systemd journal files on Linux directly from their file
//! format and the unified log on macOS, and produces typed log records for the
//! `logs` command. No external program is run.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
