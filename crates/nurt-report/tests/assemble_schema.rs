// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-report/tests/assemble_schema.rs
// Responsibility:   Assembled reports conform to the published `nurt-report/1` schema.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Assembled reports conform to the published `nurt-report/1` schema.
//!
//! The schema is checked by a validator of the keywords that the published
//! schema uses: `$ref`, `type`, `enum`, `const`, `required`, `properties`,
//! `additionalProperties`, `dependentRequired`, `items`, `minItems`,
//! `minimum`, `minLength`, `allOf`, `anyOf` and `oneOf`. The `pattern`
//! keyword is not evaluated here; the patterns of the schema are covered by
//! `Report::validate`, which every assembled report passes.

// Assertions in tests unwrap results and panic on unexpected values.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use nurt_contracts::config::ReportConfig;
use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::report::{
    Change, ChangeAction, Hunk, Irreversible, IrreversibleEntry, Notable, NotableEntry,
    RecordsPage, Report, ReportStatus,
};
use nurt_contracts::schema;
use nurt_contracts::value::{Path, Record, RecordKind, Size};
use nurt_report::assemble::{ReportBuilder, ReportContext, ScopeDirectory, TargetChange};
use nurt_report::rules::PathContext;
use serde_json::{Map, Value};

struct Schema {
    root: Value,
}

impl Schema {
    fn report() -> Self {
        Self {
            root: serde_json::from_str(schema::REPORT).unwrap(),
        }
    }

    fn errors(&self, instance: &Value) -> Vec<String> {
        let mut errors = Vec::new();
        self.check(&self.root, instance, "", &mut errors);
        errors
    }

    fn resolve(&self, reference: &str) -> &Value {
        let pointer = reference
            .strip_prefix('#')
            .unwrap_or_else(|| panic!("external reference {reference}"));
        self.root
            .pointer(pointer)
            .unwrap_or_else(|| panic!("unresolved reference {reference}"))
    }

    fn passes(&self, schema: &Value, value: &Value) -> bool {
        let mut errors = Vec::new();
        self.check(schema, value, "", &mut errors);
        errors.is_empty()
    }

    fn check(&self, schema: &Value, value: &Value, at: &str, errors: &mut Vec<String>) {
        let rules = match schema {
            Value::Object(rules) => rules,
            Value::Bool(false) => {
                errors.push(format!("{at}: no value is allowed"));
                return;
            }
            _ => return,
        };
        if let Some(reference) = rules.get("$ref").and_then(Value::as_str) {
            self.check(self.resolve(reference), value, at, errors);
        }
        if let Some(types) = rules.get("type")
            && !type_matches(types, value)
        {
            errors.push(format!("{at}: {value} is not of type {types}"));
            return;
        }
        if let Some(options) = rules.get("enum").and_then(Value::as_array)
            && !options.contains(value)
        {
            errors.push(format!("{at}: {value} is not one of {options:?}"));
        }
        if let Some(expected) = rules.get("const")
            && expected != value
        {
            errors.push(format!("{at}: {value} differs from {expected}"));
        }
        self.check_combinators(rules, value, at, errors);
        match value {
            Value::Object(map) => self.check_object(rules, map, at, errors),
            Value::Array(items) => self.check_array(rules, items, at, errors),
            Value::String(text) => check_length(rules, text, at, errors),
            Value::Number(number) => check_minimum(rules, number, at, errors),
            _ => {}
        }
    }

    fn check_combinators(
        &self,
        rules: &Map<String, Value>,
        value: &Value,
        at: &str,
        errors: &mut Vec<String>,
    ) {
        if let Some(all) = rules.get("allOf").and_then(Value::as_array) {
            for schema in all {
                self.check(schema, value, at, errors);
            }
        }
        if let Some(any) = rules.get("anyOf").and_then(Value::as_array)
            && !any.iter().any(|schema| self.passes(schema, value))
        {
            errors.push(format!("{at}: no alternative of anyOf matches"));
        }
        if let Some(one) = rules.get("oneOf").and_then(Value::as_array) {
            let matching = one
                .iter()
                .filter(|schema| self.passes(schema, value))
                .count();
            if matching != 1 {
                errors.push(format!("{at}: {matching} alternatives of oneOf match"));
            }
        }
    }

