// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `registry:` paths in the file commands, on a registry in
//! memory.

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::record::ChangeAction;
use nurt_contracts::stage::{
    ArgValue, BoundArgs, EffectStage, ExecCtx, ReadStage, RecordStream, StageInput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_testkit::ctx::TestContext;
use nurt_testkit::host::TestHost;

use crate::commands::copy::Copy;
use crate::commands::find::Find;
use crate::commands::info::Info;
use crate::commands::list::List;
use crate::commands::move_entries::Move;
use crate::commands::read::Read;
use crate::commands::remove::Remove;
use crate::registry::testing::MemRegistry;

const KEY: &str = "registry:/HKCU/Software/Nurt";

fn call(command: &str, given: &[(&str, Value)]) -> BoundArgs {
    given
        .iter()
        .fold(BoundArgs::new(command), |args, (name, value)| {
            args.with(*name, ArgValue::Value(value.clone()), false)
        })
}

fn path(text: &str) -> Value {
    Value::Path(text.to_owned())
}

fn paths(texts: &[&str]) -> Value {
    Value::List(texts.iter().map(|text| path(text)).collect())
}

fn windows() -> TestContext {
    TestContext::with_host(TestHost::windows())
}

/// Context that reaches a registry with `HKCU/Software/Nurt`, its value
/// `Mode`, and below it the key `Sub` with the value `Leaf`.
fn with_registry(test: &TestContext) -> ExecCtx {
    let registry = MemRegistry::new(
        &[
            "HKCU",
            "HKLM",
            "HKCU/Software",
            "HKCU/Software/Nurt",
            "HKCU/Software/Nurt/Sub",
        ],
        &[
            ("HKCU/Software/Nurt/Mode", "3"),
            ("HKCU/Software/Nurt/Sub/Leaf", "x"),
        ],
    );
    test.builder().extensions(registry.extensions()).build()
}

fn records(stage: &dyn ReadStage, args: &BoundArgs, ctx: &ExecCtx) -> Result<Vec<Record>> {
    let output = stage.execute(args, StageInput::none(), ctx)?;
    StageInput::from(output).into_records()?.collect()
}

fn paths_of(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| match record.get("path") {
            Some(Value::Path(text) | Value::Text(text)) => {
                let name = match record.get("name") {
                    Some(Value::Text(name)) if record.kind == RecordKind::RegistryValue => {
                        format!("/{name}")
                    }
                    _ => String::new(),
                };
                format!("{}{name}", text.as_str())
            }
            _ => String::new(),
        })
        .collect()
}

#[test]
fn a_host_without_a_registry_rejects_registry_paths() {
    let test = TestContext::new();
    let ctx = test.ctx();
    let read = Read::default();
    let stages: [(&dyn ReadStage, &str); 4] = [
        (&read, "read"),
        (&Info, "info"),
        (&List, "list"),
        (&Find, "find"),
    ];
    for (stage, command) in stages {
        let args = call(command, &[("path", path(KEY))]);
        let error = records(stage, &args, &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::OsOnly);
    }
    let error = Remove
        .plan(
            &call("remove", &[("path", path(KEY))]),
            StageInput::none(),
            &ctx,
        )
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::OsOnly);
}

#[test]
fn a_command_without_registry_support_names_the_commands_that_have_it() {
    let test = windows();
    let ctx = test.ctx();
    let error = crate::common::path::resolve(&ctx, "registry:/HKCU/Software/Nurt").unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
    assert!(error.message.contains("`list`"), "{}", error.message);
}

#[test]
fn read_returns_the_record_of_a_value_and_refuses_a_key() {
    let test = windows();
    let ctx = with_registry(&test);
    let value = format!("{KEY}/Mode");
    let found = records(
        &Read::default(),
        &call("read", &[("path", path(&value))]),
        &ctx,
    )
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, RecordKind::RegistryValue);
    assert_eq!(found[0].get("value"), Some(&Value::Text("3".to_owned())));
    let error = records(
        &Read::default(),
        &call("read", &[("path", path(KEY))]),
        &ctx,
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::Io);
}

