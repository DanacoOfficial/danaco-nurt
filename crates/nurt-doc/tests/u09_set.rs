// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the `set` stage and of `remove` for `item` records: layout of
//! changed documents, transactions over several keys, entity tags,
//! validation, created files, undo through the journal and the operations
//! of every registered format.

// Test helpers report setup failures by panicking; clippy exempts only `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "testing")]

mod u09_support;

use std::path::Path;

use nurt_contracts::error::{ErrorCode, Result};
use nurt_contracts::journal::{Expect, OpKind, TargetAction};
use nurt_contracts::record::{ChangeAction, Item};
use nurt_contracts::report::Notable;
use nurt_contracts::stage::{Event, LogLevel};
use nurt_contracts::value::{Record, RecordKind, Size, Value};
use nurt_doc::commands::set::{RemoveItemsStage, SetStage, VALIDATE_WARNING_FLAG};
use nurt_testkit::hash::sha256;

use u09_support::{
    Fixture, Outcome, args, effect, effect_with, etag, failure, items, map, path, query,
    query_file, text,
};

const CONFIG: &str = "/data/config.json";

/// A JSON document with a text, a number and a list, indented by two
/// spaces.
const CONFIG_TEXT: &str = "{\n  \"name\": \"web\",\n  \"port\": 80,\n  \"list\": [1, 2, 3]\n}\n";

const SETTINGS: &str = "/data/settings.kvc";

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.file(CONFIG, CONFIG_TEXT.as_bytes());
    fixture
}

fn set_on(fixture: &Fixture, file: &str, values: &[(&str, Value)]) -> Result<Outcome> {
    let mut all = vec![("path", path(file))];
    all.extend_from_slice(values);
    effect(
        fixture,
        &SetStage::new(fixture.registry.clone()),
        &args("set", &all),
        None,
    )
}

fn set(fixture: &Fixture, values: &[(&str, Value)]) -> Result<Outcome> {
    set_on(fixture, CONFIG, values)
}

fn json(fixture: &Fixture, file: &str) -> serde_json::Value {
    serde_json::from_slice(&fixture.content(file).unwrap()).unwrap()
}

fn has_notable(outcome: &Outcome, code: Notable) -> bool {
    outcome.notable.iter().any(|entry| entry.code == code)
}

#[test]
fn one_key_changes_only_its_value() {
    let fixture = fixture();
    let outcome = set(
        &fixture,
        &[("key", text("port")), ("value", Value::Int(8080))],
    )
    .unwrap();
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT.replace("80,", "8080,"));
    assert_eq!(outcome.records.len(), 1);
    assert_eq!(outcome.records[0].kind, RecordKind::Effect);
    let change = &outcome.changes[0];
    assert_eq!(change.action, ChangeAction::Modify);
    assert_eq!(change.replacements, Some(1));
    assert_eq!(
        change.size_before,
        Some(Size::from_bytes(CONFIG_TEXT.len() as u64))
    );
    assert_eq!(
        change.size_after,
        Some(Size::from_bytes(CONFIG_TEXT.len() as u64 + 2))
    );
    assert_eq!(
        change.hash_before.as_deref(),
        Some(etag(CONFIG_TEXT.as_bytes()).as_str())
    );
    assert_eq!(change.flags, None);
    assert_eq!(outcome.declared.len(), 1);
    assert_eq!(outcome.declared[0].action, TargetAction::Modify);
    assert!(
        matches!(&outcome.declared[0].expect, Expect::Hash(hash) if *hash == sha256(CONFIG_TEXT.as_bytes())),
        "{:?}",
        outcome.declared[0].expect
    );
}

#[test]
fn values_change_several_keys_in_one_operation() {
    let fixture = fixture();
    let values = map(&[("name", text("api")), ("port", Value::Int(81))]);
    let outcome = set(&fixture, &[("values", values)]).unwrap();
    assert_eq!(
        json(&fixture, CONFIG),
        serde_json::json!({"name": "api", "port": 81, "list": [1, 2, 3]})
    );
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.changes[0].replacements, Some(2));
    assert_eq!(fixture.journal.operations(), 1);
}

