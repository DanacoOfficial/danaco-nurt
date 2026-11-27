// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-contracts/tests/config_precedence.rs
// Responsibility:   Precedence of the seven configuration levels, project trust and the environment layer.
// Dependencies:     ConfigLevel, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Precedence of the seven configuration levels, project trust and the
//! environment layer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests report failures by panicking"
)]

use nurt_contracts::config::{
    AgeLimit, ConfigLayer, ConfigLevel, ConfigNoteKind, DurationSetting, KeyAccess, SizeLimit,
    SizeSetting, StepLimit, allowed_at, env_layer, resolve, trusted_projects,
};
use serde_json::{Map, Value};

fn layer(level: ConfigLevel, text: &str) -> ConfigLayer {
    let values: Map<String, Value> = toml::from_str(text).unwrap();
    ConfigLayer::new(level, format!("{level}.toml"), values)
}

fn retention_layer(level: ConfigLevel, value: u32) -> ConfigLayer {
    layer(
        level,
        &format!(
            "[journal]\nmax_ops = {value}\nmax_age = \"{value}m\"\nmax_size = \"{value}gib\"\n"
        ),
    )
}

fn level_value(level: ConfigLevel) -> u32 {
    let rank = ConfigLevel::ALL
        .iter()
        .position(|candidate| *candidate == level)
        .unwrap();
    31 + u32::try_from(rank).unwrap()
}

fn expect_retention(layers: &[ConfigLayer], trusted: bool, level: ConfigLevel) {
    let resolved = resolve(layers, trusted).unwrap();
    let value = level_value(level);
    let journal = &resolved.config.journal;
    assert_eq!(
        journal.max_ops,
        StepLimit::Limited(value),
        "max_ops, top {level}"
    );
    assert_eq!(
        journal.max_age,
        AgeLimit::Limited(DurationSetting::from_secs(u64::from(value) * 60)),
        "max_age, top {level}"
    );
    assert_eq!(
        journal.max_size,
        SizeLimit::Bytes(SizeSetting::from_gib(u64::from(value))),
        "max_size, top {level}"
    );
    for key in ["journal.max_ops", "journal.max_age", "journal.max_size"] {
        assert_eq!(resolved.source(key), level, "{key}");
    }
}

#[test]
fn each_level_overrides_all_lower_levels_in_three_dimensions() {
    for top in 0..ConfigLevel::ALL.len() {
        let levels = &ConfigLevel::ALL[..=top];
        let layers: Vec<ConfigLayer> = levels
            .iter()
            .map(|level| retention_layer(*level, level_value(*level)))
            .collect();
        expect_retention(&layers, true, levels[top]);
        let reversed: Vec<ConfigLayer> = layers.iter().rev().cloned().collect();
        expect_retention(&reversed, true, levels[top]);
    }
}

#[test]
fn untrusted_project_level_is_skipped_for_journal_keys() {
    let layers: Vec<ConfigLayer> = [ConfigLevel::System, ConfigLevel::User, ConfigLevel::Project]
        .iter()
        .map(|level| retention_layer(*level, level_value(*level)))
        .collect();
    expect_retention(&layers, false, ConfigLevel::User);
    let resolved = resolve(&layers, false).unwrap();
    assert_eq!(resolved.notes.len(), 3);
    for note in &resolved.notes {
        assert_eq!(note.kind, ConfigNoteKind::JournalConfigIgnored);
        assert_eq!(note.level, ConfigLevel::Project);
        assert_eq!(note.origin, "project.toml");
    }
}

#[test]
fn layers_of_the_same_level_apply_in_order() {
    let first = layer(ConfigLevel::User, "[run]\ntimeout = \"1m\"\n");
    let second = layer(ConfigLevel::User, "[run]\ntimeout = \"2m\"\n");
    let resolved = resolve(&[first.clone(), second.clone()], false).unwrap();
    assert_eq!(resolved.config.run.timeout, DurationSetting::from_secs(120));
    let resolved = resolve(&[second, first], false).unwrap();
    assert_eq!(resolved.config.run.timeout, DurationSetting::from_secs(60));
}

