// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-cli/tests/run_file.rs
// Responsibility:   `nurt run <file>.nurt` runs a program file with parameters; any other `nurt run …` is the program of the command `run`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! `nurt run <file>.nurt` runs a program file with parameters; any other
//! `nurt run …` is the program of the command `run`.

// Test code asserts with unwrap, expect and panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use nurt_cli::args::{Invocation, resolve};

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn a_name_that_ends_with_nurt_runs_the_file() {
    for line in [
        &["run", "cleanup.nurt", "dir:", "/srv"][..],
        &["run", "cleanup.nurt", "--help"],
        &["run", "C:/proj/Cleanup.NURT", "dry"],
    ] {
        assert_eq!(
            resolve(&args(line)),
            Invocation::Subcommand {
                name: "run",
                args: args(&line[1..]),
            },
            "{line:?}"
        );
    }
}

#[test]
fn any_other_run_is_the_program_of_the_command() {
    for line in [
        &["run", "[\"go\",\"test\"]", "cwd: \"/x\""][..],
        &["run"],
        &["run", "-h"],
        &["run", "build.sh"],
    ] {
        assert_eq!(
            resolve(&args(line)),
            Invocation::Program { args: args(line) },
            "{line:?}"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
