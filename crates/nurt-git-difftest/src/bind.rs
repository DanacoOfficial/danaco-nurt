// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-difftest::bind
// Responsibility:   Binding of scenario parameters to the signatures of `catalog/git.toml`.
// Dependencies:     indexmap, nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Binding of scenario parameters to the signatures of `catalog/git.toml`,
//! and the host seen by the Nurt command.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use nurt_contracts::env::{Host, HostInfo, Os, PathStyle};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::spec::{CatalogFile, DefaultKind, ParamSpec, ParamType, ScalarType, StageSpec};
use nurt_contracts::stage::{ArgValue, BoundArgs};
use nurt_contracts::value::{Instant, Range, Regex, Value};

use crate::error::{DifftestError, Result};
use crate::git::GitEnv;
use crate::time::ScenarioClock;

/// Text of the catalog of the `git` group.
const GIT_CATALOG: &str = include_str!("../../../catalog/git.toml");

/// Name of the parameter that receives the repository.
const REPO_PARAM: &str = "repo";

/// Commands of the `git` group with their parameter signatures.
#[derive(Debug, Clone)]
pub struct Catalog {
    commands: BTreeMap<String, StageSpec>,
}

/// Bound arguments of one Nurt call.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// Arguments in signature order, with explicit defaults.
    pub args: BoundArgs,
    /// Parameters given by the scenario that the catalog does not list; they
    /// are bound with the type of their TOML value.
    pub unknown: Vec<String>,
}

impl Catalog {
    /// Reads the catalog of the `git` group built into the test runner.
    ///
    /// # Errors
    ///
    /// Fails with [`DifftestError::Catalog`] when the catalog is invalid.
    pub fn git() -> Result<Self> {
        Self::parse(GIT_CATALOG)
    }

    /// Parses a catalog file.
    ///
    /// # Errors
    ///
    /// Fails with [`DifftestError::Catalog`] when the text is not a catalog
    /// file.
    pub fn parse(text: &str) -> Result<Self> {
        let mut file: CatalogFile =
            toml::from_str(text).map_err(|error| DifftestError::Catalog(error.to_string()))?;
        file.resolve_groups();
        let commands = file
            .commands
            .into_iter()
            .map(|command| (command.name.clone(), command))
            .collect();
        Ok(Self { commands })
    }

    /// Specification of one command.
    #[must_use]
    pub fn command(&self, name: &str) -> Option<&StageSpec> {
        self.commands.get(name)
    }

    /// Binds the parameters of a scenario to the signature of `command`.
    ///
    /// The repository `repo_root` is given to the parameter `repo` when the
    /// scenario does not set it; relative paths are resolved against it.
    ///
    /// # Errors
    ///
    /// Returns a message when the command is unknown or a value does not
    /// have the type of its parameter.
    pub fn bind(
        &self,
        command: &str,
        params: &toml::Table,
        repo_root: &Path,
    ) -> Result<Binding, String> {
        let spec = self
            .command(command)
            .ok_or_else(|| format!("command `{command}` is not in catalog/git.toml"))?;
        let mut args = BoundArgs::new(command);
        for param in &spec.params {
            let (value, defaulted) = bind_param(param, params.get(&param.name), repo_root)?;
            args.insert(param.name.clone(), value, defaulted);
        }
        let mut unknown = Vec::new();
        for (name, value) in params {
            if spec.param(name).is_none() {
                args.insert(name.clone(), ArgValue::Value(infer(value)), false);
                unknown.push(name.clone());
            }
        }
        Ok(Binding { args, unknown })
    }
}

fn bind_param(
    param: &ParamSpec,
    given: Option<&toml::Value>,
    repo_root: &Path,
) -> Result<(ArgValue, bool), String> {
    let context = |message: String| format!("parameter `{}`: {message}", param.name);
    if let Some(value) = given {
        let converted = convert(&param.ty, value, repo_root).map_err(context)?;
        return Ok((ArgValue::Value(converted), false));
    }
    if param.name == REPO_PARAM {
        let path = Value::Path(language_path(repo_root).map_err(context)?);
        let value = if param.ty.accepts_list() {
            Value::List(vec![path])
        } else {
            path
        };
        return Ok((ArgValue::Value(value), false));
    }
    match &param.default {
        Some(default) if !default.is_blank() => {
            let value =
                default_value(&param.ty, default.as_str(), default.kind()).map_err(context)?;
            Ok((ArgValue::Value(value), true))
        }
        _ => Ok((ArgValue::Absent, false)),
    }
}