#[test]
fn registry_and_file_paths_are_not_mixed() {
    let test = windows();
    let ctx = with_registry(&test);
    let given = call("read", &[("path", paths(&[KEY, "/data/a.txt"]))]);
    let error = records(&Read::default(), &given, &ctx).unwrap_err();
    assert_eq!(error.code(), ErrorCode::InvalidValue);
}

#[test]
fn info_describes_keys_and_marks_missing_paths() {
    let test = windows();
    let ctx = with_registry(&test);
    let missing = format!("{KEY}/Absent");
    let given = call("info", &[("path", paths(&[KEY, &missing]))]);
    let found = records(&Info, &given, &ctx).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].get("exists"), Some(&Value::Bool(true)));
    assert_eq!(found[1].get("exists"), Some(&Value::Bool(false)));
}

#[test]
fn list_shows_keys_before_values_to_the_requested_depth() {
    let test = windows();
    let ctx = with_registry(&test);
    let shallow = records(&List, &call("list", &[("path", path(KEY))]), &ctx).unwrap();
    assert_eq!(shallow.len(), 2);
    assert_eq!(shallow[0].kind, RecordKind::Entry);
    assert_eq!(shallow[1].kind, RecordKind::RegistryValue);
    let deep = call("list", &[("path", path(KEY)), ("depth", Value::Int(2))]);
    assert_eq!(records(&List, &deep, &ctx).unwrap().len(), 3);
}

#[test]
fn find_selects_values_by_name_and_includes_the_root_for_any_type() {
    let test = windows();
    let ctx = with_registry(&test);
    let by_name = call(
        "find",
        &[
            ("path", path(KEY)),
            ("glob", Value::Glob("lea*".to_owned())),
        ],
    );
    let found = records(&Find, &by_name, &ctx).unwrap();
    assert_eq!(paths_of(&found), ["registry:/hkcu/software/nurt/sub/leaf"]);
    let any = call(
        "find",
        &[("path", path(KEY)), ("type", Value::Enum("any".to_owned()))],
    );
    let all = records(&Find, &any, &ctx).unwrap();
    assert_eq!(all.len(), 4, "the root, the subkey and two values");
}

#[test]
fn remove_declares_one_delete_target_for_the_outermost_paths() {
    let test = windows();
    let ctx = with_registry(&test);
    let value = format!("{KEY}/Mode");
    let sub = format!("{KEY}/Sub");
    let plan = Remove
        .plan(
            &call("remove", &[("path", paths(&[&value, &sub, KEY]))]),
            StageInput::none(),
            &ctx,
        )
        .unwrap();
    assert_eq!(
        plan.changes.len(),
        1,
        "the key holds the value and the subkey"
    );
    assert_eq!(plan.changes[0].action, ChangeAction::Delete);
    assert_eq!(plan.changes[0].path.as_str(), KEY);
    assert_eq!(plan.changes[0].flags, Some(vec!["recursive".to_owned()]));

    let two = Remove
        .plan(
            &call("remove", &[("path", paths(&[&value, &sub]))]),
            StageInput::none(),
            &ctx,
        )
        .unwrap();
    assert_eq!(two.changes.len(), 2);
}

#[test]
fn remove_takes_value_records_from_the_pipeline() {
    let test = windows();
    let ctx = with_registry(&test);
    let value = Record::new(RecordKind::RegistryValue)
        .with("path", path(KEY))
        .with("name", Value::Text("Mode".to_owned()));
    let plan = Remove
        .plan(
            &call("remove", &[]),
            StageInput::records(RecordStream::from_records(vec![value])),
            &ctx,
        )
        .unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].action, ChangeAction::Delete);
    assert_eq!(
        plan.changes[0].path.as_str(),
        "registry:/HKCU/Software/Nurt/Mode"
    );
}

#[test]
fn a_value_record_names_its_path_with_an_escaped_name_or_the_default() {
    let record = |name: &str| {
        Record::new(RecordKind::RegistryValue)
            .with("path", path(KEY))
            .with("name", Value::Text(name.to_owned()))
    };
    let text = |name: &str| crate::registry::record_text(&record(name)).unwrap();
    assert_eq!(text(""), format!("{KEY}/(default)"));
    assert_eq!(text("a/b%"), format!("{KEY}/a%2Fb%25"));
}