#[test]
fn tables_merge_and_arrays_replace() {
    let system = layer(
        ConfigLevel::System,
        "[run.env_defaults]\nGOFLAGS = \"-mod=mod\"\n[scope]\nproject_roots = [\"/srv/a\", \"/srv/b\"]\n",
    );
    let user = layer(
        ConfigLevel::User,
        "[run.env_defaults]\nTERM = \"xterm\"\n[scope]\nproject_roots = [\"/home/user/c\"]\n",
    );
    let resolved = resolve(&[system, user], false).unwrap();
    let env = &resolved.config.run.env_defaults;
    assert_eq!(env["GOFLAGS"], "-mod=mod");
    assert_eq!(env["TERM"], "xterm");
    assert_eq!(env["PYTHONUTF8"], "1");
    assert_eq!(resolved.config.scope.project_roots, ["/home/user/c"]);
    assert_eq!(resolved.source("run.env_defaults.TERM"), ConfigLevel::User);
    assert_eq!(
        resolved.source("run.env_defaults.GOFLAGS"),
        ConfigLevel::System
    );
}

#[test]
fn volume_keys_are_quoted_in_sources() {
    let user = layer(
        ConfigLevel::User,
        "[journal.volume.\"/mnt/data.disk\"]\nmax_ops = 40\n",
    );
    let resolved = resolve(&[user], false).unwrap();
    assert_eq!(
        resolved.config.journal.volume["/mnt/data.disk"].max_ops,
        Some(StepLimit::Limited(40))
    );
    assert_eq!(
        resolved.source("journal.volume.\"/mnt/data.disk\".max_ops"),
        ConfigLevel::User
    );
}

#[test]
fn trust_can_be_declared_only_outside_projects_and_run_time() {
    let text = "[scope]\ntrusted_projects = [\"/home/user/project\"]\n";
    for level in ConfigLevel::ALL {
        let resolved = resolve(&[layer(level, text)], true).unwrap();
        let applied = !resolved.config.scope.trusted_projects.is_empty();
        let expected = !matches!(level, ConfigLevel::Project | ConfigLevel::Runtime);
        assert_eq!(applied, expected, "{level}");
        if !expected {
            assert_eq!(resolved.notes[0].kind, ConfigNoteKind::TrustConfigIgnored);
        }
    }
}

#[test]
fn trusted_projects_ignore_project_and_run_time_layers() {
    let layers = [
        layer(
            ConfigLevel::User,
            "[scope]\ntrusted_projects = [\"/home/user/a\"]\n",
        ),
        layer(
            ConfigLevel::Project,
            "[scope]\ntrusted_projects = [\"/home/user/b\"]\n",
        ),
        layer(
            ConfigLevel::Runtime,
            "[scope]\ntrusted_projects = [\"/home/user/c\"]\n",
        ),
    ];
    assert_eq!(trusted_projects(&layers).unwrap(), ["/home/user/a"]);
}

#[test]
fn project_keys_follow_the_allowed_list() {
    let project = layer(
        ConfigLevel::Project,
        "[scope]\nbase = \"/home/user/project\"\nproject_roots = [\"/home/user/project\"]\n\
         [run]\ntimeout = \"10m\"\n[report]\nmass_threshold = 50\n\
         [syntax]\nvariant = \"c\"\n[service]\nmode = \"system\"\n[code.servers.python]\ncommand = [\"evil\"]\n",
    );
    let resolved = resolve(&[project], true).unwrap();
    let config = &resolved.config;
    assert_eq!(config.scope.base.as_deref(), Some("/home/user/project"));
    assert_eq!(config.run.timeout, DurationSetting::from_secs(600));
    assert_eq!(config.report.mass_threshold, 50);
    assert_eq!(
        config.code.servers["python"].command,
        ["pyright-langserver", "--stdio"]
    );
    let mut kinds: Vec<(String, ConfigNoteKind)> = resolved
        .notes
        .iter()
        .map(|note| (note.key.clone(), note.kind))
        .collect();
    kinds.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        kinds,
        [
            (
                "code.servers.python.command".to_owned(),
                ConfigNoteKind::ProjectConfigIgnored
            ),
            (
                "service.mode".to_owned(),
                ConfigNoteKind::TrustConfigIgnored
            ),
            (
                "syntax.variant".to_owned(),
                ConfigNoteKind::ProjectConfigIgnored
            ),
        ]
    );
}