/// Converts a TOML value to a value of type `ty`.
fn convert(ty: &ParamType, value: &toml::Value, repo_root: &Path) -> Result<Value, String> {
    match ty {
        ParamType::Scalar(scalar) => convert_scalar(*scalar, value, repo_root),
        ParamType::Enum(values) => match value {
            toml::Value::String(text) if values.contains(text) => Ok(Value::Enum(text.clone())),
            other => Err(format!("{other} is not one of {}", values.join(", "))),
        },
        ParamType::List(inner) => match value {
            toml::Value::Array(items) => items
                .iter()
                .map(|item| convert(inner, item, repo_root))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::List),
            single => Ok(Value::List(vec![convert(inner, single, repo_root)?])),
        },
        ParamType::Map(inner) => match value {
            toml::Value::Table(table) => table
                .iter()
                .map(|(key, item)| Ok((key.clone(), convert(inner, item, repo_root)?)))
                .collect::<Result<IndexMap<_, _>, String>>()
                .map(Value::Map),
            other => Err(format!("{other} is not a table")),
        },
        ParamType::Union(alternatives) => alternatives
            .iter()
            .find_map(|alternative| convert(alternative, value, repo_root).ok())
            .ok_or_else(|| format!("{value} matches none of the types {ty}")),
        other => Err(format!("type {other} cannot be written in a scenario")),
    }
}

fn convert_scalar(
    scalar: ScalarType,
    value: &toml::Value,
    repo_root: &Path,
) -> Result<Value, String> {
    let mismatch = || format!("{value} is not a value of type {scalar}");
    match (scalar, value) {
        (ScalarType::Text | ScalarType::Any, toml::Value::String(text)) => {
            Ok(Value::Text(text.clone()))
        }
        (ScalarType::Int, toml::Value::Integer(number)) => Ok(Value::Int(*number)),
        (ScalarType::Number, toml::Value::Integer(number)) => Ok(Value::Number(*number as f64)),
        (ScalarType::Number, toml::Value::Float(number)) => Ok(Value::Number(*number)),
        (ScalarType::Bool, toml::Value::Boolean(flag)) => Ok(Value::Bool(*flag)),
        (ScalarType::Path, toml::Value::String(text)) => Ok(Value::Path(resolve(repo_root, text)?)),
        (ScalarType::Glob, toml::Value::String(text)) => Ok(Value::Glob(text.clone())),
        (ScalarType::Key, toml::Value::String(text)) => Ok(Value::Key(text.clone())),
        (ScalarType::Regex, toml::Value::String(text)) => regex(text),
        (ScalarType::Instant, toml::Value::String(text)) => instant(text),
        (ScalarType::Range, toml::Value::Integer(number)) => {
            Ok(Value::Range(Range::single(*number)))
        }
        (ScalarType::Range, toml::Value::String(text)) => range(text),
        (ScalarType::Any, other) => Ok(infer(other)),
        _ => Err(mismatch()),
    }
}

/// Value of a parameter that the catalog does not describe, typed by its
/// TOML form.
fn infer(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::Text(text.clone()),
        toml::Value::Integer(number) => Value::Int(*number),
        toml::Value::Float(number) => Value::Number(*number),
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Datetime(date) => Value::Text(date.to_string()),
        toml::Value::Array(items) => Value::List(items.iter().map(infer).collect()),
        toml::Value::Table(table) => Value::Map(
            table
                .iter()
                .map(|(key, item)| (key.clone(), infer(item)))
                .collect(),
        ),
    }
}

fn regex(text: &str) -> Result<Value, String> {
    let (pattern, flags) = match text
        .strip_prefix('/')
        .and_then(|rest| rest.rsplit_once('/'))
    {
        Some((pattern, flags)) => (pattern, flags),
        None => (text, ""),
    };
    Regex::new(pattern, flags)
        .map(Value::Regex)
        .map_err(|error| format!("invalid regular expression: {}", error.reason))
}

fn instant(text: &str) -> Result<Value, String> {
    let clock = ScenarioClock::parse(text)?;
    Instant::from_unix_seconds(clock.unix_seconds())
        .map(Value::Instant)
        .ok_or_else(|| format!("instant `{text}` is out of range"))
}

fn range(text: &str) -> Result<Value, String> {
    let (start, end) = text
        .split_once("..")
        .ok_or_else(|| format!("range `{text}` is not of the form a..b, a.. or ..b"))?;
    let bound = |part: &str| -> Result<Option<i64>, String> {
        if part.is_empty() {
            return Ok(None);
        }
        part.parse()
            .map(Some)
            .map_err(|_| format!("range `{text}` has a bound that is not an integer"))
    };
    Range::new(bound(start)?, bound(end)?)
        .map(Value::Range)
        .map_err(|error| error.reason)
}

