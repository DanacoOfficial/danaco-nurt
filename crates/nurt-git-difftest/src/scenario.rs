// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::scenario
// Responsibility:   Scenario files: format, validation and loading.
// Dependencies:     nurt_contracts, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Scenario files: format, validation and loading.
//!
//! A scenario is a TOML file `scenarios/<group>/<name>.toml` whose `id` is
//! `<group>/<name>`. The format is described in the `README.md` of the crate.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use nurt_contracts::error::ErrorCode;
use nurt_contracts::value::{Record, RecordKind};
use serde::Deserialize;

use crate::error::{DifftestError, Result, io_error, scenario_error};
use crate::normalize::ROOT_PLACEHOLDER;
use crate::output::{OutputSpec, RawOutput};
use crate::time::ScenarioClock;

/// Extension of scenario files.
pub const SCENARIO_EXTENSION: &str = "toml";

/// Forms of the setup directives, for messages.
const DIRECTIVE_USAGE: &str =
    "use [\"@clock\", \"<date>\"] or [\"@write\", \"<path>\", \"<text>\"]";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    id: String,
    description: String,
    corpus: String,
    #[serde(default)]
    setup: Vec<RawStep>,
    #[serde(default)]
    files: BTreeMap<String, RawFile>,
    clock: Option<String>,
    platforms: Option<Vec<String>>,
    transport: Option<String>,
    git: RawGit,
    nurt: RawNurt,
    #[serde(default)]
    compare: RawCompare,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawStep {
    Args(Vec<String>),
    Git(RawGitStep),
    Write(RawWriteStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGitStep {
    git: Vec<String>,
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWriteStep {
    write: String,
    content: Option<String>,
    hex: Option<String>,
    symlink: Option<String>,
    #[serde(default)]
    executable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFile {
    Text(String),
    Spec(RawFileSpec),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFileSpec {
    content: Option<String>,
    hex: Option<String>,
    symlink: Option<String>,
    #[serde(default)]
    executable: bool,
    #[serde(default)]
    delete: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGit {
    args: Option<Vec<String>>,
    steps: Option<Vec<Vec<String>>>,
    #[serde(default)]
    expect_exit: i32,
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNurt {
    command: String,
    #[serde(default)]
    params: toml::Table,
    expect: Option<String>,
    #[serde(default)]
    records: Vec<toml::Table>,
    record_count: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCompare {
    state: Option<Vec<String>>,
    undo: Option<bool>,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    strict: bool,
    output: Option<RawOutput>,
}

/// Operating system on which a scenario runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Platform {
    /// Linux.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl Platform {
    /// All platforms, the default of `platforms`.
    pub const ALL: [Self; 3] = [Self::Linux, Self::Macos, Self::Windows];

    /// Platform of the running process.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    /// Name used in scenario files.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }
}

impl FromStr for Platform {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.as_str() == text)
            .ok_or_else(|| format!("unknown platform `{text}`, use linux, macos or windows"))
    }
}

/// Transport between a repository and its remote in a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Local paths and `file://` addresses.
    File,
    /// Smart HTTP served by `git http-backend`.
    Http,
    /// SSH served by a test server.
    Ssh,
}

impl FromStr for Transport {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text {
            "file" => Ok(Self::File),
            "http" => Ok(Self::Http),
            "ssh" => Ok(Self::Ssh),
            other => Err(format!(
                "unknown transport `{other}`, use file, http or ssh"
            )),
        }
    }
}

/// Content written to a path before the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileContent {
    /// File with these bytes.
    Bytes(Vec<u8>),
    /// Symbolic link with this target.
    Symlink(String),
    /// The entry is removed.
    Delete,
}

/// One write of the `files` table or of a `write` setup step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileWrite {
    /// Path relative to the repository root, with `/` separators.
    pub path: String,
    /// New content.
    pub content: FileContent,
    /// The file gets the executable bit.
    pub executable: bool,
}

/// One preparation step, run before the copies are made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupStep {
    /// A `git` command; arguments without the program name.
    Git {
        /// Arguments.
        args: Vec<String>,
        /// Working directory relative to the repository root.
        cwd: Option<String>,
    },
    /// A file write.
    Write(FileWrite),
    /// The clock of the following `git` steps.
    Clock(ScenarioClock),
}

/// The operation performed with the `git` program on copy A.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitAction {
    /// Commands, each without the program name; all but the last must exit
    /// with 0.
    pub steps: Vec<Vec<String>>,
    /// Expected exit code of the last command.
    pub expect_exit: i32,
    /// Working directory relative to the repository root.
    pub cwd: Option<String>,
}

/// Expected result of the Nurt command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expectation {
    /// The command succeeds.
    Ok,
    /// The command fails with this code.
    Error(ErrorCode),
}

impl fmt::Display for Expectation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok => f.write_str("ok"),
            Self::Error(code) => write!(f, "error:{}", code.as_str()),
        }
    }
}

impl FromStr for Expectation {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if text == "ok" {
            return Ok(Self::Ok);
        }
        let code = text
            .strip_prefix("error:")
            .ok_or_else(|| format!("expect `{text}` is neither `ok` nor `error:<CODE>`"))?;
        ErrorCode::from_str(code)
            .map(Self::Error)
            .map_err(|_| format!("expect names `{code}`, which is not a stable error code"))
    }
}