#[test]
fn a_failing_key_leaves_the_file_unchanged() {
    let fixture = fixture();
    let values = map(&[("port", Value::Int(1)), ("missing", Value::Int(2))]);
    let error = failure(set(
        &fixture,
        &[("values", values), ("create", Value::Bool(false))],
    ));
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT);
    assert_eq!(fixture.journal.operations(), 0);
}

#[test]
fn missing_keys_offer_the_closest_keys() {
    let fixture = fixture();
    let error = failure(set(
        &fixture,
        &[
            ("key", text("prot")),
            ("value", Value::Int(1)),
            ("create", Value::Bool(false)),
        ],
    ));
    assert_eq!(error.code, ErrorCode::KeyNotFound);
    assert!(
        error
            .candidates
            .iter()
            .any(|candidate| candidate.program == "port"),
        "{:?}",
        error.candidates
    );
}

#[test]
fn overlapping_keys_conflict() {
    let fixture = fixture();
    let values = map(&[
        ("list", Value::List(Vec::new())),
        ("list[0]", Value::Int(1)),
    ]);
    let error = failure(set(&fixture, &[("values", values)]));
    assert_eq!(error.code, ErrorCode::KeyConflict);
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT);
}

#[test]
fn the_current_etag_allows_the_change() {
    let fixture = fixture();
    let tag = etag(CONFIG_TEXT.as_bytes());
    set(
        &fixture,
        &[
            ("key", text("port")),
            ("value", Value::Int(1)),
            ("if_match", text(&tag)),
        ],
    )
    .unwrap();
    assert_eq!(json(&fixture, CONFIG)["port"], 1);
}

#[test]
fn an_old_etag_is_stale() {
    let fixture = fixture();
    let old = etag(b"{}");
    let error = failure(set(
        &fixture,
        &[
            ("key", text("port")),
            ("value", Value::Int(1)),
            ("if_match", text(&old)),
        ],
    ));
    assert_eq!(error.code, ErrorCode::Stale);
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT);
}

#[test]
fn a_malformed_etag_is_rejected() {
    let error = failure(set(
        &fixture(),
        &[
            ("key", text("port")),
            ("value", Value::Int(1)),
            ("if_match", text("W/\"1\"")),
        ],
    ));
    assert_eq!(error.code, ErrorCode::InvalidValue);
}

#[test]
fn a_change_between_plan_and_write_is_stale() {
    let fixture = fixture();
    let stage = SetStage::new(fixture.registry.clone());
    let call = args(
        "set",
        &[
            ("path", path(CONFIG)),
            ("key", text("port")),
            ("value", Value::Int(1)),
        ],
    );
    let error = failure(effect_with(&fixture, &stage, &call, None, |fixture| {
        fixture.file(CONFIG, b"{\"port\": 2}\n");
    }));
    assert_eq!(error.code, ErrorCode::Stale);
    assert_eq!(fixture.text(CONFIG), "{\"port\": 2}\n");
    assert_eq!(fixture.journal.operations(), 0);
}

#[test]
fn a_missing_file_is_created() {
    let fixture = fixture();
    let outcome = set_on(
        &fixture,
        "/data/new.json",
        &[("key", text("server.port")), ("value", Value::Int(8080))],
    )
    .unwrap();
    assert_eq!(
        json(&fixture, "/data/new.json"),
        serde_json::json!({"server": {"port": 8080}})
    );
    assert_eq!(outcome.changes[0].action, ChangeAction::Create);
    assert_eq!(outcome.changes[0].size_before, None);
    assert_eq!(outcome.declared[0].action, TargetAction::Create);
    assert!(matches!(outcome.declared[0].expect, Expect::Absent));
}

