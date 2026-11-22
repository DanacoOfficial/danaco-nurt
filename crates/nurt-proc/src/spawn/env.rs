// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Environment of a started program.

use std::ffi::{OsStr, OsString};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::EnvVar;

use super::Platform;

/// Variables kept from the console environment with `inherit_env: none` on
/// Windows; programs and the command interpreter fail to start without them.
const WINDOWS_REQUIRED: [&str; 3] = ["SystemRoot", "ComSpec", "PATHEXT"];

/// Starting point of the environment of a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InheritEnv {
    /// The environment of the console process.
    #[default]
    All,
    /// An empty environment, apart from the variables Windows requires.
    None,
}

/// Complete environment of a program, in the order the variables were first
/// set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildEnv {
    platform: Platform,
    vars: Vec<(OsString, OsString)>,
}

impl ChildEnv {
    /// Builds the environment from the inherited variables, `defaults` and
    /// `changes`, each overriding the earlier ones. With
    /// [`InheritEnv::None`] only the variables Windows requires are
    /// inherited. A change without a value removes the variable.
    ///
    /// # Errors
    ///
    /// Fails with `E_ARG` when a changed variable has an empty name, a name
    /// with `=` or a NUL character in its name or value.
    pub(crate) fn build<'a>(
        platform: Platform,
        inherited: Vec<(OsString, OsString)>,
        inherit: InheritEnv,
        defaults: impl IntoIterator<Item = (&'a str, &'a str)>,
        changes: &[EnvVar],
    ) -> Result<Self> {
        let mut env = Self {
            platform,
            vars: Vec::new(),
        };
        match inherit {
            InheritEnv::All => {
                for (name, value) in inherited {
                    env.set(name, value);
                }
            }
            InheritEnv::None if platform == Platform::Windows => {
                for (name, value) in inherited {
                    if WINDOWS_REQUIRED
                        .iter()
                        .any(|required| name.eq_ignore_ascii_case(required))
                    {
                        env.set(name, value);
                    }
                }
            }
            InheritEnv::None => {}
        }
        for (name, value) in defaults {
            env.set(OsString::from(name), OsString::from(value));
        }
        for change in changes {
            check_name(&change.name)?;
            match &change.value {
                Some(value) => {
                    if value.contains('\0') {
                        return Err(Error::new(
                            ErrorCode::Arg,
                            format!(
                                "the value of the environment variable `{}` contains a NUL character; remove it",
                                change.name
                            ),
                        )
                        .with_problem("nul_in_environment"));
                    }
                    env.set(OsString::from(&change.name), OsString::from(value));
                }
                None => env.remove(OsStr::new(&change.name)),
            }
        }
        Ok(env)
    }

    /// Rules of the host the environment is for.
    pub(crate) fn platform(&self) -> Platform {
        self.platform
    }

    /// Value of one variable.
    pub(crate) fn get(&self, name: &str) -> Option<&OsStr> {
        self.position(OsStr::new(name))
            .map(|index| self.vars[index].1.as_os_str())
    }

    /// Names of all variables, in order.
    pub(crate) fn names(&self) -> impl Iterator<Item = &OsStr> {
        self.vars.iter().map(|(name, _)| name.as_os_str())
    }

    /// The variables, in order.
    pub(crate) fn into_vars(self) -> Vec<(OsString, OsString)> {
        self.vars
    }

    fn position(&self, name: &OsStr) -> Option<usize> {
        self.vars.iter().position(|(existing, _)| match self.platform {
            Platform::Unix => existing == name,
            Platform::Windows => existing.eq_ignore_ascii_case(name),
        })
    }

    fn set(&mut self, name: OsString, value: OsString) {
        match self.position(&name) {
            Some(index) => self.vars[index].1 = value,
            None => self.vars.push((name, value)),
        }
    }

    fn remove(&mut self, name: &OsStr) {
        if let Some(index) = self.position(name) {
            self.vars.remove(index);
        }
    }
}

fn check_name(name: &str) -> Result<()> {
    let problem = if name.is_empty() {
        Some(("empty_environment_name", "is empty"))
    } else if name.contains('=') {
        Some(("equals_in_environment_name", "contains `=`"))
    } else if name.contains('\0') {
        Some(("nul_in_environment", "contains a NUL character"))
    } else {
        None
    };
    match problem {
        Some((code, reason)) => Err(Error::new(
            ErrorCode::Arg,
            format!("the environment variable name `{name}` {reason}; give a name without it"),
        )
        .with_problem(code)),
        None => Ok(()),
    }
}