/// Value of a catalog default literal.
fn default_value(ty: &ParamType, literal: &str, kind: DefaultKind) -> Result<Value, String> {
    let literal = literal.trim();
    match kind {
        DefaultKind::Bool => Ok(Value::Bool(literal == "true")),
        DefaultKind::Null => Ok(Value::Null),
        DefaultKind::Int => literal
            .replace('_', "")
            .parse()
            .map(Value::Int)
            .map_err(|_| format!("default `{literal}` is not an integer")),
        DefaultKind::Text => Ok(Value::Text(unquote(literal))),
        DefaultKind::Word if matches!(ty, ParamType::Enum(_)) => {
            Ok(Value::Enum(literal.to_owned()))
        }
        DefaultKind::Word => Ok(Value::Text(literal.to_owned())),
        DefaultKind::List if literal == "[]" => Ok(Value::List(Vec::new())),
        _ => Err(format!(
            "default `{literal}` cannot be bound by the test runner"
        )),
    }
}

fn unquote(literal: &str) -> String {
    let inner = literal
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(literal);
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// Absolute path of `text` relative to `root`, in language form.
fn resolve(root: &Path, text: &str) -> Result<String, String> {
    let path = Path::new(text);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    language_path(&joined)
}

fn language_path(path: &Path) -> Result<String, String> {
    AbsPath::new(path)
        .map(|abs| abs.to_language_string())
        .map_err(|_| format!("path `{}` is not a valid absolute path", path.display()))
}

/// Host of the Nurt side: the environment of the `git` side and fixed facts
/// about the machine.
#[derive(Debug, Clone)]
pub struct ScenarioHost {
    info: HostInfo,
    vars: Vec<(OsString, OsString)>,
}

impl ScenarioHost {
    /// Host with the variables of `env`, home directory `home` and the time
    /// zone of `clock`.
    #[must_use]
    pub fn new(env: &GitEnv, home: &Path, temp: &Path, clock: ScenarioClock) -> Self {
        let info = HostInfo {
            os: host_os(),
            os_version: "test".to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            hostname: "difftest".to_owned(),
            user: "nurt".to_owned(),
            home: PathBuf::from(home),
            temp: PathBuf::from(temp),
            path_style: if cfg!(windows) {
                PathStyle::Windows
            } else {
                PathStyle::Posix
            },
            timezone: clock.timezone(),
            locale: None,
            cpus: 1,
        };
        let vars = env
            .vars()
            .iter()
            .map(|(name, value)| (OsString::from(name), value.clone()))
            .collect();
        Self { info, vars }
    }
}

fn host_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Macos
    } else {
        Os::Linux
    }
}

impl Host for ScenarioHost {
    fn info(&self) -> &HostInfo {
        &self.info
    }

    fn env_var(&self, name: &str) -> Option<OsString> {
        self.vars
            .iter()
            .find(|(existing, _)| {
                if cfg!(windows) {
                    existing.to_string_lossy().eq_ignore_ascii_case(name)
                } else {
                    existing == name
                }
            })
            .map(|(_, value)| value.clone())
    }

