// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Rules for notable effects and irreversibility.
//!
//! The `notable` rules derive codes such as `mass_delete`, `file_emptied`,
//! `size_shrunk` and `system_path` from the changes of an operation and the
//! thresholds of the `report.notable` configuration, and order them by
//! severity. The `irreversible` rules collect the reasons that make an effect
//! irreversible.
