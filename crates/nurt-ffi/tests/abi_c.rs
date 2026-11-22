// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface tests in C: `tests/abi/c/abi_test.c` (feature `stub`) or
//! `tests/abi/c/engine_test.c` (engine of `nurt-core`) is compiled with
//! `clang` against `nurt.h`, linked with the shared library and run. The
//! tests are skipped when `clang` is not in `PATH`.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

mod common;

use std::process::Command;

#[cfg(feature = "stub")]
#[test]
fn c_program_passes_the_status_matrix_and_interface_checks() {
    let Some(clang) = common::find_program("clang") else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let program = work.path().join("abi_test");
    let source = common::manifest_dir().join("tests/abi/c/abi_test.c");
    common::compile_c(&clang, &[source], &program, &[]);
    let output = common::run(
        Command::new(&program)
            .env_remove("NURT_STUB_DIR")
            .env_remove("NURT_STUB_PANIC"),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}

#[cfg(not(feature = "stub"))]
#[test]
fn c_program_passes_the_interface_checks_on_the_engine() {
    let Some(clang) = common::find_program("clang") else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let program = work.path().join("engine_test");
    let source = common::manifest_dir().join("tests/abi/c/engine_test.c");
    common::compile_c(&clang, &[source], &program, &[]);
    let output = common::run_isolated(&mut Command::new(&program), work.path());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}

#[cfg(all(feature = "e2e", not(feature = "stub")))]
#[test]
fn c_program_runs_find_filter_count_and_remove_pipelines() {
    let Some(clang) = common::find_program("clang") else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let program = work.path().join("pipeline_test");
    let source = common::manifest_dir().join("tests/abi/c/pipeline_test.c");
    common::compile_c(&clang, &[source], &program, &[]);
    let output = common::run_isolated_with(
        &mut Command::new(&program),
        work.path(),
        "NURT_E2E_ENGINE",
        "1",
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}
