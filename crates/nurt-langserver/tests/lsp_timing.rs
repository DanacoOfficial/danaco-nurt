// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-langserver/tests/lsp_timing.rs
// Responsibility:   Time from a change of a document to its diagnostics.
// Dependencies:     lsp_support, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Time from a change of a document to its diagnostics.
//!
//! The diagnostics of a program of one hundred lines, read with correction
//! and bound to the catalog of the workspace, are published within the
//! budget in an optimized build. A debug build runs the same session
//! without the time check.

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod lsp_support;

use std::time::{Duration, Instant};

use lsp_support::{Client, did_open, full_options};
use serde_json::json;

/// URI of the measured document.
const URI: &str = "file:///work/long.nurt";

/// Number of lines of the measured document.
const LINES: usize = 100;

/// Longest time from a change to its diagnostics.
const BUDGET: Duration = Duration::from_millis(50);

/// Number of measured changes; the shortest time is compared with the
/// budget, so a single delay of the scheduler does not fail the test.
const ROUNDS: i64 = 5;

fn program() -> String {
    (0..LINES)
        .map(|index| {
            format!("find \"/srv/app{index}\" glob: \"*.go\" |> filter size > 1mb |> first 20\n")
        })
        .collect()
}

#[test]
fn publishes_diagnostics_of_a_hundred_lines_within_the_budget() {
    let (mut client, _) = Client::start(full_options(), json!({"capabilities": {}}));
    let text = program();
    client.notify("textDocument/didOpen", did_open(URI, &text, 1));
    let first = client.notification("textDocument/publishDiagnostics");
    assert_eq!(first["diagnostics"], json!([]), "{first}");
    let mut shortest = Duration::MAX;
    for version in 2..2 + ROUNDS {
        let started = Instant::now();
        client.notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": URI, "version": version}, "contentChanges": [{"text": text}]}),
        );
        let published = client.notification("textDocument/publishDiagnostics");
        shortest = shortest.min(started.elapsed());
        assert_eq!(published["version"], version);
    }
    if !cfg!(debug_assertions) {
        assert!(
            shortest <= BUDGET,
            "diagnostics of {LINES} lines took {shortest:?}"
        );
    }
    client.shutdown().unwrap();
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
