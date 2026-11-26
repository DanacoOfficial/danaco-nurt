// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Crash recovery tests of the journal.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods,
    unreachable_pub,
    reason = "test fixtures assert by unwrapping, create transaction scopes for the model and share public items between the modules of one test binary"
)]

mod capacity;
mod corpus;
mod failpoints;
mod inject;
mod journal;
mod journal_faults;
mod journal_many;
mod journal_prior;
mod journal_reclaim;
mod journal_scale;
mod journal_undo;
mod model;
mod oracle;
mod reference;
mod runner;
mod support;