#[test]
fn allowed_at_table() {
    use ConfigLevel::{Builtin, ConfigFile, Environment, Project, Runtime, System, User};
    let allowed = KeyAccess::Allowed;
    let trust = KeyAccess::Ignored(ConfigNoteKind::TrustConfigIgnored);
    let project = KeyAccess::Ignored(ConfigNoteKind::ProjectConfigIgnored);
    let cases = [
        (Project, "scope.base", allowed),
        (Project, "scope.project_roots", allowed),
        (Project, "scope.trusted_projects", trust),
        (Project, "scope.trust_anything", trust),
        (Project, "service.mode", trust),
        (Project, "report.notable.mass_files", allowed),
        (Project, "run.env_defaults.GOFLAGS", allowed),
        (Project, "journal.max_ops", KeyAccess::TrustedProjectOnly),
        (
            Project,
            "journal.floor.max_ops",
            KeyAccess::TrustedProjectOnly,
        ),
        (Project, "syntax.variant", project),
        (Project, "state.dir", project),
        (Project, "ext.native", project),
        (Runtime, "scope.trusted_projects", trust),
        (Runtime, "service.mode", allowed),
        (Runtime, "journal.max_ops", allowed),
        (Builtin, "scope.trusted_projects", allowed),
        (System, "scope.trusted_projects", allowed),
        (User, "scope.trusted_projects", allowed),
        (ConfigFile, "scope.trusted_projects", allowed),
        (Environment, "scope.trusted_projects", allowed),
        (System, "service.mode", allowed),
    ];
    for (level, key, expected) in cases {
        assert_eq!(allowed_at(level, key), expected, "{level} {key}");
    }
    assert_eq!(KeyAccess::TrustedProjectOnly.permits(true), Ok(()));
    assert_eq!(
        KeyAccess::TrustedProjectOnly.permits(false),
        Err(ConfigNoteKind::JournalConfigIgnored)
    );
}

#[test]
fn unknown_keys_are_noted_and_skipped() {
    let user = layer(
        ConfigLevel::User,
        "[journal]\nmax_opps = 5\n[nonexistent]\nkey = 1\n",
    );
    let resolved = resolve(&[user], false).unwrap();
    let keys: Vec<&str> = resolved
        .notes
        .iter()
        .map(|note| note.key.as_str())
        .collect();
    assert_eq!(keys, ["journal.max_opps", "nonexistent.key"]);
    assert!(
        resolved
            .notes
            .iter()
            .all(|note| note.kind == ConfigNoteKind::UnknownKey)
    );
    assert_eq!(ConfigNoteKind::UnknownKey.notable(), None);
}

#[test]
fn invalid_values_name_key_level_and_origin() {
    let user = layer(ConfigLevel::User, "[journal]\nmax_ops = 0\n");
    let error = resolve(&[user], false).unwrap_err();
    assert_eq!(error.key.as_deref(), Some("journal.max_ops"));
    assert_eq!(error.level, Some(ConfigLevel::User));
    assert_eq!(error.origin.as_deref(), Some("user.toml"));
    assert!(
        error
            .to_string()
            .starts_with("user.toml: journal.max_ops: ")
    );

    let table_expected = layer(ConfigLevel::User, "[journal]\nfloor = 10\n");
    let error = resolve(&[table_expected], false).unwrap_err();
    assert_eq!(error.key.as_deref(), Some("journal.floor"));
}

#[test]
fn environment_layer_converts_values() {
    let vars = [
        ("NURT_JOURNAL_MAX_OPS", "unlimited"),
        ("NURT_JOURNAL", "off"),
        ("NURT_PROJECT_ROOTS", "/home/user/a::/home/user/b"),
        ("NURT_THREADS", "8"),
        ("NURT_SESSION", "session-1"),
        ("NURT_UNKNOWN", "x"),
        ("PATH", "/usr/bin"),
    ];
    let environment = env_layer(vars, ':').unwrap();
    assert_eq!(environment.level, ConfigLevel::Environment);
    let user = layer(
        ConfigLevel::User,
        "[journal]\nmax_ops = 70\nenabled = true\n",
    );
    let resolved = resolve(&[environment, user], false).unwrap();
    let config = &resolved.config;
    assert_eq!(config.journal.max_ops, StepLimit::Unlimited);
    assert!(!config.journal.enabled);
    assert_eq!(config.scope.project_roots, ["/home/user/a", "/home/user/b"]);
    assert_eq!(config.limits.threads, Some(8));
    assert_eq!(config.session.id.as_deref(), Some("session-1"));
    assert_eq!(resolved.source("journal.max_ops"), ConfigLevel::Environment);
}

