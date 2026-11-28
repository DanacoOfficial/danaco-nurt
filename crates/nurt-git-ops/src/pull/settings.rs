// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-ops::pull::settings
// Responsibility:   Remote, refspecs and integration strategy of a pull, resolved from the parameters and the repository configuration as `git pull` 2.53 does.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Remote, refspecs and integration strategy of a pull, resolved from the
//! parameters and the repository configuration as `git pull` 2.53 does.

use super::params::{PullMode, PullParams};

/// Name of the remote used when neither the program nor the configuration
/// names one.
pub const DEFAULT_REMOTE: &str = "origin";

/// Read access to the effective repository configuration.
///
/// Keys are written as in `git config` (`branch.main.rebase`). A key given
/// without a value (`[pull] rebase`) reads as `"true"`.
pub trait ConfigSource {
    /// Last value of a key.
    fn get(&self, key: &str) -> Option<String>;

    /// Every value of a multi-valued key, in configuration order.
    fn get_all(&self, key: &str) -> Vec<String>;
}

/// Rebase setting of `branch.<name>.rebase` and `pull.rebase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RebaseKind {
    /// Merge instead of rebasing.
    No,
    /// Rebase; `interactive` behaves the same, because the unchanged todo
    /// list is the plain rebase.
    Plain,
    /// Rebase and recreate local merge commits (`merges`).
    Merges,
}

/// Fast-forward policy of `pull.ff` and of the integration mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FfPolicy {
    /// Fast-forward when possible, otherwise merge (`pull.ff=true`).
    Allow,
    /// Always create a merge commit (`pull.ff=false`).
    Never,
    /// Refuse anything but a fast-forward (`pull.ff=only`).
    Only,
}

impl FfPolicy {
    /// Value of the `fast_forward` parameter of `git.merge` with the same
    /// meaning.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "auto",
            Self::Never => "never",
            Self::Only => "only",
        }
    }
}

/// Integration strategy before the relation of the histories is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Strategy {
    /// Rebase setting in effect.
    pub rebase: RebaseKind,
    /// Fast-forward policy; `None` when neither the program nor `pull.ff`
    /// sets one.
    pub ff: Option<FfPolicy>,
    /// Neither the program nor the configuration chose between merge and
    /// rebase; diverged histories are then refused.
    pub rebase_unspecified: bool,
    /// `rebase.autoStash` is set: uncommitted changes do not block a rebase.
    pub autostash: bool,
}

/// A configuration value that cannot be interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSetting {
    /// Configuration key.
    pub key: String,
    /// Rejected value.
    pub value: String,
}

/// Where a pull fetches from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Remote name, a URL or `.` for the repository itself.
    pub remote: String,
    /// Refspecs given by the program; empty selects the configured
    /// `remote.<name>.fetch` refspecs.
    pub refspecs: Vec<String>,
    /// Upstream branches of the current branch (`branch.<name>.merge`), used
    /// when no refspec is given and the remote is the upstream remote.
    pub merge_refs: Vec<String>,
    /// The remote is the configured upstream remote of the current branch.
    pub upstream_remote: bool,
}

/// Parses a boolean the way `git config` does.
fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Some(true),
        "false" | "no" | "off" | "" => Some(false),
        number => number.parse::<i64>().ok().map(|number| number != 0),
    }
}

fn parse_rebase(value: &str) -> Option<RebaseKind> {
    match value {
        "merges" | "m" => Some(RebaseKind::Merges),
        "interactive" | "i" => Some(RebaseKind::Plain),
        other => parse_bool(other).map(|rebase| {
            if rebase {
                RebaseKind::Plain
            } else {
                RebaseKind::No
            }
        }),
    }
}

fn parse_ff(value: &str) -> Option<FfPolicy> {
    if value == "only" {
        return Some(FfPolicy::Only);
    }
    parse_bool(value).map(|ff| if ff { FfPolicy::Allow } else { FfPolicy::Never })
}