#[test]
fn remove_skips_missing_paths_and_refuses_root_keys() {
    let test = windows();
    let ctx = with_registry(&test);
    let missing = format!("{KEY}/Absent");
    let skipped = Remove
        .plan(
            &call("remove", &[("path", path(&missing))]),
            StageInput::none(),
            &ctx,
        )
        .unwrap();
    assert!(skipped.changes.is_empty());
    let strict = call(
        "remove",
        &[
            ("path", path(&missing)),
            ("on_missing", Value::Enum("error".to_owned())),
        ],
    );
    assert_eq!(
        Remove
            .plan(&strict, StageInput::none(), &ctx)
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    let root = call("remove", &[("path", path("registry:/HKCU"))]);
    assert_eq!(
        Remove
            .plan(&root, StageInput::none(), &ctx)
            .unwrap_err()
            .code(),
        ErrorCode::Permission
    );
}

#[test]
fn copy_declares_a_created_destination_that_names_its_source() {
    let test = windows();
    let ctx = with_registry(&test);
    let copy = call(
        "copy",
        &[
            ("from", path(KEY)),
            ("to", path("registry:/HKCU/Software/Backup")),
        ],
    );
    let plan = Copy.plan(&copy, StageInput::none(), &ctx).unwrap();
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].action, ChangeAction::Create);
    assert_eq!(
        plan.changes[0].path.as_str(),
        "registry:/HKCU/Software/Backup"
    );
    assert_eq!(
        plan.changes[0].from.as_ref().map(|from| from.as_str()),
        Some(KEY)
    );
}

#[test]
fn copy_into_a_key_keeps_the_name_of_the_source() {
    let test = windows();
    let ctx = with_registry(&test);
    let value = format!("{KEY}/Mode");
    let sub = format!("{KEY}/Sub");
    let copy = call(
        "copy",
        &[
            ("from", paths(&[&value, &sub])),
            ("into", path("registry:/HKCU/Software/Other")),
        ],
    );
    let plan = Copy.plan(&copy, StageInput::none(), &ctx).unwrap();
    let targets: Vec<&str> = plan
        .changes
        .iter()
        .map(|change| change.path.as_str())
        .collect();
    assert_eq!(
        targets,
        [
            "registry:/HKCU/Software/Other/Mode",
            "registry:/HKCU/Software/Other/Sub"
        ]
    );
}

#[test]
fn several_sources_need_into_and_files_do_not_mix_with_the_registry() {
    let test = windows();
    let ctx = with_registry(&test);
    let value = format!("{KEY}/Mode");
    let several = call(
        "copy",
        &[
            ("from", paths(&[&value, KEY])),
            ("to", path("registry:/HKCU/Software/Backup")),
        ],
    );
    assert_eq!(
        Copy.plan(&several, StageInput::none(), &ctx)
            .unwrap_err()
            .code(),
        ErrorCode::ParamConflict
    );
    let across = call("copy", &[("from", path(KEY)), ("to", path("/data/out"))]);
    assert_eq!(
        Copy.plan(&across, StageInput::none(), &ctx)
            .unwrap_err()
            .code(),
        ErrorCode::InvalidValue
    );
}

#[test]
fn move_copies_then_removes_the_source_in_one_plan() {
    let test = windows();
    let ctx = with_registry(&test);
    let sub = format!("{KEY}/Sub");
    let moved = call(
        "move",
        &[
            ("from", path(&sub)),
            ("to", path("registry:/HKCU/Software/Moved")),
        ],
    );
    let plan = Move.plan(&moved, StageInput::none(), &ctx).unwrap();
    let actions: Vec<ChangeAction> = plan.changes.iter().map(|change| change.action).collect();
    assert_eq!(actions, [ChangeAction::Create, ChangeAction::Delete]);
    assert_eq!(plan.changes[1].path.as_str(), sub);

    let root = call(
        "move",
        &[
            ("from", path("registry:/HKCU")),
            ("to", path("registry:/HKLM/Copy")),
        ],
    );
    assert_eq!(
        Move.plan(&root, StageInput::none(), &ctx)
            .unwrap_err()
            .code(),
        ErrorCode::Permission
    );
}