    fn check_object(
        &self,
        rules: &Map<String, Value>,
        map: &Map<String, Value>,
        at: &str,
        errors: &mut Vec<String>,
    ) {
        for name in array_of_strings(rules.get("required")) {
            if !map.contains_key(name) {
                errors.push(format!("{at}: missing required property {name}"));
            }
        }
        if let Some(Value::Object(dependent)) = rules.get("dependentRequired") {
            for (present, needed) in dependent {
                if map.contains_key(present) {
                    for name in array_of_strings(Some(needed)) {
                        if !map.contains_key(name) {
                            errors.push(format!("{at}: {present} requires {name}"));
                        }
                    }
                }
            }
        }
        let properties = rules.get("properties").and_then(Value::as_object);
        for (name, item) in map {
            let path = format!("{at}/{name}");
            match properties.and_then(|known| known.get(name)) {
                Some(schema) => self.check(schema, item, &path, errors),
                None => match rules.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        errors.push(format!("{at}: property {name} is not allowed"));
                    }
                    Some(schema @ Value::Object(_)) => self.check(schema, item, &path, errors),
                    _ => {}
                },
            }
        }
    }

    fn check_array(
        &self,
        rules: &Map<String, Value>,
        items: &[Value],
        at: &str,
        errors: &mut Vec<String>,
    ) {
        if let Some(minimum) = rules.get("minItems").and_then(Value::as_u64)
            && (items.len() as u64) < minimum
        {
            errors.push(format!("{at}: fewer than {minimum} items"));
        }
        if let Some(schema) = rules.get("items") {
            for (index, item) in items.iter().enumerate() {
                self.check(schema, item, &format!("{at}/{index}"), errors);
            }
        }
    }
}

fn array_of_strings(value: Option<&Value>) -> impl Iterator<Item = &str> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn type_matches(types: &Value, value: &Value) -> bool {
    let matches = |name: &str| match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    };
    match types {
        Value::String(name) => matches(name),
        Value::Array(names) => names.iter().filter_map(Value::as_str).any(matches),
        _ => true,
    }
}

fn check_length(rules: &Map<String, Value>, text: &str, at: &str, errors: &mut Vec<String>) {
    if let Some(minimum) = rules.get("minLength").and_then(Value::as_u64)
        && (text.chars().count() as u64) < minimum
    {
        errors.push(format!("{at}: shorter than {minimum} characters"));
    }
}

fn check_minimum(
    rules: &Map<String, Value>,
    number: &serde_json::Number,
    at: &str,
    errors: &mut Vec<String>,
) {
    if let (Some(minimum), Some(value)) = (
        rules.get("minimum").and_then(Value::as_f64),
        number.as_f64(),
    ) && value < minimum
    {
        errors.push(format!("{at}: {value} is below {minimum}"));
    }
}

fn ctx() -> ReportContext {
    ReportContext::new(
        ReportConfig::default(),
        PathContext::new(Os::Linux).with_home("/home/user"),
    )
    .with_project_roots(["/srv/app"])
}

fn assert_conforms(schema: &Schema, report: &Report) {
    let value = serde_json::to_value(report).unwrap();
    let errors = schema.errors(&value);
    assert!(errors.is_empty(), "{errors:#?}\n{value:#}");
}

fn replace_report(ctx: &ReportContext) -> Report {
    let change = |path: &str, replacements: u64| Change {
        size_before: Some(Size::from_bytes(1127)),
        size_after: Some(Size::from_bytes(1131)),
        hash_before: Some("sha256:1a2b3c4d".to_owned()),
        hash_after: Some("sha256:9f8e7d6c".to_owned()),
        replacements: Some(replacements),
        added: Some(replacements),
        removed: Some(replacements),
        flags: Some(vec!["crlf".to_owned()]),
        diff: Some(vec![Hunk {
            path: Path::new(path),
            old_start: 12,
            old_len: 1,
            new_start: 12,
            new_len: 1,
            lines: vec![
                "-func oldName(x int) {".to_owned(),
                "+func newName(x int) {".to_owned(),
            ],
            ..Hunk::default()
        }]),
        ..Change::new(path, ChangeAction::Modify)
    };
    ReportBuilder::changing(ctx, "replace", ReportStatus::Ok)
        .op(18)
        .ast_hash(format!("sha256:{}", "9d".repeat(32)))
        .changes([
            change("/srv/app/src/a.go", 3),
            change("/srv/app/src/c/d.go", 120),
        ])
        .notes([
            NotableEntry {
                path: Some("/srv/app/src/c/d.go".to_owned()),
                detail: Some("crlf".to_owned()),
                ..NotableEntry::new(Notable::LineEndingPreserved)
            },
            NotableEntry::new(Notable::TrustConfigIgnored),
        ])
        .next_cursor("c_5d2e")
        .build()
        .unwrap()
}

fn bulk_report(ctx: &ReportContext) -> Report {
    let mut changes: Vec<TargetChange> = (0..300)
        .map(|index| {
            TargetChange::file(Change {
                size_before: Some(Size::from_bytes(4096)),
                ..Change::new(
                    format!("/srv/app/build/{}/{index}.o", index % 4).as_str(),
                    ChangeAction::Delete,
                )
            })
        })
        .collect();
    changes.push(TargetChange::directory(Change::new(
        "/srv/app/build",
        ChangeAction::Delete,
    )));
    ReportBuilder::changing(ctx, "remove", ReportStatus::Ok)
        .op(19)
        .changes(changes)
        .scope_directory(ScopeDirectory::new("/srv/app/build", 300))
        .list_limit(50)
        .next_cursor("c_19aa")
        .build()
        .unwrap()
}

