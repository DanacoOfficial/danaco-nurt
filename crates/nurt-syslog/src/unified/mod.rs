// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reader of the macOS unified log.
//!
//! Entries are read from a backend over the log store of the system, newest
//! first, narrowed by a predicate built from the parameters of `logs`. Only
//! log messages are returned. Activities and signposts are skipped, and
//! messages the system reports as lost are counted. Every entry passes the
//! final filter of the crate. The modules that build predicates, map levels
//! and entries and continue from cursors do not call the operating system and
//! are compiled on every platform.
//!
//! | Parameter of `logs` | Unified log |
//! |---|---|
//! | `unit` | subsystem |
//! | `source` | process |
//! | `level` | threshold on the type of the message |
//! | `since`, `until` | time of the entry |
//! | `message` | composed message |

pub mod backend;
pub mod entry;
pub mod level;
pub mod predicate;
pub mod store;
