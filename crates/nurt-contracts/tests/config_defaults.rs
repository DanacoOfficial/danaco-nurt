// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Built-in defaults, the approved schema and the reference documentation of
//! the configuration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use std::path::{Path, PathBuf};

use nurt_contracts::config::{
    AgeClock, AgeLimit, ByteLimit, Config, ConfigLayer, ConfigLevel, DurationSetting, ENV_VARS,
    Language, OPEN_TABLES, Oversize, OverwriteStrategy, PROJECT_FLOOR_MAX_AGE,
    PROJECT_FLOOR_MAX_OPS, Percent, ServiceMode, SizeLimit, SizeOrPercent, SizeSetting, StepLimit,
    known_keys, resolve,
};
use nurt_contracts::kinds::SyntaxVariant;
use serde_json::{Map, Value};

const SCHEMA_ID: &str = "https://nurt.danaco-group.pl/schema/nurt-config-1.schema.json";

fn spec(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec")
        .join(path)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn language_and_scope_defaults() {
    let config = Config::default();
    assert_eq!(config.syntax.variant, SyntaxVariant::A);
    assert!(config.syntax.tolerant);
    assert_eq!(config.correction.max_read, 8);
    assert_eq!(config.correction.max_effect, 4);
    assert_eq!(
        config.correction.candidates_ttl,
        DurationSetting::from_secs(600)
    );
    assert!(config.scope.base.is_none());
    assert!(config.scope.trusted_projects.is_empty());
    assert_eq!(config.output.language, Language::En);
    assert_eq!(config.output.head, 20);
    assert_eq!(config.output.tail, 40);
}

#[test]
fn journal_limit_defaults() {
    let journal = Config::default().journal;
    assert!(journal.enabled);
    assert_eq!(journal.max_ops, StepLimit::Limited(30));
    assert_eq!(
        journal.max_age,
        AgeLimit::Limited(DurationSetting::from_secs(30 * 60))
    );
    assert_eq!(journal.max_size, SizeLimit::Unlimited);
    assert_eq!(journal.max_size_min, SizeSetting::from_gib(2));
    assert_eq!(journal.max_size_max, SizeSetting::from_gib(50));
    assert_eq!(journal.max_size_total, ByteLimit::Unlimited);
    assert_eq!(journal.max_op_size, ByteLimit::Unlimited);
    assert_eq!(journal.oversize, Oversize::Skip);
    assert_eq!(journal.age_clock, AgeClock::Wall);
}

#[test]
fn journal_storage_defaults() {
    let journal = Config::default().journal;
    assert_eq!(journal.floor.max_ops, None);
    assert_eq!(journal.floor.max_age, None);
    assert_eq!(journal.pin_max_age, DurationSetting::from_secs(24 * 3600));
    assert_eq!(journal.overwrite, OverwriteStrategy::Exchange);
    assert_eq!(journal.history_keep, 1000);
    assert_eq!(journal.dir_name, ".nurt-journal");
    assert!(!journal.volumes.network);
    assert!(!journal.volumes.removable);
    assert!(journal.volume.is_empty());
}

#[test]
fn space_and_run_defaults() {
    let config = Config::default();
    assert_eq!(
        config.space.reserve,
        SizeOrPercent::Percent(Percent::from_percent(15))
    );
    assert_eq!(config.space.reserve_min, SizeSetting::from_gib(5));
    assert_eq!(config.space.poll_ms, 250);
    assert_eq!(config.run.timeout, DurationSetting::from_secs(120));
    assert_eq!(config.run.wait, DurationSetting::from_secs(3));
    assert_eq!(config.run.env_defaults.len(), 8);
    assert_eq!(config.run.env_defaults["TERM"], "dumb");
    assert_eq!(
        config.run.preimage_budget,
        DurationSetting::from_millis(1500)
    );
    assert_eq!(config.run.preimage_copy_max, SizeSetting::from_mib(256));
    assert_eq!(config.run.preimage_exclude.len(), 7);
}

#[test]
fn remaining_section_defaults() {
    let config = Config::default();
    assert_eq!(config.report.mass_threshold, 200);
    assert!((config.report.notable.mass_ratio - 0.8).abs() < f64::EPSILON);
    assert_eq!(config.text.legacy_encodings[0], "windows-1250");
    assert_eq!(config.limits.max_records, 5_000_000);
    assert_eq!(config.limits.threads, None);
    assert_eq!(
        config.code.servers["python"].command,
        ["pyright-langserver", "--stdio"]
    );
    assert_eq!(config.code.server_idle, DurationSetting::from_secs(600));
    assert!(config.programs.catalogs.is_empty());
    assert!(!config.git.trash_refs);
    assert!(!config.ext.native);
    assert_eq!(config.service.mode, ServiceMode::User);
    assert_eq!(config.session.ttl, DurationSetting::from_secs(24 * 3600));
}

#[test]
fn project_floor_defaults_are_ten_steps_and_five_minutes() {
    assert_eq!(PROJECT_FLOOR_MAX_OPS, StepLimit::Limited(10));
    assert_eq!(
        PROJECT_FLOOR_MAX_AGE,
        AgeLimit::Limited(DurationSetting::from_secs(300))
    );
}

#[test]
fn empty_configuration_is_the_default() {
    let config: Config = toml::from_str("").unwrap();
    assert_eq!(config, Config::default());
    let resolved = resolve(&[], false).unwrap();
    assert_eq!(resolved.config, Config::default());
    assert!(resolved.sources.is_empty() && resolved.notes.is_empty());
}

#[test]
fn defaults_survive_json_and_cbor_round_trips() {
    let config = Config::default();
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(serde_json::from_value::<Config>(value).unwrap(), config);
    let mut bytes = Vec::new();
    ciborium::into_writer(&config, &mut bytes).unwrap();
    assert_eq!(
        ciborium::from_reader::<Config, _>(bytes.as_slice()).unwrap(),
        config
    );
}

fn null_keys(prefix: &str, map: &Map<String, Value>, out: &mut Vec<String>) {
    for (name, value) in map {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        match value {
            Value::Null => out.push(key),
            Value::Object(inner) => null_keys(&key, inner, out),
            _ => {}
        }
    }
}

#[test]
fn reference_file_lists_every_key_with_its_default() {
    let text = read(&spec("golden/config/defaults.toml"));
    let values: Map<String, Value> = toml::from_str(&text).unwrap();
    let config: Config = toml::from_str(&text).unwrap();
    assert_eq!(
        config,
        Config::default(),
        "defaults.toml differs from the built-in defaults"
    );

    let layer = ConfigLayer::new(ConfigLevel::User, "defaults.toml", values);
    let resolved = resolve(&[layer], false).unwrap();
    assert!(
        resolved.notes.is_empty(),
        "unexpected notes: {:?}",
        resolved.notes
    );

    let defaults = serde_json::to_value(Config::default()).unwrap();
    let mut unset = Vec::new();
    null_keys("", defaults.as_object().unwrap(), &mut unset);
    let missing: Vec<String> = known_keys()
        .unwrap()
        .into_iter()
        .filter(|key| !unset.contains(key))
        .filter(|key| key != "journal.volume")
        .filter(|key| {
            if OPEN_TABLES.contains(&key.as_str()) {
                !resolved
                    .sources
                    .keys()
                    .any(|set| set.starts_with(&format!("{key}.")))
            } else {
                !resolved.sources.contains_key(key)
            }
        })
        .collect();
    assert!(
        missing.is_empty(),
        "keys missing from defaults.toml: {missing:?}"
    );
}

#[test]
fn reference_documentation_describes_every_key_and_variable() {
    let text = read(&spec("configuration.md"));
    let keys = known_keys().unwrap();
    let missing_keys: Vec<&String> = keys
        .iter()
        .filter(|key| !text.contains(&format!("`{key}`")))
        .collect();
    assert!(
        missing_keys.is_empty(),
        "keys missing from spec/configuration.md: {missing_keys:?}"
    );
    let missing_vars: Vec<&str> = ENV_VARS
        .iter()
        .map(|var| var.name)
        .filter(|name| !text.contains(&format!("`{name}`")))
        .collect();
    assert!(
        missing_vars.is_empty(),
        "variables missing from spec/configuration.md: {missing_vars:?}"
    );
}

#[test]
fn environment_variables_name_known_keys() {
    let keys = known_keys().unwrap();
    for var in ENV_VARS {
        assert!(var.name.starts_with("NURT_"), "{}", var.name);
        assert!(
            keys.iter().any(|key| key == var.key),
            "{} sets unknown key {}",
            var.name,
            var.key
        );
    }
    let mut names: Vec<&str> = ENV_VARS.iter().map(|var| var.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), ENV_VARS.len(), "duplicate variable names");
}