#[test]
fn a_missing_file_without_create_is_not_found() {
    let error = failure(set_on(
        &fixture(),
        "/data/new.json",
        &[
            ("key", text("a")),
            ("value", Value::Int(1)),
            ("create", Value::Bool(false)),
        ],
    ));
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn the_current_value_is_not_written() {
    let fixture = fixture();
    let outcome = set(
        &fixture,
        &[("key", text("port")), ("value", Value::Int(80))],
    )
    .unwrap();
    assert!(outcome.changes.is_empty());
    assert!(has_notable(&outcome, Notable::Unchanged));
    assert_eq!(fixture.journal.operations(), 0);
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT);
}

#[test]
fn new_intermediate_keys_are_notable() {
    let fixture = fixture();
    let outcome = set(
        &fixture,
        &[
            ("key", text("tls.cert.path")),
            ("value", text("/etc/cert.pem")),
        ],
    )
    .unwrap();
    assert_eq!(
        json(&fixture, CONFIG)["tls"],
        serde_json::json!({"cert": {"path": "/etc/cert.pem"}})
    );
    assert!(
        has_notable(&outcome, Notable::KeysCreated),
        "{:?}",
        outcome.notable
    );
}

#[test]
fn list_positions_follow_the_key_rules() {
    let fixture = fixture();
    set(
        &fixture,
        &[("key", text("list[+]")), ("value", Value::Int(4))],
    )
    .unwrap();
    set(
        &fixture,
        &[("key", text("list[4]")), ("value", Value::Int(5))],
    )
    .unwrap();
    assert_eq!(
        json(&fixture, CONFIG)["list"],
        serde_json::json!([1, 2, 3, 4, 5])
    );
    let error = failure(set(
        &fixture,
        &[("key", text("list[9]")), ("value", Value::Int(9))],
    ));
    assert_eq!(error.code, ErrorCode::KeyConflict);
}

#[test]
fn a_wildcard_sets_every_match() {
    let fixture = fixture();
    let outcome = set(
        &fixture,
        &[("key", text("list[*]")), ("value", Value::Int(0))],
    )
    .unwrap();
    assert_eq!(json(&fixture, CONFIG)["list"], serde_json::json!([0, 0, 0]));
    assert_eq!(outcome.changes[0].replacements, Some(3));
}

#[test]
fn the_stage_form_sets_the_items_of_query() {
    let fixture = fixture();
    let found = query(
        &fixture,
        &args(
            "query",
            &[("expression", text(".port")), ("path", path(CONFIG))],
        ),
        None,
    )
    .unwrap();
    let stage = SetStage::new(fixture.registry.clone());
    effect(
        &fixture,
        &stage,
        &args("set", &[("value", Value::Int(8080))]),
        Some(found),
    )
    .unwrap();
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT.replace("80,", "8080,"));
}

#[test]
fn items_of_one_file_under_different_spellings_change_it_once() {
    let fixture = fixture();
    let read = |expression: &str| {
        query(
            &fixture,
            &args(
                "query",
                &[("expression", text(expression)), ("path", path(CONFIG))],
            ),
            None,
        )
        .unwrap()
    };
    let mut found = read(".port");
    let respelled = items(&read(".name")).into_iter().map(|item| {
        Record::from(Item {
            path: nurt_contracts::value::Path::new("/data/./config.json"),
            ..item
        })
    });
    found.extend(respelled);
    let stage = SetStage::new(fixture.registry.clone());
    let outcome = effect(
        &fixture,
        &stage,
        &args("set", &[("value", Value::Int(5))]),
        Some(found),
    )
    .unwrap();
    assert_eq!(outcome.changes.len(), 1);
    assert_eq!(outcome.changes[0].replacements, Some(2));
    assert_eq!(
        json(&fixture, CONFIG),
        serde_json::json!({"name": 5, "port": 5, "list": [1, 2, 3]})
    );
}

