// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/coverage/harness/mod.rs
// Responsibility:   Harness shared by the journal test suites.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Harness shared by the journal test suites: a fixture that binds the journal
//! under test to simulated or mounted volumes, tree snapshots for round-trip
//! comparisons, a virtual clock and builders of journal requests.

pub(crate) mod clock;
pub(crate) mod fixture;
pub(crate) mod request;
pub(crate) mod services;
pub(crate) mod snapshot;
pub(crate) mod volumes;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
