// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Golden configuration files in `spec/golden/config`.
//!
//! Every valid file starts with a comment, validates against the approved
//! schema, deserializes into `Config`, and survives JSON and CBOR round trips.
//! Every file in `invalid/` names the rejected key in a line
//! `# Rejected key: <key>` and is refused both by deserialization and by
//! `resolve`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::path::{Path, PathBuf};

use nurt_contracts::config::{
    AgeLimit, CatalogSource, Config, ConfigLayer, ConfigLevel, ConfigNoteKind, DurationSetting,
    Language, Percent, ServiceMode, SizeLimit, StepLimit, SyntaxConfig, resolve,
};
use nurt_contracts::kinds::SyntaxVariant;
use serde_json::{Map, Value};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/golden/config")
}

fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    files
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn values(path: &Path) -> Map<String, Value> {
    toml::from_str(&read(path)).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(nurt_contracts::schema::CONFIG).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

fn level_of(path: &Path) -> ConfigLevel {
    let name = path.file_name().unwrap().to_string_lossy();
    if name.starts_with("project-") {
        ConfigLevel::Project
    } else if name.starts_with("system-") {
        ConfigLevel::System
    } else if name.starts_with("embedding-") {
        ConfigLevel::ConfigFile
    } else {
        ConfigLevel::User
    }
}

fn load(name: &str, trusted: bool) -> (Config, Vec<(ConfigNoteKind, String)>) {
    let path = golden_dir().join(name);
    let layer = ConfigLayer::new(level_of(&path), name, values(&path));
    let resolved = resolve(&[layer], trusted).unwrap();
    let notes = resolved
        .notes
        .into_iter()
        .map(|note| (note.kind, note.key))
        .collect();
    (resolved.config, notes)
}

#[test]
fn valid_files_pass_schema_serde_and_round_trips() {
    let files = toml_files(&golden_dir());
    assert!(files.len() >= 5, "golden configuration files are missing");
    let validator = validator();
    for path in files {
        let text = read(&path);
        assert!(
            text.starts_with('#'),
            "{} must start with a comment",
            path.display()
        );
        let instance = Value::Object(values(&path));
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "{}: schema errors {errors:?}",
            path.display()
        );

        let config: Config = toml::from_str(&text).unwrap();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(
            serde_json::from_value::<Config>(json.clone()).unwrap(),
            config
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&config, &mut bytes).unwrap();
        assert_eq!(
            ciborium::from_reader::<Config, _>(bytes.as_slice()).unwrap(),
            config
        );
        assert!(
            validator.iter_errors(&json).next().is_none(),
            "{}: serialized configuration does not match the schema",
            path.display()
        );
    }
}

#[test]
fn invalid_files_are_rejected_with_their_key() {
    let files = toml_files(&golden_dir().join("invalid"));
    assert!(!files.is_empty(), "invalid golden files are missing");
    for path in files {
        let text = read(&path);
        assert!(
            text.starts_with('#'),
            "{} must start with a comment",
            path.display()
        );
        let key = text
            .lines()
            .find_map(|line| line.strip_prefix("# Rejected key: "))
            .unwrap_or_else(|| panic!("{} has no `# Rejected key:` line", path.display()))
            .trim()
            .to_owned();
        assert!(
            toml::from_str::<Config>(&text).is_err(),
            "{} deserializes but must be rejected",
            path.display()
        );
        let layer = ConfigLayer::new(ConfigLevel::User, "invalid.toml", values(&path));
        let error = resolve(&[layer], false).expect_err("resolve must fail");
        assert_eq!(
            error.key.as_deref(),
            Some(key.as_str()),
            "{}",
            path.display()
        );
        assert_eq!(error.level, Some(ConfigLevel::User));
        assert_eq!(error.origin.as_deref(), Some("invalid.toml"));
    }
}

