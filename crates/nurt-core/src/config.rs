// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::config
// Responsibility:   Configuration of an engine, assembled from its seven levels.
// Dependencies:     nurt_contracts, nurt_testkit, schemars, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Configuration of an engine, assembled from its seven levels.
//!
//! [`ConfigLoader`] reads the sources named by [`ConfigSources`]: the system
//! and user files, the project file `.nurt.toml`, the file named by
//! `NURT_CONFIG` and the configuration of an embedding application, the
//! `NURT_*` variables of the host and the run-time overrides. The levels are
//! merged by [`nurt_contracts::config::resolve`], which also applies the trust
//! rules of project files. The loader then checks that path keys are absolute
//! and raises the journal limits to the protection floors ([`JournalLimits`]).
//!
//! The project file is the nearest `.nurt.toml` in the base directory or one
//! of its parents. Its `journal.*` keys apply only when its directory is
//! listed in `scope.trusted_projects` by a level that may grant trust.
//!
//! # Examples
//!
//! ```
//! use nurt_core::config::{ConfigLoader, ConfigSources, toml_table};
//! use nurt_testkit::host::TestHost;
//!
//! let overrides = toml_table("[journal]\nmax_ops = 50\n", "overrides").unwrap();
//! let sources = ConfigSources::isolated().with_overrides(overrides);
//! let loader = ConfigLoader::new(&sources, &TestHost::linux()).unwrap();
//! let loaded = loader.load(None).unwrap();
//! assert_eq!(loaded.config().journal.max_ops.to_string(), "50");
//! ```

use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nurt_contracts::config::{
    AgeLimit, CONFIG_FILE_VAR, Config, ConfigError, ConfigLayer, ConfigLevel, ConfigNote,
    PROJECT_FILE_NAME, PROJECT_FLOOR_MAX_AGE, PROJECT_FLOOR_MAX_OPS, Resolved, StepLimit,
    env_layer, resolve, trusted_projects,
};
use nurt_contracts::env::{Host, Os};
use nurt_contracts::envelope::is_absolute_path;
use nurt_contracts::report::{Notable, NotableEntry};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use crate::error::EngineError;
use crate::paths::PlatformPaths;

/// Origin recorded for the configuration of an embedding application.
pub const EMBEDDED_ORIGIN: &str = "embedded";

/// Origin recorded for run-time overrides.
pub const RUNTIME_ORIGIN: &str = "runtime";

/// Keys whose value is a single path that must be absolute.
const PATH_KEYS: &[&str] = &[
    "scope.base",
    "state.dir",
    "output.results_dir",
    "ext.dir",
    "programs.index_dir",
];

/// Keys whose value is a list of paths that must be absolute.
const PATH_LIST_KEYS: &[&str] = &["scope.project_roots", "scope.trusted_projects"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileSet {
    Platform,
    Custom {
        system: Option<PathBuf>,
        user: Option<PathBuf>,
    },
    Disabled,
}

/// Sources of the configuration of one engine.
///
/// [`ConfigSources::new`] reads the platform files and the environment of the
/// host; [`ConfigSources::isolated`] reads neither, so that a test or an
/// embedding application sees only the values it passes.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigSources {
    files: FileSet,
    environment: bool,
    embedded: Option<Map<String, Json>>,
    overrides: Map<String, Json>,
    project_dir: Option<PathBuf>,
}

impl Default for ConfigSources {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigSources {
    /// The platform files, `NURT_CONFIG` and the `NURT_*` variables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            files: FileSet::Platform,
            environment: true,
            embedded: None,
            overrides: Map::new(),
            project_dir: None,
        }
    }

    /// No files and no environment; only embedded values and overrides.
    #[must_use]
    pub fn isolated() -> Self {
        Self {
            files: FileSet::Disabled,
            environment: false,
            ..Self::new()
        }
    }

    /// Reads the given system and user files instead of the platform files.
    #[must_use]
    pub fn with_files(mut self, system: Option<PathBuf>, user: Option<PathBuf>) -> Self {
        self.files = FileSet::Custom { system, user };
        self
    }

    /// Reads or ignores `NURT_CONFIG` and the `NURT_*` variables of the host.
    #[must_use]
    pub fn with_environment(mut self, enabled: bool) -> Self {
        self.environment = enabled;
        self
    }

    /// Configuration of the embedding application, applied at the level of
    /// the `NURT_CONFIG` file, after that file.
    #[must_use]
    pub fn with_embedded(mut self, values: Map<String, Json>) -> Self {
        self.embedded = Some(values);
        self
    }

    /// Run-time overrides (`config_overrides`), the strongest level.
    #[must_use]
    pub fn with_overrides(mut self, values: Map<String, Json>) -> Self {
        self.overrides = values;
        self
    }

    /// Directory of the project whose `.nurt.toml` applies, instead of the
    /// nearest one above the base directory.
    #[must_use]
    pub fn with_project_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.project_dir = Some(dir.into());
        self
    }

    /// Whether the platform files and the platform state directory are used.
    #[must_use]
    pub fn uses_platform_files(&self) -> bool {
        self.files == FileSet::Platform
    }
}

