// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading SQLite databases with `sql` and the checks that keep a read from
//! changing files.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(dead_code, reason = "each test file uses a subset of the fixtures")]
mod u10_support;

use std::fs;
use std::time::Instant;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::stage::{CancelReason, CancelToken, StageData, StageImpl, StageInput};
use nurt_contracts::value::{Bytes, Size, Value};
use nurt_doc::commands::sql::SqlStage;
use rusqlite::Connection;
use u10_support::{
    CtxOptions, RecordingTx, SHORT_BUSY, args, cells, code, context, count, ctx, database, failure,
    read, row, text, write,
};

/// One table with a column of every storage class.
const USERS: &str =
    "create table users(id integer primary key, name text, score real, avatar blob, note);
                     insert into users values (7, 'ada', 1.5, x'00ff', null);";

/// Recursive query without an end.
const ENDLESS: &str =
    "with recursive c(x) as (select 1 union all select x + 1 from c) select x from c";

#[test]
fn reads_cells_in_column_order_with_storage_classes() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let rows = read(&args(&db, "select * from users", Vec::new(), false), &ctx()).unwrap();
    assert_eq!(rows.len(), 1);
    let first = row(&rows[0]);
    assert_eq!(first.path.as_str(), text(&db));
    assert_eq!(first.no, 1);
    assert_eq!(first.table, None);
    assert_eq!(
        cells(&rows[0]),
        [
            ("id".to_owned(), Value::Int(7)),
            ("name".to_owned(), Value::Text("ada".to_owned())),
            ("score".to_owned(), Value::Number(1.5)),
            (
                "avatar".to_owned(),
                Value::Bytes(Bytes::new(vec![0x00, 0xff]))
            ),
            ("note".to_owned(), Value::Null),
        ]
    );
}

#[test]
fn binds_positional_numbered_and_named_parameters() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let numbered = read(
        &args(
            &db,
            "select ?2 as b, ?1 as a",
            vec![Value::Int(1), Value::Int(2)],
            false,
        ),
        &ctx(),
    )
    .unwrap();
    assert_eq!(
        cells(&numbered[0]),
        [
            ("b".to_owned(), Value::Int(2)),
            ("a".to_owned(), Value::Int(1))
        ]
    );
    let named = read(
        &args(
            &db,
            "select name from users where id = :id and :flag = 1",
            vec![Value::Int(7), Value::Bool(true)],
            false,
        ),
        &ctx(),
    )
    .unwrap();
    assert_eq!(
        cells(&named[0]),
        [("name".to_owned(), Value::Text("ada".to_owned()))]
    );
}

#[test]
fn converts_parameter_and_result_types() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let params = vec![
        Value::Bytes(Bytes::new(vec![1, 2])),
        Value::Size(Size::from_bytes(2048)),
        Value::Number(0.25),
        Value::Null,
    ];
    let rows = read(
        &args(
            &db,
            "select ? as b, ? as s, ? as n, ? as z, 1e999 as inf, 1 as d, 2 as d",
            params,
            false,
        ),
        &ctx(),
    )
    .unwrap();
    assert_eq!(
        cells(&rows[0]),
        [
            ("b".to_owned(), Value::Bytes(Bytes::new(vec![1, 2]))),
            ("s".to_owned(), Value::Int(2048)),
            ("n".to_owned(), Value::Number(0.25)),
            ("z".to_owned(), Value::Null),
            ("inf".to_owned(), Value::Text("Inf".to_owned())),
            ("d".to_owned(), Value::Int(1)),
            ("d_2".to_owned(), Value::Int(2)),
        ]
    );
}

#[test]
fn numbers_rows_per_statement_of_several_statements() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let rows = read(
        &args(
            &db,
            "select 1 as a; select 2 as b union all select 3;",
            Vec::new(),
            false,
        ),
        &ctx(),
    )
    .unwrap();
    let numbered: Vec<_> = rows
        .iter()
        .map(|record| {
            let typed = row(record);
            (typed.table, typed.no)
        })
        .collect();
    assert_eq!(
        numbered,
        [
            (Some("1".to_owned()), 1),
            (Some("2".to_owned()), 1),
            (Some("2".to_owned()), 2),
        ]
    );
}

