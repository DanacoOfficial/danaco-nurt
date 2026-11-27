// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-verify/tests/analyze_external.rs
// Responsibility:   The analysis commands with the programs installed on the system. The tests.
// Dependencies:     analyze_support, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The analysis commands with the programs installed on the system. The tests
//! run only with the `external-tools` feature and return early when a program is not installed.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a failed precondition of a test is reported by a panic"
)]

mod analyze_support;

use nurt_contracts::value::Value;

use analyze_support::{Workspace, apply, args, field, read, texts};

fn installed(program: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|entry| {
        entry
            .join(format!("{program}{}", std::env::consts::EXE_SUFFIX))
            .is_file()
    })
}

fn go_project() -> Workspace {
    let workspace = Workspace::new();
    workspace
        .file("go.mod", "module example.test/app\n\ngo 1.21\n")
        .file("main.go", "package main\n\nfunc main() {\nx:=1\n_ = x\n}\n");
    workspace
}

#[test]
#[cfg_attr(
    not(feature = "external-tools"),
    ignore = "needs the external-tools feature"
)]
fn gofmt_reports_and_fixes_an_unformatted_file() {
    if !installed("gofmt") {
        return;
    }
    let workspace = go_project();
    let ctx = workspace.system_context();
    let values = [("checks", texts(&["format"]))];
    let (output, _) = apply(&ctx, "analyze.check", &args("analyze.check", &values)).unwrap();
    assert_eq!(output.records.len(), 1);
    assert_eq!(field(&output.records[0], "path"), Some("main.go"));

    apply(&ctx, "analyze.format", &args("analyze.format", &[])).unwrap();
    assert!(workspace.read("main.go").contains("x := 1"));
}

#[test]
#[cfg_attr(
    not(feature = "external-tools"),
    ignore = "needs the external-tools feature"
)]
fn tools_lists_gofmt_for_a_go_project() {
    if !installed("gofmt") {
        return;
    }
    let workspace = go_project();
    let output = read(
        &workspace.system_context(),
        "analyze.tools",
        &args("analyze.tools", &[]),
    )
    .unwrap();
    let gofmt = output
        .records
        .iter()
        .find(|record| field(record, "tool") == Some("gofmt"))
        .unwrap();
    assert_eq!(gofmt.get("found").and_then(Value::as_bool), Some(true));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