fn outside_report(ctx: &ReportContext) -> Report {
    ReportBuilder::changing(ctx, "write", ReportStatus::Ok)
        .op(30)
        .changes([
            TargetChange::file(Change {
                size_before: Some(Size::from_bytes(4096)),
                size_after: Some(Size::ZERO),
                ..Change::new("/etc/app.conf", ChangeAction::Modify)
            })
            .replacing_existing(),
            TargetChange::file(Change {
                size_before: Some(Size::from_bytes(8192)),
                size_after: Some(Size::from_bytes(100)),
                irreversible: Some(Irreversible::NoSpaceForCopy),
                ..Change::new("/home/user/.bashrc", ChangeAction::Modify)
            }),
            TargetChange::file(Change {
                from: Some(Path::new("/srv/app/old.txt")),
                ..Change::new("/srv/app/new.txt", ChangeAction::Rename)
            }),
        ])
        .build()
        .unwrap()
}

fn other_reports(ctx: &ReportContext) -> Vec<Report> {
    vec![
        ReportBuilder::changing(ctx, "replace", ReportStatus::Planned)
            .op(7)
            .change(Change {
                replacements: Some(3),
                ..Change::new("/srv/app/src/a.go", ChangeAction::Modify)
            })
            .build()
            .unwrap(),
        ReportBuilder::changing(ctx, "remove", ReportStatus::Ok)
            .build()
            .unwrap(),
        ReportBuilder::changing(ctx, "remove", ReportStatus::Failed)
            .error(Error::new(
                ErrorCode::Assert,
                "assert paths: 12 does not hold",
            ))
            .build()
            .unwrap(),
        ReportBuilder::changing(ctx, "run", ReportStatus::Running)
            .job(Record::new(RecordKind::Job))
            .build()
            .unwrap(),
        ReportBuilder::reading(ctx, "find", ReportStatus::Ok)
            .records(RecordsPage {
                record_type: RecordKind::ALL[0],
                matched: Some(148),
                header: None,
                rows: vec![Record::new(RecordKind::ALL[0])],
            })
            .next_cursor("c_91fe")
            .build()
            .unwrap(),
        ReportBuilder::changing(ctx, "restart", ReportStatus::Ok)
            .op(27)
            .change(Change {
                before: Some("running".to_owned()),
                after: Some("running".to_owned()),
                ..Change::new("service:nginx", ChangeAction::State)
            })
            .irreversible(IrreversibleEntry {
                path: Some("service:nginx".to_owned()),
                detail: Some("restart".to_owned()),
                ..IrreversibleEntry::new(Irreversible::ServiceChanged)
            })
            .build()
            .unwrap(),
    ]
}

#[test]
fn assembled_reports_conform_to_the_schema() {
    let schema = Schema::report();
    let ctx = ctx();
    let replace = replace_report(&ctx);
    assert!(!replace.notable.is_empty());
    assert_conforms(&schema, &replace);

    let bulk = bulk_report(&ctx);
    assert!(bulk.bulk.is_some());
    assert_conforms(&schema, &bulk);

    let outside = outside_report(&ctx);
    assert!(!outside.irreversible.is_empty());
    assert!(outside.notable.len() >= 5);
    assert_conforms(&schema, &outside);

    for report in other_reports(&ctx) {
        assert_conforms(&schema, &report);
    }
}

#[test]
fn every_derived_code_and_severity_is_in_the_schema() {
    let schema = Schema::report();
    let codes = schema.resolve("#/$defs/notable_code");
    let severities = schema.resolve("#/$defs/severity");
    for code in [
        Notable::OutsideProject,
        Notable::SystemPath,
        Notable::HomeConfig,
        Notable::MassDelete,
        Notable::DeleteTree,
        Notable::OverwroteExisting,
        Notable::FileEmptied,
        Notable::SizeShrunk,
        Notable::ManyPerFile,
        Notable::TargetCountHigh,
        Notable::NothingMatched,
    ] {
        let value = serde_json::to_value(code).unwrap();
        assert!(schema.passes(codes, &value), "{code}");
    }
    for severity in nurt_contracts::report::Severity::ALL {
        let value = serde_json::to_value(severity).unwrap();
        assert!(schema.passes(severities, &value), "{severity}");
    }
}

#[test]
fn validator_rejects_reports_that_break_the_schema() {
    let schema = Schema::report();
    let mut value = serde_json::to_value(replace_report(&ctx())).unwrap();
    value["irreversible"] = Value::Array(Vec::new());
    value["unexpected"] = Value::Bool(true);
    value["targets"]["count"] = Value::from(-1);
    value["notable"][0]["severity"] = Value::from("urgent");
    let errors = schema.errors(&value);
    assert_eq!(errors.len(), 4, "{errors:#?}");
}

#[test]
fn published_golden_reports_pass_the_validator() {
    let schema = Schema::report();
    let directory = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/report");
    let mut checked = 0;
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let errors = schema.errors(&value);
        assert!(errors.is_empty(), "{}: {errors:#?}", path.display());
        checked += 1;
    }
    assert!(checked > 0);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
