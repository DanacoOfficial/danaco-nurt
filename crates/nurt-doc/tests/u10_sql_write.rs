// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes of SQLite databases with `sql write: true` through the operation
//! journal: preservation of the database and its journal files, one
//! transaction, rollback after errors and undo to the previous bytes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(dead_code, reason = "each test file uses a subset of the fixtures")]
mod u10_support;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::{ExternalChangeKind, TargetIndex};
use nurt_contracts::report::{ChangeAction, Irreversible};
use nurt_contracts::stage::{CancelReason, CancelToken, StageImpl, StageInput};
use nurt_contracts::value::{Size, Value};
use nurt_doc::commands::sql::SqlStage;
use rusqlite::Connection;
use u10_support::{
    CtxOptions, RecordingTx, SHORT_BUSY, args, cells, contents, context, count, ctx, database,
    database_files, failure, text, write,
};

/// Table of the change tests, in rollback journal mode.
const USERS: &str = "create table users(id integer primary key, name text not null);
                     insert into users(name) values ('ada');";

/// The same table in write-ahead log mode.
const USERS_WAL: &str = "pragma journal_mode = wal;
                         create table users(id integer primary key, name text not null);
                         insert into users(name) values ('ada');";

fn journal_mode(path: &std::path::Path) -> String {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row("pragma journal_mode", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn inserts_rows_and_reports_changed_row_count() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let call = args(
        &db,
        "insert into users(name) values (?), (?)",
        vec![Value::Text("eve".to_owned()), Value::Text("bob".to_owned())],
        true,
    );
    let written = write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 3);
    assert_eq!(written.changes.len(), 1);
    let change = &written.changes[0];
    assert_eq!(change.path.as_str(), text(&db));
    assert_eq!(change.action, ChangeAction::Modify);
    assert_eq!(change.rows_changed, Some(2));
    assert!(written.irreversible.is_empty());
    assert_eq!(tx.declared.len(), 4);
    assert!(tx.observed.iter().any(|observed| {
        observed.declared == Some(TargetIndex(0)) && observed.kind == ExternalChangeKind::Modified
    }));
}

#[test]
fn returns_rows_of_returning_clause() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(
        &db,
        "insert into users(name) values ('eve') returning id, name",
        Vec::new(),
        true,
    );
    let written = write(&call, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(
        cells(&written.rows[0]),
        [
            ("id".to_owned(), Value::Int(2)),
            ("name".to_owned(), Value::Text("eve".to_owned())),
        ]
    );
}

#[test]
fn failed_statement_rolls_back_all_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = fs::read(&db).unwrap();
    let mut tx = RecordingTx::new();
    let call = args(
        &db,
        "insert into users(name) values ('eve'); insert into users(name) values (null)",
        Vec::new(),
        true,
    );
    let error = failure(write(&call, &ctx(), &mut tx));
    assert_eq!(error.code(), ErrorCode::Sql);
    assert_eq!(error.details.get("statement"), Some(&serde_json::json!(2)));
    assert_eq!(count(&db, "select count(*) from users"), 1);
    assert_eq!(fs::read(&db).unwrap(), before);
    assert!(tx.observed.is_empty());
}

#[test]
fn wal_database_stays_in_wal_mode_with_checkpointed_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS_WAL);
    assert_eq!(journal_mode(&db), "wal");
    let call = args(&db, "update users set name = 'eve'", Vec::new(), true);
    let written = write(&call, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(written.changes[0].rows_changed, Some(1));
    let files = database_files(&db);
    assert!(
        !files[1].exists(),
        "the write-ahead log remains after the change"
    );
    let header = fs::read(&db).unwrap();
    // Bytes 18 and 19 of the header are the read and write versions; 2 is WAL.
    assert_eq!(&header[18..20], &[2, 2]);
    assert_eq!(journal_mode(&db), "wal");
    assert_eq!(
        count(&db, "select count(*) from users where name = 'eve'"),
        1
    );
}

#[test]
fn undo_restores_database_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let files = database_files(&db);
    let before = contents(&files);
    let mut tx = RecordingTx::new();
    let call = args(
        &db,
        "update users set name = 'eve'; create table t(x)",
        Vec::new(),
        true,
    );
    write(&call, &ctx(), &mut tx).unwrap();
    assert_ne!(contents(&files), before);
    assert_eq!(
        tx.preserved, before,
        "the files are preserved before the change"
    );
    tx.undo();
    assert_eq!(contents(&files), before);
    assert_eq!(
        count(&db, "select count(*) from users where name = 'ada'"),
        1
    );
}

#[test]
fn undo_restores_wal_side_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS_WAL);
    let reader = Connection::open(&db).unwrap();
    let _: i64 = reader
        .query_row("select count(*) from users", [], |row| row.get(0))
        .unwrap();
    let files = database_files(&db);
    let before = contents(&files);
    assert!(
        before[1].is_some() && before[2].is_some(),
        "-wal and -shm exist"
    );
    let mut tx = RecordingTx::new();
    let call = args(
        &db,
        "insert into users(name) values ('eve')",
        Vec::new(),
        true,
    );
    write(&call, &ctx(), &mut tx).unwrap();
    assert_ne!(contents(&files)[0], before[0]);
    tx.undo();
    assert_eq!(contents(&files), before);
    drop(reader);
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn change_without_space_for_copy_is_marked_irreversible() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::refusing_copies();
    let call = args(&db, "delete from users", Vec::new(), true);
    let written = write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 0);
    assert_eq!(written.irreversible.len(), 1);
    assert_eq!(written.irreversible[0].reason, Irreversible::NoSpaceForCopy);
    assert_eq!(
        written.irreversible[0].path.as_deref(),
        Some(text(&db).as_str())
    );
}

