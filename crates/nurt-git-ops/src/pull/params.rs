// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parameters of `git.pull`.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use nurt_contracts::error::{Error, ErrorCode, Result, Stage};
use nurt_contracts::stage::BoundArgs;
use nurt_contracts::value::Value;

/// How the fetched branch is integrated into the current branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PullMode {
    /// Fast-forward when possible, otherwise create a merge commit
    /// (`git pull --no-rebase`).
    Merge,
    /// Replay the local commits onto the fetched branch
    /// (`git pull --rebase`).
    Rebase,
    /// Fast-forward only; diverged histories are refused
    /// (`git pull --ff-only`).
    FastForwardOnly,
}

impl PullMode {
    /// Every mode in catalog order.
    pub const ALL: [Self; 3] = [Self::Merge, Self::Rebase, Self::FastForwardOnly];

    /// Value of the `mode` parameter.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_ops::pull::PullMode;
    ///
    /// assert_eq!(PullMode::FastForwardOnly.as_str(), "fast_forward_only");
    /// assert_eq!("rebase".parse(), Ok(PullMode::Rebase));
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::FastForwardOnly => "fast_forward_only",
        }
    }

    /// Option of `git pull` with the same meaning; it is part of the reflog
    /// messages that `git` writes.
    #[must_use]
    pub const fn git_option(self) -> &'static str {
        match self {
            Self::Merge => "--no-rebase",
            Self::Rebase => "--rebase",
            Self::FastForwardOnly => "--ff-only",
        }
    }
}

impl fmt::Display for PullMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PullMode {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == text)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::UnknownValue,
                    format!("unknown mode `{text}`; use merge, rebase or fast_forward_only"),
                )
                .with_stage(Stage::Bind)
                .with_expected(serde_json::json!([
                    "merge",
                    "rebase",
                    "fast_forward_only"
                ]))
            })
    }
}

/// Bound parameters of `git.pull`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullParams {
    /// Repository or a path inside it.
    pub repo: PathBuf,
    /// Remote given by the program; `None` when the catalog default applies,
    /// so that `branch.<name>.remote` takes precedence over `origin`.
    pub remote: Option<String>,
    /// Remote branch given by the program; `None` selects the upstream of
    /// the current branch.
    pub branch: Option<String>,
    /// Integration mode given by the program; `None` selects the mode from
    /// the repository configuration.
    pub mode: Option<PullMode>,
    /// SHA-256 fingerprint (`SHA256:<base64>`) of an SSH host key to accept
    /// when the host is not known.
    pub accept_host_key: Option<String>,
}

impl PullParams {
    /// Reads the parameters from the bound arguments.
    ///
    /// # Errors
    ///
    /// `E_MISSING_PARAM` without `repo`, `E_TYPE` for a value of the wrong
    /// type and `E_UNKNOWN_VALUE` for an unknown `mode`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::stage::{ArgValue, BoundArgs};
    /// use nurt_contracts::value::Value;
    /// use nurt_git_ops::pull::{PullMode, PullParams};
    ///
    /// let args = BoundArgs::new("git.pull")
    ///     .with("repo", ArgValue::Value(Value::Path("/home/user/repo".into())), false)
    ///     .with("remote", ArgValue::Value(Value::Text("origin".into())), true)
    ///     .with("mode", ArgValue::Value(Value::Enum("rebase".into())), false);
    /// let params = PullParams::from_args(&args).unwrap();
    /// assert_eq!(params.remote, None);
    /// assert_eq!(params.mode, Some(PullMode::Rebase));
    /// ```
    pub fn from_args(args: &BoundArgs) -> Result<Self> {
        let repo = match args.require("repo")? {
            Value::Path(path) | Value::Text(path) => PathBuf::from(path),
            _ => return Err(type_error("repo", "path")),
        };
        let remote = given_text(args, "remote")?;
        let branch = given_text(args, "branch")?;
        let mode = match given_value(args, "mode") {
            None => None,
            Some(Value::Enum(text) | Value::Text(text)) => Some(text.parse()?),
            Some(_) => return Err(type_error("mode", "enum")),
        };
        Ok(Self {
            repo,
            remote,
            branch,
            mode,
            accept_host_key: given_text(args, "accept_host_key")?,
        })
    }
}

