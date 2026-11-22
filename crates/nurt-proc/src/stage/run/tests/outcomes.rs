// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Outcomes of `run` that the run host decides: runs over input records
//! that cannot finish, runs ended by a limit, and command lines with an
//! external effect.

use nurt_contracts::report::Irreversible;
use nurt_contracts::stage::EffectPlan;

use super::*;

#[test]
fn a_batch_that_moves_to_the_background_fails_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let host = DirectRunHost::new(HostMode::Background);
    let args = run_args(&["print-argv"], dir.path(), Vec::new());
    let error = execute(&host, &args, entries(&["a", "b"]), &ctx(&[])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::StageFailed);
    assert!(!error.data().effect_applied);
    assert_eq!(host.recorded().len(), 2);
    assert!(host.recorded().iter().all(|(_, wait)| wait.is_none()));
}

#[test]
fn a_batch_ended_by_a_limit_fails_with_the_combined_record() {
    let dir = tempfile::tempdir().unwrap();
    let host = DirectRunHost::new(HostMode::KilledBy(KilledBy::Timeout));
    let args = run_args(&["print-argv"], dir.path(), vec![("batch", Value::Int(2))]);
    let error = execute(&host, &args, entries(&["a", "b", "c"]), &ctx(&[])).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Timeout);
    assert!(error.data().effect_applied);
    let exec = &error.data().details["exec"];
    assert_eq!(exec["timed_out"], true);
    assert_eq!(exec["batches"].as_array().map(Vec::len), Some(2));
    assert_eq!(exec["exit_code"], -1);
}

#[test]
fn applying_without_a_plan_of_run_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = DirectRunHost::new(HostMode::Run);
    let stage = RunStage::with_host(Arc::clone(&host) as Arc<dyn RunHost>);
    assert!(stage.as_effect().is_some());
    let args = run_args(&["print-argv"], dir.path(), Vec::new());
    let mut tx = NullTx::default();
    let error = stage
        .apply(&args, EffectPlan::new(), &mut tx, &ctx(&[]))
        .err()
        .map(|error| error.code());
    assert_eq!(error, Some(ErrorCode::StageFailed));
    assert!(host.recorded().is_empty());
}

#[test]
fn a_command_with_an_external_effect_is_marked_irreversible() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.run.irreversible_patterns = vec!["proc_fixture print-argv".to_owned()];
    let ctx = context(TestHost::current(&[]), config, RequestOptions::default());
    let host = DirectRunHost::new(HostMode::Run);
    let stage = RunStage::with_host(Arc::clone(&host) as Arc<dyn RunHost>);
    let args = run_args(&["print-argv", "release"], dir.path(), Vec::new());
    let plan = stage.plan(&args, StageInput::none(), &ctx).unwrap();
    let mut tx = NullTx::default();
    let outcome = stage.apply(&args, plan, &mut tx, &ctx).unwrap();
    assert_eq!(outcome.irreversible.len(), 1);
    assert_eq!(outcome.irreversible[0].reason, Irreversible::ExternalEffect);
    assert_eq!(
        outcome.irreversible[0].detail.as_deref(),
        Some("proc_fixture print-argv")
    );
    assert_eq!(
        host.recorded()[0].0.external_effect,
        ["proc_fixture print-argv".to_owned()]
    );
    let quiet = run_args(&["print-cwd"], dir.path(), Vec::new());
    let plan = stage.plan(&quiet, StageInput::none(), &ctx).unwrap();
    let outcome = stage.apply(&quiet, plan, &mut tx, &ctx).unwrap();
    assert!(outcome.irreversible.is_empty());
}

#[test]
fn a_malformed_configured_pattern_fails_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.run.irreversible_patterns = vec!["--force".to_owned()];
    let ctx = context(TestHost::current(&[]), config, RequestOptions::default());
    let host = DirectRunHost::new(HostMode::Run);
    let args = run_args(&["print-argv"], dir.path(), Vec::new());
    let error = execute(&host, &args, StageInput::none(), &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Config);
    assert!(host.recorded().is_empty());
}

#[test]
fn a_variable_name_the_program_cannot_receive_fails_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = Fields::new();
    env.insert("A=B".to_owned(), text("1"));
    let host = DirectRunHost::new(HostMode::Run);
    let args = run_args(&["print-env"], dir.path(), vec![("env", Value::Map(env))]);
    assert!(execute(&host, &args, StageInput::none(), &ctx(&[])).is_err());
    assert!(host.recorded().is_empty());
}
