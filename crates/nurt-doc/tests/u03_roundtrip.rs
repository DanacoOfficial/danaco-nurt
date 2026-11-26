// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Edits of TOML, INI, property and environment documents checked by the
//! harness: every document opens with its bytes kept, setting the current
//! value changes no byte, and every accepted change stays within the lines
//! of its node and reads back with the expected value.
//!
//! Property, INI and environment files store every value as text, so the
//! operations that insert the number `1` (`InsertKey`, `Append`) would read
//! back as the text `"1"`; for these formats the insertion of text values
//! is checked separately.
#![cfg(feature = "testing")]
// Helpers outside `#[test]` functions fail the test on an unreadable corpus.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::Value;
use nurt_doc::formats::env::EnvFormat;
use nurt_doc::formats::ini::{IniFormat, IniProfile};
use nurt_doc::formats::properties::PropertiesFormat;
use nurt_doc::formats::toml::TomlFormat;
use nurt_doc::model::SetOptions;
use nurt_doc::registry::{DocFormat, DocOptions, Registry};
use nurt_doc::testing::{HarnessOperation, HarnessOptions, Summary, run_corpus};

fn data() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../tests/d5")
}

/// Harness operations for formats whose values are text.
fn text_operations() -> HarnessOptions {
    HarnessOptions {
        operations: vec![
            HarnessOperation::SetSame,
            HarnessOperation::ReplaceScalar,
            HarnessOperation::ChangeType,
            HarnessOperation::Remove,
        ],
        ..HarnessOptions::default()
    }
}

fn run(dir: &str, format: &dyn DocFormat, options: &HarnessOptions) -> Summary {
    let summary = run_corpus(&data().join(dir), format, options).unwrap();
    let violations: Vec<String> = summary
        .violations
        .iter()
        .map(|v| {
            format!(
                "{}: {} {:?} {:?}: {}",
                v.path.display(),
                v.key,
                v.operation,
                v.guard,
                v.detail
            )
        })
        .collect();
    assert!(
        violations.is_empty(),
        "{} violations:\n{}",
        violations.len(),
        violations.join("\n")
    );
    let not_opened: Vec<String> = summary
        .not_opened
        .iter()
        .map(|(path, message)| format!("{}: {message}", path.display()))
        .collect();
    assert!(
        not_opened.is_empty(),
        "not opened:\n{}",
        not_opened.join("\n")
    );
    assert!(summary.accepted > 0, "no change accepted in {dir}");
    summary
}

#[test]
fn toml_test_documents_keep_their_layout_under_every_operation() {
    let summary = run(
        "third_party/toml/toml-test/valid",
        &TomlFormat,
        &HarnessOptions::default(),
    );
    assert!(summary.files > 150, "{} files", summary.files);
}

#[test]
fn own_toml_documents_keep_their_layout_under_every_operation() {
    let summary = run("corpus/toml/own", &TomlFormat, &HarnessOptions::default());
    assert!(summary.files >= 37, "{} files", summary.files);
}

/// INI profiles with the corpus directory of each.
const PROFILES: [(&str, IniProfile); 6] = [
    ("generic", IniProfile::Generic),
    ("systemd", IniProfile::Systemd),
    ("gitconfig", IniProfile::Gitconfig),
    ("desktop", IniProfile::Desktop),
    ("php", IniProfile::Php),
    ("editorconfig", IniProfile::Editorconfig),
];

/// The written files are opened with the profile detected from their
/// content. The generated files are opened with the profile of their
/// directory, as a file name selects it: a removal can take away the
/// section that the content detection relies on, such as `[core]` of a
/// Git configuration, and the harness reopens the result.
#[test]
fn ini_documents_of_every_profile_keep_their_layout() {
    let mut files = 0;
    for (name, profile) in PROFILES {
        let written = run(
            &format!("corpus/ini/{name}"),
            &IniFormat::new(),
            &text_operations(),
        );
        let generated = run(
            &format!("corpus/ini/generated/{name}"),
            &IniFormat::with_profile(profile),
            &text_operations(),
        );
        assert!(generated.files >= 25, "{name}: {} files", generated.files);
        files += written.files + generated.files;
    }
    assert!(files >= 150, "{files} files");
}

#[test]
fn property_files_keep_their_layout_and_encoding() {
    let summary = run("corpus/properties", &PropertiesFormat, &text_operations());
    assert!(summary.files >= 100, "{} files", summary.files);
}

#[test]
fn environment_files_keep_their_layout() {
    let summary = run("corpus/env", &EnvFormat, &text_operations());
    assert!(summary.files >= 100, "{} files", summary.files);
}

/// Whether every line of `before` occurs in `after`, in the same order, so
/// that a line which was changed into a copy of another line is detected.
fn keeps_every_line_in_order(before: &[u8], after: &[u8]) -> bool {
    let mut later = after.split(|byte| *byte == b'\n');
    before
        .split(|byte| *byte == b'\n')
        .all(|line| later.any(|candidate| candidate == line))
}

