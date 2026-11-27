// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-journal::retention
// Responsibility:   Retention limits and expiry.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Retention limits and expiry.
//!
//! An entry expires when any of three limits is exceeded (steps, age, size)
//! or when the user needs the space. Limits are resolved from the seven
//! configuration levels for every volume ([`resolve`]), the age of an entry
//! is measured by the selected clock ([`age`]), and expiry passes remove
//! entries in a deterministic order ([`expiry`]), leaving a tombstone for
//! `history` ([`tomb`]). [`limits`] serves `journal.limits` and
//! `journal.status`, the only place where retention state is reported.

pub(crate) mod age;
pub(crate) mod ctx;
pub(crate) mod expiry;
pub(crate) mod limits;
pub(crate) mod port;
pub(crate) mod resolve;
#[cfg(test)]
mod testing;
pub(crate) mod tomb;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