/// Parses TOML text into the tables of a configuration layer.
///
/// # Errors
///
/// Returns [`ConfigError`] with `origin` when the text is not valid TOML.
///
/// # Examples
///
/// ```
/// use nurt_core::config::toml_table;
///
/// let table = toml_table("[run]\ntimeout = \"5m\"\n", "inline").unwrap();
/// assert_eq!(table["run"]["timeout"], "5m");
/// assert!(toml_table("[run", "inline").is_err());
/// ```
pub fn toml_table(text: &str, origin: &str) -> Result<Map<String, Json>, ConfigError> {
    let invalid = |message: String| ConfigError {
        key: None,
        level: None,
        origin: Some(origin.to_owned()),
        message,
    };
    let table: toml::Table = toml::from_str(text).map_err(|error| {
        invalid(
            error
                .to_string()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        )
    })?;
    match serde_json::to_value(table) {
        Ok(Json::Object(map)) => Ok(map),
        Ok(_) => Err(invalid("expected a table".to_owned())),
        Err(error) => Err(invalid(error.to_string())),
    }
}

/// Project file found for a base directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectFile {
    /// Directory of the project.
    pub dir: PathBuf,
    /// The file `.nurt.toml`.
    pub file: PathBuf,
    /// Whether the project is listed in `scope.trusted_projects`.
    pub trusted: bool,
}

/// Which floor raised a journal limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FloorSource {
    /// A configured `journal.floor.*` key.
    Configured,
    /// The floor of values set by a project file.
    ProjectFile,
}

/// A journal limit raised to a protection floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FloorRaise {
    /// Dotted key of the limit.
    pub key: String,
    /// Value set by the configuration.
    pub configured: String,
    /// Value after raising.
    pub effective: String,
    /// Floor that raised the value.
    pub floor: FloorSource,
}

/// Effective step and age limits of the journal after the protection floors.
///
/// A floor only raises protection: a lower limit is raised to the floor and
/// `unlimited` is above every floor. Values set by a project file are raised
/// at least to [`PROJECT_FLOOR_MAX_OPS`] and [`PROJECT_FLOOR_MAX_AGE`]. Limits
/// of single volumes (`journal.volume.*`) are raised by the journal itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JournalLimits {
    /// Effective `journal.max_ops`.
    pub max_ops: StepLimit,
    /// Effective `journal.max_age`.
    pub max_age: AgeLimit,
    /// Limits that were raised, for `journal.status`.
    pub raised: Vec<FloorRaise>,
}

impl JournalLimits {
    /// Limits of a merged configuration.
    #[must_use]
    pub fn from_resolved(resolved: &Resolved) -> Self {
        let journal = &resolved.config.journal;
        let from_project = |key: &str| resolved.source(key) == ConfigLevel::Project;
        let mut raised = Vec::new();
        let ops_floor = stronger(
            journal
                .floor
                .max_ops
                .map(|floor| (floor, FloorSource::Configured)),
            from_project("journal.max_ops")
                .then_some((PROJECT_FLOOR_MAX_OPS, FloorSource::ProjectFile)),
            step_rank,
        );
        let age_floor = stronger(
            journal
                .floor
                .max_age
                .map(|floor| (floor, FloorSource::Configured)),
            from_project("journal.max_age")
                .then_some((PROJECT_FLOOR_MAX_AGE, FloorSource::ProjectFile)),
            age_rank,
        );
        Self {
            max_ops: raise(
                journal.max_ops,
                ops_floor,
                step_rank,
                "journal.max_ops",
                &mut raised,
            ),
            max_age: raise(
                journal.max_age,
                age_floor,
                age_rank,
                "journal.max_age",
                &mut raised,
            ),
            raised,
        }
    }
}