/// The operation performed with the Nurt command on copy B.
#[derive(Debug, Clone, PartialEq)]
pub struct NurtAction {
    /// Full command name, for example `git.merge`.
    pub command: String,
    /// Parameters as written; `repo` is added by the runner when absent.
    pub params: toml::Table,
    /// Expected result.
    pub expect: Expectation,
    /// Records the command must output; each pattern matches a different
    /// record.
    pub records: Vec<RecordPattern>,
    /// Exact number of output records, when set.
    pub record_count: Option<usize>,
}

/// Expected output record of the Nurt command: its kind and the values of
/// some of its fields, compared in plain JSON form.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordPattern {
    /// Record kind, for example `finding`.
    pub kind: RecordKind,
    /// Expected fields; text values may contain `$ROOT`, the case directory
    /// of the Nurt side.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl RecordPattern {
    /// Returns `true` when `record` has the kind of the pattern and every
    /// field of the pattern with an equal value.
    ///
    /// The directory `case_root` in text values of the record is read as
    /// `$ROOT`.
    #[must_use]
    pub fn matches(&self, record: &Record, case_root: &Path) -> bool {
        if record.kind != self.kind {
            return false;
        }
        let serde_json::Value::Object(actual) = record.to_plain_json() else {
            return false;
        };
        let native = case_root.to_string_lossy().into_owned();
        let forward = native.replace('\\', "/");
        self.fields.iter().all(|(name, expected)| {
            actual
                .get(name)
                .is_some_and(|value| with_root_placeholder(value, &native, &forward) == *expected)
        })
    }
}

impl fmt::Display for RecordPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields = serde_json::Value::Object(self.fields.clone());
        write!(f, "{} {fields}", self.kind.as_str())
    }
}

/// `value` with the case directory replaced by `$ROOT` in every text.
fn with_root_placeholder(
    value: &serde_json::Value,
    native: &str,
    forward: &str,
) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Json::String(text) => Json::String(
            text.replace(native, ROOT_PLACEHOLDER)
                .replace(forward, ROOT_PLACEHOLDER),
        ),
        Json::Array(items) => Json::Array(
            items
                .iter()
                .map(|item| with_root_placeholder(item, native, forward))
                .collect(),
        ),
        Json::Object(map) => Json::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), with_root_placeholder(item, native, forward)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Part of the normalized state of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    /// Index entries with their logical fields, flags and resolve-undo data.
    Index,
    /// Files, directories and links of the working tree.
    Worktree,
    /// Output of `git status --porcelain=v2`.
    Status,
    /// References and `HEAD`.
    Refs,
    /// Reflog entries.
    Reflog,
    /// State files of operations in progress.
    StateFiles,
    /// Parsed repository configuration.
    Config,
    /// Commits and annotated tags created by the operation, all of them or
    /// those reachable from a revision.
    Objects(Option<String>),
    /// Result of `git fsck --strict`.
    Fsck,
}

impl Section {
    /// Sections compared when a scenario does not list any, and after undo.
    pub const STANDARD: [Self; 8] = [
        Self::Index,
        Self::Worktree,
        Self::Status,
        Self::Refs,
        Self::Reflog,
        Self::StateFiles,
        Self::Config,
        Self::Objects(None),
    ];
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index => f.write_str("index"),
            Self::Worktree => f.write_str("worktree"),
            Self::Status => f.write_str("status"),
            Self::Refs => f.write_str("refs"),
            Self::Reflog => f.write_str("reflog"),
            Self::StateFiles => f.write_str("state_files"),
            Self::Config => f.write_str("config"),
            Self::Objects(None) => f.write_str("objects"),
            Self::Objects(Some(rev)) => write!(f, "objects:{rev}"),
            Self::Fsck => f.write_str("fsck"),
        }
    }
}

impl FromStr for Section {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        Ok(match text {
            "index" => Self::Index,
            "worktree" => Self::Worktree,
            "status" => Self::Status,
            "refs" => Self::Refs,
            "reflog" => Self::Reflog,
            "state_files" => Self::StateFiles,
            "config" => Self::Config,
            "objects" => Self::Objects(None),
            "fsck" => Self::Fsck,
            other => match other.strip_prefix("objects:") {
                Some(rev) if !rev.is_empty() => Self::Objects(Some(rev.to_owned())),
                _ => {
                    return Err(format!(
                        "unknown state `{other}`, use index, worktree, status, refs, reflog, \
                         state_files, config, objects, objects:<rev> or fsck"
                    ));
                }
            },
        })
    }
}

/// A section of the state of one repository of the scenario.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateSelector {
    /// Section.
    pub section: Section,
    /// Repository relative to the repository root (`.git/remote.git`,
    /// `../upstream`); `None` for the repository of the scenario.
    pub repo: Option<String>,
}

impl StateSelector {
    /// Selector of a section of the repository of the scenario.
    #[must_use]
    pub fn main(section: Section) -> Self {
        Self {
            section,
            repo: None,
        }
    }
}

impl fmt::Display for StateSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repo {
            None => write!(f, "{}", self.section),
            Some(repo) => write!(f, "{}@{repo}", self.section),
        }
    }
}