#[test]
fn refuses_changing_statement_without_write_and_offers_fix() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = fs::read(&db).unwrap();
    let error = failure(read(
        &args(
            &db,
            "select 1; delete from users where id = ?",
            vec![Value::Int(7)],
            false,
        ),
        &ctx(),
    ));
    assert_eq!(error.code(), ErrorCode::Sql);
    assert_eq!(error.problem.as_deref(), Some("write_required"));
    assert_eq!(
        error.fix,
        [format!(
            "sql \"{}\" \"select 1; delete from users where id = ?\" params: [7] write: true",
            text(&db)
        )]
    );
    assert_eq!(fs::read(&db).unwrap(), before);
    assert_eq!(count(&db, "select count(*) from users"), 1);
}

#[test]
fn refuses_attach_detach_and_vacuum_into() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let other = dir.path().join("other.db");
    let cases = [
        (format!("attach '{}' as other", text(&other)), "attach"),
        ("detach other".to_owned(), "attach"),
        (format!("vacuum into '{}'", text(&other)), "vacuum_into"),
        ("pragma temp_store_directory = '/tmp'".to_owned(), "pragma"),
    ];
    for (query, problem) in cases {
        for write_allowed in [false, true] {
            let call = args(&db, &query, Vec::new(), write_allowed);
            let error = if write_allowed {
                failure(write(&call, &ctx(), &mut RecordingTx::new()))
            } else {
                failure(read(&call, &ctx()))
            };
            assert_eq!(error.code(), ErrorCode::Sql, "{query}");
            assert_eq!(error.problem.as_deref(), Some(problem), "{query}");
        }
    }
    assert!(!other.exists());
}

#[test]
fn refuses_loading_extensions() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let lib = dir.path().join("ext.so");
    let call = args(
        &db,
        &format!("select load_extension('{}')", text(&lib)),
        Vec::new(),
        false,
    );
    let error = failure(read(&call, &ctx()));
    assert_eq!(error.code(), ErrorCode::Sql);
    assert!(
        error.message.contains("not authorized"),
        "{}",
        error.message
    );
}

#[test]
fn reports_parameter_count_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let few = failure(read(
        &args(&db, "select ?, ?", vec![Value::Int(1)], false),
        &ctx(),
    ));
    assert_eq!(few.problem.as_deref(), Some("parameter_count"));
    let many = failure(read(
        &args(
            &db,
            "select ?; select 2",
            vec![Value::Int(1), Value::Int(2)],
            false,
        ),
        &ctx(),
    ));
    assert_eq!(many.problem.as_deref(), Some("parameter_count"));
    assert_eq!(many.expected, Some(serde_json::json!(1)));
    let list = failure(read(
        &args(&db, "select ?", vec![Value::List(Vec::new())], false),
        &ctx(),
    ));
    assert_eq!(list.problem.as_deref(), Some("parameter_type"));
}

#[test]
fn reports_syntax_errors_with_statement_number() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let error = failure(read(
        &args(&db, "select 1; selec 2", Vec::new(), false),
        &ctx(),
    ));
    assert_eq!(error.code(), ErrorCode::Sql);
    assert_eq!(error.details.get("statement"), Some(&serde_json::json!(2)));
    assert!(error.details.contains_key("sqlite_code"));
}

#[test]
fn reports_file_of_another_format_as_doc_parse() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.txt");
    fs::write(&notes, "plain text, not a database\n").unwrap();
    let error = failure(read(&args(&notes, "select 1", Vec::new(), false), &ctx()));
    assert_eq!(error.code(), ErrorCode::DocParse);
    assert_eq!(
        error.details.get("format"),
        Some(&serde_json::json!("sqlite"))
    );
    assert_eq!(error.problem.as_deref(), Some("not_sqlite"));
    let directory = failure(read(
        &args(dir.path(), "select 1", Vec::new(), false),
        &ctx(),
    ));
    assert_eq!(directory.code(), ErrorCode::DocParse);
}

#[test]
fn reports_damaged_database_with_sqlite_code() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut bytes = fs::read(&db).unwrap();
    // The schema page starts after the 100-byte file header.
    for byte in &mut bytes[100..] {
        *byte = 0xa5;
    }
    fs::write(&db, &bytes).unwrap();
    let error = failure(read(
        &args(&db, "select * from users", Vec::new(), false),
        &ctx(),
    ));
    assert_eq!(error.code(), ErrorCode::DocParse);
    assert!(error.details.contains_key("sqlite_code"));
    assert_eq!(
        error
            .location
            .as_ref()
            .map(|location| location.path.clone()),
        Some(text(&db))
    );
}

