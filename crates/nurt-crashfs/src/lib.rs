// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-crashfs
// Responsibility:   Crash and fault injection harness of the Danaco Nurt journal.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Crash and fault injection harness of the Danaco Nurt journal.
//!
//! The crate simulates power loss on top of any mutating file system
//! implementation, injects system call errors, generates seeded crash points
//! and checks after recovery that the previous content of every target still
//! exists and that each operation is either complete or fully rolled back.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod crash;
pub mod inject;
pub mod oracle;
pub mod points;
pub mod runner;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
