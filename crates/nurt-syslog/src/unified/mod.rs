// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-syslog::unified
// Responsibility:   Reader of the macOS unified log.
// Dependencies:     logarchive, oslog
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reader of the macOS unified log.
//!
//! Entries are read from a backend over a log store, newest first, narrowed
//! by a predicate built from the parameters of `logs`. Only log messages are
//! returned. Activities and signposts are skipped, and messages the system
//! reports as lost are counted. Every entry passes the final filter of the
//! crate.
//!
//! On macOS two backends exist. `OsLogBackend` reads the local log store, or
//! a `.logarchive` directory, through the `OSLog` framework. `ArchiveBackend`
//! parses a `.logarchive` directory with the `macos-unifiedlogs` library and
//! does not depend on the framework. The modules that build predicates, map
//! levels, entries and archive records and continue from cursors do not call
//! the operating system and are compiled on every platform.
//!
//! | Parameter of `logs` | Unified log |
//! |---|---|
//! | `unit` | subsystem |
//! | `source` | process |
//! | `level` | threshold on the type of the message |
//! | `since`, `until` | time of the entry |
//! | `message` | composed message |

pub mod archive;
pub mod backend;
pub mod entry;
pub mod level;
#[cfg(target_os = "macos")]
mod logarchive;
#[cfg(target_os = "macos")]
mod oslog;
pub mod predicate;
pub mod store;

#[cfg(target_os = "macos")]
pub use logarchive::ArchiveBackend;
#[cfg(target_os = "macos")]
pub use oslog::{OsLogBackend, OsLogItems};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