fn step_rank(limit: StepLimit) -> u64 {
    match limit {
        StepLimit::Limited(steps) => u64::from(steps),
        StepLimit::Unlimited => u64::MAX,
    }
}

fn age_rank(limit: AgeLimit) -> u64 {
    match limit {
        AgeLimit::Limited(age) => age.as_millis(),
        AgeLimit::Unlimited => u64::MAX,
    }
}

fn stronger<T: Copy>(
    first: Option<(T, FloorSource)>,
    second: Option<(T, FloorSource)>,
    rank: fn(T) -> u64,
) -> Option<(T, FloorSource)> {
    match (first, second) {
        (Some(a), Some(b)) => Some(if rank(b.0) > rank(a.0) { b } else { a }),
        (a, b) => a.or(b),
    }
}

fn raise<T: Copy + Display>(
    value: T,
    floor: Option<(T, FloorSource)>,
    rank: fn(T) -> u64,
    key: &str,
    raised: &mut Vec<FloorRaise>,
) -> T {
    match floor {
        Some((floor, source)) if rank(value) < rank(floor) => {
            raised.push(FloorRaise {
                key: key.to_owned(),
                configured: value.to_string(),
                effective: floor.to_string(),
                floor: source,
            });
            floor
        }
        _ => value,
    }
}

/// Configuration in effect for one project, with the level of every key.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedConfig {
    resolved: Resolved,
    project: Option<ProjectFile>,
    journal: JournalLimits,
}

impl LoadedConfig {
    /// The effective configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.resolved.config
    }

    /// The merged levels, with the level of every key and the notes.
    #[must_use]
    pub fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    /// Level that set the dotted `key`.
    #[must_use]
    pub fn source(&self, key: &str) -> ConfigLevel {
        self.resolved.source(key)
    }

    /// Keys of the layers that were not applied.
    #[must_use]
    pub fn notes(&self) -> &[ConfigNote] {
        &self.resolved.notes
    }

    /// The project file in effect.
    #[must_use]
    pub fn project(&self) -> Option<&ProjectFile> {
        self.project.as_ref()
    }

    /// Effective journal limits after the protection floors.
    #[must_use]
    pub fn journal_limits(&self) -> &JournalLimits {
        &self.journal
    }

    /// Notable effects reporting keys that were not applied, for example
    /// `trust_config_ignored`; keys unknown to the format are not reported
    /// here.
    #[must_use]
    pub fn notable(&self) -> Vec<NotableEntry> {
        self.notes()
            .iter()
            .filter_map(|note| {
                let code: Notable = note.kind.notable()?.parse().ok()?;
                let mut entry = NotableEntry::new(code);
                entry.detail = Some(format!("{} in {}", note.key, note.origin));
                Some(entry)
            })
            .collect()
    }
}

/// Reader of the configuration levels of one engine.
///
/// The loader reads every level except the project file once, when the engine
/// opens. [`ConfigLoader::load`] adds the project file of a base directory, so
/// that requests for different projects use their own `.nurt.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLoader {
    layers: Vec<ConfigLayer>,
    trusted: Vec<String>,
    base: Option<String>,
    project_dir: Option<PathBuf>,
    case_insensitive: bool,
    state_dir: Option<PathBuf>,
}

