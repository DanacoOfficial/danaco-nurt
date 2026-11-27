// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Precedence of the seven configuration levels, trust of project files,
//! protection floors and validation, through the configuration loader of the
//! engine.

// Tests stop at the first unexpected value by unwrapping or panicking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use nurt_contracts::config::{AgeLimit, ConfigLevel, ConfigNoteKind, SizeLimit, StepLimit};
use nurt_contracts::error::{AbiStatus, ErrorCode};
use nurt_core::config::{ConfigLoader, ConfigSources, FloorSource, LoadedConfig, toml_table};
use nurt_testkit::host::TestHost;
use tempfile::TempDir;

/// Levels above the built-in defaults, in increasing precedence.
const LEVELS: [ConfigLevel; 7] = ConfigLevel::ALL;

fn golden_dir() -> PathBuf {
    Path::new(
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR")),
    )
    .join("../../spec/golden/config")
}

fn journal_values(level: usize) -> (String, String, String) {
    (
        (30 + 10 * level).to_string(),
        format!("{}m", 30 + 10 * level),
        format!("{}gib", 10 * level),
    )
}

fn journal_table(level: usize) -> String {
    let (ops, age, size) = journal_values(level);
    format!("[journal]\nmax_ops = {ops}\nmax_age = \"{age}\"\nmax_size = \"{size}\"\n")
}

struct Setup {
    temp: TempDir,
    project: PathBuf,
}

impl Setup {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        Self { temp, project }
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::write(&path, text).unwrap();
        path
    }

    fn project_file(&self, text: &str) {
        fs::write(self.project.join(".nurt.toml"), text).unwrap();
    }

    fn trust(&self) -> String {
        format!(
            // A literal TOML string keeps a Windows path as written.
            "[scope]\ntrusted_projects = ['{}']\n",
            self.project.display()
        )
    }

    fn load(&self, sources: ConfigSources, host: &TestHost) -> LoadedConfig {
        let loader = ConfigLoader::new(&sources.with_project_dir(&self.project), host).unwrap();
        loader.load(loader.project_for(None).as_deref()).unwrap()
    }
}

/// Loads a configuration in which the levels up to `top` (index into
/// [`LEVELS`]) set the three retention limits.
fn load_up_to(top: usize) -> LoadedConfig {
    let setup = Setup::new();
    let system = setup.temp.path().join("system.toml");
    let user = setup.temp.path().join("user.toml");
    let host = TestHost::linux();
    let mut sources = ConfigSources::new().with_files(Some(system.clone()), Some(user.clone()));
    if top >= 1 {
        fs::write(&system, format!("{}{}", setup.trust(), journal_table(1))).unwrap();
    }
    if top >= 2 {
        fs::write(&user, journal_table(2)).unwrap();
    }
    if top >= 3 {
        setup.project_file(&journal_table(3));
    }
    if top >= 4 {
        let file = setup.file("config-file.toml", &journal_table(4));
        host.set_var("NURT_CONFIG", file.as_os_str());
    }
    if top >= 5 {
        let (ops, age, size) = journal_values(5);
        host.set_var("NURT_JOURNAL_MAX_OPS", ops);
        host.set_var("NURT_JOURNAL_MAX_AGE", age);
        host.set_var("NURT_JOURNAL_MAX_SIZE", size);
    }
    if top >= 6 {
        sources = sources.with_overrides(toml_table(&journal_table(6), "overrides").unwrap());
    }
    setup.load(sources, &host)
}

#[test]
fn every_level_overrides_the_levels_below() {
    for (top, level) in LEVELS.iter().enumerate() {
        let loaded = load_up_to(top);
        let journal = &loaded.config().journal;
        for key in ["journal.max_ops", "journal.max_age", "journal.max_size"] {
            assert_eq!(loaded.source(key), *level, "{key} at {level}");
        }
        if top == 0 {
            assert_eq!(journal.max_ops, StepLimit::Limited(30));
            continue;
        }
        let (ops, age, size) = journal_values(top);
        assert_eq!(
            journal.max_ops,
            ops.parse::<StepLimit>().unwrap(),
            "{level}"
        );
        assert_eq!(journal.max_age, age.parse::<AgeLimit>().unwrap(), "{level}");
        assert_eq!(
            journal.max_size,
            size.parse::<SizeLimit>().unwrap(),
            "{level}"
        );
        assert!(loaded.journal_limits().raised.is_empty(), "{level}");
    }
}

