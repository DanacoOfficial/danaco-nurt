// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conformance cases of the record stages in `conformance/cases/stages`.
//!
//! Every command has at least three case directories; each holds the pair
//! `case.in.json` and `case.out.json` read by the golden runner. The form of
//! a case is described in the harness.

// Tests fail through unwrap, expect and panic by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "fixtures/stages/harness.rs"]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};

use nurt_exec::stages::COMMANDS;
use nurt_testkit::golden::{GoldenInput, GoldenRunner};

/// Smallest number of cases of one command.
const MIN_CASES: usize = 3;

fn cases_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases/stages")
}

fn case_dirs(command: &str) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(cases_root().join(command))
        .unwrap_or_else(|error| panic!("cases of `{command}`: {error}"))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn every_command_has_conformance_cases() {
    for command in COMMANDS {
        let count = case_dirs(command).len();
        assert!(count >= MIN_CASES, "`{command}` has {count} cases");
    }
}

#[test]
fn conformance_cases_give_their_expected_results() {
    for command in COMMANDS {
        for dir in case_dirs(command) {
            let report = GoldenRunner::new(&dir)
                .bless(false)
                .run(|case| match &case.input {
                    GoldenInput::Json(input) => {
                        assert_eq!(input["command"], command, "{}", dir.display());
                        harness::run_case(input)
                    }
                    GoldenInput::Text(_) => panic!("{} has a text input", dir.display()),
                })
                .unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
            assert!(!report.passed.is_empty() || !report.blessed.is_empty());
        }
    }
}
