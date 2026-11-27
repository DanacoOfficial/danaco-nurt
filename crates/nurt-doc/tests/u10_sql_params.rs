// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Named parameters, the authorizer and the row count of changes of `sql`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(dead_code, reason = "each test file uses a subset of the fixtures")]
mod u10_support;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Fields, Value};
use u10_support::{
    RecordingTx, args, args_map, cells, count, ctx, database, failure, literal_text, read, write,
};

/// One table with two rows.
const ITEMS: &str = "create table items(id integer primary key, name text);
                     insert into items values (1, 'ada'), (2, 'bob');";

fn map(entries: &[(&str, Value)]) -> Fields {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn binds_map_entries_with_and_without_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let params = map(&[
        (":id", Value::Int(2)),
        ("name", Value::Text("bob".to_owned())),
    ]);
    let call = args_map(
        &db,
        "select id from items where id = :id and name = @name",
        params,
        false,
    );
    let rows = read(&call, &ctx()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(cells(&rows[0])[0].1, Value::Int(2));
}

#[test]
fn reuses_a_name_across_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let call = args_map(
        &db,
        "select :v as a; select :v as b",
        map(&[("v", Value::Int(5))]),
        false,
    );
    let rows = read(&call, &ctx()).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn refuses_positional_placeholder_with_a_map() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let call = args_map(&db, "select ?", map(&[("a", Value::Int(1))]), false);
    let error = failure(read(&call, &ctx()));
    assert_eq!(error.code(), ErrorCode::Sql);
    assert_eq!(error.problem.as_deref(), Some("parameter_name"));
}

#[test]
fn refuses_missing_and_unused_names() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let missing = args_map(&db, "select :a", map(&[("b", Value::Int(1))]), false);
    assert_eq!(
        failure(read(&missing, &ctx())).problem.as_deref(),
        Some("parameter_name")
    );
    let unused = args_map(
        &db,
        "select :a",
        map(&[("a", Value::Int(1)), ("z", Value::Int(2))]),
        false,
    );
    let error = failure(read(&unused, &ctx()));
    assert_eq!(error.problem.as_deref(), Some("parameter_name"));
    assert!(error.message.contains('z'), "{}", error.message);
}

#[test]
fn writes_with_a_map_and_reports_rows_changed() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let call = args_map(
        &db,
        "update items set name = :name where id = :id",
        map(&[
            ("id", Value::Int(1)),
            ("name", Value::Text("eve".to_owned())),
        ]),
        true,
    );
    let written = write(&call, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(written.changes[0].rows_changed, Some(1));
    assert_eq!(
        count(&db, "select count(*) from items where name = 'eve'"),
        1
    );
}

#[test]
fn fix_for_a_read_that_changes_repeats_the_map() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let call = args_map(
        &db,
        "delete from items where id = :id",
        map(&[("id", Value::Int(7))]),
        false,
    );
    let error = failure(read(&call, &ctx()));
    assert_eq!(error.problem.as_deref(), Some("write_required"));
    assert_eq!(
        error.fix,
        [format!(
            "sql \"{}\" \"delete from items where id = :id\" params: {{\"id\": 7}} write: true",
            literal_text(&db)
        )]
    );
}

#[test]
fn refuses_transaction_control_in_a_change_but_allows_savepoints() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let refused = args(&db, "commit", Vec::new(), true);
    let error = failure(write(&refused, &ctx(), &mut RecordingTx::new()));
    assert_eq!(error.problem.as_deref(), Some("transaction"));
    let nested = args(
        &db,
        "savepoint s; delete from items; rollback to s; release s; delete from items where id = 1",
        Vec::new(),
        true,
    );
    write(&nested, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(count(&db, "select count(*) from items"), 1);
}

#[test]
fn refuses_extension_calls_in_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", ITEMS);
    let call = args(&db, "select load_extension('x')", Vec::new(), true);
    let error = failure(write(&call, &ctx(), &mut RecordingTx::new()));
    assert_eq!(error.problem.as_deref(), Some("extension"));
}
