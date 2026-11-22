// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Case format `nurt-conformance-case/1` and loading of a suite.
//!
//! A case is a directory with the file `case.toml` or an entry of a
//! collection file `*.cases.toml` or `*.cases.json`. Calls of a case are
//! given as steps or derived from the files of its directory.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::compare::{Match, sha256};
use crate::protocol::{self, member, method};
use crate::schemas::SchemaSet;
use crate::snapshot::Snapshot;
use crate::{Error, VERSION_FILE};

/// Name of the case file of a case directory.
pub const CASE_FILE: &str = "case.toml";

/// Format name of collection files.
pub const COLLECTION_FORMAT: &str = "nurt-conformance-cases/1";

/// Suffixes of collection files.
pub const COLLECTION_SUFFIXES: [&str; 2] = [".cases.toml", ".cases.json"];

/// Value of `fs_after` that requires the state before the call.
pub const UNCHANGED: &str = "unchanged";

/// Default version of the language of a case.
pub const DEFAULT_LANGUAGE: &str = "1.0";

/// Name of the schema of case files in a [`SchemaSet`].
const CASE_SCHEMA: &str = "conformance-case";

/// Files of a case directory read by the conventions of the kinds.
mod file {
    pub(super) const FS_BEFORE: &str = "fs-before.json";
    pub(super) const FS_AFTER: &str = "fs-after.json";
    pub(super) const PROGRAM: &str = "program.nurt";
    pub(super) const REQUEST: &str = "request.json";
    pub(super) const EXPECT: &str = "expect.json";
    pub(super) const REPORT: &str = "report.json";
    pub(super) const AST: &str = "ast.json";
    pub(super) const AST_HASH: &str = "ast.hash";
    pub(super) const BOUND: &str = "bound.json";
    pub(super) const BOUND_HASH: &str = "bound.hash";
}

/// Kind of a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// Program text against surface and bound trees, printer, `ast_hash`.
    Golden,
    /// Binding of programs.
    Bind,
    /// Error codes of the language.
    Error,
    /// Execution on a file system state.
    Exec,
    /// Journal, undo and redo.
    Journal,
    /// Correction of program text.
    Correction,
    /// Results equal to those of established tools.
    Parity,
    /// Application interface of the language bindings.
    Binding,
}

impl Kind {
    /// Every kind, in the order of the documentation.
    pub const ALL: [Kind; 8] = [
        Self::Golden,
        Self::Bind,
        Self::Error,
        Self::Exec,
        Self::Journal,
        Self::Correction,
        Self::Parity,
        Self::Binding,
    ];

    /// Name of the kind in case files.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Golden => "golden",
            Self::Bind => "bind",
            Self::Error => "error",
            Self::Exec => "exec",
            Self::Journal => "journal",
            Self::Correction => "correction",
            Self::Parity => "parity",
            Self::Binding => "binding",
        }
    }

    /// Returns the kind with the given name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// Class of a case.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// Required for conformance.
    Mandatory,
    /// Run only when the implementation lists the feature.
    Optional(String),
}

impl Class {
    const OPTIONAL_PREFIX: &'static str = "optional:";

    /// Reads the class from its name in case files.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text == "mandatory" {
            return Some(Self::Mandatory);
        }
        let feature = text.strip_prefix(Self::OPTIONAL_PREFIX)?;
        (!feature.is_empty()).then(|| Self::Optional(feature.to_owned()))
    }

    /// Name of the class in case files.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Mandatory => "mandatory".to_owned(),
            Self::Optional(feature) => format!("{}{feature}", Self::OPTIONAL_PREFIX),
        }
    }
}

/// Condition under which a case applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// The operating system: `linux`, `macos`, `windows` or `unix`.
    Os(String),
    /// A feature listed by the implementation.
    Feature(String),
}

impl Condition {
    /// Reads a condition from its form in case files.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(os) = text.strip_prefix("os:") {
            return ["linux", "macos", "windows", "unix"]
                .contains(&os)
                .then(|| Self::Os(os.to_owned()));
        }
        let feature = text.strip_prefix("feature:")?;
        (!feature.is_empty()).then(|| Self::Feature(feature.to_owned()))
    }

    /// Whether the condition holds on this system for an implementation
    /// with the given features.
    #[must_use]
    pub fn holds(&self, features: &BTreeSet<String>) -> bool {
        match self {
            Self::Os(os) => match os.as_str() {
                "unix" => cfg!(unix),
                "linux" => cfg!(target_os = "linux"),
                "macos" => cfg!(target_os = "macos"),
                _ => cfg!(windows),
            },
            Self::Feature(feature) => features.contains(feature),
        }
    }

    /// Form of the condition in case files.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Os(os) => format!("os:{os}"),
            Self::Feature(feature) => format!("feature:{feature}"),
        }
    }
}

