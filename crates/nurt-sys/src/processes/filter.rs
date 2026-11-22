// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selection of processes by name, account and identifier.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::{Regex as PatternValue, Value};
use regex::{Regex, RegexBuilder};

use super::model::ProcessInfo;
use super::{PARAM_NAME, PARAM_PID, PARAM_USER};

/// Suffix of program files on Windows, ignored when a name is compared as
/// text.
const WINDOWS_PROGRAM_SUFFIX: &str = ".exe";

/// Condition on the process name.
#[derive(Debug, Clone)]
pub(crate) enum NameFilter {
    /// The name equals the text, or the text followed by `.exe`.
    Text(String),
    /// The pattern matches part of the name.
    Pattern(Regex),
}

impl NameFilter {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Text(text) => {
                name == text
                    || name
                        .strip_prefix(text.as_str())
                        .is_some_and(|tail| tail.eq_ignore_ascii_case(WINDOWS_PROGRAM_SUFFIX))
            }
            Self::Pattern(regex) => regex.is_match(name),
        }
    }
}

/// Conditions of the parameters `name`, `user` and `pid`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Filter {
    name: Option<NameFilter>,
    user: Option<String>,
    pid: Option<u64>,
}

impl Filter {
    /// Reads the conditions from the bound arguments.
    ///
    /// # Errors
    ///
    /// Fails with `E_INVALID_VALUE` for an invalid regular expression or a
    /// negative identifier, and with `E_TYPE` for a value of another type.
    pub(crate) fn from_args(args: &BoundArgs) -> Result<Self> {
        let name = match args.value(PARAM_NAME) {
            None | Some(Value::Null) => None,
            Some(Value::Text(text)) => Some(NameFilter::Text(text.clone())),
            Some(Value::Regex(pattern)) => Some(NameFilter::Pattern(compile(pattern)?)),
            Some(_) => {
                return Err(Error::new(
                    ErrorCode::Type,
                    "`processes` expects text or a regular expression for `name`",
                ));
            }
        };
        let pid = match args.int(PARAM_PID)? {
            Some(pid) => Some(u64::try_from(pid).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!("`pid` must not be negative, got {pid}"),
                )
            })?),
            None => None,
        };
        Ok(Self {
            name,
            user: args.text(PARAM_USER)?.map(str::to_owned),
            pid,
        })
    }

    /// Returns `true` when no condition is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.name.is_none() && self.user.is_none() && self.pid.is_none()
    }

    /// The requested process identifier.
    pub(crate) fn pid(&self) -> Option<u64> {
        self.pid
    }

    /// Returns `true` when `process` meets every condition.
    pub(crate) fn matches(&self, process: &ProcessInfo) -> bool {
        self.pid.is_none_or(|pid| process.pid == pid)
            && self
                .name
                .as_ref()
                .is_none_or(|name| name.matches(&process.name))
            && self.user.as_deref().is_none_or(|user| {
                process.user.as_deref() == Some(user) || process.uid.as_deref() == Some(user)
            })
    }
}

/// Compiles a regular expression of the language.
fn compile(pattern: &PatternValue) -> Result<Regex> {
    RegexBuilder::new(&pattern.pattern)
        .case_insensitive(pattern.flags.case_insensitive)
        .multi_line(pattern.flags.multi_line)
        .dot_matches_new_line(pattern.flags.dot_matches_new_line)
        .ignore_whitespace(pattern.flags.extended)
        .build()
        .map_err(|error| {
            Error::new(
                ErrorCode::InvalidValue,
                format!("`name` is not a valid regular expression: {error}"),
            )
            .with_found(pattern.pattern.clone())
        })
}