fn generated_schema() -> Value {
    let schema = serde_json::to_value(schemars::schema_for!(Config)).unwrap();
    let Value::Object(mut generated) = schema else {
        panic!("schema is not an object");
    };
    let mut ordered = Map::new();
    if let Some(dialect) = generated.remove("$schema") {
        ordered.insert("$schema".to_owned(), dialect);
    }
    ordered.insert("$id".to_owned(), Value::from(SCHEMA_ID));
    ordered.extend(generated);
    Value::Object(ordered)
}

#[test]
fn schema_matches_the_approved_file() {
    let path = spec("schemas/nurt-config-1.schema.json");
    let generated = generated_schema();
    if std::env::var("NURT_BLESS").as_deref() == Ok("1") {
        let text = serde_json::to_string_pretty(&generated).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
        return;
    }
    let approved: Value = serde_json::from_str(&read(&path)).unwrap();
    assert!(
        approved == generated,
        "spec/schemas/nurt-config-1.schema.json differs from the schema generated from \
         nurt_contracts::config::Config; review the change and regenerate the file with \
         NURT_BLESS=1 cargo test -p nurt-contracts --test config_defaults"
    );
}

#[test]
fn published_schema_constant_is_the_approved_file() {
    let approved: Value = serde_json::from_str(nurt_contracts::schema::CONFIG).unwrap();
    assert_eq!(approved["$id"], SCHEMA_ID);
    assert_eq!(
        approved["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
}