/// Expected part of a result.
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    /// JSON pointer into the result.
    pub at: String,
    /// Expected value.
    pub value: Value,
    /// Comparison mode.
    pub matching: Match,
}

/// Expected state of the working directory after a call.
#[derive(Debug, Clone, PartialEq)]
pub enum FsExpectation {
    /// The state before the call.
    Unchanged,
    /// The given snapshot.
    Snapshot(Snapshot),
}

/// Call of a case and its expectations.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Method of `nurt-rpc/1`.
    pub method: String,
    /// Parameters, possibly with placeholders.
    pub params: Value,
    /// Expected parts of the result.
    pub expectations: Vec<Expectation>,
    /// Expected JSON-RPC error code.
    pub expect_error: Option<i64>,
    /// JSON pointer into the result and name of the schema it must satisfy.
    pub validate: Option<(String, String)>,
    /// Expected state of the working directory.
    pub fs_after: Option<FsExpectation>,
}

/// Files of a golden case.
#[derive(Debug, Clone, PartialEq)]
pub struct Golden {
    /// Program text per syntax variant, in the order `a`, `b`, `c`.
    pub texts: Vec<(String, String)>,
    /// Surface tree.
    pub ast: Value,
    /// Recorded `ast_hash` of the surface tree.
    pub ast_hash: Option<String>,
    /// Bound tree.
    pub bound: Option<Value>,
    /// Recorded `ast_hash` of the bound tree.
    pub bound_hash: Option<String>,
}

/// Calls of a case.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Explicit or derived steps.
    Steps(Vec<Step>),
    /// Checks of a golden case.
    Golden(Box<Golden>),
}

/// Conformance case.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    /// Identifier, unique in the suite.
    pub id: String,
    /// Kind of the case.
    pub kind: Kind,
    /// Class of the case.
    pub class: Class,
    /// Version of the language, `MAJOR.MINOR`.
    pub language: String,
    /// Conditions under which the case applies.
    pub requires: Vec<Condition>,
    /// Description of the checked behavior.
    pub summary: String,
    /// Default syntax variant.
    pub variant: String,
    /// Error code covered by the case.
    pub code: Option<String>,
    /// Further pointer patterns of volatile values.
    pub volatile: Vec<String>,
    /// Content of the configuration file.
    pub config: Option<String>,
    /// Initial state of the working directory.
    pub fs_before: Option<Snapshot>,
    /// Calls of the case.
    pub plan: Plan,
    /// File that defines the case.
    pub source: PathBuf,
}

impl Case {
    /// Every expected value of the case, for checks of coverage.
    #[must_use]
    pub fn expected_values(&self) -> Vec<&Value> {
        match &self.plan {
            Plan::Steps(steps) => steps
                .iter()
                .flat_map(|step| step.expectations.iter().map(|e| &e.value))
                .collect(),
            Plan::Golden(golden) => std::iter::once(&golden.ast)
                .chain(golden.bound.as_ref())
                .collect(),
        }
    }

    /// Major version of the language of the case.
    #[must_use]
    pub fn language_major(&self) -> &str {
        major(&self.language)
    }
}