impl FromStr for StateSelector {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (section, repo) = match text.split_once('@') {
            Some((section, repo)) => (section, Some(repo)),
            None => (text, None),
        };
        let repo = match repo {
            Some(repo) => Some(check_relative_path(repo, PathScope::Case)?),
            None => None,
        };
        Ok(Self {
            section: section.parse()?,
            repo,
        })
    }
}

/// Explicit exception from the comparison.
///
/// Written as `"<selector>"` for a whole section or `"<selector> <key>"` for
/// one key; a key ending with `*` matches every key with that prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    /// Section the rule applies to.
    pub selector: StateSelector,
    /// Key or key prefix; `None` ignores the whole section.
    pub key: Option<KeyPattern>,
}

/// Key pattern of an [`IgnoreRule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPattern {
    /// The key equals the text.
    Exact(String),
    /// The key starts with the text.
    Prefix(String),
}

impl IgnoreRule {
    /// Returns `true` when the rule removes `key` of `selector` from the
    /// comparison.
    #[must_use]
    pub fn matches(&self, selector: &StateSelector, key: &str) -> bool {
        if &self.selector != selector {
            return false;
        }
        match &self.key {
            None => true,
            Some(KeyPattern::Exact(exact)) => key == exact,
            Some(KeyPattern::Prefix(prefix)) => key.starts_with(prefix.as_str()),
        }
    }
}

impl FromStr for IgnoreRule {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let (selector, key) = match text.split_once(' ') {
            Some((selector, key)) => (selector, Some(key)),
            None => (text, None),
        };
        let key = key.map(|key| match key.strip_suffix('*') {
            Some(prefix) => KeyPattern::Prefix(prefix.to_owned()),
            None => KeyPattern::Exact(key.to_owned()),
        });
        Ok(Self {
            selector: selector.parse()?,
            key,
        })
    }
}

/// What is compared after the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareSpec {
    /// Sections compared between copy A and copy B.
    pub state: Vec<StateSelector>,
    /// Undo the Nurt operation and compare copy B with the initial state.
    pub undo: bool,
    /// Explicit exceptions, justified in the description.
    pub ignore: Vec<IgnoreRule>,
    /// Compare dates, identities and raw object identifiers as well.
    pub strict: bool,
    /// Comparison of the output records with the output of a `git` command.
    pub output: Option<OutputSpec>,
}

/// A validated scenario.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// Identifier `<group>/<name>`, equal to the path of the file.
    pub id: String,
    /// One-paragraph description.
    pub description: String,
    /// Name of the corpus repository, or `empty`.
    pub corpus: String,
    /// Preparation steps.
    pub setup: Vec<SetupStep>,
    /// Writes after the preparation steps.
    pub files: Vec<FileWrite>,
    /// Clock of both sides.
    pub clock: ScenarioClock,
    /// Platforms on which the scenario runs.
    pub platforms: Vec<Platform>,
    /// Transport to remotes.
    pub transport: Transport,
    /// Operation of the `git` program.
    pub git: GitAction,
    /// Operation of the Nurt command.
    pub nurt: NurtAction,
    /// Comparison.
    pub compare: CompareSpec,
    /// File the scenario was read from.
    pub source: PathBuf,
}

impl Scenario {
    /// Parses and validates the text of a scenario file.
    ///
    /// `expected_id` is the identifier derived from the file path; the `id`
    /// field must equal it.
    ///
    /// # Errors
    ///
    /// Returns [`DifftestError::Scenario`] naming the first invalid field.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    /// use nurt_git_difftest::scenario::{Expectation, Scenario};
    ///
    /// let text = r#"
    /// id = "branch/create"
    /// description = "Create a branch at HEAD."
    /// corpus = "linear-small"
    /// [git]
    /// args = ["branch", "topic"]
    /// [nurt]
    /// command = "git.branch"
    /// params = { create = "topic" }
    /// "#;
    /// let scenario = Scenario::parse(text, Path::new("branch/create.toml"), "branch/create")?;
    /// assert_eq!(scenario.nurt.expect, Expectation::Ok);
    /// assert!(scenario.compare.undo);
    /// # Ok::<(), nurt_git_difftest::error::DifftestError>(())
    /// ```
    pub fn parse(text: &str, source: &Path, expected_id: &str) -> Result<Self> {
        let raw: RawScenario = toml::from_str(text)
            .map_err(|error| scenario_error(source, error.to_string().trim_end().to_owned()))?;
        let fail = |message: String| scenario_error(source, message);
        if raw.id != expected_id {
            return Err(fail(format!(
                "id `{}` does not match the file path, set id = \"{expected_id}\"",
                raw.id
            )));
        }
        if raw.description.trim().is_empty() {
            return Err(fail(
                "description is empty, describe the scenario".to_owned(),
            ));
        }
        if raw.corpus.trim().is_empty() {
            return Err(fail(
                "corpus is empty, name a corpus repository or `empty`".to_owned(),
            ));
        }
        let setup = raw
            .setup
            .into_iter()
            .map(convert_step)
            .collect::<Result<Vec<_>, String>>()
            .map_err(&fail)?;
        let files = raw
            .files
            .into_iter()
            .map(|(path, file)| convert_file(&path, file))
            .collect::<Result<Vec<_>, String>>()
            .map_err(&fail)?;
        let clock = ScenarioClock::parse(raw.clock.as_deref().unwrap_or(ScenarioClock::DEFAULT))
            .map_err(&fail)?;
        let platforms = convert_platforms(raw.platforms).map_err(&fail)?;
        let transport = raw
            .transport
            .as_deref()
            .map_or(Ok(Transport::File), str::parse)
            .map_err(&fail)?;
        let git = convert_git(raw.git).map_err(&fail)?;
        let nurt = convert_nurt(raw.nurt).map_err(&fail)?;
        let compare = convert_compare(raw.compare).map_err(&fail)?;
        Ok(Self {
            id: raw.id,
            description: raw.description,
            corpus: raw.corpus,
            setup,
            files,
            clock,
            platforms,
            transport,
            git,
            nurt,
            compare,
            source: source.to_path_buf(),
        })
    }