#[test]
fn environment_layer_rejects_invalid_booleans_and_numbers() {
    let error = env_layer([("NURT_JOURNAL", "maybe")], ':').unwrap_err();
    assert_eq!(error.key.as_deref(), Some("journal.enabled"));
    assert_eq!(error.level, Some(ConfigLevel::Environment));

    let environment = env_layer([("NURT_THREADS", "many")], ':').unwrap();
    let error = resolve(&[environment], false).unwrap_err();
    assert_eq!(error.key.as_deref(), Some("limits.threads"));
}

#[test]
fn environment_list_separator_is_configurable() {
    let environment = env_layer([("NURT_PROJECT_ROOTS", "C:/a;C:/b")], ';').unwrap();
    let resolved = resolve(&[environment], false).unwrap();
    assert_eq!(resolved.config.scope.project_roots, ["C:/a", "C:/b"]);
}

#[test]
fn a_higher_level_cannot_lower_a_floor() {
    let floor = |level: ConfigLevel, ops: &str, age: &str| {
        layer(
            level,
            &format!("[journal.floor]\nmax_ops = {ops}\nmax_age = \"{age}\"\n"),
        )
    };

    let layers = [
        floor(ConfigLevel::System, "200", "1h"),
        floor(ConfigLevel::User, "20", "5m"),
    ];
    let resolved = resolve(&layers, false).unwrap();
    let kept = &resolved.config.journal.floor;
    assert_eq!(kept.max_ops, Some(StepLimit::Limited(200)));
    assert_eq!(
        kept.max_age,
        Some(AgeLimit::Limited(DurationSetting::from_secs(3600)))
    );
    assert_eq!(
        resolved.source("journal.floor.max_ops"),
        ConfigLevel::System
    );
    assert_eq!(
        resolved.source("journal.floor.max_age"),
        ConfigLevel::System
    );

    let layers = [
        floor(ConfigLevel::System, "200", "1h"),
        floor(ConfigLevel::User, "300", "2h"),
    ];
    let resolved = resolve(&layers, false).unwrap();
    let raised = &resolved.config.journal.floor;
    assert_eq!(raised.max_ops, Some(StepLimit::Limited(300)));
    assert_eq!(
        raised.max_age,
        Some(AgeLimit::Limited(DurationSetting::from_secs(7200)))
    );
    assert_eq!(resolved.source("journal.floor.max_ops"), ConfigLevel::User);

    let layers = [
        floor(ConfigLevel::System, "\"unlimited\"", "unlimited"),
        floor(ConfigLevel::Runtime, "5", "10s"),
    ];
    let resolved = resolve(&layers, false).unwrap();
    assert_eq!(
        resolved.config.journal.floor.max_ops,
        Some(StepLimit::Unlimited)
    );
    assert_eq!(
        resolved.config.journal.floor.max_age,
        Some(AgeLimit::Unlimited)
    );

    let layers = [layer(ConfigLevel::User, "[journal.floor]\nmax_ops = 20\n")];
    let resolved = resolve(&layers, false).unwrap();
    assert_eq!(
        resolved.config.journal.floor.max_ops,
        Some(StepLimit::Limited(20))
    );
}

#[test]
fn unknown_fields_of_open_table_entries_are_noted_and_skipped() {
    let user = layer(
        ConfigLevel::User,
        "[journal.volume.\"/mnt\"]\nmax_ops = 40\nbogus = 1\n\
         [code.servers.lua]\ncommand = [\"lua-ls\"]\nbogus = true\n\
         [code.servers.lua.init]\nany = { depth = 2 }\n\
         [run.env_defaults]\nANY_NAME = \"x\"\n",
    );
    let resolved = resolve(&[user], false).unwrap();
    let keys: Vec<&str> = resolved
        .notes
        .iter()
        .map(|note| note.key.as_str())
        .collect();
    assert_eq!(
        keys,
        ["code.servers.lua.bogus", "journal.volume.\"/mnt\".bogus"]
    );
    assert!(
        resolved
            .notes
            .iter()
            .all(|note| note.kind == ConfigNoteKind::UnknownKey)
    );
    let volume = &resolved.config.journal.volume["/mnt"];
    assert_eq!(volume.max_ops, Some(StepLimit::Limited(40)));
    assert_eq!(
        resolved.config.code.servers["lua"].command,
        ["lua-ls".to_owned()]
    );
    assert_eq!(resolved.config.run.env_defaults["ANY_NAME"], "x");
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