/// Returns the major part of a version `MAJOR.MINOR`.
#[must_use]
pub fn major(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// Loaded suite of cases.
#[derive(Debug, Clone)]
pub struct Suite {
    /// Root directory of the cases.
    pub root: PathBuf,
    /// Version of the suite.
    pub version: String,
    /// SHA-256 digest of the files of the suite.
    pub digest: String,
    /// Cases sorted by identifier.
    pub cases: Vec<Case>,
}

impl Suite {
    /// Loads every case below `root`.
    ///
    /// # Errors
    ///
    /// Returns the first case file that cannot be read or does not follow
    /// the format, a duplicate identifier, or a missing version file.
    pub fn load(root: &Path, schemas: &SchemaSet) -> Result<Self, Error> {
        let version_path = root.join(VERSION_FILE);
        let version = fs::read_to_string(&version_path)
            .map_err(|e| Error::io(&version_path, e))?
            .trim()
            .to_owned();
        if !is_semver(&version) {
            return Err(Error::format(
                &version_path,
                format!("{version:?} is not a version MAJOR.MINOR.PATCH"),
            ));
        }
        let mut files = Vec::new();
        collect_files(root, &mut files)?;
        files.sort();
        let mut loader = Loader {
            schemas,
            cases: Vec::new(),
        };
        let mut digest_input = Vec::new();
        for path in &files {
            let content = fs::read(path).map_err(|e| Error::io(path, e))?;
            let relative = path.strip_prefix(root).unwrap_or(path);
            digest_input.extend(relative.to_string_lossy().replace('\\', "/").as_bytes());
            digest_input.push(0);
            digest_input.extend(&content);
            digest_input.push(0);
            loader.load_file(path)?;
        }
        let mut cases = loader.cases;
        cases.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(pair) = cases.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(Error::format(
                &pair[1].source,
                format!(
                    "case {:?} is also defined in {}",
                    pair[1].id,
                    pair[0].source.display()
                ),
            ));
        }
        Ok(Self {
            root: root.to_path_buf(),
            version,
            digest: sha256(&digest_input),
            cases,
        })
    }

    /// Returns the cases selected by identifier filters and kinds.
    ///
    /// A filter selects identifiers that contain it, or, when it contains
    /// `*`, identifiers that match it as a pattern. Empty lists select every
    /// case.
    #[must_use]
    pub fn select(&self, filters: &[String], kinds: &[Kind]) -> Vec<&Case> {
        self.cases
            .iter()
            .filter(|case| kinds.is_empty() || kinds.contains(&case.kind))
            .filter(|case| {
                filters.is_empty() || filters.iter().any(|f| matches_filter(f, &case.id))
            })
            .collect()
    }
}

fn is_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether an identifier matches a filter of `--filter`.
#[must_use]
pub fn matches_filter(filter: &str, id: &str) -> bool {
    if !filter.contains('*') {
        return id.contains(filter);
    }
    let parts: Vec<&str> = filter.split('*').collect();
    let (first, rest) = parts.split_first().unwrap_or((&"", &[]));
    let Some(mut remaining) = id.strip_prefix(first) else {
        return false;
    };
    let (last, middle) = rest.split_last().unwrap_or((&"", &[]));
    for part in middle {
        match remaining.find(part) {
            Some(index) => remaining = &remaining[index + part.len()..],
            None => return false,
        }
    }
    remaining.ends_with(last)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    for entry in fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if file_type.is_dir() {
            collect_files(&path, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Members of a case or a collection entry, after validation.
#[derive(Debug, Deserialize)]
struct RawCase {
    id: String,
    kind: String,
    class: Option<String>,
    language: Option<String>,
    #[serde(default)]
    requires: Vec<String>,
    summary: String,
    variant: Option<String>,
    code: Option<String>,
    #[serde(default)]
    volatile: Vec<String>,
    config: Option<String>,
    fs_before: Option<Value>,
    step: Option<Vec<RawStep>>,
    #[serde(flatten)]
    inline: RawStep,
}

/// Members of a step, after validation.
#[derive(Debug, Default, Deserialize)]
struct RawStep {
    method: Option<String>,
    program: Option<String>,
    program_file: Option<String>,
    variant: Option<String>,
    options: Option<Value>,
    params: Option<Value>,
    expect: Option<Value>,
    expect_at: Option<String>,
    #[serde(rename = "match")]
    matching: Option<String>,
    expect_error: Option<i64>,
    validate: Option<String>,
    fs_after: Option<Value>,
}

impl RawStep {
    fn is_empty(&self) -> bool {
        self.method.is_none()
            && self.program.is_none()
            && self.program_file.is_none()
            && self.params.is_none()
            && self.expect.is_none()
            && self.expect_error.is_none()
            && self.validate.is_none()
            && self.fs_after.is_none()
    }
}

struct Loader<'a> {
    schemas: &'a SchemaSet,
    cases: Vec<Case>,
}

impl Loader<'_> {
    fn load_file(&mut self, path: &Path) -> Result<(), Error> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name == CASE_FILE {
            let document = read_document(path)?;
            self.validate(path, &document)?;
            let dir = path.parent().unwrap_or(path);
            let case = build_case(path, dir, document, true)?;
            self.cases.push(case);
        } else if COLLECTION_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            let document = read_document(path)?;
            self.validate(path, &document)?;
            let dir = path.parent().unwrap_or(path);
            let entries = match document.get("case") {
                Some(Value::Array(entries)) => entries.clone(),
                _ => Vec::new(),
            };
            for entry in entries {
                self.cases.push(build_case(path, dir, entry, false)?);
            }
        }
        Ok(())
    }

    fn validate(&self, path: &Path, document: &Value) -> Result<(), Error> {
        self.schemas
            .validate(CASE_SCHEMA, document)
            .map_err(|errors| Error::format(path, errors.join("; ")))
    }
}

