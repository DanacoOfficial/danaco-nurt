// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Conditions of the benchmark: the tool given to the model.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::BenchError;

/// Tool condition of a run.
///
/// The agent loop, the system message and the limits are the same in every
/// condition; only the tool and its description differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Condition {
    /// bash with the usual command line programs (Linux, macOS).
    Bash,
    /// PowerShell 7 (Windows).
    Pwsh,
    /// Danaco Nurt with the description of the core.
    Nurt,
    /// Danaco Nurt with the core description and group descriptions loaded
    /// when a program first uses a command of the group.
    NurtGroups,
}

impl Condition {
    /// Every condition in the order of the protocol.
    pub const ALL: [Self; 4] = [Self::Bash, Self::Pwsh, Self::Nurt, Self::NurtGroups];

    /// Name of the condition as written on the command line and in run logs.
    ///
    /// ```
    /// use nurt_bench::Condition;
    ///
    /// assert_eq!(Condition::NurtGroups.as_str(), "nurt-groups");
    /// assert_eq!("pwsh".parse::<Condition>().ok(), Some(Condition::Pwsh));
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Pwsh => "pwsh",
            Self::Nurt => "nurt",
            Self::NurtGroups => "nurt-groups",
        }
    }

    /// Whether the tool of the condition is Danaco Nurt.
    #[must_use]
    pub const fn is_nurt(self) -> bool {
        matches!(self, Self::Nurt | Self::NurtGroups)
    }

    /// Name of the file of the tool definition in `bench/protocol/tools/`,
    /// without the extension. Both Danaco Nurt conditions share one
    /// definition.
    #[must_use]
    pub const fn tool_file(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Pwsh => "pwsh",
            Self::Nurt | Self::NurtGroups => "nurt",
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Condition {
    type Err = BenchError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|condition| condition.as_str() == text)
            .ok_or_else(|| {
                BenchError::Argument(format!(
                    "unknown condition {text:?}; use bash, pwsh, nurt or nurt-groups"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_condition_from_its_name() {
        for condition in Condition::ALL {
            assert_eq!(condition.as_str().parse::<Condition>().unwrap(), condition);
            assert_eq!(condition.to_string(), condition.as_str());
        }
    }

    #[test]
    fn rejects_unknown_condition() {
        let error = "zsh".parse::<Condition>().unwrap_err();
        assert!(error.to_string().contains("unknown condition"));
    }

    #[test]
    fn nurt_conditions_share_the_tool_definition() {
        assert!(Condition::Nurt.is_nurt());
        assert!(Condition::NurtGroups.is_nurt());
        assert!(!Condition::Bash.is_nurt());
        assert_eq!(Condition::NurtGroups.tool_file(), "nurt");
        assert_eq!(Condition::Pwsh.tool_file(), "pwsh");
    }

    #[test]
    fn serializes_in_kebab_case() {
        let text = serde_json::to_string(&Condition::NurtGroups).unwrap();
        assert_eq!(text, "\"nurt-groups\"");
    }
}