impl ConfigLoader {
    /// Reads the levels named by `sources`, using `host` for the locations of
    /// the platform files and for the environment.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when a file cannot be read or parsed, when
    /// `NURT_CONFIG` names a relative or missing file, or when a value does
    /// not match its key.
    pub fn new(sources: &ConfigSources, host: &dyn Host) -> Result<Self, EngineError> {
        let paths = PlatformPaths::for_host(host);
        let (system, user) = match &sources.files {
            FileSet::Platform => (paths.system_file.clone(), paths.user_file.clone()),
            FileSet::Custom { system, user } => (system.clone(), user.clone()),
            FileSet::Disabled => (None, None),
        };
        let mut layers = Vec::new();
        push_file(&mut layers, ConfigLevel::System, system.as_deref(), false)?;
        push_file(&mut layers, ConfigLevel::User, user.as_deref(), false)?;
        if sources.environment
            && let Some(file) = config_file_var(host)?
        {
            push_file(&mut layers, ConfigLevel::ConfigFile, Some(&file), true)?;
        }
        if let Some(values) = &sources.embedded {
            layers.push(ConfigLayer::new(
                ConfigLevel::ConfigFile,
                EMBEDDED_ORIGIN,
                values.clone(),
            ));
        }
        if sources.environment {
            layers.push(environment_layer(host)?);
        }
        if !sources.overrides.is_empty() {
            layers.push(ConfigLayer::new(
                ConfigLevel::Runtime,
                RUNTIME_ORIGIN,
                sources.overrides.clone(),
            ));
        }
        let base = resolve(&layers, false)
            .map_err(EngineError::config)?
            .config
            .scope
            .base;
        let trusted = trusted_projects(&layers).map_err(EngineError::config)?;
        Ok(Self {
            layers,
            trusted,
            base,
            project_dir: sources.project_dir.clone(),
            case_insensitive: host.info().os == Os::Windows,
            state_dir: sources
                .uses_platform_files()
                .then_some(paths.state_dir)
                .flatten(),
        })
    }

    /// Layers read when the engine opened, without the project file.
    #[must_use]
    pub fn layers(&self) -> &[ConfigLayer] {
        &self.layers
    }

    /// Trusted projects declared by the levels that may declare trust.
    #[must_use]
    pub fn trusted_projects(&self) -> &[String] {
        &self.trusted
    }

    /// State directory of the operating system, when the platform files are
    /// used.
    #[must_use]
    pub fn default_state_dir(&self) -> Option<&Path> {
        self.state_dir.as_deref()
    }

    /// The project file for requests based in `base`; without `base`, the
    /// configured `scope.base`.
    ///
    /// With an explicit project directory ([`ConfigSources::with_project_dir`])
    /// only that directory is considered; otherwise the nearest `.nurt.toml`
    /// in the base directory or one of its parents.
    #[must_use]
    pub fn project_for(&self, base: Option<&str>) -> Option<PathBuf> {
        if let Some(dir) = &self.project_dir {
            let file = dir.join(PROJECT_FILE_NAME);
            return file.is_file().then_some(file);
        }
        let start = base.or(self.base.as_deref())?;
        Path::new(start)
            .ancestors()
            .map(|dir| dir.join(PROJECT_FILE_NAME))
            .find(|file| file.is_file())
    }

    /// Merges the layers with the project file `project_file`, when given.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] when the project file cannot be read or
    /// parsed, when a value does not match its key or when a path key is not
    /// absolute.
    pub fn load(&self, project_file: Option<&Path>) -> Result<LoadedConfig, EngineError> {
        let mut layers = self.layers.clone();
        let mut project = None;
        if let Some(file) = project_file {
            let dir = file.parent().unwrap_or(file).to_path_buf();
            let trusted = self.is_trusted(&dir);
            push_file(&mut layers, ConfigLevel::Project, Some(file), true)?;
            project = Some(ProjectFile {
                dir,
                file: file.to_path_buf(),
                trusted,
            });
        }
        let trusted = project.as_ref().is_some_and(|project| project.trusted);
        let resolved = resolve(&layers, trusted).map_err(EngineError::config)?;
        check_paths(&resolved)?;
        let journal = JournalLimits::from_resolved(&resolved);
        Ok(LoadedConfig {
            resolved,
            project,
            journal,
        })
    }

    fn is_trusted(&self, dir: &Path) -> bool {
        let wanted = self.normalize(&dir.to_string_lossy());
        self.trusted
            .iter()
            .any(|trusted| self.normalize(trusted) == wanted)
    }

    fn normalize(&self, path: &str) -> String {
        if self.case_insensitive {
            path.replace('\\', "/").trim_end_matches('/').to_lowercase()
        } else {
            path.trim_end_matches('/').to_owned()
        }
    }
}

