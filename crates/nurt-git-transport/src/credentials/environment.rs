// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Credentials from environment variables.
//!
//! `NURT_GIT_TOKEN_<HOST>` and `NURT_GIT_TOKEN` hold a token sent as the
//! password of Basic authentication, with the user name from
//! `NURT_GIT_USERNAME_<HOST>`, `NURT_GIT_USERNAME`, the address or the
//! configuration. `<HOST>` is the host name in capitals with every character
//! other than a letter or digit replaced by `_`, for example
//! `NURT_GIT_TOKEN_GITHUB_COM`. Variables of hosting providers apply to their
//! own hosts only: `GH_TOKEN` and `GITHUB_TOKEN` to `github.com`,
//! `GITLAB_TOKEN` to `gitlab.com`, and `CI_JOB_TOKEN` to the host named by
//! `CI_SERVER_HOST`.

/// User name sent with a token when no other user name is known; accepted
/// by hosting services that authenticate by the token alone.
const DEFAULT_TOKEN_USER: &str = "x-access-token";

/// User name required with a GitLab CI job token.
const CI_JOB_TOKEN_USER: &str = "gitlab-ci-token";

/// User name for GitLab tokens without a known user name.
const GITLAB_TOKEN_USER: &str = "oauth2";

/// A token found in the environment.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct EnvToken {
    /// Name of the variable holding the token.
    pub(crate) variable: String,
    /// User name to send with the token.
    pub(crate) username: String,
    /// The token.
    pub(crate) token: String,
}

impl std::fmt::Debug for EnvToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvToken")
            .field("variable", &self.variable)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// Suffix of host-specific variable names for `host` (without port).
