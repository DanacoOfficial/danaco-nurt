// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Byte comparison of the unified rendering with GNU `diff -u`.
//!
//! The test runs the `diff` program found in `PATH` and is ignored by
//! default: `cargo test -p nurt-text --test diff_differential -- --ignored`.
//! It passes without checking anything when `diff` is missing or is not GNU
//! diffutils.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use nurt_text::diff::{Unified, unified_diff};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

/// Lines texts are built from; see `diff_patch_properties.rs`.
const LINES: [&str; 9] = ["a", "b", "c", "", " a", "a\r", "x y", "}", "{"];

const SEED: [u8; 32] = *b"nurt-text differential gnu diff.";

const CASES: u32 = 5_000;

const MAX_CONTEXT: usize = 4;

fn gnu_diff_available() -> bool {
    Command::new("diff")
        .arg("--version")
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains("GNU diffutils"))
}

fn gnu_diff(
    dir: &Path,
    old: &str,
    new: &str,
    context: usize,
    ignore_whitespace: bool,
) -> io::Result<String> {
    let (old_path, new_path) = (dir.join("old"), dir.join("new"));
    fs::write(&old_path, old)?;
    fs::write(&new_path, new)?;
    let mut command = Command::new("diff");
    command
        .env("LC_ALL", "C")
        .arg(format!("-U{context}"))
        .args(["--label", "a", "--label", "b"]);
    if ignore_whitespace {
        command.arg("-w");
    }
    let output = command.arg(&old_path).arg(&new_path).output()?;
    String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn text() -> impl Strategy<Value = String> {
    (prop::collection::vec(0..LINES.len(), 0..24), any::<bool>()).prop_map(
        |(indexes, final_newline)| {
            let mut text = indexes
                .iter()
                .map(|&index| LINES[index])
                .collect::<Vec<_>>()
                .join("\n");
            if final_newline && !indexes.is_empty() {
                text.push('\n');
            }
            text
        },
    )
}

#[test]
#[ignore = "runs the external diff program"]
fn rendering_equals_gnu_diff_output() {
    if !gnu_diff_available() {
        eprintln!("GNU diff not found in PATH; comparison skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    };
    let mut runner =
        TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &SEED));
    let strategy = (text(), text(), 0..=MAX_CONTEXT, any::<bool>());
    runner
        .run(&strategy, |(old, new, context, ignore_whitespace)| {
            let ours = Unified::new(
                "a",
                "b",
                &unified_diff(&old, &new, context, ignore_whitespace),
            )
            .to_string();
            let theirs = gnu_diff(dir.path(), &old, &new, context, ignore_whitespace)
                .map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(ours, theirs);
            Ok(())
        })
        .unwrap();
}