#[test]
fn user_file_overrides_selected_keys() {
    let (config, notes) = load("user-typical.toml", false);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(config.syntax, {
        let mut syntax = SyntaxConfig::default();
        syntax.variant = SyntaxVariant::B;
        syntax
    });
    assert_eq!(config.output.language, Language::Pl);
    assert_eq!(config.journal.max_ops, StepLimit::Limited(100));
    assert_eq!(
        config.journal.max_age,
        AgeLimit::Limited(DurationSetting::from_secs(86_400))
    );
    assert_eq!(config.run.env_defaults["GOFLAGS"], "-mod=mod");
    assert_eq!(config.run.env_defaults["TERM"], "dumb");
    assert_eq!(
        config.code.servers["python"].command,
        ["basedpyright-langserver", "--stdio"]
    );
    assert_eq!(config.code.servers["rust"].command, ["rust-analyzer"]);
}

#[test]
fn system_file_sets_server_policy() {
    let (config, notes) = load("system-server.toml", false);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        config.journal.max_size,
        SizeLimit::OfCapacity(Percent::from_percent(10))
    );
    assert_eq!(config.journal.floor.max_ops, Some(StepLimit::Limited(20)));
    assert_eq!(
        config.journal.floor.max_age,
        Some(AgeLimit::Limited(DurationSetting::from_secs(3600)))
    );
    assert_eq!(config.service.mode, ServiceMode::System);
    assert_eq!(config.scope.trusted_projects, ["/srv/app"]);
    let data = &config.journal.volume["/srv/data"];
    assert_eq!(data.max_ops, Some(StepLimit::Limited(100)));
    assert_eq!(data.enabled, Some(true));
}

#[test]
fn embedding_file_sets_floors_and_catalogs() {
    let (config, notes) = load("embedding-engine.toml", false);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(config.journal.floor.max_ops, Some(StepLimit::Limited(10)));
    assert_eq!(config.programs.catalogs.len(), 2);
    assert!(
        matches!(&config.programs.catalogs[0], CatalogSource::Path(path) if path == "/opt/catalogs/tools.json")
    );
    match &config.programs.catalogs[1] {
        CatalogSource::Entry(entry) => {
            assert_eq!(entry.priority, 10);
            assert!(entry.enabled);
        }
        other => panic!("unexpected catalog {other:?}"),
    }
    assert_eq!(config.limits.threads, Some(8));
}

#[test]
fn trusted_project_file_applies_journal_keys() {
    let (config, notes) = load("project-trusted.toml", true);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(config.scope.base.as_deref(), Some("/home/user/project"));
    assert_eq!(config.run.timeout, DurationSetting::from_secs(600));
    assert_eq!(config.report.mass_threshold, 500);
    assert_eq!(config.journal.max_ops, StepLimit::Limited(50));
}

#[test]
fn untrusted_project_file_keeps_journal_defaults() {
    let (config, notes) = load("project-trusted.toml", false);
    assert_eq!(config.journal.max_ops, StepLimit::Limited(30));
    assert_eq!(config.run.timeout, DurationSetting::from_secs(600));
    assert!(
        notes
            .iter()
            .all(|(kind, _)| *kind == ConfigNoteKind::JournalConfigIgnored)
    );
    assert!(notes.iter().any(|(_, key)| key == "journal.max_ops"));
}

#[test]
fn project_file_cannot_trust_itself() {
    let (config, notes) = load("project-self-trust.toml", true);
    assert!(config.scope.trusted_projects.is_empty());
    assert_eq!(config.service.mode, ServiceMode::User);
    assert_eq!(config.syntax.variant, SyntaxVariant::A);
    assert_eq!(config.run.timeout, DurationSetting::from_secs(300));
    let kind_of = |key: &str| {
        notes
            .iter()
            .find(|(_, noted)| noted == key)
            .map(|(kind, _)| *kind)
            .unwrap_or_else(|| panic!("no note for {key}: {notes:?}"))
    };
    assert_eq!(
        kind_of("scope.trusted_projects"),
        ConfigNoteKind::TrustConfigIgnored
    );
    assert_eq!(kind_of("service.mode"), ConfigNoteKind::TrustConfigIgnored);
    assert_eq!(
        kind_of("syntax.variant"),
        ConfigNoteKind::ProjectConfigIgnored
    );
    assert_eq!(kind_of("state.dir"), ConfigNoteKind::ProjectConfigIgnored);
    assert_eq!(
        ConfigNoteKind::TrustConfigIgnored.notable(),
        Some("trust_config_ignored")
    );
}