#[test]
fn the_stage_form_checks_the_etag_of_items() {
    let fixture = fixture();
    let found = query(
        &fixture,
        &args(
            "query",
            &[("expression", text(".port")), ("path", path(CONFIG))],
        ),
        None,
    )
    .unwrap();
    fixture.file(CONFIG, b"{\"port\": 2}\n");
    let stage = SetStage::new(fixture.registry.clone());
    let error = failure(effect(
        &fixture,
        &stage,
        &args("set", &[("value", Value::Int(1))]),
        Some(found),
    ));
    assert_eq!(error.code, ErrorCode::Stale);
}

#[test]
fn remove_deletes_the_keys_of_items() {
    let fixture = fixture();
    let found = query(
        &fixture,
        &args(
            "query",
            &[("expression", text(".name")), ("path", path(CONFIG))],
        ),
        None,
    )
    .unwrap();
    let stage = RemoveItemsStage::new(fixture.registry.clone());
    let outcome = effect(&fixture, &stage, &args("remove", &[]), Some(found)).unwrap();
    assert_eq!(
        json(&fixture, CONFIG),
        serde_json::json!({"port": 80, "list": [1, 2, 3]})
    );
    assert_eq!(outcome.changes[0].action, ChangeAction::Modify);
}

#[test]
fn undo_restores_every_byte() {
    let fixture = fixture();
    let values = map(&[("name", text("api")), ("extra.flag", Value::Bool(true))]);
    set(&fixture, &[("values", values)]).unwrap();
    assert_eq!(fixture.journal.last_kind(), Some(OpKind::command("set")));
    fixture.journal.undo_last().unwrap();
    assert_eq!(
        fixture.journal_content(CONFIG).as_deref(),
        Some(CONFIG_TEXT.as_bytes())
    );
}

#[test]
fn undo_removes_a_created_file() {
    let fixture = fixture();
    set_on(
        &fixture,
        "/data/new.json",
        &[("key", text("a")), ("value", Value::Int(1))],
    )
    .unwrap();
    fixture.journal.undo_last().unwrap();
    assert_eq!(fixture.journal_content("/data/new.json"), None);
}

#[test]
fn files_change_only_through_the_transaction() {
    let fixture = fixture();
    let before = fixture.fs_mutations();
    let stage = SetStage::new(fixture.registry.clone());
    let call = args(
        "set",
        &[
            ("path", path(CONFIG)),
            ("key", text("port")),
            ("value", Value::Int(1)),
        ],
    );
    let ctx = fixture.ctx();
    let effect_stage = nurt_contracts::stage::StageImpl::as_effect(&stage).unwrap();
    let plan = effect_stage
        .plan(&call, u09_support::stage_input(None), &ctx)
        .unwrap();
    assert_eq!(fixture.fs_mutations(), before);
    let mut tx = fixture.journal.begin(OpKind::command("set"));
    effect_stage.apply(&call, plan, tx.as_mut(), &ctx).unwrap();
    tx.commit().unwrap();
    assert_eq!(fixture.fs_mutations(), before);
    assert_eq!(fixture.text(CONFIG), CONFIG_TEXT);
    let written = String::from_utf8(fixture.journal_content(CONFIG).unwrap()).unwrap();
    assert_eq!(written, CONFIG_TEXT.replace("80,", "1,"));
}

#[test]
fn validate_rejects_a_new_syntax_error() {
    let fixture = Fixture::new();
    fixture.file(SETTINGS, b"a = 1\nb = 2\n");
    let error = failure(set_on(
        &fixture,
        SETTINGS,
        &[("key", text("a")), ("value", text("x!!y"))],
    ));
    assert_eq!(error.code, ErrorCode::DocParse);
    assert_eq!(error.problem.as_deref(), Some("validate"));
    assert_eq!(
        error.location.as_ref().and_then(|location| location.line),
        Some(1)
    );
    assert_eq!(fixture.text(SETTINGS), "a = 1\nb = 2\n");
}