#[test]
fn a_changed_line_is_not_taken_for_a_kept_one() {
    assert!(keeps_every_line_in_order(b"a\nb\n", b"a\nnew\nb\n"));
    assert!(!keeps_every_line_in_order(b"a\nb\n", b"a\na\nnew\n"));
    assert!(!keeps_every_line_in_order(b"a\nb\n", b"b\na\n"));
}

/// Inserts a text key into every map of each corpus file and checks that
/// the bytes before the change are kept around the new line.
///
/// Each file is detected under the name it has in a project; the
/// environment file is stored as `application.env` because repositories
/// commonly exclude `.env` files from version control.
#[test]
fn text_keys_are_inserted_without_changing_other_lines() {
    let registry = Registry::builtin();
    let cases = [
        ("corpus/ini/generic/app.ini", "app.ini", "database.timeout"),
        (
            "corpus/ini/systemd/web.service",
            "web.service",
            "Service.User",
        ),
        (
            "corpus/ini/gitconfig/.gitconfig",
            ".gitconfig",
            "core.pager",
        ),
        (
            "corpus/properties/application.properties",
            "application.properties",
            "server.ssl.enabled",
        ),
        ("corpus/env/application.env", ".env", "NEW_VARIABLE"),
    ];
    for (file, name, key) in cases {
        let bytes = std::fs::read(data().join(file)).unwrap();
        let id = registry.detect(Some(Path::new(name)), &bytes).unwrap();
        let mut document = registry.open(&id, &bytes, &DocOptions::default()).unwrap();
        let key = KeyPath::parse(key).unwrap();
        let value = Value::Text("nurt value".to_owned());
        let report = document
            .handle
            .set(&key, &value, &SetOptions { create: true })
            .unwrap_or_else(|error| panic!("{file}: {error}"));
        assert_eq!(report.replacements, 1, "{file}");
        let after = document.bytes();
        assert!(
            keeps_every_line_in_order(&bytes, &after),
            "{file}: a line of the original changed"
        );
        let found = document.handle.get(&key).unwrap();
        assert_eq!(
            document.handle.tree()[found[0].node].value,
            Some(value),
            "{file}"
        );
    }
}

#[test]
fn the_registry_detects_every_format_by_name() {
    let registry = Registry::builtin();
    let cases = [
        ("Cargo.toml", "toml"),
        ("Cargo.lock", "toml"),
        ("settings.ini", "ini"),
        ("web.service", "ini"),
        ("repo/.git/config", "ini"),
        (".editorconfig", "ini"),
        ("app.desktop", "ini"),
        ("application.properties", "properties"),
        (".env", "env"),
        (".env.production", "env"),
    ];
    for (name, format) in cases {
        let found = registry.detect(Some(Path::new(name)), b"");
        assert_eq!(found.as_ref().map(|id| id.as_str()), Some(format), "{name}");
    }
    let conf = registry.detect(Some(Path::new("nginx.conf")), b"[main]\nkey = some value\n");
    assert_eq!(conf.as_ref().map(|id| id.as_str()), Some("ini"));
}

/// Refusals with `E_UNSUPPORTED_CONSTRUCT` by construct name, per corpus
/// set, as recorded in `tests/d5/data/u03/roundtrip-stats.json`.
fn unsupported_counts() -> serde_json::Value {
    let mut sets: Vec<(String, Summary)> = vec![
        (
            "toml-test".to_owned(),
            run(
                "third_party/toml/toml-test/valid",
                &TomlFormat,
                &HarnessOptions::default(),
            ),
        ),
        (
            "toml-own".to_owned(),
            run("corpus/toml/own", &TomlFormat, &HarnessOptions::default()),
        ),
    ];
    for (name, profile) in PROFILES {
        sets.push((
            format!("ini-{name}"),
            run(
                &format!("corpus/ini/{name}"),
                &IniFormat::new(),
                &text_operations(),
            ),
        ));
        sets.push((
            format!("ini-generated-{name}"),
            run(
                &format!("corpus/ini/generated/{name}"),
                &IniFormat::with_profile(profile),
                &text_operations(),
            ),
        ));
    }
    sets.push((
        "properties".to_owned(),
        run("corpus/properties", &PropertiesFormat, &text_operations()),
    ));
    sets.push((
        "env".to_owned(),
        run("corpus/env", &EnvFormat, &text_operations()),
    ));
    let counts: serde_json::Map<String, serde_json::Value> = sets
        .into_iter()
        .map(|(name, summary)| (name, serde_json::json!(summary.unsupported)))
        .collect();
    serde_json::Value::Object(counts)
}

#[test]
fn unsupported_constructs_match_the_recorded_counts() {
    let actual = unsupported_counts();
    let path = data().join("data/u03/roundtrip-stats.json");
    let recorded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        recorded,
        actual,
        "recorded counts differ; current counts:\n{}",
        serde_json::to_string_pretty(&actual).unwrap()
    );
}
