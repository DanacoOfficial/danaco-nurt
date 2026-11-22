// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Execution planning of bound Danaco Nurt programs.
//!
//! The planner turns a bound program into an execution plan: bindings,
//! streaming pipeline stages, the optional changing stage and its effect, and
//! the checks that run before and after the change.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
