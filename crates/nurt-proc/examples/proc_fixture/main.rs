// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-proc/examples/proc_fixture/main.rs
// Responsibility:   Test program for the process tests of `nurt-proc`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Test program for the process tests of `nurt-proc`.
//!
//! The first argument selects a mode. The remaining arguments belong to the
//! mode. Each test area adds its modes in its own module.

use std::process::ExitCode;

mod control;
mod jobs;
mod output;
mod spawn;
mod track;

/// Exit code for a missing or unknown mode.
const UNKNOWN_MODE: u8 = 64;

/// Handler of the modes of one test area.
type AreaHandler = fn(&str, &[String]) -> Option<ExitCode>;

const AREAS: [AreaHandler; 5] = [spawn::run, control::run, output::run, jobs::run, track::run];

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(mode) = args.next() else {
        return ExitCode::from(UNKNOWN_MODE);
    };
    let rest: Vec<String> = args.collect();
    AREAS
        .iter()
        .find_map(|handler| handler(&mode, &rest))
        .unwrap_or(ExitCode::from(UNKNOWN_MODE))
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