fn push_file(
    layers: &mut Vec<ConfigLayer>,
    level: ConfigLevel,
    path: Option<&Path>,
    required: bool,
) -> Result<(), EngineError> {
    let Some(path) = path else {
        return Ok(());
    };
    let text = match read_text(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !required => return Ok(()),
        Err(error) => {
            return Err(EngineError::io("read the configuration file", path, &error));
        }
    };
    let origin = path.to_string_lossy().into_owned();
    let values = toml_table(&text, &origin).map_err(|error| {
        EngineError::config(ConfigError {
            level: Some(level),
            ..error
        })
    })?;
    layers.push(ConfigLayer::new(level, origin, values));
    Ok(())
}

/// Text of a configuration file. A file saved by a Windows editor may start
/// with a UTF-8 byte order mark, and a file written by Windows PowerShell 5.1
/// with `>` or `Out-File` is UTF-16 with a byte order mark; both are read.
fn read_text(path: &Path) -> io::Result<String> {
    decode_text(&fs::read(path)?)
}

fn decode_text(bytes: &[u8]) -> io::Result<String> {
    let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidData, reason.to_owned());
    let utf16 = |body: &[u8], unit: fn([u8; 2]) -> u16| {
        if body.len() % 2 == 1 {
            return Err(invalid("the UTF-16 text has an odd number of bytes"));
        }
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| unit(*pair))
            .collect();
        String::from_utf16(&units).map_err(|_| invalid("the text is not valid UTF-16"))
    };
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(body, u16::from_le_bytes);
    }
    if let Some(body) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(body, u16::from_be_bytes);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(body.to_vec()).map_err(|_| invalid("stream did not contain valid UTF-8"))
}

fn config_file_var(host: &dyn Host) -> Result<Option<PathBuf>, EngineError> {
    let Some(value) = host.env_var(CONFIG_FILE_VAR) else {
        return Ok(None);
    };
    let text = value.to_string_lossy().into_owned();
    if text.is_empty() {
        return Ok(None);
    }
    if !is_absolute_path(&text) {
        return Err(EngineError::relative_path(
            CONFIG_FILE_VAR,
            Path::new(&text),
        ));
    }
    Ok(Some(PathBuf::from(text)))
}

fn environment_layer(host: &dyn Host) -> Result<ConfigLayer, EngineError> {
    let separator = if host.info().os == Os::Windows {
        ';'
    } else {
        ':'
    };
    let vars = host
        .env_vars()
        .into_iter()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)));
    env_layer(vars, separator).map_err(EngineError::config)
}

