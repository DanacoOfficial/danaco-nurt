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
use nurt_contracts::report::ChangeAction;
use nurt_contracts::value::Value;
use u10_support::{
    CtxOptions, RecordingTx, args, args_auto, contents, context, count, ctx, database,
    database_files, failure, read, text, write,
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
fn vacuum_into_a_computed_file_is_resolved_and_journaled() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("computed.db");
    let query = format!("vacuum into '{}' || ''", text(&copy));
    let mut tx = RecordingTx::new();
    let written = write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert!(copy.exists());
    assert!(written.irreversible.is_empty());
    assert!(declares(&tx, &copy));
    tx.undo();
    assert!(!copy.exists());
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
fn directory_pragmas_are_allowed_and_reported_as_effects_on_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = format!("pragma temp_store_directory = '{}'", text(dir.path()));
    let set = write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    let reset = write(
        &args_auto(&db, "pragma temp_store_directory = ''", Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(set.irreversible.len(), 1);
    assert!(
        set.irreversible[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("whole Nurt process"))
    );
    assert_eq!(reset.irreversible.len(), 1);
    let data = write(
        &args_auto(&db, "pragma data_store_directory = ''", Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(data.irreversible.len(), 1);
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

#[test]
fn attach_of_a_computed_file_is_resolved_and_journaled_before_the_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let before_other = contents(&database_files(&other));
    let mut tx = RecordingTx::new();
    let query = format!(
        "attach '{}/' || 'other.db' as o; delete from o.users",
        text(dir.path())
    );
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&other, "select count(*) from users"), 0);
    assert!(declares(&tx, &other));
    tx.undo();
    assert_eq!(contents(&database_files(&other)), before_other);
}

#[test]
fn attach_of_an_unresolvable_file_is_marked_irreversible_before_the_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let query = "attach (select name from users where 0) as o; select 1";
    let written = write(&args_auto(&db, query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(written.irreversible.len(), 1);
    assert_eq!(tx.declared.len(), 4);
}

fn wal_database(dir: &Path, name: &str) -> std::path::PathBuf {
    database(
        dir,
        name,
        "pragma journal_mode = wal; create table users(id integer primary key, name text not null); insert into users(name) values ('ada');",
    )
}

#[test]
fn a_file_uri_in_attach_is_journaled_and_undone() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let before = contents(&database_files(&other));
    let mut tx = RecordingTx::new();
    let query = format!(
        "attach 'file:{}?mode=rw' as o; delete from o.users",
        text(&other).replace(' ', "%20")
    );
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&other, "select count(*) from users"), 0);
    assert!(declares(&tx, &other));
    tx.undo();
    assert_eq!(contents(&database_files(&other)), before);
}

#[test]
fn a_file_uri_in_vacuum_into_is_journaled_as_created() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("copy.db");
    let mut tx = RecordingTx::new();
    let query = format!("vacuum into 'file:{}'", text(&copy));
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert!(copy.exists());
    assert!(declares(&tx, &copy));
    tx.undo();
    assert!(!copy.exists());
}

#[test]
fn a_memory_uri_needs_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let call = args_auto(
        &db,
        "attach 'file:m?mode=memory' as m; create table m.t(x)",
        Vec::new(),
    );
    let written = write(&call, &ctx(), &mut tx).unwrap();
    assert!(written.irreversible.is_empty());
    assert_eq!(tx.declared.len(), 4);
}

#[test]
fn a_relative_attach_resolves_against_the_base() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let mut tx = RecordingTx::new();
    let context = context(CtxOptions {
        base: Some(text(dir.path())),
        ..CtxOptions::default()
    });
    let call = args_auto(
        &db,
        "attach 'other.db' as o; delete from o.users",
        Vec::new(),
    );
    write(&call, &context, &mut tx).unwrap();
    assert_eq!(count(&other, "select count(*) from users"), 0);
    assert!(declares(&tx, &other));
}

#[test]
fn a_relative_attach_without_a_base_is_refused_before_any_effect() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = contents(&database_files(&db));
    let mut tx = RecordingTx::new();
    let call = args_auto(
        &db,
        "delete from users; attach 'stray-from-cwd.db' as o; create table o.t(x)",
        Vec::new(),
    );
    let error = failure(write(&call, &ctx(), &mut tx));
    assert_eq!(error.code(), ErrorCode::PathRelative);
    assert!(!Path::new("stray-from-cwd.db").exists());
    tx.undo();
    assert_eq!(contents(&database_files(&db)), before);
    let vacuum = args_auto(&db, "vacuum into 'stray-copy.db'", Vec::new());
    assert_eq!(
        failure(write(&vacuum, &ctx(), &mut RecordingTx::new())).code(),
        ErrorCode::PathRelative
    );
    assert!(!Path::new("stray-copy.db").exists());
}

