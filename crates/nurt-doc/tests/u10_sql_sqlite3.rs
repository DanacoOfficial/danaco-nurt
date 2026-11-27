// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u10_sql_sqlite3.rs
// Responsibility:   Differential tests of `sql` against the `sqlite3` program. The program is.
// Dependencies:     u10_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Differential tests of `sql` against the `sqlite3` program. The program is
//! used only by these tests; a test returns early when `sqlite3` is not in
//! the search path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(dead_code, reason = "each test file uses a subset of the fixtures")]
mod u10_support;

use std::path::Path;
use std::process::Command;

use u10_support::{RecordingTx, args, ctx, database, read, row, sqlite3, write};

/// Rows with integers, reals, text and null, compared by value.
const SAMPLE: &str = "create table items(id integer, label text, weight real, note);
                      insert into items values (1, 'bolt', 0.5, null), (2, 'nut', 12.25, 'zinc'),
                      (3, 'washer', -3.0, '');";

const QUERY: &str = "select id, label, weight, note from items order by id";

fn sqlite3_rows(program: &Path, db: &Path, query: &str) -> serde_json::Value {
    let output = Command::new(program)
        .arg("-json")
        .arg(db)
        .arg(query)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn sql_rows(db: &Path, query: &str) -> serde_json::Value {
    let rows = read(&args(db, query, Vec::new(), false), &ctx()).unwrap();
    serde_json::Value::Array(
        rows.iter()
            .map(|record| {
                serde_json::Value::Object(
                    row(record)
                        .cells
                        .into_iter()
                        .map(|(name, value)| (name, value.to_plain_json()))
                        .collect(),
                )
            })
            .collect(),
    )
}

#[test]
fn rows_match_sqlite3_json_output_skipped_without_sqlite3() {
    let Some(program) = sqlite3() else {
        eprintln!("skipped: the sqlite3 program is not in the search path");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "items.db", SAMPLE);
    assert_eq!(sql_rows(&db, QUERY), sqlite3_rows(&program, &db, QUERY));
}

#[test]
fn change_is_visible_to_sqlite3_skipped_without_sqlite3() {
    let Some(program) = sqlite3() else {
        eprintln!("skipped: the sqlite3 program is not in the search path");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "items.db", SAMPLE);
    let call = args(
        &db,
        "update items set weight = weight * 2 where id > 1",
        Vec::new(),
        true,
    );
    write(&call, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(sql_rows(&db, QUERY), sqlite3_rows(&program, &db, QUERY));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