#[test]
fn embedded_configuration_follows_the_config_file() {
    let setup = Setup::new();
    let host = TestHost::linux();
    let file = setup.file(
        "config-file.toml",
        "[run]\ntimeout = \"1m\"\nwait = \"5s\"\n",
    );
    host.set_var("NURT_CONFIG", file.as_os_str());
    let embedded = toml_table("[run]\ntimeout = \"2m\"\n", "embedded").unwrap();
    let loaded = setup.load(
        ConfigSources::new()
            .with_files(None, None)
            .with_embedded(embedded),
        &host,
    );
    assert_eq!(loaded.config().run.timeout.to_string(), "2m");
    assert_eq!(loaded.config().run.wait.to_string(), "5s");
    assert_eq!(loaded.source("run.timeout"), ConfigLevel::ConfigFile);
}

#[test]
fn a_project_cannot_trust_itself() {
    let setup = Setup::new();
    let golden = fs::read_to_string(golden_dir().join("project-self-trust.toml")).unwrap();
    setup.project_file(&golden);
    let loaded = setup.load(ConfigSources::isolated(), &TestHost::linux());
    let project = loaded.project().unwrap();
    assert!(!project.trusted);
    assert!(loaded.config().scope.trusted_projects.is_empty());
    let kinds = |kind: ConfigNoteKind| {
        let mut keys: Vec<String> = loaded
            .notes()
            .iter()
            .filter(|note| note.kind == kind)
            .map(|note| note.key.clone())
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(
        kinds(ConfigNoteKind::TrustConfigIgnored),
        vec!["scope.trusted_projects", "service.mode"]
    );
    assert_eq!(
        kinds(ConfigNoteKind::ProjectConfigIgnored),
        vec!["state.dir", "syntax.variant"]
    );
    assert_eq!(loaded.source("run.timeout"), ConfigLevel::Project);
    let notable: Vec<String> = loaded
        .notable()
        .iter()
        .map(|entry| entry.code.to_string())
        .collect();
    assert!(notable.contains(&"trust_config_ignored".to_owned()));
}

#[test]
fn journal_keys_of_an_untrusted_project_are_ignored() {
    let setup = Setup::new();
    setup.project_file("[journal]\nmax_ops = 3\n");
    let loaded = setup.load(ConfigSources::isolated(), &TestHost::linux());
    assert_eq!(loaded.config().journal.max_ops, StepLimit::Limited(30));
    assert_eq!(loaded.notes()[0].kind, ConfigNoteKind::JournalConfigIgnored);
}

#[test]
fn trusted_project_values_are_raised_to_the_project_floor() {
    let setup = Setup::new();
    let user = setup.file("user.toml", &setup.trust());
    setup.project_file("[journal]\nmax_ops = 3\nmax_age = \"1m\"\n");
    let loaded = setup.load(
        ConfigSources::new()
            .with_files(None, Some(user))
            .with_environment(false),
        &TestHost::linux(),
    );
    assert!(loaded.project().unwrap().trusted);
    assert_eq!(loaded.source("journal.max_ops"), ConfigLevel::Project);
    let limits = loaded.journal_limits();
    assert_eq!(limits.max_ops, StepLimit::Limited(10));
    assert_eq!(limits.max_age, "5m".parse::<AgeLimit>().unwrap());
    assert!(
        limits
            .raised
            .iter()
            .all(|raise| raise.floor == FloorSource::ProjectFile)
    );
}

#[test]
fn golden_trusted_project_applies_its_journal_keys() {
    let setup = Setup::new();
    let user = setup.file("user.toml", &setup.trust());
    setup.project_file(&fs::read_to_string(golden_dir().join("project-trusted.toml")).unwrap());
    let loaded = setup.load(
        ConfigSources::new()
            .with_files(None, Some(user))
            .with_environment(false),
        &TestHost::linux(),
    );
    assert_eq!(loaded.journal_limits().max_ops, StepLimit::Limited(50));
    assert_eq!(loaded.config().report.mass_threshold, 500);
    assert!(loaded.notes().is_empty());
}

#[test]
fn system_floor_raises_user_limits() {
    let setup = Setup::new();
    let system = golden_dir().join("system-server.toml");
    let user = setup.file("user.toml", "[journal]\nmax_ops = 15\nmax_age = \"2h\"\n");
    let loaded = setup.load(
        ConfigSources::new()
            .with_files(Some(system), Some(user))
            .with_environment(false),
        &TestHost::linux(),
    );
    let limits = loaded.journal_limits();
    assert_eq!(limits.max_ops, StepLimit::Limited(20));
    assert_eq!(limits.raised.len(), 1);
    assert_eq!(limits.raised[0].key, "journal.max_ops");
    assert_eq!(limits.raised[0].floor, FloorSource::Configured);
    assert_eq!(limits.max_age, "2h".parse::<AgeLimit>().unwrap());
}

#[test]
fn every_golden_file_loads() {
    let setup = Setup::new();
    let mut loaded = 0;
    for entry in fs::read_dir(golden_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "toml") {
            continue;
        }
        let sources = ConfigSources::new()
            .with_files(None, Some(path.clone()))
            .with_environment(false);
        let loader = ConfigLoader::new(&sources, &TestHost::linux())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        loader.load(None).unwrap();
        let embedded = toml_table(&fs::read_to_string(&path).unwrap(), "embedded").unwrap();
        setup.load(
            ConfigSources::isolated().with_embedded(embedded),
            &TestHost::linux(),
        );
        loaded += 1;
    }
    assert!(loaded >= 6);
}

