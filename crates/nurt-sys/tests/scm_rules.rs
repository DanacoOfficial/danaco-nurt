// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rules of the Windows service backend that do not call the system: state
//! and start type mapping and the restart sequence, tested on every system.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(
    dead_code,
    reason = "the module is shared with the Windows backend, which uses items the tests do not"
)]
#[path = "../src/services/windows/scm.rs"]
mod scm;