fn read<T>(
    config: &dyn ConfigSource,
    key: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, InvalidSetting> {
    match config.get(key) {
        None => Ok(None),
        Some(value) => parse(&value).map(Some).ok_or_else(|| InvalidSetting {
            key: key.to_owned(),
            value,
        }),
    }
}

/// Resolves the integration strategy from the mode and the configuration.
///
/// An explicit `merge` or `rebase` overrides `pull.ff=only`, `pull.ff` is
/// read for every mode but `fast_forward_only`, and `branch.<name>.rebase`
/// takes precedence over `pull.rebase`.
///
/// # Errors
///
/// [`InvalidSetting`] for a value that `git` rejects as well, such as
/// `pull.rebase=preserve`.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
///
/// use nurt_git_ops::pull::{ConfigSource, FfPolicy, PullMode, RebaseKind, resolve_strategy};
///
/// struct Config(BTreeMap<&'static str, &'static str>);
///
/// impl ConfigSource for Config {
///     fn get(&self, key: &str) -> Option<String> {
///         self.0.get(key).map(|value| (*value).to_owned())
///     }
///     fn get_all(&self, key: &str) -> Vec<String> {
///         self.get(key).into_iter().collect()
///     }
/// }
///
/// let config = Config(BTreeMap::from([("pull.ff", "only")]));
/// let strategy = resolve_strategy(Some(PullMode::Rebase), &config, Some("main")).unwrap();
/// assert_eq!(strategy.rebase, RebaseKind::Plain);
/// assert_eq!(strategy.ff, Some(FfPolicy::Allow));
/// ```
pub fn resolve_strategy(
    mode: Option<PullMode>,
    config: &dyn ConfigSource,
    branch: Option<&str>,
) -> Result<Strategy, InvalidSetting> {
    let ff = match mode {
        Some(PullMode::FastForwardOnly) => Some(FfPolicy::Only),
        Some(PullMode::Merge | PullMode::Rebase) => {
            read(config, "pull.ff", parse_ff)?.map(|ff| match ff {
                FfPolicy::Only => FfPolicy::Allow,
                other => other,
            })
        }
        None => read(config, "pull.ff", parse_ff)?,
    };
    let (rebase, rebase_unspecified) = match mode {
        Some(PullMode::Merge) => (RebaseKind::No, false),
        Some(PullMode::Rebase) => (RebaseKind::Plain, false),
        Some(PullMode::FastForwardOnly) | None => match configured_rebase(config, branch)? {
            Some(rebase) => (rebase, false),
            None => (RebaseKind::No, true),
        },
    };
    let autostash = read(config, "rebase.autoStash", parse_bool)?.unwrap_or(false);
    Ok(Strategy {
        rebase,
        ff,
        rebase_unspecified,
        autostash,
    })
}

fn configured_rebase(
    config: &dyn ConfigSource,
    branch: Option<&str>,
) -> Result<Option<RebaseKind>, InvalidSetting> {
    if let Some(branch) = branch {
        let key = format!("branch.{branch}.rebase");
        if let Some(rebase) = read(config, &key, parse_rebase)? {
            return Ok(Some(rebase));
        }
    }
    read(config, "pull.rebase", parse_rebase)
}

/// Resolves the remote and the refspecs to fetch.
///
/// The remote is the `remote` parameter, otherwise `branch.<name>.remote`
/// of the current branch, otherwise `origin`.
#[must_use]
pub fn resolve_target(
    params: &PullParams,
    config: &dyn ConfigSource,
    branch: Option<&str>,
) -> Target {
    let branch_remote = branch.and_then(|name| config.get(&format!("branch.{name}.remote")));
    let merge_refs = branch
        .map(|name| config.get_all(&format!("branch.{name}.merge")))
        .unwrap_or_default();
    let remote = params
        .remote
        .clone()
        .or_else(|| branch_remote.clone())
        .unwrap_or_else(|| DEFAULT_REMOTE.to_owned());
    let upstream_remote = branch_remote.as_deref() == Some(remote.as_str())
        || (branch_remote.is_none() && params.remote.is_none() && !merge_refs.is_empty());
    Target {
        refspecs: params.branch.iter().cloned().collect(),
        remote,
        merge_refs,
        upstream_remote,
    }
}

/// Full name of a branch given as a refspec source (`main` becomes
/// `refs/heads/main`).
#[must_use]
pub fn full_source_ref(source: &str) -> String {
    if source.starts_with("refs/") || source == "HEAD" {
        source.to_owned()
    } else {
        format!("refs/heads/{source}")
    }
}

/// Remote-tracking reference to which the fetch refspecs map a source reference.
///
/// The mapping is the one `git` uses to find the fork point of a rebase.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::pull::tracking_ref;
///
/// let refspecs = ["+refs/heads/*:refs/remotes/origin/*".to_owned()];
/// assert_eq!(
///     tracking_ref(&refspecs, "refs/heads/main").as_deref(),
///     Some("refs/remotes/origin/main")
/// );
/// assert_eq!(tracking_ref(&refspecs, "refs/tags/v1"), None);
/// ```
#[must_use]
pub fn tracking_ref(refspecs: &[String], source: &str) -> Option<String> {
    refspecs.iter().find_map(|refspec| {
        let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
        let (from, to) = refspec.split_once(':')?;
        if to.is_empty() || from.starts_with('^') {
            return None;
        }
        match (from.split_once('*'), to.split_once('*')) {
            (Some((prefix, suffix)), Some((to_prefix, to_suffix))) => {
                let middle = source.strip_prefix(prefix)?.strip_suffix(suffix)?;
                Some(format!("{to_prefix}{middle}{to_suffix}"))
            }
            (None, None) if from == source => Some(to.to_owned()),
            _ => None,
        }
    })
}

/// Reflog action of the equivalent `git pull` command line.
///
/// The action is `pull`, followed by the option of an explicit mode and by
/// the remote and the branch when they are given.
///
/// `git` uses the action as the prefix of every reflog message it writes
/// during the pull, so the messages written by Nurt match those of the same
/// `git pull` invocation byte for byte.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
///
/// use nurt_git_ops::pull::{PullMode, PullParams, reflog_action};
///
/// let params = PullParams {
///     repo: PathBuf::from("/home/user/repo"),
///     remote: None,
///     branch: Some("main".into()),
///     mode: Some(PullMode::Rebase),
///     accept_host_key: None,
/// };
/// assert_eq!(reflog_action(&params, "origin"), "pull --rebase origin main");
/// ```
#[must_use]
pub fn reflog_action(params: &PullParams, remote: &str) -> String {
    let mut action = String::from("pull");
    if let Some(mode) = params.mode {
        action.push(' ');
        action.push_str(mode.git_option());
    }
    if params.remote.is_some() || params.branch.is_some() {
        action.push(' ');
        action.push_str(remote);
    }
    if let Some(branch) = &params.branch {
        action.push(' ');
        action.push_str(branch);
    }
    action
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;

    #[derive(Default)]
    pub(crate) struct MapConfig(pub(crate) BTreeMap<String, Vec<String>>);

    impl MapConfig {
        pub(crate) fn with(mut self, key: &str, value: &str) -> Self {
            self.0
                .entry(key.to_owned())
                .or_default()
                .push(value.to_owned());
            self
        }
    }

    impl ConfigSource for MapConfig {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).and_then(|values| values.last().cloned())
        }

        fn get_all(&self, key: &str) -> Vec<String> {
            self.0.get(key).cloned().unwrap_or_default()
        }
    }

    fn strategy(mode: Option<PullMode>, config: &MapConfig) -> Strategy {
        resolve_strategy(mode, config, Some("main")).unwrap()
    }

    fn params(remote: Option<&str>, branch: Option<&str>, mode: Option<PullMode>) -> PullParams {
        PullParams {
            repo: PathBuf::from("/work/repo"),
            remote: remote.map(str::to_owned),
            branch: branch.map(str::to_owned),
            mode,
            accept_host_key: None,
        }
    }

    #[test]
    fn leaves_the_choice_open_without_configuration() {
        let resolved = strategy(None, &MapConfig::default());
        assert_eq!(resolved.rebase, RebaseKind::No);
        assert_eq!(resolved.ff, None);
        assert!(resolved.rebase_unspecified);
        assert!(!resolved.autostash);
    }

    #[test]
    fn reads_rebase_values_like_git() {
        let cases = [
            ("true", RebaseKind::Plain),
            ("TRUE", RebaseKind::Plain),
            ("yes", RebaseKind::Plain),
            ("1", RebaseKind::Plain),
            ("false", RebaseKind::No),
            ("0", RebaseKind::No),
            ("", RebaseKind::No),
            ("merges", RebaseKind::Merges),
            ("m", RebaseKind::Merges),
            ("interactive", RebaseKind::Plain),
            ("i", RebaseKind::Plain),
        ];
        for (value, expected) in cases {
            let config = MapConfig::default().with("pull.rebase", value);
            let resolved = strategy(None, &config);
            assert_eq!(resolved.rebase, expected, "pull.rebase={value}");
            assert!(!resolved.rebase_unspecified);
        }
    }

    #[test]
    fn rejects_values_git_rejects() {
        for (key, value) in [
            ("pull.rebase", "preserve"),
            ("pull.rebase", "bogus"),
            ("pull.ff", "bogus"),
        ] {
            let config = MapConfig::default().with(key, value);
            let error = resolve_strategy(None, &config, Some("main")).unwrap_err();
            assert_eq!(error.key, key);
            assert_eq!(error.value, value);
        }
    }

    #[test]
    fn prefers_the_branch_setting_over_pull_rebase() {
        let config = MapConfig::default()
            .with("pull.rebase", "false")
            .with("branch.main.rebase", "true");
        assert_eq!(strategy(None, &config).rebase, RebaseKind::Plain);
        let detached = resolve_strategy(None, &config, None).unwrap();
        assert_eq!(detached.rebase, RebaseKind::No);
    }

    #[test]
    fn explicit_merge_or_rebase_overrides_ff_only() {
        let config = MapConfig::default().with("pull.ff", "only");
        assert_eq!(
            strategy(Some(PullMode::Merge), &config).ff,
            Some(FfPolicy::Allow)
        );
        assert_eq!(
            strategy(Some(PullMode::Rebase), &config).ff,
            Some(FfPolicy::Allow)
        );
        assert_eq!(strategy(None, &config).ff, Some(FfPolicy::Only));
    }

    #[test]
    fn explicit_merge_keeps_pull_ff_false() {
        let config = MapConfig::default().with("pull.ff", "false");
        let resolved = strategy(Some(PullMode::Merge), &config);
        assert_eq!(resolved.ff, Some(FfPolicy::Never));
        assert_eq!(resolved.rebase, RebaseKind::No);
    }

    #[test]
    fn fast_forward_only_ignores_pull_ff_but_reads_pull_rebase() {
        let config = MapConfig::default()
            .with("pull.ff", "bogus")
            .with("pull.rebase", "true");
        let resolved = strategy(Some(PullMode::FastForwardOnly), &config);
        assert_eq!(resolved.ff, Some(FfPolicy::Only));
        assert_eq!(resolved.rebase, RebaseKind::Plain);
        let invalid = MapConfig::default().with("pull.rebase", "preserve");
        assert!(resolve_strategy(Some(PullMode::FastForwardOnly), &invalid, None).is_err());
    }

    #[test]
    fn explicit_mode_does_not_need_pull_rebase() {
        let config = MapConfig::default().with("pull.rebase", "preserve");
        let merge = resolve_strategy(Some(PullMode::Merge), &config, None).unwrap();
        assert_eq!(merge.rebase, RebaseKind::No);
        let rebase = resolve_strategy(Some(PullMode::Rebase), &config, None).unwrap();
        assert_eq!(rebase.rebase, RebaseKind::Plain);
    }

    #[test]
    fn reads_ff_values_and_autostash() {
        let config = MapConfig::default()
            .with("pull.ff", "true")
            .with("rebase.autoStash", "yes");
        let resolved = strategy(None, &config);
        assert_eq!(resolved.ff, Some(FfPolicy::Allow));
        assert!(resolved.autostash);
        let never = MapConfig::default().with("pull.ff", "off");
        assert_eq!(strategy(None, &never).ff, Some(FfPolicy::Never));
        assert_eq!(FfPolicy::Only.as_str(), "only");
    }

    #[test]
    fn takes_the_remote_from_the_branch_configuration() {
        let config = MapConfig::default()
            .with("branch.main.remote", "upstream")
            .with("branch.main.merge", "refs/heads/main");
        let target = resolve_target(&params(None, None, None), &config, Some("main"));
        assert_eq!(target.remote, "upstream");
        assert!(target.upstream_remote);
        assert_eq!(target.merge_refs, ["refs/heads/main"]);
        assert!(target.refspecs.is_empty());
    }

    #[test]
    fn defaults_to_origin() {
        let target = resolve_target(
            &params(None, None, None),
            &MapConfig::default(),
            Some("feature"),
        );
        assert_eq!(target.remote, DEFAULT_REMOTE);
        assert!(!target.upstream_remote);
        let detached = resolve_target(
            &params(None, Some("dev"), None),
            &MapConfig::default(),
            None,
        );
        assert_eq!(detached.remote, DEFAULT_REMOTE);
        assert_eq!(detached.refspecs, ["dev"]);
    }

    #[test]
    fn marks_another_remote_as_not_upstream() {
        let config = MapConfig::default()
            .with("branch.main.remote", "origin")
            .with("branch.main.merge", "refs/heads/main");
        let target = resolve_target(&params(Some("other"), None, None), &config, Some("main"));
        assert_eq!(target.remote, "other");
        assert!(!target.upstream_remote);
    }

    #[test]
    fn maps_sources_through_fetch_refspecs() {
        let refspecs = vec![
            "^refs/heads/secret".to_owned(),
            "refs/heads/stable:refs/remotes/origin/stable-copy".to_owned(),
            "+refs/heads/*:refs/remotes/origin/*".to_owned(),
        ];
        assert_eq!(
            tracking_ref(&refspecs, "refs/heads/stable").as_deref(),
            Some("refs/remotes/origin/stable-copy")
        );
        assert_eq!(
            tracking_ref(&refspecs, "refs/heads/topic/x").as_deref(),
            Some("refs/remotes/origin/topic/x")
        );
        assert_eq!(
            tracking_ref(&["refs/heads/main".to_owned()], "refs/heads/main"),
            None
        );
        assert_eq!(full_source_ref("main"), "refs/heads/main");
        assert_eq!(full_source_ref("refs/tags/v1"), "refs/tags/v1");
        assert_eq!(full_source_ref("HEAD"), "HEAD");
    }

    #[test]
    fn composes_reflog_actions_like_git_pull() {
        assert_eq!(reflog_action(&params(None, None, None), "origin"), "pull");
        assert_eq!(
            reflog_action(
                &params(None, None, Some(PullMode::FastForwardOnly)),
                "origin"
            ),
            "pull --ff-only"
        );
        assert_eq!(
            reflog_action(&params(Some("up"), None, Some(PullMode::Merge)), "up"),
            "pull --no-rebase up"
        );
        assert_eq!(
            reflog_action(&params(Some("up"), Some("dev"), None), "up"),
            "pull up dev"
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