#[test]
fn every_invalid_golden_file_names_its_key() {
    let mut rejected = 0;
    for entry in fs::read_dir(golden_dir().join("invalid")).unwrap() {
        let path = entry.unwrap().path();
        let sources = ConfigSources::new()
            .with_files(None, Some(path.clone()))
            .with_environment(false);
        let error = ConfigLoader::new(&sources, &TestHost::linux())
            .and_then(|loader| loader.load(None))
            .expect_err(&path.display().to_string());
        assert_eq!(error.status(), AbiStatus::Arg, "{}", path.display());
        let config = error.config_error().unwrap();
        assert!(config.key.is_some(), "{}", path.display());
        assert_eq!(config.level, Some(ConfigLevel::User));
        assert_eq!(error.to_error().code, ErrorCode::Config);
        rejected += 1;
    }
    assert!(rejected >= 6);
}

#[test]
fn environment_sets_state_directory_and_base() {
    let host = TestHost::linux()
        .with_var("NURT_STATE_DIR", "/var/lib/nurt")
        .with_var("NURT_BASE", "/srv/app");
    let sources = ConfigSources::new().with_files(None, None);
    let loaded = ConfigLoader::new(&sources, &host)
        .unwrap()
        .load(None)
        .unwrap();
    assert_eq!(loaded.config().state.dir.as_deref(), Some("/var/lib/nurt"));
    assert_eq!(loaded.config().scope.base.as_deref(), Some("/srv/app"));
    assert_eq!(loaded.source("state.dir"), ConfigLevel::Environment);
}

#[test]
fn config_file_variable_must_name_an_absolute_existing_file() {
    let sources = ConfigSources::new().with_files(None, None);
    let relative = TestHost::linux().with_var("NURT_CONFIG", "nurt.toml");
    let error = ConfigLoader::new(&sources, &relative).unwrap_err();
    assert_eq!(error.to_error().code, ErrorCode::PathRelative);
    let temp = tempfile::tempdir().unwrap();
    let missing = TestHost::linux().with_var("NURT_CONFIG", temp.path().join("absent.toml"));
    let error = ConfigLoader::new(&sources, &missing).unwrap_err();
    assert_eq!(error.status(), AbiStatus::Io);
}

#[test]
fn invalid_environment_values_are_rejected() {
    let host = TestHost::linux().with_var("NURT_JOURNAL", "maybe");
    let error = ConfigLoader::new(&ConfigSources::new().with_files(None, None), &host).unwrap_err();
    assert_eq!(error.status(), AbiStatus::Arg);
    let host = TestHost::linux().with_var("NURT_JOURNAL_MAX_OPS", "0");
    assert!(ConfigLoader::new(&ConfigSources::new().with_files(None, None), &host).is_err());
}

#[test]
fn unreadable_project_file_is_an_error() {
    let setup = Setup::new();
    setup.project_file("[run\n");
    let loader = ConfigLoader::new(
        &ConfigSources::isolated().with_project_dir(&setup.project),
        &TestHost::linux(),
    )
    .unwrap();
    let error = loader
        .load(loader.project_for(None).as_deref())
        .unwrap_err();
    assert_eq!(
        error.config_error().unwrap().level,
        Some(ConfigLevel::Project)
    );
    assert_eq!(loader.layers().len(), 0);
    assert!(loader.trusted_projects().is_empty());
}
