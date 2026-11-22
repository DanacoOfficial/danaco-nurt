// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Schedules for Danaco Nurt.
//!
//! The crate evaluates cron expressions, intervals and single times, keeps
//! schedule definitions and their run state, decides about missed, late and
//! overlapping runs, implements the `schedule.*` commands and provides the
//! schedule module hosted by the background service.
//!
//! Definitions live in `<state>/schedules/definitions/<id>.json` and change
//! only through the operation journal; the run state in
//! `<state>/schedules/state.json` is written only by the schedule module.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod action;
pub mod engine;
mod errors;
pub mod module;
pub mod settings;
pub mod stages;
pub mod store;
pub mod trigger;
