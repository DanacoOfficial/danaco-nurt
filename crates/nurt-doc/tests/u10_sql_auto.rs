// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `sql` without refusals: a statement that changes anything opens the
//! database for writing and journals its files first, transaction
//! statements merge with the transaction of the call, `ROLLBACK` ends the
//! call with `E_ROLLED_BACK`, `ATTACH`, `DETACH`, `VACUUM`, `VACUUM INTO`,
//! `load_extension` and the directory pragmas run, and `undo` returns every
//! file to its previous bytes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(dead_code, reason = "each test file uses a subset of the fixtures")]
mod u10_support;

use std::path::Path;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::journal::{ExternalChangeKind, TargetPath};
use nurt_contracts::value::Value;
use u10_support::{
    RecordingTx, args, args_auto, contents, count, ctx, database, database_files, failure, read,
    text, write,
};

const USERS: &str = "create table users(id integer primary key, name text not null);
                     insert into users(name) values ('ada');";

fn declared_paths(tx: &RecordingTx) -> Vec<String> {
    tx.declared
        .iter()
        .map(|spec| match &spec.path {
            TargetPath::Fs(path) => path.as_path().to_string_lossy().into_owned(),
            other => panic!("unexpected target {other:?}"),
        })
        .collect()
}

fn declares(tx: &RecordingTx, file: &Path) -> bool {
    let wanted = std::fs::canonicalize(file)
        .or_else(|_| {
            std::fs::canonicalize(file.parent().unwrap())
                .map(|dir| dir.join(file.file_name().unwrap()))
        })
        .unwrap();
    declared_paths(tx)
        .iter()
        .any(|declared| Path::new(declared) == wanted)
}

#[test]
fn a_changing_statement_without_write_changes_and_journals_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = contents(&database_files(&db));
    let mut tx = RecordingTx::new();
    let call = args_auto(&db, "insert into users(name) values ('eve')", Vec::new());
    let written = write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 2);
    assert_eq!(tx.declared.len(), 4);
    assert_eq!(written.changes[0].rows_changed, Some(1));
    tx.undo();
    assert_eq!(contents(&database_files(&db)), before);
}

#[test]
fn a_reading_statement_without_write_declares_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args_auto(
        &db,
        "select name from users; pragma user_version",
        Vec::new(),
    );
    assert_eq!(read(&call, &ctx()).unwrap().len(), 2);
    let mut tx = RecordingTx::new();
    let written = write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(written.rows.len(), 2);
    assert!(tx.declared.is_empty());
}

#[test]
fn the_read_step_leaves_a_changing_statement_to_the_effect_step() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args_auto(&db, "delete from users", Vec::new());
    let error = failure(read(&call, &ctx()));
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn transaction_statements_merge_with_the_transaction_of_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let query = "begin immediate; insert into users(name) values ('eve'); commit; \
                 begin; insert into users(name) values ('bob'); end";
    write(&args_auto(&db, query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 3);
}

#[test]
fn explicit_write_true_accepts_transaction_statements_too() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(&db, "begin; delete from users; commit", Vec::new(), true);
    write(&call, &ctx(), &mut RecordingTx::new()).unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 0);
}

#[test]
fn rollback_ends_the_call_with_rolled_back_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = contents(&database_files(&db));
    let mut tx = RecordingTx::new();
    let query =
        "insert into users(name) values ('eve'); rollback; insert into users(name) values ('bob')";
    let error = failure(write(&args_auto(&db, query, Vec::new()), &ctx(), &mut tx));
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert_eq!(count(&db, "select count(*) from users"), 1);
    assert!(tx.observed.is_empty());
    tx.undo();
    assert_eq!(contents(&database_files(&db)), before);
}

#[test]
fn rollback_to_a_savepoint_is_an_ordinary_statement() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = "savepoint s; delete from users; rollback to s; release s";
    write(
        &args_auto(&db, query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn attach_journals_the_attached_file_and_undo_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let before_main = contents(&database_files(&db));
    let before_other = contents(&database_files(&other));
    let mut tx = RecordingTx::new();
    let query = format!(
        "attach '{}' as o; insert into o.users(name) values ('zed'); \
         insert into users(name) values ('eve'); detach o",
        text(&other)
    );
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&other, "select count(*) from users"), 2);
    assert_eq!(count(&db, "select count(*) from users"), 2);
    assert!(declares(&tx, &other));
    assert_eq!(tx.declared.len(), 8);
    assert!(
        tx.observed
            .iter()
            .any(|change| change.path.as_path().ends_with("other.db")
                && change.kind == ExternalChangeKind::Modified)
    );
    tx.undo();
    assert_eq!(contents(&database_files(&db)), before_main);
    assert_eq!(contents(&database_files(&other)), before_other);
}

