// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Environment variables: the `environment` command.
//!
//! `scope: process` lists the environment of the console process as the
//! execution context reports it. On Windows `scope: user` and
//! `scope: machine` list the persistent variables of the account and of the
//! computer, stored in the registry keys returned by [`persistent_location`];
//! values of type `REG_EXPAND_SZ` are listed without expanding references.
//! Values are never masked. A value that is not valid UTF-8 is listed with
//! the invalid bytes replaced by U+FFFD and one `encoding_changed` note per
//! variable.
//!
//! Persistent variables are changed by writing the registry value, for
//! example `write "registry:/HKCU/Environment/NAME" "value" type: string`; on
//! Linux and macOS by editing profile files with `write` or `replace`.

#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use std::ffi::OsString;

use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{ProviderKey, ProviderScheme};
use nurt_contracts::record::{EnvironmentScope, EnvironmentVariable};
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Value};
use regex::{Regex, RegexBuilder};

/// Name of the command.
pub(crate) const COMMAND: &str = "environment";
/// Parameter with the variable name or pattern.
pub(crate) const NAME: &str = "name";
/// Parameter with the scope.
pub(crate) const SCOPE: &str = "scope";
/// Scheme of the registry provider.
const REGISTRY_SCHEME: &str = "registry";
/// Key of the persistent variables of the account.
const USER_KEY: [&str; 2] = ["HKCU", "Environment"];
/// Key of the persistent variables of the computer.
const MACHINE_KEY: [&str; 6] = [
    "HKLM",
    "SYSTEM",
    "CurrentControlSet",
    "Control",
    "Session Manager",
    "Environment",
];
/// Profile files where persistent variables are set on Linux and macOS.
const PROFILE_FILES: [&str; 3] = ["~/.profile", "~/.bashrc", "~/.zshenv"];

/// Registry key that holds the persistent variables of `scope`; `None` for
/// the process scope.
///
/// This is the only source of these locations: the registry provider uses it
/// to broadcast `WM_SETTINGCHANGE` after a write.
///
/// ```
/// use nurt_contracts::record::EnvironmentScope;
/// use nurt_sys::environment::persistent_location;
///
/// let key = persistent_location(EnvironmentScope::User).unwrap();
/// assert_eq!(key.segments, ["HKCU", "Environment"]);
/// assert!(persistent_location(EnvironmentScope::Process).is_none());
/// ```
#[must_use]
pub fn persistent_location(scope: EnvironmentScope) -> Option<ProviderKey> {
    let segments: &[&str] = match scope {
        EnvironmentScope::User => &USER_KEY,
        EnvironmentScope::Machine => &MACHINE_KEY,
        _ => return None,
    };
    Some(ProviderKey {
        scheme: ProviderScheme(REGISTRY_SCHEME.to_owned()),
        segments: segments.iter().map(|segment| (*segment).to_owned()).collect(),
        value: None,
    })
}

/// The `environment` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::environment::EnvironmentStage;
///
/// assert_eq!(EnvironmentStage::new().descriptor().command, "environment");
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvironmentStage;

impl EnvironmentStage {
    /// The command.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl StageImpl for EnvironmentStage {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        require_os(scope(args)?, ctx.host().info().os)?;
        NameFilter::from_args(args, ctx.host().info().os)?;
        Ok(StaticInfo::default())
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for EnvironmentStage {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let os = ctx.host().info().os;
        let scope = scope(args)?;
        require_os(scope, os)?;
        let filter = NameFilter::from_args(args, os)?;
        let variables = match scope {
            EnvironmentScope::Process => process_variables(ctx),
            other => persistent_variables(other)?,
        };
        let records: Vec<Record> = variables
            .into_iter()
            .filter(|(name, _)| filter.matches(name))
            .map(|(name, value)| {
                EnvironmentVariable {
                    name,
                    value,
                    scope,
                }
                .into()
            })
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

fn scope(args: &BoundArgs) -> Result<EnvironmentScope> {
    match args.value(SCOPE) {
        None => Ok(EnvironmentScope::Process),
        Some(value) => value
            .as_str()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::Type,
                    "`environment` expects `process`, `user` or `machine` for `scope`",
                )
            }),
    }
}

fn require_os(scope: EnvironmentScope, os: Os) -> Result<()> {
    if scope == EnvironmentScope::Process || os == Os::Windows {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::OsOnly,
        format!(
            "scope `{scope}` exists only on windows; persistent variables on this system are \
             set in profile files, changed with `write` or `replace`"
        ),
    )
    .with_fix("environment scope: process")
    .with_help(COMMAND)
    .with_detail("profiles", serde_json::json!(PROFILE_FILES)))
}

fn process_variables(ctx: &ExecCtx) -> Vec<(String, String)> {
    ctx.host()
        .env_vars()
        .into_iter()
        .map(|(name, value)| {
            let name = lossy(name);
            let value = value.into_string().unwrap_or_else(|raw| {
                ctx.note(NotableEntry {
                    path: Some(format!("env:/{name}")),
                    detail: Some(format!(
                        "the value of `{name}` is not valid UTF-8; invalid bytes were replaced"
                    )),
                    ..NotableEntry::new(Notable::EncodingChanged)
                });
                raw.to_string_lossy().into_owned()
            });
            (name, value)
        })
        .collect()
}

fn lossy(text: OsString) -> String {
    text.into_string()
        .unwrap_or_else(|raw| raw.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn persistent_variables(scope: EnvironmentScope) -> Result<Vec<(String, String)>> {
    windows::read_persistent(scope)
}

#[cfg(not(windows))]
fn persistent_variables(scope: EnvironmentScope) -> Result<Vec<(String, String)>> {
    require_os(scope, Os::Linux).map(|()| Vec::new())
}

/// Selection of variables by the `name` parameter.
#[derive(Debug, Clone)]
pub(crate) enum NameFilter {
    /// Every variable.
    All,
    /// The variable with this name; case-insensitive on Windows.
    Exact {
        /// Name.
        name: String,
        /// Names are compared regardless of case.
        ignore_case: bool,
    },
    /// Variables whose name matches the pattern.
    Pattern(Regex),
}

impl NameFilter {
    /// Filter from the `name` argument.
    ///
    /// # Errors
    ///
    /// Fails with `E_REGEX` for an invalid pattern and `E_TYPE` for a value
    /// that is neither text nor a pattern.
    pub(crate) fn from_args(args: &BoundArgs, os: Os) -> Result<Self> {
        match args.value(NAME) {
            None => Ok(Self::All),
            Some(Value::Text(name)) => Ok(Self::Exact {
                name: name.clone(),
                ignore_case: os == Os::Windows,
            }),
            Some(Value::Regex(pattern)) => RegexBuilder::new(&pattern.pattern)
                .case_insensitive(pattern.flags.case_insensitive)
                .multi_line(pattern.flags.multi_line)
                .dot_matches_new_line(pattern.flags.dot_matches_new_line)
                .ignore_whitespace(pattern.flags.extended)
                .build()
                .map(Self::Pattern)
                .map_err(|error| {
                    Error::new(ErrorCode::Regex, format!("invalid pattern for `name`: {error}"))
                }),
            Some(_) => Err(Error::new(
                ErrorCode::Type,
                "`environment` expects text or a pattern for `name`",
            )),
        }
    }

    /// Returns `true` when the variable `name` is selected.
    pub(crate) fn matches(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Exact {
                name: wanted,
                ignore_case: true,
            } => wanted.eq_ignore_ascii_case(name),
            Self::Exact { name: wanted, .. } => wanted == name,
            Self::Pattern(pattern) => pattern.is_match(name),
        }
    }
}