#[test]
fn vacuum_into_runs_in_its_place_and_copies_the_earlier_state() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("copy.db");
    let query = format!("vacuum into '{}'; delete from users", text(&copy));
    write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(count(&copy, "select count(*) from users"), 1);
    assert_eq!(count(&db, "select count(*) from users"), 0);
}

#[test]
fn undo_restores_wal_databases_main_and_attached() {
    let dir = tempfile::tempdir().unwrap();
    let db = wal_database(dir.path(), "app.db");
    let other = wal_database(dir.path(), "other.db");
    let before_main = contents(&database_files(&db));
    let before_other = contents(&database_files(&other));
    let mut tx = RecordingTx::new();
    let query = format!(
        "insert into users(name) values ('eve'); attach '{}' as o; \
         insert into o.users(name) values ('zed')",
        text(&other)
    );
    write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(count(&other, "select count(*) from users"), 2);
    tx.undo();
    assert_eq!(contents(&database_files(&db)), before_main);
    assert_eq!(contents(&database_files(&other)), before_other);
}

#[test]
fn a_failing_statement_after_attach_and_pragma_leaves_the_files_restorable() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let before_main = contents(&database_files(&db));
    let before_other = contents(&database_files(&other));
    let mut tx = RecordingTx::new();
    let query = format!(
        "insert into users(name) values ('eve'); pragma journal_mode = wal; \
         attach '{}' as o; insert into o.users(name) values ('zed'); \
         insert into missing_table values (1)",
        text(&other)
    );
    let error = failure(write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx));
    assert_eq!(error.code(), ErrorCode::Sql);
    tx.undo();
    assert_eq!(contents(&database_files(&db)), before_main);
    assert_eq!(contents(&database_files(&other)), before_other);
}

#[test]
fn rollback_after_attach_restores_the_files_and_names_nothing_kept() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = database(dir.path(), "other.db", USERS);
    let before_other = contents(&database_files(&other));
    let mut tx = RecordingTx::new();
    let query = format!(
        "attach '{}' as o; insert into o.users(name) values ('zed'); rollback",
        text(&other)
    );
    let error = failure(write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx));
    assert_eq!(error.code(), ErrorCode::RolledBack);
    tx.undo();
    assert_eq!(contents(&database_files(&other)), before_other);
}