#[test]
fn attach_takes_the_file_from_a_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let mut tx = RecordingTx::new();
    let call = args_auto(
        &db,
        "attach ? as o; delete from o.users",
        vec![Value::Text(text(&other))],
    );
    write(&call, &ctx(), &mut tx).unwrap();
    assert_eq!(count(&other, "select count(*) from users"), 0);
    assert!(declares(&tx, &other));
}

#[test]
fn attach_of_a_new_file_is_journaled_as_created_and_undo_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let created = dir.path().join("created.db");
    let mut tx = RecordingTx::new();
    let query = format!(
        "attach '{}' as n; create table n.t(x); insert into n.t values (1)",
        text(&created)
    );
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&created, "select count(*) from t"), 1);
    assert!(
        tx.observed
            .iter()
            .any(|change| change.path.as_path().ends_with("created.db")
                && change.kind == ExternalChangeKind::Created)
    );
    tx.undo();
    assert!(!created.exists());
}

#[test]
fn detach_runs_after_the_other_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let query = format!(
        "attach '{}' as o; detach o; insert into users(name) values ('eve')",
        text(&other)
    );
    write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 2);
}

#[test]
fn vacuum_into_journals_the_new_file_as_created() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("copy.db");
    let mut tx = RecordingTx::new();
    let query = format!("vacuum into '{}'", text(&copy));
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&copy, "select count(*) from users"), 1);
    assert!(declares(&tx, &copy));
    assert!(
        tx.observed
            .iter()
            .any(|change| change.path.as_path().ends_with("copy.db")
                && change.kind == ExternalChangeKind::Created)
    );
    tx.undo();
    assert!(!copy.exists());
}

#[test]
fn vacuum_into_takes_the_file_from_a_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("copy.db");
    let mut tx = RecordingTx::new();
    let call = args_auto(&db, "vacuum into ?", vec![Value::Text(text(&copy))]);
    write(&call, &ctx(), &mut tx).unwrap();
    assert!(copy.exists());
    tx.undo();
    assert!(!copy.exists());
}

#[test]
fn vacuum_into_an_unknown_file_is_marked_irreversible() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("computed.db");
    let query = format!("vacuum into '{}' || ''", text(&copy));
    let written = write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert!(copy.exists());
    assert_eq!(written.irreversible.len(), 1);
}

#[test]
fn vacuum_runs_after_the_statements_outside_the_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(
        dir.path(),
        "app.db",
        "create table big(x); with recursive c(i) as (select 1 union all select i + 1 from c where i < 5000) insert into big select zeroblob(100) from c;",
    );
    let before = std::fs::metadata(&db).unwrap().len();
    let mut tx = RecordingTx::new();
    write(
        &args_auto(&db, "vacuum; delete from big", Vec::new()),
        &ctx(),
        &mut tx,
    )
    .unwrap();
    assert_eq!(count(&db, "select count(*) from big"), 0);
    assert!(std::fs::metadata(&db).unwrap().len() <= before);
    assert_eq!(tx.declared.len(), 4);
}

#[test]
fn directory_pragmas_are_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = format!("pragma temp_store_directory = '{}'", text(dir.path()));
    write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    write(
        &args_auto(&db, "pragma data_store_directory = ''", Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
}

#[test]
fn a_pragma_runs_between_the_statements_and_commits_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = "insert into users(name) values ('eve'); pragma journal_mode = wal; insert into users(name) values ('bob')";
    write(
        &args_auto(&db, query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 3);
}

#[test]
fn load_extension_is_attempted_instead_of_refused() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let missing = dir.path().join("missing-extension");
    let query = format!("select load_extension('{}')", text(&missing));
    let error = failure(write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    assert_eq!(error.code(), ErrorCode::Sql);
    assert_ne!(error.problem.as_deref(), Some("extension"));
    assert!(
        !error.message.contains("not authorized"),
        "{}",
        error.message
    );
}