#[test]
fn validate_warn_writes_and_flags_the_change() {
    let fixture = Fixture::new();
    fixture.file(SETTINGS, b"a = 1\n");
    let outcome = set_on(
        &fixture,
        SETTINGS,
        &[
            ("key", text("a")),
            ("value", text("x!!y")),
            ("validate", text("warn")),
        ],
    )
    .unwrap();
    assert_eq!(fixture.text(SETTINGS), "a = x!!y\n");
    assert_eq!(
        outcome.changes[0].flags,
        Some(vec![VALIDATE_WARNING_FLAG.to_owned()])
    );
    let warned = fixture
        .test
        .events()
        .events()
        .into_iter()
        .any(|event| matches!(event, Event::Log(log) if log.level == LogLevel::Warn));
    assert!(warned);
}

#[test]
fn validate_off_skips_the_check() {
    let fixture = Fixture::new();
    fixture.file(SETTINGS, b"a = 1\n");
    let outcome = set_on(
        &fixture,
        SETTINGS,
        &[
            ("key", text("a")),
            ("value", text("x!!y")),
            ("validate", text("off")),
        ],
    )
    .unwrap();
    assert_eq!(fixture.text(SETTINGS), "a = x!!y\n");
    assert_eq!(outcome.changes[0].flags, None);
}

#[test]
fn existing_syntax_errors_do_not_block_other_changes() {
    let fixture = Fixture::new();
    fixture.file(SETTINGS, b"a = !!\nb = 2\n");
    set_on(
        &fixture,
        SETTINGS,
        &[("key", text("b")), ("value", Value::Int(3))],
    )
    .unwrap();
    assert_eq!(fixture.text(SETTINGS), "a = !!\nb = 3\n");
    let error = failure(set_on(
        &fixture,
        SETTINGS,
        &[("key", text("b")), ("value", text("!!"))],
    ));
    assert_eq!(error.problem.as_deref(), Some("validate"));
}

#[test]
fn an_unknown_validate_mode_is_rejected() {
    let error = failure(set(
        &fixture(),
        &[
            ("key", text("port")),
            ("value", Value::Int(1)),
            ("validate", text("strict")),
        ],
    ));
    assert_eq!(error.code, ErrorCode::InvalidValue);
}

#[test]
fn values_must_be_a_map() {
    let error = failure(set(
        &fixture(),
        &[("values", Value::List(vec![Value::Int(1)]))],
    ));
    assert_eq!(error.code, ErrorCode::Type);
}

#[test]
fn a_key_needs_a_value() {
    let error = failure(set(&fixture(), &[("key", text("port"))]));
    assert_eq!(error.code, ErrorCode::MissingParam);
    let nothing = failure(set(&fixture(), &[]));
    assert_eq!(nothing.code, ErrorCode::MissingParam);
}

#[test]
fn an_explicit_format_overrides_detection() {
    let fixture = Fixture::new();
    fixture.file("/data/settings.txt", b"a = 1\n");
    set_on(
        &fixture,
        "/data/settings.txt",
        &[
            ("key", text("a")),
            ("value", Value::Int(2)),
            ("format", text("kv")),
        ],
    )
    .unwrap();
    assert_eq!(fixture.text("/data/settings.txt"), "a = 2\n");
}

/// Sample document of a format for the operation matrix.
struct Sample {
    format: &'static str,
    file: &'static str,
    content: &'static str,
    /// Key of a scalar that the format can replace and remove.
    scalar: &'static str,
    /// Key of a member that does not exist yet.
    new_key: Option<&'static str>,
    /// Key of a list of three integers.
    list: Option<&'static str>,
}

const PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>name</key>\n\t<string>web</string>\n\t<key>list</key>\n\t<array>\n\t\t<integer>1</integer>\n\t\t<integer>2</integer>\n\t\t<integer>3</integer>\n\t</array>\n</dict>\n</plist>\n";

