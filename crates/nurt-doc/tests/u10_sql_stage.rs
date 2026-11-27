// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-doc/tests/u10_sql_stage.rs
// Responsibility:   The `sql` stage as a catalog command: the descriptor, the static check of the arguments and the effect step called without a planned state.
// Dependencies:     nurt_contracts, nurt_testkit, u10_support
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The `sql` stage as a catalog command: the descriptor, the static check of
//! the arguments and the effect step called without a planned state.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]

#[allow(dead_code, reason = "each test file uses a subset of the fixtures")]
mod u10_support;

use std::path::Path;

use nurt_contracts::config::Config;
use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::spec::StageSpec;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, CheckCtx, EffectPlan, InputKinds, StageData, StageImpl, StaticInfo,
};
use nurt_contracts::value::{Record, Value};
use nurt_doc::commands::sql::SqlStage;
use nurt_testkit::ctx::TestContext;
use u10_support::{RecordingTx, args, cells, code, count, ctx, database, failure, text};

const USERS: &str = "create table users(id integer primary key, name text not null);
insert into users(name) values ('ada');";

/// Catalog entry of `sql` with the fields the specification requires.
const SPEC: &str = "name = \"sql\"\ncategory = \"change\"\nreversibility = \"read\"\neffect = \"read\"\nimplementation = \"builtin\"\ninput = [\"none\"]\noutput = [\"row\"]\nsummary = \"Run SQL statements on an SQLite database file.\"\n";

fn check(args: &BoundArgs) -> Result<StaticInfo> {
    let spec: StageSpec = toml::from_str(SPEC).unwrap();
    let test = TestContext::new();
    let config = Config::default();
    SqlStage.check(args, &CheckCtx::new(&spec, &**test.host(), &config))
}

/// Runs the effect step with an empty plan, as an executor does when the
/// plan of the stage was not kept.
fn apply_without_plan(args: &BoundArgs, tx: &mut RecordingTx) -> Result<Vec<Record>> {
    let stage = SqlStage;
    let context = ctx();
    let outcome = stage
        .as_effect()
        .unwrap()
        .apply(args, EffectPlan::new(), tx, &context)?;
    match outcome.output.data {
        StageData::Records(stream) => stream.collect(),
        other => panic!("`sql` returned {other:?} instead of records"),
    }
}

#[test]
fn descriptor_names_a_source_command_without_input() {
    let descriptor = SqlStage.descriptor();
    assert_eq!(descriptor.command, "sql");
    assert!(descriptor.source);
    assert!(matches!(descriptor.input, InputKinds::None));
}

#[test]
fn check_reports_the_database_path() {
    let db = Path::new("/data/app.db");
    let info = check(&args(db, "select 1", Vec::new(), false)).unwrap();
    assert_eq!(info.paths, [text(db)]);
    let written = check(&args(db, "delete from users", vec![Value::Int(1)], true)).unwrap();
    assert_eq!(written.paths, [text(db)]);
}

#[test]
fn check_refuses_a_query_without_statements() {
    let error = check(&args(Path::new("/data/app.db"), " ; ; ", Vec::new(), false)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Sql);
    assert_eq!(error.problem.as_deref(), Some("empty_query"));
}

#[test]
fn check_rejects_arguments_of_other_types() {
    let db = Path::new("/data/app.db");
    let base = || args(db, "select 1", Vec::new(), false);
    let path = base().with("path", ArgValue::Value(Value::Int(1)), false);
    assert_eq!(code(check(&path)), ErrorCode::Type);
    let query = base().with("query", ArgValue::Value(Value::Bool(true)), false);
    assert_eq!(code(check(&query)), ErrorCode::Type);
    let params = base().with("params", ArgValue::Value(Value::Int(1)), false);
    assert_eq!(code(check(&params)), ErrorCode::Type);
    let write = base().with(
        "write",
        ArgValue::Value(Value::Text("yes".to_owned())),
        false,
    );
    assert!(check(&write).is_err());
}

#[test]
fn null_params_bind_no_values() {
    let db = Path::new("/data/app.db");
    let call =
        args(db, "select 1", Vec::new(), false).with("params", ArgValue::Value(Value::Null), false);
    assert_eq!(check(&call).unwrap().paths, [text(db)]);
}

#[test]
fn effect_step_without_a_planned_state_reads_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    let rows = apply_without_plan(
        &args(&db, "select name from users", Vec::new(), false),
        &mut tx,
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        cells(&rows[0]),
        [("name".to_owned(), Value::Text("ada".to_owned()))]
    );
    assert!(tx.declared.is_empty());
}

#[test]
fn effect_step_without_a_planned_state_writes_through_the_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(dir.path(), "app.db", USERS);
    let mut tx = RecordingTx::new();
    apply_without_plan(
        &args(
            &db,
            "insert into users(name) values (?)",
            vec![Value::Text("bob".to_owned())],
            true,
        ),
        &mut tx,
    )
    .unwrap();
    assert_eq!(count(&db, "select count(*) from users"), 2);
    assert!(!tx.declared.is_empty());
}

#[test]
fn effect_step_without_a_planned_state_reports_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("none.db");
    let mut tx = RecordingTx::new();
    let error = failure(apply_without_plan(
        &args(&missing, "select 1", Vec::new(), false),
        &mut tx,
    ));
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(!missing.exists());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
