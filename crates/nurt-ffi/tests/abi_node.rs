// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/tests/abi_node.rs
// Responsibility:   Interface tests in Node: `tests/abi/node/addon.c` is compiled with `clang`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Interface tests in Node: `tests/abi/node/addon.c` is compiled with `clang`
//! into a Node-API module against the headers of the installed Node and
//! loaded by `tests/abi/node/abi_test.js` (feature `stub`) or
//! `tests/abi/node/engine_test.js` (engine of `nurt-core`). The tests are
//! skipped when `node`, its headers or `clang` are missing.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a test fails at the first unexpected result"
)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Directory holding `node_api.h` of the Node found in `PATH`.
fn node_headers(node: &Path) -> Option<PathBuf> {
    let output = Command::new(node)
        .args(["-p", "process.execPath"])
        .output()
        .ok()?;
    let exec_path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let prefix = exec_path.parent()?.parent()?;
    let headers = prefix.join("include").join("node");
    headers.join("node_api.h").is_file().then_some(headers)
}

/// Compiles the Node-API module into `work`; `None` when `node`, its
/// headers or `clang` are missing.
fn build_module(work: &Path) -> Option<(PathBuf, PathBuf)> {
    let node = common::find_program("node")?;
    let clang = common::find_program("clang")?;
    let headers = node_headers(&node)?;
    let module = work.join("nurt_abi.node");
    let dir = common::manifest_dir().join("tests/abi/node");
    let mut flags = vec![
        "-shared",
        "-fPIC",
        "-pthread",
        "-DNODE_GYP_MODULE_NAME=nurt_abi",
    ];
    if cfg!(target_os = "macos") {
        flags.extend(["-undefined", "dynamic_lookup"]);
    }
    let include = format!("-I{}", headers.display());
    flags.push(&include);
    common::compile_c(&clang, &[dir.join("addon.c")], &module, &flags);
    Some((node, module))
}

#[cfg(feature = "stub")]
#[test]
fn node_module_passes_the_status_matrix_and_interface_checks() {
    let work = tempfile::tempdir().unwrap();
    let Some((node, module)) = build_module(work.path()) else {
        return;
    };
    let script = common::manifest_dir().join("tests/abi/node/abi_test.js");
    let output = common::run(
        Command::new(node)
            .arg(script)
            .arg(&module)
            .env_remove("NURT_STUB_DIR")
            .env_remove("NURT_STUB_PANIC"),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}

#[cfg(not(feature = "stub"))]
#[test]
fn node_module_passes_the_interface_checks_on_the_engine() {
    let work = tempfile::tempdir().unwrap();
    let Some((node, module)) = build_module(work.path()) else {
        return;
    };
    let script = common::manifest_dir().join("tests/abi/node/engine_test.js");
    let output = common::run_isolated(Command::new(node).arg(script).arg(&module), work.path());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