fn check_paths(resolved: &Resolved) -> Result<(), EngineError> {
    let Ok(Json::Object(values)) = serde_json::to_value(&resolved.config) else {
        return Ok(());
    };
    let value_at = |key: &str| {
        let (section, field) = key.split_once('.')?;
        values.get(section)?.get(field).cloned()
    };
    let relative = |key: &str, path: &str| {
        EngineError::config(ConfigError {
            key: Some(key.to_owned()),
            level: Some(resolved.source(key)),
            origin: None,
            message: format!("`{path}` is not an absolute path"),
        })
    };
    for key in PATH_KEYS {
        if let Some(Json::String(path)) = value_at(key)
            && !is_absolute_path(&path)
        {
            return Err(relative(key, &path));
        }
    }
    for key in PATH_LIST_KEYS {
        if let Some(Json::Array(paths)) = value_at(key)
            && let Some(path) = paths
                .iter()
                .filter_map(Json::as_str)
                .find(|path| !is_absolute_path(path))
        {
            return Err(relative(key, path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::{ConfigNoteKind, DurationSetting};
    use nurt_testkit::host::TestHost;

    use super::*;

    fn overrides(text: &str) -> ConfigSources {
        ConfigSources::isolated().with_overrides(toml_table(text, "test").unwrap())
    }

    fn load(sources: &ConfigSources) -> Result<LoadedConfig, EngineError> {
        ConfigLoader::new(sources, &TestHost::linux())?.load(None)
    }

    #[test]
    fn invalid_toml_names_the_origin() {
        let error = toml_table("[journal\nmax_ops = 1", "/etc/nurt/nurt.toml").unwrap_err();
        assert_eq!(error.origin.as_deref(), Some("/etc/nurt/nurt.toml"));
        assert!(!error.message.contains('\n'));
    }

    #[test]
    fn a_file_is_read_as_utf8_with_or_without_a_mark_or_as_utf16_with_a_mark() {
        let text = "[run]\ntimeout = \"5m\"\n";
        let mut utf8_with_mark = vec![0xEF, 0xBB, 0xBF];
        utf8_with_mark.extend_from_slice(text.as_bytes());
        let mut utf16_le = vec![0xFF, 0xFE];
        let mut utf16_be = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            utf16_le.extend_from_slice(&unit.to_le_bytes());
            utf16_be.extend_from_slice(&unit.to_be_bytes());
        }
        let encodings: [&[u8]; 4] = [
            text.as_bytes(),
            utf8_with_mark.as_slice(),
            utf16_le.as_slice(),
            utf16_be.as_slice(),
        ];
        for bytes in encodings {
            assert_eq!(decode_text(bytes).unwrap(), text);
        }
        assert!(decode_text(&[0xFF, 0xFE, 0x41]).is_err());
        assert!(decode_text(&[0xC3, 0x28]).is_err());
    }

    #[test]
    fn isolated_sources_give_the_defaults() {
        let loaded = load(&ConfigSources::isolated()).unwrap();
        assert_eq!(loaded.config(), &Config::default());
        assert!(loaded.notes().is_empty());
        assert!(loaded.project().is_none());
    }

    #[test]
    fn configured_floor_raises_lower_limits() {
        let loaded = load(&overrides(
            "[journal]\nmax_ops = 5\nmax_age = \"1m\"\n[journal.floor]\nmax_ops = 20\nmax_age = \"10m\"\n",
        ))
        .unwrap();
        let limits = loaded.journal_limits();
        assert_eq!(limits.max_ops, StepLimit::Limited(20));
        assert_eq!(
            limits.max_age,
            AgeLimit::Limited(DurationSetting::from_secs(600))
        );
        assert_eq!(limits.raised.len(), 2);
        assert_eq!(limits.raised[0].floor, FloorSource::Configured);
        assert_eq!(limits.raised[0].configured, "5");
        assert_eq!(limits.raised[0].effective, "20");
    }

    #[test]
    fn unlimited_is_above_every_floor() {
        let loaded = load(&overrides(
            "[journal]\nmax_ops = \"unlimited\"\n[journal.floor]\nmax_ops = 100\n",
        ))
        .unwrap();
        assert_eq!(loaded.journal_limits().max_ops, StepLimit::Unlimited);
        assert!(loaded.journal_limits().raised.is_empty());
    }

    #[test]
    fn unlimited_floor_raises_every_limit() {
        let loaded = load(&overrides(
            "[journal]\nmax_ops = 99\n[journal.floor]\nmax_ops = \"unlimited\"\n",
        ))
        .unwrap();
        assert_eq!(loaded.journal_limits().max_ops, StepLimit::Unlimited);
    }

    #[test]
    fn higher_values_are_kept() {
        let loaded = load(&overrides(
            "[journal]\nmax_ops = 50\n[journal.floor]\nmax_ops = 10\n",
        ))
        .unwrap();
        assert_eq!(loaded.journal_limits().max_ops, StepLimit::Limited(50));
        assert!(loaded.journal_limits().raised.is_empty());
    }

    #[test]
    fn relative_path_keys_are_rejected() {
        let error = load(&overrides("[state]\ndir = \"state\"\n")).unwrap_err();
        let config = error.config_error().unwrap();
        assert_eq!(config.key.as_deref(), Some("state.dir"));
        assert_eq!(config.level, Some(ConfigLevel::Runtime));
        let error = load(&overrides("[scope]\nproject_roots = [\"/srv\", \"src\"]\n")).unwrap_err();
        assert!(error.message().contains("`src`"));
    }

    fn project_journal_limits(trusted: bool) -> JournalLimits {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("app");
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(PROJECT_FILE_NAME),
            "[journal]\nmax_ops = 3\nmax_age = \"1m\"\n",
        )
        .unwrap();
        let user = temp.path().join("user.toml");
        let listed = if trusted {
            format!(
                "[scope]\ntrusted_projects = [{:?}]\n",
                project.to_str().unwrap()
            )
        } else {
            String::new()
        };
        fs::write(&user, listed).unwrap();
        let sources = ConfigSources::new()
            .with_files(None, Some(user))
            .with_environment(false);
        let loader = ConfigLoader::new(&sources, &TestHost::linux()).unwrap();
        let loaded = loader.load(Some(&project.join(PROJECT_FILE_NAME))).unwrap();
        assert_eq!(loaded.project().unwrap().trusted, trusted);
        loaded.journal_limits().clone()
    }

    #[test]
    fn trusted_project_journal_limits_are_raised_to_the_project_floor() {
        let limits = project_journal_limits(true);
        assert_eq!(limits.max_ops, PROJECT_FLOOR_MAX_OPS);
        assert_eq!(limits.max_age, PROJECT_FLOOR_MAX_AGE);
        assert_eq!(limits.raised.len(), 2);
        assert!(
            limits
                .raised
                .iter()
                .all(|raise| raise.floor == FloorSource::ProjectFile)
        );
    }

    #[test]
    fn untrusted_project_journal_limits_are_ignored() {
        let limits = project_journal_limits(false);
        assert_eq!(limits.max_ops, Config::default().journal.max_ops);
        assert_eq!(limits.max_age, Config::default().journal.max_age);
        assert!(limits.raised.is_empty());
    }

    #[test]
    fn runtime_trust_is_ignored() {
        let loaded = load(&overrides("[scope]\ntrusted_projects = [\"/srv/app\"]\n")).unwrap();
        assert!(loaded.config().scope.trusted_projects.is_empty());
        assert_eq!(loaded.notes()[0].kind, ConfigNoteKind::TrustConfigIgnored);
        let notable = loaded.notable();
        assert_eq!(notable[0].code, Notable::TrustConfigIgnored);
        assert_eq!(
            notable[0].detail.as_deref(),
            Some("scope.trusted_projects in runtime")
        );
    }

    #[test]
    fn unknown_keys_are_noted_but_not_notable() {
        let loaded = load(&overrides("[journal]\nmax_opz = 3\n")).unwrap();
        assert_eq!(loaded.notes()[0].kind, ConfigNoteKind::UnknownKey);
        assert!(loaded.notable().is_empty());
    }

    #[test]
    fn trust_comparison_follows_the_host() {
        let unix = ConfigLoader {
            layers: Vec::new(),
            trusted: vec!["/srv/app/".to_owned()],
            base: None,
            project_dir: None,
            case_insensitive: false,
            state_dir: None,
        };
        assert!(unix.is_trusted(Path::new("/srv/app")));
        assert!(!unix.is_trusted(Path::new("/srv/App")));
        let windows = ConfigLoader {
            trusted: vec!["C:\\Work\\App".to_owned()],
            case_insensitive: true,
            ..unix
        };
        assert!(windows.is_trusted(Path::new("c:/work/app/")));
    }

    #[test]
    fn project_file_is_found_above_the_base() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("app");
        let nested = project.join("src").join("lib");
        fs::create_dir_all(&nested).unwrap();
        fs::write(project.join(PROJECT_FILE_NAME), "[run]\ntimeout = \"5m\"\n").unwrap();
        let loader = ConfigLoader::new(&ConfigSources::isolated(), &TestHost::linux()).unwrap();
        let found = loader.project_for(nested.to_str()).unwrap();
        assert_eq!(found, project.join(PROJECT_FILE_NAME));
        let loaded = loader.load(Some(&found)).unwrap();
        assert_eq!(loaded.source("run.timeout"), ConfigLevel::Project);
        assert_eq!(loaded.project().unwrap().dir, project);
        assert!(!loaded.project().unwrap().trusted);
        assert!(loader.project_for(None).is_none());
    }

    #[test]
    fn explicit_project_directory_wins() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(PROJECT_FILE_NAME), "").unwrap();
        let sources = ConfigSources::isolated().with_project_dir(temp.path());
        let loader = ConfigLoader::new(&sources, &TestHost::linux()).unwrap();
        assert_eq!(
            loader.project_for(Some("/elsewhere")),
            Some(temp.path().join(PROJECT_FILE_NAME))
        );
    }

    #[test]
    fn platform_sources_name_the_platform_state_directory() {
        let loader = ConfigLoader::new(
            &ConfigSources::new()
                .with_files(None, None)
                .with_environment(false),
            &TestHost::linux(),
        )
        .unwrap();
        assert!(loader.default_state_dir().is_none());
        assert!(ConfigSources::new().uses_platform_files());
        assert!(!ConfigSources::isolated().uses_platform_files());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