    fn env_vars(&self) -> Vec<(OsString, OsString)> {
        self.vars.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    fn root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:/case/repo")
        } else {
            PathBuf::from("/case/repo")
        }
    }

    #[test]
    fn the_built_in_catalog_lists_every_git_command() {
        let catalog = Catalog::git().unwrap();
        for name in [
            "git.status",
            "git.merge",
            "git.push",
            "git.stash",
            "git.worktree",
        ] {
            assert!(catalog.command(name).is_some(), "{name}");
        }
    }

    #[test]
    fn adds_the_repository_and_explicit_defaults() {
        let catalog = Catalog::git().unwrap();
        let binding = catalog
            .bind("git.merge", &table("branch = \"topic\""), &root())
            .unwrap();
        let args = &binding.args;
        assert!(binding.unknown.is_empty());
        assert_eq!(args.value("branch"), Some(&Value::Text("topic".into())));
        assert_eq!(args.value("action"), Some(&Value::Enum("start".into())));
        assert!(args.is_defaulted("action"));
        assert_eq!(args.value("no_commit"), Some(&Value::Bool(false)));
        assert_eq!(args.get("message"), Some(&ArgValue::Absent));
        assert_eq!(
            args.value("repo"),
            Some(&Value::Path(
                AbsPath::new(root()).unwrap().to_language_string()
            ))
        );
    }

    #[test]
    fn wraps_the_repository_in_a_list_when_the_parameter_is_a_list() {
        let catalog = Catalog::git().unwrap();
        let binding = catalog
            .bind("git.status", &toml::Table::new(), &root())
            .unwrap();
        assert!(matches!(binding.args.value("repo"), Some(Value::List(items)) if items.len() == 1));
        assert_eq!(binding.args.value("limit"), Some(&Value::Int(200)));
    }

    #[test]
    fn resolves_relative_paths_against_the_repository() {
        let catalog = Catalog::git().unwrap();
        let binding = catalog
            .bind(
                "git.stash",
                &table("action = \"push\"\npaths = [\"a.txt\", \"dir/b\"]"),
                &root(),
            )
            .unwrap();
        let Some(Value::List(paths)) = binding.args.value("paths") else {
            panic!("paths are not a list");
        };
        assert!(matches!(&paths[1], Value::Path(path) if path.ends_with("/case/repo/dir/b")));
    }

    #[test]
    fn rejects_values_of_the_wrong_type() {
        let catalog = Catalog::git().unwrap();
        let error = catalog
            .bind("git.merge", &table("action = \"rewind\""), &root())
            .unwrap_err();
        assert!(error.contains("parameter `action`"), "{error}");
        let error = catalog
            .bind("git.merge", &table("no_commit = 1"), &root())
            .unwrap_err();
        assert!(error.contains("bool"), "{error}");
        assert!(
            catalog
                .bind("git.frobnicate", &toml::Table::new(), &root())
                .is_err()
        );
    }

    #[test]
    fn binds_parameters_missing_from_the_catalog_by_their_toml_type() {
        let catalog = Catalog::git().unwrap();
        let binding = catalog
            .bind("git.push", &table("extra = [1, \"x\"]"), &root())
            .unwrap();
        assert!(binding.unknown.contains(&"extra".to_owned()));
        assert_eq!(
            binding.args.value("extra"),
            Some(&Value::List(vec![Value::Int(1), Value::Text("x".into())]))
        );
    }

    #[test]
    fn converts_regex_range_instant_and_union_values() {
        let path = root();
        let regex_type: ParamType = "regex".parse().unwrap();
        let converted = convert(&regex_type, &toml::Value::String("/a+/i".into()), &path).unwrap();
        assert!(
            matches!(converted, Value::Regex(regex) if regex.pattern == "a+" && regex.flags.case_insensitive)
        );
        let range_type: ParamType = "range".parse().unwrap();
        assert!(convert(&range_type, &toml::Value::String("2..5".into()), &path).is_ok());
        assert!(convert(&range_type, &toml::Value::String("x".into()), &path).is_err());
        let instant_type: ParamType = "instant".parse().unwrap();
        let converted = convert(
            &instant_type,
            &toml::Value::String("2026-01-01T00:00:00Z".into()),
            &path,
        )
        .unwrap();
        assert_eq!(
            converted,
            Value::Instant(Instant::from_unix_seconds(1_767_225_600).unwrap())
        );
        let union: ParamType = "text | list<text>".parse().unwrap();
        let list = toml::Value::Array(vec![toml::Value::String("a".into())]);
        assert!(matches!(
            convert(&union, &list, &path).unwrap(),
            Value::List(_)
        ));
    }

    #[test]
    fn unquotes_text_defaults() {
        assert_eq!(unquote("\"HEAD\""), "HEAD");
        assert_eq!(unquote("\"a\\\"b\""), "a\"b");
        let ty: ParamType = "text".parse().unwrap();
        assert!(default_value(&ty, "10mb", DefaultKind::Size).is_err());
    }

    #[test]
    fn host_exposes_the_scenario_environment() {
        let home = root();
        let env = GitEnv::new(
            &home,
            ScenarioClock::parse("2026-01-01T12:00:00+02:00").unwrap(),
        );
        let host = ScenarioHost::new(
            &env,
            &home,
            &home,
            ScenarioClock::parse("2026-01-01T12:00:00+02:00").unwrap(),
        );
        assert_eq!(host.info().timezone, "Etc/GMT-2");
        assert_eq!(
            host.env_var("GIT_AUTHOR_NAME"),
            Some(OsString::from(crate::git::IDENTITY_NAME))
        );
        assert_eq!(host.env_vars().len(), env.vars().len());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