pub(crate) fn host_suffix(host: &str) -> String {
    host.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn non_empty(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    env(name).filter(|value| !value.is_empty())
}

/// Token for `host` from the variables of Nurt and, when `providers` is set,
/// of hosting providers. `username` is the user name already known from the
/// address or the configuration.
pub(crate) fn token(
    env: &dyn Fn(&str) -> Option<String>,
    host: &str,
    username: Option<&str>,
    providers: bool,
) -> Option<EnvToken> {
    let suffix = host_suffix(host);
    let configured_user = non_empty(env, &format!("NURT_GIT_USERNAME_{suffix}"))
        .or_else(|| non_empty(env, "NURT_GIT_USERNAME"))
        .or_else(|| username.map(str::to_owned));
    for variable in [
        format!("NURT_GIT_TOKEN_{suffix}"),
        "NURT_GIT_TOKEN".to_owned(),
    ] {
        if let Some(token) = non_empty(env, &variable) {
            let username = configured_user
                .clone()
                .unwrap_or_else(|| DEFAULT_TOKEN_USER.to_owned());
            return Some(EnvToken {
                variable,
                username,
                token,
            });
        }
    }
    if providers {
        return provider_token(env, host, configured_user);
    }
    None
}

fn provider_token(
    env: &dyn Fn(&str) -> Option<String>,
    host: &str,
    configured_user: Option<String>,
) -> Option<EnvToken> {
    let host = host.to_ascii_lowercase();
    let found = |variables: &[&str], default_user: &str| {
        variables.iter().find_map(|variable| {
            non_empty(env, variable).map(|token| EnvToken {
                variable: (*variable).to_owned(),
                username: configured_user
                    .clone()
                    .unwrap_or_else(|| default_user.to_owned()),
                token,
            })
        })
    };
    let ci_host = non_empty(env, "CI_SERVER_HOST").map(|h| h.to_ascii_lowercase());
    if ci_host.as_deref() == Some(host.as_str())
        && let Some(token) = non_empty(env, "CI_JOB_TOKEN")
    {
        return Some(EnvToken {
            variable: "CI_JOB_TOKEN".to_owned(),
            username: CI_JOB_TOKEN_USER.to_owned(),
            token,
        });
    }
    match host.as_str() {
        "github.com" => found(&["GH_TOKEN", "GITHUB_TOKEN"], DEFAULT_TOKEN_USER),
        "gitlab.com" => found(&["GITLAB_TOKEN"], GITLAB_TOKEN_USER),
        _ => None,
    }
}

/// Bearer token for `host` from `NURT_GIT_BEARER_TOKEN_<HOST>` or
/// `NURT_GIT_BEARER_TOKEN`, with the name of the variable.
pub(crate) fn bearer(env: &dyn Fn(&str) -> Option<String>, host: &str) -> Option<(String, String)> {
    let suffix = host_suffix(host);
    [
        format!("NURT_GIT_BEARER_TOKEN_{suffix}"),
        "NURT_GIT_BEARER_TOKEN".to_owned(),
    ]
    .into_iter()
    .find_map(|variable| non_empty(env, &variable).map(|token| (variable, token)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| vars.get(name).cloned()
    }

    #[test]
    fn builds_host_suffix_from_letters_and_digits() {
        assert_eq!(host_suffix("github.com"), "GITHUB_COM");
        assert_eq!(host_suffix("git-01.example.org"), "GIT_01_EXAMPLE_ORG");
    }

    #[test]
    fn host_specific_token_precedes_generic_token() {
        let vars = env(&[
            ("NURT_GIT_TOKEN", "generic"),
            ("NURT_GIT_TOKEN_EXAMPLE_COM", "specific"),
        ]);
        let found = token(&vars, "example.com", None, true).expect("token");
        assert_eq!(found.variable, "NURT_GIT_TOKEN_EXAMPLE_COM");
        assert_eq!(found.token, "specific");
        assert_eq!(found.username, DEFAULT_TOKEN_USER);
        let other = token(&vars, "other.org", Some("bob"), true).expect("token");
        assert_eq!(
            (other.variable.as_str(), other.username.as_str()),
            ("NURT_GIT_TOKEN", "bob")
        );
    }

    #[test]
    fn username_variables_override_known_user() {
        let vars = env(&[
            ("NURT_GIT_TOKEN", "t"),
            ("NURT_GIT_USERNAME_H", "carol"),
            ("NURT_GIT_USERNAME", "dave"),
        ]);
        assert_eq!(
            token(&vars, "h", Some("bob"), true)
                .expect("token")
                .username,
            "carol"
        );
        assert_eq!(
            token(&vars, "x", Some("bob"), true)
                .expect("token")
                .username,
            "dave"
        );
    }

    #[test]
    fn provider_variables_apply_to_their_hosts_only() {
        let vars = env(&[("GITHUB_TOKEN", "gh"), ("GITLAB_TOKEN", "gl")]);
        let github = token(&vars, "github.com", None, true).expect("github");
        assert_eq!(
            (github.variable.as_str(), github.username.as_str()),
            ("GITHUB_TOKEN", DEFAULT_TOKEN_USER)
        );
        let gitlab = token(&vars, "gitlab.com", None, true).expect("gitlab");
        assert_eq!(gitlab.username, GITLAB_TOKEN_USER);
        assert!(token(&vars, "example.com", None, true).is_none());
        assert!(token(&vars, "github.com", None, false).is_none());
    }

    #[test]
    fn ci_job_token_applies_to_the_ci_server() {
        let vars = env(&[
            ("CI_JOB_TOKEN", "job"),
            ("CI_SERVER_HOST", "git.corp.example"),
        ]);
        let found = token(&vars, "git.corp.example", Some("bob"), true).expect("job token");
        assert_eq!(
            (found.variable.as_str(), found.username.as_str()),
            ("CI_JOB_TOKEN", CI_JOB_TOKEN_USER)
        );
        assert!(token(&vars, "other.example", None, true).is_none());
    }

    #[test]
    fn empty_variables_are_ignored() {
        let vars = env(&[("NURT_GIT_TOKEN_H", ""), ("NURT_GIT_TOKEN", "t")]);
        assert_eq!(
            token(&vars, "h", None, false).expect("token").variable,
            "NURT_GIT_TOKEN"
        );
    }

    #[test]
    fn finds_bearer_token_by_host() {
        let vars = env(&[
            ("NURT_GIT_BEARER_TOKEN_H", "b1"),
            ("NURT_GIT_BEARER_TOKEN", "b2"),
        ]);
        assert_eq!(
            bearer(&vars, "h"),
            Some(("NURT_GIT_BEARER_TOKEN_H".to_owned(), "b1".to_owned()))
        );
        assert_eq!(bearer(&vars, "x").map(|(_, t)| t), Some("b2".to_owned()));
    }

    #[test]
    fn debug_output_hides_the_token() {
        let vars = env(&[("NURT_GIT_TOKEN", "hidden-value")]);
        let found = token(&vars, "h", None, false).expect("token");
        assert!(!format!("{found:?}").contains("hidden-value"));
    }
}