fn given_value<'a>(args: &'a BoundArgs, name: &str) -> Option<&'a Value> {
    if args.is_given(name) {
        args.value(name)
    } else {
        None
    }
}

fn given_text(args: &BoundArgs, name: &str) -> Result<Option<String>> {
    match given_value(args, name) {
        None => Ok(None),
        Some(Value::Text(text)) if text.is_empty() => Err(Error::new(
            ErrorCode::InvalidValue,
            format!("`{name}` is empty; give a name or leave the parameter out"),
        )
        .with_stage(Stage::Bind)),
        Some(Value::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(type_error(name, "text")),
    }
}

fn type_error(name: &str, expected: &str) -> Error {
    Error::new(
        ErrorCode::Type,
        format!("`git.pull` expects {expected} for `{name}`"),
    )
    .with_stage(Stage::Bind)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;

    use super::*;

    fn args() -> BoundArgs {
        BoundArgs::new("git.pull").with(
            "repo",
            ArgValue::Value(Value::Path("/work/repo".into())),
            false,
        )
    }

    fn text(value: &str) -> ArgValue {
        ArgValue::Value(Value::Text(value.into()))
    }

    #[test]
    fn reads_given_parameters() {
        let args = args()
            .with("remote", text("upstream"), false)
            .with("branch", text("dev"), false)
            .with(
                "mode",
                ArgValue::Value(Value::Enum("fast_forward_only".into())),
                false,
            );
        let params = PullParams::from_args(&args).unwrap();
        assert_eq!(params.repo, PathBuf::from("/work/repo"));
        assert_eq!(params.remote.as_deref(), Some("upstream"));
        assert_eq!(params.branch.as_deref(), Some("dev"));
        assert_eq!(params.mode, Some(PullMode::FastForwardOnly));
        assert_eq!(params.accept_host_key, None);
        let key = args.with("accept_host_key", text("SHA256:AAAA"), false);
        assert_eq!(
            PullParams::from_args(&key)
                .unwrap()
                .accept_host_key
                .as_deref(),
            Some("SHA256:AAAA")
        );
    }

    #[test]
    fn treats_catalog_default_remote_as_not_given() {
        let args = args().with("remote", text("origin"), true);
        let params = PullParams::from_args(&args).unwrap();
        assert_eq!(params.remote, None);
        assert_eq!(params.branch, None);
        assert_eq!(params.mode, None);
    }

    #[test]
    fn accepts_mode_as_text() {
        let args = args().with("mode", text("merge"), false);
        assert_eq!(
            PullParams::from_args(&args).unwrap().mode,
            Some(PullMode::Merge)
        );
    }

    #[test]
    fn rejects_unknown_mode() {
        let args = args().with("mode", text("squash"), false);
        let error = PullParams::from_args(&args).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownValue);
        assert!(error.message.contains("squash"));
    }

    #[test]
    fn rejects_missing_repository() {
        let args = BoundArgs::new("git.pull");
        let error = PullParams::from_args(&args).unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingParam);
    }

    #[test]
    fn rejects_values_of_the_wrong_type() {
        let repo = BoundArgs::new("git.pull").with("repo", ArgValue::Value(Value::Int(1)), false);
        assert_eq!(
            PullParams::from_args(&repo).unwrap_err().code,
            ErrorCode::Type
        );
        let remote = args().with("remote", ArgValue::Value(Value::Bool(true)), false);
        assert_eq!(
            PullParams::from_args(&remote).unwrap_err().code,
            ErrorCode::Type
        );
        let mode = args().with("mode", ArgValue::Value(Value::Int(2)), false);
        assert_eq!(
            PullParams::from_args(&mode).unwrap_err().code,
            ErrorCode::Type
        );
    }

    #[test]
    fn rejects_empty_branch_name() {
        let args = args().with("branch", text(""), false);
        assert_eq!(
            PullParams::from_args(&args).unwrap_err().code,
            ErrorCode::InvalidValue
        );
    }

    #[test]
    fn maps_modes_to_git_options() {
        let options: Vec<_> = PullMode::ALL.iter().map(|mode| mode.git_option()).collect();
        assert_eq!(options, ["--no-rebase", "--rebase", "--ff-only"]);
        assert_eq!(PullMode::Merge.to_string(), "merge");
    }
}