#[test]
fn refuses_transaction_control_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(&db, "begin; delete from users; commit", Vec::new(), true);
    let error = failure(write(&call, &ctx(), &mut RecordingTx::new()));
    assert_eq!(error.problem.as_deref(), Some("transaction"));
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn allows_savepoints_inside_the_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(
        &db,
        "savepoint s; delete from users; rollback to s; release s; insert into users(name) values ('eve')",
        Vec::new(),
        true,
    );
    write(&call, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 2);
}

#[test]
fn waits_for_write_lock_then_reports_in_use() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let holder = Connection::open(&db).unwrap();
    holder.execute_batch("begin immediate").unwrap();
    let context = context(CtxOptions {
        busy: SHORT_BUSY,
        ..CtxOptions::default()
    });
    let started = Instant::now();
    let call = args(&db, "delete from users", Vec::new(), true);
    let error = failure(write(&call, &context, &mut RecordingTx::new()));
    assert_eq!(error.code(), ErrorCode::InUse);
    assert!(started.elapsed() >= SHORT_BUSY);
    holder.execute_batch("rollback").unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

/// Returns when another connection cannot start a write, which shows that the
/// change under test holds the write lock and runs; no wait on the clock
/// stands in for that.
fn wait_until_write_locked(db: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let probe = Connection::open(db).unwrap();
    probe.busy_timeout(Duration::ZERO).unwrap();
    while Instant::now() < deadline {
        if probe.execute_batch("begin immediate").is_err() {
            return;
        }
        probe.execute_batch("rollback").unwrap();
        thread::yield_now();
    }
    panic!("the change did not start within 30 s");
}

#[test]
fn cancellation_interrupts_long_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", "create table t(x);");
    let cancel = CancelToken::new();
    let context = context(CtxOptions {
        cancel: cancel.clone(),
        ..CtxOptions::default()
    });
    let locked = db.clone();
    let canceller = thread::spawn(move || {
        wait_until_write_locked(&locked);
        cancel.cancel(CancelReason::Requested);
    });
    let call = args(
        &db,
        "insert into t with recursive c(x) as (select 1 union all select x + 1 from c) select x from c",
        Vec::new(),
        true,
    );
    let error = failure(write(&call, &context, &mut RecordingTx::new()));
    canceller.join().unwrap();
    assert_eq!(error.code(), ErrorCode::Cancelled);
    assert_eq!(count(&db, "select count(*) from t"), 0);
}

#[test]
fn returning_more_rows_than_the_limit_fails_and_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let context = context(CtxOptions {
        max_records: 2,
        ..CtxOptions::default()
    });
    let call = args(
        &db,
        "insert into users(name) values ('a'), ('b'), ('c') returning id",
        Vec::new(),
        true,
    );
    let error = failure(write(&call, &context, &mut RecordingTx::new()));
    assert_eq!(error.code(), ErrorCode::Limit);
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn plan_names_the_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(&db, "delete from users", Vec::new(), true);
    let context = ctx();
    let plan = SqlStage
        .as_effect()
        .unwrap()
        .plan(&call, StageInput::none(), &context)
        .unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].path.as_str(), text(&db));
    assert_eq!(
        plan.changes[0].size_before,
        Some(Size::from_bytes(fs::metadata(&db).unwrap().len()))
    );
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn file_of_another_format_is_refused_before_preserving() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.txt");
    fs::write(&notes, "plain text\n").unwrap();
    let mut tx = RecordingTx::new();
    let call = args(&notes, "create table t(x)", Vec::new(), true);
    assert_eq!(
        failure(write(&call, &ctx(), &mut tx)).code(),
        ErrorCode::DocParse
    );
    assert!(tx.declared.is_empty());
    assert_eq!(fs::read(&notes).unwrap(), b"plain text\n");
}

#[test]
fn select_only_change_reports_no_file_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let call = args(&db, "select count(*) as n from users", Vec::new(), true);
    let written = write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(written.changes[0].rows_changed, Some(0));
    assert!(tx.observed.is_empty());
    assert_eq!(cells(&written.rows[0]), [("n".to_owned(), Value::Int(1))]);
}

#[test]
fn creates_database_in_empty_file_and_undo_empties_it() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("new.db");
    fs::write(&empty, b"").unwrap();
    let mut tx = RecordingTx::new();
    let call = args(
        &empty,
        "create table t(x); insert into t values (1)",
        Vec::new(),
        true,
    );
    write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(count(&empty, "select count(*) from t"), 1);
    tx.undo();
    assert_eq!(fs::read(&empty).unwrap(), b"");
    assert!(!database_files(&empty)[3].exists());
}