const SAMPLES: &[Sample] = &[
    Sample {
        format: "json",
        file: "/m/a.json",
        content: "{\"name\": \"web\", \"port\": 80, \"list\": [1, 2, 3]}\n",
        scalar: "name",
        new_key: Some("extra"),
        list: Some("list"),
    },
    Sample {
        format: "jsonc",
        file: "/m/a.jsonc",
        content: "{\n  // service\n  \"name\": \"web\",\n  \"list\": [1, 2, 3],\n}\n",
        scalar: "name",
        new_key: Some("extra"),
        list: Some("list"),
    },
    Sample {
        format: "yaml",
        file: "/m/a.yaml",
        content: "name: web\nport: 80\nlist:\n  - 1\n  - 2\n  - 3\n",
        scalar: "name",
        new_key: Some("extra"),
        list: Some("list"),
    },
    Sample {
        format: "toml",
        file: "/m/a.toml",
        content: "name = \"web\"\nport = 80\nlist = [1, 2, 3]\n",
        scalar: "name",
        new_key: Some("extra"),
        list: Some("list"),
    },
    Sample {
        format: "xml",
        file: "/m/a.xml",
        content: "<config>\n  <name>web</name>\n  <port>80</port>\n</config>\n",
        scalar: "config.name",
        new_key: Some("config.extra"),
        list: None,
    },
    Sample {
        format: "ini",
        file: "/m/a.ini",
        content: "[server]\nname = web\nport = 80\n",
        scalar: "server.name",
        new_key: Some("server.extra"),
        list: None,
    },
    Sample {
        format: "csv",
        file: "/m/a.csv",
        content: "name,port\nweb,80\napi,81\n",
        scalar: "[0].name",
        new_key: None,
        list: None,
    },
    Sample {
        format: "tsv",
        file: "/m/a.tsv",
        content: "name\tport\nweb\t80\n",
        scalar: "[0].name",
        new_key: None,
        list: None,
    },
    Sample {
        format: "properties",
        file: "/m/a.properties",
        content: "name=web\nport=80\n",
        scalar: "name",
        new_key: Some("extra"),
        list: None,
    },
    Sample {
        format: "env",
        file: "/m/a.env",
        content: "NAME=web\nPORT=80\n",
        scalar: "NAME",
        new_key: Some("EXTRA"),
        list: None,
    },
    Sample {
        format: "plist",
        file: "/m/a.plist",
        content: PLIST,
        scalar: "name",
        new_key: Some("extra"),
        list: Some("list"),
    },
];

/// Runs one operation on a fresh copy of the sample, reads the result back
/// with `query` and undoes the change through the journal.
fn operation(
    sample: &Sample,
    values: &[(&str, Value)],
    read: &str,
    expected: &Value,
) -> std::result::Result<(), String> {
    let fixture = Fixture::new();
    fixture.file(sample.file, sample.content.as_bytes());
    set_on(&fixture, sample.file, values).map_err(|error| error.message.clone())?;
    let found = query_file(&fixture, sample.file, read).map_err(|error| error.message.clone())?;
    let actual = found.first().map(|item| item.value.to_plain_json());
    if actual.as_ref() != Some(&expected.to_plain_json()) {
        return Err(format!("`{read}` reads {actual:?} instead of {expected:?}"));
    }
    fixture
        .journal
        .undo_last()
        .map_err(|error| error.message.clone())?;
    if fixture.journal_content(sample.file).as_deref() != Some(sample.content.as_bytes()) {
        return Err("undo does not restore the original bytes".to_owned());
    }
    Ok(())
}

/// Removing a cell of a delimited file empties it, so the cell stays readable.
fn is_tabular(format: &str) -> bool {
    matches!(format, "csv" | "tsv")
}

fn removal(sample: &Sample) -> std::result::Result<(), String> {
    let fixture = Fixture::new();
    fixture.file(sample.file, sample.content.as_bytes());
    let read = format!(".{}", sample.scalar);
    let found = query(
        &fixture,
        &args(
            "query",
            &[("expression", text(&read)), ("path", path(sample.file))],
        ),
        None,
    )
    .map_err(|error| error.message.clone())?;
    let stage = RemoveItemsStage::new(fixture.registry.clone());
    effect(&fixture, &stage, &args("remove", &[]), Some(found))
        .map_err(|error| error.message.clone())?;
    let after = query_file(&fixture, sample.file, &read).map_err(|error| error.message.clone())?;
    match after.first().map(|item| &item.value) {
        Some(Value::Null) | None => Ok(()),
        Some(Value::Text(text)) if text.is_empty() && is_tabular(sample.format) => Ok(()),
        Some(other) => Err(format!("`{read}` still reads {other:?} after the removal")),
    }
}

