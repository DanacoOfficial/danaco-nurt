// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           tests/d2/crash/runner.rs
// Responsibility:   Process kill runner on the real file system of a temporary directory.
// Dependencies:     nurt_crashfs
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Process kill runner on the real file system of a temporary directory.
//!
//! The child is this test binary running `runner::kill9_child_scenario`. It
//! replaces a state file atomically in a loop (temporary file, file flush,
//! rename, directory flush), or, in recovery mode, removes temporary files.
//! The parent kills it at a seeded moment and checks that the state file
//! holds one complete version and that recovery leaves no temporary file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_crashfs::runner::{ProcessRunner, kill_delay, signal_ready};

const DIR_VAR: &str = "NURT_CRASH_CHILD_DIR";
const MODE_VAR: &str = "NURT_CRASH_CHILD_MODE";
const STATE: &str = "state";
const TEMP_SUFFIX: &str = ".tmp";
/// Size of one version of the state file.
const VERSION_SIZE: usize = 64 * 1024;
/// Versions written by one child run when it is not killed.
const VERSIONS: u32 = 400;
/// Temporary files created before a recovery run.
const RECOVERY_FILES: u32 = 2_000;
/// Kill runs per test.
const RUNS: u64 = 24;
/// Upper bound of the kill delay.
const MAX_DELAY: Duration = Duration::from_millis(40);

fn version(number: u32) -> Vec<u8> {
    let mut content = format!("{number:010}\n").into_bytes();
    let fill = u8::try_from(number % 251).unwrap();
    content.resize(VERSION_SIZE, fill);
    content
}

fn check_version(content: &[u8]) -> u32 {
    assert_eq!(content.len(), VERSION_SIZE, "torn state file");
    let number: u32 = std::str::from_utf8(&content[..10])
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(content, version(number).as_slice(), "mixed state file");
    number
}

fn sync_dir(dir: &Path) {
    if cfg!(unix) {
        fs::File::open(dir).unwrap().sync_all().unwrap();
    }
}

fn replace_state(dir: &Path, number: u32) {
    let temp = dir.join(format!("{number}{TEMP_SUFFIX}"));
    let mut file = fs::File::create(&temp).unwrap();
    file.write_all(&version(number)).unwrap();
    file.sync_all().unwrap();
    fs::rename(&temp, dir.join(STATE)).unwrap();
    sync_dir(dir);
}

fn recover(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with(TEMP_SUFFIX) {
            fs::remove_file(path).unwrap();
        }
    }
    sync_dir(dir);
}

fn runner(dir: &Path, mode: &str) -> ProcessRunner {
    let program = std::env::current_exe().unwrap();
    ProcessRunner::new(
        program,
        [
            "runner::kill9_child_scenario",
            "--exact",
            "--ignored",
            "--test-threads=1",
        ],
    )
    .env(DIR_VAR, dir)
    .env(MODE_VAR, mode)
}

#[test]
#[ignore = "child process of the kill runner"]
fn kill9_child_scenario() {
    let Some(dir) = std::env::var_os(DIR_VAR).map(PathBuf::from) else {
        return;
    };
    let mode = std::env::var(MODE_VAR).unwrap_or_default();
    signal_ready().unwrap();
    if mode == "recover" {
        recover(&dir);
        return;
    }
    let start = check_version(&fs::read(dir.join(STATE)).unwrap()) + 1;
    for number in start..start + VERSIONS {
        replace_state(&dir, number);
    }
}

#[test]
fn kill9_runner_interrupts_replacement() {
    let mut killed = 0;
    for seed in 0..RUNS {
        let dir = tempfile::tempdir().unwrap();
        replace_state(dir.path(), 0);
        let outcome = runner(dir.path(), "replace")
            .run_and_kill(kill_delay(seed, MAX_DELAY))
            .unwrap();
        killed += u32::from(outcome.killed);
        check_version(&fs::read(dir.path().join(STATE)).unwrap());
        recover(dir.path());
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(
            names.len(),
            1,
            "seed {seed}: temporary files survived recovery"
        );
    }
    assert!(killed > 0, "no child was killed before it finished");
}

#[test]
fn kill9_runner_interrupts_recovery() {
    for seed in 0..RUNS / 4 {
        let dir = tempfile::tempdir().unwrap();
        replace_state(dir.path(), 7);
        for index in 0..RECOVERY_FILES {
            fs::write(
                dir.path().join(format!("x{index}{TEMP_SUFFIX}")),
                b"partial",
            )
            .unwrap();
        }
        runner(dir.path(), "recover")
            .run_and_kill(kill_delay(seed, MAX_DELAY / 4))
            .unwrap();
        assert_eq!(check_version(&fs::read(dir.path().join(STATE)).unwrap()), 7);
        assert!(
            runner(dir.path(), "recover")
                .run_to_end()
                .unwrap()
                .success()
        );
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "seed {seed}: recovery is not complete");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
