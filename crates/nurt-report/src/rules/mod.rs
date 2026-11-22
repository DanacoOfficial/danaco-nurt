// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rules for notable effects and irreversibility.
//!
//! The `notable` rules derive codes such as `mass_delete`, `file_emptied`,
//! `size_shrunk` and `system_path` from the changes of an operation and the
//! thresholds of the `report.notable` configuration, merge them with the codes
//! emitted by commands and order them by severity. The `irreversible` rules
//! collect the reasons that make an effect irreversible.
//!
//! The rules describe effects; they never block an operation.

mod format;
mod hits;
mod irreversible;
mod notable;
pub(crate) mod paths;

pub use hits::ENTRIES_PER_CODE;
pub use irreversible::collect_irreversible;
pub use notable::{ALERT_FACTOR, SHRINK_MIN_BYTES, derive_notable, merge_notable};
pub use paths::{PathContext, is_fs_path};