#[test]
fn a_rollback_names_the_effects_that_stay() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = "attach (select name from users where 0) as o; rollback";
    let error = failure(write(
        &args_auto(&db, query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert!(
        error.message.contains("cannot be undone"),
        "{}",
        error.message
    );
}

#[test]
fn preview_of_a_change_touches_nothing_and_names_the_database() {
    use nurt_contracts::stage::{StageImpl, StageInput};
    use nurt_doc::commands::sql::SqlStage;
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let copy = dir.path().join("copy.db");
    let before = contents(&database_files(&db));
    let query = format!(
        "attach '{}' as o; vacuum into '{}'; delete from users",
        text(&dir.path().join("new.db")),
        text(&copy)
    );
    let call = args_auto(&db, &query, Vec::new());
    let stage = SqlStage;
    let plan = stage
        .as_effect()
        .unwrap()
        .plan(&call, StageInput::none(), &ctx())
        .unwrap();
    let paths: Vec<_> = plan
        .changes
        .iter()
        .map(|c| c.path.as_str().to_owned())
        .collect();
    assert_eq!(
        paths,
        [text(&db), text(&dir.path().join("new.db")), text(&copy)]
    );
    assert_eq!(plan.changes[1].action, ChangeAction::Create);
    assert!(!copy.exists());
    assert!(!dir.path().join("new.db").exists());
    assert_eq!(contents(&database_files(&db)), before);
}

#[test]
fn identifiers_named_like_keywords_do_not_shift_the_kinds_of_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    // The lexer reads `begin` in the column list as the start of a trigger
    // body and `end` as its end; the kind of each statement still comes from
    // its own text, so the inserts run.
    let query = "create table a(begin, \"end\"); \
                 create trigger t after update of begin on a begin select 1; end; \
                 insert into a values (1, 2); insert into a values (3, 4)";
    write(
        &args_auto(&db, query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(count(&db, "select count(*) from a"), 2);
}

#[test]
fn a_quoted_load_extension_is_attempted_not_refused() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let missing = dir.path().join("none");
    let query = format!("select \"LOAD_extension\"('{}')", text(&missing));
    let error = failure(write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    assert!(
        !error.message.contains("not authorized"),
        "{}",
        error.message
    );
}

#[test]
fn a_rollback_after_a_process_pragma_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = format!(
        "pragma temp_store_directory = '{}'; rollback",
        text(dir.path())
    );
    let error = failure(write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    write(
        &args_auto(&db, "pragma temp_store_directory = ''", Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(error.code(), ErrorCode::RolledBack);
    assert!(
        error.message.contains("whole Nurt process"),
        "{}",
        error.message
    );
    assert!(!error.message.contains("changes nothing"));
}

#[test]
fn a_failing_call_after_an_extension_call_names_the_effect() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let query = format!(
        "select LOAD_Extension('{}')",
        text(&dir.path().join("none"))
    );
    let error = failure(write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    let kept = error
        .details
        .get("irreversible")
        .unwrap_or_else(|| panic!("no detail: {error:?}"))
        .to_string();
    assert!(kept.contains("load_extension"), "{kept}");
}

#[test]
fn an_error_and_a_rollback_keep_every_effect_that_stays() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let unresolved = "attach (select name from users where 0) as o";
    let plain = failure(write(
        &args_auto(
            &db,
            &format!("{unresolved}; insert into missing values (1)"),
            Vec::new(),
        ),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    assert_eq!(plain.code(), ErrorCode::Sql);
    let kept = plain.details.get("irreversible").unwrap().to_string();
    assert!(kept.contains("could not be resolved"), "{kept}");
    let query = format!(
        "{unresolved}; pragma temp_store_directory = '{}'; rollback",
        text(dir.path())
    );
    let rolled = failure(write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    ));
    write(
        &args_auto(&db, "pragma temp_store_directory = ''", Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(rolled.code(), ErrorCode::RolledBack);
    let kept = rolled.details.get("irreversible").unwrap().to_string();
    assert!(kept.contains("could not be resolved"), "{kept}");
    assert!(kept.contains("whole Nurt process"), "{kept}");
}

#[test]
fn a_database_attached_for_reading_is_not_reported_as_changed() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let reference = database(dir.path(), "ref.db", USERS);
    let mut tx = RecordingTx::new();
    let query = format!(
        "attach '{}' as r; insert into users(name) select name from r.users",
        text(&reference)
    );
    let written = write(&args_auto(&db, &query, Vec::new()), &ctx(), &mut tx).unwrap();
    assert_eq!(written.changes.len(), 1, "{:?}", written.changes);
    assert!(
        tx.observed
            .iter()
            .all(|change| !change.path.as_path().ends_with("ref.db"))
    );
    let query = format!(
        "attach '{}' as r; insert into r.users(name) values ('zed')",
        text(&reference)
    );
    let written = write(
        &args_auto(&db, &query, Vec::new()),
        &ctx(),
        &mut RecordingTx::new(),
    )
    .unwrap();
    assert_eq!(written.changes.len(), 2);
}

#[test]
fn preview_lists_extension_and_process_pragma_effects() {
    use nurt_contracts::report::Irreversible;
    use nurt_contracts::stage::{StageImpl, StageInput};
    use nurt_doc::commands::sql::SqlStage;
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = contents(&database_files(&db));
    let plan_of = |query: &str| {
        SqlStage
            .as_effect()
            .unwrap()
            .plan(
                &args_auto(&db, query, Vec::new()),
                StageInput::none(),
                &ctx(),
            )
            .unwrap()
    };
    let plan = plan_of(
        "select load_extension('/x.so'); pragma temp_store_directory = '/tmp'; \
         PRAGMA main.data_store_directory('/tmp'); pragma journal_mode = wal",
    );
    assert_eq!(plan.irreversible.len(), 3);
    assert!(
        plan.irreversible
            .iter()
            .all(|entry| entry.reason == Irreversible::ExternalEffect)
    );
    let details: Vec<_> = plan
        .irreversible
        .iter()
        .filter_map(|entry| entry.detail.as_deref())
        .collect();
    assert!(details[0].contains("load_extension"));
    assert!(details[1].contains("temp_store_directory"));
    assert!(details[2].contains("data_store_directory"));
    assert!(
        details
            .iter()
            .all(|detail| detail.contains("from the text"))
    );
    assert!(
        plan_of("insert into users(name) values ('x')")
            .irreversible
            .is_empty()
    );
    assert!(
        plan_of("pragma temp_store_directory")
            .irreversible
            .is_empty()
    );
    assert_eq!(contents(&database_files(&db)), before);
}
