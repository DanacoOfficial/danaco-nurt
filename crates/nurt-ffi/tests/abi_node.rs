// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Interface test in Node: `tests/abi/node/addon.c` is compiled with `clang`
//! into a Node-API module against the headers of the installed Node and
//! loaded by `tests/abi/node/abi_test.js`. The test is skipped when `node`,
//! its headers or `clang` are missing.

#![cfg(all(feature = "stub", unix))]
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

#[test]
fn node_module_passes_the_status_matrix_and_interface_checks() {
    let (Some(node), Some(clang)) = (common::find_program("node"), common::find_program("clang"))
    else {
        return;
    };
    let Some(headers) = node_headers(&node) else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let module = work.path().join("nurt_abi.node");
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
    let output = common::run(
        Command::new(node)
            .arg(dir.join("abi_test.js"))
            .arg(&module)
            .env_remove("NURT_STUB_DIR")
            .env_remove("NURT_STUB_PANIC"),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}
