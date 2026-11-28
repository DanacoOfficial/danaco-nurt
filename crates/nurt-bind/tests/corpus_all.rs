// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-bind/tests/corpus_all.rs
// Responsibility:   Every correction corpus of `tests/d1/correction` run through the full pipeline, with the measures of each corpus directory.
// Dependencies:     corpus_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// A failed unwrap, expectation or panic is how a test reports a failure.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Every correction corpus of `tests/d1/correction` run through the full
//! pipeline, with the measures of each corpus directory. Run with
//! `cargo test -p nurt-bind --test corpus_all -- --ignored`.

mod corpus_support;

use std::fs;

use corpus_support::{read_cases, run, workspace};

/// Directory of the correction corpora, relative to the workspace.
const CORPORA: &str = "tests/d1/correction";

/// Largest share of `E_AMBIGUOUS` among unambiguous programs that change
/// the world.
const MAX_AMBIGUITY: f64 = 0.05;

/// Number of listed problems per corpus in a failure message.
const REPORT_LIMIT: usize = 1000;

#[test]
#[ignore = "runs every correction corpus; slow in a debug build"]
fn no_corpus_has_a_false_correction_or_a_violation() {
    let root = workspace().join(CORPORA);
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        // The corpora of the translators are checked by bash_corpus and
        // powershell_corpus of nurt-correct; binding gets the text after the
        // translator.
        .filter(|path| {
            path.file_name()
                .is_none_or(|name| name != "bash" && name != "powershell")
        })
        .collect();
    dirs.sort();
    let mut report = Vec::new();
    let mut failed = false;
    for dir in dirs {
        let cases = read_cases(&dir);
        if cases.is_empty() {
            continue;
        }
        let measures = run(&cases);
        failed |= !measures.false_corrections.is_empty()
            || !measures.violations.is_empty()
            || measures.ambiguity_share() > MAX_AMBIGUITY;
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        report.push(format!("{name}: {}", measures.report(REPORT_LIMIT)));
    }
    assert!(!failed, "{}", report.join("\n"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