#[test]
fn reports_missing_database_as_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.db");
    assert_eq!(
        code(read(&args(&missing, "select 1", Vec::new(), false), &ctx())),
        ErrorCode::NotFound
    );
    assert!(!missing.exists());
}

#[test]
fn reads_empty_file_as_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.db");
    fs::write(&empty, b"").unwrap();
    let rows = read(
        &args(
            &empty,
            "select count(*) as tables from sqlite_master",
            Vec::new(),
            false,
        ),
        &ctx(),
    )
    .unwrap();
    assert_eq!(cells(&rows[0]), [("tables".to_owned(), Value::Int(0))]);
    assert_eq!(fs::metadata(&empty).unwrap().len(), 0);
}

#[test]
fn read_keeps_database_bytes_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let before = fs::read(&db).unwrap();
    read(
        &args(
            &db,
            "select * from users; pragma user_version",
            Vec::new(),
            false,
        ),
        &ctx(),
    )
    .unwrap();
    assert_eq!(fs::read(&db).unwrap(), before);
}

#[test]
fn stops_endless_query_when_stream_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(&db, ENDLESS, Vec::new(), false);
    let context = ctx();
    let output = SqlStage
        .as_read()
        .unwrap()
        .execute(&call, StageInput::none(), &context)
        .unwrap();
    let StageData::Records(stream) = output.data else {
        panic!("`sql` returned no records");
    };
    let first: Vec<_> = stream.take(5).map(Result::unwrap).collect();
    assert_eq!(cells(&first[4]), [("x".to_owned(), Value::Int(5))]);
}

#[test]
fn cancellation_ends_the_stream_with_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let cancel = CancelToken::new();
    let context = context(CtxOptions {
        cancel: cancel.clone(),
        ..CtxOptions::default()
    });
    let call = args(&db, ENDLESS, Vec::new(), false);
    let output = SqlStage
        .as_read()
        .unwrap()
        .execute(&call, StageInput::none(), &context)
        .unwrap();
    let StageData::Records(mut stream) = output.data else {
        panic!("`sql` returned no records");
    };
    assert!(stream.next().unwrap().is_ok());
    cancel.cancel(CancelReason::Requested);
    let error = stream.find_map(Result::err).unwrap();
    assert_eq!(error.code(), ErrorCode::Cancelled);
}

#[test]
fn waits_for_lock_then_reports_in_use() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let holder = Connection::open(&db).unwrap();
    holder
        .execute_batch("begin exclusive; update users set name = 'eve'")
        .unwrap();
    let started = Instant::now();
    let context = context(CtxOptions {
        busy: SHORT_BUSY,
        ..CtxOptions::default()
    });
    let error = failure(read(
        &args(&db, "select * from users", Vec::new(), false),
        &context,
    ));
    assert_eq!(error.code(), ErrorCode::InUse);
    assert!(started.elapsed() >= SHORT_BUSY);
    holder.execute_batch("rollback").unwrap();
}

#[test]
fn read_step_refuses_write_mode() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let error = failure(read(&args(&db, "select 1", Vec::new(), true), &ctx()));
    assert_eq!(error.code(), ErrorCode::StageFailed);
}

#[test]
fn effect_step_without_write_reads_and_declares_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let written = write(
        &args(&db, "select name from users", Vec::new(), false),
        &ctx(),
        &mut tx,
    )
    .unwrap();
    assert_eq!(written.rows.len(), 1);
    assert!(written.changes.is_empty());
    assert!(tx.declared.is_empty());
}

#[test]
fn rejects_arguments_of_other_types() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let call = args(&db, "select 1", Vec::new(), false).with(
        "params",
        nurt_contracts::stage::ArgValue::Value(Value::Int(1)),
        false,
    );
    assert_eq!(code(read(&call, &ctx())), ErrorCode::Type);
    let query = args(&db, "select 1", Vec::new(), false).with(
        "query",
        nurt_contracts::stage::ArgValue::Value(Value::Int(1)),
        false,
    );
    assert_eq!(code(read(&query, &ctx())), ErrorCode::Type);
    let path = args(&db, "select 1", Vec::new(), false).with(
        "path",
        nurt_contracts::stage::ArgValue::Value(Value::Bool(true)),
        false,
    );
    assert_eq!(code(read(&path, &ctx())), ErrorCode::Type);
}
