// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-files/tests/perf.rs
// Responsibility:   Wall-clock limits of the commands of this crate. The binary is named `perf`.
// Dependencies:     notebook_support, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Wall-clock limits of the commands of this crate. The binary is named `perf`
//! so that the filter `not binary(perf)` of `cargo nextest` skips it on loaded
//! machines.

// Tests report failures by panicking.
#![allow(clippy::expect_used)]

mod notebook_support;

use std::time::{Duration, Instant};

use nurt_contracts::value::{Regex, Value};

use notebook_support::{args, counter, notebook, run, text, with_notebook};

/// Path of the notebook of the tests.
const PATH: &str = "/home/user/many.ipynb";

/// Number of matches of the expansion test.
const MATCHES: usize = 20_000;

/// Time within which all matches of one cell are replaced.
const EXPANSION_LIMIT: Duration = Duration::from_secs(5);

#[test]
fn many_matches_are_replaced_within_the_limit() {
    let source = "1x ".repeat(MATCHES);
    let cell = format!(
        concat!(
            "  {{\n   \"cell_type\": \"code\",\n   \"execution_count\": null,\n",
            "   \"id\": \"many\",\n   \"metadata\": {{}},\n   \"outputs\": [],\n",
            "   \"source\": [\n    \"{}\"\n   ]\n  }}"
        ),
        source
    );
    let test = with_notebook(PATH, &notebook(&[&cell]));
    let find = Value::Regex(Regex::new(r"(\d)x", "").expect("the pattern has valid flags"));
    let started = Instant::now();
    let effect = run(
        &test,
        &args(
            "notebook.replace",
            PATH,
            &[("id", text("many")), ("find", find), ("with", text("[$1]"))],
        ),
    )
    .expect("the replacement runs");
    let elapsed = started.elapsed();
    assert_eq!(counter(&effect, "replacements"), Some(MATCHES as u64));
    assert!(
        elapsed < EXPANSION_LIMIT,
        "replacing {MATCHES} matches took {elapsed:?}"
    );
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