fn list_operations(sample: &Sample, list: &str) -> Vec<(String, std::result::Result<(), String>)> {
    let length = format!(".{list} | length");
    let mut results = vec![
        (
            "append".to_owned(),
            operation(
                sample,
                &[
                    ("key", text(&format!("{list}[+]"))),
                    ("value", Value::Int(4)),
                ],
                &length,
                &Value::Int(4),
            ),
        ),
        (
            "insert at the end".to_owned(),
            operation(
                sample,
                &[
                    ("key", text(&format!("{list}[3]"))),
                    ("value", Value::Int(4)),
                ],
                &length,
                &Value::Int(4),
            ),
        ),
    ];
    let fixture = Fixture::new();
    fixture.file(sample.file, sample.content.as_bytes());
    let beyond = match set_on(
        &fixture,
        sample.file,
        &[
            ("key", text(&format!("{list}[9]"))),
            ("value", Value::Int(9)),
        ],
    ) {
        Err(error) if error.code == ErrorCode::KeyConflict => Ok(()),
        Err(error) => Err(format!("an index past the end fails with {:?}", error.code)),
        Ok(_) => Err("an index past the end is accepted".to_owned()),
    };
    results.push(("index past the end".to_owned(), beyond));
    results
}

fn sample_results(sample: &Sample) -> Vec<(String, std::result::Result<(), String>)> {
    let read = format!(".{}", sample.scalar);
    let mut results = vec![
        (
            "replace a scalar".to_owned(),
            operation(
                sample,
                &[("key", text(sample.scalar)), ("value", text("nurt"))],
                &read,
                &text("nurt"),
            ),
        ),
        ("remove".to_owned(), removal(sample)),
    ];
    if let Some(new_key) = sample.new_key {
        let read_new = format!(".{new_key}");
        results.push((
            "insert a key".to_owned(),
            operation(
                sample,
                &[("key", text(new_key)), ("value", text("added"))],
                &read_new,
                &text("added"),
            ),
        ));
        let values = map(&[(sample.scalar, text("nurt")), (new_key, text("added"))]);
        results.push((
            "several keys".to_owned(),
            operation(sample, &[("values", values)], &read_new, &text("added")),
        ));
    }
    if let Some(list) = sample.list {
        results.extend(list_operations(sample, list));
    }
    results
}

#[test]
fn every_registered_format_supports_the_operations() {
    let registry = u09_support::registry();
    let mut failures = Vec::new();
    let mut covered = Vec::new();
    for sample in SAMPLES {
        let id = nurt_contracts::services::DocFormatId::new(sample.format).unwrap();
        if registry.get(&id).is_none() {
            continue;
        }
        covered.push(sample.format);
        for (name, result) in sample_results(sample) {
            if let Err(message) = result {
                failures.push(format!("{} / {name}: {message}", sample.format));
            }
        }
    }
    assert_eq!(
        covered.len(),
        SAMPLES.len(),
        "formats of the matrix missing from the registry: {covered:?}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn documents_are_opened_with_the_path_of_their_file() {
    let fixture = Fixture::new();
    fixture.file("/data/app.kvp", b"name = web\n");
    set_on(
        &fixture,
        "/data/app.kvp",
        &[("key", text("name")), ("value", text("api"))],
    )
    .unwrap();
    assert_eq!(fixture.text("/data/app.kvp"), "name = api\n");
    let opened = fixture.opened.lock().unwrap();
    assert!(
        opened
            .iter()
            .all(|path| path.as_deref() == Some(Path::new("/data/app.kvp")))
    );
    assert!(!opened.is_empty());
}