/// Reads a TOML or JSON document into its JSON form.
fn read_document(path: &Path) -> Result<Value, Error> {
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let is_toml = path
        .extension()
        .is_some_and(|extension| extension == "toml");
    if is_toml {
        let value: toml::Value =
            toml::from_str(&text).map_err(|e| Error::format(path, e.to_string()))?;
        serde_json::to_value(value).map_err(|e| Error::format(path, e.to_string()))
    } else {
        serde_json::from_str(&text).map_err(|e| Error::format(path, e.to_string()))
    }
}

/// Resolves a value that is either a file name of the case directory or the
/// value itself.
fn data(source: &Path, dir: &Path, value: &Value) -> Result<Value, Error> {
    match value {
        Value::String(name) => read_document(&dir.join(name)).map_err(|error| match error {
            Error::Io { path, source: io } => Error::format(
                source_path(source),
                format!("{} cannot be read: {io}", path.display()),
            ),
            other => other,
        }),
        other => Ok(other.clone()),
    }
}

fn source_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn read_text(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

fn optional_text(path: &Path) -> Result<Option<String>, Error> {
    if path.is_file() {
        read_text(path).map(Some)
    } else {
        Ok(None)
    }
}

fn optional_json(path: &Path) -> Result<Option<Value>, Error> {
    if path.is_file() {
        read_document(path).map(Some)
    } else {
        Ok(None)
    }
}

fn snapshot(source: &Path, value: &Value) -> Result<Snapshot, Error> {
    Snapshot::from_json(value).map_err(|e| Error::format(source, e.to_string()))
}

fn build_case(source: &Path, dir: &Path, document: Value, directory: bool) -> Result<Case, Error> {
    let raw: RawCase =
        serde_json::from_value(document).map_err(|e| Error::format(source, e.to_string()))?;
    let invalid = |message: String| Error::format(source, format!("case {:?}: {message}", raw.id));
    let kind = Kind::from_name(&raw.kind)
        .ok_or_else(|| invalid(format!("unknown kind {:?}", raw.kind)))?;
    let class = Class::parse(raw.class.as_deref().unwrap_or("mandatory"))
        .ok_or_else(|| invalid("invalid class".to_owned()))?;
    let requires = raw
        .requires
        .iter()
        .map(|c| Condition::parse(c).ok_or_else(|| invalid(format!("invalid condition {c:?}"))))
        .collect::<Result<Vec<_>, _>>()?;
    let variant = raw
        .variant
        .clone()
        .unwrap_or_else(|| protocol::DEFAULT_VARIANT.to_owned());
    let fs_before = match &raw.fs_before {
        Some(value) => Some(snapshot(source, &data(source, dir, value)?)?),
        None if directory => optional_json(&dir.join(file::FS_BEFORE))?
            .map(|value| snapshot(&dir.join(file::FS_BEFORE), &value))
            .transpose()?,
        None => None,
    };
    let config = raw
        .config
        .as_ref()
        .map(|name| read_text(&dir.join(name)))
        .transpose()?;
    let plan = build_plan(source, dir, &raw, kind, &variant, directory)?;
    Ok(Case {
        id: raw.id,
        kind,
        class,
        language: raw.language.unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
        requires,
        summary: raw.summary,
        variant,
        code: raw.code,
        volatile: raw.volatile,
        config,
        fs_before,
        plan,
        source: source.to_path_buf(),
    })
}

fn build_plan(
    source: &Path,
    dir: &Path,
    raw: &RawCase,
    kind: Kind,
    variant: &str,
    directory: bool,
) -> Result<Plan, Error> {
    if let Some(steps) = &raw.step {
        return steps
            .iter()
            .map(|step| build_step(source, dir, step, variant))
            .collect::<Result<Vec<_>, _>>()
            .map(Plan::Steps);
    }
    if !raw.inline.is_empty() {
        return Ok(Plan::Steps(vec![build_step(
            source,
            dir,
            &raw.inline,
            variant,
        )?]));
    }
    if !directory {
        return Err(Error::format(
            source,
            format!(
                "case {:?} has no calls; give `step` or the keys of a step",
                raw.id
            ),
        ));
    }
    match kind {
        Kind::Golden => golden(dir).map(|g| Plan::Golden(Box::new(g))),
        Kind::Bind => bind_step(source, dir, variant).map(|s| Plan::Steps(vec![s])),
        Kind::Error | Kind::Exec | Kind::Journal => {
            run_step(source, dir, variant).map(|s| Plan::Steps(vec![s]))
        }
        Kind::Correction | Kind::Parity | Kind::Binding => Err(Error::format(
            source,
            format!(
                "case {:?} has no calls; give `step` or the keys of a step",
                raw.id
            ),
        )),
    }
}

fn build_step(source: &Path, dir: &Path, raw: &RawStep, case_variant: &str) -> Result<Step, Error> {
    let method_name = raw.method.clone().unwrap_or_else(|| method::RUN.to_owned());
    let variant = raw.variant.as_deref().unwrap_or(case_variant);
    let program = match (&raw.program, &raw.program_file) {
        (Some(program), _) => Some(program.clone()),
        (None, Some(name)) => Some(read_text(&dir.join(name))?),
        (None, None) => None,
    };
    let options = raw
        .options
        .as_ref()
        .map(|o| data(source, dir, o))
        .transpose()?;
    let params = match (&raw.params, &program) {
        (Some(params), _) => data(source, dir, params)?,
        (None, Some(program)) if protocol::takes_request(&method_name) => {
            protocol::program_request(program, variant, options.as_ref())
        }
        (None, Some(program)) => protocol::text_params(program, variant),
        (None, None) if protocol::takes_request(&method_name) => {
            let mut request =
                serde_json::json!({member::VERSION: nurt_contracts::envelope::ENVELOPE_VERSION});
            if let Some(options) = options {
                request[member::OPTIONS] = options;
            }
            request
        }
        (None, None) => Value::Object(serde_json::Map::new()),
    };
    let at = raw.expect_at.clone().unwrap_or_default();
    let matching = match &raw.matching {
        Some(name) => Match::from_name(name)
            .ok_or_else(|| Error::format(source, format!("unknown match mode {name:?}")))?,
        None => Match::Subset,
    };
    let expectations = raw
        .expect
        .as_ref()
        .map(|value| {
            data(source, dir, value).map(|value| Expectation {
                at: at.clone(),
                value,
                matching,
            })
        })
        .transpose()?
        .into_iter()
        .collect();
    let fs_after = match &raw.fs_after {
        Some(Value::String(name)) if name == UNCHANGED => Some(FsExpectation::Unchanged),
        Some(value) => Some(FsExpectation::Snapshot(snapshot(
            source,
            &data(source, dir, value)?,
        )?)),
        None => None,
    };
    Ok(Step {
        method: method_name,
        params,
        expectations,
        expect_error: raw.expect_error,
        validate: raw.validate.clone().map(|schema| (at, schema)),
        fs_after,
    })
}

fn golden(dir: &Path) -> Result<Golden, Error> {
    let mut texts = Vec::new();
    for variant in protocol::VARIANTS {
        if let Some(text) = optional_text(&dir.join(format!("{variant}.nurt")))? {
            texts.push((variant.to_owned(), text));
        }
    }
    let ast_path = dir.join(file::AST);
    let ast = read_document(&ast_path)?;
    let hash = |name: &str| -> Result<Option<String>, Error> {
        Ok(optional_text(&dir.join(name))?.map(|text| text.trim().to_owned()))
    };
    Ok(Golden {
        texts,
        ast,
        ast_hash: hash(file::AST_HASH)?,
        bound: optional_json(&dir.join(file::BOUND))?,
        bound_hash: hash(file::BOUND_HASH)?,
    })
}

fn bind_step(source: &Path, dir: &Path, variant: &str) -> Result<Step, Error> {
    let program = read_text(&dir.join(file::PROGRAM))?;
    let mut expectations = Vec::new();
    let bound = optional_json(&dir.join(file::BOUND))?;
    let validate = bound
        .is_some()
        .then(|| (format!("/{}", member::BOUND), "bound".to_owned()));
    if let Some(bound) = bound {
        expectations.push(Expectation {
            at: format!("/{}", member::BOUND),
            value: bound,
            matching: Match::Tree,
        });
    }
    if let Some(expect) = optional_json(&dir.join(file::EXPECT))? {
        expectations.push(Expectation {
            at: String::new(),
            value: expect,
            matching: Match::Subset,
        });
    }
    if expectations.is_empty() {
        return Err(Error::format(
            source,
            format!("a bind case needs {} or {}", file::BOUND, file::EXPECT),
        ));
    }
    Ok(Step {
        method: method::CHECK.to_owned(),
        params: protocol::program_request(&program, variant, None),
        expectations,
        expect_error: None,
        validate,
        fs_after: Some(FsExpectation::Unchanged),
    })
}

fn run_step(source: &Path, dir: &Path, variant: &str) -> Result<Step, Error> {
    let params = match optional_json(&dir.join(file::REQUEST))? {
        Some(request) => request,
        None => protocol::program_request(&read_text(&dir.join(file::PROGRAM))?, variant, None),
    };
    let mut expectations = Vec::new();
    if let Some(expect) = optional_json(&dir.join(file::EXPECT))? {
        expectations.push(Expectation {
            at: String::new(),
            value: expect,
            matching: Match::Subset,
        });
    }
    if let Some(report) = optional_json(&dir.join(file::REPORT))? {
        expectations.push(Expectation {
            at: "/report".to_owned(),
            value: report,
            matching: Match::Subset,
        });
    }
    let fs_after = optional_json(&dir.join(file::FS_AFTER))?
        .map(|value| snapshot(source, &value).map(FsExpectation::Snapshot))
        .transpose()?;
    if expectations.is_empty() && fs_after.is_none() {
        return Err(Error::format(
            source,
            format!(
                "the case needs {}, {} or {}",
                file::EXPECT,
                file::REPORT,
                file::FS_AFTER
            ),
        ));
    }
    Ok(Step {
        method: method::RUN.to_owned(),
        params,
        expectations,
        expect_error: None,
        validate: None,
        fs_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn load(root: &Path) -> Result<Suite, Error> {
        Suite::load(root, &SchemaSet::new().unwrap())
    }

    fn suite_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "VERSION", "1.2.3\n");
        root
    }

    #[test]
    fn kinds_classes_and_conditions_have_names() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(Kind::from_name("other"), None);
        assert_eq!(
            Class::parse("optional:testkit"),
            Some(Class::Optional("testkit".to_owned()))
        );
        assert_eq!(Class::parse("optional:"), None);
        assert_eq!(Class::Optional("x".to_owned()).name(), "optional:x");
        assert_eq!(Class::Mandatory.name(), "mandatory");
    }

    #[test]
    fn conditions_hold_by_system_and_features() {
        let features: BTreeSet<String> = ["events".to_owned()].into();
        assert!(Condition::parse("feature:events").unwrap().holds(&features));
        assert!(
            !Condition::parse("feature:testkit")
                .unwrap()
                .holds(&features)
        );
        assert_eq!(
            Condition::parse("os:unix").unwrap().holds(&features),
            cfg!(unix)
        );
        assert_eq!(
            Condition::parse("os:linux").unwrap().holds(&features),
            cfg!(target_os = "linux")
        );
        assert_eq!(
            Condition::parse("os:macos").unwrap().holds(&features),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            Condition::parse("os:windows").unwrap().holds(&features),
            cfg!(windows)
        );
        assert_eq!(Condition::parse("os:beos"), None);
        assert_eq!(Condition::parse("feature:"), None);
        assert_eq!(Condition::parse("os:unix").unwrap().name(), "os:unix");
        assert_eq!(Condition::parse("feature:x").unwrap().name(), "feature:x");
    }

    #[test]
    fn filters_match_substrings_and_patterns() {
        assert!(matches_filter("errors/", "errors/parse/a"));
        assert!(matches_filter("errors/*/a", "errors/parse/a"));
        assert!(matches_filter("*parse*", "errors/parse/a"));
        assert!(!matches_filter("golden/*", "errors/parse/a"));
        assert!(!matches_filter("errors/*/b", "errors/parse/a"));
        assert!(!matches_filter("e*x*a", "errors/parse/a"));
        assert_eq!(major("1.4"), "1");
    }

    /// Suite of one bind, one error and one golden case given by files.
    fn convention_suite() -> (tempfile::TempDir, Suite) {
        let root = suite_root();
        let r = root.path();
        write(
            r,
            "golden/g/case.toml",
            "id = \"golden/g\"\nkind = \"golden\"\nsummary = \"s\"\n",
        );
        write(r, "golden/g/a.nurt", "count\n");
        write(
            r,
            "golden/g/ast.json",
            r#"{"nurt": "1.0", "program": {"statements": []}}"#,
        );
        write(r, "golden/g/ast.hash", "sha256:00\n");
        write(
            r,
            "bind/b/case.toml",
            "id = \"bind/b\"\nkind = \"bind\"\nsummary = \"s\"\nvariant = \"c\"\n",
        );
        write(r, "bind/b/program.nurt", "count");
        write(r, "bind/b/bound.json", r#"{"bound": "1.0"}"#);
        write(r, "bind/b/expect.json", r#"{"ast_hash": "x"}"#);
        write(
            r,
            "errors/e/case.toml",
            "id = \"errors/e\"\nkind = \"error\"\ncode = \"E_IO\"\nsummary = \"s\"\n",
        );
        write(
            r,
            "errors/e/request.json",
            r#"{"nurt": "1.0", "program": "count"}"#,
        );
        write(
            r,
            "errors/e/expect.json",
            r#"{"report": {"status": "failed"}}"#,
        );
        write(r, "errors/e/report.json", r#"{"error": {"code": "E_IO"}}"#);
        write(
            r,
            "errors/e/fs-before.json",
            r#"{"entries": [{"path": "a", "type": "dir"}]}"#,
        );
        write(r, "errors/e/fs-after.json", r#"{"entries": []}"#);
        let suite = load(r).unwrap();
        (root, suite)
    }

    #[test]
    fn suites_have_a_version_a_digest_and_sorted_cases() {
        let (_root, suite) = convention_suite();
        assert_eq!(suite.version, "1.2.3");
        assert!(suite.digest.starts_with("sha256:"));
        let ids: Vec<&str> = suite.cases.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["bind/b", "errors/e", "golden/g"]);
        assert_eq!(suite.select(&[], &[Kind::Bind]).len(), 1);
        assert_eq!(suite.select(&["errors".to_owned()], &[]).len(), 1);
        assert_eq!(suite.select(&[], &[]).len(), 3);
    }

    #[test]
    fn bind_cases_check_without_effect() {
        let (_root, suite) = convention_suite();
        let Plan::Steps(bind) = &suite.cases[0].plan else {
            panic!("steps expected")
        };
        assert_eq!(bind[0].method, "check");
        assert_eq!(
            bind[0].params,
            json!({"nurt": "1.0", "program": "count", "variant": "c"})
        );
        assert_eq!(bind[0].expectations[0].matching, Match::Tree);
        assert_eq!(
            bind[0].validate,
            Some(("/bound".to_owned(), "bound".to_owned()))
        );
        assert_eq!(bind[0].fs_after, Some(FsExpectation::Unchanged));
        assert_eq!(suite.cases[0].expected_values().len(), 2);
    }

    #[test]
    fn error_and_golden_cases_read_their_files() {
        let (_root, suite) = convention_suite();
        let error = &suite.cases[1];
        assert_eq!(error.code.as_deref(), Some("E_IO"));
        assert!(error.fs_before.is_some());
        let Plan::Steps(run) = &error.plan else {
            panic!("steps expected")
        };
        assert_eq!(run[0].expectations[1].at, "/report");
        assert!(matches!(run[0].fs_after, Some(FsExpectation::Snapshot(_))));

        let Plan::Golden(golden) = &suite.cases[2].plan else {
            panic!("golden expected")
        };
        assert_eq!(golden.texts, vec![("a".to_owned(), "count\n".to_owned())]);
        assert_eq!(golden.ast_hash.as_deref(), Some("sha256:00"));
        assert_eq!(golden.bound, None);
        assert_eq!(suite.cases[2].expected_values().len(), 1);
        assert_eq!(suite.cases[2].language_major(), "1");
    }

    #[test]
    fn collections_and_explicit_steps_are_loaded() {
        let root = suite_root();
        let r = root.path();
        write(r, "exec/opts.json", r#"{"preview": true}"#);
        write(r, "exec/p.nurt", "count");
        write(r, "exec/config.toml", "[output]\n");
        write(
            r,
            "exec/set.cases.toml",
            r#"
format = "nurt-conformance-cases/1"

[[case]]
id = "exec/one"
kind = "exec"
class = "optional:testkit"
requires = ["os:unix"]
summary = "s"
program = "find \"a\""
options = "opts.json"
expect = { report = { status = "ok" } }
fs_before = { entries = [{ path = "a", type = "dir" }] }
fs_after = "unchanged"
config = "config.toml"
volatile = ["/report/x"]

[[case]]
id = "exec/two"
kind = "journal"
summary = "s"
language = "2.0"

[[case.step]]
program_file = "p.nurt"
variant = "b"

[[case.step]]
method = "page"
params = { cursor = "${step1/report/next_cursor}" }
expect = { nurt = "1.0" }
expect_at = ""
match = "exact"
validate = "report"
fs_after = { entries = [] }

[[case.step]]
method = "parse"
program = "count"

[[case.step]]
method = "info"

[[case.step]]
method = "check"
options = { limit = 1 }
expect_error = -32602
"#,
        );
        let suite = load(r).unwrap();
        let one = &suite.cases[0];
        assert_eq!(one.class, Class::Optional("testkit".to_owned()));
        assert_eq!(one.requires, vec![Condition::Os("unix".to_owned())]);
        assert_eq!(one.config.as_deref(), Some("[output]\n"));
        assert_eq!(one.volatile, vec!["/report/x"]);
        let Plan::Steps(steps) = &one.plan else {
            panic!("steps expected")
        };
        assert_eq!(steps[0].params["options"], json!({"preview": true}));
        assert_eq!(steps[0].fs_after, Some(FsExpectation::Unchanged));
        let two = &suite.cases[1];
        assert_eq!(two.language_major(), "2");
        let Plan::Steps(steps) = &two.plan else {
            panic!("steps expected")
        };
        assert_eq!(
            steps[0].params,
            json!({"nurt": "1.0", "program": "count", "variant": "b"})
        );
        assert_eq!(steps[1].expectations[0].matching, Match::Exact);
        assert_eq!(
            steps[1].validate,
            Some((String::new(), "report".to_owned()))
        );
        assert_eq!(steps[2].params, json!({"program": "count", "variant": "a"}));
        assert_eq!(steps[3].params, json!({}));
        assert_eq!(
            steps[4].params,
            json!({"nurt": "1.0", "options": {"limit": 1}})
        );
        assert_eq!(steps[4].expect_error, Some(-32602));
    }

    #[test]
    fn invalid_suites_are_rejected_with_the_file_named() {
        let root = suite_root();
        let r = root.path();
        write(
            r,
            "a/case.toml",
            "id = \"x\"\nkind = \"exec\"\nsummary = \"s\"\nprogram = \"count\"\nexpect = {}\n",
        );
        write(
            r,
            "b/case.toml",
            "id = \"x\"\nkind = \"exec\"\nsummary = \"s\"\nprogram = \"count\"\nexpect = {}\n",
        );
        let error = load(r).unwrap_err().to_string();
        assert!(error.contains("also defined"), "{error}");

        let bad = |name: &str, content: &str| {
            let root = suite_root();
            write(root.path(), name, content);
            load(root.path()).unwrap_err().to_string()
        };
        assert!(bad("c/case.toml", "id = \"x\"\nkind = \"exec\"\n").contains("case.toml"));
        assert!(bad("c/case.toml", "id = [").contains("case.toml"));
        assert!(
            bad(
                "c/case.toml",
                "id = \"x\"\nkind = \"error\"\nsummary = \"s\"\nprogram = \"a\"\nexpect = {}\n"
            )
            .contains("code")
        );
        assert!(
            bad(
                "c/case.toml",
                "id = \"x\"\nkind = \"exec\"\nsummary = \"s\"\n"
            )
            .contains("program.nurt")
        );
        assert!(
            bad(
                "c/case.toml",
                "id = \"x\"\nkind = \"bind\"\nsummary = \"s\"\n"
            )
            .contains("program.nurt")
        );
        assert!(
            bad(
                "c/case.toml",
                "id = \"x\"\nkind = \"correction\"\nsummary = \"s\"\n"
            )
            .contains("no calls")
        );
        assert!(bad("c/case.toml", "id = \"x\"\nkind = \"exec\"\nsummary = \"s\"\nprogram = \"a\"\nexpect = \"none.json\"\n").contains("none.json"));
        assert!(bad("c/case.toml", "id = \"x\"\nkind = \"exec\"\nsummary = \"s\"\nprogram = \"a\"\nfs_after = { entries = [{ path = \"/a\", type = \"dir\" }] }\n").contains("case.toml"));
        assert!(bad("c.cases.json", r#"{"format": "nurt-conformance-cases/1", "case": [{"id": "x", "kind": "exec", "summary": "s"}]}"#).contains("no calls"));
        assert!(bad("c.cases.json", "{").contains("c.cases.json"));
        assert!(
            bad(
                "c/case.toml",
                "id = \"x\"\nkind = \"golden\"\nsummary = \"s\"\n"
            )
            .contains("ast.json")
        );

        let unversioned = tempfile::tempdir().unwrap();
        assert!(
            load(unversioned.path())
                .unwrap_err()
                .to_string()
                .contains("VERSION")
        );
        write(unversioned.path(), "VERSION", "1.0");
        assert!(
            load(unversioned.path())
                .unwrap_err()
                .to_string()
                .contains("MAJOR.MINOR.PATCH")
        );
    }
}