    /// Reads and validates one scenario file below `root`.
    ///
    /// # Errors
    ///
    /// Returns [`DifftestError::Io`] when the file cannot be read and
    /// [`DifftestError::Scenario`] when it is invalid.
    pub fn load(root: &Path, file: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(file).map_err(io_error("read", file))?;
        let expected_id = scenario_id(root, file)?;
        Self::parse(&text, file, &expected_id)
    }

    /// Returns `true` when the scenario runs on `platform`.
    #[must_use]
    pub fn runs_on(&self, platform: Platform) -> bool {
        self.platforms.contains(&platform)
    }
}

/// Identifier of the scenario stored in `file` below `root`: the relative
/// path without the extension, with `/` separators.
///
/// # Errors
///
/// Returns [`DifftestError::Scenario`] when the file is not below `root` or
/// its path is not valid Unicode.
pub fn scenario_id(root: &Path, file: &Path) -> Result<String> {
    let relative = file
        .strip_prefix(root)
        .map_err(|_| scenario_error(file, "the file is outside the scenario directory"))?;
    let mut parts = Vec::new();
    for component in relative.with_extension("").components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| scenario_error(file, "the file name is not valid Unicode"))?
                    .to_owned(),
            ),
            _ => {
                return Err(scenario_error(
                    file,
                    "the file path is not a plain relative path",
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

/// Finds every scenario file below `root`, in path order.
///
/// # Errors
///
/// Returns [`DifftestError::Io`] when a directory cannot be read.
pub fn find_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(io_error("read directory", &dir))? {
            let path = entry.map_err(io_error("read directory", &dir))?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext == SCENARIO_EXTENSION)
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Loads every scenario below `root`.
///
/// Invalid files do not stop the loading; they are returned as errors next
/// to the valid scenarios. Identifiers are unique by construction.
///
/// # Errors
///
/// Returns [`DifftestError::Io`] when the directory tree cannot be listed.
pub fn load_all(root: &Path) -> Result<(Vec<Scenario>, Vec<DifftestError>)> {
    let mut scenarios = Vec::new();
    let mut errors = Vec::new();
    for file in find_files(root)? {
        match Scenario::load(root, &file) {
            Ok(scenario) => scenarios.push(scenario),
            Err(error) => errors.push(error),
        }
    }
    Ok((scenarios, errors))
}

fn convert_step(step: RawStep) -> Result<SetupStep, String> {
    match step {
        RawStep::Args(args) if args.first().is_some_and(|first| first.starts_with('@')) => {
            convert_directive(&args)
        }
        RawStep::Args(args) => Ok(SetupStep::Git {
            args: check_git_args(args, "setup step")?,
            cwd: None,
        }),
        RawStep::Git(step) => Ok(SetupStep::Git {
            args: check_git_args(step.git, "setup step")?,
            cwd: step.cwd.map(|cwd| check_cwd(&cwd)).transpose()?,
        }),
        RawStep::Write(step) => {
            let spec = RawFileSpec {
                content: step.content,
                hex: step.hex,
                symlink: step.symlink,
                executable: step.executable,
                delete: false,
            };
            let path = check_relative_path(&step.write, PathScope::Case)?;
            Ok(SetupStep::Write(file_write(path, spec)?))
        }
    }
}

/// Converts a setup step written as `["@clock", "<date>"]` or
/// `["@write", "<path>", "<text>"]`.
fn convert_directive(args: &[String]) -> Result<SetupStep, String> {
    match args {
        [name, date] if name == "@clock" => ScenarioClock::parse(date).map(SetupStep::Clock),
        [name, path, text] if name == "@write" => Ok(SetupStep::Write(FileWrite {
            path: check_relative_path(path, PathScope::Case)?,
            content: FileContent::Bytes(text.clone().into_bytes()),
            executable: false,
        })),
        [name, ..] if name == "@clock" || name == "@write" => Err(format!(
            "setup step `{name}` has the wrong number of arguments, {DIRECTIVE_USAGE}"
        )),
        [name, ..] => Err(format!(
            "unknown setup directive `{name}`, {DIRECTIVE_USAGE}"
        )),
        [] => Err("setup step is empty, give the git arguments".to_owned()),
    }
}

fn convert_file(path: &str, file: RawFile) -> Result<FileWrite, String> {
    let path = check_relative_path(path, PathScope::Worktree)?;
    match file {
        RawFile::Text(text) => Ok(FileWrite {
            path,
            content: FileContent::Bytes(text.into_bytes()),
            executable: false,
        }),
        RawFile::Spec(spec) => file_write(path, spec),
    }
}

fn file_write(path: String, spec: RawFileSpec) -> Result<FileWrite, String> {
    let given = [
        spec.content.is_some(),
        spec.hex.is_some(),
        spec.symlink.is_some(),
        spec.delete,
    ];
    if given.iter().filter(|flag| **flag).count() != 1 {
        return Err(format!(
            "file `{path}` needs exactly one of content, hex, symlink or delete"
        ));
    }
    let content = if let Some(text) = spec.content {
        FileContent::Bytes(text.into_bytes())
    } else if let Some(hex_text) = spec.hex {
        FileContent::Bytes(
            hex::decode(hex_text.trim())
                .map_err(|error| format!("file `{path}` has invalid hex content: {error}"))?,
        )
    } else if let Some(target) = spec.symlink {
        FileContent::Symlink(target)
    } else {
        FileContent::Delete
    };
    Ok(FileWrite {
        path,
        content,
        executable: spec.executable,
    })
}

fn convert_platforms(platforms: Option<Vec<String>>) -> Result<Vec<Platform>, String> {
    let Some(names) = platforms else {
        return Ok(Platform::ALL.to_vec());
    };
    let mut parsed = names
        .iter()
        .map(|name| name.parse())
        .collect::<Result<Vec<Platform>, String>>()?;
    parsed.sort();
    parsed.dedup();
    if parsed.is_empty() {
        return Err("platforms is empty, list at least one platform".to_owned());
    }
    Ok(parsed)
}

fn convert_git(git: RawGit) -> Result<GitAction, String> {
    let steps = match (git.args, git.steps) {
        (Some(args), None) => vec![check_git_args(args, "[git] args")?],
        (None, Some(steps)) if !steps.is_empty() => steps
            .into_iter()
            .map(|args| check_git_args(args, "[git] steps"))
            .collect::<Result<_, _>>()?,
        _ => return Err("[git] needs exactly one of args or a non-empty steps".to_owned()),
    };
    let cwd = git.cwd.map(|cwd| check_cwd(&cwd)).transpose()?;
    Ok(GitAction {
        steps,
        expect_exit: git.expect_exit,
        cwd,
    })
}

fn convert_nurt(nurt: RawNurt) -> Result<NurtAction, String> {
    let is_git_command = nurt
        .command
        .strip_prefix("git.")
        .is_some_and(|rest| !rest.is_empty());
    if !is_git_command {
        return Err(format!(
            "[nurt] command `{}` is not a command of the git group",
            nurt.command
        ));
    }
    let expect = nurt.expect.as_deref().unwrap_or("ok").parse()?;
    let records = nurt
        .records
        .into_iter()
        .map(convert_record)
        .collect::<Result<Vec<_>, String>>()?;
    if nurt.record_count.is_some_and(|count| count < records.len()) {
        return Err(
            "[nurt] record_count is smaller than the number of [[nurt.records]], correct one of them"
                .to_owned(),
        );
    }
    Ok(NurtAction {
        command: nurt.command,
        params: nurt.params,
        expect,
        records,
        record_count: nurt.record_count,
    })
}

fn convert_record(mut table: toml::Table) -> Result<RecordPattern, String> {
    let kind = match table.remove("kind") {
        Some(toml::Value::String(name)) => name
            .parse::<RecordKind>()
            .map_err(|_| format!("[[nurt.records]] kind `{name}` is not a record kind"))?,
        _ => return Err("[[nurt.records]] needs kind = \"<record kind>\"".to_owned()),
    };
    let mut fields = serde_json::Map::new();
    for (name, value) in table {
        let json = serde_json::to_value(&value)
            .map_err(|error| format!("[[nurt.records]] field `{name}`: {error}"))?;
        fields.insert(name, json);
    }
    Ok(RecordPattern { kind, fields })
}

fn convert_compare(compare: RawCompare) -> Result<CompareSpec, String> {
    let state = match compare.state {
        None => Section::STANDARD
            .into_iter()
            .map(StateSelector::main)
            .collect(),
        Some(names) if names.is_empty() => {
            return Err("[compare] state is empty, list at least one section".to_owned());
        }
        Some(names) => {
            let mut selectors = names
                .iter()
                .map(|name| name.parse())
                .collect::<Result<Vec<StateSelector>, String>>()?;
            selectors.dedup();
            selectors
        }
    };
    let ignore = compare
        .ignore
        .iter()
        .map(|rule| rule.parse())
        .collect::<Result<Vec<IgnoreRule>, String>>()?;
    let output = compare.output.map(OutputSpec::from_raw).transpose()?;
    Ok(CompareSpec {
        state,
        undo: compare.undo.unwrap_or(true),
        ignore,
        strict: compare.strict,
        output,
    })
}

fn check_git_args(args: Vec<String>, place: &str) -> Result<Vec<String>, String> {
    match args.first().map(String::as_str) {
        None => Err(format!("{place} is empty, give the git arguments")),
        Some("git") => Err(format!(
            "{place} starts with `git`, give the arguments without the program name"
        )),
        Some(_) => Ok(args),
    }
}

/// Where a relative path of a scenario may point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathScope {
    /// Inside the working tree, outside `.git`.
    Worktree,
    /// Anywhere inside the case directory: the repository and its siblings.
    Case,
}

/// Validates a relative path and returns it with `/` separators.
///
/// Leading `..` components may leave the repository for its siblings in the
/// case directory, but only in [`PathScope::Case`] and only one level up.
fn check_relative_path(path: &str, scope: PathScope) -> Result<String, String> {
    if path.is_empty() || path.contains('\\') || path.contains('\0') {
        return Err(format!(
            "path `{path}` must be a non-empty relative path with `/` separators"
        ));
    }
    if path.starts_with('/') || path.as_bytes().get(1) == Some(&b':') {
        return Err(format!(
            "path `{path}` must be relative to the repository root"
        ));
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let ups = parts.iter().take_while(|part| **part == "..").count();
    let rest = &parts[ups..];
    if rest.contains(&"..") || ups > 1 || (ups == 1 && scope == PathScope::Worktree) {
        return Err(format!(
            "path `{path}` leaves the allowed directory, remove the `..` components"
        ));
    }
    if ups == 1 && rest.is_empty() {
        return Err(format!("path `{path}` names the case directory itself"));
    }
    if scope == PathScope::Worktree && rest.first().is_some_and(|first| is_git_dir_name(first)) {
        return Err(format!(
            "path `{path}` points into the repository directory, use a setup step instead"
        ));
    }
    if rest.is_empty() && ups == 0 && scope == PathScope::Worktree {
        return Err(format!("path `{path}` names the repository root"));
    }
    let mut normalized: Vec<&str> = vec![".."; ups];
    normalized.extend_from_slice(rest);
    Ok(if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized.join("/")
    })
}

/// Validates the working directory of a `git` step: a path of the case
/// directory, or `..` for the case directory itself.
fn check_cwd(cwd: &str) -> Result<String, String> {
    if cwd.trim_end_matches('/') == ".." {
        return Ok("..".to_owned());
    }
    check_relative_path(cwd, PathScope::Case)
}

fn is_git_dir_name(name: &str) -> bool {
    name.trim_end_matches(['.', ' '])
        .eq_ignore_ascii_case(".git")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::value::Value;

    const MINIMAL: &str = r#"
id = "merge/minimal"
description = "Minimal scenario."
corpus = "empty"
[git]
args = ["status"]
[nurt]
command = "git.status"
"#;

    fn parse(text: &str) -> Result<Scenario> {
        Scenario::parse(text, Path::new("merge/minimal.toml"), "merge/minimal")
    }

    fn error_of(text: &str) -> String {
        parse(text).unwrap_err().to_string()
    }

    #[test]
    fn applies_defaults_to_a_minimal_scenario() {
        let scenario = parse(MINIMAL).unwrap();
        assert_eq!(scenario.platforms, Platform::ALL.to_vec());
        assert_eq!(scenario.transport, Transport::File);
        assert_eq!(scenario.clock, ScenarioClock::default());
        assert_eq!(scenario.git.expect_exit, 0);
        assert_eq!(scenario.nurt.expect, Expectation::Ok);
        assert!(scenario.nurt.records.is_empty());
        assert_eq!(scenario.nurt.record_count, None);
        assert_eq!(scenario.compare.state.len(), Section::STANDARD.len());
        assert!(scenario.compare.undo);
        assert!(!scenario.compare.strict);
    }

    #[test]
    fn parses_the_documented_example() {
        let text = r#"
id = "merge/minimal"
description = "Merge where one side modifies and the other deletes a file."
corpus = "branchy-small"
setup = [
  ["switch", "-c", "topic"], ["rm", "a.txt"], ["commit", "-m", "remove a"],
  ["switch", "main"], ["commit", "--allow-empty", "-m", "noop"],
  { git = ["init", "--bare", "remote.git"], cwd = ".." },
  { write = "../upstream/a.txt", content = "a\n", executable = true },
]
files = { "b.txt" = "local change\n", "bin/x" = { hex = "00ff" }, "gone.txt" = { delete = true } }
clock = "2026-01-01T12:00:00+01:00"
platforms = ["linux", "macos"]
[git]
args = ["merge", "--no-edit", "topic"]
expect_exit = 1
[nurt]
command = "git.merge"
params = { branch = "topic" }
expect = "error:E_GIT_CONFLICT"
[compare]
state = ["index", "worktree", "refs", "reflog", "state_files", "objects:HEAD", "config", "refs@../remote.git"]
undo = true
ignore = ["worktree build/*", "config"]
"#;
        let scenario = parse(text).unwrap();
        assert_eq!(scenario.setup.len(), 7);
        assert_eq!(
            scenario.setup[5],
            SetupStep::Git {
                args: vec!["init".into(), "--bare".into(), "remote.git".into()],
                cwd: Some("..".into())
            }
        );
        assert_eq!(scenario.files.len(), 3);
        assert_eq!(scenario.files[1].content, FileContent::Bytes(vec![0, 0xff]));
        assert_eq!(scenario.files[2].content, FileContent::Delete);
        assert_eq!(scenario.platforms, vec![Platform::Linux, Platform::Macos]);
        assert_eq!(scenario.git.expect_exit, 1);
        assert_eq!(
            scenario.nurt.expect,
            Expectation::Error(ErrorCode::GitConflict)
        );
        assert_eq!(
            scenario.compare.state[5].section,
            Section::Objects(Some("HEAD".into()))
        );
        assert_eq!(
            scenario.compare.state[7].repo.as_deref(),
            Some("../remote.git")
        );
        let worktree = StateSelector::main(Section::Worktree);
        assert!(scenario.compare.ignore[0].matches(&worktree, "build/out.txt"));
        assert!(!scenario.compare.ignore[0].matches(&worktree, "src/out.txt"));
        assert!(scenario.compare.ignore[1].matches(&StateSelector::main(Section::Config), "x"));
    }

    #[test]
    fn converts_clock_and_write_directives() {
        let text = format!(
            "setup = [[\"@clock\", \"2026-02-01T00:00:00Z\"], [\"@write\", \"a.txt\", \"x\\n\"], [\"add\", \"a.txt\"]]\n{MINIMAL}"
        );
        let scenario = parse(&text).unwrap();
        assert_eq!(
            scenario.setup[0],
            SetupStep::Clock(ScenarioClock::parse("2026-02-01T00:00:00Z").unwrap())
        );
        assert!(matches!(
            &scenario.setup[1],
            SetupStep::Write(write) if write.content == FileContent::Bytes(b"x\n".to_vec())
        ));
        assert!(matches!(&scenario.setup[2], SetupStep::Git { .. }));
        assert!(error_of(&format!("setup = [[\"@clock\"]]\n{MINIMAL}")).contains("wrong number"));
        assert!(
            error_of(&format!("setup = [[\"@sleep\", \"1\"]]\n{MINIMAL}"))
                .contains("unknown setup directive")
        );
        assert!(
            error_of(&format!("setup = [[\"@clock\", \"noon\"]]\n{MINIMAL}")).contains("clock")
        );
    }

    #[test]
    fn parses_expected_records_and_matches_them() {
        let text = format!(
            "{MINIMAL}record_count = 1\n[[nurt.records]]\nkind = \"finding\"\ncheck = \"final_newline\"\npath = \"$ROOT/repo/a.txt\"\n"
        );
        let scenario = parse(&text).unwrap();
        assert_eq!(scenario.nurt.record_count, Some(1));
        let pattern = &scenario.nurt.records[0];
        let root = Path::new("/case/b");
        let record = Record::new(RecordKind::Finding)
            .with("check", Value::from("final_newline"))
            .with("path", Value::Text("/case/b/repo/a.txt".into()))
            .with("line", Value::Int(3));
        assert!(pattern.matches(&record, root));
        let other = record.clone().with("check", Value::from("whitespace"));
        assert!(!pattern.matches(&other, root));
        let status = Record::new(RecordKind::GitStatus).with("check", Value::from("final_newline"));
        assert!(!pattern.matches(&status, root));
        assert!(pattern.to_string().starts_with("finding {"));
        assert!(
            error_of(&format!("{MINIMAL}[[nurt.records]]\ncheck = \"x\"\n")).contains("needs kind")
        );
        assert!(
            error_of(&format!("{MINIMAL}[[nurt.records]]\nkind = \"nope\"\n"))
                .contains("not a record kind")
        );
        let too_few = format!("{MINIMAL}record_count = 0\n[[nurt.records]]\nkind = \"finding\"\n");
        assert!(error_of(&too_few).contains("record_count is smaller"));
    }

    #[test]
    fn parses_output_comparisons_in_both_forms() {
        let preset = parse(&format!("{MINIMAL}[compare]\noutput = \"status\"\n")).unwrap();
        let output = preset.compare.output.unwrap();
        assert_eq!(output.format, crate::output::OutputFormat::Status);
        assert_eq!(output.git[0], "status");
        assert!(output.sort);
        let table = parse(&format!(
            "{MINIMAL}[compare.output]\ngit = [\"log\", \"--format=%H\"]\nrecord = \"{{id}}\"\nkind = \"commit\"\n"
        ))
        .unwrap();
        let output = table.compare.output.unwrap();
        assert_eq!(output.kind, Some(RecordKind::Commit));
        assert!(!output.sort);
        assert!(
            error_of(&format!("{MINIMAL}[compare]\noutput = \"porcelain\"\n"))
                .contains("is unknown")
        );
    }

    #[test]
    fn rejects_an_identifier_that_differs_from_the_path() {
        let text = MINIMAL.replace("merge/minimal", "merge/other");
        assert!(error_of(&text).contains("set id = \"merge/minimal\""));
    }

    #[test]
    fn rejects_unknown_fields() {
        let text = format!("{MINIMAL}\n[compare]\nstat = []\n");
        assert!(error_of(&text).contains("unknown field"));
    }

    #[test]
    fn rejects_both_args_and_steps() {
        let text = MINIMAL.replace(
            "args = [\"status\"]",
            "args = [\"status\"]\nsteps = [[\"status\"]]",
        );
        assert!(error_of(&text).contains("exactly one of args"));
    }

    #[test]
    fn accepts_several_git_steps() {
        let text = MINIMAL.replace(
            "args = [\"status\"]",
            "steps = [[\"add\", \"a\"], [\"commit\", \"-m\", \"x\"]]\ncwd = \"sub\"",
        );
        let scenario = parse(&text).unwrap();
        assert_eq!(scenario.git.steps.len(), 2);
        assert_eq!(scenario.git.cwd.as_deref(), Some("sub"));
    }

    #[test]
    fn rejects_arguments_that_repeat_the_program_name() {
        let text = MINIMAL.replace("[\"status\"]", "[\"git\", \"status\"]");
        assert!(error_of(&text).contains("without the program name"));
    }

    #[test]
    fn rejects_commands_outside_the_git_group() {
        let text = MINIMAL.replace("git.status", "files.list");
        assert!(error_of(&text).contains("not a command of the git group"));
    }

    #[test]
    fn rejects_unknown_error_codes() {
        let text = MINIMAL.replace(
            "command = \"git.status\"",
            "command = \"git.status\"\nexpect = \"error:E_NOPE\"",
        );
        assert!(error_of(&text).contains("not a stable error code"));
        let text = MINIMAL.replace(
            "command = \"git.status\"",
            "command = \"git.status\"\nexpect = \"fails\"",
        );
        assert!(error_of(&text).contains("neither `ok`"));
    }

    #[test]
    fn rejects_unknown_platforms_and_transports() {
        let text = format!("platforms = [\"beos\"]\n{MINIMAL}");
        assert!(error_of(&text).contains("unknown platform"));
        let text = format!("transport = \"ftp\"\n{MINIMAL}");
        assert!(error_of(&text).contains("unknown transport"));
        let text = format!("platforms = []\n{MINIMAL}");
        assert!(error_of(&text).contains("platforms is empty"));
    }

    #[test]
    fn rejects_unknown_state_sections() {
        let text = format!("{MINIMAL}\n[compare]\nstate = [\"blobs\"]\n");
        assert!(error_of(&text).contains("unknown state `blobs`"));
        let text = format!("{MINIMAL}\n[compare]\nstate = []\n");
        assert!(error_of(&text).contains("state is empty"));
    }

    #[test]
    fn rejects_file_paths_outside_the_working_tree() {
        for path in [
            "../x",
            "/etc/x",
            ".git/config",
            ".GIT/x",
            "a/../../x",
            "a\\\\b",
            "C:/x",
        ] {
            let text = format!("files = {{ \"{path}\" = \"x\" }}\n{MINIMAL}");
            assert!(parse(&text).is_err(), "{path}");
        }
    }

    #[test]
    fn rejects_files_with_several_contents() {
        let text = format!("files = {{ \"a\" = {{ content = \"x\", delete = true }} }}\n{MINIMAL}");
        assert!(error_of(&text).contains("exactly one of content"));
        let text = format!("files = {{ \"a\" = {{ hex = \"zz\" }} }}\n{MINIMAL}");
        assert!(error_of(&text).contains("invalid hex"));
    }

    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(
            check_relative_path("./a//b/", PathScope::Worktree).unwrap(),
            "a/b"
        );
        assert_eq!(
            check_relative_path("../up/x", PathScope::Case).unwrap(),
            "../up/x"
        );
        assert_eq!(check_relative_path("sub", PathScope::Case).unwrap(), "sub");
        assert!(check_relative_path("../../x", PathScope::Case).is_err());
        assert!(check_relative_path("..", PathScope::Case).is_err());
        assert!(check_relative_path(".", PathScope::Worktree).is_err());
        assert!(check_relative_path(".git. /x", PathScope::Worktree).is_err());
        assert_eq!(
            check_relative_path(".git/remote.git", PathScope::Case).unwrap(),
            ".git/remote.git"
        );
    }

    #[test]
    fn formats_selectors_and_expectations_back() {
        let selector: StateSelector = "objects:main@../up".parse().unwrap();
        assert_eq!(selector.to_string(), "objects:main@../up");
        assert_eq!(
            Expectation::Error(ErrorCode::GitDirty).to_string(),
            "error:E_GIT_DIRTY"
        );
        assert_eq!(Expectation::Ok.to_string(), "ok");
    }

    #[test]
    fn derives_identifiers_from_paths() {
        let root = Path::new("/s");
        assert_eq!(
            scenario_id(root, Path::new("/s/merge/a.toml")).unwrap(),
            "merge/a"
        );
        assert!(scenario_id(root, Path::new("/t/a.toml")).is_err());
    }

    #[test]
    fn loads_files_and_reports_invalid_ones() {
        let dir = tempfile::tempdir().unwrap();
        let group = dir.path().join("merge");
        std::fs::create_dir_all(&group).unwrap();
        std::fs::write(group.join("minimal.toml"), MINIMAL).unwrap();
        std::fs::write(group.join("broken.toml"), "id = 1").unwrap();
        std::fs::write(group.join("notes.txt"), "ignored").unwrap();
        let (scenarios, errors) = load_all(dir.path()).unwrap();
        assert_eq!(scenarios.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(scenarios[0].runs_on(Platform::current()));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
